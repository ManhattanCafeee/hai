use std::path::{Path, PathBuf};

use tokio::{process::Command, sync::OnceCell};

use super::{CONTAINER_WORKSPACE_ROOT, ShellOutput, deadline_from, output_within};
use crate::{
    agent::runtime::workspace::Workspace,
    agentcore::tool::{ToolError, tool_err},
    config::schema::{ContainerRuntime, SandboxConfig},
};

/// The agent's commands, viewed through a disposable container.
#[derive(Debug)]
pub struct ContainerSandbox {
    runtime: ContainerRuntime,
    image: String,
    mounts: Vec<PathBuf>,
    workspace: Workspace,
    timeout_secs: u64,
    /// The container, created on the first command. Only its creation is serialized here; commands
    /// run concurrently like the processes they are.
    container: OnceCell<ContainerGuard>,
}

impl ContainerSandbox {
    pub fn new(cfg: &SandboxConfig, skill_roots: Vec<PathBuf>, workspace: Workspace) -> Self {
        let mut mounts = skill_roots;
        mounts.extend(cfg.mounts.iter().cloned());
        Self {
            runtime: cfg.runtime,
            image: cfg.image.clone(),
            mounts,
            workspace,
            timeout_secs: cfg.timeout_secs,
            container: OnceCell::new(),
        }
    }

    /// The workspace on the host, which the container binds at its own root.
    pub fn workspace_dir(&self) -> &Path {
        self.workspace.dir()
    }

    /// Whether this environment exposes `path`: its workspace, or one of the directories bound into it
    /// (the skill roots and `[sandbox] mounts`). Both sides are canonicalized, so a symlink cannot walk
    /// out of a mount, and a path that does not resolve is an error rather than a refusal.
    pub fn ensure_readable(&self, path: &Path) -> Result<(), ToolError> {
        let resolved = path
            .canonicalize()
            .map_err(|e| tool_err(format!("{}: {e}", path.display())))?;
        let inside = |root: &Path| {
            root.canonicalize()
                .is_ok_and(|root| resolved.starts_with(root))
        };
        if inside(self.workspace.dir()) || self.mounts.iter().any(|mount| inside(mount)) {
            return Ok(());
        }
        Err(tool_err(format!(
            "{} is outside the sandbox: it exposes the workspace and the directories mounted into it",
            path.display()
        )))
    }

    /// The container this sandbox acts in, created on the first command; the cell serializes that
    /// creation only, so commands run concurrently like the processes they are.
    async fn container(&self) -> Result<String, ToolError> {
        let container = self
            .container
            .get_or_try_init(|| {
                ContainerGuard::create(
                    self.runtime.as_str(),
                    &self.image,
                    &self.mounts,
                    self.workspace.dir(),
                )
            })
            .await?;
        Ok(container.id.clone())
    }

    pub async fn run(
        &self,
        command: &str,
        workdir: Option<&Path>,
        timeout_secs: Option<u64>,
    ) -> Result<ShellOutput, ToolError> {
        let deadline = deadline_from(timeout_secs, self.timeout_secs);
        let id = self.container().await?;
        let mut cmd = Command::new(self.runtime.as_str());
        cmd.args(["exec", "-w"])
            .arg(workdir.unwrap_or(Path::new(CONTAINER_WORKSPACE_ROOT)))
            .arg(&id)
            .arg("bash")
            .arg("-c")
            .arg(command);
        let output = output_within(cmd, deadline).await?;
        Ok(output.into())
    }
}

#[derive(Debug)]
pub(crate) struct ContainerGuard {
    runtime: String,
    id: String,
}

impl ContainerGuard {
    /// Every mount but the workspace is bound read-only at its own path; the workspace is writable at
    /// its fixed path, which is also where a command runs by default.
    pub async fn create(
        runtime: &str,
        image: &str,
        mounts: &[PathBuf],
        workspace_dir: &Path,
    ) -> Result<Self, ToolError> {
        let mut cmd = Command::new(runtime);
        cmd.arg("create").arg("--rm");
        for mount in mounts {
            let mount = mount.display().to_string();
            cmd.arg("-v").arg(format!("{mount}:{mount}:ro"));
        }
        cmd.arg("-v").arg(format!(
            "{}:{CONTAINER_WORKSPACE_ROOT}",
            workspace_dir.display()
        ));
        cmd.arg(image).args(["sh", "-c", "sleep infinity"]);
        let output = cmd
            .output()
            .await
            .map_err(|e| tool_err(format!("Failed to turn {runtime}: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(tool_err(format!("Failed to create container: {stderr}")));
        }

        let id = String::from_utf8(output.stdout)
            .map_err(|e| tool_err(format!("Non-UTF-8 container ID: {e}")))?
            .trim()
            .to_string();
        if id.is_empty() {
            return Err(tool_err("Empty container ID"));
        }

        // The container exists from here on, so the guard owns it before anything else can fail:
        // a created-but-never-started container would otherwise stay forever, since only a stopped
        // one is removed by `--rm`.
        let guard = Self {
            id,
            runtime: runtime.to_string(),
        };

        let started = Command::new(runtime)
            .args(["start", &guard.id])
            .output()
            .await
            .map_err(|e| tool_err(format!("Failed to start container: {e}")))?;
        if !started.status.success() {
            let stderr = String::from_utf8_lossy(&started.stderr);
            return Err(tool_err(format!("Failed to start container: {stderr}")));
        }

        Ok(guard)
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.runtime)
            .args(["rm", "-f", &self.id])
            .output();
    }
}
