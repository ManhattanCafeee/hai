use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool},
    service::RunningService,
    transport::child_process::TokioChildProcess,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{ChildStderr, Command},
};

use super::tool::{AgentTool, ToolError, tool_err};
use crate::error::{AppError, Result, bail};

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// One configured MCP server, from `[mcp.<name>]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct McpConfig {
    pub r#type: McpTransport,
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
}

/// MCP transport; `streamable-http` is not implemented and fails at connect time.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum McpTransport {
    #[default]
    Stdio,
    StreamableHttp,
}

pub struct McpManager {
    tools: Vec<Arc<dyn AgentTool>>,
}

pub struct McpServerHandle {
    name: String,
    service: RunningService<RoleClient, ()>,
}

impl McpManager {
    pub async fn from_config(servers_cfg: &HashMap<String, McpConfig>) -> Result<Self> {
        let mut servers: Vec<Arc<McpServerHandle>> = Vec::new();
        for (name, mcp_cfg) in servers_cfg {
            match McpServerHandle::connect(name, mcp_cfg).await? {
                Some(handle) => servers.push(Arc::new(handle)),
                None => {
                    tracing::warn!(server = %name, "MCP server skipped: command is empty");
                }
            }
        }
        // The map's iteration order is arbitrary; tool order and the "keep the first" collision
        // winner must not depend on it.
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self {
            tools: resolve_tools(&servers).await,
        })
    }

    pub fn tools(&self) -> &[Arc<dyn AgentTool>] {
        &self.tools
    }
}

/// Resolve the tool surface once at connect time: tools a server adds or removes later only
/// become visible after a restart.
async fn resolve_tools(servers: &[Arc<McpServerHandle>]) -> Vec<Arc<dyn AgentTool>> {
    let mut tools: Vec<Arc<dyn AgentTool>> = Vec::new();
    for server in servers {
        let server_tools = match server.list_tools().await {
            Ok(server_tools) => server_tools,
            Err(e) => {
                tracing::error!(server = %server.name, "Failed to list tools: {e}");
                continue;
            }
        };
        for t in server_tools {
            let tool = McpAgentTool {
                name: registration_name(&server.name, &t.name),
                server: Arc::clone(server),
                tool: t,
            };
            if tools.iter().any(|existing| existing.name() == tool.name) {
                tracing::warn!(
                    tool = %tool.name,
                    server = %server.name,
                    "MCP tool name collision; keeping the first"
                );
                continue;
            }
            tools.push(Arc::new(tool));
        }
    }
    tools
}

impl McpServerHandle {
    /// `Ok(None)` means the entry has nothing to connect to (empty `command`).
    async fn connect(name: &str, cfg: &McpConfig) -> Result<Option<Self>> {
        if cfg.r#type != McpTransport::Stdio {
            bail!(AppError::message(format!(
                "MCP server `{name}`: transport `{type}` not implemented (only stdio)",
                type = cfg.r#type
            )));
        }
        if cfg.command.trim().is_empty() {
            return Ok(None);
        }
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args);
        if let Some(env_map) = &cfg.env {
            cmd.envs(env_map);
        }
        cmd.kill_on_drop(true);

        let (process, stderr_opt) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| AppError::message(format!("Failed to spawn MCP process `{name}`: {e}")))?;

        if let Some(stderr) = stderr_opt {
            let server_name = name.to_string();
            tokio::spawn(drain_stderr(stderr, server_name));
        }

        let service = ().serve(process).await.map_err(|e| {
            AppError::message(format!("Failed to serve MCP transport `{name}`: {e}"))
        })?;

        tracing::info!(server = %name, "MCP server connected");

        Ok(Some(Self {
            name: name.to_string(),
            service,
        }))
    }

    async fn list_tools(&self) -> Result<Vec<Tool>> {
        tokio::time::timeout(LIST_TIMEOUT, self.service.peer().list_all_tools())
            .await
            .map_err(|e| AppError::message(format!("MCP tools/list timed out: {e}")))?
            .map_err(|e| AppError::message(format!("Failed to list MCP tools: {e}")))
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Map<String, Value>,
    ) -> std::result::Result<Value, ToolError> {
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(args);
        let result = tokio::time::timeout(CALL_TIMEOUT, self.service.peer().call_tool(params))
            .await
            .map_err(|_| {
                tool_err(format!(
                    "MCP tool call timed out after {}s",
                    CALL_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| tool_err(format!("MCP tool call failed: {e}")))?;

        classify_result(result)
    }
}

struct McpAgentTool {
    /// Registered as `mcp__<server>_<tool>`, which keeps MCP tools out of the built-in name
    /// space.
    name: String,
    server: Arc<McpServerHandle>,
    tool: Tool,
}

/// Copies a server's own diagnostics into the log until it closes the pipe. `read_line` answers
/// `Ok(0)` at EOF without awaiting, so the loop has to end there rather than on any `Ok`.
async fn drain_stderr(stderr: ChildStderr, server: String) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    while matches!(reader.read_line(&mut line).await, Ok(read) if read > 0) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            tracing::debug!(target: "hai::mcp", server = %server, "{trimmed}");
        }
        line.clear();
    }
}

/// `isError` wins over any payload (a failure signal must not be short-circuited by structured
/// content); otherwise `structuredContent`, then text blocks, then null. Text that is not JSON is
/// passed through as a string.
fn classify_result(result: CallToolResult) -> std::result::Result<Value, ToolError> {
    let text: String = result
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<&str>>()
        .join("\n");

    if result.is_error.unwrap_or(false) {
        let detail = match (text.is_empty(), result.structured_content) {
            (false, _) => text,
            (true, Some(structured)) => structured.to_string(),
            (true, None) => "MCP tool returned an error".to_owned(),
        };
        return Err(tool_err(detail));
    }

    if let Some(structured) = result.structured_content {
        return Ok(structured);
    }

    if text.is_empty() {
        return Ok(Value::Null);
    }

    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn registration_name(server: &str, tool: &str) -> String {
    format!("mcp__{}_{}", sanitize(server), sanitize(tool))
}

fn sanitize(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[async_trait]
impl AgentTool for McpAgentTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        self.tool.description.as_deref().unwrap_or("")
    }

    fn schema(&self) -> Option<Value> {
        Some(Value::Object((*self.tool.input_schema).clone()))
    }

    async fn execute(&self, args: Value) -> std::result::Result<Value, ToolError> {
        let json_map = args.as_object().ok_or_else(|| {
            tool_err(format!(
                "MCP tool {} expected object args, got {:?}",
                self.name(),
                args
            ))
        })?;
        self.server
            .call_tool(&self.tool.name, json_map.clone())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_command_is_skipped() {
        let cfg = McpConfig {
            command: "   ".to_owned(),
            ..McpConfig::default()
        };
        match McpServerHandle::connect("tavily", &cfg).await {
            Ok(None) => {}
            Ok(Some(_)) => panic!("empty command must not connect"),
            Err(e) => panic!("empty command must not error: {e}"),
        }
    }

    fn call_result(
        content: Vec<ContentBlock>,
        structured: Option<serde_json::Value>,
        is_error: Option<bool>,
    ) -> CallToolResult {
        let mut result = CallToolResult::default();
        result.content = content;
        result.structured_content = structured;
        result.is_error = is_error;
        result
    }

    #[test]
    fn error_results_win_over_structured_content() {
        let structured_error = call_result(
            Vec::new(),
            Some(serde_json::json!({"ok": false})),
            Some(true),
        );
        assert_eq!(
            classify_result(structured_error)
                .expect_err("error")
                .to_string(),
            "{\"ok\":false}"
        );

        let text_error = call_result(vec![ContentBlock::text("boom")], None, Some(true));
        assert_eq!(
            classify_result(text_error).expect_err("error").to_string(),
            "boom"
        );
    }

    #[test]
    fn successful_results_prefer_structured_content() {
        let structured = call_result(Vec::new(), Some(serde_json::json!({"ok": true})), None);
        assert_eq!(
            classify_result(structured).expect("structured"),
            serde_json::json!({"ok": true})
        );

        let plain = call_result(vec![ContentBlock::text("hi")], None, None);
        assert_eq!(
            classify_result(plain).expect("text"),
            serde_json::json!("hi")
        );

        let empty = call_result(Vec::new(), None, None);
        assert_eq!(
            classify_result(empty).expect("empty"),
            serde_json::Value::Null
        );
    }

    #[test]
    fn registration_name_isolates_and_sanitizes() {
        assert_eq!(
            registration_name("Exa", "web_fetch_exa"),
            "mcp__exa_web_fetch_exa"
        );
        assert_eq!(
            registration_name("my-server.v2", "CamelCase Tool"),
            "mcp__my_server_v2_camelcase_tool"
        );
    }

    #[tokio::test]
    async fn non_stdio_transport_is_rejected() {
        let cfg = McpConfig {
            r#type: McpTransport::StreamableHttp,
            ..McpConfig::default()
        };
        let e = McpServerHandle::connect("remote", &cfg)
            .await
            .map(|_| ())
            .expect_err("unsupported transport must fail");
        assert!(
            e.to_string().contains("not implemented (only stdio)"),
            "{e}"
        );
    }
    /// A server that stops talking closes its pipe; the drain has to end there instead of spinning on
    /// a read that keeps answering `Ok(0)`.
    #[tokio::test]
    async fn draining_stderr_ends_when_the_server_closes_the_pipe() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'a line\n' >&2")
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("a shell to write stderr with");
        let stderr = child.stderr.take().expect("a piped stderr");

        tokio::time::timeout(
            Duration::from_secs(10),
            drain_stderr(stderr, "test".to_owned()),
        )
        .await
        .expect("the drain ends at EOF");

        let _: std::process::ExitStatus = child.wait().await.expect("the shell to end");
    }
}
