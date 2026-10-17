use crate::domain::vo::ChatId;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationRecord {
    pub chat_id: ChatId,

    pub messages: serde_json::Value,

    pub context_meta: serde_json::Value,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}
