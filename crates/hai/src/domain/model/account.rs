use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::domain::vo::{AccountId, IdentityId, PlatformAccountMeta, enum_column};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    pub id: AccountId,
    pub identity_id: Option<IdentityId>,

    pub platform: Platform,
    pub external_id: String,
    pub meta: Option<serde_json::Value>,

    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub last_active_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

impl Account {
    /// The platform's own record of this account, when the row carries one this build can read.
    pub fn platform_meta(&self) -> Option<PlatformAccountMeta> {
        PlatformAccountMeta::deserialize(self.meta.as_ref()?).ok()
    }
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
pub enum Platform {
    Telegram,
}

impl Platform {
    /// The column value, the cache-key prefix and the serde name are this one spelling.
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}

enum_column!(Platform);
