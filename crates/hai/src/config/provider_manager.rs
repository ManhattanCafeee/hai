//! Provider availability: the config and the environment decide which `[providers.*]` entries this
//! process can use. A block declares an override — transport, endpoint, headers — not the
//! provider's existence, so a credential in the environment or a local service answering is enough.

use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use strum::IntoEnumIterator;
use tokio::{net::TcpStream, time::timeout};
use url::Url;

use crate::{
    agentcore::{Credential, ModelKind, ModelTarget, provider::ProviderKind},
    config::{AppConfig, ConfigIssue, env::read_env_key, schema::ProviderConfig},
    error::{Result, bail},
};

/// How long a keyless provider's endpoint has to answer before it counts as unavailable.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(crate) struct ProviderEntry {
    pub config: ProviderConfig,
    pub kind: Option<ProviderKind>,
}

/// Why a provider the config names is not usable, in the terms of what the user can change.
#[derive(Debug, Clone)]
enum Missing {
    /// The name is neither a kind nor a declared endpoint.
    Unknown,
    /// Nothing to reach: a block with neither transport nor endpoint, or a kind with no shared root.
    NoEndpoint,
    /// The kind needs a credential and neither the environment nor the block supplied one.
    NoApiKey(&'static [&'static str]),
    /// The keyless endpoint did not answer.
    Unreachable(String),
}

impl Missing {
    fn issue(&self, name: &str) -> ConfigIssue {
        match self {
            Self::Unknown => {
                let supported: Vec<&str> = ProviderKind::iter().map(Into::into).collect();
                ConfigIssue::provider(
                    name,
                    format!(
                        "is not a supported provider: set `type` and `base-url` here, or name one \
                         of {}",
                        supported.join(", ")
                    ),
                )
            }
            Self::NoEndpoint => ConfigIssue::provider(name, "has no endpoint: set `base-url` here"),
            Self::NoApiKey([]) => {
                ConfigIssue::provider(name, "needs an api-key: set `api-key` here")
            }
            Self::NoApiKey(keys) => ConfigIssue::provider(
                name,
                format!(
                    "needs an api-key: set `api-key` here or {}",
                    keys.join(" / ")
                ),
            ),
            Self::Unreachable(endpoint) => {
                ConfigIssue::provider(name, format!("is not reachable at {endpoint}"))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Availability {
    Ready(ProviderEntry),
    Unavailable(Missing),
}

/// What one candidate's block and the environment say about reaching it, before its endpoint has
/// been probed.
enum Evidence {
    Ready(ProviderEntry),
    /// Keyless with no declared endpoint: the endpoint answering is the evidence.
    Reachable(ProviderEntry, String),
    Missing(Missing),
}

#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Availability>,
}

impl ProviderRegistry {
    /// Config and environment decide availability; a keyless provider that declares no endpoint is
    /// probed, so its service answering is what makes it available.
    pub async fn discover(config: &AppConfig) -> Self {
        let env = |name: &str| std::env::var(name).ok();
        let mut answers = HashMap::new();
        for endpoint in probe_targets(config, &env) {
            let answer = reachable_at(&endpoint).await;
            answers.insert(endpoint, answer);
        }
        Self::from_sources(config, &env, &|endpoint| {
            answers.get(endpoint).copied().unwrap_or(false)
        })
    }

    /// The same resolution with the reachability answers supplied instead of probed.
    pub(crate) fn from_sources(
        config: &AppConfig,
        env: &impl Fn(&str) -> Option<String>,
        reachable: &impl Fn(&str) -> bool,
    ) -> Self {
        let providers = candidates(config)
            .into_iter()
            .map(|name| {
                let availability = match evidence(config, env, &name) {
                    Evidence::Ready(entry) => Availability::Ready(entry),
                    Evidence::Reachable(entry, endpoint) if reachable(&endpoint) => {
                        Availability::Ready(entry)
                    }
                    Evidence::Reachable(_, endpoint) => {
                        Availability::Unavailable(Missing::Unreachable(endpoint))
                    }
                    Evidence::Missing(missing) => Availability::Unavailable(missing),
                };
                (name, availability)
            })
            .collect();
        Self { providers }
    }

    /// Only an available provider, which is what every lookup past startup wants.
    fn ready(&self, provider: &str) -> Option<&ProviderEntry> {
        match self.providers.get(provider) {
            Some(Availability::Ready(entry)) => Some(entry),
            _ => None,
        }
    }

    pub(crate) fn get_checked(&self, provider: &str) -> Result<&ProviderEntry> {
        match self.providers.get(provider) {
            Some(Availability::Ready(entry)) => Ok(entry),
            Some(Availability::Unavailable(missing)) => Err(missing.issue(provider).into()),
            None => Err(Missing::Unknown.issue(provider).into()),
        }
    }

    /// Whether the provider's kind speaks this model kind at all; a caller that only delegates (an
    /// absent vision or transcription block) asks this before it asks for the endpoint.
    pub fn serves(&self, kind: ModelKind, provider: &str) -> bool {
        self.ready(provider)
            .and_then(|entry| entry.kind)
            .and_then(|provider_kind| provider_kind.endpoint_for(kind))
            .is_some()
    }

    /// One endpoint resolved for a caller: the block's own `base-url` wins over the provider's
    /// answer for that kind, which is the kind's preset or its base URL. A kind the provider cannot
    /// speak fails here rather than at every request.
    pub fn endpoint(
        &self,
        kind: ModelKind,
        provider: &str,
        model: &str,
        base_url: Option<&str>,
    ) -> Result<ModelTarget> {
        let entry = self.get_checked(provider)?;
        let preset = entry
            .kind
            .and_then(|provider_kind| provider_kind.endpoint_for(kind));

        let base_url = if let Some(url) = base_url {
            url.to_owned()
        } else if let Some(preset) = preset {
            preset
                .url
                .map(str::to_owned)
                .or_else(|| entry.config.base_url.clone())
                .or_else(|| {
                    entry
                        .kind
                        .and_then(|kind| kind.default_base_url())
                        .map(str::to_owned)
                })
                .ok_or_else(|| {
                    ConfigIssue::model_role(
                        kind,
                        format!(
                            "provider '{provider}' has no endpoint for {kind}: set `base-url` here"
                        ),
                    )
                })?
        } else {
            bail!(ConfigIssue::model_role(
                kind,
                format!("provider '{provider}' serves no {kind} API: set `base-url` here"),
            ));
        };

        Ok(ModelTarget {
            base_url,
            model: model.to_owned(),
            credential: Credential::new(entry.config.api_key.clone().unwrap_or_default()),
            headers: Arc::new(crate::agentcore::provider::header_map(
                &entry.config.headers,
            )?),
        })
    }

    /// The chat transport kind of a provider, which the genai client is built from. Every kind
    /// serves chat, so a kind-less entry is the only way to have none, and that failure names the
    /// missing `type`.
    pub(crate) fn provider_kind(&self, provider: &str) -> Result<ProviderKind> {
        let kind = self.get_checked(provider)?.kind.ok_or_else(|| {
            ConfigIssue::provider(provider, "speaks no chat transport: set `type` here")
        })?;
        Ok(kind)
    }
}

/// The names availability is resolved for: every declared block plus every provider the model roles
/// name, so a name nothing mentions stays unknown.
fn candidates(config: &AppConfig) -> Vec<String> {
    let mut names: Vec<String> = config.providers.keys().cloned().collect();
    names.extend(
        crate::config::model_role::providers_named(config)
            .into_iter()
            .map(str::to_owned),
    );
    names.sort();
    names.dedup();
    names
}

/// The one place availability is decided, so probing and resolving cannot disagree on what to probe.
fn evidence(config: &AppConfig, env: &impl Fn(&str) -> Option<String>, name: &str) -> Evidence {
    let block = config.providers.get(name);
    let kind = block
        .and_then(|block| block.kind(name))
        .or_else(|| ProviderKind::from_str(name).ok());

    let Some(kind) = kind else {
        return match block {
            // A name with no block is either a kind HAI knows or a typo.
            None => Evidence::Missing(Missing::Unknown),
            Some(block) if block.base_url.is_none() => Evidence::Missing(Missing::NoEndpoint),
            Some(block) => Evidence::Ready(ProviderEntry {
                config: block.clone(),
                kind: None,
            }),
        };
    };

    let Some(base_url) = block
        .and_then(|block| block.base_url.as_deref())
        .or_else(|| kind.default_base_url())
    else {
        return Evidence::Missing(Missing::NoEndpoint);
    };

    let key = kind
        .env_keys()
        .iter()
        .find_map(|name| read_env_key(env, name))
        .or_else(|| block.and_then(|block| block.api_key.clone()));
    let entry = ProviderEntry {
        config: ProviderConfig {
            api_key: key.clone(),
            ..block.cloned().unwrap_or_default()
        },
        kind: Some(kind),
    };

    match key {
        Some(_) => Evidence::Ready(entry),
        // A block that states its own endpoint is taken at its word.
        None if block.is_some_and(|block| block.base_url.is_some()) => Evidence::Ready(entry),
        None if kind.keyless() => Evidence::Reachable(entry, base_url.to_owned()),
        None => Evidence::Missing(Missing::NoApiKey(kind.env_keys())),
    }
}

/// The endpoints a keyless provider would be reached at, deduplicated.
fn probe_targets(config: &AppConfig, env: &impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut targets: Vec<String> = candidates(config)
        .into_iter()
        .filter_map(|name| match evidence(config, env, &name) {
            Evidence::Reachable(_, endpoint) => Some(endpoint),
            _ => None,
        })
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// Whether the endpoint answers a TCP connection: a malformed URL, a refused connection and a
/// silent host all read as unavailable.
async fn reachable_at(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return false;
    };
    matches!(
        timeout(PROBE_TIMEOUT, TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    /// A key for every kind, so a test declares only the blocks it means to exercise.
    fn any_key(name: &str) -> Option<String> {
        name.ends_with("_API_KEY").then(|| "test-key".to_owned())
    }

    /// No credential anywhere in the environment.
    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn config_of(entries: &[(&str, Option<ProviderKind>)]) -> AppConfig {
        let mut config = AppConfig::default();
        for (name, kind) in entries {
            config.providers.insert(
                name.to_string(),
                ProviderConfig {
                    r#type: *kind,
                    ..ProviderConfig::default()
                },
            );
        }
        config
    }

    /// A registry of declared blocks, resolved as if every endpoint answered.
    fn registry(entries: &[(&str, Option<ProviderKind>)]) -> ProviderRegistry {
        ProviderRegistry::from_sources(&config_of(entries), &any_key, &|_| true)
    }

    fn chat(registry: &ProviderRegistry, provider: &str) -> ModelTarget {
        registry
            .endpoint(ModelKind::Chat, provider, "a-model", None)
            .unwrap()
    }

    /// A model kind the provider serves itself resolves with no configuration at all.
    #[test]
    fn a_provider_preset_resolves_the_kind() {
        let registry = registry(&[("openrouter", None)]);
        let target = registry
            .endpoint(
                ModelKind::Embedding,
                "openrouter",
                "text-embedding-3-small",
                None,
            )
            .unwrap();
        assert_eq!(target.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(target.credential.key(), "test-key");
        assert!(target.headers.is_empty());
    }

    /// The environment variable alone is a credential; the block is not what makes the provider
    /// exist.
    #[test]
    fn an_env_key_alone_makes_a_provider_usable() {
        let mut config = AppConfig::default();
        config.model_roles.default.provider = "openrouter".to_owned();
        let env = |name: &str| (name == "OPENROUTER_API_KEY").then(|| "env-key".to_owned());

        let registry = ProviderRegistry::from_sources(&config, &env, &|_| false);
        assert_eq!(
            registry.provider_kind("openrouter").unwrap(),
            ProviderKind::OpenRouter
        );
        let target = registry
            .endpoint(ModelKind::Chat, "openrouter", "a-model", None)
            .unwrap();
        assert_eq!(target.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(target.credential.key(), "env-key");
    }

    /// The environment outranks the file, and the file is still the fallback.
    #[test]
    fn an_env_key_outranks_the_block_key() {
        let mut config = config_of(&[("openai", None)]);
        config.providers.get_mut("openai").unwrap().api_key = Some("file-key".to_owned());

        let env = |name: &str| (name == "OPENAI_API_KEY").then(|| "env-key".to_owned());
        let from_env = ProviderRegistry::from_sources(&config, &env, &|_| true);
        assert_eq!(chat(&from_env, "openai").credential.key(), "env-key");

        let from_file = ProviderRegistry::from_sources(&config, &no_env, &|_| true);
        assert_eq!(chat(&from_file, "openai").credential.key(), "file-key");
    }

    /// A blank variable is unconfigured, and the failure names the variable to set.
    #[test]
    fn a_blank_env_value_is_unset() {
        let mut config = AppConfig::default();
        config.model_roles.default.provider = "openai".to_owned();

        let registry =
            ProviderRegistry::from_sources(&config, &|_| Some("  ".to_owned()), &|_| true);
        let err = registry
            .endpoint(ModelKind::Chat, "openai", "a-model", None)
            .unwrap_err();
        assert!(err.to_string().contains("OPENAI_API_KEY"), "{err}");
    }

    /// A block that states its own endpoint is taken at its word: no credential, no probe.
    #[test]
    fn a_declared_endpoint_needs_no_probe() {
        let mut config = AppConfig::default();
        config.model_roles.default.provider = "my-gateway".to_owned();
        config.providers.insert(
            "my-gateway".to_owned(),
            ProviderConfig {
                r#type: Some(ProviderKind::OpenAI),
                base_url: Some("http://gw:8080/v1".to_owned()),
                ..ProviderConfig::default()
            },
        );

        let registry = ProviderRegistry::from_sources(&config, &no_env, &|_| false);
        assert_eq!(chat(&registry, "my-gateway").base_url, "http://gw:8080/v1");
    }

    /// A keyless kind with no declared endpoint is available when its default endpoint answers.
    #[test]
    fn a_keyless_kind_is_probed_at_its_default_endpoint() {
        let mut config = AppConfig::default();
        config.model_roles.default.provider = "ollama".to_owned();

        let registry = ProviderRegistry::from_sources(&config, &no_env, &|_| true);
        let chat = chat(&registry, "ollama");
        assert_eq!(chat.base_url, "http://localhost:11434/v1");
        assert_eq!(chat.credential.key(), "");

        let registry = ProviderRegistry::from_sources(&config, &no_env, &|_| false);
        let err = registry
            .endpoint(ModelKind::Chat, "ollama", "a-model", None)
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("not reachable at http://localhost:11434/v1"),
            "{err}"
        );
    }

    /// What a name is missing decides the message: a typo names the supported kinds, a block with
    /// no endpoint names the key to set.
    #[test]
    fn the_name_is_explained_when_nothing_supplies_it() {
        let mut config = AppConfig::default();
        config.model_roles.default.provider = "nope".to_owned();
        let unknown = ProviderRegistry::from_sources(&config, &no_env, &|_| true);
        let err = unknown
            .endpoint(ModelKind::Chat, "nope", "a-model", None)
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("is not a supported provider"), "{text}");
        assert!(text.contains("openrouter"), "{text}");

        let registry = registry(&[("local-rerank", None)]);
        let err = registry
            .endpoint(ModelKind::Rerank, "local-rerank", "a-model", None)
            .unwrap_err();
        assert!(err.to_string().contains("has no endpoint"), "{err}");
    }

    /// A kind that shares no root (azure) is unusable without an endpoint, credential or not.
    #[test]
    fn a_kind_without_a_shared_root_needs_a_base_url() {
        let mut config = config_of(&[("azureopenai", None)]);
        config.providers.get_mut("azureopenai").unwrap().api_key = Some("a-key".to_owned());

        let registry = ProviderRegistry::from_sources(&config, &any_key, &|_| true);
        let err = registry
            .endpoint(ModelKind::Chat, "azureopenai", "a-model", None)
            .unwrap_err();
        assert!(err.to_string().contains("has no endpoint"), "{err}");
    }

    /// The provider's declared headers ride on every kind it serves.
    #[test]
    fn a_provider_carries_its_headers_onto_every_kind() {
        let mut config = AppConfig::default();
        config.providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                headers: HashMap::from([("x-title".to_string(), "hai".to_string())]),
                ..ProviderConfig::default()
            },
        );
        let registry = ProviderRegistry::from_sources(&config, &any_key, &|_| true);
        let target = registry
            .endpoint(
                ModelKind::Embedding,
                "openrouter",
                "text-embedding-3-small",
                None,
            )
            .unwrap();
        assert_eq!(target.headers.get("x-title").unwrap(), "hai");
    }

    /// A malformed header fails startup rather than every request.
    #[test]
    fn a_malformed_header_is_reported() {
        let mut config = AppConfig::default();
        config.providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                headers: HashMap::from([("bad header".to_string(), "x".to_string())]),
                ..ProviderConfig::default()
            },
        );
        let registry = ProviderRegistry::from_sources(&config, &any_key, &|_| true);
        let err = registry
            .endpoint(
                ModelKind::Embedding,
                "openrouter",
                "text-embedding-3-small",
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("header name"), "{err}");
    }

    /// The rerank kind carries its own URL, so a proxied chat root must not move it.
    #[test]
    fn an_absolute_preset_survives_a_proxy_base_url() {
        let mut config = AppConfig::default();
        config.providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                base_url: Some("https://proxy.internal/v1".to_string()),
                ..ProviderConfig::default()
            },
        );
        let registry = ProviderRegistry::from_sources(&config, &any_key, &|_| true);
        let target = registry
            .endpoint(
                ModelKind::Rerank,
                "openrouter",
                "cohere/rerank-4-fast",
                None,
            )
            .unwrap();
        assert_eq!(target.base_url, "https://openrouter.ai/api/v1/rerank");

        let chat = registry
            .endpoint(ModelKind::Chat, "openrouter", "a-model", None)
            .unwrap();
        assert_eq!(chat.base_url, "https://proxy.internal/v1");
    }

    /// The block's own endpoint wins over the provider's, for a self-hosted reranker.
    #[test]
    fn a_block_endpoint_wins() {
        let registry = registry(&[("openrouter", None)]);
        let target = registry
            .endpoint(
                ModelKind::Rerank,
                "openrouter",
                "bge-reranker-v2-m3",
                Some("http://127.0.0.1:8080/v1/rerank"),
            )
            .unwrap();
        assert_eq!(target.base_url, "http://127.0.0.1:8080/v1/rerank");
    }

    /// A kind the provider cannot speak is a startup error, not a request-time one.
    #[test]
    fn a_kind_the_provider_cannot_speak_is_reported() {
        let registry = registry(&[("deepseek", None)]);
        let err = registry
            .endpoint(ModelKind::Embedding, "deepseek", "some-model", None)
            .unwrap_err();
        assert!(err.to_string().contains("serves no embedding API"), "{err}");
    }

    /// A kind with no shared root needs an endpoint: without one it is
    /// unavailable, and with one a block's own `base-url` still wins.
    #[test]
    fn a_kind_without_a_shared_root_is_reported() {
        let registry = registry(&[("azure", Some(ProviderKind::AzureOpenAI))]);
        let err = registry
            .endpoint(ModelKind::Chat, "azure", "gpt-4o", None)
            .unwrap_err();
        assert!(err.to_string().contains("has no endpoint"), "{err}");

        let mut config = config_of(&[("azure", Some(ProviderKind::AzureOpenAI))]);
        config.providers.get_mut("azure").unwrap().base_url =
            Some("https://acme.openai.azure.com".to_owned());
        let registry = ProviderRegistry::from_sources(&config, &any_key, &|_| true);
        let target = registry
            .endpoint(
                ModelKind::Chat,
                "azure",
                "gpt-4o",
                Some("https://acme.openai.azure.com/openai/v1"),
            )
            .unwrap();
        assert_eq!(target.base_url, "https://acme.openai.azure.com/openai/v1");
    }

    /// This kind does not speak that model kind.
    #[test]
    fn serves_reports_the_kind_whitelist() {
        let registry = registry(&[("openrouter", None), ("deepseek", None)]);
        assert!(registry.serves(ModelKind::Rerank, "openrouter"));
        assert!(!registry.serves(ModelKind::Rerank, "deepseek"));
    }

    /// The probe reads reachability, and anything it cannot turn into a host:port is unreachable.
    #[tokio::test]
    async fn reachable_at_follows_the_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(reachable_at(&format!("http://127.0.0.1:{port}/v1")).await);
        assert!(!reachable_at("not a url").await);
    }
}
