use crate::{
    agentcore::Language,
    domain::{
        repo::Repos,
        vo::{ChatId, ChatPreferences},
    },
    error::{AppError, Result},
};

#[derive(Debug)]
pub struct PreferenceService {
    repos: Repos,
}

impl PreferenceService {
    pub(crate) fn new(repos: Repos) -> Self {
        Self { repos }
    }

    /// Absence is a value: the chat inherits the configured default. A stored blob that cannot be
    /// read is an error — returning absence would let the next `set_language` overwrite it.
    pub async fn get(&self, chat_id: ChatId) -> Result<Option<ChatPreferences>> {
        let Some(row) = self.repos.chat_preference.get(chat_id).await? else {
            return Ok(None);
        };
        parse_blob(chat_id, row.prefs).map(Some)
    }

    /// Reads, changes and writes back, so a preference added later survives an update that does
    /// not mention it.
    pub async fn set_language(&self, chat_id: ChatId, language: Language) -> Result<()> {
        let prefs = self
            .get(chat_id)
            .await?
            .unwrap_or_default()
            .with_language(language);
        self.repos
            .chat_preference
            .upsert(chat_id, &prefs.to_value()?)
            .await
    }

    /// Back to the configured default: the row goes away so a later default change reaches this
    /// chat too.
    pub async fn reset(&self, chat_id: ChatId) -> Result<()> {
        self.repos.chat_preference.delete(chat_id).await
    }
}

/// A stored blob that does not decode is an error: absence means "no row", not "unreadable row".
fn parse_blob(chat_id: ChatId, value: serde_json::Value) -> Result<ChatPreferences> {
    serde_json::from_value(value)
        .map_err(|e| AppError::message(format!("chat {chat_id} has unreadable preferences: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_blob_is_an_error_not_absence() {
        let err = parse_blob(ChatId(3), serde_json::json!(5)).unwrap_err();
        assert!(err.to_string().contains("unreadable preferences"), "{err}");
    }
}
