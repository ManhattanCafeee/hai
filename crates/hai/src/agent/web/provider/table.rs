//! Provider table: each provider is declared once here (endpoint, tool names, capabilities,
//! credential requirement, result cap).

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    super::search::{snippet, with_recency},
    Fetch, Mcp, Search, Spec,
};
use crate::{
    agent::web::{SearchRequest, SearchResponse, search::SearchResult},
    config::schema::WebProvider,
    error::Result,
};

const PARALLEL: Spec = Spec {
    id: WebProvider::Parallel,
    label: "Parallel",
    endpoint: "https://search.parallel.ai/mcp",
    mcp: Some(Mcp {
        auth_header: "authorization",
        auth_scheme: Some("Bearer"),
    }),
    fetch: Some(Fetch {
        tool: "web_fetch",
        parse: parse_parallel_fetch,
        arguments: parallel_fetch_arguments,
    }),
    search: Some(Search {
        tool: "web_search",
        arguments: parallel_search_arguments,
        parse: parse_parallel_search,
    }),
    keyless: true,
    env_key: "PARALLEL_API_KEY",
    max_results: 40,
};

const EXA: Spec = Spec {
    id: WebProvider::Exa,
    label: "Exa",
    endpoint: "https://mcp.exa.ai/mcp",
    mcp: Some(Mcp {
        auth_header: "x-api-key",
        auth_scheme: None,
    }),
    fetch: Some(Fetch {
        tool: "web_fetch_exa",
        parse: parse_exa_fetch,
        arguments: no_fetch_arguments,
    }),
    search: Some(Search {
        tool: "web_search_exa",
        arguments: exa_search_arguments,
        parse: parse_exa_search,
    }),
    keyless: true,
    env_key: "EXA_API_KEY",
    max_results: 20,
};

const TAVILY: Spec = Spec {
    id: WebProvider::Tavily,
    label: "Tavily",
    endpoint: "https://api.tavily.com/search",
    mcp: None,
    fetch: None,
    search: None,
    keyless: false,
    env_key: "TAVILY_API_KEY",
    max_results: 20,
};

/// Parallel returns full page markdown only when asked, on top of the default excerpts.
fn parallel_fetch_arguments() -> Value {
    json!({ "full_content": true })
}

/// Providers whose fetch tool takes only `urls`.
fn no_fetch_arguments() -> Value {
    Value::Null
}

/// Parallel takes an objective plus the raw queries and truncates to its own default cap, so the
/// limit is the parser's business here.
fn parallel_search_arguments(req: &SearchRequest, _limit: u32) -> Value {
    json!({ "objective": with_recency(&req.query, req.recency), "search_queries": [req.query] })
}

/// Exa's search tool takes the requested count as `numResults`.
fn exa_search_arguments(req: &SearchRequest, limit: u32) -> Value {
    json!({ "query": with_recency(&req.query, req.recency), "numResults": limit })
}

pub(super) fn spec(id: WebProvider) -> &'static Spec {
    match id {
        WebProvider::Parallel => &PARALLEL,
        WebProvider::Exa => &EXA,
        WebProvider::Tavily => &TAVILY,
    }
}
pub(super) fn parse_parallel_fetch(body: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Payload {
        #[serde(default)]
        results: Vec<Item>,
    }

    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        excerpts: Vec<String>,
        #[serde(default)]
        full_content: Option<String>,
    }

    let payload: Payload = match serde_json::from_str(body) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::warn!(error = %e, "parallel web_fetch returned an unparsable payload");
            return None;
        }
    };
    let item = payload.results.into_iter().next()?;
    item.full_content
        .filter(|content| !content.trim().is_empty())
        .or_else(|| {
            let joined = item.excerpts.join("\n\n");
            (!joined.trim().is_empty()).then_some(joined)
        })
}

pub(super) fn parse_exa_fetch(body: &str) -> Option<String> {
    (!body.trim().is_empty()).then(|| body.trim().to_owned())
}

/// Parallel's search tool replies with JSON: `title`, `url`, `publish_date` and `excerpts` per
/// result; it truncates server-side, so the caller's limit is applied here.
pub(super) fn parse_parallel_search(text: &str, limit: u32) -> Result<SearchResponse> {
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        results: Vec<Item>,
    }

    #[derive(Deserialize)]
    struct Item {
        #[serde(default)]
        title: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        publish_date: Option<String>,
        #[serde(default)]
        excerpts: Vec<String>,
    }

    let payload: Payload = serde_json::from_str(text)?;
    let results = payload
        .results
        .into_iter()
        .take(limit as usize)
        .filter(|item| !item.url.is_empty())
        .map(|item| SearchResult {
            title: item.title.trim().to_owned(),
            url: item.url,
            snippet: snippet(&item.excerpts.join(" ")),
            age: item.publish_date,
        })
        .collect();
    Ok(SearchResponse {
        results,
        ..SearchResponse::default()
    })
}

/// The tool returns plain text, not JSON: blocks look like
/// `Title: …\nURL: …\nPublished: …\nHighlights:\n…` and are separated by `---`.
pub(super) fn parse_exa_search(text: &str, _limit: u32) -> Result<SearchResponse> {
    let results = text
        .split("\n---\n")
        .filter_map(|block| {
            let mut title = String::new();
            let mut url = String::new();
            let mut age = None;
            let mut highlights = None;
            let mut lines = block.lines();
            for line in lines.by_ref() {
                if let Some(value) = line.strip_prefix("Title: ") {
                    title = value.trim().to_owned();
                } else if let Some(value) = line.strip_prefix("URL: ") {
                    url = value.trim().to_owned();
                } else if let Some(value) = line.strip_prefix("Published: ") {
                    age = non_empty(value);
                } else if line.starts_with("Highlights:") {
                    highlights = Some(lines.collect::<Vec<_>>().join("\n"));
                    break;
                }
            }
            (!url.is_empty()).then(|| SearchResult {
                title,
                url,
                snippet: highlights.as_deref().and_then(snippet),
                age,
            })
        })
        .collect();
    Ok(SearchResponse {
        results,
        ..SearchResponse::default()
    })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty() && trimmed != "N/A").then(|| trimmed.to_owned())
}
