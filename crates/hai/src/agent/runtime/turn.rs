use std::sync::Arc;

use super::sandbox::Sandbox;
use crate::{
    agent::{
        channel::ChannelHandle, event::WakeEvents, multimodal::MultimodalService,
        runtime::AppContext,
    },
    agentcore::{Language, skills::SkillManager},
    domain::{model::ChatType, service::Services},
};

pub struct TurnContext {
    pub tool: ToolContext,
    pub app: AppContext,
    pub chat_type: ChatType,
    /// Fixed for the whole turn.
    pub language: Language,
    pub events: WakeEvents,
}

/// What one turn acts on and with: its own chat on the channel, and the services and sandbox behind
/// it. Domain reads address the chat by id (`chat.chat_id()`); anything that leaves through the
/// platform goes through the handle, which already knows the chat.
#[derive(Clone)]
pub struct ToolContext {
    pub chat: ChannelHandle,
    pub services: Services,
    pub sandbox: Arc<Sandbox>,
    pub skill_manager: SkillManager,
    pub multimodal: MultimodalService,
}
