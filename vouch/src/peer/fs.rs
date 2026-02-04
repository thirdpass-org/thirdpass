use anyhow::Result;
use crate::common::fs::DataPaths;

pub fn get_root_database() -> Result<rusqlite::Connection> {
    let paths = DataPaths::new()?;
    Ok(rusqlite::Connection::open(paths.index_file)?)
}
