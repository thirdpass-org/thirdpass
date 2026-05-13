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
    pub skip_coordination: bool,
}

pub fn run_command(args: &Arguments, extension_args: &Vec<String>) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;

    let candidate = select_review_dependency(&extension_names, extension_args, &config)?.ok_or(
        format_err!("No reviewable dependencies found in the current directory."),
    )?;

    println!(
        "Selected dependency for review: {} {} ({})",
        candidate.package_name, candidate.package_version, candidate.registry_host_name
    );

    crate::command::review::run_command(&crate::command::review::Arguments {
        package_name: candidate.package_name,
        package_version: Some(candidate.package_version),
        extension_names: Some(vec![candidate.extension_name]),
        target_files: Vec::new(),
        manual: args.manual,
        agent: args.agent.clone(),
        agent_model: args.agent_model.clone(),
        agent_reasoning_effort: args.agent_reasoning_effort.clone(),
        submit_existing: false,
        skip_coordination: args.skip_coordination,
    })
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

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct DependencyReviewKey {
    extension_name: String,
    registry_host_name: String,
    package_name: String,
    package_version: String,
}

fn select_review_dependency(
    extension_names: &std::collections::BTreeSet<String>,
    extension_args: &Vec<String>,
    config: &common::config::Config,
) -> Result<Option<DependencyReviewCandidate>> {
    let extensions = extension::manage::get_enabled(extension_names, config)?;
    let working_directory = std::env::current_dir()?;
    let all_dependencies = extension::identify_file_defined_dependencies(
        &extensions,
        extension_args,
        &working_directory,
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

        for dependency_file in extension_dependencies {
            for dependency in dependency_file.dependencies {
                let package_version = match dependency.version {
                    Ok(package_version) => package_version,
                    Err(error) => {
                        log::debug!(
                            "Skipping dependency {} because version is not reviewable: {}",
                            dependency.name,
                            error.message()
                        );
                        continue;
                    }
                };
                let key = DependencyReviewKey {
                    extension_name: extension.name(),
                    registry_host_name: dependency_file.registry_host_name.clone(),
                    package_name: dependency.name,
                    package_version,
                };
                let (current_reviewer_review_count, total_review_count) =
                    count_matching_reviews(&key, &stored_reviews, &config.core.public_user_id);
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
        }
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    sort_dependency_review_candidates(&mut candidates);
    Ok(candidates.into_iter().next())
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

fn sort_dependency_review_candidates(candidates: &mut Vec<DependencyReviewCandidate>) {
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
                assert!(args.skip_coordination);
            }
            _ => panic!("Expected review-deps command."),
        }
    }

    #[test]
    fn command_rejects_old_review_deps_coordination_aliases() {
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
            "old skip-coordination alias should be rejected"
        );

        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&["thirdpass", "review-deps", "--no-submit"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "old no-submit alias should be rejected"
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
