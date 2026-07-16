//! Wire-format types shared by Thirdpass clients and the server API.
//!
//! These structures are serialized as JSON for review requests, review
//! assignments, review submissions, and review records. They intentionally keep
//! fields simple and explicit so API consumers can construct or inspect payloads
//! without depending on CLI-only state.

mod assignment;
mod package;
mod review;
mod submission;

pub use assignment::{ReviewAssignment, ReviewCandidate, ReviewRequest};
pub use package::{
    FileHash, FileHashAlgorithm, PackageManifest, PackageManifestFile, ReviewTarget,
};
pub use review::{
    AgentRunMetrics, AgentTokenUsage, Position, Priority, ReviewComment, ReviewConfidence,
    ReviewConfiguration, ReviewConfigurationAgent, ReviewExecutionEnvironment, ReviewFile,
    ReviewQuery, ReviewScope, ReviewerDetails, SecuritySummary, Selection,
};
pub use submission::{ReviewRecord, ReviewSubmission};

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
            agent_run_metrics: Some(AgentRunMetrics {
                duration_ms: 1_234,
                attempts: 3,
                started_at_unix_ms: Some(1_800_000_000_000),
                finished_at_unix_ms: Some(1_800_000_001_234),
                total_token_usage: Some(AgentTokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 80,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 120,
                }),
                last_token_usage: Some(AgentTokenUsage {
                    input_tokens: 40,
                    cached_input_tokens: 30,
                    output_tokens: 10,
                    reasoning_output_tokens: 2,
                    total_tokens: 50,
                }),
                model_context_window: Some(258_400),
                failed_attempt_duration_ms: 4_000,
                retry_wait_ms: 60_000,
                retry_reasons: vec![
                    "selected model is at capacity".to_string(),
                    "response stream disconnected".to_string(),
                ],
            }),
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
                "agent_run_metrics": {
                    "duration_ms": 1234,
                    "attempts": 3,
                    "started_at_unix_ms": 1800000000000u64,
                    "finished_at_unix_ms": 1800000001234u64,
                    "total_token_usage": {
                        "input_tokens": 100,
                        "cached_input_tokens": 80,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 5,
                        "total_tokens": 120
                    },
                    "last_token_usage": {
                        "input_tokens": 40,
                        "cached_input_tokens": 30,
                        "output_tokens": 10,
                        "reasoning_output_tokens": 2,
                        "total_tokens": 50
                    },
                    "model_context_window": 258400,
                    "failed_attempt_duration_ms": 4000,
                    "retry_wait_ms": 60000,
                    "retry_reasons": [
                        "selected model is at capacity",
                        "response stream disconnected"
                    ]
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
        assert_eq!(file.summary, None);
        assert_eq!(file.security_summary, None);
        assert_eq!(file.confidence, None);
        assert_eq!(file.agent_run_metrics, None);
    }

    #[test]
    fn agent_run_metrics_defaults_missing_retry_metadata() {
        let file: ReviewFile = serde_json::from_value(json!({
            "file_path": "src/index.js",
            "agent_run_metrics": {
                "duration_ms": 1234
            },
            "comments": []
        }))
        .expect("failed to deserialize review file");

        let metrics = file.agent_run_metrics.expect("expected agent run metrics");
        assert_eq!(metrics.attempts, 1);
        assert_eq!(metrics.failed_attempt_duration_ms, 0);
        assert_eq!(metrics.retry_wait_ms, 0);
        assert!(metrics.retry_reasons.is_empty());
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
                "review_strategy": "file-focused-review/v1",
                "review_scope": "target_file_full",
                "created_at": "2026-05-04T00:00:00Z",
                "thirdpass_version": "0.3.2"
            },
            "files": []
        }))
        .expect("failed to deserialize review submission");

        assert_eq!(submission.package_manifest, None);
        assert_eq!(submission.review_configuration, None);
    }

    #[test]
    fn review_submission_carries_review_configuration() {
        let submission: ReviewSubmission = serde_json::from_value(json!({
            "target": {
                "registry_host": "crates.io",
                "package_name": "hashbrown",
                "package_version": "0.17.1",
                "package_hash": "abc"
            },
            "reviewer_details": {
                "public_user_id": "user-1",
                "agent_name": "codex",
                "agent_model": "gpt-5.4-mini",
                "agent_reasoning_effort": "high",
                "review_strategy": "file-focused-review/v1",
                "review_scope": "target_file_full",
                "created_at": "2026-05-04T00:00:00Z",
                "thirdpass_version": "0.6.0"
            },
            "review_configuration": {
                "review_procedure": "file-focused-review/v1",
                "prompt_version": "thirdpass-file-focused-review-prompt/v1",
                "agent": {
                    "name": "codex",
                    "model": "gpt-5.4-mini",
                    "settings": {
                        "reasoning_effort": "high"
                    }
                },
                "execution_environment": {
                    "tool_policy": "codex-readonly-review/v1",
                    "settings": {
                        "sandbox": "read-only"
                    }
                }
            },
            "files": []
        }))
        .expect("failed to deserialize review submission");

        let configuration = submission
            .review_configuration
            .expect("expected review configuration");
        assert_eq!(configuration.review_procedure, "file-focused-review/v1");
        assert_eq!(
            configuration.agent.settings.get("reasoning_effort"),
            Some(&"high".to_string())
        );
        assert_eq!(
            configuration.execution_environment.settings.get("sandbox"),
            Some(&"read-only".to_string())
        );
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
        assert!(request.review_target_policies.is_empty());
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
