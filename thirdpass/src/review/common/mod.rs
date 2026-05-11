//! A module for data structures which are available to all super modules.
//!
//! This module contains data structures which are available to all super modules.
//! The number of data structures in this module should be minimized. The data structures
//! should be as simple as possible.
//!
//! Print statements are prohibited within this module. Logging is allowed.

use std::hash::Hash;

pub mod confidence;
pub mod metadata;
pub mod priority;
pub mod security_summary;
pub mod summary;

pub use confidence::ReviewConfidence;
pub use metadata::{ReviewScope, ReviewerDetails};
pub use priority::Priority;
pub use security_summary::SecuritySummary;
pub use summary::Summary;

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct ReviewTarget {
    pub file_path: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<thirdpass_core::schema::FileHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_summary: Option<SecuritySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ReviewConfidence>,
    pub comments: std::collections::BTreeSet<crate::review::comment::Comment>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Review {
    #[serde(skip)]
    pub id: crate::common::index::ID,
    #[serde(skip)]
    pub peer: crate::peer::Peer,
    pub package: crate::package::Package,
    pub targets: Vec<ReviewTarget>,
    #[serde(default)]
    pub reviewer_details: ReviewerDetails,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_summary: String,
    #[serde(default)]
    pub overall_security_summary: SecuritySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_security_confidence: Option<ReviewConfidence>,
}

impl Ord for Review {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.peer, &self.package, &self.targets, &self.id).cmp(&(
            &other.peer,
            &other.package,
            &other.targets,
            &other.id,
        ))
    }
}

impl PartialOrd for Review {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
