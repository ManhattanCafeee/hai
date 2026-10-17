use crate::domain::vo::IdentityId;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Identity {
    pub id: IdentityId,

    pub name: Option<String>,
    pub meta: Option<serde_json::Value>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}
