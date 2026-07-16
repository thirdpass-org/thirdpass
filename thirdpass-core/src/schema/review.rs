use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

use super::FileHash;

/// Review output for a single package-relative file.
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
    /// Reviewer-reported metrics for the agent invocation that reviewed this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_run_metrics: Option<AgentRunMetrics>,
    /// Specific comments reported for the reviewed file.
    pub comments: Vec<ReviewComment>,
}

/// Reviewer-reported runtime and token metrics for one agent invocation.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AgentRunMetrics {
    /// Wall-clock duration for the agent invocation.
    pub duration_ms: u64,
    /// Total agent invocation attempts needed to produce this review.
    #[serde(default = "default_agent_run_attempts")]
    pub attempts: u64,
    /// Unix timestamp in milliseconds when the invocation started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_unix_ms: Option<u64>,
    /// Unix timestamp in milliseconds when the invocation completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_unix_ms: Option<u64>,
    /// Cumulative token usage for the full invocation, when reported by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_token_usage: Option<AgentTokenUsage>,
    /// Token usage for the final model turn, when reported by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_token_usage: Option<AgentTokenUsage>,
    /// Model context window reported by the agent, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_window: Option<u64>,
    /// Wall-clock duration spent in failed attempts before the successful attempt.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub failed_attempt_duration_ms: u64,
    /// Wall-clock duration spent waiting before retry attempts.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub retry_wait_ms: u64,
    /// Retryable failure reasons observed before the successful attempt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_reasons: Vec<String>,
}

fn default_agent_run_attempts() -> u64 {
    1
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// Token counters reported by an agent invocation.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AgentTokenUsage {
    /// Input tokens sent to the model.
    pub input_tokens: u64,
    /// Input tokens served from cache.
    pub cached_input_tokens: u64,
    /// Output tokens produced by the model.
    pub output_tokens: u64,
    /// Output tokens used for model reasoning.
    pub reasoning_output_tokens: u64,
    /// Total tokens reported for this usage record.
    pub total_tokens: u64,
}

/// Comment or finding reported during a file review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    /// Human-readable comment text.
    pub comment: String,
    /// Security priority assigned to the comment.
    pub security: Priority,
    /// Complexity priority assigned to the comment.
    pub complexity: Priority,
    /// Optional source selection associated with the comment.
    #[serde(default)]
    pub selection: Option<Selection>,
}

/// Source range selected by a review comment.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Selection {
    /// Inclusive start position.
    pub start: Position,
    /// Exclusive end position.
    pub end: Position,
}

/// Zero-based line and character position within a file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Position {
    /// Zero-based line number.
    pub line: i64,
    /// Zero-based character offset within the line.
    pub character: i64,
}

/// Metadata describing the client and agent that produced a review.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewerDetails {
    /// Public reviewer identifier shown by the website.
    pub public_user_id: String,
    /// Review agent executable or provider name.
    pub agent_name: String,
    /// Review agent model identifier.
    pub agent_model: String,
    /// Review agent reasoning effort or equivalent setting.
    pub agent_reasoning_effort: String,
    /// Strategy identifier used to produce the review.
    pub review_strategy: String,
    /// Scope of files covered by the review.
    pub review_scope: ReviewScope,
    /// Review creation timestamp serialized by the client.
    pub created_at: String,
    /// Thirdpass client version that produced the review.
    pub thirdpass_version: String,
}

/// Configuration metadata describing how a review was produced.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewConfiguration {
    /// Human-readable review procedure identifier.
    pub review_procedure: String,
    /// Prompt template and output contract identifier used by the client.
    pub prompt_version: String,
    /// Agent identity and model settings used for the review.
    pub agent: ReviewConfigurationAgent,
    /// Tool, sandbox, and process settings used while running the review.
    pub execution_environment: ReviewExecutionEnvironment,
}

/// Agent identity and model-level settings for a review.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewConfigurationAgent {
    /// Review agent executable or provider name.
    pub name: String,
    /// Review agent model identifier.
    pub model: String,
    /// Model settings intentionally selected by the review client.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, String>,
}

/// Execution environment settings for a review agent invocation.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewExecutionEnvironment {
    /// Identifier for the tool policy used by the review client.
    pub tool_policy: String,
    /// Execution settings intentionally selected by the review client.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, String>,
}

/// Scope of source coverage represented by a review.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd,
)]
#[serde(rename_all = "snake_case")]
pub enum ReviewScope {
    /// The review covers the full target file.
    TargetFileFull,
    /// The review covers only part of the target file.
    #[default]
    TargetFilePartial,
}

impl ReviewScope {
    /// Return the serialized snake-case value for this review scope.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewScope::TargetFileFull => "target_file_full",
            ReviewScope::TargetFilePartial => "target_file_partial",
        }
    }

    /// Parse a review scope, defaulting unknown values to partial coverage.
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

/// Coarse priority for a finding or comment.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd,
)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Critical priority.
    Critical,
    /// Medium priority.
    #[default]
    Medium,
    /// Low priority.
    Low,
}

/// Overall security outcome for a file or package review.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd,
)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySummary {
    /// Critical security concern found.
    Critical,
    /// Medium security concern found.
    Medium,
    /// Low security concern found.
    Low,
    /// No security concern found.
    #[default]
    None,
}

/// Confidence level assigned by a review agent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum ReviewConfidence {
    /// High confidence.
    High,
    /// Medium confidence.
    Medium,
    /// Low confidence.
    Low,
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}

impl fmt::Display for SecuritySummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}

impl fmt::Display for ReviewConfidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
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

/// Query parameters for filtering review records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewQuery {
    /// Optional registry host filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_host: Option<String>,
    /// Optional package name filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// Optional package version filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    /// Optional package-relative file path filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}
