use anyhow::{format_err, Context, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use super::{apply_allowed_environment_from, metrics, prompt, truncate_for_log, AgentKind};

const APPROVAL_POLICY: &str = "never";
const COMMAND_ENV: &str = "THIRDPASS_CODEX_COMMAND";
const ALLOWED_ENV: &[&str] = &[
    "ALL_PROXY",
    "CODEX_HOME",
    "HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "NIX_SSL_CERT_FILE",
    "NO_PROXY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "PATH",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "TEMP",
    "TERM",
    "TMP",
    "TMPDIR",
];
const SANDBOX_MODE: &str = "read-only";
const TOOL_POLICY_VERSION: &str = "codex-readonly-review/v1";
const RETRY_BACKOFF_MS: &[u64] = &[15_000, 45_000, 120_000];
const RETRY_JITTER_MS: u64 = 1_000;
const RETRYABLE_FAILURE_PATTERNS: &[(&str, &str)] = &[
    (
        "selected model is at capacity",
        "selected model is at capacity",
    ),
    ("at capacity", "selected model is at capacity"),
    ("rate limit", "rate limit"),
    ("server overloaded", "server overloaded"),
    ("overloaded", "server overloaded"),
    ("temporarily unavailable", "temporarily unavailable"),
    ("http connection failed", "http connection failed"),
    ("connection failed", "connection failed"),
    (
        "response stream disconnected",
        "response stream disconnected",
    ),
    (
        "response stream connection failed",
        "response stream connection failed",
    ),
    ("timed out", "timeout"),
    ("timeout", "timeout"),
];

pub(super) fn command() -> std::ffi::OsString {
    std::env::var_os(COMMAND_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| AgentKind::Codex.binary_name().into())
}

pub(super) fn execution_environment() -> thirdpass_core::schema::ReviewExecutionEnvironment {
    let mut settings = BTreeMap::new();
    settings.insert("approval_policy".to_string(), APPROVAL_POLICY.to_string());
    settings.insert("sandbox".to_string(), SANDBOX_MODE.to_string());
    settings.insert("ephemeral".to_string(), "true".to_string());
    settings.insert("ignore_rules".to_string(), "true".to_string());
    settings.insert("skip_git_repo_check".to_string(), "true".to_string());

    thirdpass_core::schema::ReviewExecutionEnvironment {
        tool_policy: TOOL_POLICY_VERSION.to_string(),
        settings,
    }
}

pub(super) fn run(
    workspace_path: &std::path::PathBuf,
    prompt_text: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<super::AgentRunResult> {
    let mut retry_metrics = metrics::CodexRetryMetrics::default();
    let mut attempt = 1u64;
    loop {
        let attempt_started_at = std::time::Instant::now();
        match run_once(
            workspace_path,
            prompt_text,
            agent_model,
            agent_reasoning_effort,
        ) {
            Ok(mut result) => {
                if let Some(metrics) = result.run_metrics.as_mut() {
                    metrics.attempts = attempt;
                    metrics.failed_attempt_duration_ms = retry_metrics.failed_attempt_duration_ms;
                    metrics.retry_wait_ms = retry_metrics.retry_wait_ms;
                    metrics.retry_reasons = retry_metrics.retry_reasons;
                }
                return Ok(result);
            }
            Err(error) => {
                let failed_duration_ms = metrics::duration_millis_u64(attempt_started_at.elapsed());
                let error_message = error.to_string();
                let reason = error
                    .downcast_ref::<CodexExecError>()
                    .and_then(|error| error.retryable_reason.clone())
                    .or_else(|| retryable_codex_failure_reason(&error_message));
                let Some(reason) = reason else {
                    return Err(error);
                };
                retry_metrics.failed_attempt_duration_ms = retry_metrics
                    .failed_attempt_duration_ms
                    .saturating_add(failed_duration_ms);
                retry_metrics.retry_reasons.push(reason.clone());
                let wait_ms = codex_retry_delay_ms(attempt);
                retry_metrics.retry_wait_ms = retry_metrics.retry_wait_ms.saturating_add(wait_ms);
                println!(
                    "Codex transient failure: {}; retrying in {:.1}s (next attempt {})",
                    reason,
                    wait_ms as f64 / 1000.0,
                    attempt.saturating_add(1)
                );
                std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

#[derive(Debug)]
struct CodexExecError {
    message: String,
    retryable_reason: Option<String>,
}

impl std::fmt::Display for CodexExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodexExecError {}

fn run_once(
    workspace_path: &std::path::PathBuf,
    prompt_text: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<super::AgentRunResult> {
    let command_log = command_log(agent_model, agent_reasoning_effort);
    log::debug!(
        "Launching agent: {} (cwd: {})",
        command_log,
        workspace_path.display()
    );

    let output_file = tempfile::NamedTempFile::new()?;
    let output_path = output_file.path().to_path_buf();
    let schema_file = tempfile::NamedTempFile::new()?;
    std::fs::write(schema_file.path(), prompt::OUTPUT_SCHEMA)
        .context("Failed to write codex output schema.")?;

    let codex_command = AgentKind::Codex.command();
    let mut cmd = Command::new(&codex_command);
    apply_codex_environment(&mut cmd);
    apply_codex_args(
        &mut cmd,
        agent_model,
        agent_reasoning_effort,
        &output_path,
        schema_file.path(),
    );
    cmd.arg("-");
    cmd.current_dir(workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started_at_unix_ms = metrics::unix_time_ms().ok();
    let started_at = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|err| {
        format_err!(
            "Failed to start {}: {}",
            codex_command.to_string_lossy(),
            err
        )
    })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or(format_err!("Failed to open codex stdin"))?;
    stdin.write_all(prompt_text.as_bytes())?;
    drop(stdin);

    let output = child.wait_with_output()?;
    let duration_ms = metrics::duration_millis_u64(started_at.elapsed());
    let finished_at_unix_ms = metrics::unix_time_ms().ok();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    let run_metrics = metrics::codex_run_metrics(
        stdout_raw.as_ref(),
        duration_ms,
        started_at_unix_ms,
        finished_at_unix_ms,
    );

    if !output.status.success() {
        let output_payload = std::fs::read_to_string(&output_path).unwrap_or_default();
        let retryable_reason = retryable_codex_failure_reason_from_output(
            stdout_raw.as_ref(),
            stderr.as_str(),
            output_payload.as_str(),
        );
        let diagnostic_note = codex_failure_diagnostic_note(CodexFailureDiagnostic {
            phase: "process-exit",
            status: Some(output.status.to_string()),
            command_log: command_log.as_str(),
            workspace_path,
            prompt: prompt_text,
            stdout: stdout_raw.as_ref(),
            stderr: stderr.as_str(),
            output_payload: output_payload.as_str(),
        });
        let mut details = Vec::new();
        if stderr.is_empty() {
            details.push("stderr: <empty>".to_string());
        } else {
            details.push(format!("stderr: {}", truncate_for_log(&stderr, 4000)));
        }
        let stdout_trimmed = stdout_raw.trim();
        if !stdout_trimmed.is_empty() {
            details.push(format!(
                "stdout: {}",
                truncate_for_log(stdout_trimmed, 4000)
            ));
        }
        return Err(CodexExecError {
            message: format!(
                "codex exited with status {}. {}{}",
                output.status,
                details.join(" "),
                diagnostic_note
            ),
            retryable_reason,
        }
        .into());
    }

    let output_payload = std::fs::read_to_string(&output_path).unwrap_or_default();
    let output_payload = if output_payload.trim().is_empty() {
        stdout_raw.to_string()
    } else {
        output_payload
    };

    let output = prompt::parse_agent_output(&output_payload).map_err(|err| {
        let diagnostic_note = codex_failure_diagnostic_note(CodexFailureDiagnostic {
            phase: "parse-output",
            status: None,
            command_log: command_log.as_str(),
            workspace_path,
            prompt: prompt_text,
            stdout: stdout_raw.as_ref(),
            stderr: stderr.as_str(),
            output_payload: output_payload.as_str(),
        });
        let stdout_trimmed = stdout_raw.trim();
        if stdout_trimmed.is_empty() {
            format_err!("{}{}", err, diagnostic_note)
        } else {
            format_err!(
                "{}; stdout: {}{}",
                err,
                truncate_for_log(stdout_trimmed, 4000),
                diagnostic_note
            )
        }
    })?;
    let model = recorded_codex_model(agent_model, output.model.clone());
    Ok(output.into_run_result(model, Some(run_metrics)))
}

fn retryable_codex_failure_reason(message: &str) -> Option<String> {
    let message = message.trim();
    let lower = message.to_ascii_lowercase();
    if !(lower.starts_with("codex exited")
        || lower.contains("turn.failed")
        || lower.contains("\"type\":\"error\"")
        || lower.contains("response stream")
        || lower.contains("connection failed")
        || lower.contains("timed out")
        || lower.contains("timeout"))
    {
        return None;
    }

    retryable_codex_failure_reason_in_text(&lower)
}

fn retryable_codex_failure_reason_from_output(
    stdout: &str,
    stderr: &str,
    output_payload: &str,
) -> Option<String> {
    [stdout, output_payload]
        .iter()
        .find_map(|value| retryable_codex_failure_reason(value))
        .or_else(|| retryable_codex_failure_reason_in_text(&stderr.trim().to_ascii_lowercase()))
}

fn retryable_codex_failure_reason_in_text(lower: &str) -> Option<String> {
    RETRYABLE_FAILURE_PATTERNS
        .iter()
        .find(|(pattern, _)| lower.contains(pattern))
        .map(|(_, reason)| reason.to_string())
}

fn codex_retry_delay_ms(failed_attempt: u64) -> u64 {
    base_codex_retry_delay_ms(failed_attempt).saturating_add(codex_retry_jitter_ms())
}

fn base_codex_retry_delay_ms(failed_attempt: u64) -> u64 {
    RETRY_BACKOFF_MS
        .get(failed_attempt.saturating_sub(1) as usize)
        .copied()
        .unwrap_or_else(|| *RETRY_BACKOFF_MS.last().unwrap_or(&120_000))
}

fn codex_retry_jitter_ms() -> u64 {
    if RETRY_JITTER_MS == 0 {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::from(duration.subsec_nanos()) % RETRY_JITTER_MS)
        .unwrap_or(0)
}

fn recorded_codex_model(requested_model: Option<&str>, reported_model: String) -> String {
    requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
        .unwrap_or(reported_model)
}

struct CodexFailureDiagnostic<'a> {
    phase: &'a str,
    status: Option<String>,
    command_log: &'a str,
    workspace_path: &'a std::path::Path,
    prompt: &'a str,
    stdout: &'a str,
    stderr: &'a str,
    output_payload: &'a str,
}

fn codex_failure_diagnostic_note(diagnostic: CodexFailureDiagnostic<'_>) -> String {
    match save_codex_failure_diagnostic(diagnostic) {
        Ok(path) => format!(" Diagnostic written to {}.", path.display()),
        Err(error) => format!(" Failed to write diagnostic: {}.", error),
    }
}

fn save_codex_failure_diagnostic(
    diagnostic: CodexFailureDiagnostic<'_>,
) -> Result<std::path::PathBuf> {
    let data_paths = crate::common::fs::DataPaths::new()?;
    save_codex_failure_diagnostic_in(&data_paths.root_directory, diagnostic)
}

fn save_codex_failure_diagnostic_in(
    root_directory: &std::path::Path,
    diagnostic: CodexFailureDiagnostic<'_>,
) -> Result<std::path::PathBuf> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Failed to read system time.")?
        .as_secs();
    let directory = root_directory
        .join("diagnostics")
        .join("agent-failures")
        .join(format!(
            "{}-{}",
            timestamp,
            uuid::Uuid::new_v4().to_simple()
        ));
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "Failed to create diagnostic directory {}",
            directory.display()
        )
    })?;

    let metadata = format!(
        "agent: codex\nphase: {}\nstatus: {}\ncommand: {}\nworkspace: {}\n",
        diagnostic.phase,
        diagnostic.status.as_deref().unwrap_or("success"),
        diagnostic.command_log,
        diagnostic.workspace_path.display()
    );
    write_diagnostic_file(&directory, "metadata.txt", &metadata)?;
    write_diagnostic_file(&directory, "prompt.txt", diagnostic.prompt)?;
    write_diagnostic_file(&directory, "stdout.txt", diagnostic.stdout)?;
    write_diagnostic_file(&directory, "stderr.txt", diagnostic.stderr)?;
    write_diagnostic_file(
        &directory,
        "output-last-message.txt",
        diagnostic.output_payload,
    )?;
    Ok(directory)
}

fn write_diagnostic_file(
    directory: &std::path::Path,
    file_name: &str,
    contents: &str,
) -> Result<()> {
    let path = directory.join(file_name);
    std::fs::write(&path, contents)
        .with_context(|| format!("Failed to write diagnostic file {}", path.display()))
}

fn apply_codex_args(
    cmd: &mut Command,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
    output_path: &std::path::Path,
    schema_path: &std::path::Path,
) {
    cmd.arg("--ask-for-approval");
    cmd.arg(APPROVAL_POLICY);
    cmd.arg("exec");
    apply_codex_exec_args(
        cmd,
        agent_model,
        agent_reasoning_effort,
        output_path,
        schema_path,
    );
}

fn apply_codex_environment(cmd: &mut Command) {
    cmd.env_clear();
    apply_codex_environment_from(cmd, std::env::vars_os());
}

fn apply_codex_environment_from<I, K, V>(cmd: &mut Command, variables: I)
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    apply_allowed_environment_from(cmd, variables, ALLOWED_ENV);
}

fn apply_codex_exec_args(
    cmd: &mut Command,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
    output_path: &std::path::Path,
    schema_path: &std::path::Path,
) {
    if let Some(model) = agent_model {
        cmd.arg("--model");
        cmd.arg(model);
    }
    if let Some(effort) = agent_reasoning_effort {
        cmd.arg("--config");
        cmd.arg(format!("model_reasoning_effort=\"{}\"", effort));
    }
    cmd.arg("--sandbox");
    cmd.arg(SANDBOX_MODE);
    cmd.arg("--ignore-rules");
    cmd.arg("--ephemeral");
    cmd.arg("--skip-git-repo-check");
    cmd.arg("--output-schema");
    cmd.arg(schema_path);
    cmd.arg("--json");
    cmd.arg("--output-last-message");
    cmd.arg(output_path);
}

fn command_log(agent_model: Option<&str>, agent_reasoning_effort: Option<&str>) -> String {
    let mut parts = vec![AgentKind::Codex.binary_name().to_string()];
    parts.push("--ask-for-approval".to_string());
    parts.push(APPROVAL_POLICY.to_string());
    parts.push("exec".to_string());
    if let Some(model) = agent_model {
        parts.push("--model".to_string());
        parts.push(model.to_string());
    }
    if let Some(effort) = agent_reasoning_effort {
        parts.push("--config".to_string());
        parts.push(format!("model_reasoning_effort=\"{}\"", effort));
    }
    parts.push("--sandbox".to_string());
    parts.push(SANDBOX_MODE.to_string());
    parts.push("--ignore-rules".to_string());
    parts.push("--ephemeral".to_string());
    parts.push("--skip-git-repo-check".to_string());
    parts.push("--output-schema".to_string());
    parts.push("<schema>".to_string());
    parts.push("--json".to_string());
    parts.push("--output-last-message".to_string());
    parts.push("<file>".to_string());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::super::prompt::OUTPUT_SCHEMA;
    use super::{
        apply_codex_args, apply_codex_environment_from, base_codex_retry_delay_ms, command_log,
        recorded_codex_model, retryable_codex_failure_reason,
        retryable_codex_failure_reason_from_output, save_codex_failure_diagnostic_in,
        CodexFailureDiagnostic, APPROVAL_POLICY, SANDBOX_MODE,
    };
    use crate::review::tool::agent::truncate_for_log;

    #[test]
    fn recorded_codex_model_prefers_requested_model() {
        assert_eq!(
            recorded_codex_model(Some("gpt-5.4"), "GPT-5".to_string()),
            "gpt-5.4"
        );
    }

    #[test]
    fn recorded_codex_model_uses_reported_model_without_request() {
        assert_eq!(recorded_codex_model(None, "GPT-5".to_string()), "GPT-5");
    }

    #[test]
    fn save_codex_failure_diagnostic_writes_outputs() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let diagnostic_path = save_codex_failure_diagnostic_in(
            temp.path(),
            CodexFailureDiagnostic {
                phase: "parse-output",
                status: None,
                command_log: "codex exec --sandbox read-only",
                workspace_path: std::path::Path::new("/tmp/workspace"),
                prompt: "review this file",
                stdout: "stdout payload",
                stderr: "stderr payload",
                output_payload: "last message payload",
            },
        )?;

        assert!(diagnostic_path.starts_with(temp.path()));
        assert_eq!(
            std::fs::read_to_string(diagnostic_path.join("stdout.txt"))?,
            "stdout payload"
        );
        assert_eq!(
            std::fs::read_to_string(diagnostic_path.join("stderr.txt"))?,
            "stderr payload"
        );
        assert_eq!(
            std::fs::read_to_string(diagnostic_path.join("output-last-message.txt"))?,
            "last message payload"
        );
        assert_eq!(
            std::fs::read_to_string(diagnostic_path.join("prompt.txt"))?,
            "review this file"
        );
        let metadata = std::fs::read_to_string(diagnostic_path.join("metadata.txt"))?;
        assert!(metadata.contains("phase: parse-output"));
        assert!(metadata.contains("status: success"));
        assert!(metadata.contains("workspace: /tmp/workspace"));
        Ok(())
    }

    #[test]
    fn codex_output_schema_is_valid_json() {
        let schema: serde_json::Value =
            serde_json::from_str(OUTPUT_SCHEMA).expect("schema should be valid JSON");

        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["comments"]["type"], "array");
    }

    #[test]
    fn retryable_codex_failure_reason_detects_capacity_errors() {
        let message = r#"codex exited with status exit status: 1. stderr: <empty> stdout: {"type":"thread.started","thread_id":"019f1402-16a8-7ad3-bef0-ab3aa1bd9259"}
{"type":"turn.started"}
{"type":"error","message":"Selected model is at capacity. Please try a different model."}
{"type":"turn.failed","error":{"message":"Selected model is at capacity. Please try a different model."}}"#;

        assert_eq!(
            retryable_codex_failure_reason(message).as_deref(),
            Some("selected model is at capacity")
        );
    }

    #[test]
    fn retryable_codex_failure_reason_from_output_uses_untruncated_stdout() {
        let stdout = format!(
            "{}\n{}",
            "x".repeat(5000),
            r#"{"type":"error","message":"Selected model is at capacity. Please try a different model."}"#
        );
        let truncated_message = format!(
            "codex exited with status exit status: 1. stderr: <empty> stdout: {}",
            truncate_for_log(stdout.trim(), 4000)
        );

        assert_eq!(retryable_codex_failure_reason(&truncated_message), None);
        assert_eq!(
            retryable_codex_failure_reason_from_output(&stdout, "", "").as_deref(),
            Some("selected model is at capacity")
        );
    }

    #[test]
    fn retryable_codex_failure_reason_from_output_ignores_plain_stdout_mentions() {
        let stdout = "package fixture text mentioning rate limit without a Codex error event";

        assert_eq!(
            retryable_codex_failure_reason_from_output(stdout, "", ""),
            None
        );
    }

    #[test]
    fn retryable_codex_failure_reason_ignores_unknown_exit_errors() {
        let message = "codex exited with status exit status: 1. stderr: invalid option";

        assert_eq!(retryable_codex_failure_reason(message), None);
    }

    #[test]
    fn base_codex_retry_delay_uses_bounded_backoff() {
        assert_eq!(base_codex_retry_delay_ms(1), 15_000);
        assert_eq!(base_codex_retry_delay_ms(2), 45_000);
        assert_eq!(base_codex_retry_delay_ms(3), 120_000);
        assert_eq!(base_codex_retry_delay_ms(10), 120_000);
    }

    #[test]
    fn codex_args_force_review_isolation() {
        let mut cmd = std::process::Command::new("codex");
        apply_codex_args(
            &mut cmd,
            Some("gpt-5.4"),
            Some("high"),
            std::path::Path::new("output.json"),
            std::path::Path::new("schema.json"),
        );

        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|window| window == ["--ask-for-approval", APPROVAL_POLICY]));
        assert!(args.iter().any(|arg| arg == "exec"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--sandbox", SANDBOX_MODE]));
        assert!(args.iter().any(|arg| arg == "--ignore-rules"));
        assert!(args.iter().any(|arg| arg == "--ephemeral"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--output-schema", "schema.json"]));
        assert!(args.iter().any(|arg| arg == "--json"));
        assert!(
            command_log(Some("gpt-5.4"), Some("high")).contains("--ask-for-approval never exec")
        );
        assert!(command_log(Some("gpt-5.4"), Some("high"))
            .contains("--sandbox read-only --ignore-rules --ephemeral --skip-git-repo-check"));
        assert!(command_log(Some("gpt-5.4"), Some("high"))
            .contains("--output-schema <schema> --json --output-last-message <file>"));
    }

    #[test]
    fn codex_environment_uses_allowlist() {
        let mut cmd = std::process::Command::new("codex");
        apply_codex_environment_from(
            &mut cmd,
            [
                ("PATH", "/usr/bin"),
                ("OPENAI_API_KEY", "test-openai-key"),
                ("HTTPS_PROXY", "http://proxy.example"),
                ("AWS_SECRET_ACCESS_KEY", "test-aws-secret"),
                ("GITHUB_TOKEN", "test-github-token"),
                ("SSH_AUTH_SOCK", "/tmp/ssh-agent.sock"),
            ],
        );

        let env = cmd
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            env.get("PATH").and_then(|value| value.as_deref()),
            Some("/usr/bin")
        );
        assert_eq!(
            env.get("OPENAI_API_KEY").and_then(|value| value.as_deref()),
            Some("test-openai-key")
        );
        assert_eq!(
            env.get("HTTPS_PROXY").and_then(|value| value.as_deref()),
            Some("http://proxy.example")
        );
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!env.contains_key("GITHUB_TOKEN"));
        assert!(!env.contains_key("SSH_AUTH_SOCK"));
    }
}
