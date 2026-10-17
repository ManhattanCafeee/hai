use uuid::Uuid;

use crate::{
    domain::{
        model::ConversationRecord,
        repo::Repos,
        vo::{ChatEntry, ChatId, ConversationSnapshot},
    },
    error::{AppError, Result},
};

#[derive(Debug)]
pub struct ConversationRecordService {
    repos: Repos,
}

impl ConversationRecordService {
    pub fn new(repos: Repos) -> Self {
        Self { repos }
    }

    /// `Ok(None)` means there is no record yet (a fresh conversation); a database failure
    /// surfaces as `Err` instead of being turned into one.
    pub async fn get(&self, chat_id: ChatId) -> Result<Option<ConversationRecord>> {
        self.repos
            .conversation_record
            .find_by_chat_id(chat_id)
            .await
    }

    pub(crate) async fn save(
        &self,
        snap: &ConversationSnapshot,
        chat_id: ChatId,
        consumed: &[Uuid],
    ) -> Result<()> {
        let messages = serde_json::to_value(&snap.context_messages)?;
        let context_meta = serde_json::json!({
            "tokens": snap.context_meta.tokens,
            "turn_count": snap.context_meta.turn_count,
            "step_count": snap.context_meta.step_count,
        });
        self.repos
            .conversation_record
            .save(
                chat_id,
                messages,
                context_meta,
                consumed,
                jiff::Timestamp::now(),
            )
            .await
    }

    pub(crate) fn restore(&self, record: &ConversationRecord) -> Result<ConversationSnapshot> {
        restore_snapshot(record)
    }

    pub async fn append_entry(&self, chat_id: ChatId, entry: &ChatEntry) -> Result<()> {
        self.repos
            .conversation_record
            .append_entry(chat_id, entry)
            .await
    }

    pub async fn pending_entries(&self, chat_id: ChatId) -> Result<Vec<ChatEntry>> {
        self.repos
            .conversation_record
            .pending_entries(chat_id)
            .await
    }

    pub async fn chats_with_pending(&self) -> Result<Vec<ChatId>> {
        self.repos.conversation_record.chats_with_pending().await
    }
}

fn restore_snapshot(record: &ConversationRecord) -> Result<ConversationSnapshot> {
    let cid = record.chat_id;
    let messages = serde_json::from_value(record.messages.clone()).map_err(|e| {
        AppError::message(format!(
            "chat {cid}: conversation snapshot messages unreadable: {e}"
        ))
    })?;
    let context_meta = serde_json::from_value(record.context_meta.clone()).map_err(|e| {
        AppError::message(format!(
            "chat {cid}: conversation snapshot context_meta unreadable: {e}"
        ))
    })?;

    Ok(ConversationSnapshot {
        context_messages: messages,
        context_meta,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn record(messages: Value, context_meta: Value) -> ConversationRecord {
        ConversationRecord {
            chat_id: ChatId(42),
            messages,
            context_meta,
            updated_at: jiff::Timestamp::now(),
        }
    }

    #[test]
    fn corrupt_snapshot_is_an_error() {
        assert!(
            restore_snapshot(&record(
                serde_json::json!({"not": "a message list"}),
                serde_json::json!("not an object"),
            ))
            .is_err(),
            "an unreadable snapshot must not be replaced by defaults"
        );
    }
}
