pub(crate) mod agent_loop;
pub(crate) mod chat;
pub(crate) mod context;
pub(crate) mod engine;
pub(crate) mod event;
pub(crate) mod prompts;
pub(crate) mod registry;
pub(crate) mod sandbox;
pub(crate) mod startup_refeed;
pub(crate) mod task_watcher;
pub(crate) mod turn;
pub(crate) mod types;
pub(crate) mod workspace;

use std::sync::Arc;

pub use context::AppContext;
pub(crate) use context::assemble;
use derive_more::Deref;
pub use event::AgentEventBus;
use genai::Client;

use crate::{
    agent::{multimodal::MultimodalService, web::WebService},
    agentcore::{mcp::McpManager, provider, skills::SkillManager},
    config::AppConfig,
    domain::service::Services,
    error::Result,
};

#[derive(Clone, Deref)]
pub struct AgentRuntime(pub(crate) Arc<AgentRuntimeInner>);

pub struct AgentRuntimeInner {
    pub app: AppContext,
    pub client: Client,
    pub model: String,
    pub mcp_manager: McpManager,
    pub skill_manager: SkillManager,
    pub prompts: prompts::PromptLayers,
    pub web: WebService,
}

impl AgentRuntime {
    pub async fn new(app: AppContext) -> Result<Self> {
        let cfg = Arc::clone(&app.cfg);
        let mcp_manager = McpManager::from_config(&cfg.mcp).await?;
        let skill_manager = SkillManager::load(&cfg.skills.dirs, &cfg.skills.disabled).await?;
        let prompts = prompts::resolve(&cfg)?;
        let web = WebService::from_config(&cfg.web)?;

        let chat = &app.chat_transport;
        let client = provider::create_genai_client(chat.kind, &chat.target)?;

        let model = chat.target.model.clone();

        Ok(Self(Arc::new(AgentRuntimeInner {
            app,
            client,
            model,
            mcp_manager,
            skill_manager,
            prompts,
            web,
        })))
    }

    /// The process-wide values every reader asks the runtime for.
    pub(crate) fn cfg(&self) -> &AppConfig {
        &self.app.cfg
    }

    pub(crate) fn services(&self) -> &Services {
        &self.app.services
    }

    pub(crate) fn event_bus(&self) -> &AgentEventBus {
        &self.app.event_bus
    }

    pub(crate) fn multimodal(&self) -> &MultimodalService {
        &self.app.multimodal
    }
}
