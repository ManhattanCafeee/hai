use super::{ReadOutput, paging::Page};
use crate::{
    agent::context::sections::{memory::memory_record, topic::topic_record},
    agentcore::{render::render_xml, tool::ToolError},
    domain::{
        service::Services,
        vo::{ChatId, MemoryId, TopicId},
    },
};

pub(super) async fn topic(
    services: &Services,
    chat_id: ChatId,
    topic_id: TopicId,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<ReadOutput, ToolError> {
    let topic = services.topic.get(chat_id, topic_id).await?;

    let start = offset.unwrap_or(1).max(1) as usize - 1;
    let total = services.topic.notes_count(topic_id).await? as usize;
    if start >= total {
        return Ok(ReadOutput::Paged(Page::items_beyond(start, total)));
    }

    let limit = match limit {
        Some(limit) => (limit.max(1) as usize).min(total - start),
        None => total - start,
    };
    let (notes, _) = services
        .topic
        .notes(topic_id, start as i64, limit as i64)
        .await?;

    let shown = notes.len();
    let content = render_xml(&topic_record(&topic, &notes));
    Ok(ReadOutput::Paged(Page::items(content, start, total, shown)))
}

pub(super) async fn memory(
    services: &Services,
    chat_id: ChatId,
    memory_id: MemoryId,
) -> Result<ReadOutput, ToolError> {
    let memory = services.memory.get(chat_id, memory_id).await?;
    Ok(ReadOutput::Text {
        content: render_xml(&memory_record(&memory)),
    })
}
