use sqlx::PgPool;

use crate::{
    domain::{
        model::KnowledgeChunk,
        repo::{lexical, vector},
        vo::{KnowledgeChunkId, KnowledgeDocumentId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct KnowledgeChunkRepo {
    pool: PgPool,
}

impl KnowledgeChunkRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn search_by_embedding(
        &self,
        query: &[f32],
        document_ids: Option<&[KnowledgeDocumentId]>,
        limit: i64,
    ) -> Result<Vec<(KnowledgeChunkId, f64)>> {
        let raw_document_ids = document_ids.map(KnowledgeDocumentId::raw_ids);
        let rows = match raw_document_ids.as_deref() {
            Some(ids) => {
                vector::search_document_embedding_vec(&self.pool, query, ids, limit).await?
            }
            None => {
                vector::search_unscoped_embedding_vec(
                    &self.pool,
                    vector::EmbeddingTable::KnowledgeChunk,
                    query,
                    limit,
                )
                .await?
            }
        };
        Ok(rows
            .into_iter()
            .map(|(id, distance)| (KnowledgeChunkId(id), distance))
            .collect())
    }

    /// Chunks whose content carries any of the keywords, most matches first; the lexical leg of
    /// hybrid search. `document_ids` of `None` searches every document.
    pub async fn search_keywords(
        &self,
        keywords: &[String],
        document_ids: Option<&[KnowledgeDocumentId]>,
        limit: i64,
    ) -> Result<Vec<KnowledgeChunkId>> {
        let patterns = lexical::patterns(keywords);
        let raw_document_ids = document_ids.map(KnowledgeDocumentId::raw_ids);
        let rows: Vec<(uuid::Uuid,)> =
            sqlx::query_as(sqlx::AssertSqlSafe(keyword_search_sql().as_str()))
                .bind(raw_document_ids.as_deref())
                .bind(&patterns)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| KnowledgeChunkId(id)).collect())
    }

    pub async fn by_ids(&self, ids: &[KnowledgeChunkId]) -> Result<Vec<KnowledgeChunk>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows =
            sqlx::query_as::<_, KnowledgeChunk>("SELECT * FROM knowledge_chunk WHERE id = ANY($1)")
                .bind(ids)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }
}

/// The lexical leg's statement: the keyword patterns are bound at `$2`.
fn keyword_search_sql() -> String {
    format!(
        "SELECT c.id FROM knowledge_chunk c \
         WHERE ($1::uuid[] IS NULL OR c.document_id = ANY($1)) \
         AND {} \
         ORDER BY {}, \
                  c.seq ASC \
         LIMIT $3",
        lexical::match_predicate(&["c.content"], 2),
        lexical::match_rank(&["c.content"], 2),
    )
}
