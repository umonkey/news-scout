use crate::domains::folder::FolderFilter;
use crate::domains::message::Message as StoredMessage;
use crate::domains::message::MessageRepository;
use crate::domains::message::utils::{CHANNEL_ID_OFFSET, format_message_link, matches_keywords};
use crate::domains::news_candidates::NewsCandidateRepository;
use crate::domains::peer::PeerRepository;
use crate::infra::config::Config;
use crate::infra::db::Db;
use crate::infra::telegram::TelegramClient;
use crate::infra::util;
use crate::infra::util::ShutdownRx;
use anyhow::Result;
use futures::FutureExt;
use grammers_client::message::Message as ClientMessage;
use grammers_client::peer::Peer as ClientPeer;
use grammers_client::update::Message as UpdateMessage;
use grammers_client::update::MessageDeletion;
use grammers_client::update::Update;
use grammers_session::types::PeerRef;
use grammers_tl_types as tl;
use std::time::Duration;

const UPDATE_QUEUE_LIMIT: usize = 1000;
const MAINTENANCE_INTERVAL_SECS: u64 = 3600;
/// Safety cap on how many messages a single channel backfill may process in one
/// startup, so a very long downtime cannot stall the daemon indefinitely.
const BACKFILL_MESSAGE_CAP: usize = 2000;

pub struct ReaderService {
    config: Config,
    folder: FolderFilter,
    message_repo: MessageRepository,
    candidate_repo: NewsCandidateRepository,
    peer_repo: PeerRepository,
    client: TelegramClient,
}

impl ReaderService {
    pub async fn new(config: Config) -> Result<Self> {
        let db = Db::new(&config.database_path).await?;
        let message_repo = MessageRepository::new(db.connection.clone());
        let candidate_repo = NewsCandidateRepository::new(db.connection.clone());
        let peer_repo = PeerRepository::new(db.connection.clone());
        let folder = FolderFilter::default();

        let client = TelegramClient::new(
            config.api_id,
            config.api_hash.clone(),
            config.session_file.clone(),
        )
        .await?;

        Ok(Self {
            config,
            folder,
            message_repo,
            candidate_repo,
            peer_repo,
            client,
        })
    }

    pub async fn run(&mut self, shutdown: &mut ShutdownRx) -> Result<()> {
        let runner_handle = self
            .client
            .connect()
            .ok_or_else(|| anyhow::anyhow!("Client already connected or runner missing"))?;

        self.client.check_authorization().await?;

        if let Err(e) = self.client.disable_incoming_requests().await {
            log::warn!("Could not disable incoming calls/secret chats for this session: {e}");
        }

        let mut runner_handle = runner_handle.fuse();

        // The update stream must be created before `seed_peers()`: creating it
        // snapshots the saved per-channel `pts` from the session into memory so
        // the built-in catch-up can request the missed differences. `seed_peers()`
        // uses `iter_dialogs()`, which overwrites each channel's saved `pts`
        // (INSERT OR REPLACE) with its current value; doing that first would
        // clobber the state the catch-up relies on.
        let mut stream = self.client.stream_updates(true, UPDATE_QUEUE_LIMIT).await?;

        self.refresh_folder().await;

        self.seed_peers().await?;

        let mut maintenance = tokio::time::interval(Duration::from_secs(MAINTENANCE_INTERVAL_SECS));
        maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        maintenance.tick().await; // consume the immediate first tick

        loop {
            let mut folder_changed = false;
            let mut sync_cursor = false;
            let mut should_exit = false;

            tokio::select! {
                res = &mut runner_handle => {
                    match res {
                        Ok(_) => return Err(anyhow::anyhow!("Telegram background runner exited unexpectedly")),
                        Err(e) => return Err(anyhow::anyhow!("Telegram background runner panicked: {}", e)),
                    }
                }
                update = stream.next() => {
                    match update {
                        Ok(Update::NewMessage(message)) => {
                            if let Err(e) = self.handle_new_message(&message).await {
                                log::error!("Error handling new message: {e}");
                                sentry::capture_error(&*e);
                            }
                        }
                        Ok(Update::MessageEdited(message)) => {
                            if let Err(e) = self.handle_edited_message(&message).await {
                                log::error!("Error handling edited message: {e}");
                                sentry::capture_error(&*e);
                            }
                        }
                        Ok(Update::MessageDeleted(deletion)) => {
                            if let Err(e) = self.handle_message_deletion(&deletion).await {
                                log::error!("Error handling message deletion: {e}");
                                sentry::capture_error(&*e);
                            }
                        }
                        Ok(other) if is_folder_update(&other) => {
                            folder_changed = true;
                        }
                        Ok(other) if is_noisy_update(&other) => {}
                        Ok(other) => {
                            log::debug!("Ignoring update: {other:?}");
                        }
                        Err(e) => {
                            if e.is("AUTH_KEY_DUPLICATED") {
                                log::error!(
                                    "FATAL: Telegram session has been invalidated (AUTH_KEY_DUPLICATED). This usually happens when the same session is used on multiple machines. Please remove the session file and re-authenticate the bot. Process will terminate."
                                );

                                return Err(crate::services::supervisor::FatalError(anyhow::anyhow!(
                                    "Telegram session invalidated (AUTH_KEY_DUPLICATED)"
                                ))
                                .into());
                            }

                            log::error!("Error receiving update: {e}");
                            sentry::capture_error(&e);
                        }
                    }
                }
                _ = maintenance.tick() => {
                    folder_changed = true;
                    sync_cursor = true;
                }
                _ = util::wait_shutdown(shutdown) => {
                    should_exit = true;
                }
            }

            if should_exit {
                log::info!("Shutdown signal received, persisting update state.");
                stream.sync_update_state().await;
                return Ok(());
            }

            if folder_changed {
                self.refresh_folder().await;
            }

            if sync_cursor {
                stream.sync_update_state().await;

                match self.message_repo.cleanup(self.config.max_message_age).await {
                    Ok(deleted) if deleted > 0 => log::info!("Deleted {deleted} old messages."),
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("Cleanup failed: {e}");
                        sentry::capture_error(&*e);
                    }
                }
            }
        }
    }

    async fn seed_peers(&mut self) -> Result<()> {
        log::info!("Listing dialogs to seed peer cache...");

        let mut dialogs = self.client.iter_dialogs();

        while let Some(dialog) = dialogs.next().await? {
            let peer = dialog.peer();

            if matches!(peer, ClientPeer::User(_)) {
                continue;
            }

            let id = self.save_peer(peer).await?;

            // Backfill only chats that belong to the monitored folder; for the
            // rest we merely cache peers.
            if self.folder.contains(id)
                && let Err(e) = self
                    .backfill_channel(dialog.peer(), dialog.peer_ref())
                    .await
            {
                let chat_name = dialog.peer().name().unwrap_or("Unknown");

                log::error!("Backfill failed for '{chat_name}' ({id}): {e}");
                sentry::capture_error(&*e);
            }
        }

        Ok(())
    }

    /// Recovers messages that arrived while the process was down.
    ///
    /// Telegram's `getDifference` (via `catch_up`) only covers a bounded window;
    /// for longer outages it drops the gap (`Difference::TooLong`). Here we walk
    /// each monitored channel's history from newest to oldest until we reach the
    /// newest message already stored locally (the watermark), persisting and
    /// enqueuing everything in between through the same path as live messages.
    ///
    /// Chats with no stored message are skipped: they have no watermark, and
    /// backfilling them would flood the queue with the channel's entire history
    /// on first run.
    async fn backfill_channel(&mut self, chat_peer: &ClientPeer, peer_ref: PeerRef) -> Result<()> {
        let chat_id = self.get_peer_id(chat_peer);

        let Some(watermark) = self.message_repo.latest_message_id(chat_id).await? else {
            return Ok(());
        };

        let chat_name = chat_peer.name().unwrap_or("Unknown");

        log::info!("Backfilling '{chat_name}' ({chat_id}) since message id {watermark}...");

        let mut messages = self.client.iter_messages(peer_ref);
        let mut processed = 0usize;
        let mut capped = false;

        while let Some(message) = messages.next().await? {
            if (message.id() as i64) <= watermark {
                break;
            }

            if processed >= BACKFILL_MESSAGE_CAP {
                capped = true;
                break;
            }

            self.process_message(&message, chat_peer, chat_id).await?;
            processed += 1;
        }

        if capped {
            log::warn!(
                "Backfill for '{chat_name}' ({chat_id}) hit the {BACKFILL_MESSAGE_CAP}-message cap before reaching the watermark; some missed messages may not have been recovered."
            );
        }

        if processed > 0 {
            log::info!("Backfilled {processed} missed message(s) for '{chat_name}' ({chat_id}).");
        }

        Ok(())
    }

    async fn handle_edited_message(&mut self, message: &UpdateMessage) -> Result<()> {
        let Some(chat_peer) = message.peer() else {
            log::warn!("Received an edited message without a peer, skipping.");
            return Ok(());
        };

        if matches!(chat_peer, ClientPeer::User(_)) {
            log::debug!("Ignoring edited private message.");
            return Ok(());
        }

        let chat_id = self.get_peer_id(chat_peer);
        let link = self.message_link(chat_peer, chat_id, message);

        let updated = self
            .message_repo
            .update_message_text(message.id() as i64, chat_id, message.text().to_string())
            .await?;

        if updated == 0 {
            log::debug!("Edited message not found, skipping: {link}");
        } else {
            log::debug!("Updated text of message: {link}");
        }

        Ok(())
    }

    async fn handle_message_deletion(&self, deletion: &MessageDeletion) -> Result<()> {
        let chat_id = deletion
            .channel_id()
            .map(|channel_id| -CHANNEL_ID_OFFSET - channel_id);

        let mut deleted = 0;

        for &id in deletion.messages() {
            deleted += self.message_repo.delete_message(id as i64, chat_id).await?;
        }

        if deleted > 0 {
            log::info!("Deleted {deleted} message(s) after a Telegram deletion update.");
        }

        Ok(())
    }

    async fn handle_new_message(&mut self, message: &UpdateMessage) -> Result<()> {
        let Some(chat_peer) = message.peer() else {
            log::warn!("Received a message without a peer, skipping.");
            return Ok(());
        };

        if matches!(chat_peer, ClientPeer::User(_)) {
            log::debug!("Ignoring private message.");
            return Ok(());
        }

        let chat_id = self.get_peer_id(chat_peer);
        let link = self.message_link(chat_peer, chat_id, message);

        if !self.folder.contains(chat_id) {
            let chat_name = chat_peer.name().unwrap_or("Unknown");

            log::debug!(
                "Ignoring message {link} ({chat_name}) outside the '{}' folder.",
                self.config.news_folder
            );

            return Ok(());
        }

        self.process_message(message, chat_peer, chat_id).await?;

        self.mark_chat_read(message, chat_id).await;

        Ok(())
    }

    /// Persists a message and enqueues it as a candidate when it matches.
    ///
    /// Shared by live updates ([`Self::handle_new_message`]) and startup
    /// backfill ([`Self::backfill_channel`]). It never double-stores or
    /// double-enqueues because [`MessageRepository::save_message`] uses
    /// `INSERT OR IGNORE` and enqueueing only happens when a row was inserted.
    /// It does not mark the chat as read, so backfill cannot consume unread
    /// notifications.
    async fn process_message(
        &mut self,
        message: &ClientMessage,
        chat_peer: &ClientPeer,
        chat_id: i64,
    ) -> Result<()> {
        let link = self.message_link(chat_peer, chat_id, message);

        let chat_id = self.save_peer(chat_peer).await?;

        let (sender_id, sender_name) = match message.sender() {
            Some(sender_peer) => {
                let sender_id = self.get_peer_id(sender_peer);

                if self.config.excludes_from(sender_id) {
                    log::debug!("Ignoring message from excluded sender {sender_id}.");

                    return Ok(());
                }

                let id = self.save_peer(sender_peer).await?;

                (id, sender_peer.name().unwrap_or("Unknown").to_string())
            }
            None => (chat_id, chat_peer.name().unwrap_or("Unknown").to_string()),
        };

        let text = message.text().to_string();

        let stored = StoredMessage {
            id: message.id() as i64,
            chat_id,
            sender_id,
            timestamp: message.date().timestamp(),
            text: text.clone(),
            processed: 0,
            topic_id: message_topic_id(message),
        };

        let inserted = self.message_repo.save_message(stored).await?;

        log::debug!("Stored message: {link}");

        if inserted > 0 && matches_keywords(&text) && !message.outgoing() {
            let chat_name = chat_peer.name().unwrap_or("Unknown");

            self.candidate_repo
                .enqueue(chat_name, &sender_name, &link, &text)
                .await?;

            log::info!("Queued candidate: {link}");
        }

        Ok(())
    }

    async fn refresh_folder(&mut self) {
        let folder_name = self.config.news_folder.clone();

        match self.client.folder_peer_ids(&folder_name).await {
            Ok(Some(chat_ids)) => {
                log::info!(
                    "Folder '{}' resolved with {} chat(s).",
                    folder_name,
                    chat_ids.len()
                );

                self.folder.update(chat_ids);
            }
            Ok(None) => {
                log::error!(
                    "Folder '{}' not found; no messages will be processed until it exists.",
                    folder_name
                );

                self.folder.clear();
            }
            Err(e) => {
                log::error!("Failed to refresh folder '{}': {e}", folder_name);
                sentry::capture_error(&*e);
            }
        }
    }

    fn get_peer_id(&self, peer: &ClientPeer) -> i64 {
        peer.id().bot_api_dialog_id()
    }

    fn get_peer_name(&self, peer: &ClientPeer) -> Option<String> {
        match peer {
            ClientPeer::User(u) => {
                if let tl::enums::User::User(user) = &u.raw {
                    if let Some(usernames) = &user.usernames {
                        for un_enum in usernames {
                            let tl::enums::Username::Username(un) = un_enum;

                            if un.active {
                                return Some(un.username.clone());
                            }
                        }
                    }

                    return user.username.clone();
                }

                None
            }
            ClientPeer::Channel(c) => {
                if let Some(usernames) = &c.raw.usernames {
                    for un_enum in usernames {
                        let tl::enums::Username::Username(un) = un_enum;

                        if un.active {
                            return Some(un.username.clone());
                        }
                    }
                }

                c.raw.username.clone()
            }
            _ => None,
        }
    }

    async fn mark_chat_read(&self, message: &UpdateMessage, chat_id: i64) {
        let result = match message_topic_id(message) {
            Some(topic_id) => self.mark_topic_read(message, topic_id).await,
            None => message.mark_as_read().await.map_err(anyhow::Error::from),
        };

        if let Err(e) = result {
            log::warn!("Failed to mark chat {chat_id} as read: {e}");
        }
    }

    async fn mark_topic_read(&self, message: &UpdateMessage, topic_id: i64) -> Result<()> {
        let Some(peer_ref) = message.peer_ref().await else {
            return Ok(());
        };

        self.client
            .mark_discussion_read(peer_ref.into(), topic_id as i32, message.id())
            .await
    }

    fn message_link(
        &self,
        chat_peer: &ClientPeer,
        chat_id: i64,
        message: &ClientMessage,
    ) -> String {
        let username = self.get_peer_name(chat_peer);
        let topic_id = message_topic_id(message);

        format_message_link(username.as_deref(), chat_id, topic_id, message.id() as i64)
    }

    async fn save_peer(&mut self, peer: &ClientPeer) -> Result<i64> {
        let peer_id = self.get_peer_id(peer);
        let peer_name = peer.name().unwrap_or("Unknown").to_string();
        let username = self.get_peer_name(peer);

        self.peer_repo
            .save_peer(crate::domains::peer::Peer {
                id: peer_id,
                name: peer_name.clone(),
                username: username.clone(),
            })
            .await?;

        log::trace!(
            "Peer {peer_id} updated, username={} name={peer_name}",
            username.as_deref().unwrap_or("(null)")
        );

        Ok(peer_id)
    }
}

fn is_folder_update(update: &Update) -> bool {
    matches!(
        update.raw(),
        tl::enums::Update::FolderPeers(_)
            | tl::enums::Update::DialogFilter(_)
            | tl::enums::Update::DialogFilters
    )
}

fn is_noisy_update(update: &Update) -> bool {
    matches!(
        update.raw(),
        tl::enums::Update::UserStatus(_)
            | tl::enums::Update::UserTyping(_)
            | tl::enums::Update::ChatUserTyping(_)
            | tl::enums::Update::ChannelUserTyping(_)
            | tl::enums::Update::ReadHistoryInbox(_)
            | tl::enums::Update::ReadHistoryOutbox(_)
            | tl::enums::Update::ReadChannelInbox(_)
            | tl::enums::Update::ReadChannelOutbox(_)
            | tl::enums::Update::ReadMessagesContents(_)
            | tl::enums::Update::ChannelReadMessagesContents(_)
            | tl::enums::Update::ReadChannelDiscussionInbox(_)
            | tl::enums::Update::ReadChannelDiscussionOutbox(_)
            | tl::enums::Update::MessagePoll(_)
            | tl::enums::Update::ChannelMessageViews(_)
            | tl::enums::Update::PeerSettings(_)
            | tl::enums::Update::Privacy(_)
            | tl::enums::Update::DraftMessage(_)
            | tl::enums::Update::MessageReactions(_)
            | tl::enums::Update::WebPage(_)
            | tl::enums::Update::ChannelWebPage(_)
            | tl::enums::Update::PinnedChannelMessages(_)
            | tl::enums::Update::Story(_)
    )
}

fn message_topic_id(message: &ClientMessage) -> Option<i64> {
    let tl::enums::MessageReplyHeader::Header(header) = message.reply_header()? else {
        return None;
    };

    if !header.forum_topic {
        return None;
    }

    header
        .reply_to_top_id
        .or(header.reply_to_msg_id)
        .map(i64::from)
}
