use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use super::{ChatEntry, ChatId, EntryAudience, StepNumber, TurnNumber};

/// Output of one LLM call; a runtime record, the audit trail lives in the `event` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutput {
    pub turn: TurnNumber,
    pub step: StepNumber,
    pub reasoning: Option<String>,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub chat_id: ChatId,
    pub payload: AgentEventPayload,
}

impl AgentEvent {
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|e| {
            tracing::warn!(
                error = %e, kind = %self.payload.kind(),
                "failed to serialize agent event"
            );
            serde_json::Value::Null
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnEndReason {
    /// Completed normally, including turns that had nothing to say.
    Success {
        output: StepOutput,
        tool_calls: usize,
        elapsed_ms: u64,
        context_tokens: u32,
        has_spoken: bool,
    },
    /// Interrupted by a new event: the Turn ends early with its work kept, and a new Turn
    /// resumes from there.
    Steered {
        output: StepOutput,
        tool_calls: usize,
        elapsed_ms: u64,
        context_tokens: u32,
        has_spoken: bool,
    },
    /// Failure with zero state side effects; the only path that redoes the Turn from scratch.
    Failed { error: String },
}

/// What let a turn through: the two attention paths the scheduler decides, the retry a failed turn
/// left behind (which bypasses attention), and the re-dispatch a steered turn starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr, EnumIter)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StartedBy {
    Window,
    Heat,
    Retry,
    Steered,
}

/// The batch a turn read, counted by the audience the attention policy reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchComposition {
    pub addressed: u32,
    pub bystanders: u32,
}

impl BatchComposition {
    pub fn of(entries: &[ChatEntry]) -> Self {
        entries.iter().fold(
            Self {
                addressed: 0,
                bystanders: 0,
            },
            |mut counts, entry| {
                match entry.audience {
                    EntryAudience::Agent => counts.addressed += 1,
                    EntryAudience::Bystanders => counts.bystanders += 1,
                }
                counts
            },
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum AgentEventPayload {
    /// `started_by` and `composition` are absent on records written before those fields existed.
    TurnStarted {
        turn: TurnNumber,
        reason: WakeReasonKind,
        msg_count: usize,
        full_prompt: String,
        started_by: Option<StartedBy>,
        composition: Option<BatchComposition>,
    },

    ToolCall {
        turn: TurnNumber,
        step: StepNumber,
        tool: String,
        args: String,
    },
    ToolCallResult {
        turn: TurnNumber,
        step: StepNumber,
        tool: String,
        summary: String,
        success: bool,
    },

    StepCompleted {
        turn: TurnNumber,
        step: StepNumber,
        output: StepOutput,
    },

    TurnEnded {
        turn: TurnNumber,
        reason: TurnEndReason,
    },

    ModelRetry {
        turn: TurnNumber,
        reason: ModelRetryReason,
    },

    WrapUpStarted,

    WrapUpCompleted {
        #[serde(default)]
        step_count: usize,
        /// The pass's last Step's prompt tokens, not the wrapped chapter's.
        #[serde(default)]
        context_tokens: u32,
        #[serde(default)]
        summary: String,
    },

    WrapUpFailed {
        error: String,
    },
}

impl AgentEventPayload {
    /// The kind this payload is stored as. Adding a variant fails to compile until it is named here,
    /// which is what keeps the two spellings of one vocabulary from drifting apart.
    pub fn kind(&self) -> EventKind {
        match self {
            Self::TurnStarted { .. } => EventKind::TurnStarted,
            Self::ToolCall { .. } => EventKind::ToolCall,
            Self::ToolCallResult { .. } => EventKind::ToolCallResult,
            Self::StepCompleted { .. } => EventKind::StepCompleted,
            Self::TurnEnded { .. } => EventKind::TurnEnded,
            Self::ModelRetry { .. } => EventKind::ModelRetry,
            Self::WrapUpStarted => EventKind::WrapUpStarted,
            Self::WrapUpCompleted { .. } => EventKind::WrapUpCompleted,
            Self::WrapUpFailed { .. } => EventKind::WrapUpFailed,
        }
    }
}

/// One event kind as the `event` table stores it: the serde tag of [`AgentEventPayload`], spelled
/// once, so a filter names a kind instead of carrying a bare string.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    Serialize,
    Deserialize,
    IntoStaticStr,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum EventKind {
    TurnStarted,
    ToolCall,
    ToolCallResult,
    StepCompleted,
    TurnEnded,
    ModelRetry,
    WrapUpStarted,
    WrapUpCompleted,
    WrapUpFailed,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Why a Turn started, without the wake's payload: what a `TurnStarted` record stores and what
/// `hai log` shows. A stored record may spell a variant as its own name or in `snake_case`; the
/// write path emits the `snake_case` one, and both decode.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    Serialize,
    Deserialize,
    IntoStaticStr,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WakeReasonKind {
    #[serde(alias = "Observe")]
    Observe,
    #[serde(alias = "Direct")]
    Direct,
    #[serde(alias = "Mention")]
    Mention,
    #[serde(alias = "Scheduled")]
    Scheduled,
    #[serde(alias = "Command")]
    Command,
}

impl WakeReasonKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// How much of the agent's attention the reason asks for: a change aimed at the agent outranks
    /// a room it was only listening to, and an instruction it has to carry out outranks both.
    pub fn strength(self) -> u8 {
        match self {
            Self::Command | Self::Scheduled => 2,
            Self::Direct | Self::Mention => 1,
            Self::Observe => 0,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    Display,
    IntoStaticStr,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ModelRetryReason {
    ResponseWithText,
    TimeoutRetry,
    /// HTTP 200 whose body is not parseable JSON (transient proxy or gateway output).
    MalformedResponse,
}
