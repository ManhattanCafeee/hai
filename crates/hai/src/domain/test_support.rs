//! The row fixtures a test starts from: every field is spelled here once, and a test that needs
//! another value overrides that field on the value it gets back.

use crate::domain::{
    model::{
        Attachment, Memory, MemoryKind, MemoryStatus, Message, MessageRole, Perception, Platform,
        Topic, TopicStatus,
    },
    vo::{
        AttachmentId, AttachmentKind, AttachmentMeta, AttachmentParser, ChatId, MemoryId,
        MessageId, PerceptionId, SubjectKind, TopicId,
    },
};

/// A Telegram attachment with the handle and file-unique id a platform would have given it.
pub(crate) fn attachment(kind: AttachmentKind) -> Attachment {
    Attachment {
        id: AttachmentId(uuid::Uuid::from_u128(1)),
        message_id: MessageId(1),
        ordinal: 0,
        platform: Platform::Telegram,
        file_unique_id: Some("uniq".to_owned()),
        file_id: "handle".to_owned(),
        kind,
        caption: None,
        meta: AttachmentMeta::default(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

pub(crate) fn message(text: &str) -> Message {
    Message {
        id: MessageId(1),
        chat_id: ChatId(1),
        account_id: None,
        role: MessageRole::User,
        text: text.to_owned(),
        topic_id: None,
        reply_to_id: None,
        external_id: None,
        meta: None,
        sent_at: None,
        edited_at: None,
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

pub(crate) fn perception(subject: &str, parser: AttachmentParser, content: &str) -> Perception {
    Perception {
        id: PerceptionId(uuid::Uuid::from_u128(1)),
        subject_kind: SubjectKind::Handle,
        subject: subject.to_owned(),
        parser,
        focus: None,
        content: content.to_owned(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

pub(crate) fn memory(id: u128, content: &str) -> Memory {
    Memory {
        id: MemoryId(uuid::Uuid::from_u128(id)),
        account_id: None,
        chat_id: ChatId(1),
        kind: MemoryKind::Note,
        content: content.to_owned(),
        source_message_ids: Vec::new(),
        status: MemoryStatus::Active,
        pinned: false,
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

pub(crate) fn topic(id: u128, title: &str) -> Topic {
    Topic {
        id: TopicId(uuid::Uuid::from_u128(id)),
        chat_id: ChatId(1),
        title: Some(title.to_owned()),
        summary: Some("Summary".to_owned()),
        status: TopicStatus::Closed,
        first_message_id: None,
        last_message_id: None,
        started_at: jiff::Timestamp::UNIX_EPOCH,
        last_active_at: jiff::Timestamp::UNIX_EPOCH,
        closed_at: None,
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
    }
}
