pub(crate) mod default;
pub(crate) mod wrap_up;

use std::sync::Arc;

use genai::chat::{ChatMessage, ChatOptions, ToolCall};

use crate::{
    agent::runtime::AgentRuntime,
    agentcore::{Language, tool::AgentTool},
    config::schema::ChatConfig,
    domain::{
        model::ChatType,
        vo::{ModelRetryReason, StepNumber},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentRole {
    Default,
    WrapUp,
}

pub(crate) enum StepDecision {
    Continue,
    Stop,
    Retry {
        reason: ModelRetryReason,
        guide: ChatMessage,
    },
}

pub(crate) struct StepFacts {
    pub step: StepNumber,
    pub text_empty: bool,
    pub calls_empty: bool,
    pub skipped: bool,
    pub non_skip_calls: usize,
}

impl StepFacts {
    pub(crate) fn from_calls(
        step: StepNumber,
        text: &str,
        calls: &[ToolCall],
        tools: &[Arc<dyn AgentTool>],
    ) -> Self {
        let meta = |name: &str| tools.iter().find(|t| t.name() == name);
        let mut skipped = false;
        let mut non_skip_calls = 0;
        for call in calls {
            match meta(&call.fn_name) {
                Some(tool) if tool.ends_turn() => skipped = true,
                _ => non_skip_calls += 1,
            }
        }
        Self {
            step,
            text_empty: text.trim().is_empty(),
            calls_empty: calls.is_empty(),
            skipped,
            non_skip_calls,
        }
    }
}

pub(crate) struct RoleSpec {
    role: AgentRole,
    pub system_prompt: String,
    pub options: ChatOptions,
    pub steering: bool,
    contract_violations: u32,
}

impl RoleSpec {
    pub(crate) fn decide_step(&mut self, facts: StepFacts) -> StepDecision {
        match self.role {
            AgentRole::Default => default::decide(facts, &mut self.contract_violations),
            AgentRole::WrapUp => wrap_up::decide(facts),
        }
    }

    /// Whether this role's run is one the chat is waiting on: a Turn is the agent answering the
    /// chat, a wrap-up is bookkeeping the chat should not see.
    pub(crate) fn shows_typing(&self) -> bool {
        matches!(self.role, AgentRole::Default)
    }

    pub(crate) fn rejects(&self, tool: &dyn AgentTool) -> bool {
        matches!(self.role, AgentRole::WrapUp)
            && (tool.speaks() || tool.ends_turn() || tool.produces())
    }
}

impl AgentRole {
    pub(crate) fn spec(
        self,
        runtime: &AgentRuntime,
        chat_type: ChatType,
        language: Language,
    ) -> RoleSpec {
        let cfg = &runtime.cfg().agent;
        let chat = &runtime.cfg().model_roles.default;
        match self {
            AgentRole::Default => RoleSpec {
                role: self,
                system_prompt: default::system_prompt(runtime, chat_type, language),
                options: build_chat_options(chat),
                steering: cfg.context.steering,
                contract_violations: 0,
            },
            AgentRole::WrapUp => RoleSpec {
                role: self,
                system_prompt: default::system_prompt(runtime, chat_type, language),
                options: build_chat_options(chat),
                steering: false,
                contract_violations: 0,
            },
        }
    }
}

pub(crate) fn build_chat_options(cfg: &ChatConfig) -> ChatOptions {
    let mut opts = ChatOptions::default().with_temperature(cfg.temperature as f64);
    if let Some(maxt) = cfg.max_tokens {
        opts = opts.with_max_tokens(maxt);
    }
    if cfg.reasoning
        && let Some(effort) = cfg.reasoning_effort()
    {
        opts = opts.with_reasoning_effort(effort);
    }
    opts
}

#[cfg(test)]
mod tests {
    use genai::chat::{ChatOptions, ToolCall};

    use super::{AgentRole, RoleSpec, StepFacts};
    use crate::{agent::test_support::StubTool, domain::vo::StepNumber};

    fn call(name: &str) -> ToolCall {
        ToolCall {
            call_id: "1".into(),
            fn_name: name.into(),
            fn_arguments: serde_json::json!({}),
            thought_signatures: None,
        }
    }

    #[test]
    fn step_facts_come_from_tool_metadata() {
        let tools = vec![
            StubTool::new("skip").ends_turn(true).into_dyn(),
            StubTool::new("send_message").speaks(true).into_dyn(),
            StubTool::new("generate_image").produces(true).into_dyn(),
            StubTool::new("mcp_skip").into_dyn(),
        ];

        let skipped = StepFacts::from_calls(StepNumber::from(1), "hi", &[call("skip")], &tools);
        assert!(skipped.skipped);
        assert_eq!(skipped.non_skip_calls, 0);

        // Neither call skips the turn: each counts one non-skip call.
        for name in ["send_message", "generate_image"] {
            let facts = StepFacts::from_calls(StepNumber::from(1), "hi", &[call(name)], &tools);
            assert_eq!(facts.non_skip_calls, 1, "{name}");
        }

        // `ends_turn` comes from tool metadata only: a tool whose name looks like `skip`
        // but does not declare it keeps the turn going
        let lookalike = StepFacts::from_calls(StepNumber::from(1), "", &[call("mcp_skip")], &tools);
        assert!(!lookalike.skipped);
        assert_eq!(lookalike.non_skip_calls, 1);
    }

    #[test]
    fn a_wrap_up_rejects_what_speaks_ends_or_produces() {
        let spec = RoleSpec {
            role: AgentRole::WrapUp,
            system_prompt: String::new(),
            options: ChatOptions::default(),
            steering: false,
            contract_violations: 0,
        };

        assert!(spec.rejects(&*StubTool::new("send_message").speaks(true).into_dyn()));
        assert!(spec.rejects(&*StubTool::new("skip").ends_turn(true).into_dyn()));
        assert!(spec.rejects(&*StubTool::new("generate_image").produces(true).into_dyn()));
        assert!(!spec.rejects(&*StubTool::new("topic").into_dyn()));
    }
}
