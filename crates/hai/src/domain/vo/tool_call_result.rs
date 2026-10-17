use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub tool_name: String,
    pub success: bool,
    pub arguments: Value,
    pub result: Value,
}

impl ToolCallResult {
    pub fn ok(tool_name: impl Into<String>, args: Value, result: Value) -> Self {
        Self {
            tool_name: tool_name.into(),
            success: true,
            arguments: args,
            result,
        }
    }

    pub fn err(tool_name: impl Into<String>, args: Value, error: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            success: false,
            arguments: args,
            result: Value::String(error.into()),
        }
    }
}
