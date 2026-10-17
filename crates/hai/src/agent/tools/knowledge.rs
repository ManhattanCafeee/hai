use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    agent::{
        context::sections::knowledge::related_knowledge_section,
        runtime::{AgentRuntime, turn::ToolContext},
        tools::util::bounded_limit,
    },
    agentcore::{
        render::render_json,
        tool::{AgentTool, ToolError, tool_data},
    },
    domain::{
        service::Services,
        vo::{Collection, CollectionName},
    },
};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchKnowledgeBaseArgs {
    pub query: String,
    pub limit: Option<u32>,
    #[schemars(with = "Option<String>")]
    pub collection: Option<CollectionName>,
}

/// Searches the global knowledge base by meaning and by keyword, then reorders the candidates by
/// relevance. The knowledge base is long-term curated material (not conversation memory); use it
/// when an answer needs outside knowledge.
#[hai_macros::tool]
pub struct SearchKnowledgeBase {
    pub services: Services,
}

impl SearchKnowledgeBase {
    async fn exec(&self, args: SearchKnowledgeBaseArgs) -> Result<Value, ToolError> {
        let limit = i64::from(bounded_limit(args.limit, 5));
        let collections: Vec<Collection> = args
            .collection
            .map(|name| vec![Collection::Named(name)])
            .unwrap_or_default();
        let hits = self
            .services
            .knowledge
            .search(&args.query, limit, &collections)
            .await?;

        let section = related_knowledge_section(&hits);
        let count = hits.len();
        tool_data(serde_json::json!({
            "knowledge": render_json(&section),
            "count": count,
        }))
    }
}

pub fn tools(runtime: &AgentRuntime, ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    if !runtime.cfg().knowledge.enabled {
        return Vec::new();
    }
    vec![Arc::new(SearchKnowledgeBase {
        services: ctx.services.clone(),
    })]
}
