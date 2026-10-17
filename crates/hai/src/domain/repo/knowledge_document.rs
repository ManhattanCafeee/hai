use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::{
        model::{KnowledgeChunk, KnowledgeDocument},
        repo::vector,
        vo::{Collection, KnowledgeDocumentId},
    },
    error::{AppError, Result, bail},
};

#[derive(Debug, Clone)]
pub struct KnowledgeDocumentRepo {
    pool: PgPool,
}

impl KnowledgeDocumentRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_by_source(&self, source: &str) -> Result<Option<KnowledgeDocument>> {
        sqlx::query_as::<_, KnowledgeDocument>(
            "SELECT * FROM knowledge_document WHERE source = $1 LIMIT 1",
        )
        .bind(source)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list(&self, collection: Option<&Collection>) -> Result<Vec<KnowledgeDocument>> {
        let rows = match collection.map(Collection::as_str) {
            Some(col) => {
                sqlx::query_as::<_, KnowledgeDocument>(
                    "SELECT * FROM knowledge_document WHERE collection = $1 \
                     ORDER BY updated_at DESC",
                )
                .bind(col)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, KnowledgeDocument>(
                    "SELECT * FROM knowledge_document ORDER BY updated_at DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    pub async fn ids_by_collections(
        &self,
        collections: &[Collection],
    ) -> Result<Vec<KnowledgeDocumentId>> {
        if collections.is_empty() {
            return Ok(Vec::new());
        }
        let spellings: Vec<&str> = collections.iter().map(Collection::as_str).collect();
        let ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM knowledge_document WHERE collection = ANY($1)")
                .bind(spellings)
                .fetch_all(&self.pool)
                .await?;
        Ok(ids
            .into_iter()
            .map(KnowledgeDocumentId)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    pub async fn by_ids(&self, ids: &[KnowledgeDocumentId]) -> Result<Vec<KnowledgeDocument>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, KnowledgeDocument>(
            "SELECT * FROM knowledge_document WHERE id = ANY($1)",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Rebuilds a whole document in one transaction: the old rows (and their chunks) go first,
    /// then the document and all of its chunks — vectors included — are inserted. The caller
    /// generates the embeddings outside the transaction, since an embedding call must not hold a
    /// connection. `embeddings` must align with `chunks`: a mismatch is an error, not a silently
    /// truncated index.
    pub async fn upsert_document(
        &self,
        source: &str,
        doc: &KnowledgeDocument,
        chunks: &[KnowledgeChunk],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        if chunks.len() != embeddings.len() {
            bail!(AppError::message(format!(
                "{} chunks but {} embeddings for source {source}",
                chunks.len(),
                embeddings.len()
            )));
        }
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "DELETE FROM knowledge_chunk \
             WHERE document_id IN (SELECT id FROM knowledge_document WHERE source = $1)",
        )
        .bind(source)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM knowledge_document WHERE source = $1")
            .bind(source)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO knowledge_document \
             (id, title, collection, source, content, meta, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(doc.id.0)
        .bind(&doc.title)
        .bind(&doc.collection)
        .bind(&doc.source)
        .bind(&doc.content)
        .bind(doc.meta.as_ref())
        .bind(jiff_sqlx::Timestamp::from(doc.created_at))
        .bind(jiff_sqlx::Timestamp::from(doc.updated_at))
        .execute(&mut *tx)
        .await?;

        for (chunk, emb) in chunks.iter().zip(embeddings) {
            sqlx::query(
                "INSERT INTO knowledge_chunk \
                 (id, document_id, seq, content, created_at, embedding) \
                 VALUES ($1, $2, $3, $4, $5, $6::vector)",
            )
            .bind(chunk.id.0)
            .bind(chunk.document_id.0)
            .bind(chunk.seq)
            .bind(&chunk.content)
            .bind(jiff_sqlx::Timestamp::from(chunk.created_at))
            .bind(vector::vec_to_pgstring(emb))
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn delete(&self, document_id: KnowledgeDocumentId) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM knowledge_chunk WHERE document_id = $1")
            .bind(document_id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM knowledge_document WHERE id = $1")
            .bind(document_id.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
