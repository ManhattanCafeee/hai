use serde::Deserialize;
use strum::{Display, EnumString, IntoStaticStr};

use super::*;
use crate::{
    domain::vo::{
        AccountId, ChatId, MessageId, MessageMeta, PlatformMessageMeta, TopicId, enum_column,
    },
    error::Result,
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Message {
    pub id: MessageId,
    pub chat_id: ChatId,

    pub account_id: Option<AccountId>,

    pub role: MessageRole,
    pub text: String,

    pub topic_id: Option<TopicId>,

    pub reply_to_id: Option<MessageId>,

    pub external_id: Option<String>,
    pub meta: Option<serde_json::Value>,
    #[sqlx(try_from = "SqlxNullableTimestamp")]
    pub sent_at: Option<jiff::Timestamp>,
    #[sqlx(try_from = "SqlxNullableTimestamp")]
    pub edited_at: Option<jiff::Timestamp>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

impl Message {
    /// The message's content: its own text plus each attachment's caption, which is where a
    /// platform puts a media message's words.
    pub fn content(&self, attachments: &[Attachment]) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(attachments.len() + 1);
        if !self.text.is_empty() {
            parts.push(&self.text);
        }
        parts.extend(attachments.iter().filter_map(|a| a.caption.as_deref()));
        parts.join("\n")
    }

    /// The platform extras this row carries. An unreadable meta is an error: every write path stores
    /// a `MessageMeta`, so a row this build cannot read is not one this build wrote.
    pub fn platform_meta(&self) -> Result<Option<PlatformMessageMeta>> {
        let Some(raw) = self.meta.as_ref() else {
            return Ok(None);
        };
        let meta = MessageMeta::deserialize(raw)?;
        Ok(meta.platform)
    }
}

/// The payload one message row is written from.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub chat_id: ChatId,
    pub account_id: Option<AccountId>,
    pub role: MessageRole,
    pub text: String,
    pub attachments: Vec<NewAttachment>,
    pub topic_id: Option<TopicId>,
    pub reply_to_id: Option<MessageId>,
    pub external_id: Option<String>,
    pub meta: serde_json::Value,
    pub sent_at: Option<jiff::Timestamp>,
    pub edited_at: Option<jiff::Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

enum_column!(MessageRole);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        test_support::{self, message},
        vo::AttachmentKind,
    };

    fn attachment(caption: Option<&str>) -> Attachment {
        Attachment {
            caption: caption.map(str::to_owned),
            ..test_support::attachment(AttachmentKind::Photo)
        }
    }

    #[test]
    fn content_carries_each_attachments_caption() {
        assert_eq!(message("").content(&[attachment(Some("a cat"))]), "a cat");
        assert_eq!(
            message("look").content(&[attachment(Some("a cat"))]),
            "look\na cat"
        );
        assert_eq!(message("look").content(&[attachment(None)]), "look");
    }
}
