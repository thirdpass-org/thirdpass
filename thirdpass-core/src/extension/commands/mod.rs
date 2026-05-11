use super::common::Extension;
use anyhow::Result;
use structopt::{self, StructOpt};

mod common;
pub mod identify_file_defined_dependencies;
pub mod identify_package_dependencies;
pub mod registries_package_metadata;
mod static_data;

#[derive(Debug, StructOpt, Clone)]
enum Command {
    /// Get extension static data.
    #[structopt(name = "static-data")]
    StaticData,

    /// Identify package dependencies.
    #[structopt(name = identify_package_dependencies::COMMAND_NAME)]
    IdentifyPackageDependencies(identify_package_dependencies::Arguments),

    /// Identify file defined dependencies.
    #[structopt(name = identify_file_defined_dependencies::COMMAND_NAME)]
    IdentifyFileDefinedDependencies(identify_file_defined_dependencies::Arguments),

    /// Get package metadata from registries.
    #[structopt(name = registries_package_metadata::COMMAND_NAME)]
    RegistriesPackageMetadata(registries_package_metadata::Arguments),
}

fn run_command<T: Extension + std::fmt::Debug>(command: Command, extension: &mut T) -> Result<()> {
    match command {
        Command::StaticData => {
            static_data::run_command(extension)?;
        }

        Command::IdentifyPackageDependencies(args) => {
            identify_package_dependencies::run_command(&args, extension)?;
        }

        Command::IdentifyFileDefinedDependencies(args) => {
            identify_file_defined_dependencies::run_command(&args, extension)?;
        }

        Command::RegistriesPackageMetadata(args) => {
            registries_package_metadata::run_command(&args, extension)?;
        }
    }
    Ok(())
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(about = "Package Code Reviews")]
#[structopt(global_setting = structopt::clap::AppSettings::ColoredHelp)]
#[structopt(global_setting = structopt::clap::AppSettings::DeriveDisplayOrder)]
struct Opts {
    #[structopt(subcommand)]
    pub command: Command,
}

pub fn run<T: Extension + std::fmt::Debug>(extension: &mut T) -> Result<()> {
    let commands = Opts::from_args();
    match run_command(commands.command, extension) {
        Ok(_) => {}
        Err(_e) => std::process::exit(-2),
    };
    Ok(())
}
