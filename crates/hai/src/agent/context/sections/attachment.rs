use crate::{
    agent::context::RenderContext,
    agentcore::render::elements::Node,
    domain::model::{Attachment, Perception},
};

/// One `<attachment>` element: the file, its cached analyses, and the platform extras.
pub(crate) fn attachment_element(att: &Attachment, ctx: &RenderContext) -> Node {
    let mut element = Node::tag("attachment")
        .attr("id", att.id.to_string())
        .attr("type", att.kind.name());

    if let Some(perceptions) = ctx.perception_by_attachment.get(&att.id) {
        for p in perceptions.iter().filter(|p| p.focus.is_none()) {
            element = element.child(analysis_element(p, None));
        }
        for (p, focus) in perceptions
            .iter()
            .filter_map(|p| p.focus.as_deref().map(|f| (p, f)))
        {
            element = element.child(analysis_element(p, Some(focus)));
        }
    }

    if let Some(&first_id) = ctx.same_resource_as.get(&att.id) {
        element = element.attr("same_resource_as", first_id.to_string());
    }

    if let Some(hint) = att.meta.hint.as_deref() {
        element = element.attr("hint", hint);
    }

    if let Some(caption) = att.caption.as_deref() {
        element = element.attr("caption", caption);
    }

    if let Some(prompt) = att.meta.prompt.as_deref() {
        element = element.attr("prompt", prompt);
    }

    element
}

/// The base transcription has no focus attribute; a targeted reading carries the focus it answered.
fn analysis_element(perception: &Perception, focus: Option<&str>) -> Node {
    let mut element = Node::tag("analysis")
        .attr("parser", perception.parser.name())
        .child(Node::text(&perception.content));
    if let Some(focus) = focus {
        element = element.attr("focus", focus);
    }
    element
}
