use crate::common::StoreTransaction;
use anyhow::Result;

pub mod index;

pub struct Store {
    index: index::Index,
}

impl Store {
    /// Load root store.
    pub fn from_root() -> Result<Self> {
        Ok(Self {
            index: index::Index::from_root()?,
        })
    }

    /// Load temporary storage. Useful for testing.
    #[allow(dead_code)]
    pub fn from_tmp() -> Result<Self> {
        let mut index = index::Index::in_memory()?;
        index::setup_in_memory(&mut index)?;
        Ok(Self { index })
    }

    pub fn get_transaction(&mut self) -> Result<StoreTransaction<'_>> {
        Ok(StoreTransaction::new(self.index.db.transaction()?)?)
    }
}
