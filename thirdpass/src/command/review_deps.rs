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
    fn queue_package(&self) -> review::dependency_queue::DependencyQueuePackage {
        review::dependency_queue::DependencyQueuePackage {
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
    let queue_packages = discovery
        .candidates
        .iter()
        .map(DependencyReviewCandidate::queue_package)
        .collect::<Vec<_>>();
    println!(
        "Preparing dependency review queue for {} dependencies.",
        queue_packages.len()
    );
    let mut queue = review::dependency_queue::ensure_for_project(
        working_directory,
        &discovery.dependency_files,
        &queue_packages,
    )?;
    let mut session = DependencyReviewSession::default();
    println!("Dependency review started. Press Ctrl-C to stop.");
    print_queue_summary(&queue);

    loop {
        let selection = match queue.select_next_review(public_user_id)? {
            Some(selection) => selection,
            None => {
                if prepare_next_dependency(&mut queue, extensions)? {
                    continue;
                }
                session.wait_for_submissions()?;
                println!("Dependency review queue complete.");
                return Ok(());
            }
        };

        let review_number = session.completed_reviews + 1;
        print_selected_batch(review_number, &selection);

        let queue_rank = selection.queue_rank;
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
        queue.mark_batch_reviewed(queue_rank)?;
        session.record(&result.outcome);
        session.track_submission(result.submission);
        print_review_deps_progress(&queue, &session);
    }
}

fn print_queue_summary(queue: &review::dependency_queue::StoredDependencyQueue) {
    println!(
        "Dependency review queue: {} dependencies, {} prepared, {} pending at {}.",
        queue.queue.source.dependency_count,
        queue.queue.prepared_package_count(),
        queue.queue.pending_package_count(),
        queue.path.display()
    );
    if queue.queue.batch_count() > 0 {
        println!(
            "Ready review batches: {} total, {} reviewed, {} remaining.",
            queue.queue.batch_count(),
            queue.queue.reviewed_batch_count(),
            queue.queue.remaining_batch_count()
        );
    }
}

fn prepare_next_dependency(
    queue: &mut review::dependency_queue::StoredDependencyQueue,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
) -> Result<bool> {
    let Some(package) = queue.next_pending_package().cloned() else {
        return Ok(false);
    };
    let dependency_number = queue.queue.prepared_package_count() + 1;
    let dependency_total = queue.queue.source.dependency_count;

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

    match queue.prepare_next_package(extensions)? {
        Some(review::dependency_queue::DependencyQueuePreparation::Prepared {
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
        Some(review::dependency_queue::DependencyQueuePreparation::Skipped {
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
    selection: &review::dependency_queue::DependencyQueueSelection,
) {
    println!();
    println!("Review #{}", review_number);
    println!(
        "Target: {}@{} ({})",
        selection.package_name, selection.package_version, selection.registry_host
    );
    println!(
        "Queue: batch {}/{}; package batch {}; {} of {} files remaining",
        selection.queue_rank,
        selection.queue_batch_count,
        selection.package_batch_rank,
        selection.target_files.len(),
        selection.batch_file_count
    );
    println!("Files: {}", selection.target_files.join(", "));
}

fn print_review_deps_progress(
    queue: &review::dependency_queue::StoredDependencyQueue,
    session: &DependencyReviewSession,
) {
    println!(
        "Dependency review progress: {} reviewed, {} ready remaining, {} dependencies pending.",
        queue.queue.reviewed_batch_count(),
        queue.queue.remaining_batch_count(),
        queue.queue.pending_package_count()
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
    use crate::{package, peer, registry};

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
}
