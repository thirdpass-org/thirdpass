use anyhow::{format_err, Result};
use dialoguer::Input;
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::FromStr;

use crate::review::comment::{Comment, Selection};
use crate::review::comment::common::Position;
use crate::review::common::Priority;

const PROMPT_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy)]
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
}

pub struct AgentRunResult {
    pub model: String,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Deserialize)]
struct AgentOutput {
    model: String,
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

pub fn prompt_version() -> &'static str {
    PROMPT_VERSION
}

pub fn select_installed_agent() -> Result<AgentKind> {
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
) -> Result<AgentRunResult> {
    let prompt = build_prompt(display_path, file_contents);

    let mut child = Command::new(agent.binary_name())
        .current_dir(workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| {
            format_err!(
                "Failed to start {}: {}",
                agent.binary_name(),
                err
            )
        })?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or(format_err!("Failed to open agent stdin"))?;
    stdin.write_all(prompt.as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(format_err!(
            "{} exited with status {}",
            agent.binary_name(),
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let output = parse_agent_output(&stdout)?;
    let comments = output
        .comments
        .into_iter()
        .map(|comment| comment.into_comment())
        .collect();

    Ok(AgentRunResult {
        model: output.model,
        comments,
    })
}

fn build_prompt(display_path: &str, file_contents: &str) -> String {
    format!(
        r#"You are a security and complexity reviewer for open-source dependency code.
Review ONLY the single file below. You are in read-only mode.
You may inspect other files in the package if your tool supports it, but only report issues in the target file.

Focus areas (security):
- remote code execution, deserialization hazards, eval/exec, command injection
- filesystem/network misuse, credential leaks, unsafe crypto, auth bypass
- suspicious supply-chain behavior (exfiltration, hidden downloads, obfuscation)

Focus areas (complexity):
- unusually complex or hard-to-audit logic that increases security risk
- hidden control flow, reflection/metaprogramming, overly clever parsing
- deeply nested conditionals or state machines without clear invariants

Rules:
- Output ONLY valid JSON, no markdown, no extra keys.
- If there are no concrete issues, return an empty comments list.
- Comments must be specific and actionable, tied to the shown code.
- Use selection only when you can point to exact lines; otherwise omit it.
- Line/character numbers are 1-based.
- Do not speculate about other files.

Return ONLY valid JSON with this schema. Do NOT include any preamble or code fences.
{{
  "model": "<model name used>",
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

    Ok(AgentOutput { model, comments })
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

fn parse_complexity(
    priority_value: Option<&str>,
    complexity_finding: Option<bool>,
) -> Priority {
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
