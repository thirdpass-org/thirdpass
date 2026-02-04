use anyhow::{format_err, Context, Result};
use std::io::Write;

use crate::common;
use crate::review;

static REVIEW_FILE_NAME: &str = "review.json";
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

fn get_storage_file_path(review: &review::Review) -> Result<std::path::PathBuf> {
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

    let paths = common::fs::DataPaths::new()?;
    let package_specific_directory = paths.reviews_directory.join(review_directory_path);
    Ok(package_specific_directory.join(review_file_name(review)))
}

fn review_file_name(review: &review::Review) -> std::path::PathBuf {
    if review.metadata.reviewer_uuid.is_empty() {
        return std::path::PathBuf::from(REVIEW_FILE_NAME);
    }
    std::path::PathBuf::from(format!(
        "{}{}.json",
        REVIEW_FILE_PREFIX, review.metadata.reviewer_uuid
    ))
}

/// Store a review.
pub fn add(review: &review::Review) -> Result<()> {
    let file_path = get_storage_file_path(&review)?;
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
    Ok(())
}
