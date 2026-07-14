use anyhow::{format_err, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::review;

const PROJECT_REVIEW_SCHEMA_VERSION: u32 = 1;
const PROJECT_REVIEW_FILE_PREFIX: &str = "review-";

/// Return dependency reviews committed inside the project checkout.
pub(crate) fn list_dependency_reviews(project_root: &Path) -> Result<Vec<review::Review>> {
    let reviews_directory = project_root.join(".thirdpass").join("reviews");
    if !reviews_directory.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_project_review_files(&reviews_directory, &mut files)?;
    files.sort();

    let mut reviews = Vec::new();
    for file in files {
        let reader = std::io::BufReader::new(std::fs::File::open(&file)?);
        match serde_json::from_reader::<_, ProjectReviewArtifact>(reader) {
            Ok(artifact) => {
                if artifact.schema_version != PROJECT_REVIEW_SCHEMA_VERSION {
                    log::warn!(
                        "Skipping project review file {}: unsupported schema version {}",
                        file.display(),
                        artifact.schema_version
                    );
                    continue;
                }
                let mut review = artifact.review;
                review.overall_security_summary = crate::review::overall_security_summary(&review)?;
                reviews.push(review);
            }
            Err(err) => {
                log::warn!(
                    "Failed to parse project review file {}: {}",
                    file.display(),
                    err
                );
            }
        }
    }
    Ok(reviews)
}

/// Store a dependency review artifact inside the project checkout.
pub(crate) fn store_dependency_review(
    project_root: &Path,
    review: &review::Review,
) -> Result<PathBuf> {
    let artifact = ProjectReviewArtifact::from_review(review);
    let bytes = serde_json::to_vec_pretty(&artifact)?;
    let path = project_review_path(project_root, review, &bytes)?;
    write_json_atomically(&path, &bytes)?;
    Ok(path)
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ProjectReviewArtifact {
    schema_version: u32,
    review: review::Review,
}

impl ProjectReviewArtifact {
    fn from_review(review: &review::Review) -> Self {
        Self {
            schema_version: PROJECT_REVIEW_SCHEMA_VERSION,
            review: review.clone(),
        }
    }
}

/// Exact package identity used when matching project-local review coverage.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProjectReviewPackageKey {
    /// Registry host that owns this package.
    pub(crate) registry_host: String,
    /// Package name in the registry.
    pub(crate) package_name: String,
    /// Package version in the registry.
    pub(crate) package_version: String,
    /// Blake3 digest of the package source artifact.
    pub(crate) package_hash: String,
}

/// Exact file identity used when matching project-local review coverage.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProjectReviewFileKey {
    /// Package-relative path.
    pub(crate) path: String,
    /// File content hash.
    pub(crate) file_hash: thirdpass_core::schema::FileHash,
}

/// File coverage grouped by exact package identity.
pub(crate) type ProjectReviewCoverage =
    BTreeMap<ProjectReviewPackageKey, BTreeSet<ProjectReviewFileKey>>;

/// Project reviews split into matching and mismatched candidates for one package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ProjectReviewMatches {
    /// Project reviews with matching registry, package name, and version.
    pub(crate) candidate_count: usize,
    /// Candidate reviews that still match the current package and file hashes.
    pub(crate) reviews: Vec<review::Review>,
}

/// Build exact file coverage from a set of reviews.
pub(crate) fn coverage_for_reviews<'a>(
    reviews: impl IntoIterator<Item = &'a review::Review>,
) -> ProjectReviewCoverage {
    let mut coverage = ProjectReviewCoverage::new();
    for review in reviews {
        add_review_coverage(&mut coverage, review);
    }
    coverage
}

/// Build exact file coverage from reviews committed inside a project checkout.
pub(crate) fn coverage_for_project(project_root: &Path) -> Result<ProjectReviewCoverage> {
    Ok(coverage_for_reviews(&list_dependency_reviews(
        project_root,
    )?))
}

/// Add one review's target files to exact package/file coverage.
pub(crate) fn add_review_coverage(coverage: &mut ProjectReviewCoverage, review: &review::Review) {
    for registry in &review.package.registries {
        let key = package_key_from_review_registry(review, registry);
        let package_coverage = coverage.entry(key).or_default();
        for target in &review.targets {
            if let Some(key) = file_key_from_review_target(target) {
                package_coverage.insert(key);
            }
        }
    }
}

/// Return the exact package key for an analyzed dependency package.
pub(crate) fn package_key_from_record(
    package: &review::dependency_plan::DependencyReviewPackageRecord,
) -> ProjectReviewPackageKey {
    ProjectReviewPackageKey {
        registry_host: package.registry_host.clone(),
        package_name: package.package_name.clone(),
        package_version: package.package_version.clone(),
        package_hash: package.package_hash.clone(),
    }
}

/// Return the exact file key for an analyzed dependency package file.
pub(crate) fn file_key_from_plan_file(
    file: &review::dependency_plan::DependencyReviewFile,
) -> ProjectReviewFileKey {
    ProjectReviewFileKey {
        path: file.path.clone(),
        file_hash: file.file_hash.clone(),
    }
}

/// Return project reviews for a dependency that still match current package content.
pub(crate) fn matching_reviews_for_package(
    reviews: &[review::Review],
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
    current: &review::dependency_plan::DependencyReviewPackageRecord,
) -> ProjectReviewMatches {
    let candidates =
        reviews_for_package(reviews, registry_host_name, package_name, package_version);
    let candidate_count = candidates.len();
    let reviews = candidates
        .into_iter()
        .filter(|review| review_matches_package(review, current))
        .collect();

    ProjectReviewMatches {
        candidate_count,
        reviews,
    }
}

/// Return true when a review applies to the current package artifact and files.
pub(crate) fn review_matches_package(
    review: &review::Review,
    current: &review::dependency_plan::DependencyReviewPackageRecord,
) -> bool {
    review.package.name == current.package_name
        && review.package.version == current.package_version
        && review.package.package_hash == current.package_hash
        && review
            .package
            .registries
            .iter()
            .any(|registry| registry.host_name == current.registry_host)
        && review_targets_match_current_package(review, current)
}

fn package_key_from_review_registry(
    review: &review::Review,
    registry: &crate::registry::Registry,
) -> ProjectReviewPackageKey {
    ProjectReviewPackageKey {
        registry_host: registry.host_name.clone(),
        package_name: review.package.name.clone(),
        package_version: review.package.version.clone(),
        package_hash: review.package.package_hash.clone(),
    }
}

fn file_key_from_review_target(target: &review::ReviewTarget) -> Option<ProjectReviewFileKey> {
    target
        .file_hash
        .as_ref()
        .map(|file_hash| ProjectReviewFileKey {
            path: package_relative_path_string(&target.file_path),
            file_hash: file_hash.clone(),
        })
}

/// Return reviews with matching registry, package name, and package version.
pub(crate) fn reviews_for_package(
    reviews: &[review::Review],
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
) -> Vec<review::Review> {
    reviews
        .iter()
        .filter(|review| {
            review.package.name == package_name
                && review.package.version == package_version
                && review
                    .package
                    .registries
                    .iter()
                    .any(|registry| registry.host_name == registry_host_name)
        })
        .cloned()
        .collect()
}

fn review_targets_match_current_package(
    review: &review::Review,
    current: &review::dependency_plan::DependencyReviewPackageRecord,
) -> bool {
    if review.targets.is_empty() {
        return false;
    }

    let current_files = current
        .batches
        .iter()
        .flat_map(|batch| &batch.files)
        .map(file_key_from_plan_file)
        .collect::<BTreeSet<_>>();

    review.targets.iter().all(|target| {
        file_key_from_review_target(target)
            .map(|key| current_files.contains(&key))
            .unwrap_or(false)
    })
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

fn project_review_path(
    project_root: &Path,
    review: &review::Review,
    bytes: &[u8],
) -> Result<PathBuf> {
    let registry_host_name = single_registry_host(review)?;
    let package_path = review::fs::get_unique_package_path(
        &review.package.name,
        &review.package.version,
        registry_host_name,
    )?;
    let digest = blake3::hash(bytes).to_hex().as_str()[0..16].to_string();
    Ok(project_root
        .join(".thirdpass")
        .join("reviews")
        .join(package_path)
        .join(&review.package.package_hash)
        .join(format!("review-{digest}.json")))
}

fn collect_project_review_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory).context(format!(
        "can't read project review directory: {}",
        directory.display()
    ))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_project_review_files(&path, files)?;
            continue;
        }
        if is_project_review_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_project_review_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("json")
        && path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .map(|file_name| file_name.starts_with(PROJECT_REVIEW_FILE_PREFIX))
            .unwrap_or(false)
}

fn single_registry_host(review: &review::Review) -> Result<&str> {
    let mut registries = review.package.registries.iter();
    match (registries.next(), registries.next()) {
        (Some(registry), None) => Ok(&registry.host_name),
        (None, _) => Err(format_err!(
            "Project review storage requires exactly one registry for {}@{}; found none.",
            review.package.name,
            review.package.version
        )),
        (Some(_), Some(_)) => Err(format_err!(
            "Project review storage requires exactly one registry for {}@{}; found {}.",
            review.package.name,
            review.package.version,
            review.package.registries.len()
        )),
    }
}

fn write_json_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(format_err!(
        "can't find parent directory for project review: {}",
        path.display()
    ))?;
    std::fs::create_dir_all(parent).context(format!(
        "can't create project review directory: {}",
        parent.display()
    ))?;

    let mut temp_file = tempfile::NamedTempFile::new_in(parent).context(format!(
        "can't create temporary project review in: {}",
        parent.display()
    ))?;
    {
        let mut writer = std::io::BufWriter::new(temp_file.as_file_mut());
        writer.write_all(bytes)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .map_err(|error| error.error)
        .context(format!("can't replace project review: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, peer, registry};

    #[test]
    fn store_dependency_review_writes_project_artifact() -> Result<()> {
        let project = tempfile::tempdir()?;
        let review = stored_review()?;

        let path = store_dependency_review(project.path(), &review)?;
        let contents = std::fs::read_to_string(&path)?;
        let artifact: ProjectReviewArtifact = serde_json::from_str(&contents)?;

        assert!(path.starts_with(project.path().join(".thirdpass").join("reviews")));
        assert_eq!(artifact.schema_version, PROJECT_REVIEW_SCHEMA_VERSION);
        assert_eq!(artifact.review.package.name, "left-pad");
        assert!(!contents.contains(&project.path().display().to_string()));
        Ok(())
    }

    #[test]
    fn list_dependency_reviews_reads_project_artifacts() -> Result<()> {
        let project = tempfile::tempdir()?;
        let review = stored_review()?;
        store_dependency_review(project.path(), &review)?;

        let reviews = list_dependency_reviews(project.path())?;

        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].package.name, "left-pad");
        assert_eq!(reviews[0].package.package_hash, "package-hash");
        Ok(())
    }

    #[test]
    fn list_dependency_reviews_ignores_unsupported_artifacts() -> Result<()> {
        let project = tempfile::tempdir()?;
        let reviews_directory = project.path().join(".thirdpass").join("reviews");
        std::fs::create_dir_all(&reviews_directory)?;
        std::fs::write(
            reviews_directory.join("review-old.json"),
            serde_json::to_string_pretty(&ProjectReviewArtifact {
                schema_version: 99,
                review: stored_review()?,
            })?,
        )?;

        let reviews = list_dependency_reviews(project.path())?;

        assert_eq!(reviews, Vec::new());
        Ok(())
    }

    #[test]
    fn list_dependency_reviews_ignores_missing_directory() -> Result<()> {
        let project = tempfile::tempdir()?;

        let reviews = list_dependency_reviews(project.path())?;

        assert_eq!(reviews, Vec::new());
        Ok(())
    }

    fn stored_review() -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: "npmjs.com".to_string(),
            human_url: url::Url::parse("https://npmjs.com/package/left-pad")?,
            artifact_url: url::Url::parse("https://registry.npmjs.org/left-pad/-/left-pad.tgz")?,
        });

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: "left-pad".to_string(),
                version: "1.3.0".to_string(),
                registries,
                package_hash: "package-hash".to_string(),
            },
            targets: Vec::new(),
            reviewer_details: review::ReviewerDetails::default(),
            review_configuration: None,
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
