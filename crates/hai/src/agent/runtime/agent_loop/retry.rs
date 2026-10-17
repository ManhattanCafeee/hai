//! The Step loop's LLM retry policy: which failures are retried, how long to wait, and what the
//! `ModelRetry` event reports.

use genai::chat::{ChatRequest, Tool};
use tokio::time::Duration;

use super::LoopTurn;
use crate::{
    agent::runtime::types::Messages,
    agentcore::tool::{ToolError, tool_err},
    domain::vo::{AgentEventPayload, ModelRetryReason},
    util::text::truncate_chars,
};

/// Retries after a transient LLM failure: network errors, 429/5xx, and an
/// unparsable response body.
const MAX_LLM_RETRIES: u32 = 2;

pub(super) async fn llm_call_with_retry(
    turn: &LoopTurn,
    genai_tools: &[Tool],
    messages: &Messages,
) -> Result<genai::chat::ChatResponse, ToolError> {
    for attempt in 0..=MAX_LLM_RETRIES {
        let error = match turn
            .client
            .exec_chat(
                &turn.model,
                ChatRequest::new(messages.to_vec())
                    .with_system(&turn.spec.system_prompt)
                    .with_tools(genai_tools.to_vec()),
                Some(&turn.spec.options),
            )
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        let message = llm_failure_message(&error);
        let Some(delay) = retry_backoff(&error, attempt + 1).filter(|_| attempt < MAX_LLM_RETRIES)
        else {
            return Err(tool_err(message));
        };
        tracing::warn!(attempt = attempt + 1, ?delay, error = %message, "LLM request failed; retrying");
        turn.event_bus.emit(
            turn.chat_id,
            AgentEventPayload::ModelRetry {
                turn: turn.turn_number,
                reason: retry_reason(&error),
            },
        );
        tokio::time::sleep(delay).await;
    }
    Err(tool_err("LLM call failed after retries"))
}
fn retry_backoff(e: &genai::Error, attempt: u32) -> Option<Duration> {
    let webc_error = match e {
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => webc_error,
        // A 200 whose body cannot be parsed is usually a proxy/gateway artifact, so it
        // gets the same treatment as a retryable status.
        genai::Error::ChatResponseGeneration { .. } => return Some(backoff(attempt)),
        _ => return None,
    };
    match webc_error {
        genai::webc::Error::Reqwest(error) => {
            (error.is_timeout() || error.is_connect()).then(|| backoff(attempt))
        }
        genai::webc::Error::ResponseFailedStatus {
            status, headers, ..
        } => retryable_status(status.as_u16())
            .then(|| retry_after(headers.as_ref()).unwrap_or_else(|| backoff(attempt))),
        _ => None,
    }
}

fn retry_reason(e: &genai::Error) -> ModelRetryReason {
    match e {
        genai::Error::ChatResponseGeneration { .. } => ModelRetryReason::MalformedResponse,
        _ => ModelRetryReason::TimeoutRetry,
    }
}

/// `ChatResponseGeneration`'s `Display` embeds the whole request payload, and this
/// text ends up in the `TurnEnded` event — keep the cause and a body excerpt only.
fn llm_failure_message(error: &genai::Error) -> String {
    match error {
        genai::Error::ChatResponseGeneration {
            cause,
            response_body,
            ..
        } => {
            let body = response_body.as_str().unwrap_or_default();
            format!(
                "LLM response unusable: {cause}; body: {}",
                truncate_chars(body, 300)
            )
        }
        other => format!("LLM request failed: {other}"),
    }
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * u64::from(attempt))
}

/// `Retry-After` in seconds only, capped at 10s: a turn must not be held up long by
/// the remote.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds.min(10)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_status_covers_throttling_and_gateways_only() {
        for status in [429, 500, 502, 503, 504] {
            assert!(retryable_status(status), "{status}");
        }
        for status in [400, 401, 403, 404, 422, 501] {
            assert!(!retryable_status(status), "{status}");
        }
    }

    #[test]
    fn retry_after_reads_seconds_and_is_capped() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after(&headers), None);

        headers.insert(reqwest::header::RETRY_AFTER, "2".parse().expect("value"));
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(2)));

        headers.insert(reqwest::header::RETRY_AFTER, "600".parse().expect("value"));
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(10)));

        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().expect("value"),
        );
        assert_eq!(
            retry_after(&headers),
            None,
            "the HTTP-date form is unsupported"
        );
    }
}
