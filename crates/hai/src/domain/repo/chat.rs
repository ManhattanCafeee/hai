use sqlx::PgPool;

use crate::{
    domain::{
        model::{Chat, NewChat, Platform},
        vo::ChatId,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct ChatRepo {
    pool: PgPool,
}

impl ChatRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn by_platform_external_id(
        &self,
        platform: Platform,
        external_id: &str,
    ) -> Result<Option<Chat>> {
        sqlx::query_as::<_, Chat>(
            "SELECT * FROM chat WHERE platform = $1 AND external_id = $2 LIMIT 1",
        )
        .bind(platform.as_str())
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create(&self, new: NewChat) -> Result<Chat> {
        sqlx::query_as::<_, Chat>(
            "INSERT INTO chat (platform, external_id, chat_type, name, meta, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
        )
        .bind(new.platform.as_str())
        .bind(new.external_id)
        .bind(new.chat_type.as_str())
        .bind(new.name)
        .bind(new.meta)
        .bind(jiff_sqlx::Timestamp::from(new.created_at))
        .bind(jiff_sqlx::Timestamp::from(new.updated_at))
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn by_id(&self, id: ChatId) -> Result<Option<Chat>> {
        sqlx::query_as::<_, Chat>("SELECT * FROM chat WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }
}
