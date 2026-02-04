use anyhow::{format_err, Result};
use crate::common::config::Config;
use crate::store;
use uuid;
mod fs;

/// Return Err if setup is not complete, otherwise Result.
pub fn is_complete() -> Result<()> {
    if !fs::is_complete()? {
        return Err(format_err!(
            "Vouch setup has not completed yet."
        ));
    }
    Ok(())
}

pub fn ensure() -> Result<()> {
    if fs::is_complete()? {
        ensure_reviewer_uuid()?;
        return Ok(());
    }

    fs::setup(false)?;

    let mut store = store::Store::from_root()?;
    let tx = store.get_transaction()?;

    store::index::setup(&tx)?;

    tx.commit("Initialize Vouch data.")?;
    ensure_reviewer_uuid()?;
    Ok(())
}

fn ensure_reviewer_uuid() -> Result<()> {
    let mut config = Config::load()?;
    if config.core.reviewer_uuid.is_empty() {
        config.core.reviewer_uuid = uuid::Uuid::new_v4().to_hyphenated().to_string();
        config.dump()?;
    }
    Ok(())
}
