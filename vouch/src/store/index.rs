use anyhow::Result;

use crate::review;
use rusqlite;

use crate::common::StoreTransaction;
use crate::package;
use crate::peer;
use crate::registry;

pub struct Index {
    pub db: rusqlite::Connection,
}

/// An interface for the SQLite database.
///
/// This structure interfaces between SQL index structures and application wide structures.
impl Index {
    /// Load root index.
    pub fn from_root() -> Result<Self> {
        Ok(Self {
            db: peer::fs::get_root_database()?,
        })
    }

    /// Instantiate an in-memory index. Useful for tests.
    #[allow(dead_code)]
    pub fn in_memory() -> Result<Self> {
        Ok(Self {
            db: rusqlite::Connection::open_in_memory()?,
        })
    }
}

/// Setup database schema. Insert root peer.
pub fn setup(tx: &StoreTransaction) -> Result<()> {
    peer::index::setup(&tx)?;
    registry::index::setup(&tx)?;
    package::index::setup(&tx)?;
    review::index::setup(&tx)?;
    Ok(())
}

pub fn setup_in_memory(index: &mut Index) -> Result<()> {
    let tx = StoreTransaction::new(index.db.transaction()?)?;
    setup(&tx)?;
    tx.commit_index()?;
    Ok(())
}
