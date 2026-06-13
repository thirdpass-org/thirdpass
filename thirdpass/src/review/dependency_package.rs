use anyhow::{format_err, Context, Result};
use std::path::Path;

use crate::review::dependency_plan::{
    DependencyReviewBatch, DependencyReviewBatchStatus, DependencyReviewFile,
    DependencyReviewPackage, DependencyReviewPackageRecord,
};

/// Download and analyze a dependency package with its configured extension.
pub(crate) fn build_package_record(
    package: &DependencyReviewPackage,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    snapshot_id: &str,
    first_plan_rank: usize,
) -> Result<DependencyReviewPackageRecord> {
    let extension = extension_for_package(package, extensions)?;
    build_package_record_with_extension(package, extension, snapshot_id, first_plan_rank)
}

/// Download and analyze one dependency package with an already selected extension.
pub(crate) fn package_record_for_extension(
    package: &DependencyReviewPackage,
    extension: &dyn thirdpass_core::extension::Extension,
) -> Result<DependencyReviewPackageRecord> {
    build_package_record_with_extension(package, extension, "check", 1)
}

fn build_package_record_with_extension(
    package: &DependencyReviewPackage,
    extension: &dyn thirdpass_core::extension::Extension,
    snapshot_id: &str,
    first_plan_rank: usize,
) -> Result<DependencyReviewPackageRecord> {
    let metadata = primary_metadata_for_package(extension, package)?;
    let artifact_url = url::Url::parse(&metadata.artifact_url).context(format!(
        "can't parse artifact URL for {}@{}",
        package.package_name, package.package_version
    ))?;

    let workspace_manifest = crate::review::workspace::ensure(
        &package.package_name,
        &metadata.package_version,
        &metadata.registry_host_name,
        &artifact_url,
    )?;
    let result = (|| {
        let analysis = crate::review::workspace::analyse(&workspace_manifest.workspace_path)?;
        let files = collect_reviewable_files(&workspace_manifest.workspace_path, &analysis)?;
        let batches = thirdpass_core::package::build_review_batches(
            thirdpass_core::package::ReviewBatchInput {
                package: thirdpass_core::package::ReviewBatchPackage {
                    registry_host: metadata.registry_host_name.clone(),
                    package_name: package.package_name.clone(),
                    package_version: metadata.package_version.clone(),
                    package_hash: workspace_manifest.package_hash.clone(),
                },
                files,
                target_policy: extension.review_target_policy(),
            },
            review_batch_config(snapshot_id, package),
        )?;

        Ok(DependencyReviewPackageRecord {
            extension_name: package.extension_name.clone(),
            registry_host: metadata.registry_host_name.clone(),
            package_name: package.package_name.clone(),
            package_version: metadata.package_version.clone(),
            package_hash: workspace_manifest.package_hash.clone(),
            human_url: metadata.human_url.clone(),
            artifact_url: metadata.artifact_url.clone(),
            batches: plan_batches(first_plan_rank, &batches),
        })
    })();
    let remove_result = crate::review::workspace::remove(&workspace_manifest);

    match (result, remove_result) {
        (Ok(record), Ok(())) => Ok(record),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

fn extension_for_package<'a>(
    package: &DependencyReviewPackage,
    extensions: &'a [Box<dyn thirdpass_core::extension::Extension>],
) -> Result<&'a dyn thirdpass_core::extension::Extension> {
    extensions
        .iter()
        .find(|extension| extension.name() == package.extension_name)
        .map(|extension| extension.as_ref())
        .ok_or(format_err!(
            "extension '{}' is not enabled",
            package.extension_name
        ))
}

fn primary_metadata_for_package(
    extension: &dyn thirdpass_core::extension::Extension,
    package: &DependencyReviewPackage,
) -> Result<thirdpass_core::extension::RegistryPackageMetadata> {
    let version = Some(package.package_version.as_str());
    let metadata = extension.registries_package_metadata(&package.package_name, &version)?;
    let mut matching_metadata = metadata
        .into_iter()
        .filter(|metadata| metadata.registry_host_name == package.registry_host_name)
        .collect::<Vec<_>>();

    if matching_metadata.is_empty() {
        return Err(format_err!(
            "registry metadata did not include {}",
            package.registry_host_name
        ));
    }

    if let Some(index) = matching_metadata
        .iter()
        .position(|metadata| metadata.is_primary)
    {
        return Ok(matching_metadata.remove(index));
    }

    if matching_metadata.len() == 1 {
        return Ok(matching_metadata.remove(0));
    }

    Err(format_err!(
        "registry metadata for {}@{} did not identify one primary result",
        package.package_name,
        package.package_version
    ))
}

fn collect_reviewable_files(
    workspace_path: &Path,
    analysis: &thirdpass_core::package::Analysis,
) -> Result<Vec<thirdpass_core::package::ReviewableFile>> {
    let mut files = Vec::new();
    visit_workspace_files(workspace_path, workspace_path, analysis, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn visit_workspace_files(
    workspace_path: &Path,
    directory: &Path,
    analysis: &thirdpass_core::package::Analysis,
    files: &mut Vec<thirdpass_core::package::ReviewableFile>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory).context(format!(
        "can't read workspace directory: {}",
        directory.display()
    ))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_workspace_files(workspace_path, &path, analysis, files)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let metadata = entry.metadata()?;
        let relative_path = path.strip_prefix(workspace_path)?.to_path_buf();
        let line_count = reviewable_file_line_count(
            &path,
            analysis.get(&relative_path).and_then(|entry| {
                if matches!(entry.path_type, thirdpass_core::package::PathType::File) {
                    Some(entry.line_count)
                } else {
                    None
                }
            }),
        )?;
        let hash = thirdpass_core::package::file_blake3_digest(&path)?;
        files.push(thirdpass_core::package::ReviewableFile {
            path: package_relative_path_string(&relative_path),
            file_hash: thirdpass_core::schema::FileHash::blake3(hash),
            size_bytes: metadata.len(),
            extension: relative_path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase()),
            line_count,
        });
    }
    Ok(())
}

fn reviewable_file_line_count(
    path: &Path,
    analysis_line_count: Option<usize>,
) -> Result<Option<usize>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(
            analysis_line_count.unwrap_or_else(|| contents.lines().count()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(error).context(format!("can't read workspace file: {}", path.display())),
    }
}

fn plan_batches(
    first_plan_rank: usize,
    batches: &[thirdpass_core::package::ReviewBatch],
) -> Vec<DependencyReviewBatch> {
    batches
        .iter()
        .enumerate()
        .map(|(index, batch)| DependencyReviewBatch {
            plan_rank: first_plan_rank + index,
            package_batch_rank: batch.package_batch_rank,
            status: DependencyReviewBatchStatus::Pending,
            total_lines: batch.total_lines,
            files: batch
                .files
                .iter()
                .map(|file| DependencyReviewFile {
                    path: file.path.clone(),
                    file_hash: file.file_hash.clone(),
                    size_bytes: file.size_bytes,
                    extension: file.extension.clone(),
                    line_count: file.line_count,
                    file_rank: file.file_rank,
                })
                .collect(),
        })
        .collect()
}

fn review_batch_config(
    snapshot_id: &str,
    package: &DependencyReviewPackage,
) -> thirdpass_core::package::ReviewBatchConfig {
    thirdpass_core::package::ReviewBatchConfig {
        max_lines: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_LINES,
        max_files: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_FILES,
        shuffle_seed: Some(package_shuffle_seed(snapshot_id, package)),
    }
}

fn package_shuffle_seed(snapshot_id: &str, package: &DependencyReviewPackage) -> u64 {
    let material = format!(
        "{}\0{}\0{}\0{}\0{}",
        snapshot_id,
        package.extension_name,
        package.registry_host_name,
        package.package_name,
        package.package_version
    );
    let hash = blake3::hash(material.as_bytes());
    let mut seed_bytes = [0u8; 8];
    seed_bytes.copy_from_slice(&hash.as_bytes()[0..8]);
    u64::from_le_bytes(seed_bytes)
}

fn package_relative_path_string(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return ".".to_string();
    }

    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_reviewable_files_marks_utf8_and_binary_files() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let workspace = tmp.path();
        std::fs::create_dir_all(workspace.join("src"))?;
        std::fs::write(workspace.join("LICENSE"), "one\ntwo\n")?;
        std::fs::write(workspace.join("src/lib.rs"), "fn main() {}\n")?;
        std::fs::write(workspace.join("logo.png"), [0xff, 0xfe, 0xfd])?;

        let files = collect_reviewable_files(workspace, &thirdpass_core::package::Analysis::new())?;
        let line_counts = files
            .iter()
            .map(|file| (file.path.as_str(), file.line_count))
            .collect::<Vec<_>>();

        assert_eq!(
            line_counts,
            vec![
                ("LICENSE", Some(2)),
                ("logo.png", None),
                ("src/lib.rs", Some(1)),
            ]
        );
        Ok(())
    }
}
