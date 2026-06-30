use anyhow::{format_err, Result};
use structopt::{self, StructOpt};

mod check;
mod config;
mod extension;
mod review;
mod review_any;
mod review_deps;
mod review_queue;
mod setup;

/// Environment variable that enables debug-only CLI surfaces.
pub(crate) const DEBUG_CLI_ENV_VAR: &str = "THIRDPASS_DEBUG_CLI";

/// Return true when debug-only CLI surfaces should be available.
pub(crate) fn debug_cli_enabled() -> bool {
    std::env::var(DEBUG_CLI_ENV_VAR)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Return true when debug-only CLI options should stay hidden from help.
pub(crate) fn debug_cli_hidden() -> bool {
    !debug_cli_enabled()
}

/// Reject use of a debug-only CLI surface unless explicitly enabled.
pub(crate) fn require_debug_cli(feature_name: &str) -> Result<()> {
    if debug_cli_enabled() {
        Ok(())
    } else {
        Err(format_err!(
            "{} requires {}=1.",
            feature_name,
            DEBUG_CLI_ENV_VAR
        ))
    }
}

pub fn run_command(command: Command, extension_args: &[String]) -> Result<()> {
    validate_debug_cli_usage(&command)?;
    setup::ensure()?;
    match command {
        Command::Review(args) => {
            log::info!("Running command: review");
            review::run_command(&args, extension_args)?;
        }
        Command::ReviewDeps(args) => {
            log::info!("Running command: review-deps");
            review_deps::run_command(&args, extension_args)?;
        }
        Command::ReviewQueue(args) => {
            log::info!("Running command: review-queue");
            review_queue::run_command(&args)?;
        }
        Command::ReviewAny(args) => {
            log::info!("Running command: review-any");
            review_any::run_command(&args)?;
        }
        Command::Check(args) => {
            log::info!("Running command: check");
            check::run_command(&args, extension_args)?;
        }
        Command::Config(args) => {
            log::info!("Running command: config");
            config::run_command(&args)?;
        }
        Command::Extension(args) => {
            log::info!("Running command: extension");
            extension::run_subcommand(&args)?;
        }
    }
    Ok(())
}

fn validate_debug_cli_usage(command: &Command) -> Result<()> {
    match command {
        Command::Review(args) => {
            if args.plan_only {
                require_debug_cli("--plan-only")?;
            }
        }
        Command::ReviewQueue(_) => require_debug_cli("review-queue")?,
        _ => {}
    }
    Ok(())
}

#[derive(Debug, StructOpt, Clone)]
pub enum Command {
    /// Review a package release and submit findings.
    #[structopt(name = "review")]
    Review(review::Arguments),

    /// Review a dependency discovered from the current project.
    #[structopt(name = "review-deps")]
    ReviewDeps(review_deps::Arguments),

    /// Review package-version rows from a CSV queue.
    #[structopt(
        name = "review-queue",
        setting = structopt::clap::AppSettings::Hidden
    )]
    ReviewQueue(review_queue::Arguments),

    /// Review any assigned high-priority target.
    #[structopt(name = "review-any")]
    ReviewAny(review_any::Arguments),

    /// Check dependencies against reviews.
    #[structopt(name = "check")]
    Check(check::Arguments),

    /// Read and update persisted configuration.
    #[structopt(name = "config")]
    Config(config::Arguments),

    /// Manage extensions.
    #[structopt(name = "extension")]
    Extension(extension::Subcommands),
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(about = "Package Code Reviews")]
#[structopt(global_setting = structopt::clap::AppSettings::ColoredHelp)]
#[structopt(global_setting = structopt::clap::AppSettings::DeriveDisplayOrder)]
pub struct Opts {
    #[structopt(subcommand)]
    pub command: Command,
}

#[cfg(test)]
mod tests {
    use super::*;
    use structopt::StructOpt;

    #[test]
    fn cli_builds_without_panic() {
        let result = std::panic::catch_unwind(|| Opts::clap());
        assert!(result.is_ok(), "CLI definition panicked while building.");
    }

    #[test]
    fn cli_help_hides_manual_review_flags() {
        for help in [
            short_help_for::<review::Arguments>(),
            long_help_for::<review::Arguments>(),
            short_help_for::<review_any::Arguments>(),
            long_help_for::<review_any::Arguments>(),
            short_help_for::<review_deps::Arguments>(),
            long_help_for::<review_deps::Arguments>(),
            short_help_for::<review_queue::Arguments>(),
            long_help_for::<review_queue::Arguments>(),
        ] {
            assert!(
                !help.contains("--manual"),
                "manual review flag should stay hidden from CLI help:\n{}",
                help
            );
        }
    }

    #[test]
    fn cli_help_shows_plan_only_only_with_debug_cli_enabled() {
        let _lock = crate::common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        {
            let _env = crate::test_support::ScopedEnv::remove_var(DEBUG_CLI_ENV_VAR);
            let help = long_help_for::<review::Arguments>();
            assert!(
                !help.contains("--plan-only"),
                "plan-only should stay hidden without debug CLI env:\n{}",
                help
            );
        }
        {
            let _env = crate::test_support::ScopedEnv::set_var(DEBUG_CLI_ENV_VAR, "1");
            let help = long_help_for::<review::Arguments>();
            assert!(
                help.contains("--plan-only"),
                "plan-only should be visible with debug CLI env:\n{}",
                help
            );
        }
    }

    #[test]
    fn cli_help_hides_review_queue_command() {
        let help = long_help_for::<Opts>();
        assert!(
            !help.contains("review-queue"),
            "review-queue should stay hidden from CLI help:\n{}",
            help
        );
    }

    #[test]
    fn cli_parses_review_agent_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--agent", "claude"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert_eq!(args.agent.as_deref(), Some("claude"));
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_review_agent_overrides() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&[
                "thirdpass",
                "review",
                "d3",
                "4.10.0",
                "--agent-model",
                "gpt-5.4",
                "--agent-reasoning-effort",
                "high",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert_eq!(args.agent_model.as_deref(), Some("gpt-5.4"));
                assert_eq!(args.agent_reasoning_effort.as_deref(), Some("high"));
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_submit_existing_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&[
                "thirdpass",
                "review",
                "d3",
                "4.10.0",
                "--file",
                "build/d3.js",
                "--submit-existing",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert!(args.submit_existing);
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_review_local_only_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--local-only"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert!(args.local_only);
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_review_deps_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "axum", "0.8.9", "--deps"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert_eq!(args.package_name, "axum");
                assert_eq!(args.package_version.as_deref(), Some("0.8.9"));
                assert!(args.deps);
                assert!(!args.plan_only);
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_review_deps_plan_only_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&[
                "thirdpass",
                "review",
                "axum",
                "0.8.9",
                "--deps",
                "--plan-only",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert_eq!(args.package_name, "axum");
                assert_eq!(args.package_version.as_deref(), Some("0.8.9"));
                assert!(args.deps);
                assert!(args.plan_only);
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_parses_review_queue_command() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&[
                "thirdpass",
                "review-queue",
                "queue.csv",
                "--extension",
                "py",
                "--plan-only",
                "--local-only",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::ReviewQueue(args) => {
                assert_eq!(args.csv_path, std::path::PathBuf::from("queue.csv"));
                assert_eq!(args.extension_names, Some(vec!["py".to_string()]));
                assert!(args.plan_only);
                assert!(args.local_only);
            }
            _ => panic!("Expected review-queue command."),
        }
    }

    #[test]
    fn review_queue_requires_debug_cli_env() {
        let _lock = crate::common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let _env = crate::test_support::ScopedEnv::remove_var(DEBUG_CLI_ENV_VAR);
        let command = Command::ReviewQueue(review_queue::Arguments {
            csv_path: std::path::PathBuf::from("queue.csv"),
            extension_names: Some(vec!["py".to_string()]),
            plan_only: true,
            local_only: false,
            agent: None,
            agent_model: None,
            agent_reasoning_effort: None,
        });

        let error =
            validate_debug_cli_usage(&command).expect_err("review-queue should require debug CLI");

        assert_eq!(
            error.to_string(),
            "review-queue requires THIRDPASS_DEBUG_CLI=1."
        );
    }

    #[test]
    fn cli_rejects_removed_review_coordination_flags() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--skip-coordination"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "removed skip-coordination flag should be rejected"
        );

        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--no-submit"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "removed no-submit flag should be rejected"
        );
    }

    #[test]
    fn cli_review_help_uses_local_only_name() {
        let help = long_help_for::<review::Arguments>();

        assert!(
            help.contains("--local-only"),
            "review help should show the local-only flag:\n{}",
            help
        );
        assert!(
            !help.contains("--skip-coordination"),
            "review help should not show removed skip-coordination flag:\n{}",
            help
        );
    }

    #[test]
    fn cli_parses_check_output_flag() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "check", "d3", "4.10.0", "--output", "json"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Check(args) => {
                assert_eq!(args.output, check::OutputFormat::Json);
            }
            _ => panic!("Expected check command."),
        }
    }

    #[test]
    fn cli_rejects_admin_subcommand() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "admin", "quarantine-review", "review-1"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(parsed.unwrap().is_err(), "Expected admin parsing to fail.");
    }

    #[test]
    fn cli_rejects_extension_install_commands() {
        for command in ["add", "remove"] {
            let parsed = std::panic::catch_unwind(|| {
                Opts::from_iter_safe(&["thirdpass", "extension", command, "ansible"])
            });

            assert!(parsed.is_ok(), "CLI parsing panicked.");
            assert!(
                parsed.unwrap().is_err(),
                "Expected extension {} parsing to fail.",
                command
            );
        }
    }

    #[test]
    fn cli_parses_config_get_without_field() {
        let parsed =
            std::panic::catch_unwind(|| Opts::from_iter_safe(&["thirdpass", "config", "get"]));

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Config(args) => match args.subcommand {
                Some(config::Subcommand::Get(get_args)) => {
                    assert_eq!(get_args.name, None);
                }
                _ => panic!("Expected config get command."),
            },
            _ => panic!("Expected config command."),
        }
    }

    #[test]
    fn cli_parses_config_get_with_field() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "config", "get", "review-tool.agent"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Config(args) => match args.subcommand {
                Some(config::Subcommand::Get(get_args)) => {
                    assert_eq!(get_args.name.as_deref(), Some("review-tool.agent"));
                }
                _ => panic!("Expected config get command."),
            },
            _ => panic!("Expected config command."),
        }
    }

    #[test]
    fn cli_parses_config_set() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "config", "set", "review-tool.agent", "claude"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Config(args) => match args.subcommand {
                Some(config::Subcommand::Set(set_args)) => {
                    assert_eq!(set_args.name, "review-tool.agent");
                    assert_eq!(set_args.value, "claude");
                }
                _ => panic!("Expected config set command."),
            },
            _ => panic!("Expected config command."),
        }
    }

    #[test]
    fn cli_rejects_config_without_subcommand() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "config", "core.api-base"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(parsed.unwrap().is_err(), "Expected parsing to fail.");
    }

    fn short_help_for<T: StructOpt>() -> String {
        let app = T::clap();
        let mut output = Vec::new();
        app.write_help(&mut output).expect("failed to write help");
        String::from_utf8(output).expect("help output is not UTF-8")
    }

    fn long_help_for<T: StructOpt>() -> String {
        let mut app = T::clap();
        let mut output = Vec::new();
        app.write_long_help(&mut output)
            .expect("failed to write long help");
        String::from_utf8(output).expect("long help output is not UTF-8")
    }
}
