use anyhow::Result;

use super::discovery::{DependencyReviewCandidate, DependencyReviewDiscovery};
use crate::review;

/// Agent and submission options reused for each dependency review batch.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct ReviewExecutionOptions {
    /// Run manual review instead of an automated agent review.
    pub(crate) manual: bool,
    /// Selected review agent override.
    pub(crate) agent: Option<String>,
    /// Selected review agent model override.
    pub(crate) agent_model: Option<String>,
    /// Selected review agent reasoning effort override.
    pub(crate) agent_reasoning_effort: Option<String>,
    /// Save reviews locally without submission.
    pub(crate) local_only: bool,
}

/// One concrete dependency review batch selected from a plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewRunRequest {
    /// Package name to pass to the package review command.
    pub(crate) package_name: String,
    /// Concrete package version to review.
    pub(crate) package_version: String,
    /// Extension name that can retrieve this package.
    pub(crate) extension_name: String,
    /// Package-relative files selected for this batch.
    pub(crate) target_files: Vec<String>,
    /// Review execution options inherited from the dependency command.
    pub(crate) options: ReviewExecutionOptions,
}

/// Result returned by a dependency review batch runner.
pub(crate) struct DependencyReviewRunResult {
    /// Review generated for the selected package files.
    pub(crate) review: review::Review,
    /// Number of package files included in this review.
    pub(crate) target_file_count: usize,
    /// Whether the review was accepted by the configured API.
    pub(crate) submitted: bool,
    /// Background submission ticket for the saved review.
    pub(crate) submission: Option<review::submission::Ticket>,
}

/// Executes a selected dependency review batch.
pub(crate) trait DependencyReviewRunner {
    /// Run one review command for selected package files.
    fn run(
        &self,
        request: DependencyReviewRunRequest,
        submitter: Option<&review::submission::Submitter>,
    ) -> Result<DependencyReviewRunResult>;
}

#[derive(Default)]
struct DependencyReviewSession {
    completed_reviews: usize,
    reviewed_files: usize,
    accepted_submissions: usize,
    submission_tickets: Vec<review::submission::Ticket>,
}

impl DependencyReviewSession {
    fn record(&mut self, result: &DependencyReviewRunResult) {
        self.completed_reviews += 1;
        self.reviewed_files += result.target_file_count;
        if result.submitted {
            self.accepted_submissions += 1;
        }
    }

    fn track_submission(&mut self, ticket: Option<review::submission::Ticket>) {
        if let Some(ticket) = ticket {
            self.submission_tickets.push(ticket);
        }
    }

    fn queued_submission_count(&self) -> usize {
        self.submission_tickets.len()
    }

    fn wait_for_submissions(&mut self) -> Result<()> {
        let tickets = std::mem::take(&mut self.submission_tickets);
        let summary = review::submission::wait_for_submissions(tickets)?;
        self.accepted_submissions += summary.submitted;
        Ok(())
    }
}

/// Result of trying to prepare the next dependency package.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DependencyPreparationOutcome {
    /// A package was prepared and appended to the plan at this index.
    Prepared { package_index: usize },
    /// A package was skipped because it could not be prepared.
    Skipped,
}

/// Prepare all dependency packages and print a plan-only summary.
pub(crate) fn run_discovered_dependency_review_plan(
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    working_directory: &std::path::Path,
    discovery: DependencyReviewDiscovery,
) -> Result<()> {
    let review_packages = discovery
        .candidates
        .iter()
        .map(DependencyReviewCandidate::review_package)
        .collect::<Vec<_>>();
    println!(
        "Preparing dependency review plan for {} dependencies.",
        review_packages.len()
    );
    let mut plan = review::dependency_plan::plan_for_project(
        working_directory,
        &discovery.dependency_files,
        &review_packages,
    )?;
    print_plan_summary(&plan);

    while prepare_next_dependency(&mut plan, extensions)?.is_some() {}

    print_plan_only_summary(&plan);
    Ok(())
}

/// Execute discovered dependency reviews until the plan is complete.
pub(crate) fn run_discovered_dependency_reviews(
    options: &ReviewExecutionOptions,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    working_directory: &std::path::Path,
    discovery: DependencyReviewDiscovery,
    public_user_id: &str,
    submitter: Option<&review::submission::Submitter>,
    runner: &dyn DependencyReviewRunner,
) -> Result<()> {
    let review_packages = discovery
        .candidates
        .iter()
        .map(DependencyReviewCandidate::review_package)
        .collect::<Vec<_>>();
    println!(
        "Preparing dependency review plan for {} dependencies.",
        review_packages.len()
    );
    let mut plan = review::dependency_plan::plan_for_project(
        working_directory,
        &discovery.dependency_files,
        &review_packages,
    )?;
    let mut reusable_project_reviews = review::project::list_dependency_reviews(working_directory)?;
    let mut session = DependencyReviewSession::default();
    let mut last_project_review_summary =
        review::dependency_plan::DependencyProjectReviewSummary::default();
    println!("Dependency review started. Press Ctrl-C to stop.");
    print_plan_summary(&plan);

    loop {
        let selection = plan.select_next_review(public_user_id)?;
        let project_review_summary =
            plan.project_review_summary_for_reviews(&reusable_project_reviews);
        if project_review_summary != last_project_review_summary {
            print_project_review_summary(&project_review_summary);
            last_project_review_summary = project_review_summary;
        }

        let selection = match selection {
            Some(selection) => selection,
            None => match prepare_next_dependency(&mut plan, extensions)? {
                Some(DependencyPreparationOutcome::Prepared { package_index }) => {
                    let reuse_summary =
                        review::dependency_reuse::copy_matching_global_reviews_for_package(
                            working_directory,
                            &plan.packages[package_index],
                            public_user_id,
                            &mut reusable_project_reviews,
                        )?;
                    print_global_review_reuse_summary(&reuse_summary);
                    continue;
                }
                Some(DependencyPreparationOutcome::Skipped) => continue,
                None => {
                    session.wait_for_submissions()?;
                    println!("Dependency review plan complete.");
                    return Ok(());
                }
            },
        };

        let review_number = session.completed_reviews + 1;
        print_selected_batch(review_number, &selection);

        let plan_rank = selection.plan_rank;
        let result = runner.run(
            DependencyReviewRunRequest {
                package_name: selection.package_name,
                package_version: selection.package_version,
                extension_name: selection.extension_name,
                target_files: selection.target_files,
                options: options.clone(),
            },
            submitter,
        )?;
        let project_review_path =
            review::project::store_dependency_review(working_directory, &result.review)?;
        println!("Project review saved: {}.", project_review_path.display());
        plan.mark_batch_reviewed(plan_rank)?;
        session.record(&result);
        session.track_submission(result.submission);
        print_review_deps_progress(&plan, &session);
    }
}

fn print_plan_only_summary(plan: &review::dependency_plan::DependencyReviewPlan) {
    let file_count = plan
        .packages
        .iter()
        .flat_map(|package| &package.batches)
        .flat_map(|batch| &batch.files)
        .count();

    println!();
    println!("Dependency review plan prepared.");
    println!("Prepared dependencies: {}.", plan.packages.len());
    println!("Skipped dependencies: {}.", plan.skipped_packages.len());
    println!("Review batches: {}.", plan.batch_count());
    println!("Review files: {}.", file_count);

    if !plan.skipped_packages.is_empty() {
        println!("Skipped dependency details:");
        for skipped in &plan.skipped_packages {
            println!(
                "- {}@{} ({}): {}",
                skipped.package_name,
                skipped.package_version,
                skipped.registry_host,
                skipped.reason
            );
        }
    }
}

fn print_project_review_summary(summary: &review::dependency_plan::DependencyProjectReviewSummary) {
    if summary.is_empty() {
        return;
    }
    for line in project_review_summary_lines(summary) {
        println!("{}", line);
    }
}

pub(crate) fn project_review_summary_lines(
    summary: &review::dependency_plan::DependencyProjectReviewSummary,
) -> Vec<String> {
    let mut lines = Vec::new();
    if summary.matching_reviews > 0 {
        lines.push(format!(
            "Using {} committed project {}.",
            summary.matching_reviews,
            plural(summary.matching_reviews, "review", "reviews")
        ));
    }
    if summary.covered_files > 0 {
        lines.push(format!(
            "Skipping {} {} already covered by committed reviews.",
            summary.covered_files,
            plural(summary.covered_files, "file", "files")
        ));
    }
    if summary.mismatched_reviews > 0 {
        lines.push(format!(
            "{} committed project review {}.",
            summary.mismatched_reviews,
            plural(summary.mismatched_reviews, "mismatch", "mismatches")
        ));
    }
    lines
}

fn print_global_review_reuse_summary(summary: &review::dependency_reuse::GlobalReviewReuseSummary) {
    if summary.is_empty() {
        return;
    }

    println!(
        "Copied {} global {} into project reviews, covering {} {}.",
        summary.copied_reviews,
        plural(summary.copied_reviews, "review", "reviews"),
        summary.covered_files,
        plural(summary.covered_files, "file", "files")
    );
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

fn print_plan_summary(plan: &review::dependency_plan::DependencyReviewPlan) {
    println!(
        "Dependency review plan: {} dependencies, {} prepared, {} pending.",
        plan.source.dependency_count,
        plan.prepared_package_count(),
        plan.pending_package_count()
    );
    if plan.batch_count() > 0 {
        println!(
            "Ready review batches: {} total, {} reviewed, {} remaining.",
            plan.batch_count(),
            plan.reviewed_batch_count(),
            plan.remaining_batch_count()
        );
    }
}

fn prepare_next_dependency(
    plan: &mut review::dependency_plan::DependencyReviewPlan,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
) -> Result<Option<DependencyPreparationOutcome>> {
    let Some(package) = plan.next_pending_package().cloned() else {
        return Ok(None);
    };
    let package_index = plan.packages.len();
    let dependency_number = plan.prepared_package_count() + 1;
    let dependency_total = plan.source.dependency_count;

    println!();
    println!(
        "Preparing dependency {}/{}: {}@{} ({})",
        dependency_number,
        dependency_total,
        package.package_name,
        package.package_version,
        package.registry_host_name
    );
    println!("Fetching metadata, source archive, and file inventory.");

    match plan.prepare_next_package(extensions)? {
        Some(review::dependency_plan::DependencyReviewPreparation::Prepared {
            package_name,
            package_version,
            registry_host,
            batch_count,
            file_count,
            ..
        }) => {
            println!(
                "Prepared {}@{} ({}): {} batches, {} files.",
                package_name, package_version, registry_host, batch_count, file_count
            );
            Ok(Some(DependencyPreparationOutcome::Prepared {
                package_index,
            }))
        }
        Some(review::dependency_plan::DependencyReviewPreparation::Skipped {
            package_name,
            package_version,
            registry_host,
            reason,
            ..
        }) => {
            println!(
                "Skipped {}@{} ({}): {}",
                package_name, package_version, registry_host, reason
            );
            Ok(Some(DependencyPreparationOutcome::Skipped))
        }
        None => Ok(None),
    }
}

fn print_selected_batch(
    review_number: usize,
    selection: &review::dependency_plan::DependencyReviewSelection,
) {
    println!();
    println!("Review #{}", review_number);
    println!(
        "Target: {}@{} ({})",
        selection.package_name, selection.package_version, selection.registry_host
    );
    println!(
        "Plan: batch {}/{}; package batch {}; {} of {} files remaining",
        selection.plan_rank,
        selection.plan_batch_count,
        selection.package_batch_rank,
        selection.target_files.len(),
        selection.batch_file_count
    );
    println!("Files: {}", selection.target_files.join(", "));
}

fn print_review_deps_progress(
    plan: &review::dependency_plan::DependencyReviewPlan,
    session: &DependencyReviewSession,
) {
    println!(
        "Dependency review progress: {} reviewed, {} ready remaining, {} dependencies pending.",
        plan.reviewed_batch_count(),
        plan.remaining_batch_count(),
        plan.pending_package_count()
    );
    println!(
        "Session total: {} reviews completed, {} uploads accepted, {} uploads queued, {} files reviewed.",
        session.completed_reviews,
        session.accepted_submissions,
        session.queued_submission_count(),
        session.reviewed_files
    );
}

#[cfg(test)]
mod tests {
    use super::super::discovery::DependencyReviewCandidate;
    use super::*;
    use crate::common;
    use crate::package;
    use crate::peer;
    use crate::registry;
    use crate::test_support::{DependencyReviewFixture, FixtureExtension};
    use std::cell::RefCell;

    #[test]
    fn review_deps_reuses_committed_project_reviews_without_running_review() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-e2e-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review()?;
        let runner = RecordingDependencyReviewRunner::new(&fixture);

        run_discovered_dependency_reviews(
            &ReviewExecutionOptions::local_only(),
            &[Box::new(FixtureExtension::new(&fixture))],
            fixture.project_root(),
            fixture_discovery(&fixture, 0, 0),
            "current-user",
            None,
            &runner,
        )?;

        assert!(runner.calls().is_empty());
        assert_eq!(review::fs::list()?, Vec::new());
        assert_eq!(
            review::project::list_dependency_reviews(fixture.project_root())?.len(),
            1
        );
        Ok(())
    }

    #[test]
    fn review_deps_copies_matching_global_reviews_into_project() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-global-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_global_review("current-user")?;
        let runner = RecordingDependencyReviewRunner::new(&fixture);

        run_discovered_dependency_reviews(
            &ReviewExecutionOptions::local_only(),
            &[Box::new(FixtureExtension::new(&fixture))],
            fixture.project_root(),
            fixture_discovery(&fixture, 1, 1),
            "current-user",
            None,
            &runner,
        )?;

        assert!(runner.calls().is_empty());
        let global_reviews = review::fs::list()?;
        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        assert_eq!(global_reviews.len(), 1);
        assert_eq!(project_reviews.len(), 1);
        assert_eq!(
            project_reviews[0].reviewer_details.public_user_id,
            "current-user"
        );
        assert_eq!(project_reviews[0].targets.len(), 2);
        Ok(())
    }

    #[test]
    fn review_deps_reviews_only_files_not_covered_by_global_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-partial-global-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_global_review_for_files("current-user", &["README.md"])?;
        let runner = RecordingDependencyReviewRunner::new(&fixture);

        run_discovered_dependency_reviews(
            &ReviewExecutionOptions::local_only(),
            &[Box::new(FixtureExtension::new(&fixture))],
            fixture.project_root(),
            fixture_discovery(&fixture, 1, 1),
            "current-user",
            None,
            &runner,
        )?;

        assert_eq!(runner.calls(), vec![vec!["src/lib.rs".to_string()]]);

        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        assert_eq!(project_reviews.len(), 2);
        let mut target_paths = project_reviews
            .iter()
            .flat_map(|project_review| &project_review.targets)
            .map(|target| target.file_path.display().to_string())
            .collect::<Vec<_>>();
        target_paths.sort();
        assert_eq!(
            target_paths,
            vec!["README.md".to_string(), "src/lib.rs".to_string()]
        );
        Ok(())
    }

    #[test]
    fn package_dependency_workflow_reviews_root_package_and_dependencies() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        for case in [
            PackageDependencyCliCase {
                extension_name: "py",
                registry_host_name: "pypi.org",
                package_name: "sample-package",
                package_version: "1.2.3",
                dependency_name: "sample-dependency",
                dependency_version: "0.4.5",
            },
            PackageDependencyCliCase {
                extension_name: "ansible",
                registry_host_name: "galaxy.ansible.com",
                package_name: "sample.collection",
                package_version: "2.0.0",
                dependency_name: "sample.dependency",
                dependency_version: "3.0.0",
            },
        ] {
            assert_package_dependency_workflow_reviews_expected_packages(case)?;
        }
        Ok(())
    }

    #[test]
    fn package_dependency_plan_only_prepares_without_running_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let case = PackageDependencyCliCase {
            extension_name: "py",
            registry_host_name: "pypi.org",
            package_name: "sample-package",
            package_version: "1.2.3",
            dependency_name: "sample-dependency",
            dependency_version: "0.4.5",
        };
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-plan-only-")?;
        let _env = fixture.enter_client_environment();
        prepare_cached_package_workspace(
            case.registry_host_name,
            case.package_name,
            case.package_version,
        )?;
        prepare_cached_package_workspace(
            case.registry_host_name,
            case.dependency_name,
            case.dependency_version,
        )?;

        let mut config = common::config::Config::default();
        config.core.public_user_id = "current-user".to_string();
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(PackageDependencyExtension { case })];
        let discovery = super::super::discover_package_review_dependencies(
            case.package_name,
            &Some(case.package_version.to_string()),
            &extensions,
            &[],
            &config,
        )?;

        run_discovered_dependency_review_plan(&extensions, fixture.project_root(), discovery)?;

        assert!(review::project::list_dependency_reviews(fixture.project_root())?.is_empty());
        Ok(())
    }

    #[test]
    fn review_deps_reports_mismatched_committed_project_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-mismatch-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review_with_package_hash("mismatched-package-hash")?;

        let mut plan = review::dependency_plan::plan_for_project(
            fixture.project_root(),
            &[fixture.dependency_file().to_path_buf()],
            &[review::dependency_plan::DependencyReviewPackage {
                extension_name: "fixture".to_string(),
                registry_host_name: fixture.registry_host_name().to_string(),
                package_name: fixture.package_name().to_string(),
                package_version: fixture.package_version().to_string(),
            }],
        )?;
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(FixtureExtension::new(&fixture))];
        plan.prepare_next_package(&extensions)?;

        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        let summary = plan.project_review_summary_for_reviews(&project_reviews);
        assert_eq!(
            summary,
            review::dependency_plan::DependencyProjectReviewSummary {
                matching_reviews: 0,
                covered_files: 0,
                mismatched_reviews: 1,
            }
        );
        assert_eq!(
            project_review_summary_lines(&summary),
            vec!["1 committed project review mismatch.".to_string()]
        );

        let selection = plan
            .select_next_review("current-user")?
            .expect("mismatched review should not cover the package");
        let mut target_files = selection.target_files;
        target_files.sort();
        assert_eq!(
            target_files,
            vec!["README.md".to_string(), "src/lib.rs".to_string()]
        );
        Ok(())
    }

    #[test]
    fn project_review_summary_lines_report_committed_review_status() {
        let lines = project_review_summary_lines(
            &review::dependency_plan::DependencyProjectReviewSummary {
                matching_reviews: 2,
                covered_files: 1,
                mismatched_reviews: 3,
            },
        );

        assert_eq!(
            lines,
            vec![
                "Using 2 committed project reviews.".to_string(),
                "Skipping 1 file already covered by committed reviews.".to_string(),
                "3 committed project review mismatches.".to_string(),
            ]
        );
    }

    impl ReviewExecutionOptions {
        fn local_only() -> Self {
            Self {
                local_only: true,
                ..Self::default()
            }
        }
    }

    fn fixture_discovery(
        fixture: &DependencyReviewFixture,
        current_reviewer_review_count: usize,
        total_review_count: usize,
    ) -> DependencyReviewDiscovery {
        DependencyReviewDiscovery {
            dependency_files: vec![fixture.dependency_file().to_path_buf()],
            candidates: vec![DependencyReviewCandidate {
                extension_name: "fixture".to_string(),
                registry_host_name: fixture.registry_host_name().to_string(),
                package_name: fixture.package_name().to_string(),
                package_version: fixture.package_version().to_string(),
                current_reviewer_review_count,
                total_review_count,
            }],
        }
    }

    struct RecordingDependencyReviewRunner<'a> {
        fixture: &'a DependencyReviewFixture,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl<'a> RecordingDependencyReviewRunner<'a> {
        fn new(fixture: &'a DependencyReviewFixture) -> Self {
            Self {
                fixture,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl DependencyReviewRunner for RecordingDependencyReviewRunner<'_> {
        fn run(
            &self,
            request: DependencyReviewRunRequest,
            submitter: Option<&review::submission::Submitter>,
        ) -> Result<DependencyReviewRunResult> {
            assert!(submitter.is_none());
            assert!(request.options.local_only);
            self.calls.borrow_mut().push(request.target_files.clone());

            let target_paths = request
                .target_files
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let review = self
                .fixture
                .review_for_files("current-user", &target_paths)?;
            let target_file_count = review.targets.len();

            Ok(DependencyReviewRunResult {
                review,
                target_file_count,
                submitted: false,
                submission: None,
            })
        }
    }

    #[derive(Clone, Copy)]
    struct PackageDependencyCliCase {
        extension_name: &'static str,
        registry_host_name: &'static str,
        package_name: &'static str,
        package_version: &'static str,
        dependency_name: &'static str,
        dependency_version: &'static str,
    }

    fn assert_package_dependency_workflow_reviews_expected_packages(
        case: PackageDependencyCliCase,
    ) -> Result<()> {
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-package-cli-")?;
        let _env = fixture.enter_client_environment();
        prepare_cached_package_workspace(
            case.registry_host_name,
            case.package_name,
            case.package_version,
        )?;
        prepare_cached_package_workspace(
            case.registry_host_name,
            case.dependency_name,
            case.dependency_version,
        )?;

        let mut config = common::config::Config::default();
        config.core.public_user_id = "current-user".to_string();
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(PackageDependencyExtension { case })];
        let runner = RecordingPackageCommandRunner::new(case);
        let discovery = super::super::discover_package_review_dependencies(
            case.package_name,
            &Some(case.package_version.to_string()),
            &extensions,
            &[],
            &config,
        )?;

        run_discovered_dependency_reviews(
            &ReviewExecutionOptions::local_only(),
            &extensions,
            fixture.project_root(),
            discovery,
            "current-user",
            None,
            &runner,
        )?;

        let mut calls = runner.calls();
        calls.sort_by(|left, right| left.package_name.cmp(&right.package_name));
        let mut expected = vec![
            expected_package_review_call(case, case.package_name, case.package_version),
            expected_package_review_call(case, case.dependency_name, case.dependency_version),
        ];
        expected.sort_by(|left, right| left.package_name.cmp(&right.package_name));
        assert_eq!(calls, expected);
        Ok(())
    }

    fn prepare_cached_package_workspace(
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> Result<()> {
        let data_paths = common::fs::DataPaths::new()?;
        let package_path = thirdpass_core::package::unique_package_path(
            package_name,
            package_version,
            registry_host_name,
        )?;
        let package_directory = data_paths.ongoing_reviews_directory.join(package_path);
        let workspace_name = format!(
            "{}-{}",
            package_name.replace('/', "_").replace('\\', "_"),
            package_version
        );
        let workspace_path = package_directory.join(workspace_name);
        std::fs::create_dir_all(&workspace_path)?;
        std::fs::write(
            workspace_path.join("README.md"),
            package_file_contents(package_name),
        )?;

        let archive_path = package_directory.join("archive.tar.gz");
        std::fs::write(
            &archive_path,
            package_archive_contents(package_name, package_version),
        )?;
        let manifest = thirdpass_core::package::Manifest {
            workspace_path,
            manifest_path: package_directory.join("manifest.json"),
            artifact_path: archive_path,
            package_hash: package_hash_for(package_name, package_version),
        };
        std::fs::write(
            &manifest.manifest_path,
            serde_json::to_string_pretty(&manifest)?,
        )?;
        Ok(())
    }

    struct PackageDependencyExtension {
        case: PackageDependencyCliCase,
    }

    impl thirdpass_core::extension::Extension for PackageDependencyExtension {
        fn name(&self) -> String {
            self.case.extension_name.to_string()
        }

        fn registries(&self) -> Vec<String> {
            vec![self.case.registry_host_name.to_string()]
        }

        fn identify_package_dependencies(
            &self,
            _package_name: &str,
            package_version: &Option<&str>,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::PackageDependencies>> {
            Ok(vec![thirdpass_core::extension::PackageDependencies {
                package_version: Ok(package_version
                    .unwrap_or(self.case.package_version)
                    .to_string()),
                registry_host_name: self.case.registry_host_name.to_string(),
                dependencies: vec![thirdpass_core::extension::Dependency {
                    name: self.case.dependency_name.to_string(),
                    version: Ok(self.case.dependency_version.to_string()),
                }],
            }])
        }

        fn identify_file_defined_dependencies(
            &self,
            _working_directory: &std::path::Path,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::FileDefinedDependencies>> {
            Ok(Vec::new())
        }

        fn registries_package_metadata(
            &self,
            package_name: &str,
            package_version: &Option<&str>,
        ) -> Result<Vec<thirdpass_core::extension::RegistryPackageMetadata>> {
            let package_version = package_version.unwrap_or(self.case.package_version);
            Ok(vec![thirdpass_core::extension::RegistryPackageMetadata {
                registry_host_name: self.case.registry_host_name.to_string(),
                human_url: format!("https://{}/{}", self.case.registry_host_name, package_name),
                artifact_url: format!(
                    "https://{}/{}/{}.tar.gz",
                    self.case.registry_host_name, package_name, package_version
                ),
                is_primary: true,
                package_version: package_version.to_string(),
            }])
        }
    }

    #[derive(Debug, Clone, Eq, PartialEq)]
    struct RecordedPackageCommandCall {
        package_name: String,
        package_version: String,
        extension_names: Option<Vec<String>>,
        target_files: Vec<String>,
        local_only: bool,
    }

    struct RecordingPackageCommandRunner {
        case: PackageDependencyCliCase,
        calls: RefCell<Vec<RecordedPackageCommandCall>>,
    }

    impl RecordingPackageCommandRunner {
        fn new(case: PackageDependencyCliCase) -> Self {
            Self {
                case,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<RecordedPackageCommandCall> {
            self.calls.borrow().clone()
        }
    }

    impl DependencyReviewRunner for RecordingPackageCommandRunner {
        fn run(
            &self,
            request: DependencyReviewRunRequest,
            submitter: Option<&review::submission::Submitter>,
        ) -> Result<DependencyReviewRunResult> {
            assert!(submitter.is_none());
            assert!(request.options.agent.is_none());
            assert!(request.options.agent_model.is_none());
            assert!(request.options.agent_reasoning_effort.is_none());
            self.calls.borrow_mut().push(RecordedPackageCommandCall {
                package_name: request.package_name.clone(),
                package_version: request.package_version.clone(),
                extension_names: Some(vec![request.extension_name.clone()]),
                target_files: request.target_files.clone(),
                local_only: request.options.local_only,
            });

            let review = package_review_for_runner(
                self.case,
                &request.package_name,
                &request.package_version,
                &request.target_files,
            )?;
            let target_file_count = review.targets.len();

            Ok(DependencyReviewRunResult {
                review,
                target_file_count,
                submitted: false,
                submission: None,
            })
        }
    }

    fn expected_package_review_call(
        case: PackageDependencyCliCase,
        package_name: &str,
        package_version: &str,
    ) -> RecordedPackageCommandCall {
        RecordedPackageCommandCall {
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            extension_names: Some(vec![case.extension_name.to_string()]),
            target_files: vec!["README.md".to_string()],
            local_only: true,
        }
    }

    fn package_review_for_runner(
        case: PackageDependencyCliCase,
        package_name: &str,
        package_version: &str,
        target_files: &[String],
    ) -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: case.registry_host_name.to_string(),
            human_url: url::Url::parse(&format!(
                "https://{}/{}",
                case.registry_host_name, package_name
            ))?,
            artifact_url: url::Url::parse(&format!(
                "https://{}/{}/{}.tar.gz",
                case.registry_host_name, package_name, package_version
            ))?,
        });

        let targets = target_files
            .iter()
            .map(|path| review::ReviewTarget {
                file_path: path.into(),
                file_hash: Some(package_file_hash(package_name)),
                agent_summary: None,
                security_summary: Some(review::SecuritySummary::None),
                confidence: None,
                agent_run_metrics: None,
                comments: std::collections::BTreeSet::new(),
            })
            .collect::<Vec<_>>();

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: package_name.to_string(),
                version: package_version.to_string(),
                registries,
                package_hash: package_hash_for(package_name, package_version),
            },
            targets,
            reviewer_details: review::ReviewerDetails {
                public_user_id: "current-user".to_string(),
                ..review::ReviewerDetails::default()
            },
            review_configuration: None,
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::None,
            overall_security_confidence: None,
        })
    }

    fn package_file_contents(package_name: &str) -> Vec<u8> {
        format!("# {}\n", package_name).into_bytes()
    }

    fn package_file_hash(package_name: &str) -> thirdpass_core::schema::FileHash {
        thirdpass_core::schema::FileHash::blake3(
            blake3::hash(&package_file_contents(package_name))
                .to_hex()
                .as_str()
                .to_string(),
        )
    }

    fn package_archive_contents(package_name: &str, package_version: &str) -> Vec<u8> {
        format!("archive for {}@{}\n", package_name, package_version).into_bytes()
    }

    fn package_hash_for(package_name: &str, package_version: &str) -> String {
        blake3::hash(&package_archive_contents(package_name, package_version))
            .to_hex()
            .as_str()
            .to_string()
    }
}
