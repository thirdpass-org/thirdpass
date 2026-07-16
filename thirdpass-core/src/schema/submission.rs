use serde::{Deserialize, Serialize};

use super::{
    PackageManifest, ReviewConfidence, ReviewConfiguration, ReviewFile, ReviewTarget,
    ReviewerDetails, SecuritySummary,
};

/// Review submission sent by a client to the Thirdpass server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSubmission {
    /// Package release under review.
    pub target: ReviewTarget,
    /// Client and agent metadata for the reviewer.
    pub reviewer_details: ReviewerDetails,
    /// Review procedure, prompt, agent, and execution metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_configuration: Option<ReviewConfiguration>,
    /// Files covered by this submission.
    pub files: Vec<ReviewFile>,
    /// Authoritative package file inventory, when supplied by trusted tooling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manifest: Option<PackageManifest>,
    /// Overall security summary across reviewed files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_security_summary: Option<SecuritySummary>,
    /// Agent confidence in the overall security summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_security_confidence: Option<ReviewConfidence>,
    /// Agent-written package-level summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
}

/// Approved review record returned by the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRecord {
    /// Server-assigned review identifier.
    pub id: String,
    /// Package release that was reviewed.
    pub target: ReviewTarget,
    /// Client and agent metadata for the reviewer.
    pub reviewer_details: ReviewerDetails,
    /// Review procedure, prompt, agent, and execution metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_configuration: Option<ReviewConfiguration>,
    /// Files covered by this review.
    pub files: Vec<ReviewFile>,
    /// Agent-written package-level summary.
    #[serde(default)]
    pub agent_summary: Option<String>,
    /// Overall security summary across reviewed files.
    pub overall_security_summary: SecuritySummary,
    /// Agent confidence in the overall security summary.
    #[serde(default)]
    pub overall_security_confidence: Option<ReviewConfidence>,
}
