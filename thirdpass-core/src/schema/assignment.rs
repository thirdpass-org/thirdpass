use serde::{Deserialize, Serialize};

/// Request body used by a client when asking the server for review work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// Explicit candidate targets the client can choose from.
    pub candidates: Vec<ReviewCandidate>,
    /// Registry hosts supported by the requesting client.
    pub supported_registry_hosts: Vec<String>,
    /// Automatic target selection policies for supported registry hosts.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub review_target_policies:
        std::collections::BTreeMap<String, crate::extension::ReviewTargetPolicy>,
}

/// Server assignment response for one review request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAssignment {
    /// Assigned target, or `None` when no target is available.
    pub target: Option<ReviewCandidate>,
}

/// Candidate package files that a client can review.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewCandidate {
    /// Registry host that identifies the package ecosystem.
    pub registry_host: String,
    /// Package name to review.
    pub package_name: String,
    /// Package version to review.
    pub package_version: String,
    /// Primary file path for single-file targets.
    pub file_path: String,
    /// Full file list for bundled targets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_paths: Vec<String>,
    /// Package content hash for the selected release archive.
    pub package_hash: String,
}

impl ReviewCandidate {
    /// Return the files included in this assignment.
    pub fn target_file_paths(&self) -> Vec<String> {
        if self.file_paths.is_empty() {
            vec![self.file_path.clone()]
        } else {
            self.file_paths.clone()
        }
    }
}
