use crate::{
    agent::context::fmt::format_time_dyn,
    agentcore::render::elements::Node,
    domain::model::{Topic, TopicNote},
};

pub fn topic_element(topic: &Topic, need_close: bool) -> Node {
    let mut el = Node::tag("topic");
    if need_close {
        el = el.attr("need-close", true);
    }
    el.attr("id", topic.id.0)
        .attr("status", topic.status.as_str())
        .attr("started_at", format_time_dyn(topic.started_at))
        .attr("last_active", format_time_dyn(topic.last_active_at))
        .attr("title", topic.title.as_deref().unwrap_or("No Title"))
        .child(Node::text(topic.summary.as_deref().unwrap_or("No Summary")))
}

pub fn topic_record(topic: &Topic, notes: &[TopicNote]) -> Node {
    let mut el = topic_element(topic, false);
    if let (Some(first), Some(last)) = (topic.first_message_id, topic.last_message_id) {
        el = el.attr("messages", format!("{first}-{last}"));
    }
    el.children(notes.iter().map(note_element).collect())
}

fn note_element(note: &TopicNote) -> Node {
    Node::tag("note")
        .attr("id", note.id)
        .attr("at", format_time_dyn(note.created_at))
        .child(Node::text(&note.text))
}
