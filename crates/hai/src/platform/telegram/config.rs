//! The Telegram keys of one `[channel.<name>]` section. The shared config layer hands the section over
//! untouched, so this is where a Telegram channel's settings are read — and where a key that belongs to
//! no platform is rejected.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{AppError, Result, bail};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct TelegramConfig {
    pub bot_token: String,
    pub allowed_chat_ids: Vec<i64>,
    pub rich_message: bool,
}

impl TelegramConfig {
    /// Reads this platform's keys out of one `[channel.<name>]` section; an unknown key fails here, at
    /// startup, rather than at the first update.
    pub(super) fn parse(key: &str, settings: &HashMap<String, serde_json::Value>) -> Result<Self> {
        // The section name is the channel's identity in the store, where a `:` would make the
        // `platform:name` spelling ambiguous.
        if key.is_empty() || key.contains(':') {
            bail!(AppError::message(format!(
                "channel section '{key}' must not be empty or contain ':'"
            )));
        }
        let value = serde_json::Value::Object(settings.clone().into_iter().collect());
        let cfg: Self = serde_json::from_value(value)
            .map_err(|e| AppError::message(format!("channel '{key}': {e}")))?;
        if cfg.bot_token.trim().is_empty() {
            bail!(AppError::message(format!(
                "channel '{key}': bot-token is required"
            )));
        }
        if cfg.allowed_chat_ids.is_empty() {
            tracing::warn!(
                channel = key,
                "channel has no allowed-chat-ids; every incoming chat will be rejected"
            );
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn section(entries: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    /// A name that names no section, or that cannot be read back out of the stored spelling, is
    /// refused where the channel is built rather than stored as an ambiguous key.
    #[test]
    fn a_section_name_that_cannot_be_stored_is_refused() {
        let settings = section(&[("bot-token", json!("a-token"))]);
        for bad in ["", "a:b", "main:"] {
            assert!(
                TelegramConfig::parse(bad, &settings).is_err(),
                "{bad:?} must not pass"
            );
        }
    }

    #[test]
    fn this_platforms_own_keys_are_read() {
        let cfg = TelegramConfig::parse(
            "telegram",
            &section(&[
                ("bot-token", json!("a-token")),
                ("allowed-chat-ids", json!([1, 2])),
                ("rich-message", json!(true)),
            ]),
        )
        .expect("a readable section");

        assert_eq!(cfg.bot_token, "a-token");
        assert_eq!(cfg.allowed_chat_ids, vec![1, 2]);
        assert!(cfg.rich_message);
    }

    /// A key no platform claims is a typo, and a typo has to fail rather than be ignored.
    #[test]
    fn an_unknown_key_is_refused() {
        let err = TelegramConfig::parse("telegram", &section(&[("bot-tokn", json!("a-token"))]))
            .unwrap_err();
        assert!(
            err.to_string().contains("channel 'telegram'"),
            "the section is named in the error: {err}"
        );
    }

    #[test]
    fn a_missing_token_fails_startup() {
        assert!(
            TelegramConfig::parse("telegram", &section(&[("allowed-chat-ids", json!([1]))]))
                .is_err()
        );
        assert!(
            TelegramConfig::parse(
                "telegram",
                &section(&[
                    ("bot-token", json!("   ")),
                    ("allowed-chat-ids", json!([1]))
                ]),
            )
            .is_err()
        );
    }

    /// An empty allowlist warns rather than failing: the channel starts and then rejects every chat.
    #[test]
    fn an_empty_allowlist_only_warns() {
        let cfg = TelegramConfig::parse("telegram", &section(&[("bot-token", json!("a-token"))]))
            .expect("a readable section");
        assert!(cfg.allowed_chat_ids.is_empty());
    }
}
