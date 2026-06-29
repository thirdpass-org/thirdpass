use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::review::{dependency_package, project};

const DEPENDENCY_REVIEW_PLAN_VERSION: u32 = 1;

/// One dependency package that should be considered for local review.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct DependencyReviewPackage {
    /// Extension name that discovered this dependency.
    pub(crate) extension_name: String,
    /// Registry host that owns this dependency.
    pub(crate) registry_host_name: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Resolved package version in the registry.
    pub(crate) package_version: String,
}

impl DependencyReviewPlan {
    /// Refresh batch status from local review storage and select the next work item.
    pub(crate) fn select_next_review(
        &mut self,
        public_user_id: &str,
    ) -> Result<Option<DependencyReviewSelection>> {
        let coverage =
            dependency_review_coverage(public_user_id, Path::new(&self.source.project_root))?;
        refresh_plan_progress(self, &coverage);
        Ok(select_next_review(self, &coverage))
    }

    /// Summarize the supplied project reviews against prepared packages.
    pub(crate) fn project_review_summary_for_reviews(
        &self,
        project_reviews: &[crate::review::Review],
    ) -> DependencyProjectReviewSummary {
        project_review_summary(self, project_reviews)
    }

    /// Return the next dependency package waiting to be prepared.
    pub(crate) fn next_pending_package(&self) -> Option<&DependencyReviewPackage> {
        self.pending_packages.first()
    }

    /// Download and analyze the next pending dependency package.
    pub(crate) fn prepare_next_package(
        &mut self,
        extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    ) -> Result<Option<DependencyReviewPreparation>> {
        let Some(package) = self.pending_packages.first().cloned() else {
            return Ok(None);
        };

        let first_plan_rank = self.batch_count() + 1;
        let preparation = match dependency_package::build_package_record(
            &package,
            extensions,
            &self.snapshot_id,
            first_plan_rank,
        ) {
            Ok(record) => {
                let preparation = DependencyReviewPreparation::Prepared {
                    extension_name: record.extension_name.clone(),
                    registry_host: record.registry_host.clone(),
                    package_name: record.package_name.clone(),
                    package_version: record.package_version.clone(),
                    batch_count: record.batches.len(),
                    file_count: record.batches.iter().map(|batch| batch.files.len()).sum(),
                };
                self.packages.push(record);
                preparation
            }
            Err(error) => {
                let skipped = skipped_dependency_package(&package, error);
                let preparation = DependencyReviewPreparation::Skipped {
                    extension_name: skipped.extension_name.clone(),
                    registry_host: skipped.registry_host.clone(),
                    package_name: skipped.package_name.clone(),
                    package_version: skipped.package_version.clone(),
                    reason: skipped.reason.clone(),
                };
                self.skipped_packages.push(skipped);
                preparation
            }
        };

        self.pending_packages.remove(0);
        Ok(Some(preparation))
    }

    /// Mark a dependency batch as reviewed for this command run.
    pub(crate) fn mark_batch_reviewed(&mut self, plan_rank: usize) -> Result<()> {
        let _ = set_batch_status(self, plan_rank, DependencyReviewBatchStatus::Reviewed);
        Ok(())
    }
}

/// Committed project review status for prepared dependency packages.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct DependencyProjectReviewSummary {
    /// Committed project reviews that match current package and file hashes.
    pub(crate) matching_reviews: usize,
    /// Files skipped because committed project reviews already cover them.
    pub(crate) covered_files: usize,
    /// Committed project reviews for current packages that do not match.
    pub(crate) mismatched_reviews: usize,
}

impl DependencyProjectReviewSummary {
    /// Return true when there is nothing useful to report.
    pub(crate) fn is_empty(&self) -> bool {
        self.matching_reviews == 0 && self.covered_files == 0 && self.mismatched_reviews == 0
    }
}

/// A selected dependency batch ready to hand to the review command.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewSelection {
    /// Extension name that can retrieve this package.
    pub(crate) extension_name: String,
    /// Registry host that owns this package.
    pub(crate) registry_host: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Package version in the registry.
    pub(crate) package_version: String,
    /// One-based plan batch rank.
    pub(crate) plan_rank: usize,
    /// Total batch count in the plan.
    pub(crate) plan_batch_count: usize,
    /// One-based batch rank within this package.
    pub(crate) package_batch_rank: usize,
    /// Number of files in the full batch.
    pub(crate) batch_file_count: usize,
    /// Package-relative files that still need local review coverage.
    pub(crate) target_files: Vec<String>,
}

/// Result of preparing one pending dependency package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum DependencyReviewPreparation {
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

/// Local review plan built from a project's dependency files.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewPlan {
    /// Plan schema version.
    pub(crate) schema_version: u32,
    /// Plan creation timestamp in Unix seconds.
    pub(crate) generated_at_unix: u64,
    /// Batch sizing limits used to build this plan.
    pub(crate) batch_limits: DependencyReviewBatchLimits,
    /// Stable dependency snapshot identifier used for package batch shuffling.
    pub(crate) snapshot_id: String,
    /// Project dependency snapshot used to derive this plan.
    pub(crate) source: DependencyReviewSource,
    /// Packages successfully analyzed into review batches.
    pub(crate) packages: Vec<DependencyReviewPackageRecord>,
    /// Packages discovered but not yet downloaded or analyzed.
    pub(crate) pending_packages: Vec<DependencyReviewPackage>,
    /// Packages skipped while building the plan.
    pub(crate) skipped_packages: Vec<SkippedDependencyReviewPackage>,
}

impl DependencyReviewPlan {
    /// Count all review batches across planned packages.
    pub(crate) fn batch_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.batches.len())
            .sum()
    }

    /// Count batches marked as reviewed in this plan.
    pub(crate) fn reviewed_batch_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|package| &package.batches)
            .filter(|batch| batch.status == DependencyReviewBatchStatus::Reviewed)
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

/// Batch sizing limits captured in a dependency review plan.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub(crate) struct DependencyReviewBatchLimits {
    /// Maximum total line count to include in one batch.
    pub(crate) max_lines_per_batch: usize,
    /// Maximum number of files to include in one batch.
    pub(crate) max_files_per_batch: usize,
}

/// Project files and dependency count used to derive a dependency review plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewSource {
    /// Stable identifier for this dependency snapshot.
    pub(crate) snapshot_id: String,
    /// Absolute project root used for dependency discovery.
    pub(crate) project_root: String,
    /// Dependency files that contributed dependency candidates.
    pub(crate) dependency_files: Vec<DependencyReviewSourceFile>,
    /// Number of distinct dependency packages discovered.
    pub(crate) dependency_count: usize,
}

/// Dependency source file identity.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub(crate) struct DependencyReviewSourceFile {
    /// Dependency file path.
    pub(crate) path: String,
    /// Blake3 digest of the dependency file contents.
    pub(crate) blake3: String,
}

/// One analyzed dependency package in a review plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewPackageRecord {
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
    pub(crate) batches: Vec<DependencyReviewBatch>,
}

/// One bounded group of files to review together.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewBatch {
    /// One-based rank across the whole plan.
    pub(crate) plan_rank: usize,
    /// One-based rank within this package.
    pub(crate) package_batch_rank: usize,
    /// Current local review status for this batch.
    pub(crate) status: DependencyReviewBatchStatus,
    /// Total line count across batch files.
    pub(crate) total_lines: usize,
    /// Files included in this batch.
    pub(crate) files: Vec<DependencyReviewFile>,
}

/// Local review status for one dependency review batch.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DependencyReviewBatchStatus {
    /// The batch still has files without local review coverage.
    Pending,
    /// All batch files have local review coverage.
    Reviewed,
}

/// One file included in a local dependency review batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewFile {
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

/// Dependency package skipped while building a local review plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct SkippedDependencyReviewPackage {
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
struct DependencySnapshotKey<'a> {
    schema_version: u32,
    batch_limits: DependencyReviewBatchLimits,
    project_root: &'a str,
    dependency_files: &'a [DependencyReviewSourceFile],
    packages: &'a [DependencyReviewPackage],
}

/// Build a dependency review plan for the current project dependency snapshot.
pub(crate) fn plan_for_project(
    project_root: &Path,
    dependency_files: &[PathBuf],
    packages: &[DependencyReviewPackage],
) -> Result<DependencyReviewPlan> {
    let project_root = canonical_path(project_root)?;
    let source_files = dependency_source_files(dependency_files)?;
    let packages = sorted_packages(packages);
    let project_root_string = project_root.display().to_string();
    let snapshot_id = dependency_snapshot_id(&project_root_string, &source_files, &packages)?;
    new_plan(&project_root_string, source_files, packages, &snapshot_id)
}

fn new_plan(
    project_root: &str,
    dependency_files: Vec<DependencyReviewSourceFile>,
    packages: Vec<DependencyReviewPackage>,
    snapshot_id: &str,
) -> Result<DependencyReviewPlan> {
    Ok(DependencyReviewPlan {
        schema_version: DEPENDENCY_REVIEW_PLAN_VERSION,
        generated_at_unix: now_unix_seconds()?,
        batch_limits: dependency_plan_batch_limits(),
        snapshot_id: snapshot_id.to_string(),
        source: DependencyReviewSource {
            snapshot_id: snapshot_id.to_string(),
            project_root: project_root.to_string(),
            dependency_files,
            dependency_count: packages.len(),
        },
        packages: Vec::new(),
        pending_packages: packages,
        skipped_packages: Vec::new(),
    })
}

fn dependency_source_files(paths: &[PathBuf]) -> Result<Vec<DependencyReviewSourceFile>> {
    let mut source_files = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for path in paths {
        let path = canonical_path(path)?;
        if !seen.insert(path.clone()) {
            continue;
        }
        let blake3 = thirdpass_core::package::file_blake3_digest(&path)
            .context(format!("can't hash dependency file: {}", path.display()))?;
        source_files.push(DependencyReviewSourceFile {
            path: path.display().to_string(),
            blake3,
        });
    }

    source_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(source_files)
}

fn sorted_packages(packages: &[DependencyReviewPackage]) -> Vec<DependencyReviewPackage> {
    packages
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn dependency_snapshot_id(
    project_root: &str,
    dependency_files: &[DependencyReviewSourceFile],
    packages: &[DependencyReviewPackage],
) -> Result<String> {
    let key = DependencySnapshotKey {
        schema_version: DEPENDENCY_REVIEW_PLAN_VERSION,
        batch_limits: dependency_plan_batch_limits(),
        project_root,
        dependency_files,
        packages,
    };
    let bytes = serde_json::to_vec(&key)?;
    Ok(blake3::hash(&bytes).to_hex().as_str().to_string())
}

fn dependency_review_coverage(
    public_user_id: &str,
    project_root: &Path,
) -> Result<project::ProjectReviewCoverage> {
    let mut coverage = local_review_coverage(public_user_id)?;
    for (key, files) in project::coverage_for_project(project_root)? {
        coverage.entry(key).or_default().extend(files);
    }
    Ok(coverage)
}

fn local_review_coverage(public_user_id: &str) -> Result<project::ProjectReviewCoverage> {
    let mut coverage = project::ProjectReviewCoverage::new();
    for stored in crate::review::fs::list_with_status()? {
        let review = stored.review;
        if review.reviewer_details.public_user_id != public_user_id {
            continue;
        }

        project::add_review_coverage(&mut coverage, &review);
    }
    Ok(coverage)
}

fn project_review_summary(
    plan: &DependencyReviewPlan,
    project_reviews: &[crate::review::Review],
) -> DependencyProjectReviewSummary {
    let coverage = project::coverage_for_reviews(project_reviews);
    let mut summary = DependencyProjectReviewSummary::default();

    for package in &plan.packages {
        let package_key = project::package_key_from_record(package);
        let covered_files = coverage.get(&package_key);
        summary.covered_files += package
            .batches
            .iter()
            .flat_map(|batch| &batch.files)
            .filter(|file| {
                covered_files
                    .map(|covered_files| {
                        covered_files.contains(&project::file_key_from_plan_file(file))
                    })
                    .unwrap_or(false)
            })
            .count();

        let matches = project::matching_reviews_for_package(
            project_reviews,
            &package.registry_host,
            &package.package_name,
            &package.package_version,
            package,
        );
        summary.matching_reviews += matches.reviews.len();
        summary.mismatched_reviews += matches
            .candidate_count
            .saturating_sub(matches.reviews.len());
    }

    summary
}

fn refresh_plan_progress(
    plan: &mut DependencyReviewPlan,
    coverage: &project::ProjectReviewCoverage,
) -> bool {
    let mut changed = false;
    for package in &mut plan.packages {
        let package_key = project::package_key_from_record(package);
        let covered_files = coverage.get(&package_key);
        for batch in &mut package.batches {
            let new_status = if batch_is_covered(batch, covered_files) {
                DependencyReviewBatchStatus::Reviewed
            } else {
                DependencyReviewBatchStatus::Pending
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
    plan: &DependencyReviewPlan,
    coverage: &project::ProjectReviewCoverage,
) -> Option<DependencyReviewSelection> {
    let plan_batch_count = plan.batch_count();
    for package in &plan.packages {
        let package_key = project::package_key_from_record(package);
        let covered_files = coverage.get(&package_key);
        for batch in &package.batches {
            if batch.status == DependencyReviewBatchStatus::Reviewed {
                continue;
            }

            let target_files = uncovered_batch_files(batch, covered_files);
            if target_files.is_empty() {
                continue;
            }

            return Some(DependencyReviewSelection {
                extension_name: package.extension_name.clone(),
                registry_host: package.registry_host.clone(),
                package_name: package.package_name.clone(),
                package_version: package.package_version.clone(),
                plan_rank: batch.plan_rank,
                plan_batch_count,
                package_batch_rank: batch.package_batch_rank,
                batch_file_count: batch.files.len(),
                target_files,
            });
        }
    }
    None
}

fn set_batch_status(
    plan: &mut DependencyReviewPlan,
    plan_rank: usize,
    status: DependencyReviewBatchStatus,
) -> bool {
    for batch in plan
        .packages
        .iter_mut()
        .flat_map(|package| &mut package.batches)
    {
        if batch.plan_rank == plan_rank {
            if batch.status == status {
                return false;
            }
            batch.status = status;
            return true;
        }
    }
    false
}

fn batch_is_covered(
    batch: &DependencyReviewBatch,
    covered_files: Option<&BTreeSet<project::ProjectReviewFileKey>>,
) -> bool {
    let Some(covered_files) = covered_files else {
        return false;
    };
    batch
        .files
        .iter()
        .all(|file| covered_files.contains(&project::file_key_from_plan_file(file)))
}

fn uncovered_batch_files(
    batch: &DependencyReviewBatch,
    covered_files: Option<&BTreeSet<project::ProjectReviewFileKey>>,
) -> Vec<String> {
    batch
        .files
        .iter()
        .filter(|file| {
            covered_files
                .map(|covered_files| {
                    !covered_files.contains(&project::file_key_from_plan_file(file))
                })
                .unwrap_or(true)
        })
        .map(|file| file.path.clone())
        .collect()
}

fn dependency_plan_batch_limits() -> DependencyReviewBatchLimits {
    DependencyReviewBatchLimits {
        max_lines_per_batch: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_LINES,
        max_files_per_batch: thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_FILES,
    }
}

fn skipped_dependency_package(
    package: &DependencyReviewPackage,
    error: anyhow::Error,
) -> SkippedDependencyReviewPackage {
    SkippedDependencyReviewPackage {
        extension_name: package.extension_name.clone(),
        registry_host: package.registry_host_name.clone(),
        package_name: package.package_name.clone(),
        package_version: package.package_version.clone(),
        reason: error.to_string(),
    }
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .context(format!("can't canonicalize path: {}", path.display()))
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
    fn dependency_snapshot_id_is_stable_when_packages_are_sorted() -> Result<()> {
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
            dependency_snapshot_id("/project", &files, &left_packages)?,
            dependency_snapshot_id("/project", &files, &right_packages)?
        );
        Ok(())
    }

    #[test]
    fn dependency_snapshot_id_changes_when_dependency_snapshot_changes() -> Result<()> {
        let packages = vec![package("rs", "crates.io", "serde", "1.0.0")];

        let first = dependency_snapshot_id(
            "/project",
            &[source_file("/project/Cargo.toml", "first-hash")],
            &packages,
        )?;
        let second = dependency_snapshot_id(
            "/project",
            &[source_file("/project/Cargo.toml", "second-hash")],
            &packages,
        )?;

        assert_ne!(first, second);
        Ok(())
    }

    #[test]
    fn new_plan_keeps_packages_pending() -> Result<()> {
        let packages = vec![
            package("rs", "crates.io", "serde", "1.0.0"),
            package("js", "npmjs.com", "left-pad", "1.3.0"),
        ];

        let plan = new_plan(
            "/project",
            vec![source_file("/project/Cargo.lock", "source-hash")],
            packages.clone(),
            "plan-id",
        )?;

        assert_eq!(plan.snapshot_id, "plan-id");
        assert_eq!(plan.source.snapshot_id, "plan-id");
        assert_eq!(plan.source.dependency_count, 2);
        assert_eq!(plan.packages, Vec::new());
        assert_eq!(plan.pending_packages, packages);
        assert_eq!(plan.skipped_packages, Vec::new());
        Ok(())
    }

    #[test]
    fn prepare_next_package_skips_missing_extension_without_other_packages() -> Result<()> {
        let mut plan = sample_plan();
        plan.pending_packages = vec![package("rs", "crates.io", "serde", "1.0.0")];
        plan.source.dependency_count = 1;
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> = Vec::new();

        let preparation = plan
            .prepare_next_package(&extensions)?
            .expect("package should be skipped");

        assert_eq!(
            preparation,
            DependencyReviewPreparation::Skipped {
                extension_name: "rs".to_string(),
                registry_host: "crates.io".to_string(),
                package_name: "serde".to_string(),
                package_version: "1.0.0".to_string(),
                reason: "extension 'rs' is not enabled".to_string(),
            }
        );
        assert_eq!(plan.pending_package_count(), 0);
        assert_eq!(plan.skipped_packages.len(), 1);
        assert_eq!(plan.packages.len(), 0);
        Ok(())
    }

    #[test]
    fn select_next_review_skips_covered_files() {
        let mut plan = plan_with_batches();
        let mut coverage = project::ProjectReviewCoverage::new();
        coverage.insert(
            project::package_key_from_record(&plan.packages[0]),
            reviewed_files(&plan, &["src/a.rs", "src/b.rs", "src/c.rs"]),
        );

        assert!(refresh_plan_progress(&mut plan, &coverage));
        let selection = select_next_review(&plan, &coverage)
            .expect("partially covered plan should still select work");

        assert_eq!(plan.reviewed_batch_count(), 1);
        assert_eq!(plan.remaining_batch_count(), 1);
        assert_eq!(selection.plan_rank, 2);
        assert_eq!(selection.plan_batch_count, 2);
        assert_eq!(selection.package_batch_rank, 2);
        assert_eq!(selection.batch_file_count, 2);
        assert_eq!(selection.target_files, vec!["src/d.rs".to_string()]);
    }

    #[test]
    fn select_next_review_returns_none_when_plan_is_covered() {
        let mut plan = plan_with_batches();
        let mut coverage = project::ProjectReviewCoverage::new();
        coverage.insert(
            project::package_key_from_record(&plan.packages[0]),
            reviewed_files(&plan, &["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]),
        );

        assert!(refresh_plan_progress(&mut plan, &coverage));

        assert_eq!(plan.reviewed_batch_count(), 2);
        assert_eq!(plan.remaining_batch_count(), 0);
        assert_eq!(select_next_review(&plan, &coverage), None);
    }

    #[test]
    fn select_next_review_requires_matching_file_hash() {
        let mut plan = plan_with_batches();
        let mut coverage = project::ProjectReviewCoverage::new();
        coverage.insert(
            project::package_key_from_record(&plan.packages[0]),
            vec![project::ProjectReviewFileKey {
                path: "src/a.rs".to_string(),
                file_hash: thirdpass_core::schema::FileHash::blake3("different-hash"),
            }]
            .into_iter()
            .collect(),
        );

        assert!(!refresh_plan_progress(&mut plan, &coverage));
        let selection = select_next_review(&plan, &coverage)
            .expect("hash mismatch should leave the file uncovered");

        assert_eq!(selection.plan_rank, 1);
        assert_eq!(
            selection.target_files,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }

    #[test]
    fn select_next_review_requires_matching_package_hash() {
        let mut plan = plan_with_batches();
        let mut key = project::package_key_from_record(&plan.packages[0]);
        key.package_hash = "different-package-hash".to_string();
        let mut coverage = project::ProjectReviewCoverage::new();
        coverage.insert(
            key,
            reviewed_files(&plan, &["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]),
        );

        assert!(!refresh_plan_progress(&mut plan, &coverage));
        let selection = select_next_review(&plan, &coverage)
            .expect("package hash mismatch should leave the package uncovered");

        assert_eq!(selection.plan_rank, 1);
        assert_eq!(
            selection.target_files,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }

    #[test]
    fn project_review_coverage_counts_project_artifacts_from_any_reviewer() -> Result<()> {
        let project = tempfile::tempdir()?;
        let plan = plan_with_batches();
        let review = review_for_plan_file(&plan, "other-user", "src/a.rs")?;
        project::store_dependency_review(project.path(), &review)?;

        let coverage = project::coverage_for_project(project.path())?;
        let key = project::package_key_from_record(&plan.packages[0]);
        let covered_files = coverage
            .get(&key)
            .expect("project review should cover the package");

        assert!(covered_files.contains(&project::ProjectReviewFileKey {
            path: "src/a.rs".to_string(),
            file_hash: thirdpass_core::schema::FileHash::blake3("file-hash-0"),
        }));
        Ok(())
    }

    fn source_file(path: &str, blake3: &str) -> DependencyReviewSourceFile {
        DependencyReviewSourceFile {
            path: path.to_string(),
            blake3: blake3.to_string(),
        }
    }

    fn package(
        extension_name: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> DependencyReviewPackage {
        DependencyReviewPackage {
            extension_name: extension_name.to_string(),
            registry_host_name: registry_host_name.to_string(),
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
        }
    }

    fn sample_plan() -> DependencyReviewPlan {
        DependencyReviewPlan {
            schema_version: DEPENDENCY_REVIEW_PLAN_VERSION,
            generated_at_unix: 1,
            batch_limits: dependency_plan_batch_limits(),
            snapshot_id: "snapshot-id".to_string(),
            source: DependencyReviewSource {
                snapshot_id: "snapshot-id".to_string(),
                project_root: "/project".to_string(),
                dependency_files: vec![source_file("/project/Cargo.toml", "hash")],
                dependency_count: 1,
            },
            packages: Vec::new(),
            pending_packages: Vec::new(),
            skipped_packages: Vec::new(),
        }
    }

    fn reviewed_files(
        plan: &DependencyReviewPlan,
        paths: &[&str],
    ) -> BTreeSet<project::ProjectReviewFileKey> {
        plan.packages
            .iter()
            .flat_map(|package| &package.batches)
            .flat_map(|batch| &batch.files)
            .filter(|file| paths.contains(&file.path.as_str()))
            .map(project::file_key_from_plan_file)
            .collect()
    }

    fn plan_with_batches() -> DependencyReviewPlan {
        let mut plan = sample_plan();
        plan.packages = vec![DependencyReviewPackageRecord {
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
        plan
    }

    fn review_for_plan_file(
        plan: &DependencyReviewPlan,
        public_user_id: &str,
        path: &str,
    ) -> Result<crate::review::Review> {
        let package = &plan.packages[0];
        let file = plan
            .packages
            .iter()
            .flat_map(|package| &package.batches)
            .flat_map(|batch| &batch.files)
            .find(|file| file.path == path)
            .expect("test review target should exist");
        let mut registries = BTreeSet::new();
        registries.insert(crate::registry::Registry {
            id: 0,
            host_name: package.registry_host.clone(),
            human_url: url::Url::parse(&package.human_url)?,
            artifact_url: url::Url::parse(&package.artifact_url)?,
        });

        Ok(crate::review::Review {
            id: 0,
            peer: crate::peer::Peer::default(),
            package: crate::package::Package {
                id: 0,
                name: package.package_name.clone(),
                version: package.package_version.clone(),
                registries,
                package_hash: package.package_hash.clone(),
            },
            targets: vec![crate::review::ReviewTarget {
                file_path: PathBuf::from(path),
                file_hash: Some(file.file_hash.clone()),
                agent_summary: None,
                security_summary: Some(crate::review::SecuritySummary::None),
                confidence: None,
                agent_run_metrics: None,
                comments: BTreeSet::new(),
            }],
            reviewer_details: crate::review::ReviewerDetails {
                public_user_id: public_user_id.to_string(),
                ..crate::review::ReviewerDetails::default()
            },
            agent_summary: String::new(),
            overall_security_summary: crate::review::SecuritySummary::None,
            overall_security_confidence: None,
        })
    }

    fn batch(plan_rank: usize, package_batch_rank: usize, paths: &[&str]) -> DependencyReviewBatch {
        DependencyReviewBatch {
            plan_rank,
            package_batch_rank,
            status: DependencyReviewBatchStatus::Pending,
            total_lines: paths.len(),
            files: paths
                .iter()
                .enumerate()
                .map(|(index, path)| DependencyReviewFile {
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
