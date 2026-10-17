use serde_json::Value;
use sqlx::PgPool;

use crate::{
    domain::{
        model::{Message, MessageRole, NewAttachment, NewMessage},
        repo::{ConversationRecordRepo, MessageRepo},
        vo::{AccountId, ChatEntry, ChatId, EntryAudience, EntryKind},
    },
    error::{AppError, Result},
};

/// One incoming platform change: the row's content and the change that says it happened.
#[derive(Debug, Clone)]
pub struct PlatformChange {
    pub chat_id: ChatId,
    pub account_id: AccountId,
    pub text: String,
    pub attachments: Vec<NewAttachment>,
    pub external_id: String,
    pub meta: Value,
    pub sent_at: Option<jiff::Timestamp>,
    /// What happened to this message; an edit carries the platform's own edit time.
    pub kind: ChangeKind,
    /// The platform id of the message being replied to, resolved to a row inside the transaction.
    pub reply_external_id: Option<String>,
    pub audience: EntryAudience,
}

/// What happened to the message this change carries. A creation can still carry the platform's edit
/// time: a message can be seen for the first time after it was edited.
#[derive(Debug, Clone, Copy)]
pub enum ChangeKind {
    Created { edited_at: Option<jiff::Timestamp> },
    Edited { edited_at: jiff::Timestamp },
}

#[derive(Debug, Clone)]
pub struct IngestRepo {
    pool: PgPool,
}

impl IngestRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// One change lands as one transaction: the projection keeps the current content and `pending`
    /// gains the change that says it happened. A redelivered creation refreshes the row without a
    /// second entry, and an edit no newer than the stored one is dropped whole.
    pub async fn apply(&self, change: PlatformChange) -> Result<Message> {
        let mut tx = self.pool.begin().await?;
        let existing = MessageRepo::find_by_chat_external_id_in_tx(
            &mut tx,
            change.chat_id,
            &change.external_id,
        )
        .await?;

        match change.kind {
            ChangeKind::Edited { edited_at } => {
                let Some(existing) = existing else {
                    return Err(AppError::message(format!(
                        "edit of message {} that was never stored",
                        change.external_id
                    )));
                };
                if existing.edited_at.is_some_and(|stored| stored >= edited_at) {
                    return Ok(existing);
                }
                let updated = MessageRepo::refresh_in_tx(
                    &mut tx,
                    &existing,
                    change.text,
                    change.meta,
                    Some(edited_at),
                    &change.attachments,
                )
                .await?;
                let entry = ChatEntry::new(
                    change.audience,
                    EntryKind::MessageEdited {
                        message_id: existing.id,
                        edited_at,
                    },
                    edited_at,
                );
                ConversationRecordRepo::append_entry_in_tx(&mut tx, change.chat_id, &entry).await?;
                tx.commit().await?;
                Ok(updated)
            }
            ChangeKind::Created { edited_at } => {
                // The same message arriving twice is one change: the row is refreshed in place and
                // keeps its own edit time, without a second entry.
                if let Some(existing) = existing {
                    let updated = MessageRepo::refresh_in_tx(
                        &mut tx,
                        &existing,
                        change.text,
                        change.meta,
                        existing.edited_at,
                        &change.attachments,
                    )
                    .await?;
                    tx.commit().await?;
                    return Ok(updated);
                }

                let reply_to_id = match &change.reply_external_id {
                    Some(external_id) => {
                        MessageRepo::id_by_chat_external_id_in_tx(
                            &mut tx,
                            change.chat_id,
                            external_id,
                        )
                        .await?
                    }
                    None => None,
                };
                let message = MessageRepo::create_in_tx(
                    &mut tx,
                    NewMessage {
                        chat_id: change.chat_id,
                        account_id: Some(change.account_id),
                        role: MessageRole::User,
                        text: change.text,
                        attachments: change.attachments,
                        topic_id: None,
                        reply_to_id,
                        external_id: Some(change.external_id),
                        meta: change.meta,
                        sent_at: change.sent_at,
                        edited_at,
                    },
                )
                .await?;
                let at = change.sent_at.unwrap_or_else(jiff::Timestamp::now);
                let entry = ChatEntry::new(
                    change.audience,
                    EntryKind::MessageCreated {
                        message_id: message.id,
                    },
                    at,
                );
                ConversationRecordRepo::append_entry_in_tx(&mut tx, change.chat_id, &entry).await?;
                tx.commit().await?;
                Ok(message)
            }
        }
    }
}
