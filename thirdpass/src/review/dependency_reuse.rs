use anyhow::Result;
use std::path::Path;

use crate::review::{self, dependency_plan, project};

/// Global review reuse materialized into project review artifacts.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct GlobalReviewReuseSummary {
    /// Matching global reviews copied into the project checkout.
    pub(crate) copied_reviews: usize,
    /// Previously uncovered files covered by copied global reviews.
    pub(crate) covered_files: usize,
}

impl GlobalReviewReuseSummary {
    /// Return true when no global reviews were copied.
    pub(crate) fn is_empty(&self) -> bool {
        self.copied_reviews == 0 && self.covered_files == 0
    }
}

/// Copy exact matching machine-local dependency reviews into project artifacts.
pub(crate) fn copy_matching_global_reviews_for_package(
    project_root: &Path,
    package: &dependency_plan::DependencyReviewPackageRecord,
    public_user_id: &str,
    project_reviews: &mut Vec<review::Review>,
) -> Result<GlobalReviewReuseSummary> {
    let package_key = project::package_key_from_record(package);
    let mut project_coverage = project::coverage_for_reviews(project_reviews.iter());
    let mut stored_reviews = review::fs::list_with_status()?;
    stored_reviews.sort_by(|left, right| left.path.cmp(&right.path));

    let global_reviews = stored_reviews
        .into_iter()
        .filter(|stored| stored.review.reviewer_details.public_user_id == public_user_id)
        .map(|stored| stored.review)
        .collect::<Vec<_>>();
    let matches = project::matching_reviews_for_package(
        &global_reviews,
        &package.registry_host,
        &package.package_name,
        &package.package_version,
        package,
    );

    let mut summary = GlobalReviewReuseSummary::default();
    for global_review in matches.reviews {
        let review_coverage = project::coverage_for_reviews(std::iter::once(&global_review));
        let Some(review_files) = review_coverage.get(&package_key) else {
            continue;
        };
        let already_covered = project_coverage.get(&package_key);
        let newly_covered_files = review_files
            .iter()
            .filter(|file| {
                already_covered
                    .map(|files| !files.contains(file))
                    .unwrap_or(true)
            })
            .count();

        if newly_covered_files == 0 {
            continue;
        }

        project::store_dependency_review(project_root, &global_review)?;
        project::add_review_coverage(&mut project_coverage, &global_review);
        project_reviews.push(global_review);
        summary.copied_reviews += 1;
        summary.covered_files += newly_covered_files;
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common;
    use crate::test_support::{DependencyReviewFixture, FixtureExtension};

    #[test]
    fn copies_matching_global_reviews_into_project_artifacts() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-dependency-reuse-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_global_review_for_files("current-user", &["README.md"])?;

        let mut plan = dependency_plan::plan_for_project(
            fixture.project_root(),
            &[fixture.dependency_file().to_path_buf()],
            &[dependency_plan::DependencyReviewPackage {
                extension_name: "fixture".to_string(),
                registry_host_name: fixture.registry_host_name().to_string(),
                package_name: fixture.package_name().to_string(),
                package_version: fixture.package_version().to_string(),
            }],
        )?;
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(FixtureExtension::new(&fixture))];
        plan.prepare_next_package(&extensions)?;

        let mut project_reviews = Vec::new();
        let summary = copy_matching_global_reviews_for_package(
            fixture.project_root(),
            &plan.packages[0],
            "current-user",
            &mut project_reviews,
        )?;

        assert_eq!(
            summary,
            GlobalReviewReuseSummary {
                copied_reviews: 1,
                covered_files: 1,
            }
        );
        assert_eq!(project_reviews.len(), 1);

        let stored_project_reviews = project::list_dependency_reviews(fixture.project_root())?;
        assert_eq!(stored_project_reviews.len(), 1);
        assert_eq!(
            stored_project_reviews[0].targets[0]
                .file_path
                .display()
                .to_string(),
            "README.md"
        );
        Ok(())
    }
}
