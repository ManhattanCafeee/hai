use crate::{agentcore::render::Node, domain::model::Perception};

pub fn perception_item(p: &Perception) -> Node {
    let mut el = Node::tag("perception").attr("id", p.id.0);
    if let Some(focus) = &p.focus {
        el = el.attr("focus", focus.as_str());
    }
    el.child(Node::text(&p.content))
}
