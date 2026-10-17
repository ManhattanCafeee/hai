use std::borrow::Cow;

use crate::{
    domain::{
        model::{Memory, Topic},
        repo::lexical::{MAX_LEXICAL_TERMS, lexical_terms},
        service::{
            Services,
            fuse::reciprocal_rank_fusion,
            rank::{self, ScorerErrorPolicy},
        },
        vo::{ChatId, MemoryId, TimeRange, TopicId},
    },
    error::Result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallScope {
    All,
    Memory,
    Topic,
}

impl RecallScope {
    fn includes_memory(self) -> bool {
        matches!(self, Self::All | Self::Memory)
    }

    fn includes_topic(self) -> bool {
        matches!(self, Self::All | Self::Topic)
    }
}

#[derive(Debug)]
pub struct RecallQuery {
    /// Semantic question, embedded for the vector search.
    pub query: Option<String>,
    /// Literal keywords (names, error strings, proper nouns); substring matched, not embedded.
    /// Empty keywords fall back to terms derived from `query`.
    pub keywords: Vec<String>,
    pub scope: RecallScope,
    pub range: TimeRange,
    pub limit: i64,
    pub on_error: ScorerErrorPolicy,
}

#[derive(Debug, Clone)]
pub enum RecallItem {
    Memory(Memory),
    Topic(Topic),
}

impl RecallItem {
    fn key(&self) -> RecallKey {
        match self {
            Self::Memory(memory) => RecallKey::Memory(memory.id),
            Self::Topic(topic) => RecallKey::Topic(topic.id),
        }
    }

    /// The text the reranker scores this item by.
    fn text(&self) -> String {
        match self {
            Self::Memory(memory) => memory.content.clone(),
            Self::Topic(topic) => {
                let title = topic.title.as_deref().unwrap_or_default();
                let summary = topic.summary.as_deref().unwrap_or_default();
                format!("{title} {summary}").trim().to_owned()
            }
        }
    }

    fn at(&self) -> jiff::Timestamp {
        match self {
            Self::Memory(memory) => memory.created_at,
            Self::Topic(topic) => topic.started_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RecallKey {
    Memory(MemoryId),
    Topic(TopicId),
}

/// Recalls this chat's memories and topics, best first: the vector and lexical legs are fused by
/// rank, and a configured reranker reorders the fusion.
pub async fn recall(
    services: &Services,
    chat_id: ChatId,
    query: RecallQuery,
) -> Result<Vec<RecallItem>> {
    let pool = rank::pool(services.scorer.as_ref(), query.limit);
    let fetch = (query.limit * 2).max(10).max(pool as i64);
    let mut lists: Vec<Vec<RecallItem>> = Vec::new();
    let mut keywords = query.keywords.clone();
    if keywords.is_empty()
        && let Some(text) = query.query.as_deref()
    {
        keywords = lexical_terms(text, MAX_LEXICAL_TERMS);
    }

    if let Some(text) = query
        .query
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        let embedding = services.embedding.generate_embedding(text).await?;
        if query.scope.includes_memory() {
            lists.push(
                services
                    .memory
                    .search_related(chat_id, &embedding, fetch)
                    .await?
                    .into_iter()
                    .map(RecallItem::Memory)
                    .collect(),
            );
        }
        if query.scope.includes_topic() {
            lists.push(
                services
                    .topic
                    .search_related(chat_id, &embedding, fetch)
                    .await?
                    .into_iter()
                    .map(RecallItem::Topic)
                    .collect(),
            );
        }
    }

    if !keywords.is_empty() {
        if query.scope.includes_memory() {
            lists.push(
                services
                    .memory
                    .search_keywords(chat_id, &keywords, query.range, fetch)
                    .await?
                    .into_iter()
                    .map(RecallItem::Memory)
                    .collect(),
            );
        }
        if query.scope.includes_topic() {
            lists.push(
                services
                    .topic
                    .search_keywords(chat_id, &keywords, query.range, fetch)
                    .await?
                    .into_iter()
                    .map(RecallItem::Topic)
                    .collect(),
            );
        }
    }

    if lists.is_empty() {
        return list_by_time(services, chat_id, &query, fetch).await;
    }

    let fusion = reciprocal_rank_fusion(lists, RecallItem::key, pool);
    let ranked = match (&services.scorer, query.query.as_deref()) {
        (Some(scorer), Some(text)) => {
            rank::rerank(
                scorer,
                text,
                fusion,
                |item| Cow::Owned(item.text()),
                query.on_error,
                "recall",
            )
            .await?
        }
        _ => fusion,
    };
    Ok(items_from(ranked, query.limit))
}

fn items_from(fusion: Vec<(RecallItem, f64)>, limit: i64) -> Vec<RecallItem> {
    fusion
        .into_iter()
        .take(limit as usize)
        .map(|(item, _)| item)
        .collect()
}

async fn list_by_time(
    services: &Services,
    chat_id: ChatId,
    query: &RecallQuery,
    fetch: i64,
) -> Result<Vec<RecallItem>> {
    let mut items: Vec<RecallItem> = Vec::new();
    if query.scope.includes_memory() {
        items.extend(
            services
                .memory
                .list_by_time(chat_id, query.range, fetch)
                .await?
                .into_iter()
                .map(RecallItem::Memory),
        );
    }
    if query.scope.includes_topic() {
        items.extend(
            services
                .topic
                .list_by_time(chat_id, query.range, fetch)
                .await?
                .into_iter()
                .map(RecallItem::Topic),
        );
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.at()));
    items.truncate(query.limit as usize);
    Ok(items)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{agentcore::api::rerank::Scorer, domain::test_support, test_support::FakeRerank};

    fn memory(id: u128) -> RecallItem {
        RecallItem::Memory(test_support::memory(id, &format!("memory {id}")))
    }

    fn topic(id: u128) -> RecallItem {
        RecallItem::Topic(test_support::topic(id, &format!("topic {id}")))
    }

    fn fusion() -> Vec<(RecallItem, f64)> {
        vec![(memory(1), 0.02), (memory(2), 0.01), (topic(3), 0.005)]
    }

    fn ids(items: &[RecallItem]) -> Vec<u128> {
        items
            .iter()
            .map(|item| match item {
                RecallItem::Memory(memory) => memory.id.0.as_u128(),
                RecallItem::Topic(topic) => topic.id.0.as_u128(),
            })
            .collect()
    }

    /// A rerank transport that answers whatever the test hands it: `None` is an outage.
    fn scorer(scores: Option<Vec<f64>>) -> Scorer {
        let service = match scores {
            Some(scores) => FakeRerank::scores(scores),
            None => FakeRerank::down(),
        };
        Scorer::new(Arc::new(service), 10, 500)
    }

    fn text_of(item: &RecallItem) -> Cow<'_, str> {
        Cow::Owned(item.text())
    }

    #[tokio::test]
    async fn a_rerank_reorders_and_keeps_the_limit() {
        let ranked = rank::rerank(
            &scorer(Some(vec![0.1, 0.9, 0.4])),
            "q",
            fusion(),
            text_of,
            ScorerErrorPolicy::Fail,
            "recall",
        )
        .await
        .unwrap();

        assert_eq!(ids(&items_from(ranked, 2)), [2, 3]);
    }

    #[tokio::test]
    async fn a_rerank_failure_surfaces_when_the_policy_says_so() {
        let err = rank::rerank(
            &scorer(None),
            "q",
            fusion(),
            text_of,
            ScorerErrorPolicy::Fail,
            "recall",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("rerank is down"));
    }

    #[tokio::test]
    async fn the_chapter_policy_keeps_the_fusion_order() {
        let ranked = rank::rerank(
            &scorer(None),
            "q",
            fusion(),
            text_of,
            ScorerErrorPolicy::KeepFusionOrder,
            "recall",
        )
        .await
        .unwrap();

        assert_eq!(ids(&items_from(ranked, 2)), [1, 2]);
    }

    /// A response whose shape breaks the contract is a bug, not an outage: it surfaces even for the
    /// policy that tolerates a failed rerank.
    #[tokio::test]
    async fn a_score_count_mismatch_is_a_contract_error() {
        assert!(
            rank::rerank(
                &scorer(Some(vec![0.5])),
                "q",
                fusion(),
                text_of,
                ScorerErrorPolicy::Fail,
                "recall",
            )
            .await
            .is_err(),
            "a score count that does not match the candidates"
        );
        assert!(
            rank::rerank(
                &scorer(Some(vec![0.5])),
                "q",
                fusion(),
                text_of,
                ScorerErrorPolicy::KeepFusionOrder,
                "recall",
            )
            .await
            .is_err(),
            "the tolerant policy covers transport failures, not contract violations"
        );
    }
}
