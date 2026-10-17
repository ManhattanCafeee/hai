use std::sync::Arc;

use crate::{
    agentcore::Language,
    config::AppConfig,
    domain::{service::Services, vo::ChatId},
    error::Result,
};

/// The settings a chat runs with: the configured default, overridden by the chat's own stored
/// preference where one exists. Absence is a value — a reset restores the default.
#[derive(Clone)]
pub struct Settings {
    cfg: Arc<AppConfig>,
    services: Services,
}

impl Settings {
    pub fn new(cfg: Arc<AppConfig>, services: Services) -> Self {
        Self { cfg, services }
    }

    pub async fn language(&self, chat_id: ChatId) -> Result<Language> {
        let prefs = self.services.preference.get(chat_id).await?;
        Ok(prefs
            .unwrap_or_default()
            .language_or(self.cfg.agent.language))
    }

    pub async fn set_language(&self, chat_id: ChatId, language: Language) -> Result<()> {
        self.services
            .preference
            .set_language(chat_id, language)
            .await
    }

    pub async fn reset_language(&self, chat_id: ChatId) -> Result<()> {
        self.services.preference.reset(chat_id).await
    }
}
