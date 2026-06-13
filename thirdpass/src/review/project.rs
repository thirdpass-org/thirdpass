use anyhow::{format_err, Context, Result};
use serde::{Deserialize, Serialize};
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
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
