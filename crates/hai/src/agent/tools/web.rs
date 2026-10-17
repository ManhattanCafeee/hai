use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    agent::{
        runtime::AgentRuntime,
        tools::util::bounded_limit,
        web::{Recency, SearchRequest, SearchResponse, WebService},
    },
    agentcore::{
        render::{Node, render_json},
        tool::{AgentTool, ToolError, schema_of, tool_data},
    },
    util::text::truncate_chars,
};

const DEFAULT_RESULTS: u32 = 10;
const MAX_ANSWER_CHARS: usize = 2000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebSearchArgs {
    pub query: String,
    /// Restrict results to those published within this window
    pub recency: Option<Recency>,
    pub limit: Option<u32>,
}

pub struct WebSearch {
    description: String,
    web: WebService,
}

#[async_trait]
impl AgentTool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Option<Value> {
        Some(schema_of::<WebSearchArgs>())
    }

    async fn execute(&self, args: Value) -> Result<Value, ToolError> {
        let args: WebSearchArgs = serde_json::from_value(args)?;
        let request = SearchRequest {
            query: args.query,
            recency: args.recency,
            limit: bounded_limit(args.limit, DEFAULT_RESULTS),
        };
        let response = self.web.search(&request).await?;
        tool_data(json!({
            "search": render_json(&section(&response)),
            "count": response.results.len(),
        }))
    }
}

pub fn tools(runtime: &AgentRuntime) -> Vec<Arc<dyn AgentTool>> {
    if !runtime.web.search_available() {
        return Vec::new();
    }
    vec![Arc::new(WebSearch {
        description: "Searches the web. It returns only the answer, titles, links and snippets, never page bodies. The query understands `site:`/`inurl:`/`intitle:`/`filetype:`, `\"phrase\"` and `-exclusion` constraints (the provider filters on them); the publication window is set with `recency`.".to_owned(),
        web: runtime.web.clone(),
    })]
}

fn section(response: &SearchResponse) -> Node {
    let mut node = Node::tag("search");
    if let Some(answer) = &response.answer {
        node = node
            .child(Node::tag("answer").child(Node::text(truncate_chars(answer, MAX_ANSWER_CHARS))));
    }
    for note in &response.notes {
        node = node.child(Node::tag("note").child(Node::text(note.clone())));
    }
    node = node.children(
        response
            .results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                let mut item = Node::tag("result")
                    .attr("rank", index as i64 + 1)
                    .attr("title", result.title.clone())
                    .attr("url", result.url.clone());
                if let Some(age) = &result.age {
                    item = item.attr("age", age.clone());
                }
                match &result.snippet {
                    Some(snippet) => item.child(Node::text(snippet.clone())),
                    None => item,
                }
            })
            .collect(),
    );
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_renders_answer_notes_and_results() {
        let response: SearchResponse = serde_json::from_value(json!({
            "answer": "conclusion",
            "results": [
                { "title": "Title", "url": "https://a.example/", "snippet": "snippet", "age": "2026-01-01" }
            ],
            "notes": ["site:docs.rs matched nothing; constraint relaxed"]
        }))
        .expect("valid response");
        let value = render_json(&section(&response));
        assert_eq!(
            value,
            json!({
                "_tag": "search",
                "children": [
                    { "_tag": "answer", "children": ["conclusion"] },
                    { "_tag": "note", "children": ["site:docs.rs matched nothing; constraint relaxed"] },
                    {
                        "_tag": "result",
                        "rank": 1,
                        "title": "Title",
                        "url": "https://a.example/",
                        "age": "2026-01-01",
                        "children": ["snippet"]
                    }
                ]
            })
        );
    }
}
