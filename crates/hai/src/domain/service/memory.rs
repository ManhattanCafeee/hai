use std::sync::Arc;

use uuid::Uuid;

use super::hydrate;
use crate::{
    agentcore::api::embedding::EmbeddingService,
    domain::{
        model::{Memory, MemoryKind, MemoryStatus, NewMemory},
        repo::Repos,
        vo::{AccountId, ChatId, MemoryId, MessageId, TimeRange},
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct RecordOutcome {
    pub memory: Memory,
    pub duplicate: bool,
    pub similar: Vec<Memory>,
}

/// A hit closer than this counts as a possible duplicate. It is an L2 distance from the `<->`
/// query, calibrated for unit-normalized embeddings, where L2 orders the same as cosine.
const SIMILAR_DISTANCE: f64 = 0.35;

/// The error every miss reports, whether the row never existed or belongs to another chat: the scope
/// lives in the statement, so both look the same from here.
fn missing(id: MemoryId) -> AppError {
    AppError::message(format!("Memory not found: {id}"))
}

#[derive(Debug, Clone)]
pub struct MemoryService {
    repos: Repos,
    embedding: Arc<dyn EmbeddingService>,
}

impl MemoryService {
    pub fn new(repos: Repos, embedding: Arc<dyn EmbeddingService>) -> Self {
        Self { repos, embedding }
    }

    pub async fn record(
        &self,
        kind: MemoryKind,
        chat_id: ChatId,
        content: String,
        account_id: Option<AccountId>,
        source_message_ids: &[MessageId],
    ) -> Result<RecordOutcome> {
        if let Some(existing) = self
            .repos
            .memory
            .find_duplicate(kind, chat_id, &content, account_id)
            .await?
        {
            return Ok(RecordOutcome {
                memory: existing,
                duplicate: true,
                similar: Vec::new(),
            });
        }

        let embedding = self.embedding.generate_embedding(&content).await?;
        let similar = self
            .nearest(chat_id, &embedding, 3)
            .await?
            .into_iter()
            .filter(|(_, distance)| *distance <= SIMILAR_DISTANCE)
            .map(|(memory, _)| memory)
            .collect();

        let now = jiff::Timestamp::now();
        let memory = self
            .repos
            .memory
            .create(
                NewMemory {
                    id: MemoryId(Uuid::now_v7()),
                    kind,
                    status: MemoryStatus::Active,
                    chat_id,
                    account_id,
                    content,
                    source_message_ids: source_message_ids.to_vec(),
                    created_at: now,
                    updated_at: now,
                },
                &embedding,
            )
            .await?;

        Ok(RecordOutcome {
            memory,
            duplicate: false,
            similar,
        })
    }

    pub async fn revise(&self, chat_id: ChatId, id: MemoryId, content: String) -> Result<Memory> {
        let embedding = self.embedding.generate_embedding(&content).await?;
        self.repos
            .memory
            .revise(chat_id, id, &content, &embedding)
            .await?
            .ok_or_else(|| missing(id))
    }

    pub async fn invalidate(&self, chat_id: ChatId, id: MemoryId) -> Result<Memory> {
        self.repos
            .memory
            .set_status(chat_id, id, MemoryStatus::Invalidated)
            .await?
            .ok_or_else(|| missing(id))
    }

    pub async fn forget(&self, chat_id: ChatId, id: MemoryId) -> Result<()> {
        if self.repos.memory.delete(chat_id, id).await? {
            Ok(())
        } else {
            Err(missing(id))
        }
    }

    pub async fn pin(&self, chat_id: ChatId, id: MemoryId, pinned: bool) -> Result<Memory> {
        self.repos
            .memory
            .set_pinned(chat_id, id, pinned)
            .await?
            .ok_or_else(|| missing(id))
    }

    pub async fn get(&self, chat_id: ChatId, id: MemoryId) -> Result<Memory> {
        self.repos
            .memory
            .find_by_id(chat_id, id)
            .await?
            .ok_or_else(|| missing(id))
    }

    pub async fn pinned(&self, chat_id: ChatId, limit: i64) -> Result<Vec<Memory>> {
        self.repos.memory.pinned(chat_id, limit).await
    }

    pub async fn search_related(
        &self,
        chat_id: ChatId,
        query: &[f32],
        limit: i64,
    ) -> Result<Vec<Memory>> {
        Ok(self
            .nearest(chat_id, query, limit)
            .await?
            .into_iter()
            .map(|(memory, _)| memory)
            .collect())
    }

    async fn nearest(
        &self,
        chat_id: ChatId,
        query: &[f32],
        limit: i64,
    ) -> Result<Vec<(Memory, f64)>> {
        let rows = self
            .repos
            .memory
            .search_by_embedding(chat_id, query, limit)
            .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<MemoryId> = rows.iter().map(|(id, _)| *id).collect();
        let by_id = self.repos.memory.by_ids(&ids).await?;
        Ok(hydrate::merge_hydrated(
            rows,
            by_id
                .into_iter()
                .map(|memory| (memory.id, memory))
                .collect(),
        ))
    }

    pub async fn search_keywords(
        &self,
        chat_id: ChatId,
        keywords: &[String],
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        self.repos
            .memory
            .search_by_keywords(chat_id, keywords, range, limit)
            .await
    }

    pub async fn list_by_time(
        &self,
        chat_id: ChatId,
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        self.repos.memory.by_chat_time(chat_id, range, limit).await
    }
}
