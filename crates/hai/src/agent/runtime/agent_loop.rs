mod retry;

use std::sync::Arc;

use genai::{
    Client,
    chat::{ChatMessage, ContentPart, MessageContent, Tool, ToolCall, ToolResponse},
};

use super::{event::Doorbell, types::Messages};
use crate::{
    agent::{
        role::{RoleSpec, StepDecision, StepFacts},
        runtime::AgentRuntime,
    },
    agentcore::tool::{AgentTool, ToolError, schema, tool_err},
    domain::vo::{
        AgentEventPayload, ChatId, ModelRetryReason, Step, StepNumber, StepOutput, ToolCallResult,
        TurnNumber,
    },
};

const MAX_STEPS: usize = 20;

pub(crate) struct LoopTurn {
    pub client: Client,
    pub model: String,
    pub messages: Messages,
    pub spec: RoleSpec,
    pub doorbell: Doorbell,
    pub event_bus: super::AgentEventBus,
    pub chat_id: ChatId,
    pub turn_number: TurnNumber,
}

impl LoopTurn {
    pub(crate) fn new(
        runtime: &AgentRuntime,
        spec: RoleSpec,
        chat_id: ChatId,
        messages: Messages,
        doorbell: Doorbell,
        turn_number: TurnNumber,
    ) -> Self {
        Self {
            client: runtime.client.clone(),
            model: runtime.model.clone(),
            messages,
            spec,
            doorbell,
            event_bus: runtime.event_bus().clone(),
            chat_id,
            turn_number,
        }
    }
}

pub(crate) struct LoopOutput {
    pub steps: Vec<Step>,
    pub messages: Messages,
    /// Whether something new arrived mid-turn.
    pub steered: bool,
}

pub(crate) async fn run_loop(
    mut turn: LoopTurn,
    tools: &[Arc<dyn AgentTool>],
) -> Result<LoopOutput, ToolError> {
    let genai_tools = prepare_genai_tools(tools);
    let mut steps: Vec<Step> = Vec::new();

    loop {
        let step = run_step(&mut turn, &genai_tools, tools, steps.len()).await?;
        let hit_step_limit = !step.stop && steps.len() + 1 >= MAX_STEPS;
        if hit_step_limit {
            tracing::warn!(
                turn = turn.turn_number.0,
                step = steps.len() + 1,
                "turn truncated at MAX_STEPS"
            );
        }
        steps.push(step.step);

        if let Some(reason) = step.retry {
            turn.event_bus.emit(
                turn.chat_id,
                AgentEventPayload::ModelRetry {
                    turn: turn.turn_number,
                    reason,
                },
            );
        }

        if step.stop || hit_step_limit {
            return Ok(LoopOutput {
                steps,
                messages: turn.messages,
                steered: false,
            });
        }

        if turn.spec.steering && turn.doorbell.take_signal() {
            return Ok(LoopOutput {
                steps,
                messages: turn.messages,
                steered: true,
            });
        }
    }
}

struct StepRun {
    step: Step,
    stop: bool,
    retry: Option<ModelRetryReason>,
}

async fn run_step(
    turn: &mut LoopTurn,
    genai_tools: &[Tool],
    tools: &[Arc<dyn AgentTool>],
    index: usize,
) -> Result<StepRun, ToolError> {
    let step = StepNumber::from(index + 1);
    let res = retry::llm_call_with_retry(turn, genai_tools, &turn.messages).await?;

    let response_text = res.texts().join("\n");
    let reasoning = res.reasoning_content.clone();
    let prompt_tokens = res.usage.prompt_tokens.unwrap_or(0) as u32;
    let completion_tokens = res.usage.completion_tokens.unwrap_or(0) as u32;
    let active_calls: Vec<ToolCall> = res.into_tool_calls();

    turn.messages.push(build_assistant_message(
        &response_text,
        &active_calls,
        reasoning.clone(),
    ));

    turn.event_bus.emit(
        turn.chat_id,
        AgentEventPayload::StepCompleted {
            turn: turn.turn_number,
            step,
            output: StepOutput {
                turn: turn.turn_number,
                step,
                reasoning: reasoning.clone(),
                response: response_text.clone(),
            },
        },
    );

    let tool_calls = run_tool_calls(turn, &active_calls, tools, step).await;

    let (stop, retry) = match turn.spec.decide_step(StepFacts::from_calls(
        step,
        &response_text,
        &active_calls,
        tools,
    )) {
        StepDecision::Continue => (false, None),
        StepDecision::Stop => (true, None),
        StepDecision::Retry { reason, guide } => {
            turn.messages.push(guide);
            (false, Some(reason))
        }
    };

    Ok(StepRun {
        step: Step {
            tool_calls,
            response: response_text,
            reasoning,
            prompt_tokens,
            completion_tokens,
        },
        stop,
        retry,
    })
}

async fn run_tool_calls(
    turn: &mut LoopTurn,
    calls: &[ToolCall],
    tools: &[Arc<dyn AgentTool>],
    step: StepNumber,
) -> Vec<ToolCallResult> {
    let mut results: Vec<ToolCallResult> = Vec::new();
    for call in calls {
        turn.event_bus.emit(
            turn.chat_id,
            AgentEventPayload::ToolCall {
                turn: turn.turn_number,
                step,
                tool: call.fn_name.clone(),
                args: call.fn_arguments.to_string(),
            },
        );
        let result = execute_single_tool(call, tools, &turn.spec, &mut turn.messages).await;
        turn.event_bus.emit(
            turn.chat_id,
            AgentEventPayload::ToolCallResult {
                turn: turn.turn_number,
                step,
                tool: call.fn_name.clone(),
                summary: result.result.to_string(),
                success: result.success,
            },
        );
        results.push(result);
    }
    results
}

fn prepare_genai_tools(tools: &[Arc<dyn AgentTool>]) -> Vec<Tool> {
    tools
        .iter()
        .map(|t| {
            let mut tool = Tool::new(t.name()).with_description(t.description());
            if let Some(s) = t.schema() {
                tool = tool.with_schema(schema::normalize_root(t.name(), s));
            }
            tool
        })
        .collect()
}

fn build_assistant_message(
    response_text: &str,
    tool_calls: &[ToolCall],
    reasoning_content: Option<String>,
) -> ChatMessage {
    let mut parts = Vec::new();
    if !response_text.is_empty() {
        parts.push(ContentPart::from_text(response_text));
    }
    for call in tool_calls {
        parts.push(ContentPart::ToolCall(call.clone()));
    }
    let mut msg = ChatMessage::assistant(MessageContent::from_parts(parts));
    if let Some(r) = reasoning_content {
        msg = msg.with_reasoning_content(Some(r));
    }
    msg
}

async fn execute_single_tool(
    call: &ToolCall,
    tools: &[Arc<dyn AgentTool>],
    spec: &RoleSpec,
    messages: &mut Messages,
) -> ToolCallResult {
    let tool_name = &call.fn_name;
    let args = &call.fn_arguments;

    let result = match tools.iter().find(|t| t.name() == tool_name) {
        Some(tool) if spec.rejects(tool.as_ref()) => Err(tool_err(format!(
            "{tool_name} is not available in this pass"
        ))),
        Some(tool) => tool.execute(args.clone()).await,
        None => Err(tool_err(format!("Unknown tool: {tool_name}"))),
    };

    match result {
        Ok(val) => {
            messages.push(ChatMessage::from(ToolResponse::from_tool_call(
                call,
                val.to_string(),
            )));
            ToolCallResult::ok(tool_name.clone(), args.clone(), val.clone())
        }
        Err(e) => {
            messages.push(ChatMessage::from(ToolResponse::from_tool_call(
                call,
                format!("Error: {e}"),
            )));
            ToolCallResult::err(tool_name.clone(), args.clone(), format!("{e}"))
        }
    }
}
