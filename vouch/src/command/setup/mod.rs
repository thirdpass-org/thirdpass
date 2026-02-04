use anyhow::Result;
use crate::common::config::Config;
use crate::store;
use uuid;
mod fs;

pub fn ensure() -> Result<()> {
    if fs::is_complete()? {
        ensure_core_config()?;
        return Ok(());
    }

    fs::setup(false)?;

    let mut store = store::Store::from_root()?;
    let tx = store.get_transaction()?;

    store::index::setup(&tx)?;

    tx.commit("Initialize Vouch data.")?;
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
        config.core.api_base = "https://api.vouch.review".to_string();
        changed = true;
    }
    if changed {
        config.dump()?;
    }
    Ok(())
}
