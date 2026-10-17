use std::collections::HashMap;

use genai::{
    Client, ServiceTarget, WebConfig,
    adapter::AdapterKind,
    resolver::{AuthData, Endpoint},
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::{
    agentcore::{
        model_kind::{ModelKind, ModelKindSpec, ModelTarget},
        url::ensure_trailing_slash,
    },
    error::{AppError, Result},
};

/// A provider's chat transport: the kind it speaks, and the endpoint it speaks on.
#[derive(Debug, Clone)]
pub struct ChatTransport {
    pub kind: ProviderKind,
    pub target: ModelTarget,
}

/// The chat transports a `[providers.*]` entry can name. Beyond the chat transport a kind states
/// which other model kinds it serves and where they live (`ProviderKind::endpoint_for`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive, serialize_all = "lowercase")]
pub enum ProviderKind {
    OpenRouter,
    OpenAI,
    Anthropic,
    Google,
    DeepSeek,
    Groq,
    Ollama,
    XAI,
    AzureOpenAI,
    MiniMax,
    Phind,
    Requesty,
}

/// The OpenRouter rerank route: the rerank kind of a provider whose chat root is elsewhere, so
/// it is stated absolutely rather than derived from the base URL.
const OPENROUTER_RERANK: &str = "https://openrouter.ai/api/v1/rerank";

/// What a provider kind is beyond its name: where it lives and which genai adapter serves its chat.
#[derive(Debug, Clone, Copy)]
struct KindSpec {
    /// Absent for a kind with no shared root, which reaches its deployment through the configured
    /// `base-url`.
    base_url: Option<&'static str>,
    adapter: AdapterKind,
    /// Reachable without a credential: the endpoint answering is what makes the kind available.
    keyless: bool,
    /// Environment variables a credential may come from, in precedence order; empty = none.
    env_keys: &'static [&'static str],
}

impl ProviderKind {
    fn spec(self) -> KindSpec {
        let chat = |base_url, adapter, env_keys| KindSpec {
            base_url: Some(base_url),
            adapter,
            keyless: false,
            env_keys,
        };
        match self {
            Self::OpenRouter => chat(
                "https://openrouter.ai/api/v1",
                AdapterKind::OpenRouter,
                &["OPENROUTER_API_KEY"],
            ),
            Self::OpenAI => chat(
                "https://api.openai.com/v1",
                AdapterKind::OpenAI,
                &["OPENAI_API_KEY"],
            ),
            Self::Anthropic => chat(
                "https://api.anthropic.com",
                AdapterKind::Anthropic,
                &["ANTHROPIC_API_KEY"],
            ),
            Self::Google => chat(
                "https://generativelanguage.googleapis.com",
                AdapterKind::Gemini,
                &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            ),
            Self::DeepSeek => chat(
                "https://api.deepseek.com/v1",
                AdapterKind::DeepSeek,
                &["DEEPSEEK_API_KEY"],
            ),
            Self::Groq => chat(
                "https://api.groq.com/openai/v1",
                AdapterKind::Groq,
                &["GROQ_API_KEY"],
            ),
            Self::Ollama => KindSpec {
                base_url: Some("http://localhost:11434/v1"),
                adapter: AdapterKind::Ollama,
                keyless: true,
                env_keys: &[],
            },
            Self::XAI => chat("https://api.x.ai/v1", AdapterKind::Xai, &["XAI_API_KEY"]),
            Self::AzureOpenAI => KindSpec {
                base_url: None,
                adapter: AdapterKind::OpenAI,
                keyless: false,
                env_keys: &["AZURE_OPENAI_API_KEY"],
            },
            Self::MiniMax => chat(
                "https://api.minimax.io/v1",
                AdapterKind::MiniMax,
                &["MINIMAX_API_KEY"],
            ),
            Self::Phind => chat(
                "https://api.phind.com",
                AdapterKind::OpenAI,
                &["PHIND_API_KEY"],
            ),
            Self::Requesty => chat(
                "https://router.requesty.ai/v1",
                AdapterKind::OpenAI,
                &["REQUESTY_API_KEY"],
            ),
        }
    }

    pub(crate) fn default_base_url(&self) -> Option<&'static str> {
        self.spec().base_url
    }

    /// Whether the endpoint answering is availability evidence on its own.
    pub(crate) fn keyless(self) -> bool {
        self.spec().keyless
    }

    pub(crate) fn env_keys(self) -> &'static [&'static str] {
        self.spec().env_keys
    }

    /// Whether this kind's chat transport is the OpenAI wire shape HAI builds by hand for the
    /// non-chat kinds. Anthropic, Google and Azure speak another shape (or carry the key in
    /// another header), so a block that wants one of those must state its own `base-url`.
    fn openai_compatible(self) -> bool {
        !matches!(self, Self::Anthropic | Self::Google | Self::AzureOpenAI)
    }

    fn has_embeddings(self) -> bool {
        matches!(self, Self::OpenAI | Self::OpenRouter | Self::Ollama)
    }

    fn has_speech(self) -> bool {
        matches!(self, Self::OpenAI | Self::OpenRouter)
    }

    /// The image shape HAI sends is OpenRouter's chat-completions `modalities` request, not the
    /// `/images/generations` API OpenAI and others expose.
    fn has_images(self) -> bool {
        self == Self::OpenRouter
    }

    /// What this kind serves for one model kind: where it lives, when it is not the
    /// provider's own base URL. A model kind this kind cannot speak is absent.
    pub(crate) fn endpoint_for(self, kind: ModelKind) -> Option<ModelKindSpec> {
        let at_root = || Some(ModelKindSpec { url: None });
        match kind {
            ModelKind::Chat => at_root(),
            ModelKind::Vision | ModelKind::Transcription if self.openai_compatible() => at_root(),
            ModelKind::Embedding if self.has_embeddings() => at_root(),
            ModelKind::Image if self.has_images() => at_root(),
            ModelKind::Speech if self.has_speech() => at_root(),
            ModelKind::Rerank => (self == Self::OpenRouter).then_some(ModelKindSpec {
                url: Some(OPENROUTER_RERANK),
            }),
            _ => None,
        }
    }

    fn genai_adapter_kind(&self) -> AdapterKind {
        self.spec().adapter
    }
}

/// Builds the chat client for one provider: the resolved chat endpoint supplies the target, the
/// credential and the provider's own headers.
pub(crate) fn create_genai_client(kind: ProviderKind, chat: &ModelTarget) -> Result<Client> {
    // Every chat transport carries its key as a bearer token, so the credential is read as a bare
    // key here; a chat kind that used its own key header needs its own path.
    let api_key = chat.credential.key().to_owned();
    let base_url = ensure_trailing_slash(chat.base_url.clone());

    let mut builder = Client::builder()
        .with_adapter_kind(kind.genai_adapter_kind())
        .with_auth_resolver_fn(move |_| Ok(Some(AuthData::from_single(api_key))))
        .with_service_target_resolver_fn(move |mut target: ServiceTarget| {
            target.endpoint = Endpoint::from_owned(base_url);
            Ok(target)
        });
    if !chat.headers.is_empty() {
        builder = builder
            .with_web_config(WebConfig::default().with_default_headers((*chat.headers).clone()));
    }
    Ok(builder.build())
}

/// The provider's declared headers as a header map; a malformed name or value fails startup rather
/// than every request.
pub(crate) fn header_map(headers: &HashMap<String, String>) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let key = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            AppError::message(format!("provider header name '{name}' is invalid: {e}"))
        })?;
        let value = HeaderValue::from_str(value).map_err(|e| {
            AppError::message(format!(
                "provider header '{name}' has an invalid value: {e}"
            ))
        })?;
        map.insert(key, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speaks(kind: ProviderKind, model_kind: ModelKind) -> bool {
        kind.endpoint_for(model_kind).is_some()
    }

    /// A kind answers only the model kinds it can actually speak.
    #[test]
    fn a_kind_lists_only_the_surfaces_it_speaks() {
        for model_kind in [
            ModelKind::Chat,
            ModelKind::Vision,
            ModelKind::Transcription,
            ModelKind::Embedding,
            ModelKind::Image,
            ModelKind::Speech,
            ModelKind::Rerank,
        ] {
            assert!(speaks(ProviderKind::OpenRouter, model_kind), "{model_kind}");
        }
        assert!(speaks(ProviderKind::Ollama, ModelKind::Embedding));
        assert!(!speaks(ProviderKind::Ollama, ModelKind::Speech));
        assert!(!speaks(ProviderKind::DeepSeek, ModelKind::Embedding));
        assert!(!speaks(ProviderKind::DeepSeek, ModelKind::Rerank));
        assert!(!speaks(ProviderKind::Anthropic, ModelKind::Vision));
        assert!(!speaks(ProviderKind::Google, ModelKind::Transcription));
        assert!(speaks(ProviderKind::OpenAI, ModelKind::Vision));
        assert!(!speaks(ProviderKind::OpenAI, ModelKind::Image));
    }

    /// The rerank kind of OpenRouter is absolute: a chat proxy must not move it.
    #[test]
    fn the_rerank_surface_carries_its_own_url() {
        let spec = ProviderKind::OpenRouter
            .endpoint_for(ModelKind::Rerank)
            .unwrap();
        assert_eq!(spec.url, Some(OPENROUTER_RERANK));
        assert_eq!(
            ProviderKind::OpenRouter
                .endpoint_for(ModelKind::Chat)
                .unwrap()
                .url,
            None
        );
    }
}
