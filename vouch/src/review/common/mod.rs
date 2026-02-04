//! A module for data structures which are available to all super modules.
//!
//! This module contains data structures which are available to all super modules.
//! The number of data structures in this module should be minimized. The data structures
//! should be as simple as possible.
//!
//! Print statements are prohibited within this module. Logging is allowed.

use serde;
use std::hash::Hash;

pub mod priority;
pub mod metadata;
pub mod security_summary;
pub mod summary;

pub use metadata::ReviewMetadata;
pub use priority::Priority;
pub use security_summary::SecuritySummary;
pub use summary::Summary;

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Review {
    #[serde(skip)]
    pub id: crate::common::index::ID,
    #[serde(skip)]
    pub peer: crate::peer::Peer,
    pub package: crate::package::Package,
    pub comments: std::collections::BTreeSet<crate::review::comment::Comment>,
    #[serde(default)]
    pub metadata: ReviewMetadata,
    #[serde(default)]
    pub target_file: Option<std::path::PathBuf>,
    #[serde(default)]
    pub overall_security_summary: SecuritySummary,
}

impl Ord for Review {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.peer, &self.package, &self.comments, &self.id).cmp(&(
            &other.peer,
            &other.package,
            &other.comments,
            &other.id,
        ))
    }
}

impl PartialOrd for Review {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
