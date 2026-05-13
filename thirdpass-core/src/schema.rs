use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewTarget {
    pub registry_host: String,
    pub package_name: String,
    pub package_version: String,
    pub package_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFile {
    /// Path of the reviewed file relative to the package root.
    pub file_path: String,
    /// Content hash for the reviewed file, when the client can compute it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<FileHash>,
    /// Agent-written summary for this individual file review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Security severity for this individual file review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_summary: Option<SecuritySummary>,
    /// Agent confidence for this individual file review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ReviewConfidence>,
    /// Specific comments reported for the reviewed file.
    pub comments: Vec<ReviewComment>,
}

/// File inventory for a package archive.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PackageManifest {
    /// Regular files found in the extracted package archive.
    pub files: Vec<PackageManifestFile>,
}

/// Metadata for a regular file in a package archive.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct PackageManifestFile {
    /// Path of the file relative to the package root.
    pub path: String,
    /// Size of the file contents in bytes.
    pub size_bytes: u64,
}

/// Content hash for a file included in a review.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FileHash {
    /// Algorithm used to produce the hash digest.
    pub algorithm: FileHashAlgorithm,
    /// Lowercase hexadecimal hash digest.
    pub value: String,
}

impl FileHash {
    /// Build a Blake3 file hash from a lowercase hexadecimal digest.
    pub fn blake3(value: impl Into<String>) -> Self {
        Self {
            algorithm: FileHashAlgorithm::Blake3,
            value: value.into(),
        }
    }
}

/// Supported content hash algorithms for reviewed files.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "lowercase")]
pub enum FileHashAlgorithm {
    /// The Blake3 cryptographic hash algorithm.
    Blake3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub comment: String,
    pub security: Priority,
    pub complexity: Priority,
    #[serde(default)]
    pub selection: Option<Selection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: i64,
    pub character: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSubmission {
    pub target: ReviewTarget,
    #[serde(alias = "metadata")]
    pub reviewer_details: ReviewerDetails,
    pub files: Vec<ReviewFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manifest: Option<PackageManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_security_summary: Option<SecuritySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_security_confidence: Option<ReviewConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub id: String,
    pub target: ReviewTarget,
    pub reviewer_details: ReviewerDetails,
    pub files: Vec<ReviewFile>,
    #[serde(default)]
    pub agent_summary: Option<String>,
    pub overall_security_summary: SecuritySummary,
    #[serde(default)]
    pub overall_security_confidence: Option<ReviewConfidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerDetails {
    pub public_user_id: String,
    pub agent_name: String,
    pub agent_model: String,
    pub agent_reasoning_effort: String,
    pub review_strategy: String,
    pub review_scope: ReviewScope,
    pub created_at: String,
    #[serde(alias = "tool_version")]
    pub thirdpass_version: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewScope {
    TargetFileFull,
    TargetFilePartial,
}

impl ReviewScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewScope::TargetFileFull => "target_file_full",
            ReviewScope::TargetFilePartial => "target_file_partial",
        }
    }

    pub fn parse_or_partial(value: &str) -> Self {
        match value {
            "target_file_full" => ReviewScope::TargetFileFull,
            "target_file_partial" => ReviewScope::TargetFilePartial,
            _ => ReviewScope::TargetFilePartial,
        }
    }
}

impl FromStr for ReviewScope {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse_or_partial(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Critical,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySummary {
    Critical,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReviewConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// Explicit candidate targets the client can choose from.
    pub candidates: Vec<ReviewCandidate>,
    /// Registry hosts supported by the requesting client.
    pub supported_registry_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAssignment {
    pub target: Option<ReviewCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewCandidate {
    /// Registry host that identifies the package ecosystem.
    pub registry_host: String,
    /// Package name to review.
    pub package_name: String,
    /// Package version to review.
    pub package_version: String,
    /// Primary file path for legacy clients and single-file targets.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBatchRequest {
    pub targets: Vec<ReviewTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBatchResponse {
    pub reviews: Vec<ReviewRecord>,
}

impl FromStr for Priority {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.to_lowercase().as_str() {
            "critical" => Ok(Priority::Critical),
            "medium" => Ok(Priority::Medium),
            "low" => Ok(Priority::Low),
            _ => Err(()),
        }
    }
}

impl FromStr for SecuritySummary {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.to_lowercase().as_str() {
            "critical" => Ok(SecuritySummary::Critical),
            "medium" => Ok(SecuritySummary::Medium),
            "low" => Ok(SecuritySummary::Low),
            "none" => Ok(SecuritySummary::None),
            _ => Err(()),
        }
    }
}

impl FromStr for ReviewConfidence {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.to_lowercase().as_str() {
            "high" => Ok(ReviewConfidence::High),
            "medium" => Ok(ReviewConfidence::Medium),
            "low" => Ok(ReviewConfidence::Low),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn review_file_serializes_blake3_hash_metadata() {
        let file = ReviewFile {
            file_path: "src/index.js".to_string(),
            file_hash: Some(FileHash::blake3("abc123")),
            summary: Some("Reviewed the entrypoint.".to_string()),
            security_summary: Some(SecuritySummary::Low),
            confidence: Some(ReviewConfidence::High),
            comments: vec![],
        };

        let value = serde_json::to_value(file).expect("failed to serialize review file");

        assert_eq!(
            value,
            json!({
                "file_path": "src/index.js",
                "file_hash": {
                    "algorithm": "blake3",
                    "value": "abc123"
                },
                "summary": "Reviewed the entrypoint.",
                "security_summary": "low",
                "confidence": "high",
                "comments": []
            })
        );
    }

    #[test]
    fn review_file_defaults_missing_file_hash() {
        let file: ReviewFile = serde_json::from_value(json!({
            "file_path": "src/index.js",
            "comments": []
        }))
        .expect("failed to deserialize review file");

        assert_eq!(file.file_hash, None);
        assert_eq!(file.summary, None);
        assert_eq!(file.security_summary, None);
        assert_eq!(file.confidence, None);
    }

    #[test]
    fn review_submission_defaults_missing_package_manifest() {
        let submission: ReviewSubmission = serde_json::from_value(json!({
            "target": {
                "registry_host": "npmjs.com",
                "package_name": "axios",
                "package_version": "1.6.8",
                "package_hash": "sha256:abc"
            },
            "reviewer_details": {
                "public_user_id": "user-1",
                "agent_name": "codex",
                "agent_model": "gpt-5.5",
                "agent_reasoning_effort": "high",
                "review_strategy": "package-release/v1",
                "review_scope": "target_file_full",
                "created_at": "2026-05-04T00:00:00Z",
                "thirdpass_version": "0.3.2"
            },
            "files": []
        }))
        .expect("failed to deserialize review submission");

        assert_eq!(submission.package_manifest, None);
    }

    #[test]
    fn review_request_carries_supported_registry_hosts() {
        let request: ReviewRequest = serde_json::from_value(json!({
            "candidates": [],
            "supported_registry_hosts": ["crates.io", "npmjs.com"]
        }))
        .expect("failed to deserialize review request");

        assert!(request.candidates.is_empty());
        assert_eq!(
            request.supported_registry_hosts,
            vec!["crates.io", "npmjs.com"]
        );
    }

    #[test]
    fn review_candidate_defaults_to_single_file_target() {
        let candidate: ReviewCandidate = serde_json::from_value(json!({
            "registry_host": "crates.io",
            "package_name": "hashbrown",
            "package_version": "0.17.1",
            "file_path": "src/map.rs",
            "package_hash": "hash"
        }))
        .expect("failed to deserialize review candidate");

        assert_eq!(candidate.target_file_paths(), vec!["src/map.rs"]);
    }

    #[test]
    fn review_candidate_can_include_bundled_file_targets() {
        let candidate: ReviewCandidate = serde_json::from_value(json!({
            "registry_host": "crates.io",
            "package_name": "hashbrown",
            "package_version": "0.17.1",
            "file_path": "src/map.rs",
            "file_paths": ["src/map.rs", "src/raw.rs"],
            "package_hash": "hash"
        }))
        .expect("failed to deserialize review candidate");

        assert_eq!(candidate.file_path, "src/map.rs");
        assert_eq!(
            candidate.target_file_paths(),
            vec!["src/map.rs", "src/raw.rs"]
        );
    }
}
