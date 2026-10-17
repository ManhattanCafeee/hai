use std::sync::Arc;

use genai::chat::ChatMessage;

use crate::{agentcore::tool::AgentTool, domain::vo::Step};

#[derive(Clone)]
pub struct Messages(Arc<Vec<ChatMessage>>);

impl Messages {
    pub fn new(msgs: Vec<ChatMessage>) -> Self {
        Self(Arc::new(msgs))
    }

    pub fn push(&mut self, msg: ChatMessage) {
        Arc::make_mut(&mut self.0).push(msg);
    }

    pub fn extend(&mut self, msgs: impl IntoIterator<Item = ChatMessage>) {
        Arc::make_mut(&mut self.0).extend(msgs);
    }

    pub fn to_vec(&self) -> Vec<ChatMessage> {
        (*self.0).clone()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

pub struct TurnOutput {
    pub messages: Messages,
    pub steps: Vec<Step>,
    pub has_spoken: bool,
}

pub(crate) fn has_spoken(steps: &[Step], tools: &[Arc<dyn AgentTool>]) -> bool {
    steps
        .iter()
        .flat_map(|s| &s.tool_calls)
        .any(|tc| tools.iter().any(|t| t.name() == tc.tool_name && t.speaks()))
}

/// The completion signal of a Busy state: the outcome of a turn, of a steering interruption, or of
/// a wrap-up.
pub(crate) enum BusySignal {
    Turn(TurnOutput),
    /// Ended early because something new arrived mid-turn (steering): everything already
    /// processed takes effect, and the arrival starts the next turn right away.
    Steered(TurnOutput),
    WrapUp {
        summary: String,
        context_tokens: u32,
    },
    TurnFailed,
    WrapUpFailed(String),
    /// Cut short because the process is stopping: nothing is consumed, so the next start reads the
    /// same accumulation again.
    Cancelled,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::has_spoken;
    use crate::{
        agent::test_support::StubTool,
        agentcore::tool::AgentTool,
        domain::vo::{Step, ToolCallResult},
    };

    fn step(tool: &str) -> Step {
        Step {
            tool_calls: vec![ToolCallResult::ok(tool, json!({}), json!({"ok": true}))],
            response: String::new(),
            reasoning: None,
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    #[test]
    fn has_spoken_follows_tool_metadata() {
        let tools: Vec<Arc<dyn AgentTool>> = vec![
            StubTool::new("send_message").speaks(true).into_dyn(),
            StubTool::new("topic").into_dyn(),
        ];

        assert!(has_spoken(&[step("send_message")], &tools));
        assert!(!has_spoken(&[step("topic")], &tools));
        assert!(!has_spoken(&[step("unknown_tool")], &tools));
    }
}
