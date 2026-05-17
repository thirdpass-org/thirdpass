use anyhow::{format_err, Result};
mod agent;
mod vscode;

use crate::common;

pub use agent::{review_strategy, AgentKind, AgentRunResult};

pub fn check_manual_install(config: &mut common::config::Config) -> Result<()> {
    // Skip check if previously passed.
    if config.review_tool.install_check {
        return Ok(());
    }
    vscode::setup()?;

    config.review_tool.install_check = true;
    config.dump()?;

    Ok(())
}

pub fn run_manual(
    workspace_directory: &std::path::Path,
    config: &common::config::Config,
) -> Result<()> {
    assert!(
        config.review_tool.install_check,
        "Attempted to run review tool whilst install check is false."
    );

    log::debug!("Running review tool.");
    vscode::run(workspace_directory)?;
    log::debug!("Review tool exit complete.");
    Ok(())
}

pub fn select_agent(
    config: &mut common::config::Config,
    override_agent: Option<AgentKind>,
) -> Result<AgentKind> {
    if let Some(agent) = override_agent {
        if !agent.is_installed() {
            return Err(format_err!(
                "Requested agent '{}' is not installed.",
                agent.name()
            ));
        }
        let agent_name = agent.name();
        if config.review_tool.agent.as_deref() != Some(agent_name) {
            config.review_tool.agent = Some(agent_name.to_string());
            config.dump()?;
        }
        return Ok(agent);
    }

    let preferred = config
        .review_tool
        .agent
        .as_deref()
        .and_then(AgentKind::from_name);
    let agent = agent::select_installed_agent(preferred)?;
    let agent_name = agent.name();
    if config.review_tool.agent.as_deref() != Some(agent_name) {
        config.review_tool.agent = Some(agent_name.to_string());
        config.dump()?;
    }
    Ok(agent)
}

pub fn run_agent(
    agent: AgentKind,
    workspace_path: &std::path::PathBuf,
    display_path: &str,
    file_contents: &str,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> Result<AgentRunResult> {
    agent::run(
        agent,
        workspace_path,
        display_path,
        file_contents,
        agent_model,
        agent_reasoning_effort,
    )
}

/// Setup reviews directory within workspace.
pub fn ensure_reviews_directory(
    workspace_directory: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let review_directory = vscode::setup_reviews_directory(workspace_directory)?;
    Ok(review_directory)
}
