use std::sync::Arc;

use serde_json::Value;

use crate::{
    agent::runtime::turn::ToolContext,
    agentcore::tool::{AgentTool, ToolError, tool_ok},
};

/// End this turn.
#[hai_macros::tool(args = none, ends_turn)]
pub struct Skip;

impl Skip {
    async fn exec(&self) -> Result<Value, ToolError> {
        tool_ok()
    }
}

pub fn tools(_ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Skip)]
}
