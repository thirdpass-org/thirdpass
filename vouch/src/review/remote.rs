use crate::common;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;
use crate::review::comment::{Comment, Selection};
use crate::review::common::{Priority, ReviewConfidence, ReviewerDetails, SecuritySummary};
use anyhow::{format_err, Result};
use vouch_lib::schema as api;

pub type ReviewCandidate = api::ReviewCandidate;
pub type ReviewQuery = api::ReviewQuery;

#[derive(Debug, serde::Deserialize)]
struct ReviewSubmitResponse {
    id: String,
}

pub fn submit(
    review: &review::Review,
    package_manifest: &api::PackageManifest,
    config: &common::config::Config,
) -> Result<String> {
    let registry = get_primary_registry(&review.package)?;
    let target = api::ReviewTarget {
        registry_host: registry.host_name.clone(),
        package_name: review.package.name.clone(),
        package_version: review.package.version.clone(),
        package_hash: review.package.package_hash.clone(),
    };
    let files = to_api_review_files(&review.targets);

    let payload = api::ReviewSubmission {
        target,
        files,
        package_manifest: Some(package_manifest.clone()),
        reviewer_details: to_api_reviewer_details(&review.reviewer_details),
        agent_summary: if review.agent_summary.trim().is_empty() {
            None
        } else {
            Some(review.agent_summary.clone())
        },
        overall_security_summary: None,
        overall_security_confidence: None,
    };

    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/reviews")?;
    let mut request = client
        .post(url)
        .header("User-Agent", common::HTTP_USER_AGENT);
    if !config.core.api_key.is_empty() {
        request = request.header("X-API-Key", config.core.api_key.clone());
    }
    let response = request.json(&payload).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format_err!(
            "Failed to submit review ({}): {}",
            status,
            body
        ));
    }
    Ok(response.json::<ReviewSubmitResponse>()?.id)
}

pub fn fetch(
    query: &api::ReviewQuery,
    config: &common::config::Config,
) -> Result<Vec<api::ReviewRecord>> {
    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/reviews")?;
    let mut request = client
        .get(url)
        .header("User-Agent", common::HTTP_USER_AGENT);
    if !config.core.api_key.is_empty() {
        request = request.header("X-API-Key", config.core.api_key.clone());
    }
    let response = request.query(&query).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format_err!(
            "Failed to fetch reviews ({}): {}",
            status,
            body
        ));
    }
    let reviews = response.json::<Vec<api::ReviewRecord>>()?;
    Ok(reviews)
}

pub fn request_target(
    candidates: Vec<api::ReviewCandidate>,
    config: &common::config::Config,
) -> Result<Option<api::ReviewCandidate>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let payload = api::ReviewRequest {
        candidates: Some(candidates),
    };
    let assignment = match post_review_request(&payload, config) {
        Ok(assignment) => assignment,
        Err(err) => {
            log::warn!("Failed to request target from API: {}", err);
            return Ok(None);
        }
    };
    Ok(assignment.target)
}

pub fn request_global_target(
    config: &common::config::Config,
) -> Result<Option<api::ReviewCandidate>> {
    let payload = api::ReviewRequest { candidates: None };
    Ok(post_review_request(&payload, config)?.target)
}

fn post_review_request(
    payload: &api::ReviewRequest,
    config: &common::config::Config,
) -> Result<api::ReviewAssignment> {
    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/review-requests")?;
    let mut request = client
        .post(url)
        .header("User-Agent", common::HTTP_USER_AGENT);
    if !config.core.api_key.is_empty() {
        request = request.header("X-API-Key", config.core.api_key.clone());
    }
    let response = request.json(&payload).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format_err!("Review request failed ({}): {}", status, body));
    }
    Ok(response.json::<api::ReviewAssignment>()?)
}

pub fn store_records(
    records: Vec<api::ReviewRecord>,
    config: &common::config::Config,
) -> Result<usize> {
    let mut stored = 0;
    for record in records {
        if record.reviewer_details.reviewer_uuid == config.core.reviewer_uuid {
            continue;
        }
        store_record(record, config)?;
        stored += 1;
    }
    Ok(stored)
}

fn store_record(record: api::ReviewRecord, config: &common::config::Config) -> Result<()> {
    let api::ReviewRecord {
        target,
        reviewer_details,
        files,
        overall_security_summary,
        overall_security_confidence,
        agent_summary,
        ..
    } = record;
    let registry = build_registry(&target)?;
    let package = build_package(&target, &registry);
    let peer = peer::reviewer_peer(&reviewer_details.reviewer_uuid, &config.core.api_base)?;
    let targets = files
        .into_iter()
        .map(|file| {
            let api::ReviewFile {
                file_path,
                file_hash,
                summary,
                security_summary,
                confidence,
                comments,
            } = file;
            let comments = comments
                .into_iter()
                .map(|comment| from_remote_comment(comment, &file_path))
                .collect::<std::collections::BTreeSet<_>>();
            review::ReviewTarget {
                file_path: std::path::PathBuf::from(file_path),
                file_hash,
                agent_summary: summary,
                security_summary: security_summary.as_ref().map(from_api_security_summary),
                confidence: confidence.as_ref().map(from_api_confidence),
                comments,
            }
        })
        .collect::<Vec<_>>();

    let review = review::Review {
        id: 0,
        peer,
        package,
        targets,
        reviewer_details: from_api_reviewer_details(&reviewer_details),
        agent_summary: agent_summary.unwrap_or_default(),
        overall_security_summary: from_api_security_summary(&overall_security_summary),
        overall_security_confidence: overall_security_confidence
            .as_ref()
            .map(from_api_confidence),
    };

    review::store_submitted(&review)?;
    Ok(())
}

fn to_api_review_files(targets: &[review::ReviewTarget]) -> Vec<api::ReviewFile> {
    targets
        .iter()
        .map(|target| api::ReviewFile {
            file_path: target.file_path.display().to_string(),
            file_hash: target.file_hash.clone(),
            summary: target.agent_summary.clone(),
            security_summary: target
                .security_summary
                .as_ref()
                .map(to_api_security_summary),
            confidence: target.confidence.as_ref().map(to_api_confidence),
            comments: target
                .comments
                .iter()
                .cloned()
                .map(to_remote_comment)
                .collect(),
        })
        .collect()
}

fn to_remote_comment(comment: Comment) -> api::ReviewComment {
    api::ReviewComment {
        comment: comment.message,
        security: to_api_priority(&comment.security),
        complexity: to_api_priority(&comment.complexity),
        selection: comment.selection.as_ref().map(to_api_selection),
    }
}

fn from_remote_comment(comment: api::ReviewComment, file_path: &str) -> Comment {
    Comment {
        id: 0,
        security: from_api_priority(&comment.security),
        complexity: from_api_priority(&comment.complexity),
        summary: None,
        path: std::path::PathBuf::from(file_path),
        message: comment.comment,
        selection: comment.selection.as_ref().map(from_api_selection),
    }
}

fn to_api_selection(selection: &Selection) -> api::Selection {
    api::Selection {
        start: api::Position {
            line: selection.start.line,
            character: selection.start.character,
        },
        end: api::Position {
            line: selection.end.line,
            character: selection.end.character,
        },
    }
}

fn from_api_selection(selection: &api::Selection) -> Selection {
    Selection {
        start: crate::review::comment::common::Position {
            line: selection.start.line,
            character: selection.start.character,
        },
        end: crate::review::comment::common::Position {
            line: selection.end.line,
            character: selection.end.character,
        },
    }
}

fn to_api_priority(priority: &Priority) -> api::Priority {
    match priority {
        Priority::Critical => api::Priority::Critical,
        Priority::Medium => api::Priority::Medium,
        Priority::Low => api::Priority::Low,
    }
}

fn from_api_priority(priority: &api::Priority) -> Priority {
    match priority {
        api::Priority::Critical => Priority::Critical,
        api::Priority::Medium => Priority::Medium,
        api::Priority::Low => Priority::Low,
    }
}

fn to_api_security_summary(summary: &SecuritySummary) -> api::SecuritySummary {
    match summary {
        SecuritySummary::Critical => api::SecuritySummary::Critical,
        SecuritySummary::Medium => api::SecuritySummary::Medium,
        SecuritySummary::Low => api::SecuritySummary::Low,
        SecuritySummary::None => api::SecuritySummary::None,
    }
}

fn from_api_security_summary(summary: &api::SecuritySummary) -> SecuritySummary {
    match summary {
        api::SecuritySummary::Critical => SecuritySummary::Critical,
        api::SecuritySummary::Medium => SecuritySummary::Medium,
        api::SecuritySummary::Low => SecuritySummary::Low,
        api::SecuritySummary::None => SecuritySummary::None,
    }
}

fn to_api_confidence(confidence: &ReviewConfidence) -> api::ReviewConfidence {
    match confidence {
        ReviewConfidence::High => api::ReviewConfidence::High,
        ReviewConfidence::Medium => api::ReviewConfidence::Medium,
        ReviewConfidence::Low => api::ReviewConfidence::Low,
    }
}

fn from_api_confidence(confidence: &api::ReviewConfidence) -> ReviewConfidence {
    match confidence {
        api::ReviewConfidence::High => ReviewConfidence::High,
        api::ReviewConfidence::Medium => ReviewConfidence::Medium,
        api::ReviewConfidence::Low => ReviewConfidence::Low,
    }
}

fn to_api_reviewer_details(details: &ReviewerDetails) -> api::ReviewerDetails {
    api::ReviewerDetails {
        reviewer_uuid: details.reviewer_uuid.clone(),
        agent_name: details.agent_name.clone(),
        agent_model: details.agent_model.clone(),
        agent_reasoning_effort: details.agent_reasoning_effort.clone(),
        review_strategy: details.review_strategy.clone(),
        review_scope: to_api_review_scope(&details.review_scope),
        created_at: details.created_at.clone(),
        vouch_version: details.vouch_version.clone(),
    }
}

fn from_api_reviewer_details(details: &api::ReviewerDetails) -> ReviewerDetails {
    ReviewerDetails {
        reviewer_uuid: details.reviewer_uuid.clone(),
        agent_name: details.agent_name.clone(),
        agent_model: details.agent_model.clone(),
        agent_reasoning_effort: details.agent_reasoning_effort.clone(),
        review_strategy: details.review_strategy.clone(),
        review_scope: from_api_review_scope(&details.review_scope),
        created_at: details.created_at.clone(),
        vouch_version: details.vouch_version.clone(),
    }
}

fn to_api_review_scope(scope: &review::ReviewScope) -> api::ReviewScope {
    match scope {
        review::ReviewScope::TargetFileFull => api::ReviewScope::TargetFileFull,
        review::ReviewScope::TargetFilePartial => api::ReviewScope::TargetFilePartial,
    }
}

fn from_api_review_scope(scope: &api::ReviewScope) -> review::ReviewScope {
    match scope {
        api::ReviewScope::TargetFileFull => review::ReviewScope::TargetFileFull,
        api::ReviewScope::TargetFilePartial => review::ReviewScope::TargetFilePartial,
    }
}

fn get_primary_registry<'a>(package: &'a package::Package) -> Result<&'a registry::Registry> {
    let registry = package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    Ok(registry)
}

fn build_registry(target: &api::ReviewTarget) -> Result<registry::Registry> {
    let host = target.registry_host.as_str();
    let human_url = url::Url::parse(&format!("https://{}/", host))?;
    let artifact_url = url::Url::parse(&format!("https://{}/artifact", host))?;
    Ok(registry::Registry {
        id: 0,
        host_name: target.registry_host.clone(),
        human_url,
        artifact_url,
    })
}

fn build_package(target: &api::ReviewTarget, registry: &registry::Registry) -> package::Package {
    package::Package {
        id: 0,
        name: target.package_name.clone(),
        version: target.package_version.clone(),
        registries: maplit::btreeset! { registry.clone() },
        package_hash: target.package_hash.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_api_review_files_preserves_file_hash() {
        let file_hash = api::FileHash::blake3("abc123");
        let targets = vec![review::ReviewTarget {
            file_path: std::path::PathBuf::from("index.js"),
            file_hash: Some(file_hash.clone()),
            agent_summary: Some("Reviewed the file.".to_string()),
            security_summary: Some(SecuritySummary::Low),
            confidence: Some(ReviewConfidence::High),
            comments: std::collections::BTreeSet::new(),
        }];

        let files = to_api_review_files(&targets);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "index.js");
        assert_eq!(files[0].file_hash, Some(file_hash));
        assert_eq!(files[0].summary.as_deref(), Some("Reviewed the file."));
        assert_eq!(files[0].security_summary, Some(api::SecuritySummary::Low));
        assert_eq!(files[0].confidence, Some(api::ReviewConfidence::High));
    }
}
