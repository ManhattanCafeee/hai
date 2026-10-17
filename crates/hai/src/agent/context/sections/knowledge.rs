use crate::{
    agentcore::render::elements::Node,
    domain::{service::knowledge::RelatedChunk, vo::Collection},
};

fn knowledge_item(chunk: &RelatedChunk) -> Node {
    let collection = match &chunk.collection {
        Collection::Uncategorised => "-",
        named => named.as_str(),
    };
    let mut item = Node::tag("knowledge_item")
        .attr("id", chunk.chunk_id.0)
        .attr("document", &chunk.document_title)
        .attr("collection", collection);
    if let Some(distance) = chunk.distance {
        item = item.attr("relevance", format!("{distance:.4}"));
    }
    if let Some(score) = chunk.score {
        item = item.attr("score", format!("{score:.4}"));
    }
    item.child(Node::text(&chunk.content))
}

pub fn related_knowledge_section(chunks: &[RelatedChunk]) -> Node {
    Node::tag("knowledge").children(chunks.iter().map(knowledge_item).collect::<Vec<_>>())
}
