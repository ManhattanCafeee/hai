use crate::{
    agent::{runtime::registry::AgentRegistry, settings::Settings},
    domain::{service::Services, vo::ChatId},
};

/// What a command needs from the host: the chat it arrived in, the settings it may read or change,
/// the services it may write through, and the agent it may ring.
pub(crate) struct CommandContext {
    pub(crate) chat_id: ChatId,
    pub(crate) settings: Settings,
    pub(crate) agents: AgentRegistry,
    pub(crate) services: Services,
}
