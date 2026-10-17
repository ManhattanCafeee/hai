use serde::Deserialize;

use crate::domain::{
    model::Event,
    vo::{AgentEvent, AgentEventPayload, EventKind, StartedBy, TurnEndReason, TurnNumber},
};

/// How many lines of a long field are shown before it folds.
pub(super) const PREVIEW_LINES: usize = 6;

/// What the timeline groups by and the structured filters match on, read from the payload here so
/// nothing downstream has to parse it back out of a rendered line.
#[derive(Debug, Clone)]
pub(super) struct EventFacts {
    pub turn: Option<TurnNumber>,
    pub tool: Option<String>,
    pub ok: Option<bool>,
    pub outcome: Option<TurnOutcome>,
}

/// How a finished turn ended, as a timeline row summarises it without expanding.
#[derive(Debug, Clone, Copy)]
pub(super) struct TurnOutcome {
    /// Absent when the end record carries no measurement: a failed turn reports neither.
    pub tools: Option<usize>,
    pub elapsed_ms: Option<u64>,
    pub failed: bool,
    pub replied: bool,
}

/// One titled block of an event's detail. A block long enough to bury the rest folds to
/// [`PREVIEW_LINES`].
pub(super) struct Section {
    pub title: &'static str,
    pub lines: Vec<String>,
    pub long: bool,
}

impl Section {
    fn new(title: &'static str, lines: Vec<String>) -> Self {
        Self {
            long: lines.len() > PREVIEW_LINES,
            title,
            lines,
        }
    }

    /// A titled body: the text split into lines, a long one foldable. Every line keeps the two-space
    /// indent the field lines have, so all of a section's text lines up under its title.
    fn body(title: &'static str, text: &str) -> Self {
        Self::new(
            title,
            text.lines().map(|line| format!("  {line}")).collect(),
        )
    }

    /// The body as shown: every line, or the first `PREVIEW_LINES` while folded.
    pub fn shown(&self, folded: bool) -> Vec<&str> {
        let count = if folded && self.long {
            PREVIEW_LINES
        } else {
            self.lines.len()
        };
        self.lines.iter().take(count).map(String::as_str).collect()
    }

    /// The marker that stands in for the lines a folded body hides.
    pub fn fold_note(&self, folded: bool) -> Option<String> {
        (folded && self.long).then(|| {
            format!(
                "… +{} more lines (Enter to expand)",
                self.lines.len() - PREVIEW_LINES
            )
        })
    }
}

/// Every section's title and shown body, one blank line between them: what the non-TUI path prints.
pub(super) fn render_sections(sections: &[Section], folded: bool) -> String {
    let blocks: Vec<String> = sections
        .iter()
        .map(|section| {
            let mut block = format!("{}:", section.title);
            for line in section.shown(folded) {
                block.push('\n');
                block.push_str(line);
            }
            if let Some(note) = section.fold_note(folded) {
                block.push('\n');
                block.push_str(&note);
            }
            block
        })
        .collect();
    blocks.join("\n\n")
}

pub(super) struct EventDisplay {
    pub tag: &'static str,
    pub one_liner: String,
    pub sections: Vec<Section>,
    pub chat_id: i64,
    /// The kind this payload is stored as; `None` for a payload that failed to deserialize.
    pub kind: Option<EventKind>,
    pub color: (u8, u8, u8),
    pub facts: EventFacts,
}

impl EventDisplay {
    /// Placeholder for a payload that failed to deserialize: the row stays visible with an empty
    /// detail pane.
    pub fn unparsed() -> Self {
        Self {
            tag: "?",
            one_liner: String::from("<unparsed>"),
            sections: vec![Section::body("Fields", "<unparsed event payload>")],
            chat_id: 0,
            kind: None,
            color: (156, 163, 175),
            facts: EventFacts {
                turn: None,
                tool: None,
                ok: None,
                outcome: None,
            },
        }
    }

    pub fn from_event(event: &Event) -> Option<Self> {
        let ae = AgentEvent::deserialize(&event.payload).ok()?;
        let payload = &ae.payload;
        let tag = tag_for_kind(payload);
        let color = color_rgb(payload);

        let one_liner = match payload {
            AgentEventPayload::TurnEnded {
                turn,
                reason: TurnEndReason::Failed { error },
                ..
            } => {
                let preview: String = error.chars().take(60).collect();
                format!("FAIL   {turn}  {preview}")
            }
            AgentEventPayload::ModelRetry { turn, reason, .. } => {
                format!("RETRY  {turn}  {reason}")
            }
            AgentEventPayload::TurnStarted {
                turn,
                reason,
                msg_count,
                started_by,
                ..
            } => {
                let by = started_by_label(started_by).unwrap_or("—");
                format!("TURN   {turn}  {reason}  msgs:{msg_count}  by:{by}")
            }
            AgentEventPayload::ToolCall {
                turn,
                step,
                tool,
                args,
                ..
            } => {
                let preview: String = args.chars().take(40).collect();
                format!("TOOL   {turn}.{step}  {tool}({preview})")
            }
            AgentEventPayload::ToolCallResult {
                turn,
                step,
                tool,
                summary,
                success,
                ..
            } => {
                let preview: String = summary.chars().take(40).collect();
                format!(
                    "TOOL   {turn}.{step}  {tool}  {}  {}",
                    preview,
                    if *success { "✓" } else { "✗" }
                )
            }
            AgentEventPayload::TurnEnded {
                reason:
                    TurnEndReason::Success {
                        output,
                        tool_calls,
                        elapsed_ms,
                        context_tokens,
                        ..
                    },
                ..
            } => {
                format!(
                    "DONE   {}  {tool_calls}tools  {:.1}s  ctx {}t",
                    output.turn,
                    *elapsed_ms as f64 / 1000.0,
                    context_tokens
                )
            }
            AgentEventPayload::TurnEnded {
                turn,
                reason: TurnEndReason::Steered { .. },
                ..
            } => {
                format!("STEER  {turn}")
            }
            AgentEventPayload::StepCompleted { turn, step, .. } => {
                format!("STEP   {turn}.{step}")
            }
            AgentEventPayload::WrapUpStarted => "WRAP  start".to_string(),
            AgentEventPayload::WrapUpCompleted {
                step_count,
                context_tokens,
                ..
            } => {
                let ctx = if *context_tokens > 0 {
                    format!("  ctx {}t", context_tokens)
                } else {
                    String::new()
                };
                format!("WRAP  {step_count} steps{ctx}")
            }
            AgentEventPayload::WrapUpFailed { error, .. } => {
                let preview: String = error.chars().take(60).collect();
                format!("FAIL   wrap-up  {preview}")
            }
        };

        let sections = sections(payload);

        Some(Self {
            tag,
            one_liner,
            sections,
            chat_id: ae.chat_id.0,
            kind: Some(payload.kind()),
            color,
            facts: facts(payload),
        })
    }
}

/// What `hai log` calls the ground a turn started on; `None` when the record predates the field.
fn started_by_label(started_by: &Option<StartedBy>) -> Option<&'static str> {
    started_by.as_ref().map(|path| <&str>::from(*path))
}

/// The turn, tool and outcome a payload carries: what the timeline groups by and what the structured
/// filters match on.
fn facts(payload: &AgentEventPayload) -> EventFacts {
    let turn_only = |turn: TurnNumber| EventFacts {
        turn: Some(turn),
        tool: None,
        ok: None,
        outcome: None,
    };
    match payload {
        AgentEventPayload::TurnStarted { turn, .. }
        | AgentEventPayload::StepCompleted { turn, .. }
        | AgentEventPayload::ModelRetry { turn, .. } => turn_only(*turn),
        AgentEventPayload::ToolCall { turn, tool, .. } => EventFacts {
            turn: Some(*turn),
            tool: Some(tool.clone()),
            ok: None,
            outcome: None,
        },
        AgentEventPayload::ToolCallResult {
            turn,
            tool,
            success,
            ..
        } => EventFacts {
            turn: Some(*turn),
            tool: Some(tool.clone()),
            ok: Some(*success),
            outcome: None,
        },
        AgentEventPayload::TurnEnded { turn, reason } => EventFacts {
            turn: Some(*turn),
            tool: None,
            ok: None,
            outcome: Some(outcome_of(reason)),
        },
        // A wrap-up carries no turn number of its own: it stands alone on the timeline.
        AgentEventPayload::WrapUpStarted
        | AgentEventPayload::WrapUpCompleted { .. }
        | AgentEventPayload::WrapUpFailed { .. } => EventFacts {
            turn: None,
            tool: None,
            ok: None,
            outcome: None,
        },
    }
}

/// What a finished turn reports: the summary a collapsed timeline row shows.
fn outcome_of(reason: &TurnEndReason) -> TurnOutcome {
    match reason {
        TurnEndReason::Success {
            tool_calls,
            elapsed_ms,
            has_spoken,
            ..
        }
        | TurnEndReason::Steered {
            tool_calls,
            elapsed_ms,
            has_spoken,
            ..
        } => TurnOutcome {
            tools: Some(*tool_calls),
            elapsed_ms: Some(*elapsed_ms),
            failed: false,
            replied: *has_spoken,
        },
        TurnEndReason::Failed { .. } => TurnOutcome {
            tools: None,
            elapsed_ms: None,
            failed: true,
            replied: false,
        },
    }
}

fn tag_for_kind(payload: &AgentEventPayload) -> &'static str {
    match payload {
        AgentEventPayload::TurnStarted { .. } => "TURN",
        AgentEventPayload::ToolCall { .. } | AgentEventPayload::ToolCallResult { .. } => "TOOL",
        AgentEventPayload::StepCompleted { .. } => "STEP",
        AgentEventPayload::TurnEnded {
            reason: TurnEndReason::Failed { .. },
            ..
        } => "FAIL",
        AgentEventPayload::TurnEnded {
            reason: TurnEndReason::Steered { .. },
            ..
        } => "STEER",
        AgentEventPayload::TurnEnded { .. } => "DONE",
        AgentEventPayload::ModelRetry { .. } => "RETRY",
        AgentEventPayload::WrapUpFailed { .. } => "FAIL",
        AgentEventPayload::WrapUpStarted | AgentEventPayload::WrapUpCompleted { .. } => "WRAP",
    }
}

/// The sections of one event's detail, in reading order: the fields first, then the bodies that
/// would otherwise bury them.
#[derive(Default)]
struct Sections {
    fields: Vec<String>,
    bodies: Vec<Section>,
}

impl Sections {
    fn field(&mut self, label: &str, value: impl std::fmt::Display) {
        self.fields.push(format!("  {label}:  {value}"));
    }

    fn block(&mut self, title: &'static str, body: &str) {
        self.bodies.push(Section::body(title, body));
    }

    fn build(self) -> Vec<Section> {
        let mut sections = Vec::with_capacity(self.bodies.len() + 1);
        if !self.fields.is_empty() {
            sections.push(Section::new("Fields", self.fields));
        }
        sections.extend(self.bodies);
        sections
    }
}

/// The sections of one event's detail. The caller has already read the row and deserialized the
/// payload, so this takes the payload rather than the row it would have to read again.
fn sections(ae: &AgentEventPayload) -> Vec<Section> {
    let mut d = Sections::default();

    match ae {
        AgentEventPayload::TurnStarted {
            turn,
            reason,
            msg_count,
            full_prompt,
            started_by,
            composition,
        } => {
            d.field("Turn", turn);
            d.field("Reason", reason);
            d.field("Messages", msg_count);
            if let Some(path) = started_by_label(started_by) {
                d.field("Started by", path);
            }
            if let Some(counts) = composition {
                d.field(
                    "Batch",
                    format!(
                        "{} addressed / {} bystanders",
                        counts.addressed, counts.bystanders
                    ),
                );
            }
            if !full_prompt.is_empty() {
                d.block("Prompt", &display_json_value(full_prompt));
            }
        }
        AgentEventPayload::TurnEnded {
            turn,
            reason: TurnEndReason::Failed { error },
            ..
        } => {
            d.field("Turn", turn);
            d.field("Outcome", "failed");
            d.block("Error", error);
        }
        AgentEventPayload::ModelRetry { turn, reason, .. } => {
            d.field("Turn", turn);
            d.field("Reason", reason);
        }
        AgentEventPayload::StepCompleted { turn, step, output } => {
            d.field("Step", format!("{}.{}", turn, step));
            if let Some(rs) = &output.reasoning {
                d.block("Thinking", rs);
            }
            d.block("Response", &output.response);
        }
        AgentEventPayload::ToolCall { tool, args, .. } => {
            d.field("Tool", tool);
            d.block("Arguments", &display_json_value(args));
        }
        AgentEventPayload::ToolCallResult {
            tool,
            summary,
            success,
            ..
        } => {
            d.field("Tool", tool);
            d.block("Result", &display_json_value(summary));
            d.field("Status", if *success { "✓" } else { "✗" });
        }
        AgentEventPayload::TurnEnded {
            turn,
            reason:
                TurnEndReason::Success {
                    output,
                    tool_calls,
                    elapsed_ms,
                    context_tokens,
                    ..
                }
                | TurnEndReason::Steered {
                    output,
                    tool_calls,
                    elapsed_ms,
                    context_tokens,
                    ..
                },
            ..
        } => {
            d.field("Turn", turn);
            d.field("Step", format!("{}.{}", output.turn, output.step));
            if let Some(rs) = &output.reasoning {
                d.block("Thinking", rs);
            }
            if !output.response.is_empty() {
                d.block("Response", &output.response);
            }
            d.field("Tool Calls", tool_calls);
            d.field("Duration", format!("{:.1}s", *elapsed_ms as f64 / 1000.0));
            d.field("Context Tokens", format!("{}", context_tokens));
        }
        AgentEventPayload::WrapUpStarted => {
            d.field("Action", "start");
        }
        AgentEventPayload::WrapUpCompleted {
            step_count,
            context_tokens,
            summary,
        } => {
            d.field("Steps", step_count);
            if *context_tokens > 0 {
                d.field("Context Tokens", context_tokens);
            }
            if !summary.is_empty() {
                d.block("Summary", summary);
            }
        }
        AgentEventPayload::WrapUpFailed { error } => {
            d.block("Error", error);
        }
    }

    d.build()
}

fn display_json_value(s: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
        Err(_) => s.to_string(),
    }
}

pub(super) fn fmt_time(ts: jiff::Timestamp) -> String {
    let zoned = ts.to_zoned(jiff::tz::TimeZone::system());
    if zoned.date() == jiff::Zoned::now().date() {
        zoned.strftime("%H:%M:%S").to_string()
    } else {
        zoned.strftime("%m-%d %H:%M").to_string()
    }
}

fn color_rgb(payload: &AgentEventPayload) -> (u8, u8, u8) {
    match payload {
        AgentEventPayload::TurnEnded {
            reason: TurnEndReason::Failed { .. },
            ..
        }
        | AgentEventPayload::WrapUpFailed { .. } => (239, 68, 68),
        AgentEventPayload::ModelRetry { .. } => (250, 176, 5),
        AgentEventPayload::StepCompleted { .. } => (56, 189, 248),
        AgentEventPayload::TurnStarted { .. } => (255, 255, 255),
        AgentEventPayload::ToolCall { .. } => (59, 130, 246),
        AgentEventPayload::ToolCallResult { success, .. } if !success => (239, 68, 68),
        AgentEventPayload::ToolCallResult { .. } => (34, 197, 94),
        AgentEventPayload::TurnEnded { .. } => (255, 255, 255),
        AgentEventPayload::WrapUpStarted | AgentEventPayload::WrapUpCompleted { .. } => {
            (168, 85, 247)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(payload: serde_json::Value) -> Event {
        Event {
            seq: 1,
            payload,
            created_at: jiff::Timestamp::from_second(0).expect("a valid second"),
        }
    }

    /// The grouping facts come from the payload, beside the rendered text rather than parsed back out
    /// of it — and the payload is read from the borrowed value, which is what makes a 20 KB prompt
    /// cheap to display.
    #[test]
    fn facts_come_from_the_payload() {
        let ended = event(json!({
            "chat_id": 7,
            "payload": {
                "event": "turn-ended",
                "turn": 5,
                "reason": { "failed": { "error": "boom" } }
            }
        }));
        let display = EventDisplay::from_event(&ended).expect("a parsed event");
        assert_eq!(display.chat_id, 7);
        assert_eq!(display.facts.turn, Some(TurnNumber(5)));
        let outcome = display.facts.outcome.expect("a failed turn reports one");
        assert!(outcome.failed);
        assert!(!outcome.replied);

        let started = event(json!({
            "chat_id": 7,
            "payload": {
                "event": "turn-started",
                "turn": 5,
                "reason": "direct",
                "msg_count": 3,
                "full_prompt": "the prompt",
                "started_by": "window",
                "composition": { "addressed": 1, "bystanders": 2 }
            }
        }));
        let display = EventDisplay::from_event(&started).expect("a parsed event");
        assert_eq!(display.facts.turn, Some(TurnNumber(5)));
        assert!(
            display.facts.outcome.is_none(),
            "a start reports no outcome"
        );

        let wrap_up = event(json!({ "chat_id": 7, "payload": { "event": "wrap-up-started" } }));
        let display = EventDisplay::from_event(&wrap_up).expect("a parsed event");
        assert!(display.facts.turn.is_none(), "a wrap-up carries no turn");
    }

    /// A long field becomes one foldable section: the preview while folded, everything when not —
    /// and the fields stay in front of it.
    #[test]
    fn a_long_field_folds_to_its_preview() {
        let full_prompt: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let started = event(json!({
            "chat_id": 7,
            "payload": {
                "event": "turn-started",
                "turn": 5,
                "reason": "direct",
                "msg_count": 1,
                "full_prompt": full_prompt,
            }
        }));
        let display = EventDisplay::from_event(&started).expect("a parsed event");

        assert_eq!(display.kind, Some(EventKind::TurnStarted));
        let fields = display.sections.first().expect("the fields come first");
        assert_eq!(fields.title, "Fields");
        assert!(!fields.long, "a handful of fields never folds");

        let prompt = display
            .sections
            .iter()
            .find(|section| section.title == "Prompt")
            .expect("the prompt is a section of its own");
        assert!(prompt.long);
        assert_eq!(prompt.shown(false).len(), 40);
        assert_eq!(prompt.shown(true).len(), PREVIEW_LINES);
        assert_eq!(
            prompt.fold_note(true).as_deref(),
            Some("… +34 more lines (Enter to expand)")
        );
        assert!(prompt.fold_note(false).is_none());
    }
}
