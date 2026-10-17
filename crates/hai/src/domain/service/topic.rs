use std::sync::Arc;

use super::hydrate;
use crate::{
    agentcore::api::embedding::EmbeddingService,
    domain::{
        model::{Topic, TopicNote, embed_text},
        repo::Repos,
        vo::{ChatId, MessageId, TimeRange, TopicId},
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct TopicService {
    repos: Repos,
    embedding: Arc<dyn EmbeddingService>,
}

impl TopicService {
    pub fn new(repos: Repos, embedding: Arc<dyn EmbeddingService>) -> Self {
        Self { repos, embedding }
    }

    pub async fn open(
        &self,
        chat_id: ChatId,
        title: &str,
        summary: &str,
        message_ids: &[MessageId],
    ) -> Result<Topic> {
        let embedding = self
            .embedding
            .generate_embedding(&embed_text(title, summary))
            .await?;
        self.repos
            .topic
            .open(chat_id, title, summary, &embedding, message_ids)
            .await
    }

    pub async fn note(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        text: &str,
        message_ids: &[MessageId],
    ) -> Result<TopicNote> {
        self.repos
            .topic
            .add_note(chat_id, topic_id, text, message_ids)
            .await?
            .ok_or_else(|| missing(topic_id))
    }

    /// A closed topic is still writable; the retrieval embedding is recomputed.
    pub async fn revise(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        title: Option<&str>,
        summary: Option<&str>,
    ) -> Result<Topic> {
        // The embedding describes what the row will say, so the half that is not being changed comes
        // from the row; the write itself takes the same halves and coalesces.
        let current = self.get(chat_id, topic_id).await?;
        let effective_title = title.or(current.title.as_deref()).unwrap_or_default();
        let effective_summary = summary.or(current.summary.as_deref()).unwrap_or_default();
        let embedding = self
            .embedding
            .generate_embedding(&embed_text(effective_title, effective_summary))
            .await?;
        self.repos
            .topic
            .update_fields(chat_id, topic_id, title, summary, &embedding)
            .await?
            .ok_or_else(|| missing(topic_id))
    }

    pub async fn close(&self, chat_id: ChatId, topic_id: TopicId) -> Result<Topic> {
        self.repos
            .topic
            .close(chat_id, topic_id)
            .await?
            .ok_or_else(|| missing(topic_id))
    }

    pub async fn link(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        message_ids: &[MessageId],
    ) -> Result<Topic> {
        if !self
            .repos
            .topic
            .link_messages(chat_id, topic_id, message_ids)
            .await?
        {
            return Err(missing(topic_id));
        }
        // The link refreshed the row's time range, so the caller reads what it just wrote.
        self.get(chat_id, topic_id).await
    }

    pub async fn drop(&self, chat_id: ChatId, topic_id: TopicId) -> Result<()> {
        if self.repos.topic.delete(chat_id, topic_id).await? {
            Ok(())
        } else {
            Err(missing(topic_id))
        }
    }

    pub async fn get(&self, chat_id: ChatId, topic_id: TopicId) -> Result<Topic> {
        self.repos
            .topic
            .by_id(chat_id, topic_id)
            .await?
            .ok_or_else(|| missing(topic_id))
    }

    pub async fn notes(
        &self,
        topic_id: TopicId,
        offset: i64,
        limit: i64,
    ) -> Result<(Vec<TopicNote>, i64)> {
        let notes = self.repos.topic.notes(topic_id, offset, limit).await?;
        let total = self.repos.topic.notes_count(topic_id).await?;
        Ok((notes, total))
    }

    pub async fn notes_count(&self, topic_id: TopicId) -> Result<i64> {
        self.repos.topic.notes_count(topic_id).await
    }

    pub async fn active_topics(&self, chat_id: ChatId) -> Result<Vec<Topic>> {
        self.repos.topic.active_by_chat(chat_id).await
    }

    pub async fn list_by_time(
        &self,
        chat_id: ChatId,
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Topic>> {
        self.repos.topic.by_chat_time(chat_id, range, limit).await
    }

    pub async fn search_keywords(
        &self,
        chat_id: ChatId,
        keywords: &[String],
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Topic>> {
        self.repos
            .topic
            .search_by_keywords(chat_id, keywords, range, limit)
            .await
    }

    pub async fn search_related(
        &self,
        chat_id: ChatId,
        query: &[f32],
        limit: i64,
    ) -> Result<Vec<Topic>> {
        let rows = self
            .repos
            .topic
            .search_by_embedding(chat_id, query, limit)
            .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<TopicId> = rows.iter().map(|(id, _)| *id).collect();
        let by_id = self.repos.topic.by_ids(&ids).await?;
        Ok(hydrate::merge_hydrated(
            rows,
            by_id.into_iter().map(|topic| (topic.id, topic)).collect(),
        )
        .into_iter()
        .map(|(topic, _)| topic)
        .collect())
    }
}

fn missing(topic_id: TopicId) -> AppError {
    AppError::message(format!("Topic not found: {topic_id}"))
}
