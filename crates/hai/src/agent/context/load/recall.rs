use std::collections::HashSet;

use crate::{
    domain::{
        model::{Memory, Topic},
        service::{RecallItem, RecallQuery, RecallScope, ScorerErrorPolicy, Services, recall},
        vo::{ChatId, MemoryId, TimeRange, TopicId},
    },
    error::Result,
};

/// Automatic recall counts for a chapter's first turn.
const MEMORY_LIMIT: usize = 3;
const TOPIC_LIMIT: usize = 2;

#[derive(Debug, Default)]
pub struct ChapterRecall {
    pub memories: Vec<Memory>,
    pub topics: Vec<Topic>,
}

pub async fn chapter_recall(
    services: &Services,
    chat_id: ChatId,
    window_text: &str,
    current_memories: &[Memory],
    current_topics: &[Topic],
) -> Result<ChapterRecall> {
    let query = window_text.trim();
    if query.is_empty() {
        return Ok(ChapterRecall::default());
    }

    let exclude_memories: HashSet<MemoryId> = current_memories.iter().map(|m| m.id).collect();
    let exclude_topics: HashSet<TopicId> = current_topics.iter().map(|t| t.id).collect();

    let recalled = recall(
        services,
        chat_id,
        RecallQuery {
            query: Some(query.to_owned()),
            keywords: Vec::new(),
            scope: RecallScope::All,
            range: TimeRange::default(),
            limit: (MEMORY_LIMIT + TOPIC_LIMIT + exclude_memories.len() + exclude_topics.len() + 1)
                as i64,
            on_error: ScorerErrorPolicy::KeepFusionOrder,
        },
    )
    .await?;

    Ok(ChapterRecall {
        memories: pick_memories(&recalled, &exclude_memories, MEMORY_LIMIT),
        topics: pick_topics(&recalled, &exclude_topics, TOPIC_LIMIT),
    })
}

fn pick_memories(items: &[RecallItem], exclude: &HashSet<MemoryId>, limit: usize) -> Vec<Memory> {
    items
        .iter()
        .filter_map(|item| match item {
            RecallItem::Memory(memory) => Some(memory.clone()),
            RecallItem::Topic(_) => None,
        })
        .filter(|memory| !exclude.contains(&memory.id))
        .take(limit)
        .collect()
}

fn pick_topics(items: &[RecallItem], exclude: &HashSet<TopicId>, limit: usize) -> Vec<Topic> {
    items
        .iter()
        .filter_map(|item| match item {
            RecallItem::Topic(topic) => Some(topic.clone()),
            RecallItem::Memory(_) => None,
        })
        .filter(|topic| !exclude.contains(&topic.id))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(id: u128) -> Memory {
        crate::domain::test_support::memory(id, &format!("memory {id}"))
    }

    fn topic(id: u128) -> Topic {
        crate::domain::test_support::topic(id, &format!("topic {id}"))
    }

    /// Entries already in `<current>` must not reappear in `<recall>`, which would inject them twice
    /// into the same prompt.
    #[test]
    fn recall_picks_skip_entries_already_in_current() {
        let excluded = memory(1);
        let picked = pick_memories(
            &[
                RecallItem::Memory(excluded.clone()),
                RecallItem::Memory(memory(2)),
                RecallItem::Topic(topic(9)),
                RecallItem::Memory(memory(3)),
                RecallItem::Memory(memory(4)),
            ],
            &[excluded.id].into_iter().collect(),
            3,
        );
        let ids: Vec<u128> = picked.iter().map(|m| m.id.0.as_u128()).collect();
        assert_eq!(ids, [2, 3, 4]);
    }

    #[test]
    fn recall_picks_keep_order_and_respect_limit() {
        let picked = pick_topics(
            &[
                RecallItem::Topic(topic(1)),
                RecallItem::Memory(memory(7)),
                RecallItem::Topic(topic(2)),
                RecallItem::Topic(topic(3)),
            ],
            &HashSet::new(),
            2,
        );
        let ids: Vec<u128> = picked.iter().map(|t| t.id.0.as_u128()).collect();
        assert_eq!(ids, [1, 2]);
    }
}
