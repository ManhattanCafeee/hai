//! Tavily Search API: returns an answer plus results, over REST rather than MCP.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    Recency, SearchEngine, SearchRequest, SearchResponse, SearchResult, limit_of, snippet,
};
use crate::{
    agent::web::provider::Provider,
    config::{ConfigIssue, schema::WebProvider},
    error::{AppError, Result, bail},
    util::text::truncate_chars,
};

pub struct Tavily {
    provider: Arc<Provider>,
}

impl Tavily {
    pub fn new(provider: Arc<Provider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl SearchEngine for Tavily {
    fn kind(&self) -> WebProvider {
        self.provider.id()
    }

    async fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        let spec = self.provider.spec();
        let body = RequestBody {
            query: &req.query,
            max_results: limit_of(req, spec.max_results),
            include_answer: true,
            time_range: req.recency.map(Recency::label),
        };
        let api_key = self.provider.api_key().ok_or_else(|| {
            ConfigIssue::web_provider(
                &spec.label.to_lowercase(),
                format!(
                    "requires a credential: set `api-key` here or {}",
                    spec.env_key
                ),
            )
        })?;
        let resp = self
            .provider
            .http()
            .post(spec.endpoint)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!(AppError::message(format!(
                "Tavily HTTP {status}: {}",
                truncate_chars(&body, 300)
            )));
        }
        parse(&body)
    }
}

#[derive(Serialize)]
struct RequestBody<'a> {
    query: &'a str,
    max_results: u32,
    include_answer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<&'static str>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    answer: Option<String>,
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
    content: String,
}

fn parse(body: &str) -> Result<SearchResponse> {
    let response: Response = serde_json::from_str(body)?;
    Ok(SearchResponse {
        answer: response.answer.filter(|a| !a.trim().is_empty()),
        results: response
            .results
            .into_iter()
            .map(|item| SearchResult {
                title: item.title.trim().to_owned(),
                url: item.url,
                snippet: snippet(&item.content),
                age: None,
            })
            .collect(),
        notes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maps_answer_and_results() {
        let body = r#"{
            "answer": "conclusion",
            "results": [
                { "title": "A title", "url": "https://a.example/", "content": "body summary", "score": 0.9 }
            ]
        }"#;
        let resp = parse(body).expect("valid response");
        assert_eq!(resp.answer.as_deref(), Some("conclusion"));
        assert_eq!(resp.results[0].title, "A title");
        assert_eq!(resp.results[0].snippet.as_deref(), Some("body summary"));
    }

    #[test]
    fn parse_drops_blank_answer() {
        let resp = parse(r#"{"answer": "  ", "results": []}"#).expect("valid response");
        assert!(resp.answer.is_none());
    }
}
