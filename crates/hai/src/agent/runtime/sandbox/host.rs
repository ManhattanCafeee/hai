use std::path::Path;

use super::{ShellOutput, deadline_from, output_within};
use crate::{agent::runtime::workspace::Workspace, agentcore::tool::ToolError};

/// The host's own commands, for a process running without a sandbox.
#[derive(Debug)]
pub struct HostSandbox {
    workspace: Workspace,
    timeout_secs: u64,
}

impl HostSandbox {
    pub fn new(workspace: Workspace, timeout_secs: u64) -> Self {
        Self {
            workspace,
            timeout_secs,
        }
    }

    pub fn workspace_dir(&self) -> &Path {
        self.workspace.dir()
    }

    pub async fn run(
        &self,
        command: &str,
        workdir: Option<&Path>,
        timeout_secs: Option<u64>,
    ) -> Result<ShellOutput, ToolError> {
        let deadline = deadline_from(timeout_secs, self.timeout_secs);
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(workdir.unwrap_or(self.workspace.dir()));
        let output = output_within(cmd, deadline).await?;
        Ok(output.into())
    }
}
