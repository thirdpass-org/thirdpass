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
    let discovery = review::dependencies::discover_local_review_dependencies(
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

    review::dependencies::run_discovered_dependency_reviews(
        &dependency_execution_options(args),
        &extensions,
        &working_directory,
        discovery,
        &config.core.public_user_id,
        submitter.as_ref(),
        &CommandDependencyReviewRunner,
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
    runner: &dyn review::dependencies::DependencyReviewRunner,
) -> Result<()> {
    let discovery = review::dependencies::discover_package_review_dependencies(
        &args.package_name,
        &args.package_version,
        extensions,
        extension_args,
        config,
    )?;

    if discovery.candidates.is_empty() {
        return Err(format_err!(
            "No reviewable dependencies found for package {}.",
            args.package_name
        ));
    }

    if args.plan_only {
        return review::dependencies::run_discovered_dependency_review_plan(
            extensions,
            working_directory,
            discovery,
        );
    }

    review::dependencies::run_discovered_dependency_reviews(
        &package_dependency_execution_options(args),
        extensions,
        working_directory,
        discovery,
        &config.core.public_user_id,
        submitter,
        runner,
    )
}

struct CommandDependencyReviewRunner;

impl review::dependencies::DependencyReviewRunner for CommandDependencyReviewRunner {
    fn run(
        &self,
        request: review::dependencies::DependencyReviewRunRequest,
        submitter: Option<&review::submission::Submitter>,
    ) -> Result<review::dependencies::DependencyReviewRunResult> {
        let result = crate::command::review::run_command_with_result(
            &crate::command::review::Arguments {
                package_name: request.package_name,
                package_version: Some(request.package_version),
                extension_names: Some(vec![request.extension_name]),
                target_files: request.target_files,
                deps: false,
                plan_only: false,
                manual: request.options.manual,
                agent: request.options.agent,
                agent_model: request.options.agent_model,
                agent_reasoning_effort: request.options.agent_reasoning_effort,
                submit_existing: false,
                local_only: request.options.local_only,
            },
            submitter,
        )?;

        Ok(review::dependencies::DependencyReviewRunResult {
            target_file_count: result.outcome.target_file_count,
            submitted: result.outcome.submitted,
            review: result.review,
            submission: result.submission,
        })
    }
}

fn dependency_execution_options(args: &Arguments) -> review::dependencies::ReviewExecutionOptions {
    review::dependencies::ReviewExecutionOptions {
        manual: args.manual,
        agent: args.agent.clone(),
        agent_model: args.agent_model.clone(),
        agent_reasoning_effort: args.agent_reasoning_effort.clone(),
        local_only: args.local_only,
    }
}

fn package_dependency_execution_options(
    args: &crate::command::review::Arguments,
) -> review::dependencies::ReviewExecutionOptions {
    review::dependencies::ReviewExecutionOptions {
        manual: args.manual,
        agent: args.agent.clone(),
        agent_model: args.agent_model.clone(),
        agent_reasoning_effort: args.agent_reasoning_effort.clone(),
        local_only: args.local_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
