use anyhow::Result;

use crate::review;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct DependencyReport {
    pub summary: review::SecuritySummary,
    pub name: String,
    pub version: Option<String>,
    pub review_count: Option<usize>,
    pub note: Option<String>,
}

/// Project-local reviews available while building a dependency report.
pub struct ProjectReviewContext<'a> {
    /// Extension that can resolve and prepare the current package artifact.
    pub extension: &'a dyn thirdpass_core::extension::Extension,
    /// Reviews read from the current project's `.thirdpass/reviews` directory.
    pub reviews: &'a [review::Review],
}

/// Given a local project dependency, create a corresponding review report from known reviews.
pub fn get_dependency_report(
    dependency: &thirdpass_core::extension::Dependency,
    registry_host_name: &str,
    config: &crate::common::config::Config,
    project_review_context: Option<&ProjectReviewContext>,
) -> Result<DependencyReport> {
    let package_version = match &dependency.version {
        Ok(version) => version.clone(),
        Err(error) => {
            return Ok(DependencyReport {
                summary: review::SecuritySummary::Medium,
                name: dependency.name.clone(),
                version: None,
                review_count: None,
                note: Some(error.to_string()),
            });
        }
    };

    let sync_note = match pull_latest_reviews(
        registry_host_name,
        &dependency.name,
        &package_version,
        config,
    ) {
        Ok(_) => None,
        Err(err) => {
            log::warn!(
                "Failed to sync latest reviews for {name}@{version} ({registry}): {error}",
                name = dependency.name,
                version = package_version,
                registry = registry_host_name,
                error = err
            );
            Some("sync failed; using local cache".to_string())
        }
    };

    let mut reviews = filter_reviews(
        &review::fs::list()?,
        registry_host_name,
        &dependency.name,
        &package_version,
    );
    let project_note = match project_review_context {
        Some(context) => match matching_project_reviews(
            context,
            registry_host_name,
            &dependency.name,
            &package_version,
        ) {
            Ok(project_reviews) => {
                let note = project_review_note(&project_reviews);
                reviews.extend(project_reviews.reviews);
                reviews = deduplicate_reviews(reviews);
                note
            }
            Err(err) => {
                log::warn!(
                    "Failed to validate project reviews for {name}@{version} ({registry}): {error}",
                    name = dependency.name,
                    version = package_version,
                    registry = registry_host_name,
                    error = err
                );
                Some("project review validation failed".to_string())
            }
        },
        None => None,
    };
    let sync_note = merge_notes(sync_note, project_note);

    if reviews.is_empty() {
        // Report no reviews found for dependency.
        return Ok(DependencyReport {
            summary: review::SecuritySummary::None,
            name: dependency.name.clone(),
            version: Some(package_version.clone()),
            review_count: Some(0),
            note: sync_note,
        });
    }

    let stats = get_dependency_stats(&reviews)?;
    let status = get_dependency_status(&stats)?;
    let note = merge_notes(get_dependency_note(&stats), sync_note);

    Ok(DependencyReport {
        summary: status,
        name: dependency.name.clone(),
        version: Some(package_version.clone()),
        review_count: Some(reviews.len()),
        note,
    })
}

fn pull_latest_reviews(
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
    config: &crate::common::config::Config,
) -> Result<()> {
    let query = review::remote::ReviewQuery {
        registry_host: Some(registry_host_name.to_string()),
        package_name: Some(package_name.to_string()),
        package_version: Some(package_version.to_string()),
        file_path: None,
    };
    let records = review::remote::fetch(&query, config)?;
    review::remote::store_records(records, config)?;
    Ok(())
}

fn filter_reviews(
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

#[derive(Debug)]
struct ProjectReviewMatches {
    candidate_count: usize,
    reviews: Vec<review::Review>,
}

fn matching_project_reviews(
    context: &ProjectReviewContext,
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
) -> Result<ProjectReviewMatches> {
    let candidates = filter_reviews(
        context.reviews,
        registry_host_name,
        package_name,
        package_version,
    );
    if candidates.is_empty() {
        return Ok(ProjectReviewMatches {
            candidate_count: 0,
            reviews: Vec::new(),
        });
    }
    let candidate_count = candidates.len();

    let package = review::dependency_plan::DependencyReviewPackage {
        extension_name: context.extension.name(),
        registry_host_name: registry_host_name.to_string(),
        package_name: package_name.to_string(),
        package_version: package_version.to_string(),
    };
    let current =
        review::dependency_plan::package_record_for_extension(&package, context.extension)?;

    Ok(ProjectReviewMatches {
        candidate_count,
        reviews: candidates
            .into_iter()
            .filter(|review| project_review_matches_current_package(review, &current))
            .collect(),
    })
}

fn project_review_note(matches: &ProjectReviewMatches) -> Option<String> {
    if matches.candidate_count == 0 {
        None
    } else if matches.reviews.is_empty() {
        Some("project reviews stale".to_string())
    } else {
        Some(format!("project reviews ({})", matches.reviews.len()))
    }
}

fn project_review_matches_current_package(
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
        && project_review_targets_match_current_package(review, current)
}

fn project_review_targets_match_current_package(
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
        .map(|file| (file.path.clone(), file.file_hash.clone()))
        .collect::<BTreeSet<_>>();

    review.targets.iter().all(|target| {
        target
            .file_hash
            .as_ref()
            .map(|file_hash| {
                current_files.contains(&(
                    package_relative_path_string(&target.file_path),
                    file_hash.clone(),
                ))
            })
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

fn deduplicate_reviews(reviews: Vec<review::Review>) -> Vec<review::Review> {
    reviews
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Debug, Default, Clone)]
struct DependencyStats {
    pub total_review_count: usize,
    pub count_critical_comments: i32,
    pub count_medium_comments: i32,
}

fn get_dependency_stats(reviews: &[review::Review]) -> Result<DependencyStats> {
    let mut stats = DependencyStats {
        total_review_count: reviews.len(),
        ..DependencyStats::default()
    };

    for review in reviews {
        match review::overall_security_summary(review)? {
            review::SecuritySummary::Critical => stats.count_critical_comments += 1,
            review::SecuritySummary::Medium => stats.count_medium_comments += 1,
            review::SecuritySummary::Low => {}
            review::SecuritySummary::None => {}
        }
    }
    Ok(stats)
}

fn get_dependency_status(stats: &DependencyStats) -> Result<review::SecuritySummary> {
    if stats.count_critical_comments > 0 {
        return Ok(review::SecuritySummary::Critical);
    }
    if stats.count_medium_comments > 0 {
        return Ok(review::SecuritySummary::Medium);
    }
    if stats.total_review_count == 0 {
        return Ok(review::SecuritySummary::None);
    }
    Ok(review::SecuritySummary::Low)
}

fn get_dependency_note(stats: &DependencyStats) -> Option<String> {
    let mut note_parts = Vec::<_>::new();
    if stats.count_critical_comments > 0 {
        note_parts.push(format!("critical ({})", stats.count_critical_comments));
    }

    if stats.count_medium_comments > 0 {
        note_parts.push(format!("medium ({})", stats.count_medium_comments));
    }

    if note_parts.is_empty() {
        None
    } else {
        Some(note_parts.join("; "))
    }
}

fn merge_notes(primary_note: Option<String>, secondary_note: Option<String>) -> Option<String> {
    match (primary_note, secondary_note) {
        (None, None) => None,
        (Some(note), None) => Some(note),
        (None, Some(note)) => Some(note),
        (Some(primary), Some(secondary)) => Some(format!("{}; {}", primary, secondary)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, peer, registry};
    use std::path::PathBuf;

    #[test]
    fn project_review_matches_current_package_by_package_and_file_hash() -> Result<()> {
        let current = package_record(
            "package-hash",
            &[("src/lib.rs", "file-hash"), ("README.md", "readme-hash")],
        );
        let review = stored_review("package-hash", &[("src/lib.rs", "file-hash")])?;

        assert!(project_review_matches_current_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_different_package_hash() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("different-package-hash", &[("src/lib.rs", "file-hash")])?;

        assert!(!project_review_matches_current_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_different_file_hash() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("package-hash", &[("src/lib.rs", "different-file-hash")])?;

        assert!(!project_review_matches_current_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_empty_target_set() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("package-hash", &[])?;

        assert!(!project_review_matches_current_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_note_reports_matching_and_stale_reviews() -> Result<()> {
        assert_eq!(
            project_review_note(&ProjectReviewMatches {
                candidate_count: 0,
                reviews: Vec::new(),
            }),
            None
        );
        assert_eq!(
            project_review_note(&ProjectReviewMatches {
                candidate_count: 1,
                reviews: Vec::new(),
            }),
            Some("project reviews stale".to_string())
        );
        assert_eq!(
            project_review_note(&ProjectReviewMatches {
                candidate_count: 2,
                reviews: vec![stored_review(
                    "package-hash",
                    &[("src/lib.rs", "file-hash")]
                )?],
            }),
            Some("project reviews (1)".to_string())
        );
        Ok(())
    }

    fn package_record(
        package_hash: &str,
        files: &[(&str, &str)],
    ) -> review::dependency_plan::DependencyReviewPackageRecord {
        review::dependency_plan::DependencyReviewPackageRecord {
            extension_name: "fixture".to_string(),
            registry_host: "fixture.registry".to_string(),
            package_name: "fixture-package".to_string(),
            package_version: "1.0.0".to_string(),
            package_hash: package_hash.to_string(),
            human_url: "https://fixture.registry/fixture-package".to_string(),
            artifact_url: "https://fixture.registry/fixture-package-1.0.0.tar.gz".to_string(),
            batches: vec![review::dependency_plan::DependencyReviewBatch {
                plan_rank: 1,
                package_batch_rank: 1,
                status: review::dependency_plan::DependencyReviewBatchStatus::Pending,
                total_lines: files.len(),
                files: files
                    .iter()
                    .enumerate()
                    .map(|(index, (path, file_hash))| {
                        review::dependency_plan::DependencyReviewFile {
                            path: path.to_string(),
                            file_hash: thirdpass_core::schema::FileHash::blake3(*file_hash),
                            size_bytes: 10,
                            extension: None,
                            line_count: 1,
                            file_rank: index + 1,
                        }
                    })
                    .collect(),
            }],
        }
    }

    fn stored_review(package_hash: &str, targets: &[(&str, &str)]) -> Result<review::Review> {
        let mut registries = BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: "fixture.registry".to_string(),
            human_url: url::Url::parse("https://fixture.registry/fixture-package")?,
            artifact_url: url::Url::parse("https://fixture.registry/fixture-package-1.0.0.tar.gz")?,
        });

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: "fixture-package".to_string(),
                version: "1.0.0".to_string(),
                registries,
                package_hash: package_hash.to_string(),
            },
            targets: targets
                .iter()
                .map(|(path, file_hash)| review::ReviewTarget {
                    file_path: PathBuf::from(path),
                    file_hash: Some(thirdpass_core::schema::FileHash::blake3(*file_hash)),
                    agent_summary: None,
                    security_summary: Some(review::SecuritySummary::None),
                    confidence: None,
                    comments: BTreeSet::new(),
                })
                .collect(),
            reviewer_details: review::ReviewerDetails::default(),
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::None,
            overall_security_confidence: None,
        })
    }
}
