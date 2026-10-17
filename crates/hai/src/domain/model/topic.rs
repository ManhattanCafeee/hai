use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use super::*;
use crate::domain::vo::{ChatId, MessageId, TopicId, enum_column};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Topic {
    pub id: TopicId,
    pub chat_id: ChatId,

    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: TopicStatus,

    pub first_message_id: Option<MessageId>,
    pub last_message_id: Option<MessageId>,

    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub started_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub last_active_at: jiff::Timestamp,
    #[sqlx(try_from = "SqlxNullableTimestamp")]
    pub closed_at: Option<jiff::Timestamp>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicNote {
    pub id: i64,
    pub topic_id: TopicId,
    pub text: String,
    pub message_ids: Vec<MessageId>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TopicStatus {
    Active,
    Closed,
}

impl TopicStatus {
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}

enum_column!(TopicStatus);

/// The text that gets embedded: title and summary; notes stay out of the search surface.
pub(crate) fn embed_text(title: &str, summary: &str) -> String {
    format!("{title}\n{summary}")
}
