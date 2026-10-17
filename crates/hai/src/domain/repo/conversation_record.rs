use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::{
        model::ConversationRecord,
        vo::{ChatEntry, ChatId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct ConversationRecordRepo {
    pool: PgPool,
}

impl ConversationRecordRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_by_chat_id(&self, chat_id: ChatId) -> Result<Option<ConversationRecord>> {
        sqlx::query_as::<_, ConversationRecord>(
            "SELECT * FROM conversation WHERE chat_id = $1 LIMIT 1",
        )
        .bind(chat_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Writes the snapshot and consumes the batch in one statement, whether or not the chat has a
    /// record yet: a separate insert branch races the ingest that appends to the same row, and its
    /// duplicate-key failure would lose the write. Consumption is relative to what `pending` holds
    /// when the write lands, which is what keeps an entry appended while the Turn ran.
    pub async fn save(
        &self,
        chat_id: ChatId,
        messages: Value,
        context_meta: Value,
        consumed: &[Uuid],
        updated_at: jiff::Timestamp,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO conversation (chat_id, messages, context_meta, updated_at) \
             VALUES ($1, $2, $3, $5) \
             ON CONFLICT (chat_id) DO UPDATE SET messages = $2, context_meta = $3, \
             pending = COALESCE((SELECT jsonb_agg(e) FROM jsonb_array_elements(conversation.pending) e \
             WHERE (e->>'id')::uuid <> ALL($4)), '[]'::jsonb), updated_at = $5",
        )
        .bind(chat_id.0)
        .bind(messages)
        .bind(context_meta)
        .bind(consumed)
        .bind(jiff_sqlx::Timestamp::from(updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Appends one change on its own transaction, for a producer outside the ingest path.
    pub async fn append_entry(&self, chat_id: ChatId, entry: &ChatEntry) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        Self::append_entry_in_tx(&mut tx, chat_id, entry).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Appends one change to the chat's accumulation, relative to what `pending` already holds, so
    /// an ingest and a turn's commit cannot overwrite each other's entry. A chat with no record yet
    /// gets the empty one.
    pub(crate) async fn append_entry_in_tx(
        tx: &mut sqlx::PgConnection,
        chat_id: ChatId,
        entry: &ChatEntry,
    ) -> Result<()> {
        let entry = serde_json::to_value(entry)?;
        sqlx::query(
            "INSERT INTO conversation (chat_id, messages, context_meta, pending) \
             VALUES ($1, '[]'::jsonb, '{}'::jsonb, jsonb_build_array($2::jsonb)) \
             ON CONFLICT (chat_id) DO UPDATE \
             SET pending = conversation.pending || jsonb_build_array($2::jsonb), updated_at = now()",
        )
        .bind(chat_id.0)
        .bind(entry)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    /// The accumulated changes of a chat, oldest first. A stored entry that no longer parses is an
    /// error, not an empty backlog.
    pub async fn pending_entries(&self, chat_id: ChatId) -> Result<Vec<ChatEntry>> {
        let pending: Option<Value> =
            sqlx::query_scalar("SELECT pending FROM conversation WHERE chat_id = $1")
                .bind(chat_id.0)
                .fetch_optional(&self.pool)
                .await?;
        let Some(pending) = pending else {
            return Ok(Vec::new());
        };
        let mut entries: Vec<ChatEntry> = serde_json::from_value(pending)?;
        entries.sort_by(|a, b| a.at.cmp(&b.at).then(a.id.cmp(&b.id)));
        Ok(entries)
    }

    pub async fn chats_with_pending(&self) -> Result<Vec<ChatId>> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT chat_id FROM conversation WHERE jsonb_array_length(pending) > 0 \
             ORDER BY chat_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(ChatId).collect())
    }
}
