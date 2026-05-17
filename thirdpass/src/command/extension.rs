use anyhow::{format_err, Result};
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;

#[derive(Debug, StructOpt, Clone)]
pub enum Subcommands {
    /// Enable extension.
    Enable(EnableArguments),

    /// Disable extension without deleting.
    Disable(DisableArguments),

    /// List installed extensions.
    List(ListArguments),
}

pub fn run_subcommand(subcommand: &Subcommands) -> Result<()> {
    match subcommand {
        Subcommands::Enable(args) => {
            log::info!("Running command: extension enable");
            enable(args)?;
        }
        Subcommands::Disable(args) => {
            log::info!("Running command: extension disable");
            disable(args)?;
        }
        Subcommands::List(args) => {
            log::info!("Running command: extension list");
            list(args)?;
        }
    }
    Ok(())
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct EnableArguments {
    /// Extension name.
    pub name: String,
}

fn enable(args: &EnableArguments) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;

    let name = extension::manage::clean_name(&args.name);
    let all_extension_names = extension::manage::get_all_names(&config)?;
    if !all_extension_names.contains(&name) {
        return Err(format_err!(
            "Failed to find extension. Known extensions: {}",
            all_extension_names
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    extension::manage::enable(&name, &mut config)?;
    println!("Enabled extension: {}", name);
    Ok(())
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct DisableArguments {
    /// Extension name.
    pub name: String,
}

fn disable(args: &DisableArguments) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;

    let name = extension::manage::clean_name(&args.name);
    let all_extension_names = extension::manage::get_all_names(&config)?;
    if !all_extension_names.contains(&name) {
        return Err(format_err!(
            "Failed to find extension. Known extensions: {}",
            all_extension_names
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    extension::manage::disable(&name, &mut config)?;
    println!("Disabled extension: {}", name);
    Ok(())
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct ListArguments {}

fn list(_args: &ListArguments) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    for name in extension::manage::get_all_names(&config)? {
        println!("{}", name);
    }
    Ok(())
}
