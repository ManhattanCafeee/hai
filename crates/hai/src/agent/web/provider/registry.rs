//! Assembly: config plus an injected environment decide which providers are instantiated.

use std::sync::Arc;

use super::{
    FetchStep, McpClient, Provider, Providers, key::resolve_key, table::spec, transport::Transport,
};
use crate::{
    config::schema::{FetchEngine, WebConfig, WebFetchConfig, WebProvider, WebSearchConfig},
    error::{AppError, Result},
};

impl Providers {
    pub fn from_config(cfg: &WebConfig, env: &impl Fn(&str) -> Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(cfg.fetch.user_agent.clone())
            .build()
            .map_err(|e| AppError::message(format!("Failed to build HTTP client: {e}")))?;
        let instances: Vec<Arc<Provider>> = WebProvider::BUILTIN_ORDER
            .into_iter()
            .filter_map(|id| instantiate(cfg, id, &http, env))
            .collect();
        Ok(Self { http, instances })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn search_instances(&self, cfg: &WebSearchConfig) -> Vec<Arc<Provider>> {
        order(cfg)
            .into_iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// Naming a provider with no fetch capability, or one that was never instantiated (missing
    /// credential), is an assembly-time configuration error rather than a runtime fallback.
    pub fn fetch_steps(&self, cfg: &WebFetchConfig) -> Result<Vec<FetchStep>> {
        let mut steps = Vec::with_capacity(cfg.chain.len());
        for engine in &cfg.chain {
            match engine {
                FetchEngine::Local => steps.push(FetchStep::Local),
                FetchEngine::Provider(id) => {
                    let spec = spec(*id);
                    let capability = spec.fetch.as_ref().ok_or_else(|| {
                        AppError::message(format!(
                            "web provider `{}` has no fetch capability",
                            spec.label
                        ))
                    })?;
                    let provider = self.get(*id).ok_or_else(|| {
                        AppError::message(format!(
                            "web provider `{}` is not available (api-key missing?)",
                            spec.label
                        ))
                    })?;
                    steps.push(FetchStep::Remote {
                        client: Arc::clone(provider.mcp()?),
                        capability,
                    });
                }
            }
        }
        Ok(steps)
    }

    fn get(&self, id: WebProvider) -> Option<Arc<Provider>> {
        self.instances
            .iter()
            .find(|provider| provider.id() == id)
            .cloned()
    }
}
pub(super) fn instantiate(
    cfg: &WebConfig,
    id: WebProvider,
    http: &reqwest::Client,
    env: &impl Fn(&str) -> Option<String>,
) -> Option<Arc<Provider>> {
    let spec = spec(id);
    let key = resolve_key(cfg, id, spec, env);
    if key.value.is_none() && !spec.keyless {
        tracing::warn!(
            provider = spec.label,
            env = spec.env_key,
            "web provider skipped: no credential (config api-key or environment)"
        );
        return None;
    }
    tracing::debug!(
        provider = spec.label,
        credential = ?key.source,
        transport = if spec.mcp.is_some() { "mcp" } else { "rest" },
        fetch = spec.fetch.is_some(),
        "web provider ready"
    );
    let mcp = spec.mcp.as_ref().map(|mcp| {
        Arc::new(McpClient {
            spec,
            transport: Transport::new(
                http.clone(),
                spec.endpoint,
                spec.label,
                mcp.auth_header,
                mcp.auth_scheme,
                key.value.clone(),
            ),
        })
    });
    Some(Arc::new(Provider {
        id,
        http: http.clone(),
        mcp,
        key,
    }))
}
pub(super) fn order(cfg: &WebSearchConfig) -> Vec<WebProvider> {
    let configured = if cfg.chain.is_empty() {
        &WebProvider::BUILTIN_ORDER[..]
    } else {
        &cfg.chain[..]
    };
    let mut kinds: Vec<WebProvider> = Vec::with_capacity(configured.len());
    for kind in configured {
        if !kinds.contains(kind) {
            kinds.push(*kind);
        }
    }
    kinds
}
