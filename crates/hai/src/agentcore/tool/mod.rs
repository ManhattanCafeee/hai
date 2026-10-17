use async_trait::async_trait;
use serde_json::{Value, json};
use thiserror::Error;

use crate::error::AppError;

pub(crate) mod schema;

/// Tool failure text as the model sees it; both `From` impls flatten their error into this
/// message.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct ToolError(String);

impl From<AppError> for ToolError {
    fn from(e: AppError) -> Self {
        Self(e.to_string())
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Option<Value>;
    async fn execute(&self, args: Value) -> Result<Value, ToolError>;

    fn ends_turn(&self) -> bool {
        false
    }

    /// Whether this call produces something the user receives (a message, an image, or a voice
    /// note).
    fn speaks(&self) -> bool {
        false
    }

    /// Whether this call makes something (spending a model call) without putting it in front of the
    /// user — a wrap-up must not start one.
    fn produces(&self) -> bool {
        false
    }
}

/// JSON Schema for a tool's args as schemars emits it; the root is normalized for the wire by
/// `agentcore/tool/schema.rs`.
pub fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("valid schema")
}

pub fn tool_ok() -> Result<Value, ToolError> {
    Ok(json!({ "ok": true }))
}

pub fn tool_data(data: Value) -> Result<Value, ToolError> {
    Ok(json!({ "ok": true, "data": data }))
}

pub fn tool_err(msg: impl Into<String>) -> ToolError {
    ToolError(msg.into())
}
