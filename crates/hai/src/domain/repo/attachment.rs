use crate::{
    domain::{
        model::{Attachment, NewAttachment},
        vo::{AttachmentId, MessageId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct AttachmentRepo {
    pool: sqlx::PgPool,
}

impl AttachmentRepo {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn by_id(&self, id: AttachmentId) -> Result<Option<Attachment>> {
        sqlx::query_as::<_, Attachment>("SELECT * FROM attachment WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn by_message_ids(&self, ids: &[MessageId]) -> Result<Vec<Attachment>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, Attachment>(
            "SELECT * FROM attachment WHERE message_id = ANY($1) ORDER BY message_id, ordinal",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// One message's attachments, in ordinal order; called inside the message write's transaction.
    pub(crate) async fn insert_many(
        executor: &mut sqlx::PgConnection,
        message_id: MessageId,
        rows: &[NewAttachment],
    ) -> Result<()> {
        for row in rows {
            sqlx::query(
                "INSERT INTO attachment (id, message_id, ordinal, platform, file_unique_id, \
                 file_id, kind, caption, meta) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(row.id)
            .bind(message_id.0)
            .bind(row.ordinal)
            .bind(row.platform.as_str())
            .bind(row.file_unique_id.as_deref())
            .bind(&row.file_id)
            .bind(row.kind.name())
            .bind(row.caption.as_deref())
            .bind(serde_json::to_value(&row.meta)?)
            .execute(&mut *executor)
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn delete_for_message(
        executor: &mut sqlx::PgConnection,
        message_id: MessageId,
    ) -> Result<()> {
        sqlx::query("DELETE FROM attachment WHERE message_id = $1")
            .bind(message_id.0)
            .execute(&mut *executor)
            .await?;
        Ok(())
    }
}
