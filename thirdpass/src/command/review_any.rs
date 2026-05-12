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

    /// Skip review submission after the assigned target is reviewed.
    #[structopt(long = "skip-coordination", alias = "no-submit")]
    pub skip_coordination: bool,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;

    let target = review::remote::request_global_target(&config)?
        .ok_or(format_err!("No review target is currently available."))?;
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
    println!(
        "Selected review target: {} {} {} ({})",
        target.package_name, target.package_version, display_files, target.registry_host
    );

    crate::command::review::run_command(&crate::command::review::Arguments {
        package_name: target.package_name,
        package_version: Some(target.package_version),
        extension_names: Some(vec![extension_name]),
        target_files,
        manual: args.manual,
        agent: args.agent.clone(),
        agent_model: args.agent_model.clone(),
        agent_reasoning_effort: args.agent_reasoning_effort.clone(),
        submit_existing: false,
        skip_coordination: args.skip_coordination,
    })
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
                "gpt-5.5",
                "--agent-reasoning-effort",
                "high",
                "--skip-coordination",
            ])
        });

        assert!(parsed.is_ok(), "CLI parsing panicked.");
        let parsed = parsed.unwrap().expect("CLI parsing failed.");
        match parsed.command {
            crate::command::Command::ReviewAny(args) => {
                assert_eq!(args.agent.as_deref(), Some("codex"));
                assert_eq!(args.agent_model.as_deref(), Some("gpt-5.5"));
                assert_eq!(args.agent_reasoning_effort.as_deref(), Some("high"));
                assert!(args.skip_coordination);
            }
            _ => panic!("Expected review-any command."),
        }
    }
}
