pub mod env;
pub mod issue;
pub mod load;
pub mod meta;
pub mod model_role;
pub mod paths;
pub mod provider_manager;
pub mod schema;

use std::collections::HashMap;

pub use issue::ConfigIssue;
pub use paths::Paths;
pub use provider_manager::ProviderRegistry;
use serde::{Deserialize, Serialize};

use crate::config::schema::{
    AgentConfig, ChannelSection, DatabaseConfig, KnowledgeConfig, LoggingConfig, ModelRolesConfig,
    PromptConfig, ProviderConfig, SandboxConfig, SkillsConfig, ToolsConfig, WebConfig,
};

/// The whole config; [`AppConfig::load`] resolves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct AppConfig {
    pub agent: AgentConfig,
    pub prompt: PromptConfig,
    pub knowledge: KnowledgeConfig,
    pub providers: HashMap<String, ProviderConfig>,
    pub model_roles: ModelRolesConfig,
    pub web: WebConfig,
    pub channel: HashMap<String, ChannelSection>,
    pub mcp: HashMap<String, crate::agentcore::mcp::McpConfig>,
    pub sandbox: SandboxConfig,
    pub skills: SkillsConfig,
    pub tools: ToolsConfig,
    pub logging: LoggingConfig,
    pub database: DatabaseConfig,
}
