use crate::common::config::Config;
use anyhow::Result;
use uuid;
mod fs;

pub fn ensure() -> Result<()> {
    if fs::is_complete()? {
        ensure_core_config()?;
        return Ok(());
    }

    fs::setup(false)?;

    ensure_core_config()?;
    Ok(())
}

fn ensure_core_config() -> Result<()> {
    let mut config = Config::load()?;
    let mut changed = false;
    if config.core.reviewer_uuid.is_empty() {
        config.core.reviewer_uuid = uuid::Uuid::new_v4().to_hyphenated().to_string();
        changed = true;
    }
    if config.core.api_base.is_empty() {
        config.core.api_base = "https://api.thirdpass.review".to_string();
        changed = true;
    }
    if config.review_tool.agent.is_none() {
        config.review_tool.agent = Some("codex".to_string());
        changed = true;
    }
    if config.review_tool.agent_model.is_none() {
        config.review_tool.agent_model = Some("gpt-5.5".to_string());
        changed = true;
    }
    if config.review_tool.agent_reasoning_effort.is_none() {
        config.review_tool.agent_reasoning_effort = Some("high".to_string());
        changed = true;
    }
    if changed {
        config.dump()?;
    }
    Ok(())
}
