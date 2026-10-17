use uuid::Uuid;

use crate::domain::vo::{KnowledgeChunkId, KnowledgeDocumentId};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KnowledgeChunk {
    pub id: KnowledgeChunkId,
    pub document_id: KnowledgeDocumentId,
    pub seq: i32,
    pub content: String,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}

impl KnowledgeChunk {
    pub fn new(document_id: KnowledgeDocumentId, seq: i32, content: String) -> Self {
        Self {
            id: KnowledgeChunkId(Uuid::now_v7()),
            document_id,
            seq,
            content,
            created_at: jiff::Timestamp::now(),
        }
    }
}
