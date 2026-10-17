use serde_json::Value;
use sqlx::PgPool;

use crate::{
    domain::{model::ChatPreference, vo::ChatId},
    error::Result,
};

#[derive(Debug, Clone)]
pub struct ChatPreferenceRepo {
    pool: PgPool,
}

impl ChatPreferenceRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, chat_id: ChatId) -> Result<Option<ChatPreference>> {
        sqlx::query_as::<_, ChatPreference>("SELECT * FROM chat_preference WHERE chat_id = $1")
            .bind(chat_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn upsert(&self, chat_id: ChatId, prefs: &Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO chat_preference (chat_id, prefs, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (chat_id) DO UPDATE SET prefs = EXCLUDED.prefs, updated_at = now()",
        )
        .bind(chat_id)
        .bind(prefs)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, chat_id: ChatId) -> Result<()> {
        sqlx::query("DELETE FROM chat_preference WHERE chat_id = $1")
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
