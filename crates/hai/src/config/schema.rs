use std::{collections::HashMap, path::PathBuf, str::FromStr};

use genai::chat::ReasoningEffort;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr};

use crate::{
    agentcore::{Language, ModelKind, ProviderKind},
    config::{
        ConfigIssue, Paths,
        meta::{AGENT_NAME, PROJECT_NAME},
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct PersonalityConfig {
    pub name: String,
}

impl Default for PersonalityConfig {
    fn default() -> Self {
        Self {
            name: AGENT_NAME.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct AgentConfig {
    /// Default for chats that never set their own (`docs/topics/prompting.md`).
    pub language: Language,

    pub context: ContextConfig,
    pub personality: PersonalityConfig,
    pub attention: AttentionConfig,
    pub multimodal: MultimodalConfig,
    pub retrieval: RetrievalConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct PromptConfig {
    pub soul: Option<String>,
    pub agents: Option<String>,
    pub group: Option<String>,
    pub private: Option<String>,
    pub wrap_up: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ContextConfig {
    /// First-round seed: unread messages, topped up with history to reach this count.
    pub context_seed_cap: i64,
    /// Cap on pinned, still-valid Memory entries in the chapter-opening injection block.
    pub pinned_memory_limit: i64,
    /// Idle hours after which a Topic renders the need-close hint.
    pub topic_idle_hours: i64,
    /// Seconds without a run after which the chapter closes.
    pub session_idle_timeout_secs: u64,
    pub steering: bool,
    /// Context tokens (the last Turn's prompt tokens) at which the chapter wraps up; 0 disables.
    pub compact_token_threshold: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            context_seed_cap: 10,
            pinned_memory_limit: 20,
            topic_idle_hours: 2,
            session_idle_timeout_secs: 600,
            steering: true,
            compact_token_threshold: 150_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct AttentionConfig {
    /// Baseline attention (0-1): the chance an idle agent attends to a Chat unprompted.
    pub base_attention: f64,
    /// How long the agent keeps attending after being addressed or after speaking itself; with the
    /// attention window open, everything pending dispatches.
    pub window_secs: f64,
}

impl Default for AttentionConfig {
    fn default() -> Self {
        Self {
            base_attention: 0.05,
            window_secs: 30.0,
        }
    }
}

/// How HAI reads attached media: the switches and the analysis templates, kept apart from which
/// model does the reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct MultimodalConfig {
    pub vision: VisionAnalysisConfig,
    pub transcription: TranscriptionAnalysisConfig,
    /// What one request may carry inline; `0` leaves it unbounded. A file over it is refused before
    /// it is read, since the provider's media shapes carry their data in the request.
    pub inline_max_bytes: u64,
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            vision: VisionAnalysisConfig::default(),
            transcription: TranscriptionAnalysisConfig::default(),
            inline_max_bytes: 20 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct VisionAnalysisConfig {
    pub enabled: bool,
    pub image_prompt: Option<String>,
    pub video_prompt: Option<String>,
}

impl Default for VisionAnalysisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            image_prompt: None,
            video_prompt: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TranscriptionAnalysisConfig {
    pub enabled: bool,
    pub prompt: Option<String>,
}

impl Default for TranscriptionAnalysisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt: None,
        }
    }
}

/// Retrieval scoring: the candidate pool one rerank request carries, and how long it may take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct RetrievalConfig {
    pub candidates: usize,
    pub candidate_chars: usize,
    pub timeout_secs: u64,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            candidates: 30,
            candidate_chars: 768,
            timeout_secs: 10,
        }
    }
}

/// Model selection per ModelRole: the `default` chat role plus one block per role. A block declares
/// the model and how to call it; how HAI behaves around it lives under `[agent]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ModelRolesConfig {
    pub default: ChatConfig,
    pub embedding: EmbeddingConfig,
    /// Analyzing video reuses this block; there is no separate one.
    pub vision: Option<VisionConfig>,
    pub transcription: Option<TranscriptionConfig>,
    pub speech: Option<SpeechConfig>,
    pub image: Option<ImageConfig>,
    pub rerank: Option<RerankConfig>,
}

/// The `default` ModelRole: the chat model both `AgentRole`s run on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ChatConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub reasoning: bool,
    pub reasoning_effort: String,
    pub temperature: f32,
    /// The endpoint to reach this model at, when it is not the provider's own.
    pub base_url: Option<String>,
}

impl ChatConfig {
    pub fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        let r = ReasoningEffort::from_keyword(&self.reasoning_effort);
        if r.is_none() {
            tracing::warn!("Invalid reasoning_effort '{}'", self.reasoning_effort);
        }
        r
    }
}

/// The `rerank` ModelRole: the cross-encoder that scores retrieval candidates. The provider answers
/// that kind unless the block states its own `base-url`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct RerankConfig {
    /// Which provider serves the rerank kind; an empty name is the `default` block's provider.
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
}

/// Ceiling on the characters one rerank request carries. A cross-encoder answers within its own
/// context window and a CJK document spends about a token per character, so the pool stays under
/// this with room for the query.
pub const RERANK_REQUEST_CHARS: usize = 24_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct VisionConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TranscriptionConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct SpeechConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub voice: String,
    /// Speech rate; the API client clamps it to 0.25..4.0.
    pub speed: f32,
    pub base_url: Option<String>,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            provider: None,
            model: None,
            voice: String::new(),
            speed: 1.0,
            base_url: None,
        }
    }
}

/// The `embedding` ModelRole. `dimension` is the model's output width, and the pgvector columns are
/// created with it; the block has to state it, since HAI does not know a model's width.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct EmbeddingConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub dimension: Option<i32>,
    pub base_url: Option<String>,
}

/// The width the embedding vectors are stored at. The block states it: a guess would size every
/// vector column wrongly.
pub fn resolve_embedding_dimension(block: &EmbeddingConfig) -> Result<i32> {
    block.dimension.ok_or_else(|| {
        ConfigIssue::model_role(
            ModelKind::Embedding,
            "dimension is required (the pgvector columns are created with it)",
        )
        .into()
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ImageConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

/// Networking: the body-extraction chain behind `read` and the retrieval chain behind `web_search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct WebConfig {
    pub fetch: WebFetchConfig,
    pub search: WebSearchConfig,
    /// Provider credentials, shared by the search and fetch chains.
    pub providers: WebProvidersConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct WebFetchConfig {
    pub chain: Vec<FetchEngine>,
    pub timeout_secs: u64,
    pub max_bytes: usize,
    pub user_agent: String,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            chain: vec![
                FetchEngine::Local,
                FetchEngine::Provider(WebProvider::Parallel),
            ],
            timeout_secs: 20,
            max_bytes: 2 * 1024 * 1024,
            user_agent: format!("{}/{}", PROJECT_NAME, env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Body-extraction engine. `local` extracts in-process with no endpoint or credential; every
/// other value names a [`WebProvider`] and is usable only if its declared capability includes
/// `fetch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum FetchEngine {
    Local,
    Provider(WebProvider),
}

impl FetchEngine {
    /// The one spelling of the local engine: [`FetchEngine::as_str`], the parser and the error text
    /// all read it.
    const LOCAL: &'static str = "local";

    pub fn as_str(self) -> &'static str {
        match self {
            FetchEngine::Local => Self::LOCAL,
            FetchEngine::Provider(provider) => provider.into(),
        }
    }
}

impl TryFrom<String> for FetchEngine {
    type Error = AppError;

    fn try_from(raw: String) -> std::result::Result<Self, Self::Error> {
        raw.parse().map_err(|_| {
            let providers: Vec<&str> = WebProvider::BUILTIN_ORDER
                .iter()
                .map(|id| (*id).into())
                .collect();
            AppError::message(format!(
                "unknown fetch engine {raw:?}, expected {:?} or one of {}",
                FetchEngine::LOCAL,
                providers.join(", ")
            ))
        })
    }
}

impl From<FetchEngine> for String {
    fn from(engine: FetchEngine) -> Self {
        engine.as_str().to_owned()
    }
}

impl std::fmt::Display for FetchEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FetchEngine {
    type Err = strum::ParseError;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        match raw {
            Self::LOCAL => Ok(FetchEngine::Local),
            other => other.parse().map(FetchEngine::Provider),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Empty falls back to the built-in order.
    pub chain: Vec<WebProvider>,
    pub timeout_secs: u64,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            timeout_secs: 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct WebProvidersConfig {
    pub parallel: ApiKeyConfig,
    pub exa: ApiKeyConfig,
    pub tavily: ApiKeyConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ApiKeyConfig {
    pub api_key: Option<String>,
}

/// Web provider identity, shared by the search and fetch chains; the capability table lives in
/// `agent/web/provider/table.rs`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum WebProvider {
    #[default]
    Parallel,
    Exa,
    Tavily,
}

impl WebProvider {
    pub const BUILTIN_ORDER: [WebProvider; 3] =
        [WebProvider::Parallel, WebProvider::Exa, WebProvider::Tavily];
}

impl WebProvidersConfig {
    /// This provider's `api-key`; blank counts as unconfigured.
    pub fn api_key(&self, provider: WebProvider) -> Option<&str> {
        let slot = match provider {
            WebProvider::Parallel => &self.parallel,
            WebProvider::Exa => &self.exa,
            WebProvider::Tavily => &self.tavily,
        };
        slot.api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 10,
        }
    }
}

impl ProviderConfig {
    /// The chat transport this entry names: the explicit `type`, else the name when it is one.
    /// A provider that serves only a non-chat kind (a self-hosted reranker) names neither.
    pub fn kind(&self, name: &str) -> Option<ProviderKind> {
        self.r#type.or_else(|| ProviderKind::from_str(name).ok())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ProviderConfig {
    pub r#type: Option<ProviderKind>,
    /// API key; omittable for local services such as Ollama.
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    /// Extra request headers for this provider (gateways and resellers).
    pub headers: HashMap<String, String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::Display,
    IntoStaticStr,
    EnumString,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

impl ContainerRuntime {
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }

    fn detect() -> Self {
        if which::which("podman").is_ok() {
            Self::Podman
        } else {
            Self::Docker
        }
    }
}

impl Default for ContainerRuntime {
    fn default() -> Self {
        Self::detect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct KnowledgeConfig {
    /// When false, `search_knowledge_base` is not mounted.
    pub enabled: bool,
    /// Target chunk length, in Unicode chars.
    pub chunk_size: usize,
    /// Overlap between adjacent chunks, in Unicode chars.
    pub chunk_overlap: usize,
    /// Split trigger and sub-split budget for one unit, in Unicode chars; a unit that cannot be
    /// split below it stays oversized.
    pub chunk_max: usize,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_size: 512,
            chunk_overlap: 51,
            chunk_max: 1536,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub runtime: ContainerRuntime,
    pub image: String,
    pub timeout_secs: u64,
    /// Extra directories bound read-only into the container at their own paths.
    pub mounts: Vec<PathBuf>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            runtime: ContainerRuntime::default(),
            image: "ubuntu:latest".into(),
            timeout_secs: 30,
            mounts: Vec::new(),
        }
    }
}

/// Directories scanned for `SKILL.md`, both global and project-local.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct SkillsConfig {
    pub dirs: Vec<PathBuf>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            dirs: Paths::inferred().skill_dirs().to_vec(),
            disabled: Vec::new(),
        }
    }
}

/// Disabled tool names; an MCP tool is named by its registration name `mcp__<server>_<tool>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ToolsConfig {
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
        }
    }
}

impl LoggingConfig {
    pub fn level(&self) -> Result<tracing::Level> {
        tracing::Level::from_str(&self.level)
            .map_err(|e| AppError::message(format!("Invalid logging level '{}': {e}", self.level)))
    }
}

/// One `[channel.<name>]` section: the platform it names and the keys only that platform understands.
/// The shared layer passes the section on untouched; the platform that claims it parses and
/// validates its own keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ChannelSection {
    /// The platform this section names; absent means the section's own name is the platform.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// The section's own keys, as written.
    #[serde(flatten)]
    pub settings: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::test_support::assert_names_agree;

    /// `[channel.<name>]` is neutral: the shared layer keeps the platform's own keys untouched, so a
    /// second platform's keys need no change here.
    #[test]
    fn a_channel_section_keeps_its_platforms_keys() {
        use serde_json::json;

        use crate::config::AppConfig;

        let cfg: AppConfig = toml::from_str(
            "[channel.telegram]\n\
             bot-token = \"a-token\"\n\
             allowed-chat-ids = [1]\n\
             rich-message = true\n\
             \n\
             [channel.main]\n\
             type = \"telegram\"\n",
        )
        .expect("a readable config");

        let named = &cfg.channel["telegram"];
        assert_eq!(named.kind, None, "no type means the name is the platform");
        assert_eq!(named.settings["bot-token"], json!("a-token"));
        assert_eq!(named.settings["allowed-chat-ids"], json!([1]));
        assert_eq!(cfg.channel["main"].kind.as_deref(), Some("telegram"));

        // `hai config --format toml` prints the section as written.
        let printed = toml::to_string_pretty(&cfg).expect("a printable config");
        assert!(printed.contains("bot-token"), "{printed}");
        assert!(printed.contains("rich-message"), "{printed}");
    }

    #[test]
    fn config_enum_names_agree_between_serde_and_strum() {
        assert_names_agree(
            [WebProvider::Parallel, WebProvider::Exa, WebProvider::Tavily],
            |provider| provider.into(),
        );
        assert_names_agree(
            [
                crate::agentcore::mcp::McpTransport::Stdio,
                crate::agentcore::mcp::McpTransport::StreamableHttp,
            ],
            |transport| transport.into(),
        );
        assert_names_agree(
            [ContainerRuntime::Docker, ContainerRuntime::Podman],
            |runtime| runtime.as_str(),
        );
        assert_names_agree(ProviderKind::iter(), |kind| kind.into());
    }

    /// The two bounds on a rerank request have to hold together for the default to be usable.
    #[test]
    fn the_rerank_defaults_fit_a_chunk_and_a_request() {
        let retrieval = AgentConfig::default().retrieval;
        let knowledge = KnowledgeConfig::default();

        assert!(
            retrieval.candidate_chars >= knowledge.chunk_size,
            "a candidate narrower than a chunk would rerank a truncated passage"
        );
        assert!(
            retrieval.candidates * retrieval.candidate_chars <= RERANK_REQUEST_CHARS,
            "the candidate pool would not fit one rerank request"
        );
    }

    #[test]
    fn fetch_engine_round_trips_as_flat_strings() {
        let parsed: Vec<FetchEngine> =
            serde_json::from_str(r#"["local","parallel","exa"]"#).expect("config vocabulary");
        assert_eq!(
            parsed,
            vec![
                FetchEngine::Local,
                FetchEngine::Provider(WebProvider::Parallel),
                FetchEngine::Provider(WebProvider::Exa)
            ]
        );
        assert_eq!(
            serde_json::to_string(&parsed).expect("serialize"),
            r#"["local","parallel","exa"]"#
        );
    }

    #[test]
    fn api_key_reads_the_provider_slot_and_ignores_blanks() {
        let mut cfg = WebProvidersConfig::default();
        assert!(cfg.api_key(WebProvider::Tavily).is_none());

        cfg.tavily.api_key = Some("  ".to_owned());
        assert!(cfg.api_key(WebProvider::Tavily).is_none());

        cfg.tavily.api_key = Some(" k ".to_owned());
        assert_eq!(cfg.api_key(WebProvider::Tavily), Some("k"));

        cfg.exa.api_key = Some("e".to_owned());
        assert_eq!(cfg.api_key(WebProvider::Exa), Some("e"));
        assert!(cfg.api_key(WebProvider::Parallel).is_none());
    }

    #[test]
    fn fetch_engine_rejects_unknown_vocabulary() {
        assert!(serde_json::from_str::<FetchEngine>(r#""brave""#).is_err());
        assert!(serde_json::from_str::<FetchEngine>(r#"{"provider":"exa"}"#).is_err());
    }

    #[test]
    fn a_missing_embedding_dimension_is_an_error() {
        assert!(resolve_embedding_dimension(&EmbeddingConfig::default()).is_err());
    }
}
