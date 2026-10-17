use serde_json::Value;
use sqlx::PgPool;

use crate::{
    domain::{
        model::{Message, MessageRole, NewAttachment, NewMessage},
        repo::AttachmentRepo,
        vo::{ChatId, MessageId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct MessageRepo {
    pool: PgPool,
}

impl MessageRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_by_chat_external_id(
        &self,
        chat_id: ChatId,
        external_id: &str,
    ) -> Result<Option<Message>> {
        sqlx::query_as::<_, Message>(
            "SELECT * FROM message WHERE chat_id = $1 AND external_id = $2 LIMIT 1",
        )
        .bind(chat_id.0)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub(crate) async fn find_by_chat_external_id_in_tx(
        tx: &mut sqlx::PgConnection,
        chat_id: ChatId,
        external_id: &str,
    ) -> Result<Option<Message>> {
        sqlx::query_as::<_, Message>(
            "SELECT * FROM message WHERE chat_id = $1 AND external_id = $2 LIMIT 1",
        )
        .bind(chat_id.0)
        .bind(external_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Into::into)
    }

    pub async fn create(&self, new: NewMessage) -> Result<Message> {
        let mut tx = self.pool.begin().await?;
        let message = Self::create_in_tx(&mut tx, new).await?;
        tx.commit().await?;
        Ok(message)
    }

    pub(crate) async fn create_in_tx(
        tx: &mut sqlx::PgConnection,
        new: NewMessage,
    ) -> Result<Message> {
        let message = sqlx::query_as::<_, Message>(
            "INSERT INTO message (chat_id, account_id, role, text, topic_id, \
             reply_to_id, external_id, meta, sent_at, edited_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING *",
        )
        .bind(new.chat_id)
        .bind(new.account_id)
        .bind(new.role.as_str())
        .bind(new.text)
        .bind(new.topic_id)
        .bind(new.reply_to_id)
        .bind(new.external_id)
        .bind(new.meta)
        .bind(new.sent_at.map(jiff_sqlx::Timestamp::from))
        .bind(new.edited_at.map(jiff_sqlx::Timestamp::from))
        .fetch_one(&mut *tx)
        .await?;
        AttachmentRepo::insert_many(tx, message.id, &new.attachments).await?;
        Ok(message)
    }

    /// Replaces a message's text, meta, edit time and attachments in one transaction, so a
    /// re-delivery of the same external id cannot leave the row and its attachments disagreeing.
    pub async fn update_with_attachments(
        &self,
        id: MessageId,
        text: String,
        meta: Value,
        edited_at: Option<jiff::Timestamp>,
        attachments: &[NewAttachment],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        Self::update_with_attachments_in_tx(&mut tx, id, text, meta, edited_at, attachments)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn update_with_attachments_in_tx(
        tx: &mut sqlx::PgConnection,
        id: MessageId,
        text: String,
        meta: Value,
        edited_at: Option<jiff::Timestamp>,
        attachments: &[NewAttachment],
    ) -> Result<()> {
        sqlx::query(
            "UPDATE message SET text = $2, meta = $3, edited_at = $4, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id.0)
        .bind(text)
        .bind(meta)
        .bind(edited_at.map(jiff_sqlx::Timestamp::from))
        .execute(&mut *tx)
        .await?;
        AttachmentRepo::delete_for_message(tx, id).await?;
        AttachmentRepo::insert_many(tx, id, attachments).await?;
        Ok(())
    }

    /// Replaces a row's content and attachments with a redelivered change's, and hands back the row
    /// as it now stands; the in-tx variant is what the ingest transaction uses.
    pub async fn refresh(
        &self,
        existing: &Message,
        text: String,
        meta: Value,
        edited_at: Option<jiff::Timestamp>,
        attachments: &[NewAttachment],
    ) -> Result<Message> {
        self.update_with_attachments(existing.id, text, meta, edited_at, attachments)
            .await?;
        Ok(self
            .by_id(existing.id)
            .await?
            .expect("the row was written just above"))
    }

    pub(crate) async fn refresh_in_tx(
        tx: &mut sqlx::PgConnection,
        existing: &Message,
        text: String,
        meta: Value,
        edited_at: Option<jiff::Timestamp>,
        attachments: &[NewAttachment],
    ) -> Result<Message> {
        Self::update_with_attachments_in_tx(tx, existing.id, text, meta, edited_at, attachments)
            .await?;
        Ok(Self::by_id_in_tx(tx, existing.id)
            .await?
            .expect("the row was updated in this transaction"))
    }

    pub async fn by_id(&self, id: MessageId) -> Result<Option<Message>> {
        sqlx::query_as::<_, Message>("SELECT * FROM message WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn by_id_in_tx(
        tx: &mut sqlx::PgConnection,
        id: MessageId,
    ) -> Result<Option<Message>> {
        sqlx::query_as::<_, Message>("SELECT * FROM message WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&mut *tx)
            .await
            .map_err(Into::into)
    }

    pub async fn by_ids(&self, ids: &[MessageId]) -> Result<Vec<Message>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows =
            sqlx::query_as::<_, Message>("SELECT * FROM message WHERE id = ANY($1) ORDER BY id")
                .bind(ids)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub(crate) async fn id_by_chat_external_id_in_tx(
        tx: &mut sqlx::PgConnection,
        chat_id: ChatId,
        external_id: &str,
    ) -> Result<Option<MessageId>> {
        let id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM message WHERE chat_id = $1 AND external_id = $2 LIMIT 1",
        )
        .bind(chat_id.0)
        .bind(external_id)
        .fetch_optional(&mut *tx)
        .await?;
        Ok(id.map(MessageId))
    }

    /// The newest `limit` messages of the chat, ascending.
    pub async fn recent_asc(&self, chat_id: ChatId, limit: i64) -> Result<Vec<Message>> {
        let mut rows = sqlx::query_as::<_, Message>(
            "SELECT * FROM message WHERE chat_id = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(chat_id.0)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    pub async fn has_newer_from_user(&self, chat_id: ChatId, id: MessageId) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM message \
             WHERE chat_id = $1 AND role = $2 AND id > $3)",
        )
        .bind(chat_id.0)
        .bind(MessageRole::User.as_str())
        .bind(id.0)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    pub async fn update_meta(&self, id: MessageId, meta: Option<Value>) -> Result<()> {
        sqlx::query("UPDATE message SET meta = $2 WHERE id = $1")
            .bind(id.0)
            .bind(meta)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
