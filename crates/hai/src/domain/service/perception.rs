use std::sync::Arc;

use crate::{
    agentcore::api::embedding::EmbeddingService,
    domain::{
        model::Perception,
        repo::Repos,
        vo::{AttachmentParser, PerceptionId, Subject, SubjectKind},
    },
    error::Result,
};

#[derive(Debug)]
pub struct PerceptionService {
    repos: Repos,
    embedding: Arc<dyn EmbeddingService>,
}

impl PerceptionService {
    pub fn new(repos: Repos, embedding: Arc<dyn EmbeddingService>) -> Self {
        Self { repos, embedding }
    }

    pub async fn find(
        &self,
        subject: &Subject,
        parser: AttachmentParser,
        focus: Option<&str>,
    ) -> Result<Option<Perception>> {
        self.repos.perception.find(subject, parser, focus).await
    }

    /// One round trip per subject kind; a file can yield several rows (base transcription plus
    /// targeted readings). `Subject.value` is the `subject` column verbatim, so nothing re-adds a
    /// platform prefix to an already-qualified handle.
    pub async fn find_by_subjects(&self, subjects: &[Subject]) -> Result<Vec<Perception>> {
        let mut rows = Vec::new();
        for kind in [SubjectKind::Handle, SubjectKind::Url] {
            let values: Vec<String> = subjects
                .iter()
                .filter(|s| s.kind == kind)
                .map(|s| s.value.clone())
                .collect();
            if values.is_empty() {
                continue;
            }
            rows.extend(
                self.repos
                    .perception
                    .find_by_subjects(kind, &values)
                    .await?,
            );
        }
        Ok(rows)
    }

    pub async fn upsert(
        &self,
        subject: &Subject,
        parser: AttachmentParser,
        focus: Option<&str>,
        content: &str,
    ) -> Result<Perception> {
        // The row that comes out is the one this call wrote, not the one it read: the content is what
        // the caller asked for either way. The only way the update finds nothing is a concurrent
        // delete, and then a fresh row is what the caller wants.
        let perception = match self.repos.perception.find(subject, parser, focus).await? {
            Some(existing) => {
                self.repos
                    .perception
                    .update_content(existing.id, content)
                    .await?
            }
            None => None,
        };
        let perception = match perception {
            Some(updated) => updated,
            None => {
                self.repos
                    .perception
                    .create(subject, parser, focus, content)
                    .await?
            }
        };
        self.spawn_embedding(perception.id, content);
        Ok(perception)
    }

    /// Fire-and-forget on purpose: a perception row is a cache, so a missing vector only costs the
    /// next read a recomputation.
    fn spawn_embedding(&self, id: PerceptionId, content: &str) {
        let embedding = Arc::clone(&self.embedding);
        let repo = self.repos.perception.clone();
        let content = content.to_string();
        tokio::spawn(async move {
            let vector = match embedding.generate_embedding(&content).await {
                Ok(vector) => vector,
                Err(e) => {
                    tracing::warn!(%id, "Failed to embed perception: {e}");
                    return;
                }
            };
            if let Err(e) = repo.store_embedding(id, &vector).await {
                tracing::warn!(%id, "Failed to store perception embedding: {e}");
            }
        });
    }
}
