use regex::Regex;
use std::sync::LazyLock;

pub const CHANNEL_ID_OFFSET: i64 = 1_000_000_000_000;

static KEYWORD_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "озеленен|благоустр|вырубк|дерев[оауь]|крониров|топпинг|полив|каначапатум|kanachapatum|канач ереван|kanach yerevan|kanachyerevan|umonkey|yeknomy|realumonkey",
    )
    .expect("keyword regex should compile")
});

pub fn format_message_link(
    username: Option<&str>,
    chat_id: i64,
    topic_id: Option<i64>,
    message_id: i64,
) -> String {
    match username {
        Some(username) => match topic_id {
            Some(topic_id) => format!("https://t.me/{username}/{topic_id}/{message_id}"),
            None => format!("https://t.me/{username}/{message_id}"),
        },
        None => {
            let chat_id_short = chat_id.abs() % CHANNEL_ID_OFFSET;

            match topic_id {
                Some(topic_id) => format!("https://t.me/c/{chat_id_short}/{topic_id}/{message_id}"),
                None => format!("https://t.me/c/{chat_id_short}/{message_id}"),
            }
        }
    }
}

pub fn matches_keywords(text: &str) -> bool {
    KEYWORD_PATTERN.is_match(&text.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_message_link_with_username() {
        let result = format_message_link(Some("username"), -1001234567890, None, 42);
        assert_eq!(result, "https://t.me/username/42");
    }

    #[test]
    fn test_format_message_link_without_username() {
        let result = format_message_link(None, -1001234567890, None, 42);
        assert_eq!(result, "https://t.me/c/1234567890/42");
    }

    #[test]
    fn test_format_message_link_with_positive_id() {
        let result = format_message_link(None, 1234567890, None, 42);
        assert_eq!(result, "https://t.me/c/1234567890/42");
    }

    #[test]
    fn test_format_message_link_with_topic_and_username() {
        let result = format_message_link(Some("username"), -1001234567890, Some(120), 42);
        assert_eq!(result, "https://t.me/username/120/42");
    }

    #[test]
    fn test_format_message_link_with_topic_without_username() {
        let result = format_message_link(None, -1001234567890, Some(120), 42);
        assert_eq!(result, "https://t.me/c/1234567890/120/42");
    }

    #[test]
    fn test_matches_keywords_case_insensitive() {
        assert!(matches_keywords("KANACHAPATUM"));
    }

    #[test]
    fn test_matches_keywords_negative() {
        assert!(!matches_keywords("hello world"));
    }

    #[test]
    fn test_matches_keywords_positive() {
        assert!(matches_keywords("Озеленение двора"));
    }
}
