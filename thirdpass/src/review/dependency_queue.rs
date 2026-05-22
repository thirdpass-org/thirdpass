use anyhow::{format_err, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

const DEPENDENCY_QUEUE_SCHEMA_VERSION: u32 = 2;

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
    /// Refresh parcel status from local review storage and select the next work item.
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

    /// Mark a queue parcel as reviewed and persist the queue.
    pub(crate) fn mark_parcel_reviewed(&mut self, queue_rank: usize) -> Result<()> {
        if set_parcel_status(
            &mut self.queue,
            queue_rank,
            DependencyQueueParcelStatus::Reviewed,
        ) {
            write_queue_atomically(&self.path, &self.queue)?;
        }
        Ok(())
    }
}

/// A selected dependency parcel ready to hand to the review command.
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
    /// One-based queue parcel rank.
    pub(crate) queue_rank: usize,
    /// Total parcel count in the queue.
    pub(crate) queue_parcel_count: usize,
    /// One-based parcel rank within this package.
    pub(crate) package_parcel_rank: usize,
    /// Number of files in the full parcel.
    pub(crate) parcel_file_count: usize,
    /// Package-relative files that still need local review coverage.
    pub(crate) target_files: Vec<String>,
}

/// Local review queue built from a project's dependency files.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueue {
    /// Queue schema version.
    pub(crate) schema_version: u32,
    /// Queue creation timestamp in Unix seconds.
    pub(crate) generated_at_unix: u64,
    /// Parcel sizing limits used to build this queue.
    pub(crate) parcel_limits: DependencyQueueParcelLimits,
    /// Project dependency snapshot used to derive this queue.
    pub(crate) source: DependencyQueueSource,
    /// Packages successfully analyzed into review parcels.
    pub(crate) packages: Vec<DependencyQueuePackageRecord>,
    /// Packages skipped while building the queue.
    pub(crate) skipped_packages: Vec<SkippedDependencyPackage>,
}

impl DependencyQueue {
    /// Count all review parcels across queued packages.
    pub(crate) fn parcel_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.parcels.len())
            .sum()
    }

    /// Count parcels marked as reviewed in this queue.
    pub(crate) fn reviewed_parcel_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|package| &package.parcels)
            .filter(|parcel| parcel.status == DependencyQueueParcelStatus::Reviewed)
            .count()
    }
}

/// Parcel sizing limits captured in a stored queue.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueParcelLimits {
    /// Maximum total line count to include in one parcel.
    pub(crate) max_lines_per_parcel: usize,
    /// Maximum number of files to include in one parcel.
    pub(crate) max_files_per_parcel: usize,
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
    /// Review parcels built for this package.
    pub(crate) parcels: Vec<DependencyQueueParcel>,
}

/// One bounded group of files to review together.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyQueueParcel {
    /// One-based rank across the whole queue.
    pub(crate) queue_rank: usize,
    /// One-based rank within this package.
    pub(crate) package_parcel_rank: usize,
    /// Current local review status for this parcel.
    pub(crate) status: DependencyQueueParcelStatus,
    /// Total line count across parcel files.
    pub(crate) total_lines: usize,
    /// Files included in this parcel.
    pub(crate) files: Vec<DependencyQueueFile>,
}

/// Local review status for one dependency queue parcel.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DependencyQueueParcelStatus {
    /// The parcel still has files without local review coverage.
    Pending,
    /// All parcel files have local review coverage.
    Reviewed,
}

/// One file included in a local dependency review parcel.
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
    /// Line count used for parcel sizing.
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
    parcel_limits: DependencyQueueParcelLimits,
    project_root: &'a str,
    dependency_files: &'a [DependencyQueueSourceFile],
    packages: &'a [DependencyQueuePackage],
}

/// Ensure a dependency review queue exists for a project dependency snapshot.
pub(crate) fn ensure_for_project(
    project_root: &Path,
    dependency_files: &[PathBuf],
    packages: &[DependencyQueuePackage],
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
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

    let queue = build_queue(
        &project_root_string,
        source_files,
        packages,
        extensions,
        &queue_id,
    )?;
    write_queue_atomically(&queue_path, &queue)?;
    Ok(StoredDependencyQueue {
        path: queue_path,
        queue,
    })
}

fn build_queue(
    project_root: &str,
    dependency_files: Vec<DependencyQueueSourceFile>,
    packages: Vec<DependencyQueuePackage>,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    queue_id: &str,
) -> Result<DependencyQueue> {
    let parcel_limits = dependency_queue_parcel_limits();
    let mut package_records = Vec::new();
    let mut skipped_packages = Vec::new();
    let mut next_queue_rank = 1usize;

    for package in &packages {
        match build_package_record(package, extensions, queue_id, next_queue_rank) {
            Ok(record) => {
                next_queue_rank += record.parcels.len();
                package_records.push(record);
            }
            Err(error) => skipped_packages.push(skipped_dependency_package(package, error)),
        }
    }

    Ok(DependencyQueue {
        schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
        generated_at_unix: now_unix_seconds()?,
        parcel_limits,
        source: DependencyQueueSource {
            project_root: project_root.to_string(),
            dependency_files,
            dependency_count: packages.len(),
        },
        packages: package_records,
        skipped_packages,
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
        let parcels = thirdpass_core::package::build_review_parcels(
            thirdpass_core::package::ReviewParcelInput {
                package: thirdpass_core::package::ReviewParcelPackage {
                    registry_host: metadata.registry_host_name.clone(),
                    package_name: package.package_name.clone(),
                    package_version: metadata.package_version.clone(),
                    package_hash: workspace_manifest.package_hash.clone(),
                },
                files,
                target_policy: extension.review_target_policy(),
            },
            review_parcel_config(queue_id, package),
        )?;

        Ok(DependencyQueuePackageRecord {
            extension_name: package.extension_name.clone(),
            registry_host: metadata.registry_host_name.clone(),
            package_name: package.package_name.clone(),
            package_version: metadata.package_version.clone(),
            package_hash: workspace_manifest.package_hash.clone(),
            human_url: metadata.human_url.clone(),
            artifact_url: metadata.artifact_url.clone(),
            parcels: queue_parcels(first_queue_rank, &parcels),
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

fn queue_parcels(
    first_queue_rank: usize,
    parcels: &[thirdpass_core::package::ReviewParcel],
) -> Vec<DependencyQueueParcel> {
    parcels
        .iter()
        .enumerate()
        .map(|(index, parcel)| DependencyQueueParcel {
            queue_rank: first_queue_rank + index,
            package_parcel_rank: parcel.package_parcel_rank,
            status: DependencyQueueParcelStatus::Pending,
            total_lines: parcel.total_lines,
            files: parcel
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
        parcel_limits: dependency_queue_parcel_limits(),
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
        for parcel in &mut package.parcels {
            let new_status = if parcel_is_covered(parcel, covered_files) {
                DependencyQueueParcelStatus::Reviewed
            } else {
                DependencyQueueParcelStatus::Pending
            };
            if parcel.status != new_status {
                parcel.status = new_status;
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
    let queue_parcel_count = queue.parcel_count();
    for package in &queue.packages {
        let package_key = package_review_key(package);
        let covered_files = coverage.get(&package_key);
        for parcel in &package.parcels {
            if parcel.status == DependencyQueueParcelStatus::Reviewed {
                continue;
            }

            let target_files = uncovered_parcel_files(parcel, covered_files);
            if target_files.is_empty() {
                continue;
            }

            return Some(DependencyQueueSelection {
                extension_name: package.extension_name.clone(),
                registry_host: package.registry_host.clone(),
                package_name: package.package_name.clone(),
                package_version: package.package_version.clone(),
                queue_rank: parcel.queue_rank,
                queue_parcel_count,
                package_parcel_rank: parcel.package_parcel_rank,
                parcel_file_count: parcel.files.len(),
                target_files,
            });
        }
    }
    None
}

fn set_parcel_status(
    queue: &mut DependencyQueue,
    queue_rank: usize,
    status: DependencyQueueParcelStatus,
) -> bool {
    for parcel in queue
        .packages
        .iter_mut()
        .flat_map(|package| &mut package.parcels)
    {
        if parcel.queue_rank == queue_rank {
            if parcel.status == status {
                return false;
            }
            parcel.status = status;
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

fn parcel_is_covered(
    parcel: &DependencyQueueParcel,
    covered_files: Option<&BTreeSet<String>>,
) -> bool {
    let Some(covered_files) = covered_files else {
        return false;
    };
    parcel
        .files
        .iter()
        .all(|file| covered_files.contains(&file.path))
}

fn uncovered_parcel_files(
    parcel: &DependencyQueueParcel,
    covered_files: Option<&BTreeSet<String>>,
) -> Vec<String> {
    parcel
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

fn dependency_queue_parcel_limits() -> DependencyQueueParcelLimits {
    DependencyQueueParcelLimits {
        max_lines_per_parcel: thirdpass_core::package::DEFAULT_REVIEW_PARCEL_MAX_LINES,
        max_files_per_parcel: thirdpass_core::package::DEFAULT_REVIEW_PARCEL_MAX_FILES,
    }
}

fn review_parcel_config(
    queue_id: &str,
    package: &DependencyQueuePackage,
) -> thirdpass_core::package::ReviewParcelConfig {
    let limits = dependency_queue_parcel_limits();
    thirdpass_core::package::ReviewParcelConfig {
        max_lines: limits.max_lines_per_parcel,
        max_files: limits.max_files_per_parcel,
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
        let mut queue = queue_with_parcels();
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

        assert_eq!(queue.reviewed_parcel_count(), 1);
        assert_eq!(selection.queue_rank, 2);
        assert_eq!(selection.queue_parcel_count, 2);
        assert_eq!(selection.package_parcel_rank, 2);
        assert_eq!(selection.parcel_file_count, 2);
        assert_eq!(selection.target_files, vec!["src/d.rs".to_string()]);
    }

    #[test]
    fn select_next_review_returns_none_when_queue_is_covered() {
        let mut queue = queue_with_parcels();
        let mut coverage = PackageReviewCoverage::new();
        coverage.insert(
            package_review_key(&queue.packages[0]),
            ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]
                .iter()
                .map(|path| path.to_string())
                .collect(),
        );

        assert!(refresh_queue_progress(&mut queue, &coverage));

        assert_eq!(queue.reviewed_parcel_count(), 2);
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
            parcel_limits: dependency_queue_parcel_limits(),
            source: DependencyQueueSource {
                project_root: "/project".to_string(),
                dependency_files: vec![source_file("/project/Cargo.toml", "hash")],
                dependency_count: 1,
            },
            packages: Vec::new(),
            skipped_packages: Vec::new(),
        }
    }

    fn queue_with_parcels() -> DependencyQueue {
        let mut queue = sample_queue();
        queue.packages = vec![DependencyQueuePackageRecord {
            extension_name: "rs".to_string(),
            registry_host: "crates.io".to_string(),
            package_name: "demo".to_string(),
            package_version: "1.0.0".to_string(),
            package_hash: "package-hash".to_string(),
            human_url: "https://crates.io/crates/demo/1.0.0".to_string(),
            artifact_url: "https://static.crates.io/crates/demo/demo-1.0.0.crate".to_string(),
            parcels: vec![
                parcel(1, 1, &["src/a.rs", "src/b.rs"]),
                parcel(2, 2, &["src/c.rs", "src/d.rs"]),
            ],
        }];
        queue
    }

    fn parcel(
        queue_rank: usize,
        package_parcel_rank: usize,
        paths: &[&str],
    ) -> DependencyQueueParcel {
        DependencyQueueParcel {
            queue_rank,
            package_parcel_rank,
            status: DependencyQueueParcelStatus::Pending,
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
