use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::vo::{Collection, KnowledgeDocumentId};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KnowledgeDocument {
    pub id: KnowledgeDocumentId,

    pub title: String,
    /// The collection this document is filed under: a named one, or `Collection::Uncategorised`.
    pub collection: Collection,
    pub source: String,
    pub content: String,
    pub meta: Option<serde_json::Value>,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub updated_at: jiff::Timestamp,
}

impl KnowledgeDocument {
    pub fn new(
        title: String,
        collection: Collection,
        source: String,
        content: String,
        meta: Option<serde_json::Value>,
    ) -> Self {
        Self {
            id: KnowledgeDocumentId(Uuid::now_v7()),
            title,
            collection,
            source,
            content,
            meta,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        }
    }
}

/// Typed contents of the `meta` column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDocumentMeta {
    /// sha256 of the document content; half of the import idempotency check.
    pub file_hash: String,
    pub chunker_version: String,
}
