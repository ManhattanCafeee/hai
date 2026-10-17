pub mod addressing;
pub mod knowledge;
pub mod memory;
pub mod read;
pub mod recall;
pub mod schedule;
pub mod send;
pub mod shell;
pub mod skip;
pub mod topic;
pub mod util;
pub mod wait;
pub mod web;

use std::sync::{Arc, OnceLock};

use crate::agentcore::tool::AgentTool;

/// Providers (e.g. Anthropic) require tool names to match `^[a-zA-Z0-9_-]{1,64}$`.
const MAX_TOOL_NAME_LEN: usize = 64;

pub(crate) fn sanitize(
    tools: Vec<Arc<dyn AgentTool>>,
    disabled: &[String],
) -> Vec<Arc<dyn AgentTool>> {
    report(&tools, disabled);
    tools
        .into_iter()
        .filter(|tool| {
            let name = tool.name();
            !disabled.iter().any(|entry| entry == name) && valid_name(name)
        })
        .collect()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TOOL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn report(tools: &[Arc<dyn AgentTool>], disabled: &[String]) {
    static REPORTED: OnceLock<()> = OnceLock::new();
    REPORTED.get_or_init(|| {
        for tool in tools {
            let name = tool.name();
            if disabled.iter().any(|entry| entry == name) {
                tracing::warn!(tool = name, "tool disabled by config");
            } else if !valid_name(name) {
                tracing::warn!(tool = name, "tool name rejected by provider naming rules");
            }
        }
        for name in disabled {
            if !tools.iter().any(|tool| tool.name() == name) {
                tracing::warn!(
                    tool = name,
                    "tool name in [tools] disabled matches no mounted tool"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use schemars::schema_for;
    use serde_json::json;
    use strum::IntoEnumIterator;

    use super::{
        memory::{MemoryArgs, MemoryOp},
        schedule::{ScheduleArgs, ScheduleOp},
        topic::{TopicArgs, TopicOp},
    };
    use crate::{
        agent::test_support::StubTool, agentcore::tool::AgentTool, test_support::assert_names_agree,
    };

    #[test]
    fn sanitize_drops_disabled_and_bad_names() {
        let tools: Vec<Arc<dyn AgentTool>> = vec![
            StubTool::new("keep_me").into_dyn(),
            StubTool::new("disabled_tool").into_dyn(),
            StubTool::new("bad.name").into_dyn(),
            StubTool::new("x".repeat(65)).into_dyn(),
        ];

        let kept: Vec<String> = super::sanitize(tools, &["disabled_tool".to_string()])
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();

        assert_eq!(kept, ["keep_me"]);
    }

    /// OpenAI-compatible APIs require tool parameters to be an object schema; a `oneOf` or enum
    /// root is either rejected or degraded into an empty object.
    #[test]
    fn op_tool_schemas_are_objects() {
        for (schema, expected) in [
            (
                json!(schema_for!(TopicArgs)),
                ["open", "note", "revise", "close", "link", "drop"].as_slice(),
            ),
            (
                json!(schema_for!(MemoryArgs)),
                ["record", "revise", "invalidate", "forget", "pin"].as_slice(),
            ),
            (
                json!(schema_for!(ScheduleArgs)),
                ["create", "list", "cancel"].as_slice(),
            ),
        ] {
            assert_eq!(schema["type"], "object", "{schema}");
            assert_eq!(op_values(&schema), expected, "{schema}");
        }
    }

    /// The `op` value set (schemars puts the enum under `$defs` and references it from the
    /// property as a `$ref`).
    fn op_values(schema: &serde_json::Value) -> Vec<String> {
        let op = &schema["properties"]["op"];
        let node = op["$ref"]
            .as_str()
            .and_then(|r| schema.pointer(r.strip_prefix('#')?))
            .unwrap_or(op);
        node["enum"]
            .as_array()
            .expect("op enum")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn op_enum_names_agree_between_serde_and_strum() {
        assert_names_agree(TopicOp::iter(), |op| op.into());
        assert_names_agree(MemoryOp::iter(), |op| op.into());
        assert_names_agree(ScheduleOp::iter(), |op| op.into());
    }

    #[test]
    fn op_payloads_deserialize() {
        let topic: TopicArgs =
            serde_json::from_value(json!({"op": "open", "title": "Title", "summary": "Summary"}))
                .expect("valid payload");
        assert_eq!(topic.op, TopicOp::Open);
        let memory: MemoryArgs = serde_json::from_value(
            json!({"op": "forget", "id": "3f0c1a9e-1b2c-4d5e-8f90-112233445566"}),
        )
        .expect("valid payload");
        assert_eq!(memory.op, MemoryOp::Forget);
        assert!(serde_json::from_value::<ScheduleArgs>(json!({"op": "nope"})).is_err());
        assert!(serde_json::from_value::<MemoryArgs>(json!({})).is_err());
    }
}
