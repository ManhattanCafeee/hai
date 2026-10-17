//! The `[model-roles]` table: every role resolves through one policy, so a role is a row here and
//! nowhere else.

use strum::IntoEnumIterator;

use crate::{
    agentcore::{ModelKind, ModelTarget, provider::ChatTransport},
    config::{AppConfig, ConfigIssue, ProviderRegistry},
    error::{Result, bail},
};

/// The selectors one `[model-roles.<role>]` block names, or `None` when the config declares no block
/// for that role at all. An unnamed key is `None` inside either way: a block that leaves out
/// `provider` inherits the `default` block's, and one that leaves out `model` has not configured its
/// role.
#[derive(Debug, Clone, Copy)]
struct Selectors<'a> {
    provider: Option<&'a str>,
    model: Option<&'a str>,
    base_url: Option<&'a str>,
}

/// The selectors of a block whose keys are optional.
fn selectors<'a>(
    provider: Option<&'a String>,
    model: Option<&'a String>,
    base_url: Option<&'a String>,
) -> Selectors<'a> {
    Selectors {
        provider: provider.map(String::as_str),
        model: model.map(String::as_str),
        base_url: base_url.map(String::as_str),
    }
}

/// The selectors of a block that states its provider and model as plain strings, so an empty value
/// is what "not named" looks like there. The chat and rerank blocks are the two that do.
fn stated_selectors<'a>(
    provider: &'a str,
    model: &'a str,
    base_url: Option<&'a String>,
) -> Selectors<'a> {
    Selectors {
        provider: (!provider.is_empty()).then_some(provider),
        model: (!model.is_empty()).then_some(model),
        base_url: base_url.map(String::as_str),
    }
}

/// What an absent or model-less block means for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RolePolicy {
    /// Run on the main model when that provider serves the kind; otherwise the role is off.
    InheritMain,
    /// The role is off.
    Off,
    /// The role is required: startup fails.
    Required,
}

/// The table: one arm per role — what a block-less row means, and the selectors it names. Video
/// reads the `vision` row's block, which is why there is no row of its own.
fn row(config: &AppConfig, kind: ModelKind) -> (RolePolicy, Option<Selectors<'_>>) {
    let roles = &config.model_roles;
    match kind {
        ModelKind::Chat => (
            RolePolicy::Required,
            Some(stated_selectors(
                &roles.default.provider,
                &roles.default.model,
                roles.default.base_url.as_ref(),
            )),
        ),
        ModelKind::Vision => (
            RolePolicy::InheritMain,
            roles
                .vision
                .as_ref()
                .map(|b| selectors(b.provider.as_ref(), b.model.as_ref(), b.base_url.as_ref())),
        ),
        ModelKind::Transcription => (
            RolePolicy::InheritMain,
            roles
                .transcription
                .as_ref()
                .map(|b| selectors(b.provider.as_ref(), b.model.as_ref(), b.base_url.as_ref())),
        ),
        ModelKind::Speech => (
            RolePolicy::Off,
            roles
                .speech
                .as_ref()
                .map(|b| selectors(b.provider.as_ref(), b.model.as_ref(), b.base_url.as_ref())),
        ),
        ModelKind::Image => (
            RolePolicy::Off,
            roles
                .image
                .as_ref()
                .map(|b| selectors(b.provider.as_ref(), b.model.as_ref(), b.base_url.as_ref())),
        ),
        ModelKind::Embedding => (
            RolePolicy::Required,
            Some(selectors(
                roles.embedding.provider.as_ref(),
                roles.embedding.model.as_ref(),
                roles.embedding.base_url.as_ref(),
            )),
        ),
        ModelKind::Rerank => (
            RolePolicy::Off,
            roles
                .rerank
                .as_ref()
                .map(|b| stated_selectors(&b.provider, &b.model, b.base_url.as_ref())),
        ),
    }
}

/// The ModelRoles a process resolved: one target per role, plus the chat transport the genai client
/// is built from.
#[derive(Debug)]
pub(crate) struct ModelRoles {
    pub chat: ChatTransport,
    pub vision: Option<ModelTarget>,
    pub transcription: Option<ModelTarget>,
    pub speech: Option<ModelTarget>,
    pub image: Option<ModelTarget>,
    pub embedding: ModelTarget,
    pub rerank: Option<ModelTarget>,
}

/// Resolves the whole table against the providers this process can reach.
pub(crate) fn resolve(config: &AppConfig, registry: &ProviderRegistry) -> Result<ModelRoles> {
    let chat = required_role(config, registry, ModelKind::Chat)?;
    let chat = ChatTransport {
        kind: registry.provider_kind(&config.model_roles.default.provider)?,
        target: chat,
    };
    Ok(ModelRoles {
        chat,
        vision: resolve_role(config, registry, ModelKind::Vision)?,
        transcription: resolve_role(config, registry, ModelKind::Transcription)?,
        speech: resolve_role(config, registry, ModelKind::Speech)?,
        image: resolve_role(config, registry, ModelKind::Image)?,
        embedding: required_role(config, registry, ModelKind::Embedding)?,
        rerank: resolve_role(config, registry, ModelKind::Rerank)?,
    })
}

/// The embedding role, resolved without the rest of the table.
pub(crate) fn embedding(config: &AppConfig, registry: &ProviderRegistry) -> Result<ModelTarget> {
    required_role(config, registry, ModelKind::Embedding)
}

/// A role the process cannot do without: the table marks only chat and embedding required, and a
/// required row fails on an empty model rather than resolving to nothing.
fn required_role(
    config: &AppConfig,
    registry: &ProviderRegistry,
    kind: ModelKind,
) -> Result<ModelTarget> {
    Ok(resolve_role(config, registry, kind)?.expect("a required role always resolves to a target"))
}

/// The providers the roles name, which availability is resolved for; a block nothing declares names
/// none.
pub(crate) fn providers_named(config: &AppConfig) -> Vec<&str> {
    ModelKind::iter()
        .filter_map(|kind| row(config, kind).1)
        .filter_map(|selectors| selectors.provider)
        .collect()
}

/// Resolves one row: the target its block names, or `None` for a role that is off.
fn resolve_role(
    config: &AppConfig,
    registry: &ProviderRegistry,
    kind: ModelKind,
) -> Result<Option<ModelTarget>> {
    let (policy, declared) = row(config, kind);
    let main = &config.model_roles.default;
    let provider = declared
        .and_then(|selectors| selectors.provider)
        .unwrap_or(main.provider.as_str());
    let base_url = declared.and_then(|selectors| selectors.base_url);

    let Some(model) = declared.and_then(|selectors| selectors.model) else {
        return match policy {
            // A row nobody declared runs on the main model, and stays off when that provider cannot
            // speak the kind: the user never declared it.
            RolePolicy::InheritMain if declared.is_none() => {
                if !registry.serves(kind, provider) {
                    return Ok(None);
                }
                Ok(Some(target(
                    registry,
                    kind,
                    provider,
                    &main.model,
                    main.base_url.as_deref(),
                )?))
            }
            // A declared block that names no model keeps its own provider and takes the main model.
            RolePolicy::InheritMain => Ok(Some(target(
                registry,
                kind,
                provider,
                &main.model,
                base_url,
            )?)),
            RolePolicy::Off => Ok(None),
            RolePolicy::Required => bail!(ConfigIssue::model_role(kind, model_required(kind))),
        };
    };
    Ok(Some(target(registry, kind, provider, model, base_url)?))
}

/// One endpoint, or the error an unnamed provider earns. The name is empty exactly when the row
/// inherited `default` and `default` names nobody, so that is the block the message points at.
fn target(
    registry: &ProviderRegistry,
    kind: ModelKind,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> Result<ModelTarget> {
    if provider.is_empty() {
        bail!(ConfigIssue::model_role(
            ModelKind::Chat,
            "no provider is named: set `provider` here"
        ));
    }
    registry.endpoint(kind, provider, model, base_url)
}

/// The message a required role's model-less block earns; the two required roles differ in why.
fn model_required(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Embedding => "is required (memory/retrieval depends on it) but model is empty",
        _ => "model is required",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{EmbeddingConfig, ImageConfig, ProviderConfig, SpeechConfig};

    /// The roles these tests exercise are about resolution, not availability, so every provider is
    /// given a credential and every endpoint answers.
    fn registry(config: &AppConfig) -> ProviderRegistry {
        ProviderRegistry::from_sources(config, &|_| Some("test-key".to_owned()), &|_| true)
    }

    /// The config every case starts from: one chat role, its provider declared, plus the embedding
    /// role every process insists on.
    fn config(provider: &str, model: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.model_roles.default.provider = provider.into();
        cfg.model_roles.default.model = model.into();
        cfg.providers.insert(provider.into(), Default::default());
        cfg.model_roles.embedding = EmbeddingConfig {
            provider: Some("openai".into()),
            model: Some("text-embedding-3-small".into()),
            dimension: None,
            base_url: None,
        };
        cfg.providers.insert("openai".into(), Default::default());
        cfg
    }

    fn roles(cfg: &AppConfig) -> Result<ModelRoles> {
        super::resolve(cfg, &registry(cfg))
    }

    fn image(model: Option<&str>, provider: Option<&str>) -> ImageConfig {
        ImageConfig {
            provider: provider.map(str::to_owned),
            model: model.map(str::to_owned),
            base_url: None,
        }
    }

    /// An absent inheriting block runs on the main model, its endpoint included, so a proxied
    /// `[model-roles.default]` does not send the delegated call to the provider's own root.
    #[test]
    fn an_absent_inheriting_block_takes_the_main_endpoint() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.default.base_url = Some("https://gateway.internal/v1".into());

        let roles = roles(&cfg).unwrap();
        let vision = roles.vision.unwrap();
        assert_eq!(vision.base_url, "https://gateway.internal/v1");
        assert_eq!(vision.model, "gpt-4o");
        assert_eq!(
            roles.transcription.unwrap().base_url,
            "https://gateway.internal/v1"
        );
    }

    /// An absent inheriting block stays off when the main provider cannot read that media, instead
    /// of failing startup over a block nobody configured.
    #[test]
    fn an_absent_inheriting_block_is_off_when_the_main_provider_cannot_read_the_media() {
        let cfg = config("anthropic", "claude-sonnet-4-5");

        let roles = roles(&cfg).unwrap();
        assert!(roles.vision.is_none());
        assert!(roles.transcription.is_none());
        assert!(roles.image.is_none());
        assert!(roles.speech.is_none());
        assert!(roles.rerank.is_none());
    }

    /// A block that names a model and no provider runs on the `default` block's provider, which is
    /// where its endpoint comes from too.
    #[test]
    fn a_named_model_runs_on_the_default_provider() {
        let mut cfg = config("openrouter", "openai/gpt-4o");
        cfg.providers.insert(
            "openrouter".into(),
            ProviderConfig {
                base_url: Some("https://gateway.internal/v1".into()),
                ..Default::default()
            },
        );
        cfg.model_roles.image = Some(image(Some("some-image-model"), None));

        let roles = roles(&cfg).unwrap();
        assert_eq!(roles.image.unwrap().base_url, "https://gateway.internal/v1");
    }

    /// A dedicated block that names a provider and no model is off, and a speech block in that state
    /// is off before its voice is even looked at.
    #[test]
    fn a_dedicated_block_with_no_model_is_off() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.providers
            .insert("openrouter".into(), Default::default());
        cfg.model_roles.image = Some(image(None, Some("openrouter")));
        cfg.model_roles.speech = Some(SpeechConfig {
            provider: Some("openrouter".into()),
            model: None,
            voice: String::new(),
            speed: 1.0,
            base_url: None,
        });
        cfg.model_roles.rerank = Some(crate::config::schema::RerankConfig {
            provider: "openrouter".into(),
            model: String::new(),
            base_url: None,
        });

        let roles = roles(&cfg).unwrap();
        assert!(roles.image.is_none());
        assert!(roles.speech.is_none());
        assert!(roles.rerank.is_none());
    }

    /// A rerank block that names only a model runs on the `default` provider, which is where an
    /// OpenRouter cross-encoder lives.
    #[test]
    fn a_rerank_block_takes_the_default_provider_and_the_kinds_own_endpoint() {
        let mut cfg = config("openrouter", "openai/gpt-4o");
        cfg.model_roles.rerank = Some(crate::config::schema::RerankConfig {
            provider: String::new(),
            model: "cohere/rerank-4-fast".into(),
            base_url: None,
        });

        let roles = roles(&cfg).unwrap();
        assert_eq!(
            roles.rerank.unwrap().base_url,
            "https://openrouter.ai/api/v1/rerank"
        );
    }

    /// A required role with no model fails startup, with the reason embedding is required.
    #[test]
    fn a_required_role_with_no_model_fails() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.embedding.model = None;

        let err = roles(&cfg).unwrap_err();
        assert!(
            err.to_string().contains(
                "[model-roles.embedding] is required (memory/retrieval depends on it) but model is \
                 empty"
            ),
            "{err}"
        );
    }

    /// An empty `default` provider is the one key the message points at, whichever role inherited it.
    #[test]
    fn an_unnamed_default_provider_points_at_the_default_block() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.default.provider = String::new();
        cfg.model_roles.image = Some(image(Some("some-image-model"), None));

        let err = roles(&cfg).unwrap_err();
        assert!(
            err.to_string()
                .contains("[model-roles.default] no provider is named: set `provider` here"),
            "{err}"
        );
    }

    /// The chat role is required like embedding is, for the one thing a Step loop cannot do without.
    #[test]
    fn chat_without_a_model_fails() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.default.model = String::new();

        let err = roles(&cfg).unwrap_err();
        assert!(
            err.to_string()
                .contains("[model-roles.default] model is required"),
            "{err}"
        );
    }

    /// Availability is resolved for the providers the roles name; a role nobody declared, and an
    /// unnamed `default` provider, name none.
    #[test]
    fn providers_named_lists_the_default_and_every_named_role() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.image = Some(image(Some("some-image-model"), Some("openrouter")));
        assert!(providers_named(&cfg).contains(&"openai"));
        assert!(providers_named(&cfg).contains(&"openrouter"));

        cfg.model_roles.default.provider = String::new();
        assert!(!providers_named(&cfg).contains(&""));
    }

    /// A declared block on a provider that cannot serve the kind fails startup rather than every
    /// request.
    #[test]
    fn a_declared_block_on_a_provider_that_cannot_serve_the_kind_fails() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.providers.insert("deepseek".into(), Default::default());
        cfg.model_roles.embedding.provider = Some("deepseek".into());

        let err = roles(&cfg).unwrap_err();
        assert!(err.to_string().contains("serves no embedding API"), "{err}");
    }

    /// The same config through the whole table is refused: the one role the rebuild needs resolves
    /// on its own, with no chat model configured at all.
    #[test]
    fn the_embedding_role_resolves_without_a_chat_model() {
        let mut cfg = AppConfig::default();
        cfg.model_roles.embedding = EmbeddingConfig {
            provider: Some("openai".into()),
            model: Some("text-embedding-3-small".into()),
            dimension: None,
            base_url: None,
        };
        cfg.providers.insert("openai".into(), Default::default());

        let registry = registry(&cfg);
        assert_eq!(
            embedding(&cfg, &registry).unwrap().base_url,
            "https://api.openai.com/v1"
        );

        let err = roles(&cfg).unwrap_err();
        assert!(
            err.to_string()
                .contains("[model-roles.default] model is required"),
            "{err}"
        );
    }
}
