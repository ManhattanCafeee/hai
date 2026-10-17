use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "platform")]
pub enum PlatformAccountMeta {
    Telegram(TelegramAccountMeta),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramAccountMeta {
    pub first_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl PlatformAccountMeta {
    pub fn full_name(&self) -> String {
        match self {
            PlatformAccountMeta::Telegram(v) => match &v.last_name {
                Some(last) => format!("{} {}", v.first_name, last),
                None => v.first_name.clone(),
            },
        }
    }

    pub fn username(&self) -> Option<String> {
        match self {
            PlatformAccountMeta::Telegram(v) => v.username.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessageMeta {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageMeta {
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformMessageMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmMessageMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "platform")]
pub enum PlatformMessageMeta {
    Telegram(TelegramMessageMeta),
}

impl PlatformMessageMeta {
    /// The part of the message the sender chose to point at, when the platform carries one: a quote
    /// the platform added by itself says nothing a reference preview does not already show.
    pub fn excerpt(&self) -> Option<&str> {
        match self {
            Self::Telegram(meta) => meta
                .quote
                .as_ref()
                .filter(|quote| quote.is_manual)
                .map(|quote| quote.text.as_str()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramMessageMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<TelegramQuote>,
}

/// The part of a message a reference points at. Telegram fills this in for a reply that quotes an
/// excerpt; `is_manual` tells a chosen excerpt apart from one the server added by itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramQuote {
    pub text: String,
    /// Position of the quoted part in the original message, in UTF-16 code units.
    pub position: u32,
    pub is_manual: bool,
}
