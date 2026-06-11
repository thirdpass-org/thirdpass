use anyhow::{format_err, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::review;

const PROJECT_REVIEW_SCHEMA_VERSION: u32 = 1;

/// Store a dependency review artifact inside the project checkout.
pub(crate) fn store_dependency_review(
    project_root: &Path,
    source: &review::dependency_queue::DependencyQueueSource,
    review: &review::Review,
) -> Result<PathBuf> {
    let artifact = ProjectReviewArtifact::from_source(project_root, source, review);
    let bytes = serde_json::to_vec_pretty(&artifact)?;
    let path = project_review_path(project_root, review, &bytes)?;
    write_json_atomically(&path, &bytes)?;
    Ok(path)
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ProjectReviewArtifact {
    schema_version: u32,
    source: ProjectReviewSource,
    review: review::Review,
}

impl ProjectReviewArtifact {
    fn from_source(
        project_root: &Path,
        source: &review::dependency_queue::DependencyQueueSource,
        review: &review::Review,
    ) -> Self {
        let source_project_root = PathBuf::from(&source.project_root);
        let dependency_files = source
            .dependency_files
            .iter()
            .map(|file| ProjectReviewSourceFile {
                path: project_relative_path(&source_project_root, Path::new(&file.path)),
                blake3: file.blake3.clone(),
            })
            .collect();

        Self {
            schema_version: PROJECT_REVIEW_SCHEMA_VERSION,
            source: ProjectReviewSource {
                snapshot_id: source.snapshot_id.clone(),
                project_root: project_relative_path(project_root, project_root),
                dependency_files,
                dependency_count: source.dependency_count,
            },
            review: review.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ProjectReviewSource {
    snapshot_id: String,
    project_root: String,
    dependency_files: Vec<ProjectReviewSourceFile>,
    dependency_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ProjectReviewSourceFile {
    path: String,
    blake3: String,
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

fn project_relative_path(project_root: &Path, path: &Path) -> String {
    if path == project_root {
        return ".".to_string();
    }

    path.strip_prefix(project_root)
        .ok()
        .map(package_relative_path_string)
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_string())
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
    fn store_dependency_review_writes_project_relative_artifact() -> Result<()> {
        let project = tempfile::tempdir()?;
        let dependency_file = project.path().join("package-lock.json");
        std::fs::write(&dependency_file, "{}")?;
        let source = dependency_source(project.path(), &dependency_file);
        let review = stored_review()?;

        let path = store_dependency_review(project.path(), &source, &review)?;
        let contents = std::fs::read_to_string(&path)?;
        let artifact: ProjectReviewArtifact = serde_json::from_str(&contents)?;

        assert!(path.starts_with(project.path().join(".thirdpass").join("reviews")));
        assert_eq!(artifact.schema_version, PROJECT_REVIEW_SCHEMA_VERSION);
        assert_eq!(artifact.source.project_root, ".");
        assert_eq!(
            artifact.source.dependency_files[0].path,
            "package-lock.json"
        );
        assert_eq!(
            artifact.source.dependency_files[0].blake3,
            "dependency-hash"
        );
        assert_eq!(artifact.source.snapshot_id, "snapshot-id");
        assert_eq!(artifact.review.package.name, "left-pad");
        assert!(!contents.contains(&project.path().display().to_string()));
        Ok(())
    }

    fn dependency_source(
        project_root: &Path,
        dependency_file: &Path,
    ) -> review::dependency_queue::DependencyQueueSource {
        review::dependency_queue::DependencyQueueSource {
            snapshot_id: "snapshot-id".to_string(),
            project_root: project_root.display().to_string(),
            dependency_files: vec![review::dependency_queue::DependencyQueueSourceFile {
                path: dependency_file.display().to_string(),
                blake3: "dependency-hash".to_string(),
            }],
            dependency_count: 1,
        }
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
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
