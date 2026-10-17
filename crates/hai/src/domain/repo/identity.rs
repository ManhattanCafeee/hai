use serde_json::Value;
use sqlx::PgPool;

use crate::{
    domain::{model::Identity, vo::IdentityId},
    error::Result,
};

#[derive(Debug, Clone)]
pub struct IdentityRepo {
    pool: PgPool,
}

impl IdentityRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        id: IdentityId,
        name: Option<&str>,
        meta: Option<Value>,
        created_at: jiff::Timestamp,
        updated_at: jiff::Timestamp,
    ) -> Result<Identity> {
        sqlx::query_as::<_, Identity>(
            "INSERT INTO identity (id, name, meta, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5) RETURNING *",
        )
        .bind(id.0)
        .bind(name)
        .bind(meta)
        .bind(jiff_sqlx::Timestamp::from(created_at))
        .bind(jiff_sqlx::Timestamp::from(updated_at))
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }
}
