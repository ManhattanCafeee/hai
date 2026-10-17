//! Hosted MCP over HTTP (Streamable HTTP, without an `initialize` handshake): text results come
//! from `tools/call`. Endpoint, tool names, auth header and credential all come from the table in
//! `provider/table.rs`, and search and fetch share one transport instance.

use std::fmt;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{
    error::{AppError, Result},
    util::text::truncate_chars,
};

#[derive(Clone)]
pub(super) struct Transport {
    http: reqwest::Client,
    endpoint: &'static str,
    server: &'static str,
    auth_header: &'static str,
    auth_scheme: Option<&'static str>,
    api_key: Option<String>,
}

impl Transport {
    pub fn new(
        http: reqwest::Client,
        endpoint: &'static str,
        server: &'static str,
        auth_header: &'static str,
        auth_scheme: Option<&'static str>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            http,
            endpoint,
            server,
            auth_header,
            auth_scheme,
            api_key,
        }
    }

    pub async fn call(&self, tool: &str, arguments: Value) -> Result<String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let mut request = self
            .http
            .post(self.endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.header(self.auth_header, auth_value(self.auth_scheme, key));
        }
        let resp = request.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(self.failed(format!("HTTP {status}: {}", truncate_chars(&text, 300))));
        }
        let payload: Value = serde_json::from_str(payload_json(&text))
            .map_err(|e| self.failed(format!("invalid response: {e}")))?;
        text_of(&payload).map_err(|error| self.failed(error.to_string()))
    }

    fn failed(&self, message: String) -> crate::error::AppError {
        AppError::message(format!("{}: {message}", self.server))
    }
}

fn auth_value(scheme: Option<&'static str>, key: &str) -> String {
    match scheme {
        Some(scheme) => format!("{scheme} {key}"),
        None => key.to_owned(),
    }
}

/// Callers only see these folded into `AppError`.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReplyError {
    /// Either a server error (JSON-RPC `error`) or a tool reporting failure (`isError`); the string
    /// is the reason in both cases.
    Reported(String),
    MissingResult,
    Empty,
}

impl fmt::Display for ReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplyError::Reported(message) => f.write_str(message),
            ReplyError::MissingResult => f.write_str("missing result"),
            ReplyError::Empty => f.write_str("empty result"),
        }
    }
}

fn text_of(payload: &Value) -> std::result::Result<String, ReplyError> {
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string());
        return Err(ReplyError::Reported(message));
    }
    let result = payload.get("result").ok_or(ReplyError::MissingResult)?;
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(ReplyError::Reported(truncate_chars(&content, 300)));
    }
    if content.trim().is_empty() {
        return Err(ReplyError::Empty);
    }
    Ok(content)
}

/// Takes the first `data:` line of an SSE response; a plain JSON body passes through unchanged.
fn payload_json(body: &str) -> &str {
    if body.trim_start().starts_with('{') {
        return body;
    }
    body.lines()
        .find_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .unwrap_or(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_value_prefixes_the_declared_scheme() {
        assert_eq!(auth_value(Some("Bearer"), "secret"), "Bearer secret");
        assert_eq!(auth_value(None, "secret"), "secret");
    }

    #[test]
    fn payload_json_handles_sse_and_plain_body() {
        assert_eq!(payload_json("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(
            payload_json("event: message\ndata: {\"a\":1}\n\n"),
            "{\"a\":1}"
        );
    }

    #[test]
    fn text_of_joins_parts_and_reports_failures() {
        assert_eq!(
            text_of(&json!({ "result": { "content": [
                { "type": "text", "text": "first part" }, { "type": "text", "text": "second part" }
            ] } }))
            .as_deref(),
            Ok("first part\nsecond part")
        );
        assert_eq!(
            text_of(&json!({ "error": { "message": "boom" } })).unwrap_err(),
            ReplyError::Reported("boom".to_owned())
        );
        assert_eq!(text_of(&json!({})).unwrap_err(), ReplyError::MissingResult);
        assert_eq!(
            text_of(&json!({ "result": { "isError": true, "content": [{ "text": "denied" }] } }))
                .unwrap_err(),
            ReplyError::Reported("denied".to_owned())
        );
        assert_eq!(
            text_of(&json!({ "result": { "content": [{ "type": "text", "text": "  " }] } }))
                .unwrap_err(),
            ReplyError::Empty
        );
    }
}
