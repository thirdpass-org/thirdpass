use anyhow::{format_err, Result};
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;
use crate::review;

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Review a dependency discovered from the current project."
)]
pub struct Arguments {
    /// Restrict dependency discovery to specific extension names (repeatable).
    /// Example values: py, js, rs.
    #[structopt(long = "extension", short = "e", name = "name")]
    pub extension_names: Option<Vec<String>>,

    /// Run manual review in VS Code instead of an automated agent review.
    #[structopt(long = "manual", hidden = true)]
    pub manual: bool,

    /// Select review agent (`codex` or `claude`). Persists as default.
    #[structopt(long = "agent", value_name = "agent")]
    pub agent: Option<String>,

    /// Set default model for Codex runs. Persists as default.
    #[structopt(long = "agent-model", value_name = "model")]
    pub agent_model: Option<String>,

    /// Set default reasoning effort for Codex runs. Persists as default.
    #[structopt(long = "agent-reasoning-effort", value_name = "effort")]
    pub agent_reasoning_effort: Option<String>,

    /// Use local target selection and save the review locally without submission.
    #[structopt(long = "local-only")]
    pub local_only: bool,
}

pub fn run_command(args: &Arguments, extension_args: &[String]) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let submitter = if args.local_only {
        None
    } else {
        Some(review::submission::Submitter::start()?)
    };
    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;
    let working_directory = std::env::current_dir()?;
    let discovery = discover_local_review_dependencies(
        &extensions,
        extension_args,
        &working_directory,
        &config,
    )?;

    if discovery.candidates.is_empty() {
        return Err(format_err!(
            "No reviewable dependencies found in the current directory."
        ));
    }

    run_discovered_dependency_reviews(
        args,
        &extensions,
        &working_directory,
        discovery,
        &config.core.public_user_id,
        submitter.as_ref(),
    )
}

pub(crate) fn run_package_command(
    args: &crate::command::review::Arguments,
    extension_args: &[String],
) -> Result<()> {
    if args.plan_only {
        crate::command::require_debug_cli("--plan-only")?;
    }

    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let submitter = if args.local_only || args.plan_only {
        None
    } else {
        Some(review::submission::Submitter::start()?)
    };
    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;
    let working_directory = std::env::current_dir()?;

    run_package_command_with_runner(
        args,
        extension_args,
        &extensions,
        &working_directory,
        &config,
        submitter.as_ref(),
        &CommandDependencyReviewRunner,
    )
}

fn run_package_command_with_runner(
    args: &crate::command::review::Arguments,
    extension_args: &[String],
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    working_directory: &std::path::Path,
    config: &common::config::Config,
    submitter: Option<&review::submission::Submitter>,
    runner: &dyn DependencyReviewRunner,
) -> Result<()> {
    let discovery = discover_package_review_dependencies(
        &args.package_name,
        &args.package_version,
        extensions,
        extension_args,
        &config,
    )?;

    if discovery.candidates.is_empty() {
        return Err(format_err!(
            "No reviewable dependencies found for package {}.",
            args.package_name
        ));
    }

    let dependency_args = Arguments {
        extension_names: args.extension_names.clone(),
        manual: args.manual,
        agent: args.agent.clone(),
        agent_model: args.agent_model.clone(),
        agent_reasoning_effort: args.agent_reasoning_effort.clone(),
        local_only: args.local_only,
    };

    if args.plan_only {
        return run_discovered_dependency_review_plan(extensions, working_directory, discovery);
    }

    run_discovered_dependency_reviews_with_runner(
        &dependency_args,
        extensions,
        working_directory,
        discovery,
        &config.core.public_user_id,
        submitter,
        runner,
    )
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DependencyReviewCandidate {
    extension_name: String,
    registry_host_name: String,
    package_name: String,
    package_version: String,
    current_reviewer_review_count: usize,
    total_review_count: usize,
}

impl DependencyReviewCandidate {
    fn review_package(&self) -> review::dependency_plan::DependencyReviewPackage {
        review::dependency_plan::DependencyReviewPackage {
            extension_name: self.extension_name.clone(),
            registry_host_name: self.registry_host_name.clone(),
            package_name: self.package_name.clone(),
            package_version: self.package_version.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DependencyReviewDiscovery {
    dependency_files: Vec<std::path::PathBuf>,
    candidates: Vec<DependencyReviewCandidate>,
}

#[derive(Default)]
struct DependencyReviewSession {
    completed_reviews: usize,
    reviewed_files: usize,
    accepted_submissions: usize,
    submission_tickets: Vec<review::submission::Ticket>,
}

impl DependencyReviewSession {
    fn record(&mut self, outcome: &crate::command::review::ReviewCommandOutcome) {
        self.completed_reviews += 1;
        self.reviewed_files += outcome.target_file_count;
        if outcome.submitted {
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

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct DependencyReviewKey {
    extension_name: String,
    registry_host_name: String,
    package_name: String,
    package_version: String,
}

/// Executes a selected dependency review batch.
trait DependencyReviewRunner {
    /// Run one review command for selected package files.
    fn run(
        &self,
        args: &crate::command::review::Arguments,
        submitter: Option<&review::submission::Submitter>,
    ) -> Result<crate::command::review::ReviewCommandResult>;
}

/// Production runner backed by the normal review command implementation.
struct CommandDependencyReviewRunner;

impl DependencyReviewRunner for CommandDependencyReviewRunner {
    fn run(
        &self,
        args: &crate::command::review::Arguments,
        submitter: Option<&review::submission::Submitter>,
    ) -> Result<crate::command::review::ReviewCommandResult> {
        crate::command::review::run_command_with_result(args, submitter)
    }
}

fn run_discovered_dependency_reviews(
    args: &Arguments,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    working_directory: &std::path::Path,
    discovery: DependencyReviewDiscovery,
    public_user_id: &str,
    submitter: Option<&review::submission::Submitter>,
) -> Result<()> {
    run_discovered_dependency_reviews_with_runner(
        args,
        extensions,
        working_directory,
        discovery,
        public_user_id,
        submitter,
        &CommandDependencyReviewRunner,
    )
}

fn run_discovered_dependency_review_plan(
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

fn run_discovered_dependency_reviews_with_runner(
    args: &Arguments,
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
            &crate::command::review::Arguments {
                package_name: selection.package_name,
                package_version: Some(selection.package_version),
                extension_names: Some(vec![selection.extension_name]),
                target_files: selection.target_files,
                deps: false,
                plan_only: false,
                manual: args.manual,
                agent: args.agent.clone(),
                agent_model: args.agent_model.clone(),
                agent_reasoning_effort: args.agent_reasoning_effort.clone(),
                submit_existing: false,
                local_only: args.local_only,
            },
            submitter,
        )?;
        let project_review_path =
            review::project::store_dependency_review(working_directory, &result.review)?;
        println!("Project review saved: {}.", project_review_path.display());
        plan.mark_batch_reviewed(plan_rank)?;
        session.record(&result.outcome);
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

fn project_review_summary_lines(
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

fn discover_local_review_dependencies(
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
    working_directory: &std::path::Path,
    config: &common::config::Config,
) -> Result<DependencyReviewDiscovery> {
    let all_dependencies = extension::identify_file_defined_dependencies(
        extensions,
        extension_args,
        working_directory,
    )?;

    let stored_reviews = review::fs::list()?;
    let mut dependency_files = std::collections::BTreeSet::<std::path::PathBuf>::new();
    let mut candidates =
        std::collections::BTreeMap::<DependencyReviewKey, DependencyReviewCandidate>::new();

    for (extension, extension_dependencies) in extensions.iter().zip(all_dependencies.into_iter()) {
        let extension_dependencies = match extension_dependencies {
            Ok(dependencies) => dependencies,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };

        for dependency_file in extension_dependencies {
            dependency_files.insert(dependency_file.path.clone());
            for dependency in dependency_file.dependencies {
                insert_dependency_candidate(
                    &mut candidates,
                    extension.name(),
                    dependency_file.registry_host_name.clone(),
                    dependency,
                    &stored_reviews,
                    &config.core.public_user_id,
                );
            }
        }
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    sort_dependency_review_candidates(&mut candidates);
    Ok(DependencyReviewDiscovery {
        dependency_files: dependency_files.into_iter().collect(),
        candidates,
    })
}

fn discover_package_review_dependencies(
    package_name: &str,
    package_version: &Option<String>,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
    config: &common::config::Config,
) -> Result<DependencyReviewDiscovery> {
    let package_version = package_version.as_deref();
    let all_dependencies = extension::identify_package_dependencies(
        package_name,
        &package_version,
        extensions,
        extension_args,
    )?;

    let stored_reviews = review::fs::list()?;
    let mut candidates =
        std::collections::BTreeMap::<DependencyReviewKey, DependencyReviewCandidate>::new();

    for (extension, extension_dependencies) in extensions.iter().zip(all_dependencies.into_iter()) {
        let extension_dependencies = match extension_dependencies {
            Ok(dependencies) => dependencies,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };

        for package_dependencies in extension_dependencies {
            insert_dependency_candidate(
                &mut candidates,
                extension.name(),
                package_dependencies.registry_host_name.clone(),
                thirdpass_core::extension::Dependency {
                    name: package_name.to_string(),
                    version: package_dependencies.package_version,
                },
                &stored_reviews,
                &config.core.public_user_id,
            );
            for dependency in package_dependencies.dependencies {
                insert_dependency_candidate(
                    &mut candidates,
                    extension.name(),
                    package_dependencies.registry_host_name.clone(),
                    dependency,
                    &stored_reviews,
                    &config.core.public_user_id,
                );
            }
        }
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    sort_dependency_review_candidates(&mut candidates);
    Ok(DependencyReviewDiscovery {
        dependency_files: Vec::new(),
        candidates,
    })
}

fn insert_dependency_candidate(
    candidates: &mut std::collections::BTreeMap<DependencyReviewKey, DependencyReviewCandidate>,
    extension_name: String,
    registry_host_name: String,
    dependency: thirdpass_core::extension::Dependency,
    stored_reviews: &[review::Review],
    public_user_id: &str,
) {
    let package_version = match dependency.version {
        Ok(package_version) => package_version,
        Err(error) => {
            log::debug!(
                "Skipping dependency {} because version is not reviewable: {}",
                dependency.name,
                error
            );
            return;
        }
    };
    let key = DependencyReviewKey {
        extension_name,
        registry_host_name,
        package_name: dependency.name,
        package_version,
    };
    let (current_reviewer_review_count, total_review_count) =
        count_matching_reviews(&key, stored_reviews, public_user_id);
    candidates
        .entry(key.clone())
        .or_insert_with(|| DependencyReviewCandidate {
            extension_name: key.extension_name,
            registry_host_name: key.registry_host_name,
            package_name: key.package_name,
            package_version: key.package_version,
            current_reviewer_review_count,
            total_review_count,
        });
}

fn count_matching_reviews(
    candidate: &DependencyReviewKey,
    reviews: &[review::Review],
    public_user_id: &str,
) -> (usize, usize) {
    let mut current_reviewer_review_count = 0;
    let mut total_review_count = 0;
    for review in reviews {
        if !matches_dependency_candidate(candidate, review) {
            continue;
        }

        total_review_count += 1;
        if review.reviewer_details.public_user_id == public_user_id {
            current_reviewer_review_count += 1;
        }
    }
    (current_reviewer_review_count, total_review_count)
}

fn matches_dependency_candidate(candidate: &DependencyReviewKey, review: &review::Review) -> bool {
    review.package.name == candidate.package_name
        && review.package.version == candidate.package_version
        && review
            .package
            .registries
            .iter()
            .any(|registry| registry.host_name == candidate.registry_host_name)
}

fn sort_dependency_review_candidates(candidates: &mut [DependencyReviewCandidate]) {
    candidates.sort_by(|a, b| {
        a.current_reviewer_review_count
            .cmp(&b.current_reviewer_review_count)
            .then_with(|| a.total_review_count.cmp(&b.total_review_count))
            .then_with(|| a.registry_host_name.cmp(&b.registry_host_name))
            .then_with(|| a.package_name.cmp(&b.package_name))
            .then_with(|| a.package_version.cmp(&b.package_version))
            .then_with(|| a.extension_name.cmp(&b.extension_name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common;
    use crate::test_support::{DependencyReviewFixture, FixtureExtension};
    use crate::{package, peer, registry};
    use std::cell::RefCell;

    #[test]
    fn sort_dependency_review_candidates_prefers_review_needs() {
        let mut candidates = vec![
            candidate("js", "npmjs.com", "covered-by-user", "1.0.0", 1, 1),
            candidate("js", "npmjs.com", "globally-covered", "1.0.0", 0, 2),
            candidate("js", "npmjs.com", "uncovered", "1.0.0", 0, 0),
        ];

        sort_dependency_review_candidates(&mut candidates);

        let names = candidates
            .iter()
            .map(|candidate| candidate.package_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["uncovered", "globally-covered", "covered-by-user"]
        );
    }

    #[test]
    fn count_matching_reviews_counts_current_reviewer_and_total() -> Result<()> {
        let candidate = DependencyReviewKey {
            extension_name: "js".to_string(),
            registry_host_name: "npmjs.com".to_string(),
            package_name: "left-pad".to_string(),
            package_version: "1.3.0".to_string(),
        };
        let reviews = vec![
            stored_review("user-a", "npmjs.com", "left-pad", "1.3.0")?,
            stored_review("user-b", "npmjs.com", "left-pad", "1.3.0")?,
            stored_review("user-a", "npmjs.com", "left-pad", "1.2.0")?,
            stored_review("user-a", "pypi.org", "left-pad", "1.3.0")?,
        ];

        assert_eq!(
            count_matching_reviews(&candidate, &reviews, "user-a"),
            (1, 2)
        );
        Ok(())
    }

    #[test]
    fn command_parses_review_deps_args() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-deps",
                "--extension",
                "js",
                "--agent",
                "codex",
                "--local-only",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::ReviewDeps(args) => {
                assert_eq!(args.extension_names, Some(vec!["js".to_string()]));
                assert_eq!(args.agent.as_deref(), Some("codex"));
                assert!(args.local_only);
            }
            _ => panic!("Expected review-deps command."),
        }
    }

    #[test]
    fn command_rejects_review_deps_package_args() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-deps",
                "axum",
                "0.8.9",
                "--extension",
                "rs",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "review-deps should not accept package positionals"
        );
    }

    #[test]
    fn insert_dependency_candidate_adds_review_counts() -> Result<()> {
        let mut candidates = std::collections::BTreeMap::new();
        let reviews = vec![
            stored_review("user-a", "crates.io", "axum", "0.8.9")?,
            stored_review("user-b", "crates.io", "axum", "0.8.9")?,
        ];

        insert_dependency_candidate(
            &mut candidates,
            "rs".to_string(),
            "crates.io".to_string(),
            thirdpass_core::extension::Dependency {
                name: "axum".to_string(),
                version: Ok("0.8.9".to_string()),
            },
            &reviews,
            "user-a",
        );

        let candidate = candidates.values().next().expect("candidate was not added");
        assert_eq!(candidate.package_name, "axum");
        assert_eq!(candidate.package_version, "0.8.9");
        assert_eq!(candidate.current_reviewer_review_count, 1);
        assert_eq!(candidate.total_review_count, 2);
        Ok(())
    }

    #[test]
    fn command_rejects_removed_review_deps_coordination_flags() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-deps",
                "--skip-coordination",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "removed skip-coordination flag should be rejected"
        );

        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&["thirdpass", "review-deps", "--no-submit"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "removed no-submit flag should be rejected"
        );
    }

    #[test]
    fn review_deps_reuses_committed_project_reviews_without_running_review() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-review-deps-e2e-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review()?;

        run_discovered_dependency_reviews(
            &Arguments {
                extension_names: Some(vec!["fixture".to_string()]),
                manual: false,
                agent: None,
                agent_model: None,
                agent_reasoning_effort: None,
                local_only: true,
            },
            &[Box::new(FixtureExtension::new(&fixture))],
            fixture.project_root(),
            fixture_discovery(&fixture, 0, 0),
            "current-user",
            None,
        )?;

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

        run_discovered_dependency_reviews(
            &Arguments {
                extension_names: Some(vec!["fixture".to_string()]),
                manual: false,
                agent: None,
                agent_model: None,
                agent_reasoning_effort: None,
                local_only: true,
            },
            &[Box::new(FixtureExtension::new(&fixture))],
            fixture.project_root(),
            fixture_discovery(&fixture, 1, 1),
            "current-user",
            None,
        )?;

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

        run_discovered_dependency_reviews_with_runner(
            &Arguments {
                extension_names: Some(vec!["fixture".to_string()]),
                manual: false,
                agent: None,
                agent_model: None,
                agent_reasoning_effort: None,
                local_only: true,
            },
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
    fn review_deps_package_command_reaches_plan_for_py_and_ansible() -> Result<()> {
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
            assert_package_deps_command_reaches_plan(case)?;
        }
        Ok(())
    }

    #[test]
    fn review_deps_package_plan_only_does_not_run_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let _debug_env =
            crate::test_support::ScopedEnv::set_var(crate::command::DEBUG_CLI_ENV_VAR, "1");
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

        let args = parse_package_review_deps_args(case, true);
        let mut config = common::config::Config::default();
        config.core.public_user_id = "current-user".to_string();
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(PackageDependencyExtension { case })];
        let runner = RecordingPackageCommandRunner::new(case);

        run_package_command_with_runner(
            &args,
            &[],
            &extensions,
            fixture.project_root(),
            &config,
            None,
            &runner,
        )?;

        assert!(runner.calls().is_empty());
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

    fn candidate(
        extension_name: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
        current_reviewer_review_count: usize,
        total_review_count: usize,
    ) -> DependencyReviewCandidate {
        DependencyReviewCandidate {
            extension_name: extension_name.to_string(),
            registry_host_name: registry_host_name.to_string(),
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            current_reviewer_review_count,
            total_review_count,
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
            args: &crate::command::review::Arguments,
            submitter: Option<&review::submission::Submitter>,
        ) -> Result<crate::command::review::ReviewCommandResult> {
            assert!(submitter.is_none());
            self.calls.borrow_mut().push(args.target_files.clone());

            let target_paths = args
                .target_files
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let review = self
                .fixture
                .review_for_files("current-user", &target_paths)?;
            let outcome = crate::command::review::ReviewCommandOutcome {
                package_name: review.package.name.clone(),
                package_version: review.package.version.clone(),
                target_file_count: review.targets.len(),
                comment_count: 0,
                critical_comment_count: 0,
                medium_comment_count: 0,
                low_comment_count: 0,
                submitted: false,
            };

            Ok(crate::command::review::ReviewCommandResult {
                review,
                outcome,
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

    fn assert_package_deps_command_reaches_plan(case: PackageDependencyCliCase) -> Result<()> {
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

        let args = parse_package_review_deps_args(case, false);
        assert!(args.deps);
        assert!(!args.plan_only);
        let extension_names = args
            .extension_names
            .as_ref()
            .expect("package review args should include one extension");
        assert_eq!(
            extension_names.as_slice(),
            &[case.extension_name.to_string()]
        );

        let mut config = common::config::Config::default();
        config.core.public_user_id = "current-user".to_string();
        let extensions: Vec<Box<dyn thirdpass_core::extension::Extension>> =
            vec![Box::new(PackageDependencyExtension { case })];
        let runner = RecordingPackageCommandRunner::new(case);

        run_package_command_with_runner(
            &args,
            &[],
            &extensions,
            fixture.project_root(),
            &config,
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

    fn parse_package_review_deps_args(
        case: PackageDependencyCliCase,
        plan_only: bool,
    ) -> crate::command::review::Arguments {
        let mut argv = vec![
            "thirdpass",
            "review",
            case.package_name,
            case.package_version,
            "--deps",
            "--extension",
            case.extension_name,
            "--local-only",
        ];
        if plan_only {
            argv.push("--plan-only");
        }

        let parsed = std::panic::catch_unwind(|| crate::command::Opts::from_iter_safe(&argv));

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::Review(args) => args,
            _ => panic!("Expected review command."),
        }
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
        deps: bool,
        plan_only: bool,
        local_only: bool,
        submit_existing: bool,
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
            args: &crate::command::review::Arguments,
            submitter: Option<&review::submission::Submitter>,
        ) -> Result<crate::command::review::ReviewCommandResult> {
            assert!(submitter.is_none());
            assert!(args.agent.is_none());
            assert!(args.agent_model.is_none());
            assert!(args.agent_reasoning_effort.is_none());
            let package_version = args
                .package_version
                .as_deref()
                .expect("dependency planner should pass a concrete package version");
            self.calls.borrow_mut().push(RecordedPackageCommandCall {
                package_name: args.package_name.clone(),
                package_version: package_version.to_string(),
                extension_names: args.extension_names.clone(),
                target_files: args.target_files.clone(),
                deps: args.deps,
                plan_only: args.plan_only,
                local_only: args.local_only,
                submit_existing: args.submit_existing,
            });

            let review = package_review_for_runner(
                self.case,
                &args.package_name,
                package_version,
                &args.target_files,
            )?;
            let outcome = crate::command::review::ReviewCommandOutcome {
                package_name: review.package.name.clone(),
                package_version: review.package.version.clone(),
                target_file_count: review.targets.len(),
                comment_count: 0,
                critical_comment_count: 0,
                medium_comment_count: 0,
                low_comment_count: 0,
                submitted: false,
            };

            Ok(crate::command::review::ReviewCommandResult {
                review,
                outcome,
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
            deps: false,
            plan_only: false,
            local_only: true,
            submit_existing: false,
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

    fn stored_review(
        public_user_id: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: registry_host_name.to_string(),
            human_url: url::Url::parse("https://registry.example/pkg")?,
            artifact_url: url::Url::parse("https://registry.example/pkg.tgz")?,
        });

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: package_name.to_string(),
                version: package_version.to_string(),
                registries,
                package_hash: "package-hash".to_string(),
            },
            targets: Vec::new(),
            reviewer_details: review::ReviewerDetails {
                public_user_id: public_user_id.to_string(),
                ..Default::default()
            },
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
