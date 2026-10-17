use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::domain::{
    model::Platform,
    vo::{ChatId, enum_column},
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Chat {
    pub id: ChatId,

    pub platform: Platform,
    pub external_id: String,
    pub chat_type: ChatType,
    pub name: Option<String>,
    pub meta: Option<serde_json::Value>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

/// The payload one chat row is written from.
#[derive(Debug, Clone)]
pub struct NewChat {
    pub platform: Platform,
    pub external_id: String,
    pub chat_type: ChatType,
    pub name: Option<String>,
    pub meta: Option<serde_json::Value>,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Display,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    Private,
    Group,
    Channel,
}

enum_column!(ChatType);

impl ChatType {
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}
