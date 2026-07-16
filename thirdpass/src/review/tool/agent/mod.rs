use anyhow::{format_err, Result};
use dialoguer::Input;
use std::collections::BTreeMap;
use std::process::Command;

use crate::review::comment::Comment;
use crate::review::common::ReviewConfidence;

mod claude;
mod codex;
mod metrics;
mod prompt;

const REVIEW_STRATEGY: &str = "file-focused-review/v1";
const REVIEW_PROCEDURE: &str = "file-focused-review/v1";
const PROMPT_VERSION: &str = "thirdpass-file-focused-review-prompt/v1";

/// Supported automated review agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// OpenAI Codex CLI.
    Codex,
    /// Anthropic Claude CLI.
    Claude,
}

impl AgentKind {
    fn binary_name(&self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::Claude => "claude",
        }
    }

    fn command(&self) -> std::ffi::OsString {
        match self {
            AgentKind::Codex => codex::command(),
            AgentKind::Claude => self.binary_name().into(),
        }
    }

    /// Return the persisted agent name.
    pub fn name(&self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::Claude => "claude",
        }
    }

    /// Return true when the agent command is available.
    pub fn is_installed(&self) -> bool {
        is_command_available(&self.command())
    }

    /// Parse a persisted or CLI-provided agent name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "codex" => Some(AgentKind::Codex),
            "claude" => Some(AgentKind::Claude),
            _ => None,
        }
    }
}

/// Output collected from one automated review agent run.
pub struct AgentRunResult {
    /// Model recorded for the review artifact.
    pub model: String,
    /// Review comments emitted by the agent.
    pub comments: Vec<Comment>,
    /// Target-specific review summary emitted by the agent.
    pub summary: Option<String>,
    /// Agent-reported confidence for the target review.
    pub confidence: Option<ReviewConfidence>,
    /// Runtime and token metrics collected from the agent process.
    pub run_metrics: Option<thirdpass_core::schema::AgentRunMetrics>,
}

/// Return the strategy identifier stored on review artifacts.
pub fn review_strategy() -> &'static str {
    REVIEW_STRATEGY
}

/// Return the procedure identifier stored in review configuration metadata.
pub fn review_procedure() -> &'static str {
    REVIEW_PROCEDURE
}

/// Return the prompt version stored in review configuration metadata.
pub fn prompt_version() -> &'static str {
    PROMPT_VERSION
}

/// Build review configuration metadata for an agent run.
pub fn review_configuration(
    agent: AgentKind,
    agent_model: &str,
    agent_reasoning_effort: &str,
) -> thirdpass_core::schema::ReviewConfiguration {
    let mut agent_settings = BTreeMap::new();
    if !agent_reasoning_effort.trim().is_empty() {
        agent_settings.insert(
            "reasoning_effort".to_string(),
            agent_reasoning_effort.to_string(),
        );
    }

    thirdpass_core::schema::ReviewConfiguration {
        review_procedure: review_procedure().to_string(),
        prompt_version: prompt_version().to_string(),
        agent: thirdpass_core::schema::ReviewConfigurationAgent {
            name: agent.name().to_string(),
            model: agent_model.to_string(),
            settings: agent_settings,
        },
        execution_environment: execution_environment(agent),
    }
}

fn execution_environment(agent: AgentKind) -> thirdpass_core::schema::ReviewExecutionEnvironment {
    match agent {
        AgentKind::Codex => codex::execution_environment(),
        AgentKind::Claude => claude::execution_environment(),
    }
}

/// Select an installed agent, preferring the persisted choice when possible.
pub fn select_installed_agent(preferred: Option<AgentKind>) -> Result<AgentKind> {
    let mut available = Vec::new();
    if AgentKind::Codex.is_installed() {
        available.push(AgentKind::Codex);
    }
    if AgentKind::Claude.is_installed() {
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

/// Run an automated review agent against one target file.
pub fn run(
    agent: AgentKind,
    workspace_path: &std::path::PathBuf,
    display_path: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<AgentRunResult> {
    let prompt = prompt::build_prompt(display_path);
    match agent {
        AgentKind::Codex => {
            codex::run(workspace_path, &prompt, agent_model, agent_reasoning_effort)
        }
        AgentKind::Claude => claude::run(workspace_path, &prompt),
    }
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

fn is_command_available(command: &std::ffi::OsStr) -> bool {
    let command_path = std::path::Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.is_file();
    }

    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(command).is_file()))
}

fn truncate_for_log(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let mut truncated = chars.by_ref().take(max_len).collect::<String>();
    if chars.next().is_none() {
        return value.to_string();
    }
    truncated.push('\u{2026}');
    truncated.push_str("<truncated>");
    truncated
}

#[cfg(test)]
mod tests {
    use super::{
        is_command_available, review_configuration, review_strategy, truncate_for_log, AgentKind,
    };

    #[test]
    fn review_strategy_identifies_file_focused_review() {
        assert_eq!(review_strategy(), "file-focused-review/v1");
    }

    #[test]
    fn review_configuration_records_codex_review_metadata() {
        let configuration = review_configuration(AgentKind::Codex, "gpt-5.4-mini", "high");

        assert_eq!(configuration.review_procedure, "file-focused-review/v1");
        assert_eq!(
            configuration.prompt_version,
            "thirdpass-file-focused-review-prompt/v1"
        );
        assert_eq!(configuration.agent.name, "codex");
        assert_eq!(configuration.agent.model, "gpt-5.4-mini");
        assert_eq!(
            configuration.agent.settings.get("reasoning_effort"),
            Some(&"high".to_string())
        );
        assert_eq!(
            configuration.execution_environment.tool_policy,
            "codex-readonly-review/v1"
        );
        assert_eq!(
            configuration.execution_environment.settings.get("sandbox"),
            Some(&"read-only".to_string())
        );
        assert_eq!(
            configuration
                .execution_environment
                .settings
                .get("approval_policy"),
            Some(&"never".to_string())
        );
    }

    #[test]
    fn truncate_for_log_handles_utf8() {
        assert_eq!(
            truncate_for_log("\u{05d0}\u{05d1}\u{05d2}\u{05d3}\u{05d4}", 3),
            "\u{05d0}\u{05d1}\u{05d2}\u{2026}<truncated>"
        );
        assert_eq!(
            truncate_for_log("\u{05d0}\u{05d1}\u{05d2}", 3),
            "\u{05d0}\u{05d1}\u{05d2}"
        );
    }

    #[test]
    fn command_available_accepts_explicit_path() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let command_path = dir.path().join("codex");
        std::fs::write(&command_path, "")?;

        assert!(is_command_available(command_path.as_os_str()));
        assert!(!is_command_available(
            dir.path().join("missing-codex").as_os_str()
        ));

        Ok(())
    }
}
