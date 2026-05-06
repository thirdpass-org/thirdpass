use anyhow::Result;
use structopt::{self, StructOpt};

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Read and update Thirdpass configuration.",
    after_help = "Examples:\n    thirdpass config get\n    thirdpass config get core.api-base\n    thirdpass config set review-tool.agent claude"
)]
pub struct Arguments {
    #[structopt(subcommand)]
    pub subcommand: Option<Subcommand>,
}

#[derive(Debug, StructOpt, Clone)]
pub enum Subcommand {
    /// Print one config value, or all values when no field is provided.
    #[structopt(
        name = "get",
        about = "Get persisted configuration values.",
        after_help = "Examples:\n    thirdpass config get\n    thirdpass config get review-tool.agent"
    )]
    Get(GetArguments),

    /// Set one persisted configuration value.
    #[structopt(
        name = "set",
        about = "Set a persisted configuration value.",
        after_help = "Example:\n    thirdpass config set review-tool.agent codex"
    )]
    Set(SetArguments),
}

#[derive(Debug, StructOpt, Clone)]
pub struct GetArguments {
    /// Config field name.
    #[structopt(name = "field")]
    pub name: Option<String>,
}

#[derive(Debug, StructOpt, Clone)]
pub struct SetArguments {
    /// Config field name.
    #[structopt(name = "field")]
    pub name: String,

    /// Config field value.
    #[structopt(name = "value")]
    pub value: String,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    let mut config = crate::common::config::Config::load()?;
    match &args.subcommand {
        Some(Subcommand::Get(get)) => {
            if let Some(name) = &get.name {
                println!("{}", config.get(&name)?);
            } else {
                println!("{}", config);
            }
        }
        Some(Subcommand::Set(set)) => {
            config.set(&set.name, &set.value)?;
            config.dump()?;
        }
        None => {
            // Default to `get` behavior for convenience.
            println!("{}", config);
        }
    }
    Ok(())
}
