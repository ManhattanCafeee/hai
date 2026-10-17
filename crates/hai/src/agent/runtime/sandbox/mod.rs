//! The environment the agent's tools act on: a container while one runs, the host when none does.

use std::{
    path::{Path, PathBuf},
    process::Output,
};

use tokio::process::Command;

use super::workspace::Workspace;
use crate::{
    agentcore::tool::{ToolError, tool_err},
    config::schema::SandboxConfig,
};

pub(crate) mod container;
pub(crate) mod host;

pub use container::ContainerSandbox;
pub use host::HostSandbox;

/// The workspace's mount point inside a container: the path a containerised command spells the chat's
/// workspace with, while the host spells the same directory `host_workspace()`.
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";
const MAX_TIMEOUT_SECS: u64 = 3600;

pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl From<std::process::Output> for ShellOutput {
    /// A process that died without a code reports `-1`: the tools read `exit_code` as a number.
    fn from(output: std::process::Output) -> Self {
        Self {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        }
    }
}

/// Owns where a command runs and which paths the tools may open, and nothing else.
#[derive(Debug)]
pub enum Sandbox {
    Container(ContainerSandbox),
    Host(HostSandbox),
}

impl Sandbox {
    /// A disposable container, with the skill roots and the configured mounts bound read-only at their
    /// own paths and the workspace writable at its fixed mount point.
    pub fn container(cfg: &SandboxConfig, skill_roots: Vec<PathBuf>, workspace: Workspace) -> Self {
        Self::Container(ContainerSandbox::new(cfg, skill_roots, workspace))
    }

    /// The host itself: commands run in this process's own shell.
    pub fn host(workspace: Workspace, timeout_secs: u64) -> Self {
        Self::Host(HostSandbox::new(workspace, timeout_secs))
    }

    /// The directory a command runs in when it names none, as this environment spells it.
    pub fn command_root(&self) -> &Path {
        match self {
            Self::Container(_) => Path::new(CONTAINER_WORKSPACE_ROOT),
            Self::Host(host) => host.workspace_dir(),
        }
    }

    /// The chat's workspace on the host's filesystem: what `read` opens for `workspace://<rel>` and for
    /// a relative path.
    pub fn host_workspace(&self) -> &Path {
        match self {
            Self::Container(container) => container.workspace_dir(),
            Self::Host(host) => host.workspace_dir(),
        }
    }

    /// Refuses a path this environment does not expose. A container exposes its workspace and the
    /// directories bound into it, so a read cannot walk out through a mount or a symlink; the host
    /// exposes everything, because that is what turning the sandbox off means.
    pub fn ensure_readable(&self, path: &Path) -> Result<(), ToolError> {
        match self {
            Self::Container(container) => container.ensure_readable(path),
            Self::Host(_) => Ok(()),
        }
    }

    pub async fn run(
        &self,
        command: &str,
        workdir: Option<&Path>,
        timeout_secs: Option<u64>,
    ) -> Result<ShellOutput, ToolError> {
        match self {
            Self::Container(container) => container.run(command, workdir, timeout_secs).await,
            Self::Host(host) => host.run(command, workdir, timeout_secs).await,
        }
    }
}

/// Requested value → actual deadline: `Some(0)` means unlimited, `Some(n)` is clamped to the cap,
/// `None` falls back to the configured default.
fn deadline_from(requested: Option<u64>, default: u64) -> Option<u64> {
    match requested {
        Some(0) => None,
        Some(secs) => Some(secs.min(MAX_TIMEOUT_SECS)),
        None => Some(default),
    }
}

/// One command under `deadline`, its output as the process produced it.
async fn output_within(cmd: Command, deadline: Option<u64>) -> Result<Output, ToolError> {
    let mut cmd = cmd;
    cmd.kill_on_drop(true);
    let output = match deadline {
        Some(secs) => tokio::time::timeout(std::time::Duration::from_secs(secs), cmd.output())
            .await
            .map_err(|_| tool_err(format!("Command timed out after {secs}s")))?,
        None => cmd.output().await,
    }
    .map_err(|e| tool_err(format!("Failed to execute command: {e}")))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_maps_request_onto_default_and_cap() {
        assert_eq!(deadline_from(Some(0), 30), None);
        assert_eq!(deadline_from(None, 30), Some(30));
        assert_eq!(deadline_from(Some(7200), 30), Some(MAX_TIMEOUT_SECS));
        assert_eq!(deadline_from(Some(30), 60), Some(30));
    }

    /// The container's own tree stands in for the process's: what a command writes into the workspace
    /// is the file the host opens, and the container's private paths are the one place the two differ.
    /// Gated because it needs a container runtime.
    #[tokio::test]
    async fn the_container_shares_the_workspace_with_the_host() {
        if std::env::var("HAI_SANDBOX_SMOKE").is_err() {
            println!("skipping the_container_shares_the_workspace_with_the_host");
            return;
        }
        let (workspace, dir) = crate::agent::test_support::created_workspace().await;
        let sandbox = Sandbox::container(&SandboxConfig::default(), Vec::new(), workspace);

        let write = sandbox
            .run("echo hi > a.txt", None, None)
            .await
            .expect("write");
        assert_eq!(write.exit_code, 0, "stderr: {}", write.stderr);
        let read = sandbox.run("cat a.txt", None, None).await.expect("read");
        assert_eq!(read.stdout, "hi\n");

        let shared = tokio::fs::read_to_string(dir.path().join("a.txt"))
            .await
            .expect("the host sees the container's file");
        assert_eq!(shared, "hi\n");
        assert!(sandbox.ensure_readable(&dir.path().join("a.txt")).is_ok());
        assert!(sandbox.ensure_readable(Path::new("/etc/hostname")).is_err());

        let internal = sandbox
            .run("cat /etc/hostname", None, None)
            .await
            .expect("cat");
        assert_eq!(internal.exit_code, 0);
    }

    #[tokio::test]
    async fn a_timed_out_command_leaves_no_process_behind() {
        let (workspace, dir) = crate::agent::test_support::created_workspace().await;
        let sandbox = Sandbox::host(workspace, 1);
        let pid_file = dir.path().join("pid");
        let command = format!("echo $$ > {}; exec sleep 30", pid_file.display());

        let error = sandbox
            .run(&command, None, None)
            .await
            .err()
            .expect("times out");
        assert!(error.to_string().contains("timed out"), "{error}");

        let pid = tokio::fs::read_to_string(&pid_file)
            .await
            .expect("pid file")
            .trim()
            .to_owned();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the timed-out child is gone"
        );
    }
}
