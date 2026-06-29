use anyhow::Result;

use crate::review;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct DependencyReport {
    pub summary: review::SecuritySummary,
    pub name: String,
    pub version: Option<String>,
    pub review_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_reviews: Option<CommittedReviewReport>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct CommittedReviewReport {
    pub matching_count: usize,
    pub mismatch_count: usize,
    pub covered_file_count: usize,
    pub total_file_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum DependencyNote {
    VersionResolutionFailed(String),
    SyncFailed,
    ProjectReviewValidationFailed,
    ProjectReviewMismatch { count: usize },
    CriticalFindings { count: i32 },
    MediumFindings { count: i32 },
}

impl DependencyNote {
    fn render(self) -> String {
        match self {
            Self::VersionResolutionFailed(error) => error,
            Self::SyncFailed => "sync failed; using local cache".to_string(),
            Self::ProjectReviewValidationFailed => "project review validation failed".to_string(),
            Self::ProjectReviewMismatch { count } => {
                format!("committed review mismatch ({count})")
            }
            Self::CriticalFindings { count } => format!("critical ({count})"),
            Self::MediumFindings { count } => format!("medium ({count})"),
        }
    }
}

/// Review data shared while building dependency reports for one command.
pub struct DependencyReportContext<'a> {
    /// Extension that can resolve and prepare the current package artifact.
    pub extension: &'a dyn thirdpass_core::extension::Extension,
    /// Reviews read from the current project's `.thirdpass/reviews` directory.
    pub project_reviews: &'a [review::Review],
    /// Reviews read from local global storage, plus reviews fetched during the command.
    pub local_reviews: &'a mut Vec<review::Review>,
}

/// Given a local project dependency, create a corresponding review report from known reviews.
pub fn get_dependency_report(
    dependency: &thirdpass_core::extension::Dependency,
    registry_host_name: &str,
    config: &crate::common::config::Config,
    report_context: &mut DependencyReportContext,
) -> Result<DependencyReport> {
    let package_version = match &dependency.version {
        Ok(version) => version.clone(),
        Err(error) => {
            return Ok(DependencyReport {
                summary: review::SecuritySummary::Medium,
                name: dependency.name.clone(),
                version: None,
                review_count: None,
                committed_reviews: None,
                note: render_notes(Some(DependencyNote::VersionResolutionFailed(
                    error.to_string(),
                ))),
            });
        }
    };

    let mut context_notes = Vec::new();
    match pull_latest_reviews(
        registry_host_name,
        &dependency.name,
        &package_version,
        config,
    ) {
        Ok(fetched_reviews) => report_context.local_reviews.extend(fetched_reviews),
        Err(err) => {
            log::warn!(
                "Failed to sync latest reviews for {name}@{version} ({registry}): {error}",
                name = dependency.name,
                version = package_version,
                registry = registry_host_name,
                error = err
            );
            context_notes.push(DependencyNote::SyncFailed);
        }
    }

    let mut reviews = review::project::reviews_for_package(
        report_context.local_reviews.as_slice(),
        registry_host_name,
        &dependency.name,
        &package_version,
    );
    let committed_reviews = match committed_project_reviews(
        report_context,
        registry_host_name,
        &dependency.name,
        &package_version,
    ) {
        Ok(committed) => {
            context_notes.extend(committed_review_note(&committed.report));
            reviews.extend(committed.matching_reviews);
            reviews = deduplicate_reviews(reviews);
            Some(committed.report)
        }
        Err(err) => {
            log::warn!(
                "Failed to validate project reviews for {name}@{version} ({registry}): {error}",
                name = dependency.name,
                version = package_version,
                registry = registry_host_name,
                error = err
            );
            context_notes.push(DependencyNote::ProjectReviewValidationFailed);
            None
        }
    };

    if reviews.is_empty() {
        // Report no reviews found for dependency.
        return Ok(DependencyReport {
            summary: review::SecuritySummary::None,
            name: dependency.name.clone(),
            version: Some(package_version.clone()),
            review_count: Some(0),
            committed_reviews,
            note: render_notes(context_notes),
        });
    }

    let stats = get_dependency_stats(&reviews)?;
    let status = get_dependency_status(&stats)?;
    let mut notes = dependency_security_notes(&stats);
    notes.extend(context_notes);

    Ok(DependencyReport {
        summary: status,
        name: dependency.name.clone(),
        version: Some(package_version.clone()),
        review_count: Some(reviews.len()),
        committed_reviews,
        note: render_notes(notes),
    })
}

fn pull_latest_reviews(
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
    config: &crate::common::config::Config,
) -> Result<Vec<review::Review>> {
    let query = review::remote::ReviewQuery {
        registry_host: Some(registry_host_name.to_string()),
        package_name: Some(package_name.to_string()),
        package_version: Some(package_version.to_string()),
        file_path: None,
    };
    let records = review::remote::fetch(&query, config)?;
    review::remote::store_records_with_reviews(records, config)
}

struct CommittedProjectReviews {
    report: CommittedReviewReport,
    matching_reviews: Vec<review::Review>,
}

fn committed_project_reviews(
    context: &DependencyReportContext,
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
) -> Result<CommittedProjectReviews> {
    let candidates = review::project::reviews_for_package(
        context.project_reviews,
        registry_host_name,
        package_name,
        package_version,
    );
    let candidate_count = candidates.len();

    let package = review::dependency_plan::DependencyReviewPackage {
        extension_name: context.extension.name(),
        registry_host_name: registry_host_name.to_string(),
        package_name: package_name.to_string(),
        package_version: package_version.to_string(),
    };
    let current =
        review::dependency_package::package_record_for_extension(&package, context.extension)?;

    let mut matches = review::project::matching_reviews_for_package(
        &candidates,
        registry_host_name,
        package_name,
        package_version,
        &current,
    );
    matches.candidate_count = candidate_count;
    let report = committed_review_report(&current, &matches);
    Ok(CommittedProjectReviews {
        report,
        matching_reviews: matches.reviews,
    })
}

fn committed_review_report(
    current: &review::dependency_plan::DependencyReviewPackageRecord,
    matches: &review::project::ProjectReviewMatches,
) -> CommittedReviewReport {
    let package_key = review::project::package_key_from_record(current);
    let coverage = review::project::coverage_for_reviews(&matches.reviews);
    let covered_files = coverage.get(&package_key);
    let current_files = current
        .batches
        .iter()
        .flat_map(|batch| &batch.files)
        .map(review::project::file_key_from_plan_file)
        .collect::<BTreeSet<_>>();
    let covered_file_count = current_files
        .iter()
        .filter(|file| {
            covered_files
                .map(|covered_files| covered_files.contains(file))
                .unwrap_or(false)
        })
        .count();

    CommittedReviewReport {
        matching_count: matches.reviews.len(),
        mismatch_count: matches
            .candidate_count
            .saturating_sub(matches.reviews.len()),
        covered_file_count,
        total_file_count: current_files.len(),
    }
}

fn committed_review_note(report: &CommittedReviewReport) -> Option<DependencyNote> {
    if report.mismatch_count > 0 {
        Some(DependencyNote::ProjectReviewMismatch {
            count: report.mismatch_count,
        })
    } else {
        None
    }
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

fn dependency_security_notes(stats: &DependencyStats) -> Vec<DependencyNote> {
    let mut notes = Vec::new();
    if stats.count_critical_comments > 0 {
        notes.push(DependencyNote::CriticalFindings {
            count: stats.count_critical_comments,
        });
    }

    if stats.count_medium_comments > 0 {
        notes.push(DependencyNote::MediumFindings {
            count: stats.count_medium_comments,
        });
    }

    notes
}

fn render_notes(notes: impl IntoIterator<Item = DependencyNote>) -> Option<String> {
    let parts = notes
        .into_iter()
        .map(DependencyNote::render)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
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

        assert!(review::project::review_matches_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_different_package_hash() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("different-package-hash", &[("src/lib.rs", "file-hash")])?;

        assert!(!review::project::review_matches_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_different_file_hash() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("package-hash", &[("src/lib.rs", "different-file-hash")])?;

        assert!(!review::project::review_matches_package(&review, &current));
        Ok(())
    }

    #[test]
    fn project_review_rejects_empty_target_set() -> Result<()> {
        let current = package_record("package-hash", &[("src/lib.rs", "file-hash")]);
        let review = stored_review("package-hash", &[])?;

        assert!(!review::project::review_matches_package(&review, &current));
        Ok(())
    }

    #[test]
    fn committed_review_report_counts_matching_mismatched_and_covered_files() -> Result<()> {
        let current = package_record(
            "package-hash",
            &[("src/lib.rs", "file-hash"), ("README.md", "readme-hash")],
        );
        assert_eq!(
            committed_review_report(
                &current,
                &review::project::ProjectReviewMatches {
                    candidate_count: 2,
                    reviews: vec![stored_review(
                        "package-hash",
                        &[("src/lib.rs", "file-hash")]
                    )?],
                },
            ),
            CommittedReviewReport {
                matching_count: 1,
                mismatch_count: 1,
                covered_file_count: 1,
                total_file_count: 2,
            }
        );
        Ok(())
    }

    #[test]
    fn committed_review_note_reports_mismatches_only() -> Result<()> {
        assert_eq!(
            committed_review_note(&CommittedReviewReport {
                matching_count: 0,
                mismatch_count: 0,
                covered_file_count: 0,
                total_file_count: 2,
            }),
            None
        );
        assert_eq!(
            committed_review_note(&CommittedReviewReport {
                matching_count: 1,
                mismatch_count: 0,
                covered_file_count: 1,
                total_file_count: 2,
            }),
            None
        );
        assert_eq!(
            committed_review_note(&CommittedReviewReport {
                matching_count: 0,
                mismatch_count: 1,
                covered_file_count: 0,
                total_file_count: 2,
            }),
            Some(DependencyNote::ProjectReviewMismatch { count: 1 })
        );
        assert_eq!(
            render_notes(committed_review_note(&CommittedReviewReport {
                matching_count: 0,
                mismatch_count: 1,
                covered_file_count: 0,
                total_file_count: 2,
            })),
            Some("committed review mismatch (1)".to_string())
        );
        Ok(())
    }

    #[test]
    fn committed_review_report_counts_missing_committed_reviews() {
        let current = package_record(
            "package-hash",
            &[("src/lib.rs", "file-hash"), ("README.md", "readme-hash")],
        );
        assert_eq!(
            committed_review_report(
                &current,
                &review::project::ProjectReviewMatches {
                    candidate_count: 0,
                    reviews: Vec::new(),
                },
            ),
            CommittedReviewReport {
                matching_count: 0,
                mismatch_count: 0,
                covered_file_count: 0,
                total_file_count: 2,
            }
        );
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
                    agent_run_metrics: None,
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
