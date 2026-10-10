use crate::domains::message::utils::CHANNEL_ID_OFFSET;
use crate::infra::util;
use anyhow::{Result, anyhow};
use grammers_client::Client;
use grammers_client::client::{
    DialogIter, MessageIter, PasswordToken, UpdateStream, UpdatesConfiguration,
};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use grammers_session::types::PeerRef;
use grammers_session::updates::UpdatesLike;
use grammers_tl_types as tl;
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct TelegramClient {
    api_id: i32,
    api_hash: String,
    client: Client,
    runner: Option<tokio::task::JoinHandle<()>>,
    updates: Option<UnboundedReceiver<UpdatesLike>>,
}

impl TelegramClient {
    pub async fn new(api_id: i32, api_hash: String, session_path: String) -> Result<Self> {
        let session = Arc::new(SqliteSession::open(session_path).await?);

        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::new(session, api_id);

        let client = Client::new(handle);

        Ok(Self {
            api_id,
            api_hash,
            client,
            runner: Some(tokio::spawn(async move {
                let _ = runner.run().await;
            })),
            updates: Some(updates),
        })
    }

    pub fn connect(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.runner.take()
    }

    pub async fn stream_updates(
        &mut self,
        catch_up: bool,
        queue_limit: usize,
    ) -> Result<UpdateStream> {
        let updates = self
            .updates
            .take()
            .ok_or_else(|| anyhow!("Telegram updates stream has already been taken"))?;

        let configuration = UpdatesConfiguration {
            catch_up,
            update_queue_limit: Some(queue_limit),
        };

        let stream = self.client.stream_updates(updates, configuration).await;

        Ok(stream)
    }

    pub fn iter_dialogs(&self) -> DialogIter {
        self.client.iter_dialogs()
    }

    pub fn iter_messages(&self, peer: PeerRef) -> MessageIter {
        self.client.iter_messages(peer)
    }

    pub async fn check_authorization(&self) -> Result<()> {
        if self.client.is_authorized().await? {
            return Ok(());
        }

        log::info!("Not authorized. Starting QR code sign-in process.");

        // 1. Request the initial Token
        let mut current_token_object = self
            .client
            .invoke(&tl::functions::auth::ExportLoginToken {
                api_id: self.api_id,
                api_hash: self.api_hash.clone(),
                except_ids: Vec::new(),
            })
            .await?;

        while !self.client.is_authorized().await? {
            match current_token_object {
                tl::enums::auth::LoginToken::Token(ref t) => {
                    // 2. Display the QR Code
                    let url = format!("tg://login?token={}", base64_url::encode(&t.token));
                    println!("\nScan this QR code with your Telegram app:");
                    qr2term::print_qr(&url)?;
                    log::info!("Waiting for scan...");

                    // 3. Poll loop
                    loop {
                        util::sleep(1).await;

                        let res = self
                            .client
                            .invoke(&tl::functions::auth::ExportLoginToken {
                                api_id: self.api_id,
                                api_hash: self.api_hash.clone(),
                                except_ids: Vec::new(),
                            })
                            .await;

                        match res {
                            Ok(tl::enums::auth::LoginToken::Success(_s)) => {
                                log::info!("✅ Scanned! Completing login...");
                                // This break will exit the polling loop and re-check is_authorized() in while
                                break;
                            }
                            Ok(tl::enums::auth::LoginToken::Token(new_t)) => {
                                if new_t.token != t.token {
                                    log::debug!("🔄 Token refreshed.");
                                    current_token_object =
                                        tl::enums::auth::LoginToken::Token(new_t);
                                    // This break will exit the polling loop and re-match in while to show new QR
                                    break;
                                }
                            }
                            Ok(tl::enums::auth::LoginToken::MigrateTo(_)) => {
                                return Err(anyhow!("Migration required (not implemented)"));
                            }
                            Err(e) => {
                                if e.is("SESSION_PASSWORD_NEEDED") {
                                    log::info!("✅ QR Scanned! 2FA Password required.");
                                    // Handle password
                                    self.handle_password().await?;
                                    // After password, we should be authorized, while loop will exit.
                                    break;
                                }

                                log::error!("Error polling token: {e:?}");
                                return Err(e.into());
                            }
                        }
                    }
                }
                tl::enums::auth::LoginToken::Success(_) => break,
                tl::enums::auth::LoginToken::MigrateTo(_) => {
                    return Err(anyhow!("Migration required"));
                }
            }
        }

        log::info!("🎉 Successfully signed in!");
        Ok(())
    }

    pub async fn folder_peer_ids(&self, folder_name: &str) -> Result<Option<HashSet<i64>>> {
        let filters = self
            .client
            .invoke(&tl::functions::messages::GetDialogFilters {})
            .await?;

        let tl::enums::messages::DialogFilters::Filters(filters) = filters;

        let Some(filter) = filters.filters.into_iter().find_map(|filter| match filter {
            tl::enums::DialogFilter::Filter(filter) => {
                let tl::enums::TextWithEntities::Entities(title) = &filter.title;

                (title.text.trim() == folder_name.trim()).then_some(filter)
            }
            _ => None,
        }) else {
            return Ok(None);
        };

        if filter.contacts
            || filter.non_contacts
            || filter.groups
            || filter.broadcasts
            || filter.bots
        {
            log::warn!(
                "Folder '{folder_name}' uses category rules; membership based on explicit peers may be incomplete."
            );
        }

        let mut ids = HashSet::new();

        for peer in filter
            .pinned_peers
            .iter()
            .chain(filter.include_peers.iter())
        {
            if let Some(id) = input_peer_to_id(peer) {
                ids.insert(id);
            }
        }

        for peer in &filter.exclude_peers {
            if let Some(id) = input_peer_to_id(peer) {
                ids.remove(&id);
            }
        }

        Ok(Some(ids))
    }

    pub async fn mark_discussion_read(
        &self,
        peer: tl::enums::InputPeer,
        msg_id: i32,
        read_max_id: i32,
    ) -> Result<()> {
        self.client
            .invoke(&tl::functions::messages::ReadDiscussion {
                peer,
                msg_id,
                read_max_id,
            })
            .await?;

        Ok(())
    }

    async fn handle_password(&self) -> Result<()> {
        // Fetch the 2FA configuration
        let password_res = self
            .client
            .invoke(&tl::functions::account::GetPassword {})
            .await;

        match password_res {
            Ok(tl::enums::account::Password::Password(p)) => {
                let hint = p.hint.as_deref().unwrap_or("no hint");
                println!("\n🔐 Enter 2FA Password (Hint: {hint}):");

                print!("Password: ");
                io::stdout().flush()?;
                let password = rpassword::read_password()?;

                self.client
                    .check_password(PasswordToken::new(p), password.trim())
                    .await?;

                Ok(())
            }
            Err(e) => Err(anyhow!("Failed to get password info: {e}")),
        }
    }
}

fn input_peer_to_id(peer: &tl::enums::InputPeer) -> Option<i64> {
    match peer {
        tl::enums::InputPeer::Chat(chat) => Some(-chat.chat_id),
        tl::enums::InputPeer::Channel(channel) => Some(-CHANNEL_ID_OFFSET - channel.channel_id),
        tl::enums::InputPeer::User(user) => Some(user.user_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_peer_to_id_channel() {
        let peer = tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id: 7,
            access_hash: 0,
        });

        assert_eq!(input_peer_to_id(&peer), Some(-CHANNEL_ID_OFFSET - 7));
    }

    #[test]
    fn test_input_peer_to_id_chat() {
        let peer = tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: 42 });

        assert_eq!(input_peer_to_id(&peer), Some(-42));
    }

    #[test]
    fn test_input_peer_to_id_ignores_other_variants() {
        assert_eq!(input_peer_to_id(&tl::enums::InputPeer::Empty), None);
        assert_eq!(input_peer_to_id(&tl::enums::InputPeer::PeerSelf), None);
    }

    #[test]
    fn test_input_peer_to_id_user() {
        let peer = tl::enums::InputPeer::User(tl::types::InputPeerUser {
            user_id: 5,
            access_hash: 0,
        });

        assert_eq!(input_peer_to_id(&peer), Some(5));
    }
}
