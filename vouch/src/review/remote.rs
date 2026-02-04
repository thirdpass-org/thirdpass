use anyhow::{format_err, Result};
use serde::{Deserialize, Serialize};
use crate::common;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;
use crate::review::comment::{Comment, Selection};
use crate::review::common::{Priority, ReviewMetadata, SecuritySummary};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReviewTarget {
    pub registry_host: String,
    pub package_name: String,
    pub package_version: String,
    pub file_path: String,
    pub artifact_hash: String,
}

#[derive(Debug, Serialize)]
struct ReviewSubmission {
    target: ReviewTarget,
    metadata: ReviewMetadata,
    comments: Vec<ReviewComment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overall_security_summary: Option<SecuritySummary>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRecord {
    #[serde(rename = "id")]
    _id: String,
    target: ReviewTarget,
    metadata: ReviewMetadata,
    comments: Vec<ReviewComment>,
    overall_security_summary: SecuritySummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReviewComment {
    comment: String,
    security: Priority,
    complexity: Priority,
    file: String,
    #[serde(default)]
    selection: Option<Selection>,
}

impl ReviewComment {
    fn into_comment(self) -> Comment {
        Comment {
            id: 0,
            security: self.security,
            complexity: self.complexity,
            summary: None,
            path: std::path::PathBuf::from(self.file),
            message: self.comment,
            selection: self.selection,
        }
    }
}

#[derive(Debug, Default, Serialize)]
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

pub fn submit(review: &review::Review, config: &common::config::Config) -> Result<()> {
    let target_file = review
        .target_file
        .as_ref()
        .ok_or(format_err!("Review target file missing; cannot submit."))?;
    let registry = get_primary_registry(&review.package)?;
    let target = ReviewTarget {
        registry_host: registry.host_name.clone(),
        package_name: review.package.name.clone(),
        package_version: review.package.version.clone(),
        file_path: target_file.display().to_string(),
        artifact_hash: review.package.artifact_hash.clone(),
    };
    let comments = review
        .comments
        .iter()
        .cloned()
        .map(to_remote_comment)
        .collect::<Vec<_>>();

    let payload = ReviewSubmission {
        target,
        metadata: review.metadata.clone(),
        comments,
        overall_security_summary: Some(review.overall_security_summary.clone()),
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
    Ok(())
}

pub fn fetch(
    query: &ReviewQuery,
    config: &common::config::Config,
) -> Result<Vec<ReviewRecord>> {
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
    let reviews = response.json::<Vec<ReviewRecord>>()?;
    Ok(reviews)
}

#[derive(Debug, Serialize)]
struct ReviewRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<ReviewTarget>>,
}

#[derive(Debug, Deserialize)]
struct ReviewAssignment {
    target: Option<ReviewTarget>,
}

pub fn request_target(
    candidates: Vec<ReviewTarget>,
    config: &common::config::Config,
) -> Result<Option<ReviewTarget>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let payload = ReviewRequest {
        candidates: Some(candidates),
    };
    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/review-requests")?;
    let mut request = client
        .post(url)
        .header("User-Agent", common::HTTP_USER_AGENT);
    if !config.core.api_key.is_empty() {
        request = request.header("X-API-Key", config.core.api_key.clone());
    }
    let response = match request.json(&payload).send() {
        Ok(response) => response,
        Err(err) => {
            log::warn!("Failed to request target from API: {}", err);
            return Ok(None);
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        log::warn!("Review request failed ({}): {}", status, body);
        return Ok(None);
    }
    let assignment = match response.json::<ReviewAssignment>() {
        Ok(assignment) => assignment,
        Err(err) => {
            log::warn!("Failed to parse review request response: {}", err);
            return Ok(None);
        }
    };
    Ok(assignment.target)
}

pub fn store_records(
    records: Vec<ReviewRecord>,
    config: &common::config::Config,
) -> Result<usize> {
    let mut stored = 0;
    for record in records {
        if record.metadata.reviewer_uuid == config.core.reviewer_uuid {
            continue;
        }
        store_record(record, config)?;
        stored += 1;
    }
    Ok(stored)
}

fn store_record(
    record: ReviewRecord,
    config: &common::config::Config,
) -> Result<()> {
    let ReviewRecord {
        target,
        metadata,
        comments,
        overall_security_summary,
        ..
    } = record;
    let registry = build_registry(&target)?;
    let package = build_package(&target, &registry);
    let peer = peer::reviewer_peer(&metadata.reviewer_uuid, &config.core.api_base)?;
    let comments = comments
        .into_iter()
        .map(|comment| comment.into_comment())
        .collect::<std::collections::BTreeSet<_>>();

    let review = review::Review {
        id: 0,
        peer,
        package,
        comments,
        metadata,
        target_file: Some(std::path::PathBuf::from(target.file_path)),
        overall_security_summary,
    };

    review::store(&review)?;
    Ok(())
}

fn to_remote_comment(comment: Comment) -> ReviewComment {
    ReviewComment {
        comment: comment.message,
        security: comment.security,
        complexity: comment.complexity,
        file: comment.path.display().to_string(),
        selection: comment.selection,
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

fn build_registry(target: &ReviewTarget) -> Result<registry::Registry> {
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

fn build_package(
    target: &ReviewTarget,
    registry: &registry::Registry,
) -> package::Package {
    package::Package {
        id: 0,
        name: target.package_name.clone(),
        version: target.package_version.clone(),
        registries: maplit::btreeset! { registry.clone() },
        artifact_hash: target.artifact_hash.clone(),
    }
}
