use anyhow::{format_err, Context, Result};
use dialoguer::Input;
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::FromStr;

use crate::review::comment::common::Position;
use crate::review::comment::{Comment, Selection};
use crate::review::common::{Priority, ReviewConfidence};

const CODEX_APPROVAL_POLICY: &str = "never";
const CODEX_ALLOWED_ENV: &[&str] = &[
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
const CODEX_SANDBOX_MODE: &str = "read-only";
const CLAUDE_ALLOWED_ENV: &[&str] = &[
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
const CLAUDE_PERMISSION_MODE: &str = "dontAsk";
const REVIEW_STRATEGY: &str = "package-release/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Codex,
    Claude,
}

impl AgentKind {
    fn binary_name(&self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::Claude => "claude",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::Claude => "claude",
        }
    }

    pub fn is_installed(&self) -> bool {
        is_command_available(self.binary_name())
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "codex" => Some(AgentKind::Codex),
            "claude" => Some(AgentKind::Claude),
            _ => None,
        }
    }
}

pub struct AgentRunResult {
    pub model: String,
    pub comments: Vec<Comment>,
    pub summary: Option<String>,
    pub confidence: Option<ReviewConfidence>,
}

#[derive(Debug, Deserialize)]
struct AgentOutput {
    model: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    confidence: Option<ReviewConfidence>,
    comments: Vec<AgentComment>,
}

#[derive(Debug, Deserialize)]
struct AgentComment {
    comment: String,
    security: Priority,
    complexity: Priority,
    #[serde(rename = "file")]
    path: PathBuf,
    #[serde(default)]
    selection: Option<Selection>,
}

impl AgentComment {
    fn into_comment(self) -> Comment {
        Comment {
            id: 0,
            security: self.security,
            complexity: self.complexity,
            path: self.path,
            message: self.comment,
            selection: self.selection,
        }
    }
}

pub fn review_strategy() -> &'static str {
    REVIEW_STRATEGY
}

pub fn select_installed_agent(preferred: Option<AgentKind>) -> Result<AgentKind> {
    let mut available = Vec::new();
    if is_command_available("codex") {
        available.push(AgentKind::Codex);
    }
    if is_command_available("claude") {
        available.push(AgentKind::Claude);
    }

    if available.is_empty() {
        return Err(format_err!(
            "No supported agents found. Install codex or claude."
        ));
    }

    if let Some(preferred) = preferred {
        if available.contains(&preferred) {
            return Ok(preferred);
        }
    }

    if available.len() == 1 {
        return Ok(available[0]);
    }

    println!("Select agent:");
    for (index, agent) in available.iter().enumerate() {
        println!("  {}. {}", index + 1, agent.name());
    }
    let selection: usize = Input::new()
        .with_prompt("Enter number")
        .validate_with(|value: &usize| {
            if *value == 0 || *value > available.len() {
                Err("Selection out of range.")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    Ok(available[selection - 1])
}

pub fn run(
    agent: AgentKind,
    workspace_path: &std::path::PathBuf,
    display_path: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<AgentRunResult> {
    let prompt = build_prompt(display_path);

    if agent == AgentKind::Codex {
        return run_codex_exec(workspace_path, &prompt, agent_model, agent_reasoning_effort);
    }

    log::debug!(
        "Launching agent: {} (cwd: {})",
        build_agent_log(agent, agent_model, agent_reasoning_effort),
        workspace_path.display()
    );
    let mut command = Command::new(agent.binary_name());
    if agent == AgentKind::Claude {
        apply_claude_environment(&mut command);
        apply_claude_args(&mut command);
    }
    let mut child = command
        .current_dir(workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format_err!("Failed to start {}: {}", agent.binary_name(), err))?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or(format_err!("Failed to open agent stdin"))?;
    if let Err(err) = stdin.write_all(prompt.as_bytes()) {
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
            if let Some(message) = detect_agent_failure(agent, stdout_trimmed, &stderr) {
                return Err(format_err!("{}", message));
            }
            return Err(format_err!(
                "{} terminated early (broken pipe). {}",
                agent.binary_name(),
                details.join(" ")
            ));
        }
        return Err(err.into());
    }

    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        if let Some(message) = detect_agent_failure(agent, stdout_raw.trim(), &stderr) {
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
            agent.binary_name(),
            output.status,
            details.join(" ")
        ));
    }

    let stdout = stdout_raw.to_string();
    let output = parse_agent_output(&stdout).map_err(|err| {
        if stderr.is_empty() {
            err
        } else {
            format_err!("{}; stderr: {}", err, stderr)
        }
    })?;
    let comments = output
        .comments
        .into_iter()
        .map(|comment| comment.into_comment())
        .collect();

    Ok(AgentRunResult {
        model: recorded_codex_model(agent_model, output.model),
        comments,
        summary: output.summary.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }),
        confidence: output.confidence,
    })
}

fn run_codex_exec(
    workspace_path: &std::path::PathBuf,
    prompt: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<AgentRunResult> {
    let command_log = build_agent_log(AgentKind::Codex, agent_model, agent_reasoning_effort);
    log::debug!(
        "Launching agent: {} (cwd: {})",
        command_log,
        workspace_path.display()
    );

    let output_file = tempfile::NamedTempFile::new()?;
    let output_path = output_file.path().to_path_buf();

    let mut cmd = Command::new(AgentKind::Codex.binary_name());
    apply_codex_environment(&mut cmd);
    apply_codex_args(&mut cmd, agent_model, agent_reasoning_effort, &output_path);
    cmd.arg("-");
    cmd.current_dir(workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| format_err!("Failed to start codex: {}", err))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or(format_err!("Failed to open codex stdin"))?;
    stdin.write_all(prompt.as_bytes())?;
    drop(stdin);

    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_raw = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        let output_payload = std::fs::read_to_string(&output_path).unwrap_or_default();
        let diagnostic_note = codex_failure_diagnostic_note(CodexFailureDiagnostic {
            phase: "process-exit",
            status: Some(output.status.to_string()),
            command_log: command_log.as_str(),
            workspace_path,
            prompt,
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
        return Err(format_err!(
            "codex exited with status {}. {}{}",
            output.status,
            details.join(" "),
            diagnostic_note
        ));
    }

    let output_payload = std::fs::read_to_string(&output_path).unwrap_or_default();
    let output_payload = if output_payload.trim().is_empty() {
        stdout_raw.to_string()
    } else {
        output_payload
    };

    let output = parse_agent_output(&output_payload).map_err(|err| {
        let diagnostic_note = codex_failure_diagnostic_note(CodexFailureDiagnostic {
            phase: "parse-output",
            status: None,
            command_log: command_log.as_str(),
            workspace_path,
            prompt,
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
    let comments = output
        .comments
        .into_iter()
        .map(|comment| comment.into_comment())
        .collect();

    Ok(AgentRunResult {
        model: recorded_codex_model(agent_model, output.model),
        comments,
        summary: output.summary.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }),
        confidence: output.confidence,
    })
}

fn recorded_codex_model(requested_model: Option<&str>, reported_model: String) -> String {
    requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
        .unwrap_or(reported_model)
}

fn truncate_for_log(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let mut truncated = chars.by_ref().take(max_len).collect::<String>();
    if chars.next().is_none() {
        return value.to_string();
    }
    truncated.push_str("…<truncated>");
    truncated
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
) {
    cmd.arg("--ask-for-approval");
    cmd.arg(CODEX_APPROVAL_POLICY);
    cmd.arg("exec");
    apply_codex_exec_args(cmd, agent_model, agent_reasoning_effort, output_path);
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
    apply_allowed_environment_from(cmd, variables, CODEX_ALLOWED_ENV);
}

fn apply_claude_args(cmd: &mut Command) {
    cmd.arg("-p");
    cmd.arg("--input-format");
    cmd.arg("text");
    cmd.arg("--output-format");
    cmd.arg("text");
    cmd.arg("--permission-mode");
    cmd.arg(CLAUDE_PERMISSION_MODE);
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
    apply_allowed_environment_from(cmd, variables, CLAUDE_ALLOWED_ENV);
}

fn apply_allowed_environment_from<I, K, V>(cmd: &mut Command, variables: I, allowed_env: &[&str])
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    for (key, value) in variables {
        if allows_env_key(key.as_ref(), allowed_env) {
            cmd.env(key, value);
        }
    }
}

fn allows_env_key(key: &std::ffi::OsStr, allowed_env: &[&str]) -> bool {
    allowed_env
        .iter()
        .any(|allowed| key == std::ffi::OsStr::new(allowed))
}

fn apply_codex_exec_args(
    cmd: &mut Command,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
    output_path: &std::path::Path,
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
    cmd.arg(CODEX_SANDBOX_MODE);
    cmd.arg("--ignore-rules");
    cmd.arg("--ephemeral");
    cmd.arg("--skip-git-repo-check");
    cmd.arg("--output-last-message");
    cmd.arg(output_path);
}

fn build_agent_log(
    agent: AgentKind,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> String {
    let mut parts = vec![agent.binary_name().to_string()];
    if agent == AgentKind::Codex {
        parts.push("--ask-for-approval".to_string());
        parts.push(CODEX_APPROVAL_POLICY.to_string());
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
        parts.push(CODEX_SANDBOX_MODE.to_string());
        parts.push("--ignore-rules".to_string());
        parts.push("--ephemeral".to_string());
    } else if agent == AgentKind::Claude {
        parts.push("-p".to_string());
        parts.push("--input-format".to_string());
        parts.push("text".to_string());
        parts.push("--output-format".to_string());
        parts.push("text".to_string());
        parts.push("--permission-mode".to_string());
        parts.push(CLAUDE_PERMISSION_MODE.to_string());
        parts.push("--tools".to_string());
        parts.push("Read".to_string());
        parts.push("--disable-slash-commands".to_string());
        parts.push("--strict-mcp-config".to_string());
        parts.push("--no-session-persistence".to_string());
        parts.push("--no-chrome".to_string());
        parts.push("--setting-sources".to_string());
        parts.push("user".to_string());
    }
    parts.join(" ")
}

fn detect_agent_failure(agent: AgentKind, stdout: &str, stderr: &str) -> Option<String> {
    if agent != AgentKind::Claude {
        return None;
    }

    let combined = format!("{} {}", stdout, stderr).to_lowercase();
    let limit_markers = [
        "hit your limit",
        "rate limit",
        "quota",
        "usage limit",
        "limit · resets",
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
    for part in value.split('·') {
        let trimmed = part.trim();
        if trimmed.to_lowercase().contains("reset") {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn build_prompt(display_path: &str) -> String {
    format!(
        r#"You are a malicious-code reviewer for open-source dependency archives.
Your goal is to detect evidence of supply-chain compromise or malicious behavior.
This is NOT a general vulnerability audit: avoid generic "unsafe pattern" findings unless they
are used to execute hidden/encoded/remote/untrusted payloads or are unsafe-by-default.

Review ONLY the target file at the path below. You are in read-only mode.
Inspect the target file from the current workspace before returning JSON.
You may inspect other files in the package if your tool supports it, but only report issues in the target file.
If the target file is binary, unreadable, or not meaningful as text, treat the review as a reachability review:
inspect package metadata, install scripts, wrappers, source files, and manifests for references that execute,
load, unpack, import, or otherwise pass control to the target file.

Focus areas (security):
- install-time execution (preinstall/postinstall), hidden subprocess execution
- credential/secret harvesting (env vars, .npmrc, .ssh, cloud metadata, tokens)
- data exfiltration (network calls, webhooks, DNS, pastebins)
- hidden downloads or dynamic code loading (remote fetch + eval/exec, require from URL)
- obfuscation/deobfuscation used to construct or execute payloads (base64, XOR, RC4)
- persistence or environment tampering (shell profiles, PATH, startup files)
- crypto-mining or unrelated system probing

Focus areas (complexity):
- heavy obfuscation or packing, control-flow flattening
- reflection/dynamic dispatch that hides behavior
- deliberately confusing parsing/decoding pipelines that mask intent

Rules:
- Output ONLY valid JSON, no markdown, no extra keys.
- Always include a brief summary and confidence, even if there are no comments.
- The summary must be specific to the target file: describe what it appears to contain or do,
  and mention the security-relevant behavior you checked.
- If comments is empty, the summary must positively state that no concrete malicious or
  supply-chain indicators were found and briefly name the checked categories that were absent,
  such as install hooks, network/exfiltration, credential access, dynamic code loading,
  obfuscation, or persistence.
- Do not use generic clean summaries like "looks fine" or "no issues found" without explaining
  what was reviewed.
- If there are no concrete malicious or supply-chain indicators, return an empty comments list.
- Comments must be specific and actionable, tied to the shown code, and include evidence:
  behavior + trigger + impact + why it is suspicious.
- Comments may mention other files only as context for behavior in the target file.
- Do not report a comment if the suspicious behavior is only present in another file.
- For binary or unreadable target files, only report when another package file uses the target as an opaque executable,
  loadable payload, unpacked artifact, or surprising runtime asset.
- Each comment's file field and selection must point to the target file.
- Bundled/minified code is in scope, but only report when behavior is clearly malicious or suspicious-by-default.
- Do NOT flag common patterns (eval/new Function/dynamic require) unless tied to executing
  encoded/remote/untrusted input or concealing a payload.
- Do not flag clearly intentional, explicitly signposted risky capabilities when they are consistent
  with the package's apparent purpose.
- Do flag misleading, hidden, or insecure-by-default behavior, including security-sensitive actions that are implicit,
  surprising, or not opt-in.
- Prefer false negatives over low-confidence findings; if uncertain, return no comments.
- Use selection only when you can point to exact lines; otherwise omit it.
- Line/character numbers are 1-based.
- Do not speculate about other files.

Return ONLY valid JSON with this schema. Do NOT include any preamble or code fences.
{{
  "model": "<model name used>",
  "summary": "<one or two sentence target-specific summary of what was reviewed and what was found or ruled out>",
  "confidence": "high|medium|low",
  "comments": [
    {{
      "comment": "string (what is the issue and why it matters)",
      "security": "critical|medium|low",
      "complexity": "critical|medium|low",
      "file": "{file_path}",
      "selection": {{
        "start": {{"line": <int>, "character": <int>}},
        "end": {{"line": <int>, "character": <int>}}
      }}
    }}
  ]
}}

If no issues are found, return an empty comments list.

Target file path (relative to current workspace): {file_path}
"#,
        file_path = display_path
    )
}

fn parse_agent_output(raw: &str) -> Result<AgentOutput> {
    let trimmed = raw.trim();
    if let Ok(output) = serde_json::from_str::<AgentOutput>(trimmed) {
        return Ok(output);
    }

    let extracted = extract_json_payload(raw).unwrap_or_else(|| trimmed.to_string());
    if let Ok(output) = serde_json::from_str::<AgentOutput>(&extracted) {
        return Ok(output);
    }

    let value: Value = serde_json::from_str(&extracted).map_err(|err| {
        format_err!(
            "Failed to parse agent JSON output: {}. Output: {}",
            err,
            extracted
        )
    })?;
    parse_agent_value(value)
}

fn is_command_available(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(name).is_file()))
}

fn extract_json_payload(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```json") {
        let rest = &trimmed[start + "```json".len()..];
        if let Some(end) = rest.find("```") {
            return Some(rest[..end].trim().to_string());
        }
    }
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + "```".len()..];
        if let Some(end) = rest.find("```") {
            return Some(rest[..end].trim().to_string());
        }
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(trimmed[start..=end].to_string())
}

fn parse_agent_value(value: Value) -> Result<AgentOutput> {
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let confidence_value = value
        .get("confidence")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("overall_security_confidence")
                .and_then(|v| v.as_str())
        });
    let confidence = match confidence_value {
        Some(value) => ReviewConfidence::from_str(value).ok(),
        None => None,
    };
    let comments_value = value
        .get("comments")
        .ok_or(format_err!("Agent output missing comments array"))?;
    let comments_array = comments_value
        .as_array()
        .ok_or(format_err!("Agent comments is not an array"))?;

    let mut comments = Vec::new();
    for entry in comments_array {
        let comment = entry
            .get("comment")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("description").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();
        if comment.is_empty() {
            log::warn!("Skipping agent comment without description.");
            continue;
        }

        let path_value = entry
            .get("file")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("path").and_then(|v| v.as_str()));
        let path_value = match path_value {
            Some(path_value) => path_value,
            None => {
                log::warn!("Skipping agent comment without file/path.");
                continue;
            }
        };

        let security = parse_priority(
            entry.get("security").and_then(|v| v.as_str()),
            entry.get("severity").and_then(|v| v.as_str()),
            entry.get("security_finding").and_then(|v| v.as_bool()),
        );
        let complexity = parse_complexity(
            entry.get("complexity").and_then(|v| v.as_str()),
            entry.get("complexity_finding").and_then(|v| v.as_bool()),
        );

        let selection = parse_selection(entry);
        comments.push(AgentComment {
            comment,
            security,
            complexity,
            path: PathBuf::from(path_value),
            selection,
        });
    }

    Ok(AgentOutput {
        model,
        summary,
        confidence,
        comments,
    })
}

fn parse_priority(
    priority_value: Option<&str>,
    severity_value: Option<&str>,
    security_finding: Option<bool>,
) -> Priority {
    if let Some(value) = priority_value {
        if let Ok(priority) = Priority::from_str(value) {
            return priority;
        }
    }
    if let Some(value) = severity_value {
        let value = value.to_lowercase();
        return match value.as_str() {
            "critical" | "high" => Priority::Critical,
            "medium" | "moderate" => Priority::Medium,
            "low" | "info" => Priority::Low,
            _ => Priority::Medium,
        };
    }
    if let Some(true) = security_finding {
        return Priority::Medium;
    }
    Priority::Low
}

fn parse_complexity(priority_value: Option<&str>, complexity_finding: Option<bool>) -> Priority {
    if let Some(value) = priority_value {
        if let Ok(priority) = Priority::from_str(value) {
            return priority;
        }
    }
    if let Some(true) = complexity_finding {
        return Priority::Medium;
    }
    Priority::Low
}

fn parse_selection(entry: &Value) -> Option<Selection> {
    if let Some(selection_value) = entry.get("selection") {
        let start = selection_value.get("start")?;
        let end = selection_value.get("end")?;
        let start_line = start.get("line")?.as_i64()?;
        let start_char = start.get("character")?.as_i64()?;
        let end_line = end.get("line")?.as_i64()?;
        let end_char = end.get("character")?.as_i64()?;
        return Some(Selection {
            start: Position {
                line: start_line,
                character: start_char,
            },
            end: Position {
                line: end_line,
                character: end_char,
            },
        });
    }

    let start_line = entry.get("line_start").and_then(|v| v.as_i64());
    let end_line = entry.get("line_end").and_then(|v| v.as_i64());
    if let (Some(start_line), Some(end_line)) = (start_line, end_line) {
        return Some(Selection {
            start: Position {
                line: start_line,
                character: 1,
            },
            end: Position {
                line: end_line,
                character: 1,
            },
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        apply_claude_args, apply_claude_environment_from, apply_codex_args,
        apply_codex_environment_from, build_agent_log, build_prompt, recorded_codex_model,
        review_strategy, save_codex_failure_diagnostic_in, truncate_for_log, AgentKind,
        CodexFailureDiagnostic, CLAUDE_PERMISSION_MODE, CODEX_APPROVAL_POLICY, CODEX_SANDBOX_MODE,
    };

    #[test]
    fn review_strategy_identifies_package_release_strategy() {
        assert_eq!(review_strategy(), "package-release/v1");
    }

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
    fn build_prompt_points_agent_at_target_path() {
        let prompt = build_prompt("src/index.js");

        assert!(prompt.contains("Target file path (relative to current workspace): src/index.js"));
        assert!(prompt.contains("Inspect the target file from the current workspace"));
        assert!(prompt.contains("The summary must be specific to the target file"));
        assert!(prompt.contains("briefly name the checked categories that were absent"));
        assert!(prompt.contains("treat the review as a reachability review"));
        assert!(prompt.contains("uses the target as an opaque executable"));
    }

    #[test]
    fn build_prompt_does_not_embed_file_contents() {
        let prompt = build_prompt("data/labels.json");

        assert!(!prompt.contains("--- FILE CONTENTS ---"));
        assert!(!prompt.contains("review me"));
    }

    #[test]
    fn truncate_for_log_handles_utf8() {
        assert_eq!(
            truncate_for_log("\u{05d0}\u{05d1}\u{05d2}\u{05d3}\u{05d4}", 3),
            "\u{05d0}\u{05d1}\u{05d2}…<truncated>"
        );
        assert_eq!(
            truncate_for_log("\u{05d0}\u{05d1}\u{05d2}", 3),
            "\u{05d0}\u{05d1}\u{05d2}"
        );
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
    fn codex_args_force_review_isolation() {
        let mut cmd = std::process::Command::new("codex");
        apply_codex_args(
            &mut cmd,
            Some("gpt-5.4"),
            Some("high"),
            std::path::Path::new("output.json"),
        );

        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|window| window == ["--ask-for-approval", CODEX_APPROVAL_POLICY]));
        assert!(args.iter().any(|arg| arg == "exec"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--sandbox", CODEX_SANDBOX_MODE]));
        assert!(args.iter().any(|arg| arg == "--ignore-rules"));
        assert!(args.iter().any(|arg| arg == "--ephemeral"));
        assert!(
            build_agent_log(AgentKind::Codex, Some("gpt-5.4"), Some("high"))
                .contains("--ask-for-approval never exec")
        );
        assert!(
            build_agent_log(AgentKind::Codex, Some("gpt-5.4"), Some("high"))
                .contains("--sandbox read-only --ignore-rules --ephemeral")
        );
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
            .any(|window| window == ["--permission-mode", CLAUDE_PERMISSION_MODE]));
        assert!(args.windows(2).any(|window| window == ["--tools", "Read"]));
        assert!(args.iter().any(|arg| arg == "--disable-slash-commands"));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(args.iter().any(|arg| arg == "--no-chrome"));
        assert!(args
            .windows(2)
            .any(|window| window == ["--setting-sources", "user"]));
        assert!(!args.iter().any(|arg| arg == "--bare"));
        assert!(build_agent_log(AgentKind::Claude, None, None)
            .contains("--permission-mode dontAsk --tools Read"));
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
