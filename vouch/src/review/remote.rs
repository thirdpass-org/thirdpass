use anyhow::{format_err, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::common;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;
use crate::review::comment::{Comment, Selection};
use crate::review::common::{Priority, ReviewMetadata, SecuritySummary};

#[derive(Debug, Serialize, Deserialize)]
struct ReviewTarget {
    registry_host: String,
    package_name: String,
    package_version: String,
    file_path: String,
    artifact_hash: String,
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
    id: String,
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

pub fn store_records(
    records: Vec<ReviewRecord>,
    config: &common::config::Config,
    tx: &common::StoreTransaction,
) -> Result<usize> {
    let mut stored = 0;
    for record in records {
        if record.metadata.reviewer_uuid == config.core.reviewer_uuid {
            continue;
        }
        upsert_record(record, config, tx)?;
        stored += 1;
    }
    Ok(stored)
}

fn upsert_record(
    record: ReviewRecord,
    config: &common::config::Config,
    tx: &common::StoreTransaction,
) -> Result<()> {
    let ReviewRecord {
        target,
        metadata,
        comments,
        overall_security_summary,
        ..
    } = record;
    let registry = registry::index::ensure_host(&target.registry_host, tx)?;
    let package = ensure_package(&target, &registry, tx)?;
    let peer = peer::index::ensure_reviewer_peer(&metadata.reviewer_uuid, &config.core.api_base, tx)?;
    let comments = insert_comments(comments, tx)?;

    let existing = review::index::get(
        &review::index::Fields {
            peer: Some(&peer),
            package_name: Some(&package.name),
            package_version: Some(&package.version),
            ..Default::default()
        },
        tx,
    )?;

    let review = if let Some(current) = existing
        .into_iter()
        .find(|candidate| candidate.package.artifact_hash == target.artifact_hash)
    {
        review::Review {
            id: current.id,
            peer,
            package,
            comments,
            metadata,
            target_file: Some(std::path::PathBuf::from(target.file_path)),
            overall_security_summary,
        }
    } else {
        let inserted = review::index::insert(&comments, &peer, &package, tx)?;
        review::Review {
            id: inserted.id,
            peer,
            package,
            comments,
            metadata,
            target_file: Some(std::path::PathBuf::from(target.file_path)),
            overall_security_summary,
        }
    };

    review::store(&review, tx)?;
    Ok(())
}

fn ensure_package(
    target: &ReviewTarget,
    registry: &registry::Registry,
    tx: &common::StoreTransaction,
) -> Result<package::Package> {
    let candidates = package::index::get(
        &package::index::Fields {
            package_name: Some(&target.package_name),
            package_version: Some(&target.package_version),
            ..Default::default()
        },
        tx,
    )?;
    if let Some(existing) = candidates
        .into_iter()
        .find(|candidate| candidate.artifact_hash == target.artifact_hash)
    {
        return Ok(existing);
    }

    package::index::insert(
        &target.package_name,
        &target.package_version,
        &maplit::btreeset! { registry.clone() },
        &target.artifact_hash,
        tx,
    )
}

fn insert_comments(
    comments: Vec<ReviewComment>,
    tx: &common::StoreTransaction,
) -> Result<BTreeSet<Comment>> {
    let mut inserted = BTreeSet::new();
    for comment in comments {
        let comment = comment.into_comment();
        let comment = review::comment::index::insert(&comment, tx)?;
        inserted.insert(comment);
    }
    Ok(inserted)
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
