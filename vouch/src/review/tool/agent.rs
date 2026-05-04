use anyhow::{format_err, Result};
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

const REVIEW_STRATEGY: &str = "supply-chain-dependency/v1";

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
            summary: None,
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
    file_contents: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<AgentRunResult> {
    let prompt = build_prompt(display_path, file_contents);

    if agent == AgentKind::Codex {
        return run_codex_exec(workspace_path, &prompt, agent_model, agent_reasoning_effort);
    }

    log::debug!(
        "Launching agent: {} (cwd: {})",
        build_agent_log(agent, agent_model, agent_reasoning_effort),
        workspace_path.display()
    );
    let mut child = Command::new(agent.binary_name())
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
    log::debug!(
        "Launching agent: {} (cwd: {})",
        build_agent_log(AgentKind::Codex, agent_model, agent_reasoning_effort),
        workspace_path.display()
    );

    let output_file = tempfile::NamedTempFile::new()?;
    let output_path = output_file.path().to_path_buf();

    let mut cmd = Command::new(AgentKind::Codex.binary_name());
    cmd.arg("exec");
    apply_codex_exec_args(&mut cmd, agent_model, agent_reasoning_effort, &output_path);
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
            "codex exited with status {}. {}",
            output.status,
            details.join(" ")
        ));
    }

    let output_payload = std::fs::read_to_string(&output_path).unwrap_or_default();
    let output_payload = if output_payload.trim().is_empty() {
        stdout_raw.to_string()
    } else {
        output_payload
    };

    let output = parse_agent_output(&output_payload).map_err(|err| {
        let stdout_trimmed = stdout_raw.trim();
        if stdout_trimmed.is_empty() {
            err
        } else {
            format_err!(
                "{}; stdout: {}",
                err,
                truncate_for_log(stdout_trimmed, 4000)
            )
        }
    })?;
    let comments = output
        .comments
        .into_iter()
        .map(|comment| comment.into_comment())
        .collect();

    Ok(AgentRunResult {
        model: output.model,
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
    if value.len() <= max_len {
        return value.to_string();
    }
    let mut truncated = value[..max_len].to_string();
    truncated.push_str("…<truncated>");
    truncated
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
        parts.push("exec".to_string());
        if let Some(model) = agent_model {
            parts.push("--model".to_string());
            parts.push(model.to_string());
        }
        if let Some(effort) = agent_reasoning_effort {
            parts.push("--config".to_string());
            parts.push(format!("model_reasoning_effort=\"{}\"", effort));
        }
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

fn build_prompt(display_path: &str, file_contents: &str) -> String {
    format!(
        r#"You are a malicious-code reviewer for open-source dependency archives.
Your goal is to detect evidence of supply-chain compromise or malicious behavior.
This is NOT a general vulnerability audit: avoid generic "unsafe pattern" findings unless they
are used to execute hidden/encoded/remote/untrusted payloads or are unsafe-by-default.

Review ONLY the single file below. You are in read-only mode.
You may inspect other files in the package if your tool supports it, but only report issues in the target file.

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
- If there are no concrete malicious or supply-chain indicators, return an empty comments list.
- Comments must be specific and actionable, tied to the shown code, and include evidence:
  behavior + trigger + impact + why it is suspicious.
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
  "summary": "<one or two sentence summary of the review in your own words>",
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

File path: {file_path}

--- FILE CONTENTS ---
{file_contents}
"#,
        file_path = display_path,
        file_contents = file_contents
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
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|path| path.join(name).is_file())
    })
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
    use super::{recorded_codex_model, review_strategy};

    #[test]
    fn review_strategy_identifies_supply_chain_dependency_strategy() {
        assert_eq!(review_strategy(), "supply-chain-dependency/v1");
    }

    #[test]
    fn recorded_codex_model_prefers_requested_model() {
        assert_eq!(
            recorded_codex_model(Some("gpt-5.5"), "GPT-5".to_string()),
            "gpt-5.5"
        );
    }

    #[test]
    fn recorded_codex_model_uses_reported_model_without_request() {
        assert_eq!(recorded_codex_model(None, "GPT-5".to_string()), "GPT-5");
    }
}
