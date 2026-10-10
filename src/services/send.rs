use crate::domains::news_candidates::{NewsCandidate, NewsCandidateRepository};
use crate::infra::config::Config;
use crate::infra::db::Db;
use crate::infra::telegram_bot::TelegramBot;
use crate::infra::util;
use crate::infra::util::ShutdownRx;
use anyhow::{Result, anyhow};
use rig::completion::{AssistantContent, CompletionModel, Message};
use rig::prelude::*;
use rig::providers::openrouter;

pub struct SendService {
    candidate_repo: NewsCandidateRepository,
    bot: TelegramBot,
    model: openrouter::CompletionModel,
    prompt: String,
}

impl SendService {
    pub async fn new(config: Config) -> Result<Self> {
        let db = Db::new(&config.database_path).await?;
        let candidate_repo = NewsCandidateRepository::new(db.connection);

        // Pin the default HTTP client: the OpenRouter provider threads its `H`
        // type parameter through `completion_model`, so it cannot be inferred.
        let client: openrouter::Client = openrouter::Client::new(&config.openrouter_api_key)?;
        let model = client.completion_model(config.openrouter_model.as_str());

        let bot = TelegramBot::new(
            config.report_token.clone(),
            config.report_chat_id.chat_id,
            config.report_chat_id.thread_id,
        );

        Ok(Self {
            candidate_repo,
            bot,
            model,
            prompt: config.prompt,
        })
    }

    pub async fn run(&self, shutdown: &mut ShutdownRx) -> Result<()> {
        loop {
            let candidate = match self.candidate_repo.next().await {
                Ok(next) => next,
                Err(e) => {
                    log::error!("Failed to read the candidate queue: {e}");
                    sentry::capture_error(&*e);

                    if !self.wait(60, shutdown).await {
                        return Ok(());
                    }

                    continue;
                }
            };

            let Some(candidate) = candidate else {
                if !self.wait(5, shutdown).await {
                    return Ok(());
                }

                continue;
            };

            match self.process(&candidate).await {
                Ok(()) => {
                    if let Err(e) = self.candidate_repo.delete(candidate.id).await {
                        log::error!("Failed to delete candidate {}: {e}", candidate.id);
                        sentry::capture_error(&*e);

                        if !self.wait(60, shutdown).await {
                            return Ok(());
                        }

                        continue;
                    }

                    log::info!("Sent candidate {} ({})", candidate.id, candidate.url);

                    if !self.wait(5, shutdown).await {
                        return Ok(());
                    }
                }
                Err(e) => {
                    log::error!("Failed to process candidate {}: {e}", candidate.id);
                    sentry::capture_error(&*e);

                    if !self.wait(60, shutdown).await {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn process(&self, candidate: &NewsCandidate) -> Result<()> {
        let response = self
            .model
            .completion_request(Message::user(build_user_message(candidate)))
            .preamble(self.prompt.clone())
            .send()
            .await?;

        let text = response
            .choice
            .iter()
            .filter_map(|content| match content {
                AssistantContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if is_ignore(&text) {
            log::info!(
                "Dropping ignored candidate {} ({})",
                candidate.id,
                candidate.url
            );

            return Ok(());
        }

        if text.trim().is_empty() {
            return Err(anyhow!(
                "model returned empty text for candidate {}",
                candidate.id
            ));
        }

        let message = format!("{}\n\n{}", text.trim(), candidate.url);

        self.bot.send_message(&message).await?;

        Ok(())
    }

    /// Waits for the given number of seconds, returning `false` when a
    /// shutdown signal arrives instead.
    async fn wait(&self, seconds: u64, shutdown: &mut ShutdownRx) -> bool {
        tokio::select! {
            _ = util::sleep(seconds) => true,
            _ = util::wait_shutdown(shutdown) => false,
        }
    }
}

fn is_ignore(response: &str) -> bool {
    response.trim().eq_ignore_ascii_case("[IGNORE]")
}

fn build_user_message(candidate: &NewsCandidate) -> String {
    format!(
        "Chat: {}\nSender: {}\nText: {}\nSource: {}",
        candidate.chat_name, candidate.sender_name, candidate.text, candidate.url
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> NewsCandidate {
        NewsCandidate {
            id: 1,
            chat_name: "Chat Name".to_string(),
            sender_name: "Sender Name".to_string(),
            url: "https://t.me/example/1".to_string(),
            text: "Some message text".to_string(),
            created_at: 0,
        }
    }

    #[test]
    fn test_is_ignore_matches_whole_reply() {
        assert!(is_ignore("[IGNORE]"));
        assert!(is_ignore(" [IGNORE] "));
        assert!(is_ignore("[ignore]"));
    }

    #[test]
    fn test_is_ignore_rejects_other_replies() {
        assert!(!is_ignore("some text"));
        assert!(!is_ignore("[IGNORE] extra"));
    }

    #[test]
    fn test_build_user_message_contains_candidate_fields() {
        let message = build_user_message(&candidate());

        assert!(message.contains("Chat Name"));
        assert!(message.contains("Sender Name"));
        assert!(message.contains("Some message text"));
        assert!(message.contains("https://t.me/example/1"));
    }
}
