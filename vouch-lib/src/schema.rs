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
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<FileHash>,
    pub comments: Vec<ReviewComment>,
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
    pub reviewer_uuid: String,
    pub agent_name: String,
    pub agent_model: String,
    pub agent_reasoning_effort: String,
    pub review_strategy: String,
    pub review_scope: ReviewScope,
    pub created_at: String,
    #[serde(alias = "tool_version")]
    pub vouch_version: String,
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

    pub fn from_str(value: &str) -> Self {
        match value {
            "target_file_full" => ReviewScope::TargetFileFull,
            "target_file_partial" => ReviewScope::TargetFilePartial,
            _ => ReviewScope::TargetFilePartial,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Critical,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySummary {
    Critical,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<ReviewCandidate>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAssignment {
    pub target: Option<ReviewCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewCandidate {
    pub registry_host: String,
    pub package_name: String,
    pub package_version: String,
    pub file_path: String,
    pub package_hash: String,
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
    }
}
