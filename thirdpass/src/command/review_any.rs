use anyhow::{format_err, Result};
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;
use crate::review;

const NIGHTSHIFT_IDLE_SLEEP: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Review any assigned high-priority target."
)]
pub struct Arguments {
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

    /// Keep reviewing assigned targets until interrupted.
    #[structopt(long = "nightshift")]
    pub nightshift: bool,

    /// Restrict assigned targets to a registry host. May be repeated.
    #[structopt(long = "registry", value_name = "registry")]
    pub registry_hosts: Vec<String>,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let submitter = review::submission::Submitter::start()?;
    let supported_registry_hosts =
        review::remote::supported_registry_hosts_for_filter(&config, &args.registry_hosts)?;

    if args.nightshift {
        return run_nightshift(args, &config, &supported_registry_hosts, &submitter);
    }

    let target = review::remote::request_global_target(&config, &supported_registry_hosts)?
        .ok_or(format_err!("No review target is currently available."))?;
    let mut result = run_assigned_target(args, &config, target, None, &submitter)?;
    if let Some(ticket) = result.submission.take() {
        result.outcome.submitted = review::submission::wait_for_submission(ticket)?;
    }
    Ok(())
}

fn run_nightshift(
    args: &Arguments,
    config: &common::config::Config,
    supported_registry_hosts: &[String],
    submitter: &review::submission::Submitter,
) -> Result<()> {
    let mut session = NightshiftSession::default();
    println!("Nightshift started. Press Ctrl-C to stop.");
    println!("Looking for high-priority package files to review.");

    loop {
        match review::remote::request_global_target(config, supported_registry_hosts) {
            Ok(Some(target)) => {
                let review_number = session.completed_reviews + 1;
                let mut result =
                    run_assigned_target(args, config, target, Some(review_number), submitter)?;
                if let Some(ticket) = result.submission.take() {
                    result.outcome.submitted = review::submission::wait_for_submission(ticket)?;
                }
                session.record(&result.outcome);
                print_nightshift_progress(&session, &result.outcome);
                println!("Looking for next target.");
            }
            Ok(None) => sleep_after_idle("No review target is currently available."),
            Err(err) => {
                if review::remote::is_authentication_required_error(&err) {
                    return Err(err);
                }
                sleep_after_idle(&format!("Failed to request review target: {}", err));
            }
        }
    }
}

fn run_assigned_target(
    args: &Arguments,
    config: &common::config::Config,
    target: review::remote::ReviewCandidate,
    nightshift_review_number: Option<usize>,
    submitter: &review::submission::Submitter,
) -> Result<crate::command::review::ReviewCommandResult> {
    let extension_name = config
        .extensions
        .registries
        .get(&target.registry_host)
        .cloned()
        .ok_or(format_err!(
            "No installed extension is configured for registry: {}",
            target.registry_host
        ))?;

    let target_files = target.target_file_paths();
    let display_files = target_files.join(", ");
    print_assigned_target(&target, &display_files, nightshift_review_number);

    crate::command::review::run_command_with_result(
        &crate::command::review::Arguments {
            package_name: target.package_name,
            package_version: Some(target.package_version),
            extension_names: Some(vec![extension_name]),
            target_files,
            deps: false,
            plan_only: false,
            manual: args.manual,
            agent: args.agent.clone(),
            agent_model: args.agent_model.clone(),
            agent_reasoning_effort: args.agent_reasoning_effort.clone(),
            submit_existing: false,
            local_only: false,
        },
        Some(submitter),
    )
}

fn print_assigned_target(
    target: &review::remote::ReviewCandidate,
    display_files: &str,
    nightshift_review_number: Option<usize>,
) {
    if let Some(review_number) = nightshift_review_number {
        println!();
        println!(
            "{}",
            nightshift_target_header(review_number, target, display_files)
        );
        println!();
        return;
    }

    println!(
        "Selected review target: {} {} {} ({})",
        target.package_name, target.package_version, display_files, target.registry_host
    );
}

fn nightshift_target_header(
    review_number: usize,
    target: &review::remote::ReviewCandidate,
    display_files: &str,
) -> String {
    format!(
        "Review #{}\nTarget: {}@{} ({})\nFiles: {}",
        review_number,
        target.package_name,
        target.package_version,
        target.registry_host,
        display_files
    )
}

fn sleep_after_idle(message: &str) {
    println!(
        "{} Retrying in {} seconds.",
        message,
        NIGHTSHIFT_IDLE_SLEEP.as_secs()
    );
    std::thread::sleep(NIGHTSHIFT_IDLE_SLEEP);
}

#[derive(Debug, Default)]
struct NightshiftSession {
    completed_reviews: usize,
    submitted_reviews: usize,
    reviewed_files: usize,
    shared_findings: usize,
}

impl NightshiftSession {
    fn record(&mut self, outcome: &crate::command::review::ReviewCommandOutcome) {
        self.completed_reviews += 1;
        if outcome.submitted {
            self.submitted_reviews += 1;
        }
        self.reviewed_files += outcome.target_file_count;
        if outcome.submitted {
            self.shared_findings += outcome.comment_count;
        }
    }
}

fn print_nightshift_progress(
    session: &NightshiftSession,
    outcome: &crate::command::review::ReviewCommandOutcome,
) {
    println!();
    println!("{}", nightshift_impact_line(session, outcome));
    println!("{}", nightshift_total_line(session));
    println!();
}

fn nightshift_impact_line(
    session: &NightshiftSession,
    outcome: &crate::command::review::ReviewCommandOutcome,
) -> String {
    let status = if outcome.submitted {
        format!("submitted review #{}", session.submitted_reviews)
    } else {
        format!("saved review #{}", session.completed_reviews)
    };
    format!(
        "Impact: {} for {}@{} ({}; {}).",
        status,
        outcome.package_name,
        outcome.package_version,
        pluralize(outcome.target_file_count, "file reviewed", "files reviewed"),
        review_finding_summary(outcome)
    )
}

fn nightshift_total_line(session: &NightshiftSession) -> String {
    format!(
        "Nightshift total: {}, {}, {}.",
        pluralize(
            session.submitted_reviews,
            "review submitted",
            "reviews submitted"
        ),
        pluralize(session.reviewed_files, "file reviewed", "files reviewed"),
        pluralize(session.shared_findings, "finding shared", "findings shared")
    )
}

fn review_finding_summary(outcome: &crate::command::review::ReviewCommandOutcome) -> String {
    if outcome.comment_count == 0 {
        return "clean coverage added".to_string();
    }

    let mut severities = Vec::new();
    push_count(
        &mut severities,
        outcome.critical_comment_count,
        "critical finding",
        "critical findings",
    );
    push_count(
        &mut severities,
        outcome.medium_comment_count,
        "medium finding",
        "medium findings",
    );
    push_count(
        &mut severities,
        outcome.low_comment_count,
        "low finding",
        "low findings",
    );

    format!(
        "{}: {}",
        pluralize(outcome.comment_count, "finding", "findings"),
        severities.join(", ")
    )
}

fn push_count(parts: &mut Vec<String>, count: usize, singular: &str, plural: &str) {
    if count > 0 {
        parts.push(pluralize(count, singular, plural));
    }
}

fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {}", singular)
    } else {
        format!("{} {}", count, plural)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parses_review_any_args() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-any",
                "--agent",
                "codex",
                "--agent-model",
                "gpt-5.4",
                "--agent-reasoning-effort",
                "high",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::ReviewAny(args) => {
                assert_eq!(args.agent.as_deref(), Some("codex"));
                assert_eq!(args.agent_model.as_deref(), Some("gpt-5.4"));
                assert_eq!(args.agent_reasoning_effort.as_deref(), Some("high"));
                assert!(!args.nightshift);
                assert!(args.registry_hosts.is_empty());
            }
            _ => panic!("Expected review-any command."),
        }
    }

    #[test]
    fn command_parses_review_any_registry_args() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-any",
                "--registry",
                "crates.io",
                "--registry",
                "pypi.org",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::ReviewAny(args) => {
                assert_eq!(args.registry_hosts, vec!["crates.io", "pypi.org"]);
            }
            _ => panic!("Expected review-any command."),
        }
    }

    #[test]
    fn command_rejects_review_any_coordination_flags() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&[
                "thirdpass",
                "review-any",
                "--skip-coordination",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "review-any should reject skip-coordination"
        );

        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&["thirdpass", "review-any", "--no-submit"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "review-any should reject no-submit"
        );
    }

    #[test]
    fn command_parses_review_any_nightshift_arg() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&["thirdpass", "review-any", "--nightshift"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::ReviewAny(args) => {
                assert!(args.nightshift);
            }
            _ => panic!("Expected review-any command."),
        }
    }

    #[test]
    fn command_rejects_removed_review_any_loop_arg() {
        let parsed = std::panic::catch_unwind(|| {
            crate::command::Opts::from_iter_safe(&["thirdpass", "review-any", "--loop"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(parsed.unwrap().is_err(), "Expected --loop to fail.");
    }

    #[test]
    fn nightshift_progress_reports_clean_review_impact() {
        let outcome = OutcomeBuilder::new("d3", "4.10.0").build();
        let mut session = NightshiftSession::default();
        session.record(&outcome);

        assert_eq!(
            nightshift_impact_line(&session, &outcome),
            "Impact: submitted review #1 for d3@4.10.0 (1 file reviewed; clean coverage added)."
        );
        assert_eq!(
            nightshift_total_line(&session),
            "Nightshift total: 1 review submitted, 1 file reviewed, 0 findings shared."
        );
    }

    #[test]
    fn nightshift_progress_reports_finding_breakdown() {
        let outcome = OutcomeBuilder::new("left-pad", "1.0.0")
            .target_file_count(2)
            .critical_findings(1)
            .medium_findings(2)
            .build();
        let mut session = NightshiftSession::default();
        session.record(&outcome);

        assert_eq!(
            nightshift_impact_line(&session, &outcome),
            "Impact: submitted review #1 for left-pad@1.0.0 (2 files reviewed; 3 findings: 1 critical finding, 2 medium findings)."
        );
        assert_eq!(
            nightshift_total_line(&session),
            "Nightshift total: 1 review submitted, 2 files reviewed, 3 findings shared."
        );
    }

    #[test]
    fn nightshift_target_header_groups_review_context() {
        let target = review::remote::ReviewCandidate {
            registry_host: "crates.io".to_string(),
            package_name: "hashbrown".to_string(),
            package_version: "0.17.1".to_string(),
            file_path: ".cargo_vcs_info.json".to_string(),
            file_paths: vec![".cargo_vcs_info.json".to_string(), "Cargo.toml".to_string()],
            package_hash: "blake3:abc".to_string(),
        };

        assert_eq!(
            nightshift_target_header(7, &target, ".cargo_vcs_info.json, Cargo.toml"),
            "Review #7\nTarget: hashbrown@0.17.1 (crates.io)\nFiles: .cargo_vcs_info.json, Cargo.toml"
        );
    }

    struct OutcomeBuilder {
        package_name: String,
        package_version: String,
        target_file_count: usize,
        critical_comment_count: usize,
        medium_comment_count: usize,
        low_comment_count: usize,
        submitted: bool,
    }

    impl OutcomeBuilder {
        fn new(package_name: &str, package_version: &str) -> Self {
            Self {
                package_name: package_name.to_string(),
                package_version: package_version.to_string(),
                target_file_count: 1,
                critical_comment_count: 0,
                medium_comment_count: 0,
                low_comment_count: 0,
                submitted: true,
            }
        }

        fn target_file_count(mut self, target_file_count: usize) -> Self {
            self.target_file_count = target_file_count;
            self
        }

        fn critical_findings(mut self, critical_comment_count: usize) -> Self {
            self.critical_comment_count = critical_comment_count;
            self
        }

        fn medium_findings(mut self, medium_comment_count: usize) -> Self {
            self.medium_comment_count = medium_comment_count;
            self
        }

        fn build(self) -> crate::command::review::ReviewCommandOutcome {
            let comment_count =
                self.critical_comment_count + self.medium_comment_count + self.low_comment_count;
            crate::command::review::ReviewCommandOutcome {
                package_name: self.package_name,
                package_version: self.package_version,
                target_file_count: self.target_file_count,
                comment_count,
                critical_comment_count: self.critical_comment_count,
                medium_comment_count: self.medium_comment_count,
                low_comment_count: self.low_comment_count,
                submitted: self.submitted,
            }
        }
    }
}
