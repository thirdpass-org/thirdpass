use anyhow::{format_err, Context, Result};
use serde::Deserialize;
use std::convert::TryFrom;

#[derive(Debug, Default)]
pub(super) struct CodexRetryMetrics {
    pub(super) failed_attempt_duration_ms: u64,
    pub(super) retry_wait_ms: u64,
    pub(super) retry_reasons: Vec<String>,
}

pub(super) fn codex_run_metrics(
    stdout_jsonl: &str,
    duration_ms: u64,
    started_at_unix_ms: Option<u64>,
    finished_at_unix_ms: Option<u64>,
) -> thirdpass_core::schema::AgentRunMetrics {
    let token_info = parse_codex_token_info(stdout_jsonl);
    thirdpass_core::schema::AgentRunMetrics {
        duration_ms,
        attempts: 1,
        started_at_unix_ms,
        finished_at_unix_ms,
        total_token_usage: token_info
            .as_ref()
            .and_then(|info| info.total_token_usage.clone()),
        last_token_usage: token_info
            .as_ref()
            .and_then(|info| info.last_token_usage.clone()),
        model_context_window: token_info.and_then(|info| info.model_context_window),
        failed_attempt_duration_ms: 0,
        retry_wait_ms: 0,
        retry_reasons: Vec::new(),
    }
}

#[derive(Debug, Deserialize)]
struct CodexJsonEvent {
    #[serde(rename = "type")]
    event_type: String,
    payload: Option<CodexJsonPayload>,
    usage: Option<CodexJsonUsage>,
}

#[derive(Debug, Deserialize)]
struct CodexJsonPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    info: Option<CodexTokenInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexTokenInfo {
    total_token_usage: Option<thirdpass_core::schema::AgentTokenUsage>,
    last_token_usage: Option<thirdpass_core::schema::AgentTokenUsage>,
    model_context_window: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct CodexJsonUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
}

impl CodexJsonUsage {
    fn into_agent_token_usage(self) -> thirdpass_core::schema::AgentTokenUsage {
        let total_tokens = self.input_tokens.saturating_add(self.output_tokens);
        thirdpass_core::schema::AgentTokenUsage {
            input_tokens: self.input_tokens,
            cached_input_tokens: self.cached_input_tokens,
            output_tokens: self.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens,
            total_tokens,
        }
    }
}

fn parse_codex_token_info(stdout_jsonl: &str) -> Option<CodexTokenInfo> {
    let mut last: Option<CodexTokenInfo> = None;
    for line in stdout_jsonl
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(event) = serde_json::from_str::<CodexJsonEvent>(line) else {
            continue;
        };
        if event.event_type == "turn.completed" {
            if let Some(usage) = event.usage {
                let model_context_window = last.as_ref().and_then(|info| info.model_context_window);
                let token_usage = usage.into_agent_token_usage();
                last = Some(CodexTokenInfo {
                    total_token_usage: Some(token_usage.clone()),
                    last_token_usage: Some(token_usage),
                    model_context_window,
                });
            }
            continue;
        }
        if event.event_type != "event_msg" {
            continue;
        }
        let Some(payload) = event.payload else {
            continue;
        };
        if payload.payload_type.as_deref() != Some("token_count") {
            continue;
        }
        if let Some(info) = payload.info {
            last = Some(info);
        }
    }
    last
}

pub(super) fn unix_time_ms() -> Result<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Failed to read system time.")?;
    u64::try_from(duration.as_millis()).map_err(|_| format_err!("System time overflowed u64."))
}

pub(super) fn duration_millis_u64(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{codex_run_metrics, parse_codex_token_info};

    #[test]
    fn parse_codex_token_info_reads_last_non_null_token_count() {
        let stdout = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2,"reasoning_output_tokens":1,"total_tokens":12},"last_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2,"reasoning_output_tokens":1,"total_tokens":12},"model_context_window":1000}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":8,"output_tokens":3,"reasoning_output_tokens":2,"total_tokens":23},"last_token_usage":{"input_tokens":5,"cached_input_tokens":2,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":6},"model_context_window":2000}}}"#;

        let info = parse_codex_token_info(stdout).expect("expected token info");

        assert_eq!(info.model_context_window, Some(2000));
        assert_eq!(
            info.total_token_usage
                .expect("expected total usage")
                .total_tokens,
            23
        );
        assert_eq!(
            info.last_token_usage
                .expect("expected last usage")
                .input_tokens,
            5
        );
    }

    #[test]
    fn parse_codex_token_info_reads_turn_completed_usage() {
        let stdout = r#"{"type":"thread.started","thread_id":"019f13d6-10a1-7680-ae98-a4f4cb4e5788"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
{"type":"turn.completed","usage":{"input_tokens":12021,"cached_input_tokens":9600,"output_tokens":22,"reasoning_output_tokens":15}}"#;

        let info = parse_codex_token_info(stdout).expect("expected token info");
        let total = info.total_token_usage.expect("expected total usage");
        let last = info.last_token_usage.expect("expected last usage");

        assert_eq!(total.input_tokens, 12021);
        assert_eq!(total.cached_input_tokens, 9600);
        assert_eq!(total.output_tokens, 22);
        assert_eq!(total.reasoning_output_tokens, 15);
        assert_eq!(total.total_tokens, 12043);
        assert_eq!(last, total);
        assert_eq!(info.model_context_window, None);
    }

    #[test]
    fn codex_run_metrics_combines_timing_and_token_usage() {
        let stdout = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":8,"output_tokens":3,"reasoning_output_tokens":2,"total_tokens":23},"last_token_usage":{"input_tokens":5,"cached_input_tokens":2,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":6},"model_context_window":2000}}}"#;

        let metrics = codex_run_metrics(stdout, 123, Some(1000), Some(1123));

        assert_eq!(metrics.duration_ms, 123);
        assert_eq!(metrics.attempts, 1);
        assert_eq!(metrics.started_at_unix_ms, Some(1000));
        assert_eq!(metrics.finished_at_unix_ms, Some(1123));
        assert_eq!(metrics.model_context_window, Some(2000));
        assert_eq!(metrics.failed_attempt_duration_ms, 0);
        assert_eq!(metrics.retry_wait_ms, 0);
        assert!(metrics.retry_reasons.is_empty());
        assert_eq!(
            metrics
                .total_token_usage
                .expect("expected total usage")
                .cached_input_tokens,
            8
        );
    }
}
