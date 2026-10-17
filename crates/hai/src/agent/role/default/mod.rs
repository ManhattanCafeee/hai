pub(crate) mod prompt;

use std::sync::Arc;

use genai::chat::ChatMessage;

use crate::{
    agent::{
        role::{StepDecision, StepFacts},
        runtime::{AgentRuntime, prompts::environment, turn::ToolContext},
        tools::{knowledge, memory, read, recall, schedule, send, shell, skip, topic, wait, web},
    },
    agentcore::{Language, tool::AgentTool},
    domain::{
        model::ChatType,
        vo::{ModelRetryReason, StepNumber},
    },
};

pub(crate) fn system_prompt(
    runtime: &AgentRuntime,
    chat_type: ChatType,
    language: Language,
) -> String {
    let cfg = &runtime.cfg().agent;
    let prompts = &runtime.prompts;
    prompt::SystemPromptBuilder::new()
        .personality(&cfg.personality.name, prompts.soul(language))
        .agents(prompts.agents(language))
        .environment(environment(runtime.cfg().sandbox.enabled))
        .chat_type(
            prompts.group(language),
            prompts.private(language),
            chat_type,
        )
        .language(language)
        .skills(&runtime.skill_manager)
        .build()
}

pub(crate) fn tools(runtime: &AgentRuntime, ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    let mut all = Vec::new();
    all.extend(skip::tools(ctx));
    all.extend(wait::tools(ctx));
    all.extend(send::tools(ctx));
    all.extend(topic::tools(ctx));
    all.extend(schedule::tools(ctx));
    all.extend(memory::tools(ctx));
    all.extend(recall::tools(ctx));
    all.extend(knowledge::tools(runtime, ctx));
    all.extend(read::tools(runtime, ctx));
    all.extend(web::tools(runtime));
    all.extend(shell::tools(ctx));
    all
}

const MAX_CONTRACT_RETRIES: u32 = 1;

pub(super) fn decide(facts: StepFacts, violations: &mut u32) -> StepDecision {
    let violating = !facts.text_empty && facts.non_skip_calls == 0;
    *violations = if violating { *violations + 1 } else { 0 };
    if violating && *violations <= MAX_CONTRACT_RETRIES {
        return StepDecision::Retry {
            reason: ModelRetryReason::ResponseWithText,
            guide: speaking_contract_guide(facts.step),
        };
    }
    if facts.skipped || facts.non_skip_calls == 0 {
        return StepDecision::Stop;
    }
    StepDecision::Continue
}

fn speaking_contract_guide(step: StepNumber) -> ChatMessage {
    ChatMessage::user(format!(
        "[speaking contract] Step {} violated the contract: the response text was not emitted via send_message. Emit that same text via send_message, or call skip to end the turn.",
        step.0
    ))
}

#[cfg(test)]
mod tests {
    use super::{StepDecision, StepFacts, decide};
    use crate::domain::vo::StepNumber;

    fn facts(text_empty: bool, skipped: bool, non_skip_calls: usize) -> StepFacts {
        StepFacts {
            step: StepNumber::from(1),
            text_empty,
            calls_empty: non_skip_calls == 0 && !skipped,
            skipped,
            non_skip_calls,
        }
    }

    #[test]
    fn skip_with_empty_text_stops() {
        let mut violations = 0;
        assert!(matches!(
            decide(facts(true, true, 0), &mut violations),
            StepDecision::Stop
        ));
        assert_eq!(violations, 0);
    }

    #[test]
    fn skip_with_text_retries_then_stops() {
        let mut violations = 0;
        assert!(matches!(
            decide(facts(false, true, 0), &mut violations),
            StepDecision::Retry { .. }
        ));
        assert_eq!(violations, 1);
        assert!(matches!(
            decide(facts(false, true, 0), &mut violations),
            StepDecision::Stop
        ));
        assert_eq!(violations, 2);
    }

    #[test]
    fn text_only_retries_then_stops() {
        let mut violations = 0;
        assert!(matches!(
            decide(facts(false, false, 0), &mut violations),
            StepDecision::Retry { .. }
        ));
        assert_eq!(violations, 1);
        assert!(matches!(
            decide(facts(false, false, 0), &mut violations),
            StepDecision::Stop
        ));
        assert_eq!(violations, 2);
    }

    #[test]
    fn send_call_with_text_continues_and_resets_strikes() {
        let mut violations = 1;
        assert!(matches!(
            decide(facts(false, false, 1), &mut violations),
            StepDecision::Continue
        ));
        assert_eq!(violations, 0);
    }

    #[test]
    fn empty_text_without_calls_stops() {
        let mut violations = 0;
        assert!(matches!(
            decide(facts(true, false, 0), &mut violations),
            StepDecision::Stop
        ));
        assert_eq!(violations, 0);
    }
}
