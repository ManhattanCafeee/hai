use std::{sync::Arc, time::Duration};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    agent::runtime::turn::ToolContext,
    agentcore::tool::{AgentTool, ToolError, tool_err, tool_ok},
};

const MIN_SECS: u8 = 1;
const MAX_SECS: u8 = 60;
const DEFAULT_SECS: u8 = 3;

fn default_secs() -> u8 {
    DEFAULT_SECS
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitArgs {
    #[serde(default = "default_secs")]
    #[schemars(range(min = MIN_SECS, max = MAX_SECS))]
    secs: u8,
}

/// Wait for a moment.
///
/// When the other party may still be typing, or has not finished what they were saying, it can be worth waiting so that more messages arrive and are handled together.
#[hai_macros::tool]
pub struct Wait;

impl Wait {
    async fn exec(&self, args: WaitArgs) -> Result<serde_json::Value, ToolError> {
        if !(MIN_SECS..=MAX_SECS).contains(&args.secs) {
            return Err(tool_err(format!(
                "secs must be {MIN_SECS}-{MAX_SECS}, got {}",
                args.secs
            )));
        }
        tokio::time::sleep(Duration::from_secs(args.secs as u64)).await;
        tool_ok()
    }
}

pub fn tools(_ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Wait)]
}
