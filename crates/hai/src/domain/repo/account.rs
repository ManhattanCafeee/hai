use serde_json::Value;
use sqlx::PgPool;

use crate::{
    domain::{
        model::{Account, Platform},
        vo::{AccountId, IdentityId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct AccountRepo {
    pool: PgPool,
}

impl AccountRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn by_platform_external_id(
        &self,
        platform: Platform,
        external_id: &str,
    ) -> Result<Option<Account>> {
        sqlx::query_as::<_, Account>(
            "SELECT * FROM account WHERE platform = $1 AND external_id = $2 LIMIT 1",
        )
        .bind(platform.as_str())
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create(
        &self,
        platform: Platform,
        external_id: &str,
        meta: Option<Value>,
        last_active_at: jiff::Timestamp,
        created_at: jiff::Timestamp,
        updated_at: jiff::Timestamp,
    ) -> Result<Account> {
        sqlx::query_as::<_, Account>(
            "INSERT INTO account (platform, external_id, meta, last_active_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        )
        .bind(platform.as_str())
        .bind(external_id)
        .bind(meta)
        .bind(jiff_sqlx::Timestamp::from(last_active_at))
        .bind(jiff_sqlx::Timestamp::from(created_at))
        .bind(jiff_sqlx::Timestamp::from(updated_at))
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Adopts the bot account a database written before keyed bot accounts holds: one shared
    /// `(system, 'bot')` row becomes this channel's `(platform, external_id)`. `None` when there is no
    /// such row (every database this build created, and every later call).
    pub async fn adopt_legacy_bot_account(
        &self,
        platform: Platform,
        external_id: &str,
    ) -> Result<Option<Account>> {
        sqlx::query_as::<_, Account>(
            "UPDATE account SET platform = $1, external_id = $2 \
             WHERE platform = 'system' AND external_id = 'bot' RETURNING *",
        )
        .bind(platform.as_str())
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn update_last_active_at(&self, id: AccountId, at: jiff::Timestamp) -> Result<()> {
        sqlx::query("UPDATE account SET last_active_at = $2 WHERE id = $1")
            .bind(id.0)
            .bind(jiff_sqlx::Timestamp::from(at))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn by_id(&self, id: AccountId) -> Result<Option<Account>> {
        sqlx::query_as::<_, Account>("SELECT * FROM account WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn by_identity_id(&self, identity_id: IdentityId) -> Result<Vec<Account>> {
        sqlx::query_as::<_, Account>("SELECT * FROM account WHERE identity_id = $1")
            .bind(identity_id.0)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn bind_identity(
        &self,
        account_id: AccountId,
        identity_id: IdentityId,
    ) -> Result<()> {
        sqlx::query("UPDATE account SET identity_id = $2 WHERE id = $1")
            .bind(account_id.0)
            .bind(identity_id.0)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
