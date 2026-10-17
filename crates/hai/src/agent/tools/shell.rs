use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::addressing::{self, Directory};
use crate::{
    agent::{
        runtime::{sandbox::Sandbox, turn::ToolContext},
        tools::util::deserialize_option_lenient_u64,
    },
    agentcore::tool::{AgentTool, ToolError, schema_of, tool_data},
};

const HEAD_BYTES: usize = 24 * 1024;
const TAIL_BYTES: usize = 8 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellArgs {
    pub command: String,
    /// Relative to the workspace, or workspace://<path>; the workspace is the default
    #[serde(default, deserialize_with = "addressing::deserialize_workdir")]
    #[schemars(with = "Option<String>")]
    pub workdir: Option<Directory>,
    /// Timeout in seconds; 0 means no limit (for long tasks), a larger value is clamped to the
    /// sandbox's cap
    #[serde(default, deserialize_with = "deserialize_option_lenient_u64")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug)]
pub struct Bash {
    pub description: String,
    pub sandbox: Arc<Sandbox>,
}

#[async_trait]
impl AgentTool for Bash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Option<Value> {
        Some(schema_of::<ShellArgs>())
    }

    async fn execute(&self, args: Value) -> Result<Value, ToolError> {
        let typed: ShellArgs = serde_json::from_value(args)?;
        // A relative workdir resolves against the environment's own root, an absolute one stands: a
        // command's `cd` target must exist where the command runs.
        let workdir = typed
            .workdir
            .as_ref()
            .map(|path| path.command_path(self.sandbox.command_root()));

        let output = self
            .sandbox
            .run(&typed.command, workdir.as_deref(), typed.timeout_secs)
            .await?;

        let (stdout, stdout_elided_bytes) = cap_output(&output.stdout);
        let (stderr, stderr_elided_bytes) = cap_output(&output.stderr);

        tool_data(json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": output.exit_code,
            "stdout_elided_bytes": stdout_elided_bytes,
            "stderr_elided_bytes": stderr_elided_bytes,
        }))
    }
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    let description = "Run a shell command and return stdout/stderr/exit_code. When the output is too long only the head and tail are kept.".to_owned();
    vec![Arc::new(Bash {
        description,
        sandbox: ctx.sandbox.clone(),
    })]
}

fn cap_output(text: &str) -> (String, usize) {
    if text.len() <= HEAD_BYTES + TAIL_BYTES {
        return (text.to_owned(), 0);
    }
    let head_end = floor_boundary(text, HEAD_BYTES);
    let tail_start = ceil_boundary(text, text.len() - TAIL_BYTES);
    let elided = tail_start - head_end;
    (
        format!(
            "{head}\n[output truncated: {elided} bytes elided; redirect to a file and read it with offset/limit]\n{tail}",
            head = &text[..head_end],
            tail = &text[tail_start..],
        ),
        elided,
    )
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceil_boundary(text: &str, mut at: usize) -> usize {
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_output_keeps_short_text() {
        assert_eq!(cap_output("done"), ("done".to_owned(), 0));
    }

    #[test]
    fn cap_output_keeps_head_and_tail() {
        // Three-byte characters so the cut lands mid-character and the boundary helpers matter.
        let text = format!(
            "{}{}{}",
            "→".repeat(HEAD_BYTES),
            "…".repeat(1000),
            "★".repeat(TAIL_BYTES)
        );
        let (capped, elided) = cap_output(&text);
        assert!(capped.starts_with("→"));
        assert!(capped.ends_with("★"));
        assert!(capped.contains("bytes elided"));
        assert!(capped.is_char_boundary(capped.len()));
        assert!(capped.len() < text.len());

        let head_len = capped.find("\n[output truncated").expect("marker");
        let tail_len = capped.len() - (capped.rfind("]\n").expect("marker") + 2);
        assert_eq!(elided, text.len() - head_len - tail_len);
    }
}
