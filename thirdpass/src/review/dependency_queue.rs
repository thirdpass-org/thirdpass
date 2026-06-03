use anyhow::{format_err, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

const DEPENDENCY_QUEUE_SCHEMA_VERSION: u32 = 4;

/// One dependency package that should be considered for the local queue.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct DependencyQueuePackage {
    /// Extension name that discovered this dependency.
    pub(crate) extension_name: String,
    /// Registry host that owns this dependency.
    pub(crate) registry_host_name: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Resolved package version in the registry.
    pub(crate) package_version: String,
}

/// A stored local dependency review queue and its filesystem path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct StoredDependencyQueue {
    /// Path of the queue JSON file.
    pub(crate) path: PathBuf,
    /// Queue contents.
    pub(crate) queue: DependencyQueue,
}

impl StoredDependencyQueue {
    /// Refresh batch status from local review storage and select the next work item.
    pub(crate) fn select_next_review(
        &mut self,
        public_user_id: &str,
    ) -> Result<Option<DependencyQueueSelection>> {
        let coverage = local_review_coverage(public_user_id)?;
        let changed = refresh_queue_progress(&mut self.queue, &coverage);
        if changed {
            write_queue_atomically(&self.path, &self.queue)?;
        }
        Ok(select_next_review(&self.queue, &coverage))
    }

    /// Return the next dependency package waiting to be prepared.
    pub(crate) fn next_pending_package(&self) -> Option<&DependencyQueuePackage> {
        self.queue.pending_packages.first()
    }

    /// Download, analyze, and persist the next pending dependency package.
    pub(crate) fn prepare_next_package(
        &mut self,
        extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    ) -> Result<Option<DependencyQueuePreparation>> {
        let Some(package) = self.queue.pending_packages.first().cloned() else {
            return Ok(None);
        };

        let first_queue_rank = self.queue.batch_count() + 1;
        let preparation = match build_package_record(
            &package,
            extensions,
            &self.queue.queue_id,
            first_queue_rank,
        ) {
            Ok(record) => {
                let preparation = DependencyQueuePreparation::Prepared {
                    extension_name: record.extension_name.clone(),
                    registry_host: record.registry_host.clone(),
                    package_name: record.package_name.clone(),
                    package_version: record.package_version.clone(),
                    batch_count: record.batches.len(),
                    file_count: record.batches.iter().map(|batch| batch.files.len()).sum(),
                };
                self.queue.packages.push(record);
                preparation
            }
            Err(error) => {
                let skipped = skipped_dependency_package(&package, error);
                let preparation = DependencyQueuePreparation::Skipped {
                    extension_name: skipped.extension_name.clone(),
                    registry_host: skipped.registry_host.clone(),
                    package_name: skipped.package_name.clone(),
                    package_version: skipped.package_version.clone(),
                    reason: skipped.reason.clone(),
                };
                self.queue.skipped_packages.push(skipped);
                preparation
            }
        };

        self.queue.pending_packages.remove(0);
        write_queue_atomically(&self.path, &self.queue)?;
        Ok(Some(preparation))
    }

    /// Mark a queue batch as reviewed and persist the queue.
    pub(crate) fn mark_batch_reviewed(&mut self, queue_rank: usize) -> Result<()> {
        if set_batch_status(
            &mut self.queue,
            queue_rank,
            DependencyQueueBatchStatus::Reviewed,
        ) {
            write_queue_atomically(&self.path, &self.queue)?;
        }
        Ok(())
    }
}

/// A selected dependency batch ready to hand to the review command.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyQueueSelection {
    /// Extension name that can retrieve this package.
    pub(crate) extension_name: String,
    /// Registry host that owns this package.
    pub(crate) registry_host: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Package version in the registry.
    pub(crate) package_version: String,
    /// One-based queue batch rank.
    pub(crate) queue_rank: usize,
    /// Total batch count in the queue.
    pub(crate) queue_batch_count: usize,
    /// One-based batch rank within this package.
    pub(crate) package_batch_rank: usize,
    /// Number of files in the full batch.
    pub(crate) batch_file_count: usize,
    /// Package-relative files that still need local review coverage.
    pub(crate) target_files: Vec<String>,
}

/// Result of preparing one pending dependency package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum DependencyQueuePreparation {
    /// The package was analyzed into review batches.
    Prepared {
        /// Extension name that can retrieve this package.
        extension_name: String,
        /// Registry host that owns this package.
        registry_host: String,
        /// Package name in the registry.
        package_name: String,
        /// Package version in the registry.
        package_version: String,
        /// Number of review batches prepared for this package.
        batch_count: usize,
        /// Number of files covered by the prepared batches.
        file_count: usize,
    },
    /// The package could not be prepared and was skipped.
    Skipped {
        /// Extension name that discovered this package.
        extension_name: String,
        /// Registry host that owns this package.
        registry_host: String,
        /// Package name in the registry.
        package_name: String,
        /// Package version in the registry.
        package_version: String,
        /// Human-readable skip reason.
        reason: String,
    },
}

/// Local review queue built from a project's dependency files.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueue {
    /// Queue schema version.
    pub(crate) schema_version: u32,
    /// Queue creation timestamp in Unix seconds.
    pub(crate) generated_at_unix: u64,
    /// Batch sizing limits used to build this queue.
    pub(crate) batch_limits: DependencyQueueBatchLimits,
    /// Stable queue identifier used for package batch shuffling.
    pub(crate) queue_id: String,
    /// Project dependency snapshot used to derive this queue.
    pub(crate) source: DependencyQueueSource,
    /// Packages successfully analyzed into review batches.
    pub(crate) packages: Vec<DependencyQueuePackageRecord>,
    /// Packages discovered but not yet downloaded or analyzed.
    pub(crate) pending_packages: Vec<DependencyQueuePackage>,
    /// Packages skipped while building the queue.
    pub(crate) skipped_packages: Vec<SkippedDependencyPackage>,
}

impl DependencyQueue {
    /// Count all review batches across queued packages.
    pub(crate) fn batch_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.batches.len())
            .sum()
    }

    /// Count batches marked as reviewed in this queue.
    pub(crate) fn reviewed_batch_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|package| &package.batches)
            .filter(|batch| batch.status == DependencyQueueBatchStatus::Reviewed)
            .count()
    }

    /// Count batches that still need local review coverage.
    pub(crate) fn remaining_batch_count(&self) -> usize {
        self.batch_count()
            .saturating_sub(self.reviewed_batch_count())
    }

    /// Count dependency packages already prepared or skipped.
    pub(crate) fn prepared_package_count(&self) -> usize {
        self.packages.len() + self.skipped_packages.len()
    }

    /// Count dependency packages still waiting to be prepared.
    pub(crate) fn pending_package_count(&self) -> usize {
        self.pending_packages.len()
    }
}

/// Batch sizing limits captured in a stored queue.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueBatchLimits {
    /// Maximum total line count to include in one batch.
    pub(crate) max_lines_per_batch: usize,
    /// Maximum number of files to include in one batch.
    pub(crate) max_files_per_batch: usize,
}

/// Project files and dependency count used to derive a queue.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueSource {
    /// Absolute project root used for dependency discovery.
    pub(crate) project_root: String,
    /// Dependency files that contributed dependency candidates.
    pub(crate) dependency_files: Vec<DependencyQueueSourceFile>,
    /// Number of distinct dependency packages discovered.
    pub(crate) dependency_count: usize,
}

/// Dependency source file identity.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueSourceFile {
    /// Dependency file path.
    pub(crate) path: String,
    /// Blake3 digest of the dependency file contents.
    pub(crate) blake3: String,
}

/// One analyzed dependency package in a queue.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueuePackageRecord {
    /// Extension name that discovered this package.
    pub(crate) extension_name: String,
    /// Registry host that owns this package.
    pub(crate) registry_host: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Package version in the registry.
    pub(crate) package_version: String,
    /// Blake3 digest of the package source artifact.
    pub(crate) package_hash: String,
    /// Human-readable registry package URL.
    pub(crate) human_url: String,
    /// Source artifact download URL.
    pub(crate) artifact_url: String,
    /// Review batches built for this package.
    pub(crate) batches: Vec<DependencyQueueBatch>,
}

/// One bounded group of files to review together.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueBatch {
    /// One-based rank across the whole queue.
    pub(crate) queue_rank: usize,
    /// One-based rank within this package.
    pub(crate) package_batch_rank: usize,
    /// Current local review status for this batch.
    pub(crate) status: DependencyQueueBatchStatus,
    /// Total line count across batch files.
    pub(crate) total_lines: usize,
    /// Files included in this batch.
    pub(crate) files: Vec<DependencyQueueFile>,
}

/// Local review status for one dependency queue batch.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DependencyQueueBatchStatus {
    /// The batch still has files without local review coverage.
    Pending,
    /// All batch files have local review coverage.
    Reviewed,
}

/// One file included in a local dependency review batch.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueFile {
    /// Package-relative path.
    pub(crate) path: String,
    /// Blake3 digest of the file contents.
    pub(crate) file_hash: thirdpass_core::schema::FileHash,
    /// File size in bytes.
    pub(crate) size_bytes: u64,
    /// Lowercase extension without the leading dot, when known.
    pub(crate) extension: Option<String>,
    /// Line count used for batch sizing.
    pub(crate) line_count: usize,
    /// Stable rank among reviewable files before shuffling.
    pub(crate) file_rank: usize,
}

/// Dependency package skipped while building a local queue.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SkippedDependencyPackage {
    /// Extension name that discovered this package.
    pub(crate) extension_name: String,
    /// Registry host that owns this package.
    pub(crate) registry_host: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Package version in the registry.
    pub(crate) package_version: String,
    /// Human-readable reason the package was skipped.
    pub(crate) reason: String,
}

#[derive(Debug, Serialize)]
struct DependencyQueueKey<'a> {
    schema_version: u32,
    batch_limits: DependencyQueueBatchLimits,
    project_root: &'a str,
    dependency_files: &'a [DependencyQueueSourceFile],
    packages: &'a [DependencyQueuePackage],
}

/// Ensure a dependency review queue exists for a project dependency snapshot.
pub(crate) fn ensure_for_project(
    project_root: &Path,
    dependency_files: &[PathBuf],
    packages: &[DependencyQueuePackage],
) -> Result<StoredDependencyQueue> {
    let project_root = canonical_path(project_root)?;
    let source_files = dependency_source_files(dependency_files)?;
    let packages = sorted_packages(packages);
    let project_root_string = project_root.display().to_string();
    let queue_id = queue_id(&project_root_string, &source_files, &packages)?;
    let queue_path = queue_path(&queue_id)?;

    if queue_path.is_file() {
        let queue = read_queue(&queue_path)?;
        return Ok(StoredDependencyQueue {
            path: queue_path,
            queue,
        });
    }

    let queue = new_queue(&project_root_string, source_files, packages, &queue_id)?;
    write_queue_atomically(&queue_path, &queue)?;
    Ok(StoredDependencyQueue {
        path: queue_path,
        queue,
    })
}

fn new_queue(
    project_root: &str,
    dependency_files: Vec<DependencyQueueSourceFile>,
    packages: Vec<DependencyQueuePackage>,
    queue_id: &str,
) -> Result<DependencyQueue> {
    Ok(DependencyQueue {
        schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
        generated_at_unix: now_unix_seconds()?,
        batch_limits: dependency_queue_batch_limits(),
        queue_id: queue_id.to_string(),
        source: DependencyQueueSource {
            project_root: project_root.to_string(),
            dependency_files,
            dependency_count: packages.len(),
        },
        packages: Vec::new(),
        pending_packages: packages,
        skipped_packages: Vec::new(),
    })
}

fn build_package_record(
    package: &DependencyQueuePackage,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    queue_id: &str,
    first_queue_rank: usize,
) -> Result<DependencyQueuePackageRecord> {
    let extension = extension_for_package(package, extensions)?;
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
            review_batch_config(queue_id, package),
        )?;

        Ok(DependencyQueuePackageRecord {
            extension_name: package.extension_name.clone(),
            registry_host: metadata.registry_host_name.clone(),
            package_name: package.package_name.clone(),
            package_version: metadata.package_version.clone(),
            package_hash: workspace_manifest.package_hash.clone(),
            human_url: metadata.human_url.clone(),
            artifact_url: metadata.artifact_url.clone(),
            batches: queue_batches(first_queue_rank, &batches),
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
    package: &DependencyQueuePackage,
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
    package: &DependencyQueuePackage,
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

fn queue_batches(
    first_queue_rank: usize,
    batches: &[thirdpass_core::package::ReviewBatch],
) -> Vec<DependencyQueueBatch> {
    batches
        .iter()
        .enumerate()
        .map(|(index, batch)| DependencyQueueBatch {
            queue_rank: first_queue_rank + index,
            package_batch_rank: batch.package_batch_rank,
            status: DependencyQueueBatchStatus::Pending,
            total_lines: batch.total_lines,
            files: batch
                .files
                .iter()
                .map(|file| DependencyQueueFile {
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

fn dependency_source_files(paths: &[PathBuf]) -> Result<Vec<DependencyQueueSourceFile>> {
    let mut source_files = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for path in paths {
        let path = canonical_path(path)?;
        if !seen.insert(path.clone()) {
            continue;
        }
        let blake3 = thirdpass_core::package::file_blake3_digest(&path)
            .context(format!("can't hash dependency file: {}", path.display()))?;
        source_files.push(DependencyQueueSourceFile {
            path: path.display().to_string(),
            blake3,
        });
    }

    source_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(source_files)
}

fn sorted_packages(packages: &[DependencyQueuePackage]) -> Vec<DependencyQueuePackage> {
    packages
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn queue_path(queue_id: &str) -> Result<PathBuf> {
    let data_paths = crate::common::fs::DataPaths::new()?;
    Ok(data_paths
        .dependency_queues_directory
        .join("projects")
        .join(queue_id)
        .join("queue.json"))
}

fn queue_id(
    project_root: &str,
    dependency_files: &[DependencyQueueSourceFile],
    packages: &[DependencyQueuePackage],
) -> Result<String> {
    let key = DependencyQueueKey {
        schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
        batch_limits: dependency_queue_batch_limits(),
        project_root,
        dependency_files,
        packages,
    };
    let bytes = serde_json::to_vec(&key)?;
    Ok(blake3::hash(&bytes).to_hex().as_str().to_string())
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct PackageReviewKey {
    registry_host: String,
    package_name: String,
    package_version: String,
    package_hash: String,
}

type PackageReviewCoverage = BTreeMap<PackageReviewKey, BTreeSet<String>>;

fn local_review_coverage(public_user_id: &str) -> Result<PackageReviewCoverage> {
    let mut coverage = PackageReviewCoverage::new();
    for stored in crate::review::fs::list_with_status()? {
        let review = stored.review;
        if review.reviewer_details.public_user_id != public_user_id {
            continue;
        }

        for registry in &review.package.registries {
            let key = PackageReviewKey {
                registry_host: registry.host_name.clone(),
                package_name: review.package.name.clone(),
                package_version: review.package.version.clone(),
                package_hash: review.package.package_hash.clone(),
            };
            let package_coverage = coverage.entry(key).or_default();
            for target in &review.targets {
                package_coverage.insert(package_relative_path_string(&target.file_path));
            }
        }
    }
    Ok(coverage)
}

fn refresh_queue_progress(queue: &mut DependencyQueue, coverage: &PackageReviewCoverage) -> bool {
    let mut changed = false;
    for package in &mut queue.packages {
        let package_key = package_review_key(package);
        let covered_files = coverage.get(&package_key);
        for batch in &mut package.batches {
            let new_status = if batch_is_covered(batch, covered_files) {
                DependencyQueueBatchStatus::Reviewed
            } else {
                DependencyQueueBatchStatus::Pending
            };
            if batch.status != new_status {
                batch.status = new_status;
                changed = true;
            }
        }
    }
    changed
}

fn select_next_review(
    queue: &DependencyQueue,
    coverage: &PackageReviewCoverage,
) -> Option<DependencyQueueSelection> {
    let queue_batch_count = queue.batch_count();
    for package in &queue.packages {
        let package_key = package_review_key(package);
        let covered_files = coverage.get(&package_key);
        for batch in &package.batches {
            if batch.status == DependencyQueueBatchStatus::Reviewed {
                continue;
            }

            let target_files = uncovered_batch_files(batch, covered_files);
            if target_files.is_empty() {
                continue;
            }

            return Some(DependencyQueueSelection {
                extension_name: package.extension_name.clone(),
                registry_host: package.registry_host.clone(),
                package_name: package.package_name.clone(),
                package_version: package.package_version.clone(),
                queue_rank: batch.queue_rank,
                queue_batch_count,
                package_batch_rank: batch.package_batch_rank,
                batch_file_count: batch.files.len(),
                target_files,
            });
        }
    }
    None
}

fn set_batch_status(
    queue: &mut DependencyQueue,
    queue_rank: usize,
    status: DependencyQueueBatchStatus,
) -> bool {
    for batch in queue
        .packages
        .iter_mut()
        .flat_map(|package| &mut package.batches)
    {
        if batch.queue_rank == queue_rank {
            if batch.status == status {
                return false;
            }
            batch.status = status;
            return true;
        }
    }
    false
}

fn package_review_key(package: &DependencyQueuePackageRecord) -> PackageReviewKey {
    PackageReviewKey {
        registry_host: package.registry_host.clone(),
        package_name: package.package_name.clone(),
        package_version: package.package_version.clone(),
        package_hash: package.package_hash.clone(),
    }
}

fn batch_is_covered(
    batch: &DependencyQueueBatch,
    covered_files: Option<&BTreeSet<String>>,
) -> bool {
    let Some(covered_files) = covered_files else {
        return false;
    };
    batch
        .files
        .iter()
        .all(|file| covered_files.contains(&file.path))
}

fn uncovered_batch_files(
    batch: &DependencyQueueBatch,
    covered_files: Option<&BTreeSet<String>>,
) -> Vec<String> {
    batch
        .files
        .iter()
        .filter(|file| {
            covered_files
                .map(|covered_files| !covered_files.contains(&file.path))
                .unwrap_or(true)
        })
        .map(|file| file.path.clone())
        .collect()
}

fn dependency_queue_batch_limits() -> DependencyQueueBatchLimits {
    DependencyQueueBatchLimits {
        max_lines_per_batch: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_LINES,
        max_files_per_batch: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_FILES,
    }
}

fn review_batch_config(
    queue_id: &str,
    package: &DependencyQueuePackage,
) -> thirdpass_core::package::ReviewBatchConfig {
    let limits = dependency_queue_batch_limits();
    thirdpass_core::package::ReviewBatchConfig {
        max_lines: limits.max_lines_per_batch,
        max_files: limits.max_files_per_batch,
        shuffle_seed: Some(package_shuffle_seed(queue_id, package)),
    }
}

fn package_shuffle_seed(queue_id: &str, package: &DependencyQueuePackage) -> u64 {
    let material = format!(
        "{}\0{}\0{}\0{}\0{}",
        queue_id,
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

fn skipped_dependency_package(
    package: &DependencyQueuePackage,
    error: anyhow::Error,
) -> SkippedDependencyPackage {
    SkippedDependencyPackage {
        extension_name: package.extension_name.clone(),
        registry_host: package.registry_host_name.clone(),
        package_name: package.package_name.clone(),
        package_version: package.package_version.clone(),
        reason: error.to_string(),
    }
}

fn read_queue(path: &Path) -> Result<DependencyQueue> {
    let file = std::fs::File::open(path).context(format!(
        "can't open dependency review queue: {}",
        path.display()
    ))?;
    serde_json::from_reader(std::io::BufReader::new(file)).context(format!(
        "can't parse dependency review queue: {}",
        path.display()
    ))
}

fn write_queue_atomically(path: &Path, queue: &DependencyQueue) -> Result<()> {
    let parent = path.parent().ok_or(format_err!(
        "can't find parent directory for dependency review queue: {}",
        path.display()
    ))?;
    std::fs::create_dir_all(parent).context(format!(
        "can't create dependency review queue directory: {}",
        parent.display()
    ))?;

    let mut temp_file = tempfile::NamedTempFile::new_in(parent).context(format!(
        "can't create temporary dependency review queue in: {}",
        parent.display()
    ))?;
    {
        let mut writer = std::io::BufWriter::new(temp_file.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, queue).context(format!(
            "can't serialize dependency review queue: {}",
            path.display()
        ))?;
        writer.write_all(b"\n")?;
        writer.flush().context(format!(
            "can't flush temporary dependency review queue: {}",
            path.display()
        ))?;
    }
    temp_file.as_file().sync_all().context(format!(
        "can't sync temporary dependency review queue: {}",
        path.display()
    ))?;
    temp_file
        .persist(path)
        .map_err(|error| error.error)
        .context(format!(
            "can't replace dependency review queue atomically: {}",
            path.display()
        ))?;
    sync_parent_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(directory: &Path) -> Result<()> {
    std::fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .context(format!(
            "can't sync dependency review queue directory: {}",
            directory.display()
        ))
}

#[cfg(not(unix))]
fn sync_parent_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .context(format!("can't canonicalize path: {}", path.display()))
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

fn now_unix_seconds() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_id_is_stable_when_packages_are_sorted() -> Result<()> {
        let files = vec![source_file("/project/Cargo.toml", "source-hash")];
        let mut left_packages = vec![
            package("rs", "crates.io", "serde", "1.0.0"),
            package("js", "npmjs.com", "left-pad", "1.3.0"),
        ];
        let mut right_packages = left_packages.clone();
        right_packages.reverse();

        left_packages = sorted_packages(&left_packages);
        right_packages = sorted_packages(&right_packages);

        assert_eq!(
            queue_id("/project", &files, &left_packages)?,
            queue_id("/project", &files, &right_packages)?
        );
        Ok(())
    }

    #[test]
    fn queue_id_changes_when_dependency_snapshot_changes() -> Result<()> {
        let packages = vec![package("rs", "crates.io", "serde", "1.0.0")];

        let first = queue_id(
            "/project",
            &[source_file("/project/Cargo.toml", "first-hash")],
            &packages,
        )?;
        let second = queue_id(
            "/project",
            &[source_file("/project/Cargo.toml", "second-hash")],
            &packages,
        )?;

        assert_ne!(first, second);
        Ok(())
    }

    #[test]
    fn new_queue_keeps_packages_pending() -> Result<()> {
        let packages = vec![
            package("rs", "crates.io", "serde", "1.0.0"),
            package("js", "npmjs.com", "left-pad", "1.3.0"),
        ];

        let queue = new_queue(
            "/project",
            vec![source_file("/project/Cargo.lock", "source-hash")],
            packages.clone(),
            "queue-id",
        )?;

        assert_eq!(queue.queue_id, "queue-id");
        assert_eq!(queue.source.dependency_count, 2);
        assert_eq!(queue.packages, Vec::new());
        assert_eq!(queue.pending_packages, packages);
        assert_eq!(queue.skipped_packages, Vec::new());
        Ok(())
    }

    #[test]
    fn prepare_next_package_skips_missing_extension_without_other_packages() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let mut queue = sample_queue();
        queue.pending_packages = vec![package("rs", "crates.io", "serde", "1.0.0")];
        queue.source.dependency_count = 1;
        let mut stored = StoredDependencyQueue {
            path: tmp.path().join("queue.json"),
            queue,
        };
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> = Vec::new();

        let preparation = stored
            .prepare_next_package(&extensions)?
            .expect("package should be skipped");

        assert_eq!(
            preparation,
            DependencyQueuePreparation::Skipped {
                extension_name: "rs".to_string(),
                registry_host: "crates.io".to_string(),
                package_name: "serde".to_string(),
                package_version: "1.0.0".to_string(),
                reason: "extension 'rs' is not enabled".to_string(),
            }
        );
        assert_eq!(stored.queue.pending_package_count(), 0);
        assert_eq!(stored.queue.skipped_packages.len(), 1);
        assert_eq!(stored.queue.packages.len(), 0);
        assert_eq!(read_queue(&stored.path)?, stored.queue);
        Ok(())
    }

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

    #[test]
    fn write_queue_replaces_queue_atomically() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("queue.json");
        let mut queue = sample_queue();

        write_queue_atomically(&path, &queue)?;
        let stored = read_queue(&path)?;
        assert_eq!(stored, queue);

        queue.source.dependency_count = 2;
        write_queue_atomically(&path, &queue)?;
        let stored = read_queue(&path)?;
        assert_eq!(stored.source.dependency_count, 2);
        Ok(())
    }

    #[test]
    fn select_next_review_skips_covered_files() {
        let mut queue = queue_with_batches();
        let mut coverage = PackageReviewCoverage::new();
        coverage.insert(
            package_review_key(&queue.packages[0]),
            ["src/a.rs", "src/b.rs", "src/c.rs"]
                .iter()
                .map(|path| path.to_string())
                .collect(),
        );

        assert!(refresh_queue_progress(&mut queue, &coverage));
        let selection = select_next_review(&queue, &coverage)
            .expect("partially covered queue should still select work");

        assert_eq!(queue.reviewed_batch_count(), 1);
        assert_eq!(queue.remaining_batch_count(), 1);
        assert_eq!(selection.queue_rank, 2);
        assert_eq!(selection.queue_batch_count, 2);
        assert_eq!(selection.package_batch_rank, 2);
        assert_eq!(selection.batch_file_count, 2);
        assert_eq!(selection.target_files, vec!["src/d.rs".to_string()]);
    }

    #[test]
    fn select_next_review_returns_none_when_queue_is_covered() {
        let mut queue = queue_with_batches();
        let mut coverage = PackageReviewCoverage::new();
        coverage.insert(
            package_review_key(&queue.packages[0]),
            ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]
                .iter()
                .map(|path| path.to_string())
                .collect(),
        );

        assert!(refresh_queue_progress(&mut queue, &coverage));

        assert_eq!(queue.reviewed_batch_count(), 2);
        assert_eq!(queue.remaining_batch_count(), 0);
        assert_eq!(select_next_review(&queue, &coverage), None);
    }

    fn source_file(path: &str, blake3: &str) -> DependencyQueueSourceFile {
        DependencyQueueSourceFile {
            path: path.to_string(),
            blake3: blake3.to_string(),
        }
    }

    fn package(
        extension_name: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> DependencyQueuePackage {
        DependencyQueuePackage {
            extension_name: extension_name.to_string(),
            registry_host_name: registry_host_name.to_string(),
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
        }
    }

    fn sample_queue() -> DependencyQueue {
        DependencyQueue {
            schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
            generated_at_unix: 1,
            batch_limits: dependency_queue_batch_limits(),
            queue_id: "queue-id".to_string(),
            source: DependencyQueueSource {
                project_root: "/project".to_string(),
                dependency_files: vec![source_file("/project/Cargo.toml", "hash")],
                dependency_count: 1,
            },
            packages: Vec::new(),
            pending_packages: Vec::new(),
            skipped_packages: Vec::new(),
        }
    }

    fn queue_with_batches() -> DependencyQueue {
        let mut queue = sample_queue();
        queue.packages = vec![DependencyQueuePackageRecord {
            extension_name: "rs".to_string(),
            registry_host: "crates.io".to_string(),
            package_name: "demo".to_string(),
            package_version: "1.0.0".to_string(),
            package_hash: "package-hash".to_string(),
            human_url: "https://crates.io/crates/demo/1.0.0".to_string(),
            artifact_url: "https://static.crates.io/crates/demo/demo-1.0.0.crate".to_string(),
            batches: vec![
                batch(1, 1, &["src/a.rs", "src/b.rs"]),
                batch(2, 2, &["src/c.rs", "src/d.rs"]),
            ],
        }];
        queue
    }

    fn batch(queue_rank: usize, package_batch_rank: usize, paths: &[&str]) -> DependencyQueueBatch {
        DependencyQueueBatch {
            queue_rank,
            package_batch_rank,
            status: DependencyQueueBatchStatus::Pending,
            total_lines: paths.len(),
            files: paths
                .iter()
                .enumerate()
                .map(|(index, path)| DependencyQueueFile {
                    path: path.to_string(),
                    file_hash: thirdpass_core::schema::FileHash::blake3(format!(
                        "file-hash-{index}"
                    )),
                    size_bytes: 10,
                    extension: Some("rs".to_string()),
                    line_count: 1,
                    file_rank: index + 1,
                })
                .collect(),
        }
    }
}
