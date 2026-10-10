use anyhow::{Context, Result, anyhow};
use std::str::FromStr;
use std::{env, fs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportChatId {
    pub chat_id: i64,
    pub thread_id: Option<i64>,
}

impl std::str::FromStr for ReportChatId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let (chat_id, thread_id) = match value.split_once(':') {
            Some((chat_id, thread_id)) => (
                chat_id.trim().parse::<i64>()?,
                Some(thread_id.trim().parse::<i64>()?),
            ),
            None => (value.trim().parse::<i64>()?, None),
        };

        Ok(Self { chat_id, thread_id })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub api_id: i32,
    pub api_hash: String,
    pub session_file: String,
    pub database_path: String,
    pub sentry_dsn: Option<String>,
    pub openrouter_api_key: String,
    pub openrouter_model: String,
    pub report_chat_id: ReportChatId,
    pub report_token: String,
    pub exclude_from: Vec<i64>,
    pub max_message_age: i64,
    pub news_folder: String,
    pub prompt: String,
}

fn default_database_path() -> String {
    "var/history.db".to_string()
}

fn default_openrouter_model() -> String {
    "google/gemini-3.7-flash".to_string()
}

fn default_max_message_age() -> i64 {
    604800
}

fn default_news_folder() -> String {
    "News".to_string()
}

fn default_session_file() -> String {
    "var/session.dat".to_string()
}

const DEFAULT_PROMPT_FILE: &str = "analyze-prompt.md";

impl Config {
    pub fn excludes_from(&self, sender_id: i64) -> bool {
        self.exclude_from.contains(&sender_id)
    }

    pub fn load() -> Result<Self> {
        Ok(Self {
            api_id: require_parsed("API_ID")?,
            api_hash: require_value("API_HASH")?,
            session_file: get_value("SESSION_FILE")?.unwrap_or_else(default_session_file),
            database_path: get_value("DATABASE_PATH")?.unwrap_or_else(default_database_path),
            sentry_dsn: get_value("SENTRY_DSN")?,
            openrouter_api_key: require_value("OPENROUTER_API_KEY")?,
            openrouter_model: get_value("OPENROUTER_MODEL")?
                .unwrap_or_else(default_openrouter_model),
            report_chat_id: require_parsed("REPORT_CHAT_ID")?,
            report_token: require_value("REPORT_TOKEN")?,
            exclude_from: get_ints("EXCLUDE_FROM")?,
            max_message_age: get_parsed("MAX_MESSAGE_AGE")?.unwrap_or_else(default_max_message_age),
            news_folder: get_value("NEWS_FOLDER")?.unwrap_or_else(default_news_folder),
            prompt: resolve_prompt(get_value("NEWS_SCOUT_PROMPT")?, |path| {
                fs::read_to_string(path).map_err(anyhow::Error::from)
            })?,
        })
    }
}

fn get_ints(name: &str) -> Result<Vec<i64>> {
    let Some(raw) = get_value(name)? else {
        return Ok(Vec::new());
    };

    parse_ints(&raw)
}

fn get_parsed<T>(name: &str) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    get_value(name)?
        .map(|raw| {
            raw.parse::<T>()
                .map_err(|e| anyhow!("{name} is invalid: {e}"))
        })
        .transpose()
}

fn get_value(name: &str) -> Result<Option<String>> {
    get_value_with(
        name,
        |key| env::var(key).ok(),
        |path| fs::read_to_string(path).map_err(anyhow::Error::from),
    )
}

fn get_value_with(
    name: &str,
    lookup: impl Fn(&str) -> Option<String>,
    read_file: impl Fn(&str) -> Result<String>,
) -> Result<Option<String>> {
    let file_var = format!("{name}_FILE");

    if let Some(path) = non_empty(lookup(&file_var)) {
        let contents =
            read_file(&path).with_context(|| format!("cannot read {file_var} file at {path}"))?;

        return Ok(Some(contents.trim().to_string()));
    }

    Ok(non_empty(lookup(name)))
}

/// Resolves the system prompt from an optional inline/file value, falling back
/// to reading [`DEFAULT_PROMPT_FILE`] from the current directory. A missing
/// prompt is a hard error.
fn resolve_prompt(
    value: Option<String>,
    read_file: impl Fn(&str) -> Result<String>,
) -> Result<String> {
    match value {
        Some(prompt) => Ok(prompt),
        None => read_file(DEFAULT_PROMPT_FILE).with_context(|| {
            format!(
                "cannot read {DEFAULT_PROMPT_FILE} (expected in the current directory; \
                 set NEWS_SCOUT_PROMPT or NEWS_SCOUT_PROMPT_FILE to override)"
            )
        }),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn parse_ints(raw: &str) -> Result<Vec<i64>> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i64>()
                .with_context(|| format!("invalid integer: {value}"))
        })
        .collect()
}

fn require_parsed<T>(name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let raw = require_value(name)?;

    raw.parse::<T>()
        .map_err(|e| anyhow!("{name} is invalid: {e}"))
}

fn require_value(name: &str) -> Result<String> {
    get_value(name)?.ok_or_else(|| anyhow!("{name} or {name}_FILE must be set"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_get_value_with_env_only() {
        let mut lookup: HashMap<String, String> = HashMap::new();
        lookup.insert("NAME".to_string(), "value".to_string());

        let value = get_value_with(
            "NAME",
            |key| lookup.get(key).cloned(),
            |_| Ok(String::new()),
        )
        .unwrap();

        assert_eq!(value, Some("value".to_string()));
    }

    #[test]
    fn test_get_value_with_empty_env_is_unset() {
        let mut lookup: HashMap<String, String> = HashMap::new();
        lookup.insert("NAME".to_string(), String::new());

        let value = get_value_with(
            "NAME",
            |key| lookup.get(key).cloned(),
            |_| Ok(String::new()),
        )
        .unwrap();

        assert_eq!(value, None);
    }

    #[test]
    fn test_get_value_with_file_error_does_not_fall_back() {
        let mut lookup: HashMap<String, String> = HashMap::new();
        lookup.insert("NAME".to_string(), "env".to_string());
        lookup.insert("NAME_FILE".to_string(), "/missing".to_string());

        let result = get_value_with(
            "NAME",
            |key| lookup.get(key).cloned(),
            |_| Err(anyhow!("boom")),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_get_value_with_file_wins_over_env() {
        let mut lookup: HashMap<String, String> = HashMap::new();
        lookup.insert("NAME".to_string(), "env".to_string());
        lookup.insert("NAME_FILE".to_string(), "/run/secrets/name".to_string());

        let read_file = |path: &str| {
            assert_eq!(path, "/run/secrets/name");

            Ok("trimmed\n".to_string())
        };

        let value = get_value_with("NAME", |key| lookup.get(key).cloned(), read_file).unwrap();

        assert_eq!(value, Some("trimmed".to_string()));
    }

    #[test]
    fn test_get_value_with_neither_set_is_none() {
        let lookup: HashMap<String, String> = HashMap::new();

        let value = get_value_with(
            "NAME",
            |key| lookup.get(key).cloned(),
            |_| Ok(String::new()),
        )
        .unwrap();

        assert_eq!(value, None);
    }

    #[test]
    fn test_non_empty() {
        assert_eq!(non_empty(Some(String::new())), None);
        assert_eq!(non_empty(Some("x".to_string())), Some("x".to_string()));
    }

    #[test]
    fn test_parse_ints_defaults_to_empty() {
        assert_eq!(parse_ints("").unwrap(), Vec::<i64>::new());
    }

    #[test]
    fn test_parse_ints_parses_comma_separated_values() {
        assert_eq!(parse_ints("111, 222").unwrap(), vec![111, 222]);
    }

    #[test]
    fn test_parse_ints_rejects_invalid_values() {
        assert!(parse_ints("abc").is_err());
    }

    #[test]
    fn test_report_chat_id_parses_chat_id_and_thread_id() {
        assert_eq!(
            "-12345:7".parse::<ReportChatId>().unwrap(),
            ReportChatId {
                chat_id: -12345,
                thread_id: Some(7),
            }
        );
    }

    #[test]
    fn test_report_chat_id_parses_plain_chat_id() {
        assert_eq!(
            "1".parse::<ReportChatId>().unwrap(),
            ReportChatId {
                chat_id: 1,
                thread_id: None,
            }
        );
    }

    #[test]
    fn test_report_chat_id_rejects_invalid_values() {
        for value in ["abc", "-1:x", "-1:"] {
            assert!(value.parse::<ReportChatId>().is_err());
        }
    }

    #[test]
    fn test_report_chat_id_tolerates_surrounding_whitespace() {
        assert_eq!(
            "-12345 : 7".parse::<ReportChatId>().unwrap(),
            ReportChatId {
                chat_id: -12345,
                thread_id: Some(7),
            }
        );
    }

    #[test]
    fn test_resolve_prompt_uses_inline_value() {
        let prompt = resolve_prompt(Some("inline prompt".to_string()), |_| {
            panic!("the default file must not be read when a prompt is set");
        })
        .unwrap();

        assert_eq!(prompt, "inline prompt");
    }

    #[test]
    fn test_resolve_prompt_reads_default_file_when_unset() {
        let prompt = resolve_prompt(None, |path| {
            assert_eq!(path, DEFAULT_PROMPT_FILE);

            Ok("file prompt".to_string())
        })
        .unwrap();

        assert_eq!(prompt, "file prompt");
    }

    #[test]
    fn test_resolve_prompt_missing_default_file_is_hard_error() {
        let result = resolve_prompt(None, |_| Err(anyhow!("cannot read: boom")));

        assert!(result.is_err());

        let error = result.unwrap_err().to_string();
        assert!(error.contains("analyze-prompt.md"));
        assert!(error.contains("NEWS_SCOUT_PROMPT"));
    }
}
