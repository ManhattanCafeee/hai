use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::domain::vo::{AccountId, ChatId, MemoryId, MessageId, enum_column};

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub status: MemoryStatus,
    pub chat_id: ChatId,
    pub account_id: Option<AccountId>,
    pub content: String,
    pub source_message_ids: Vec<MessageId>,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Memory {
    pub id: MemoryId,

    pub account_id: Option<AccountId>,

    pub chat_id: ChatId,

    pub kind: MemoryKind,
    pub content: String,
    pub source_message_ids: Vec<MessageId>,
    pub status: MemoryStatus,
    pub pinned: bool,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

#[derive(
    Debug,
    Clone,
    Copy,
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
pub enum MemoryKind {
    UserFact,
    Note,
    Knowledge,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}

enum_column!(MemoryKind, MemoryStatus);

#[derive(
    Debug,
    Clone,
    Copy,
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
pub enum MemoryStatus {
    Active,
    Invalidated,
}

impl MemoryStatus {
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}
