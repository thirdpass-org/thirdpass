use anyhow::{format_err, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use super::{apply_allowed_environment_from, prompt, truncate_for_log, AgentKind};

const ALLOWED_ENV: &[&str] = &[
    "ALL_PROXY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CONFIG_DIR",
    "HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "NIX_SSL_CERT_FILE",
    "NO_PROXY",
    "PATH",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "TEMP",
    "TERM",
    "TMP",
    "TMPDIR",
];
const PERMISSION_MODE: &str = "dontAsk";
const TOOL_POLICY_VERSION: &str = "claude-readonly-review/v1";

pub(super) fn execution_environment() -> thirdpass_core::schema::ReviewExecutionEnvironment {
    let mut settings = BTreeMap::new();
    settings.insert("permission_mode".to_string(), PERMISSION_MODE.to_string());
    settings.insert("allowed_tools".to_string(), "Read".to_string());
    settings.insert("disable_slash_commands".to_string(), "true".to_string());
    settings.insert("strict_mcp_config".to_string(), "true".to_string());
    settings.insert("no_session_persistence".to_string(), "true".to_string());
    settings.insert("no_chrome".to_string(), "true".to_string());
    settings.insert("setting_sources".to_string(), "user".to_string());

    thirdpass_core::schema::ReviewExecutionEnvironment {
        tool_policy: TOOL_POLICY_VERSION.to_string(),
        settings,
    }
}

pub(super) fn run(
    workspace_path: &std::path::PathBuf,
    prompt_text: &str,
) -> Result<super::AgentRunResult> {
    log::debug!(
        "Launching agent: {} (cwd: {})",
        command_log(),
        workspace_path.display()
    );
    let agent_command = AgentKind::Claude.command();
    let mut command = Command::new(&agent_command);
    apply_claude_environment(&mut command);
    apply_claude_args(&mut command);
    let mut child = command
        .current_dir(workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            format_err!(
                "Failed to start {}: {}",
                agent_command.to_string_lossy(),
                err
            )
        })?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or(format_err!("Failed to open agent stdin"))?;
    if let Err(err) = stdin.write_all(prompt_text.as_bytes()) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            let output = child.wait_with_output()?;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout_raw = String::from_utf8_lossy(&output.stdout);
            let stdout_trimmed = stdout_raw.trim();
            let mut details = Vec::new();
            if stderr.is_empty() {
                details.push("stderr: <empty>".to_string());
            } else {
                details.push(format!("stderr: {}", truncate_for_log(&stderr, 4000)));
            }
            if !stdout_trimmed.is_empty() {
                details.push(format!(
                    "stdout: {}",
                    truncate_for_log(stdout_trimmed, 4000)
                ));
            }
            if let Some(message) = detect_failure(stdout_trimmed, &stderr) {
                return Err(format_err!("{}", message));
            }
            return Err(format_err!(
                "{} terminated early (broken pipe). {}",
                AgentKind::Claude.binary_name(),
                details.join(" ")
            ));
        }
        return Err(err.into());
    }

    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        if let Some(message) = detect_failure(stdout_raw.trim(), &stderr) {
            return Err(format_err!("{}", message));
        }
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
        return Err(format_err!(
            "{} exited with status {}. {}",
            AgentKind::Claude.binary_name(),
            output.status,
            details.join(" ")
        ));
    }

    let stdout = stdout_raw.to_string();
    let output = prompt::parse_agent_output(&stdout).map_err(|err| {
        if stderr.is_empty() {
            err
        } else {
            format_err!("{}; stderr: {}", err, stderr)
        }
    })?;
    let model = output.model.clone();
    Ok(output.into_run_result(model, None))
}

fn apply_claude_args(cmd: &mut Command) {
    cmd.arg("-p");
    cmd.arg("--input-format");
    cmd.arg("text");
    cmd.arg("--output-format");
    cmd.arg("text");
    cmd.arg("--permission-mode");
    cmd.arg(PERMISSION_MODE);
    cmd.arg("--tools");
    cmd.arg("Read");
    cmd.arg("--disable-slash-commands");
    cmd.arg("--strict-mcp-config");
    cmd.arg("--no-session-persistence");
    cmd.arg("--no-chrome");
    cmd.arg("--setting-sources");
    cmd.arg("user");
}

fn apply_claude_environment(cmd: &mut Command) {
    cmd.env_clear();
    apply_claude_environment_from(cmd, std::env::vars_os());
}

fn apply_claude_environment_from<I, K, V>(cmd: &mut Command, variables: I)
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    apply_allowed_environment_from(cmd, variables, ALLOWED_ENV);
}

fn command_log() -> String {
    let mut parts = vec![AgentKind::Claude.binary_name().to_string()];
    parts.push("-p".to_string());
    parts.push("--input-format".to_string());
    parts.push("text".to_string());
    parts.push("--output-format".to_string());
    parts.push("text".to_string());
    parts.push("--permission-mode".to_string());
    parts.push(PERMISSION_MODE.to_string());
    parts.push("--tools".to_string());
    parts.push("Read".to_string());
    parts.push("--disable-slash-commands".to_string());
    parts.push("--strict-mcp-config".to_string());
    parts.push("--no-session-persistence".to_string());
    parts.push("--no-chrome".to_string());
    parts.push("--setting-sources".to_string());
    parts.push("user".to_string());
    parts.join(" ")
}

fn detect_failure(stdout: &str, stderr: &str) -> Option<String> {
    let combined = format!("{} {}", stdout, stderr).to_lowercase();
    let limit_markers = [
        "hit your limit",
        "rate limit",
        "quota",
        "usage limit",
        "limit \u{00b7} resets",
    ];
    if !limit_markers.iter().any(|marker| combined.contains(marker)) {
        return None;
    }

    let reset_hint = extract_reset_hint(stdout)
        .or_else(|| extract_reset_hint(stderr))
        .unwrap_or_else(|| "reset time not provided".to_string());

    Some(format!(
        "Claude usage limit reached ({reset_hint}). Try again after reset or use --agent codex.",
        reset_hint = reset_hint
    ))
}

fn extract_reset_hint(value: &str) -> Option<String> {
    for part in value.split('\u{00b7}') {
        let trimmed = part.trim();
        if trimmed.to_lowercase().contains("reset") {
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{apply_claude_args, apply_claude_environment_from, command_log, PERMISSION_MODE};

    #[test]
    fn claude_args_force_noninteractive_read_only_tools() {
        let mut cmd = std::process::Command::new("claude");
        apply_claude_args(&mut cmd);

        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.iter().any(|arg| arg == "-p"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--permission-mode", PERMISSION_MODE]));
        assert!(args.windows(2).any(|window| window == ["--tools", "Read"]));
        assert!(args.iter().any(|arg| arg == "--disable-slash-commands"));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(args.iter().any(|arg| arg == "--no-chrome"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--setting-sources", "user"]));
        assert!(!args.iter().any(|arg| arg == "--bare"));
        assert!(command_log().contains("--permission-mode dontAsk --tools Read"));
    }

    #[test]
    fn claude_environment_uses_allowlist() {
        let mut cmd = std::process::Command::new("claude");
        apply_claude_environment_from(
            &mut cmd,
            [
                ("PATH", "/usr/bin"),
                ("ANTHROPIC_API_KEY", "test-anthropic-key"),
                ("HTTPS_PROXY", "http://proxy.example"),
                ("OPENAI_API_KEY", "test-openai-key"),
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
            env.get("ANTHROPIC_API_KEY")
                .and_then(|value| value.as_deref()),
            Some("test-anthropic-key")
        );
        assert_eq!(
            env.get("HTTPS_PROXY").and_then(|value| value.as_deref()),
            Some("http://proxy.example")
        );
        assert!(!env.contains_key("OPENAI_API_KEY"));
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!env.contains_key("GITHUB_TOKEN"));
        assert!(!env.contains_key("SSH_AUTH_SOCK"));
    }
}
