//! Web search: a provider fallback chain, tried in order until one returns results.

mod mcp;
mod query;
mod tavily;

use std::{borrow::Cow, sync::Arc, time::Duration};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::provider::Providers;
use crate::{
    config::schema::{WebProvider, WebSearchConfig},
    error::{AppError, Result, bail},
    util::text::truncate_chars,
};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    strum::IntoStaticStr,
    strum::EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum Recency {
    Day,
    Week,
    Month,
    Year,
}

impl Recency {
    /// Wire form: embedded in the query hint text and sent as Tavily's `time_range`.
    pub fn label(self) -> &'static str {
        self.into()
    }
}

pub(super) fn limit_of(req: &SearchRequest, max_results: u32) -> u32 {
    req.limit.clamp(1, max_results)
}

pub(super) fn with_recency(query: &str, recency: Option<Recency>) -> Cow<'_, str> {
    match recency {
        Some(recency) => {
            let label = recency.label();
            Cow::Owned(format!("{query} (prefer results from the last {label})"))
        }
        None => Cow::Borrowed(query),
    }
}

pub struct SearchRequest {
    pub query: String,
    pub recency: Option<Recency>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub age: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SearchResponse {
    pub answer: Option<String>,
    pub results: Vec<SearchResult>,
    /// Local filter constraints that were relaxed because they matched nothing (`site:`/`inurl:`/…).
    /// These reach the model, so the wording is part of the tool output.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn kind(&self) -> WebProvider;

    async fn search(&self, req: &SearchRequest) -> Result<SearchResponse>;
}

#[derive(Clone)]
pub struct SearchChain {
    engines: Arc<[Box<dyn SearchEngine>]>,
    timeout: Duration,
}

impl SearchChain {
    pub fn from_providers(providers: &Providers, cfg: &WebSearchConfig) -> Result<Self> {
        let instances = providers.search_instances(cfg);
        let mut engines: Vec<Box<dyn SearchEngine>> = Vec::with_capacity(instances.len());
        for provider in instances {
            let engine: Box<dyn SearchEngine> = match provider.spec().search.as_ref() {
                Some(capability) => {
                    Box::new(mcp::McpSearch::new(Arc::clone(provider.mcp()?), capability))
                }
                // Tavily is the one provider that answers search over REST rather than MCP.
                None => Box::new(tavily::Tavily::new(provider)),
            };
            engines.push(engine);
        }
        Ok(Self::new(engines, Duration::from_secs(cfg.timeout_secs)))
    }

    fn new(engines: Vec<Box<dyn SearchEngine>>, timeout: Duration) -> Self {
        Self {
            engines: Arc::from(engines),
            timeout,
        }
    }

    pub fn available(&self) -> bool {
        !self.engines.is_empty()
    }

    pub async fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        if self.engines.is_empty() {
            bail!(AppError::message("No web search provider configured"));
        }
        let query = query::Query::parse(&req.query);
        let asked = SearchRequest {
            query: req.query.clone(),
            recency: req.recency,
            limit: query.widen(req.limit),
        };
        let mut failures = Vec::new();
        for engine in self.engines.iter() {
            let kind = engine.kind();
            match tokio::time::timeout(self.timeout, engine.search(&asked)).await {
                Ok(Ok(mut resp)) if !resp.results.is_empty() => {
                    let (results, notes) = query::apply(&query, resp.results);
                    resp.results = results.into_iter().take(req.limit as usize).collect();
                    resp.notes = notes;
                    return Ok(resp);
                }
                Ok(Ok(_)) => failures.push(format!("{kind}: empty result")),
                Ok(Err(e)) => {
                    tracing::warn!(provider = %kind, error = %e, "Web search failed");
                    failures.push(format!("{kind}: {e}"));
                }
                Err(_) => failures.push(format!("{kind}: timeout")),
            }
        }
        Err(AppError::message(format!(
            "All web search providers failed: {}",
            failures.join("; ")
        )))
    }
}

pub(super) fn snippet(text: &str) -> Option<String> {
    let cleaned = super::html::strip_tags(text);
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| truncate_chars(trimmed, 240))
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::test_support::assert_names_agree;

    /// The four outcomes the fallback chain has to distinguish, driven by a fake engine so the
    /// tests stay off the network.
    enum Outcome {
        Results(usize),
        Empty,
        Fail,
        Hang,
    }

    struct Fake {
        provider: WebProvider,
        outcome: Outcome,
    }

    #[async_trait]
    impl SearchEngine for Fake {
        fn kind(&self) -> WebProvider {
            self.provider
        }

        async fn search(&self, _req: &SearchRequest) -> Result<SearchResponse> {
            match self.outcome {
                Outcome::Results(count) => Ok(SearchResponse {
                    results: (0..count)
                        .map(|i| SearchResult {
                            title: format!("t{i}"),
                            url: format!("https://example.com/{i}"),
                            snippet: None,
                            age: None,
                        })
                        .collect(),
                    ..SearchResponse::default()
                }),
                Outcome::Empty => Ok(SearchResponse::default()),
                Outcome::Fail => Err(AppError::message("boom")),
                Outcome::Hang => {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(SearchResponse::default())
                }
            }
        }
    }

    fn chain(engines: Vec<(WebProvider, Outcome)>, timeout_ms: u64) -> SearchChain {
        let engines: Vec<Box<dyn SearchEngine>> = engines
            .into_iter()
            .map(|(provider, outcome)| {
                Box::new(Fake { provider, outcome }) as Box<dyn SearchEngine>
            })
            .collect();
        SearchChain::new(engines, Duration::from_millis(timeout_ms))
    }

    fn req() -> SearchRequest {
        SearchRequest {
            query: "q".to_owned(),
            recency: None,
            limit: 5,
        }
    }

    #[test]
    fn recency_names_agree_between_serde_and_strum() {
        assert_names_agree(Recency::iter(), |recency| recency.label());
    }

    #[test]
    fn recency_maps_to_labels_and_query_hint() {
        assert_eq!(with_recency("q", None), "q");
        assert_eq!(
            with_recency("q", Some(Recency::Week)),
            "q (prefer results from the last week)"
        );
    }

    #[tokio::test]
    async fn empty_result_advances_to_the_next_engine() {
        let chain = chain(
            vec![
                (WebProvider::Parallel, Outcome::Empty),
                (WebProvider::Exa, Outcome::Results(2)),
            ],
            100,
        );
        let response = chain.search(&req()).await.expect("second engine answers");
        assert_eq!(response.results.len(), 2);
    }

    #[tokio::test]
    async fn first_results_win_and_later_engines_are_not_consulted() {
        let chain = chain(
            vec![
                (WebProvider::Parallel, Outcome::Results(1)),
                (WebProvider::Exa, Outcome::Fail),
            ],
            100,
        );
        assert_eq!(
            chain
                .search(&req())
                .await
                .expect("first engine answers")
                .results
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn hanging_engine_times_out_and_the_chain_advances() {
        let chain = chain(
            vec![
                (WebProvider::Parallel, Outcome::Hang),
                (WebProvider::Exa, Outcome::Results(1)),
            ],
            10,
        );
        assert_eq!(
            chain
                .search(&req())
                .await
                .expect("timeout advances")
                .results
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn all_failures_are_reported_per_provider() {
        let chain = chain(
            vec![
                (WebProvider::Parallel, Outcome::Fail),
                (WebProvider::Exa, Outcome::Empty),
            ],
            100,
        );
        let err = match chain.search(&req()).await {
            Err(e) => e,
            Ok(_) => panic!("every engine failed"),
        };
        let message = err.to_string();
        assert!(message.contains("parallel"), "{message}");
        assert!(message.contains("boom"), "{message}");
        assert!(
            message.contains("exa") && message.contains("empty result"),
            "{message}"
        );
    }
}
