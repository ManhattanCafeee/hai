use crate::{
    agent::context::fmt::format_time_dyn,
    agentcore::render::elements::Node,
    domain::{model::Memory, vo::MessageId},
};

pub fn memory_element(memory: &Memory) -> Node {
    Node::tag("memory")
        .attr("id", memory.id.0)
        .attr("kind", memory.kind.as_str())
        .attr("pinned", memory.pinned)
        .attr("at", format_time_dyn(memory.created_at))
        .child(Node::text(&memory.content))
}

pub fn memory_record(memory: &Memory) -> Node {
    let mut el = memory_element(memory).attr("status", memory.status.as_str());
    if !memory.source_message_ids.is_empty() {
        el = el.attr("messages", source_range(&memory.source_message_ids));
    }
    el
}

fn source_range(ids: &[MessageId]) -> String {
    let first = ids.iter().min().copied().unwrap_or_default();
    let last = ids.iter().max().copied().unwrap_or_default();
    format!("{first}-{last}")
}
