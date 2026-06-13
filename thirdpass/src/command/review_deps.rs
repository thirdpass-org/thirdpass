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
    let discovery = discover_package_review_dependencies(
        &args.package_name,
        &args.package_version,
        &extensions,
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

    run_discovered_dependency_reviews(
        &dependency_args,
        &extensions,
        &working_directory,
        discovery,
        &config.core.public_user_id,
        submitter.as_ref(),
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

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct DependencyReviewKey {
    extension_name: String,
    registry_host_name: String,
    package_name: String,
    package_version: String,
}

fn run_discovered_dependency_reviews(
    args: &Arguments,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    working_directory: &std::path::Path,
    discovery: DependencyReviewDiscovery,
    public_user_id: &str,
    submitter: Option<&review::submission::Submitter>,
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
    let mut session = DependencyReviewSession::default();
    println!("Dependency review started. Press Ctrl-C to stop.");
    print_plan_summary(&plan);

    loop {
        let selection = match plan.select_next_review(public_user_id)? {
            Some(selection) => selection,
            None => {
                if prepare_next_dependency(&mut plan, extensions)? {
                    continue;
                }
                session.wait_for_submissions()?;
                println!("Dependency review plan complete.");
                return Ok(());
            }
        };

        let review_number = session.completed_reviews + 1;
        print_selected_batch(review_number, &selection);

        let plan_rank = selection.plan_rank;
        let result = crate::command::review::run_command_with_result(
            &crate::command::review::Arguments {
                package_name: selection.package_name,
                package_version: Some(selection.package_version),
                extension_names: Some(vec![selection.extension_name]),
                target_files: selection.target_files,
                deps: false,
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
) -> Result<bool> {
    let Some(package) = plan.next_pending_package().cloned() else {
        return Ok(false);
    };
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
            Ok(true)
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
            Ok(true)
        }
        None => Ok(false),
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
    use crate::{package, peer, registry};
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::{Path, PathBuf};

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
        let fixture = ReviewDepsFixture::new()?;
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
            DependencyReviewDiscovery {
                dependency_files: vec![fixture.dependency_file.clone()],
                candidates: vec![DependencyReviewCandidate {
                    extension_name: "fixture".to_string(),
                    registry_host_name: fixture.registry_host_name.clone(),
                    package_name: fixture.package_name.clone(),
                    package_version: fixture.package_version.clone(),
                    current_reviewer_review_count: 0,
                    total_review_count: 0,
                }],
            },
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

    struct ReviewDepsFixture {
        root: tempfile::TempDir,
        project: PathBuf,
        dependency_file: PathBuf,
        registry_host_name: String,
        package_name: String,
        package_version: String,
        package_hash: String,
        files: Vec<FixturePackageFile>,
    }

    impl ReviewDepsFixture {
        fn new() -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix("thirdpass-review-deps-e2e-")
                .tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir_all(&project)?;
            let dependency_file = project.join("deps.lock");
            std::fs::write(&dependency_file, "fixture-package 1.0.0\n")?;

            Ok(Self {
                root,
                project,
                dependency_file,
                registry_host_name: "fixture.registry".to_string(),
                package_name: "fixture-package".to_string(),
                package_version: "1.0.0".to_string(),
                package_hash: "fixture-package-hash".to_string(),
                files: vec![
                    FixturePackageFile {
                        path: PathBuf::from("README.md"),
                        contents: b"# Fixture\n".to_vec(),
                    },
                    FixturePackageFile {
                        path: PathBuf::from("src/lib.rs"),
                        contents: b"pub fn fixture() {}\n".to_vec(),
                    },
                ],
            })
        }

        fn enter_client_environment(&self) -> ScopedEnv {
            let client_root = self.root.path().join("client");
            ScopedEnv::set(&[
                ("HOME", client_root.join("home")),
                ("XDG_CONFIG_HOME", client_root.join("xdg-config")),
                ("XDG_DATA_HOME", client_root.join("xdg-data")),
            ])
        }

        fn project_root(&self) -> &Path {
            &self.project
        }

        fn prepare_cached_workspace(&self) -> Result<()> {
            let data_paths = common::fs::DataPaths::new()?;
            let package_path = thirdpass_core::package::unique_package_path(
                &self.package_name,
                &self.package_version,
                &self.registry_host_name,
            )?;
            let package_directory = data_paths.ongoing_reviews_directory.join(package_path);
            let workspace_path =
                package_directory.join(format!("{}-{}", self.package_name, self.package_version));
            std::fs::create_dir_all(&workspace_path)?;
            for file in &self.files {
                let path = workspace_path.join(&file.path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, &file.contents)?;
            }

            let archive_path = package_directory.join("archive.tar.gz");
            std::fs::write(&archive_path, b"stand-in archive bytes")?;
            let manifest = thirdpass_core::package::Manifest {
                workspace_path,
                manifest_path: package_directory.join("manifest.json"),
                artifact_path: archive_path,
                package_hash: self.package_hash.clone(),
            };
            let mut manifest_file = std::fs::File::create(&manifest.manifest_path)?;
            manifest_file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
            Ok(())
        }

        fn write_project_review(&self) -> Result<()> {
            review::project::store_dependency_review(self.project_root(), &self.review()?)?;
            Ok(())
        }

        fn review(&self) -> Result<review::Review> {
            let mut registries = std::collections::BTreeSet::new();
            registries.insert(registry::Registry {
                id: 0,
                host_name: self.registry_host_name.clone(),
                human_url: url::Url::parse("https://fixture.registry/fixture-package")?,
                artifact_url: url::Url::parse(
                    "https://fixture.registry/fixture-package-1.0.0.tar.gz",
                )?,
            });

            let workspace_path = self.cached_workspace_path()?;
            let targets = self
                .files
                .iter()
                .map(|file| {
                    let file_hash = thirdpass_core::package::file_blake3_digest(
                        &workspace_path.join(&file.path),
                    )?;
                    Ok(review::ReviewTarget {
                        file_path: file.path.clone(),
                        file_hash: Some(thirdpass_core::schema::FileHash::blake3(file_hash)),
                        agent_summary: None,
                        security_summary: Some(review::SecuritySummary::None),
                        confidence: None,
                        comments: std::collections::BTreeSet::new(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(review::Review {
                id: 0,
                peer: peer::Peer::default(),
                package: package::Package {
                    id: 0,
                    name: self.package_name.clone(),
                    version: self.package_version.clone(),
                    registries,
                    package_hash: self.package_hash.clone(),
                },
                targets,
                reviewer_details: review::ReviewerDetails {
                    public_user_id: "committed-reviewer".to_string(),
                    ..review::ReviewerDetails::default()
                },
                agent_summary: String::new(),
                overall_security_summary: review::SecuritySummary::None,
                overall_security_confidence: None,
            })
        }

        fn cached_workspace_path(&self) -> Result<PathBuf> {
            let data_paths = common::fs::DataPaths::new()?;
            Ok(data_paths
                .ongoing_reviews_directory
                .join(thirdpass_core::package::unique_package_path(
                    &self.package_name,
                    &self.package_version,
                    &self.registry_host_name,
                )?)
                .join(format!("{}-{}", self.package_name, self.package_version)))
        }
    }

    struct FixturePackageFile {
        path: PathBuf,
        contents: Vec<u8>,
    }

    struct FixtureExtension {
        registry_host_name: String,
        package_name: String,
        package_version: String,
    }

    impl FixtureExtension {
        fn new(fixture: &ReviewDepsFixture) -> Self {
            Self {
                registry_host_name: fixture.registry_host_name.clone(),
                package_name: fixture.package_name.clone(),
                package_version: fixture.package_version.clone(),
            }
        }
    }

    impl thirdpass_core::extension::Extension for FixtureExtension {
        fn name(&self) -> String {
            "fixture".to_string()
        }

        fn registries(&self) -> Vec<String> {
            vec![self.registry_host_name.clone()]
        }

        fn identify_package_dependencies(
            &self,
            _package_name: &str,
            _package_version: &Option<&str>,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::PackageDependencies>> {
            Ok(Vec::new())
        }

        fn identify_file_defined_dependencies(
            &self,
            _working_directory: &Path,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::FileDefinedDependencies>> {
            Ok(Vec::new())
        }

        fn registries_package_metadata(
            &self,
            package_name: &str,
            package_version: &Option<&str>,
        ) -> Result<Vec<thirdpass_core::extension::RegistryPackageMetadata>> {
            assert_eq!(package_name, self.package_name);
            assert_eq!(*package_version, Some(self.package_version.as_str()));
            Ok(vec![thirdpass_core::extension::RegistryPackageMetadata {
                registry_host_name: self.registry_host_name.clone(),
                human_url: "https://fixture.registry/fixture-package".to_string(),
                artifact_url: "https://fixture.registry/fixture-package-1.0.0.tar.gz".to_string(),
                is_primary: true,
                package_version: self.package_version.clone(),
            }])
        }
    }

    struct ScopedEnv {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl ScopedEnv {
        fn set(values: &[(&'static str, PathBuf)]) -> Self {
            let previous = values
                .iter()
                .map(|(name, _)| (*name, std::env::var_os(name)))
                .collect::<Vec<_>>();
            for (name, value) in values {
                std::env::set_var(name, value);
            }
            Self { previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (name, value) in self.previous.iter().rev() {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
