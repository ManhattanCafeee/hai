use jiff::Timestamp;
use serde_json::Value;

use crate::domain::vo::ChatId;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChatPreference {
    pub chat_id: ChatId,
    pub prefs: Value,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: Timestamp,
}
