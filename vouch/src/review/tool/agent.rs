use anyhow::{format_err, Result};
use dialoguer::{theme::ColorfulTheme, Select};
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

    let options: Vec<&str> = available.iter().map(|agent| agent.name()).collect();
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select agent")
        .items(&options)
        .default(0)
        .interact()?;
    Ok(available[selection])
}

pub fn run(
    agent: AgentKind,
    target_path: &std::path::PathBuf,
    file_contents: &str,
) -> Result<AgentRunResult> {
    let prompt = build_prompt(target_path, file_contents);

    let mut child = Command::new(agent.binary_name())
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

fn build_prompt(target_path: &std::path::PathBuf, file_contents: &str) -> String {
    format!(
        r#"You are a security and quality reviewer. Review the file below.

Return ONLY valid JSON with this schema:
{{
  "model": "<model name used>",
  "comments": [
    {{
      "comment": "string",
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
        file_path = target_path.display(),
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
