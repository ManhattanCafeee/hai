//! Providers: the identity, endpoint, tool names and credentials shared by search (`web_search`)
//! and fetch (the `read` engine chain).

mod key;
mod registry;
mod table;
mod transport;

use std::sync::Arc;

use serde_json::{Value, json};

use self::{key::Key, table::spec, transport::Transport};
use crate::{
    agent::web::{SearchRequest, SearchResponse, WebUrl},
    config::schema::{FetchEngine, WebProvider},
    error::{AppError, Result},
};

pub(super) struct Spec {
    pub id: WebProvider,
    pub label: &'static str,
    pub endpoint: &'static str,
    /// `None` = this provider does not speak MCP
    pub mcp: Option<Mcp>,
    /// `None` = no fetch capability
    pub fetch: Option<Fetch>,
    /// `None` = the provider answers search over REST rather than MCP
    pub search: Option<Search>,
    pub keyless: bool,
    /// Env var for the credential; it outranks the config `api-key`, and a blank value counts as
    /// unconfigured.
    pub env_key: &'static str,
    /// Per-search result cap, imposed by the provider API.
    pub max_results: u32,
}

pub(super) struct Mcp {
    pub auth_header: &'static str,
    /// Header value scheme prefix (e.g. `Bearer`); `None` sends the raw key.
    pub auth_scheme: Option<&'static str>,
}

/// Fetch capability: the tool name, its parser, and any provider-specific call arguments.
pub(super) struct Fetch {
    pub tool: &'static str,
    pub parse: fn(&str) -> Option<String>,
    /// Extra top-level arguments merged into the fetch call; `Value::Null` when the tool takes only
    /// `urls`.
    pub arguments: fn() -> Value,
}

/// Search capability: the MCP tool name, its arguments and its parser, declared by the table.
pub(super) struct Search {
    pub tool: &'static str,
    /// The `tools/call` arguments for one query; `limit` is the request's limit already clamped to
    /// the provider's `max_results`.
    pub arguments: fn(&SearchRequest, u32) -> Value,
    /// The tool's reply and the same clamped limit: parsing owns the limit where the provider
    /// truncates server-side.
    pub parse: fn(&str, u32) -> Result<SearchResponse>,
}
pub(super) struct Provider {
    id: WebProvider,
    http: reqwest::Client,
    mcp: Option<Arc<McpClient>>,
    key: Key,
}

pub(super) struct McpClient {
    spec: &'static Spec,
    transport: Transport,
}

impl McpClient {
    pub fn spec(&self) -> &'static Spec {
        self.spec
    }

    pub async fn call(&self, tool: &str, arguments: Value) -> Result<String> {
        self.transport.call(tool, arguments).await
    }

    /// A failure is only logged and degrades to `None` so the engine chain can keep descending.
    pub async fn fetch_page(&self, capability: &Fetch, url: &WebUrl) -> Option<String> {
        let arguments = fetch_arguments(capability, url);
        match self.transport.call(capability.tool, arguments).await {
            Ok(body) => (capability.parse)(&body),
            Err(error) => {
                tracing::warn!(
                    url = url.as_str(),
                    provider = self.spec.label,
                    %error,
                    "web fetch failed"
                );
                None
            }
        }
    }
}

/// The `tools/call` arguments for one URL: `urls` plus whatever the capability declares.
fn fetch_arguments(capability: &Fetch, url: &WebUrl) -> Value {
    let mut arguments = json!({ "urls": [url.as_str()] });
    let extra = (capability.arguments)();
    if let (Some(map), Value::Object(extra)) = (arguments.as_object_mut(), extra) {
        map.extend(extra);
    }
    arguments
}

impl Provider {
    pub fn id(&self) -> WebProvider {
        self.id
    }

    pub fn spec(&self) -> &'static Spec {
        spec(self.id)
    }

    pub fn api_key(&self) -> Option<&str> {
        self.key.value.as_deref()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// A table entry that declares MCP but has no transport is an assembly-time inconsistency, so
    /// reaching the error arm means the registry was built wrongly.
    pub fn mcp(&self) -> Result<&Arc<McpClient>> {
        self.mcp.as_ref().ok_or_else(|| {
            AppError::message(format!(
                "web provider `{}` has no MCP transport",
                self.spec().label
            ))
        })
    }
}
/// Built once at assembly time; it owns the shared HTTP client and every available instance, and
/// is the only place that decides who can search and who can fetch.
pub(super) struct Providers {
    http: reqwest::Client,
    instances: Vec<Arc<Provider>>,
}

pub(super) enum FetchStep {
    Local,
    Remote {
        client: Arc<McpClient>,
        capability: &'static Fetch,
    },
}

impl FetchStep {
    /// This step's identity in configuration vocabulary; it is what a `read` result reports as its
    /// `engine`.
    pub fn kind(&self) -> FetchEngine {
        match self {
            FetchStep::Local => FetchEngine::Local,
            FetchStep::Remote { client, .. } => FetchEngine::Provider(client.spec().id),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        key::{KeySource, resolve_key},
        registry::order,
        table::{
            parse_exa_fetch, parse_exa_search, parse_parallel_fetch, parse_parallel_search, spec,
        },
        *,
    };
    use crate::config::schema::{
        FetchEngine, WebConfig, WebFetchConfig, WebProvider, WebSearchConfig,
    };

    impl Providers {
        fn ids(&self) -> Vec<WebProvider> {
            self.instances.iter().map(|p| p.id()).collect()
        }
    }

    /// Injected environment: production passes the process environment, tests pass a map.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    fn none() -> impl Fn(&str) -> Option<String> {
        |_: &str| None
    }

    fn registry(cfg: &WebConfig, env: &impl Fn(&str) -> Option<String>) -> Providers {
        Providers::from_config(cfg, env).expect("registry")
    }

    fn ids_of(providers: &Providers) -> Vec<WebProvider> {
        providers.ids()
    }

    #[test]
    fn every_provider_declares_label_transport_and_limits() {
        for id in WebProvider::BUILTIN_ORDER {
            let spec = spec(id);
            assert_eq!(spec.id, id);
            assert!(!spec.label.is_empty());
            assert!(!spec.env_key.is_empty());
            assert!(spec.max_results >= 1);
            assert!(spec.endpoint.starts_with("https://"), "{}", spec.label);
            if let Some(mcp) = &spec.mcp {
                assert!(!mcp.auth_header.is_empty(), "{}", spec.label);
                let search = spec.search.as_ref().expect("an MCP provider searches");
                assert!(!search.tool.is_empty(), "{}", spec.label);
            }
        }
        let tavily = spec(WebProvider::Tavily);
        assert!(tavily.mcp.is_none(), "tavily goes through the REST adapter");
        assert!(tavily.fetch.is_none());
        assert!(tavily.search.is_none());
    }

    #[test]
    fn order_prefers_configured_list_and_dedupes() {
        let mut cfg = WebConfig::default();
        assert_eq!(order(&cfg.search), WebProvider::BUILTIN_ORDER.to_vec());

        cfg.search.chain = vec![WebProvider::Exa, WebProvider::Exa, WebProvider::Parallel];
        assert_eq!(
            order(&cfg.search),
            vec![WebProvider::Exa, WebProvider::Parallel]
        );
    }

    #[test]
    fn build_skips_provider_that_requires_a_key() {
        let cfg = WebConfig::default();
        assert_eq!(
            ids_of(&registry(&cfg, &none())),
            vec![WebProvider::Parallel, WebProvider::Exa]
        );

        let mut with_key = cfg.clone();
        with_key.providers.tavily.api_key = Some("k".to_owned());
        let providers = registry(&with_key, &none());
        assert_eq!(
            ids_of(&providers),
            vec![WebProvider::Parallel, WebProvider::Exa, WebProvider::Tavily]
        );
        assert_eq!(
            resolve_key(
                &cfg,
                WebProvider::Tavily,
                spec(WebProvider::Tavily),
                &none()
            )
            .source,
            KeySource::None
        );
        assert_eq!(
            providers
                .search_instances(&WebSearchConfig::default())
                .iter()
                .find(|provider| provider.id() == WebProvider::Tavily)
                .expect("tavily is configured")
                .api_key(),
            Some("k")
        );
    }

    #[test]
    fn build_reads_provider_key_from_environment() {
        let cfg = WebConfig::default();
        let name = spec(WebProvider::Tavily).env_key;

        let providers = registry(&cfg, &env_of(&[(name, "  from-env  ")]));
        assert_eq!(
            ids_of(&providers),
            vec![WebProvider::Parallel, WebProvider::Exa, WebProvider::Tavily]
        );
        assert_eq!(
            resolve_key(
                &cfg,
                WebProvider::Tavily,
                spec(WebProvider::Tavily),
                &env_of(&[(name, "from-env")])
            )
            .source,
            KeySource::Env
        );
        assert_eq!(
            ids_of(&registry(&cfg, &env_of(&[(name, "   ")]))),
            vec![WebProvider::Parallel, WebProvider::Exa],
            "a blank environment variable counts as unset"
        );
    }

    #[test]
    fn environment_key_wins_over_config() {
        let mut cfg = WebConfig::default();
        cfg.providers.tavily.api_key = Some("from-config".to_owned());
        let name = spec(WebProvider::Tavily).env_key;

        let providers = registry(&cfg, &env_of(&[(name, "from-env")]));
        let instances = providers.search_instances(&WebSearchConfig::default());
        let tavily = instances
            .iter()
            .find(|provider| provider.id() == WebProvider::Tavily)
            .expect("tavily is configured");
        assert_eq!(tavily.api_key(), Some("from-env"));
    }

    #[test]
    fn fetch_steps_keep_the_configured_order_and_validate_capability() {
        let providers = registry(&WebConfig::default(), &none());

        let mixed = WebFetchConfig {
            chain: vec![
                FetchEngine::Local,
                FetchEngine::Provider(WebProvider::Parallel),
            ],
            ..WebFetchConfig::default()
        };
        let steps = providers.fetch_steps(&mixed).expect("steps");
        assert_eq!(steps.len(), 2);
        assert!(matches!(steps[0].kind(), FetchEngine::Local));
        assert!(matches!(
            steps[1].kind(),
            FetchEngine::Provider(WebProvider::Parallel)
        ));

        let incapable = WebFetchConfig {
            chain: vec![FetchEngine::Provider(WebProvider::Tavily)],
            ..WebFetchConfig::default()
        };
        let err = match providers.fetch_steps(&incapable) {
            Err(err) => err,
            Ok(_) => panic!("tavily has no fetch tool"),
        };
        assert!(err.to_string().contains("no fetch capability"), "{err}");
    }

    #[test]
    fn fetch_arguments_merge_declared_extras() {
        fn parse(_: &str) -> Option<String> {
            None
        }
        let plain = Fetch {
            tool: "t",
            parse,
            arguments: || Value::Null,
        };
        let full = Fetch {
            tool: "t",
            parse,
            arguments: || json!({ "full_content": true }),
        };
        let url = WebUrl::parse("https://a.example/").expect("url");
        assert_eq!(
            fetch_arguments(&plain, &url),
            json!({ "urls": ["https://a.example/"] })
        );
        assert_eq!(
            fetch_arguments(&full, &url),
            json!({ "urls": ["https://a.example/"], "full_content": true })
        );
    }

    #[test]
    fn parallel_fetch_prefers_full_content_then_excerpts() {
        let excerpts = r#"{"extract_id":"e1","results":[{"url":"https://a/","excerpts":["first excerpt","second excerpt"],"full_content":null}]}"#;
        assert_eq!(
            parse_parallel_fetch(excerpts).as_deref(),
            Some("first excerpt\n\nsecond excerpt")
        );

        let full = r#"{"results":[{"url":"https://a/","excerpts":["excerpt"],"full_content":"whole article body"}]}"#;
        assert_eq!(
            parse_parallel_fetch(full).as_deref(),
            Some("whole article body")
        );

        assert_eq!(parse_parallel_fetch(r#"{"results":[]}"#), None);
        assert_eq!(parse_parallel_fetch("not json"), None);
    }

    #[test]
    fn exa_fetch_passes_text_through() {
        assert_eq!(
            parse_exa_fetch("  body text \n").as_deref(),
            Some("body text")
        );
        assert_eq!(parse_exa_fetch("  \n").as_deref(), None);
    }

    #[test]
    fn parse_splits_blocks_and_fields() {
        let text = "Title: First title\nURL: https://a.example/\nPublished: 2026-01-01T00:00:00.000Z\nAuthor: N/A\nHighlights:\nfirst body\n...\nsecond body\n\n---\n\nTitle: Second title\nURL: https://b.example/\nPublished: N/A\nAuthor: N/A\nHighlights:\nthird body";
        let response = parse_exa_search(text, 10).expect("valid payload");
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].title, "First title");
        assert_eq!(response.results[0].url, "https://a.example/");
        assert_eq!(
            response.results[0].age.as_deref(),
            Some("2026-01-01T00:00:00.000Z")
        );
        assert!(
            response.results[0]
                .snippet
                .as_deref()
                .is_some_and(|s| s.contains("first body"))
        );
        assert_eq!(response.results[1].url, "https://b.example/");
        assert_eq!(response.results[1].age, None);
    }

    #[test]
    fn parse_skips_blocks_without_url() {
        assert!(
            parse_exa_search("Title: no link\nHighlights:\ncontent", 10)
                .expect("valid payload")
                .results
                .is_empty()
        );
    }

    #[test]
    fn parse_maps_excerpts_and_dates() {
        let text = r#"{
            "search_id": "search_1",
            "results": [
                { "url": "https://a.example/", "title": "A title", "publish_date": "2026-01-01",
                  "excerpts": ["first excerpt", "second excerpt"] },
                { "url": "", "title": "no link", "excerpts": [] }
            ]
        }"#;
        let response = parse_parallel_search(text, 10).expect("valid payload");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "A title");
        assert_eq!(response.results[0].age.as_deref(), Some("2026-01-01"));
        assert_eq!(
            response.results[0].snippet.as_deref(),
            Some("first excerpt second excerpt")
        );
    }

    #[test]
    fn parse_respects_limit() {
        let text =
            r#"{"results":[{"url":"https://a/"},{"url":"https://b/"},{"url":"https://c/"}]}"#;
        assert_eq!(
            parse_parallel_search(text, 2)
                .expect("valid payload")
                .results
                .len(),
            2
        );
    }
}
