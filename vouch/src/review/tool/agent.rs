use anyhow::{format_err, Result};
use dialoguer::Input;
use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::review::comment::{Comment, Selection};
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

Return ONLY valid JSON with this schema:
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
    serde_json::from_str::<AgentOutput>(trimmed).map_err(|err| {
        format_err!(
            "Failed to parse agent JSON output: {}. Output: {}",
            err,
            trimmed
        )
    })
}

fn is_command_available(name: &str) -> bool {
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|path| path.join(name).is_file())
    })
}
