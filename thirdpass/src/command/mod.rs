use anyhow::Result;
use structopt::{self, StructOpt};

mod check;
mod config;
mod extension;
mod review;
mod review_any;
mod review_deps;
mod setup;

pub fn run_command(command: Command, extension_args: &Vec<String>) -> Result<()> {
    setup::ensure()?;
    match command {
        Command::Review(args) => {
            log::info!("Running command: review");
            review::run_command(&args)?;
        }
        Command::ReviewDeps(args) => {
            log::info!("Running command: review-deps");
            review_deps::run_command(&args, &extension_args)?;
        }
        Command::ReviewAny(args) => {
            log::info!("Running command: review-any");
            review_any::run_command(&args)?;
        }
        Command::Check(args) => {
            log::info!("Running command: check");
            check::run_command(&args, &extension_args)?;
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

#[derive(Debug, StructOpt, Clone)]
pub enum Command {
    /// Review a package release and submit findings.
    #[structopt(name = "review")]
    Review(review::Arguments),

    /// Review a dependency discovered from the current project.
    #[structopt(name = "review-deps")]
    ReviewDeps(review_deps::Arguments),

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
        ] {
            assert!(
                !help.contains("--manual"),
                "manual review flag should stay hidden from CLI help:\n{}",
                help
            );
        }
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
                "gpt-5.5",
                "--agent-reasoning-effort",
                "high",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            Command::Review(args) => {
                assert_eq!(args.agent_model.as_deref(), Some("gpt-5.5"));
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
                assert!(args.skip_coordination);
            }
            _ => panic!("Expected review command."),
        }
    }

    #[test]
    fn cli_rejects_old_review_coordination_aliases() {
        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--skip-coordination"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "old skip-coordination alias should be rejected"
        );

        let parsed = std::panic::catch_unwind(|| {
            Opts::from_iter_safe(&["thirdpass", "review", "d3", "4.10.0", "--no-submit"])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        assert!(
            parsed.unwrap().is_err(),
            "old no-submit alias should be rejected"
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
            "review help should hide the old skip-coordination alias:\n{}",
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
    fn cli_rejects_legacy_config_shape() {
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
