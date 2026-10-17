#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub seq: i64,

    pub payload: serde_json::Value,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}
