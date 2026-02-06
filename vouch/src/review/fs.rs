use anyhow::{format_err, Context, Result};
use std::io::Write;

use crate::common;
use crate::review;

static REVIEW_FILE_PREFIX: &str = "review-";

/// Given a package, returns a package version specific relative directory path.
///
/// Example: "pypi.org/numpy/1.18.5"
pub fn get_unique_package_path(
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<std::path::PathBuf> {
    let registry_host_name = std::path::PathBuf::from(&registry_host_name);
    Ok(registry_host_name
        .join(&package_name)
        .join(&package_version))
}

fn get_storage_file_path(
    review: &review::Review,
    base_directory: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    // TODO: Handle multiple registries.
    let review_directory_path = get_unique_package_path(
        &review.package.name,
        &review.package.version,
        &review
            .package
            .registries
            .iter()
            .next()
            .ok_or(format_err!("Package does not have associated registries."))?
            .host_name,
    )?;

    let reviewer = if review.reviewer_details.reviewer_uuid.is_empty() {
        "unknown".to_string()
    } else {
        review.reviewer_details.reviewer_uuid.clone()
    };
    let package_specific_directory = base_directory
        .join(review_directory_path)
        .join(&review.package.artifact_hash)
        .join(reviewer);
    Ok(package_specific_directory.join(review_file_name()))
}

fn review_file_name() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}{}.json",
        REVIEW_FILE_PREFIX,
        uuid::Uuid::new_v4().to_hyphenated()
    ))
}

/// Store a review.
pub fn add(
    review: &review::Review,
    status: ReviewStorageStatus,
) -> Result<std::path::PathBuf> {
    let paths = common::fs::DataPaths::new()?;
    let base_directory = match status {
        ReviewStorageStatus::Submitted => &paths.reviews_directory,
        ReviewStorageStatus::Pending => &paths.pending_reviews_directory,
    };
    let file_path = get_storage_file_path(&review, base_directory)?;
    let parent_directory = file_path.parent().ok_or(format_err!(
        "Can't find parent directory for file path: {}",
        file_path.display()
    ))?;
    std::fs::create_dir_all(&parent_directory).context(format!(
        "Can't create directory: {}",
        parent_directory.display()
    ))?;

    if file_path.is_file() {
        std::fs::remove_file(&file_path)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&file_path)
        .context(format!(
            "Can't open/create file for writing: {}",
            file_path.display()
        ))?;
    file.write_all(serde_json::to_string_pretty(&review)?.as_bytes())?;
    Ok(file_path)
}

pub fn list() -> Result<Vec<review::Review>> {
    Ok(list_with_status()?
        .into_iter()
        .map(|stored| stored.review)
        .collect())
}

pub fn list_with_status() -> Result<Vec<StoredReview>> {
    let paths = common::fs::DataPaths::new()?;
    if !paths.reviews_directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_review_files(&paths.reviews_directory, &mut files)?;

    let mut reviews = Vec::new();
    for file in files {
        let reader = std::io::BufReader::new(std::fs::File::open(&file)?);
        match serde_json::from_reader::<_, review::Review>(reader) {
            Ok(mut review) => {
                review.overall_security_summary =
                    crate::review::overall_security_summary(&review)?;
                let status = if file.starts_with(&paths.pending_reviews_directory) {
                    ReviewStorageStatus::Pending
                } else {
                    ReviewStorageStatus::Submitted
                };
                reviews.push(StoredReview {
                    path: file,
                    status,
                    review,
                });
            }
            Err(err) => {
                log::warn!("Failed to parse review file {}: {}", file.display(), err);
            }
        }
    }
    Ok(reviews)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReviewStorageStatus {
    Pending,
    Submitted,
}

#[derive(Debug, Clone)]
pub struct StoredReview {
    pub path: std::path::PathBuf,
    pub status: ReviewStorageStatus,
    pub review: review::Review,
}

pub fn promote(
    review: &review::Review,
    pending_path: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    let paths = common::fs::DataPaths::new()?;
    let file_name = pending_path.file_name().ok_or(format_err!(
        "Failed to read review filename: {}",
        pending_path.display()
    ))?;
    let destination_base = &paths.reviews_directory;
    let destination_directory = get_storage_file_path(review, destination_base)?
        .parent()
        .ok_or(format_err!(
            "Failed to build destination directory for review."
        ))?
        .to_path_buf();
    std::fs::create_dir_all(&destination_directory).context(format!(
        "Can't create directory: {}",
        destination_directory.display()
    ))?;
    let destination_path = destination_directory.join(file_name);
    if destination_path.exists() {
        std::fs::remove_file(&destination_path)?;
    }
    std::fs::rename(&pending_path, &destination_path)?;
    Ok(destination_path)
}

fn collect_review_files(
    directory: &std::path::PathBuf,
    files: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name == ".ongoing" {
                    continue;
                }
            }
            collect_review_files(&path, files)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}
