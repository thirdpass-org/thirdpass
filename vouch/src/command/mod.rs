use anyhow::Result;
use structopt::{self, StructOpt};

mod check;
mod config;
mod extension;
mod review;
mod setup;

pub fn run_command(command: Command, extension_args: &Vec<String>) -> Result<()> {
    setup::ensure()?;
    match command {
        Command::Review(args) => {
            log::info!("Running command: review");
            review::run_command(&args)?;
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
    /// Review a package.
    #[structopt(name = "review")]
    Review(review::Arguments),

    /// Check dependencies against reviews.
    #[structopt(name = "check")]
    Check(check::Arguments),

    /// Configure settings.
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
