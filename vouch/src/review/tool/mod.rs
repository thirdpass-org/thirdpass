use anyhow::Result;
mod agent;
mod vscode;

use crate::common;

pub use agent::{prompt_version as agent_prompt_version, AgentKind, AgentRunResult};

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
    workspace_directory: &std::path::PathBuf,
    config: &common::config::Config,
) -> Result<()> {
    assert!(
        config.review_tool.install_check,
        "Attempted to run review tool whilst install check is false."
    );

    log::debug!("Running review tool.");
    vscode::run(&workspace_directory)?;
    log::debug!("Review tool exit complete.");
    Ok(())
}

pub fn select_agent() -> Result<AgentKind> {
    agent::select_installed_agent()
}

pub fn run_agent(
    agent: AgentKind,
    workspace_path: &std::path::PathBuf,
    display_path: &str,
    file_contents: &str,
) -> Result<AgentRunResult> {
    agent::run(agent, workspace_path, display_path, file_contents)
}

/// Setup reviews directory within workspace.
pub fn ensure_reviews_directory(
    workspace_directory: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    let review_directory = vscode::setup_reviews_directory(&workspace_directory)?;
    Ok(review_directory)
}
