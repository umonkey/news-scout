use anyhow::{Result, anyhow};

pub struct TelegramBot {
    token: String,
    chat_id: i64,
    thread_id: Option<i64>,
}

#[derive(serde::Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    parse_mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<i64>,
}

impl TelegramBot {
    pub fn new(token: String, chat_id: i64, thread_id: Option<i64>) -> Self {
        Self {
            token,
            chat_id,
            thread_id,
        }
    }

    pub async fn send_message(&self, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);

        let request = SendMessageRequest {
            chat_id: self.chat_id,
            text,
            parse_mode: "HTML",
            message_thread_id: self.thread_id,
        };

        let response = reqwest::Client::new()
            .post(url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();

            return Err(anyhow!("Telegram error: {error_text}"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_message_request_includes_thread_id() {
        let bot = TelegramBot::new("t".into(), -1, Some(7));

        let request = SendMessageRequest {
            chat_id: bot.chat_id,
            text: "hello",
            parse_mode: "HTML",
            message_thread_id: bot.thread_id,
        };

        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(value["message_thread_id"], 7);
    }

    #[test]
    fn test_send_message_request_omits_thread_id_when_absent() {
        let bot = TelegramBot::new("t".into(), -1, None);

        let request = SendMessageRequest {
            chat_id: bot.chat_id,
            text: "hello",
            parse_mode: "HTML",
            message_thread_id: bot.thread_id,
        };

        let value = serde_json::to_value(&request).unwrap();

        assert!(value.get("message_thread_id").is_none());
    }
}
