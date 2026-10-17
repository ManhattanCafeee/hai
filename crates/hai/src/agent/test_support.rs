//! Test helpers the agent layer's modules share.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    agent::runtime::workspace::Workspace,
    agentcore::tool::{AgentTool, ToolError},
    domain::vo::ChatId,
    test_support::TempDir,
};

/// A tool whose metadata a test sets explicitly; every field left unset keeps the trait default
/// (silent, turn-continuing, non-producing) and the call returns `null`.
pub(crate) struct StubTool {
    name: String,
    ends_turn: bool,
    speaks: bool,
    produces: bool,
}

impl StubTool {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ends_turn: false,
            speaks: false,
            produces: false,
        }
    }

    pub(crate) fn ends_turn(mut self, value: bool) -> Self {
        self.ends_turn = value;
        self
    }

    pub(crate) fn speaks(mut self, value: bool) -> Self {
        self.speaks = value;
        self
    }

    pub(crate) fn produces(mut self, value: bool) -> Self {
        self.produces = value;
        self
    }

    pub(crate) fn into_dyn(self) -> Arc<dyn AgentTool> {
        Arc::new(self)
    }
}

#[async_trait]
impl AgentTool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        ""
    }

    fn schema(&self) -> Option<Value> {
        None
    }

    async fn execute(&self, _args: Value) -> Result<Value, ToolError> {
        Ok(Value::Null)
    }

    fn ends_turn(&self) -> bool {
        self.ends_turn
    }

    fn speaks(&self) -> bool {
        self.speaks
    }

    fn produces(&self) -> bool {
        self.produces
    }
}

/// A workspace belonging to a chat that does not exist, plus the guard that removes its directory: a
/// test needs a real directory only when it writes into one.
pub(crate) async fn created_workspace() -> (Workspace, TempDir) {
    let workspace = Workspace::create(ChatId(uuid::Uuid::new_v4().as_u128() as i64))
        .await
        .expect("workspace directory");
    let dir = TempDir::adopt(workspace.dir().to_path_buf());
    (workspace, dir)
}
