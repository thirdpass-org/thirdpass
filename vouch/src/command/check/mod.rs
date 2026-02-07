use anyhow::Result;
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;

mod fs;
mod output;
mod package;
mod report;
mod table;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Table,
    Plain,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "plain" => Ok(OutputFormat::Plain),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!(
                "Unknown output format '{}'. Supported values: table, plain, json.",
                value
            )),
        }
    }
}

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct Arguments {
    /// Package name.
    #[structopt(name = "package-name")]
    pub package_name: Option<String>,

    /// Package version.
    #[structopt(name = "package-version", requires("package-name"))]
    pub package_version: Option<String>,

    /// Specify an extension for handling the package or dependencies.
    /// Example values: py, js, rs
    #[structopt(long = "extension", short = "e", name = "name")]
    pub extension_names: Option<Vec<String>>,

    /// Output format for dependency reports.
    #[structopt(
        long = "output",
        default_value = "table",
        possible_values = &["table", "plain", "json"]
    )]
    pub output: OutputFormat,
}

pub fn run_command(args: &Arguments, extension_args: &Vec<String>) -> Result<()> {
    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let config = config;
    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;
    let output = if args.output == OutputFormat::Table
        && !atty::is(atty::Stream::Stdout)
    {
        log::warn!(
            "Falling back to plain output because stdout is not a TTY."
        );
        OutputFormat::Plain
    } else {
        args.output
    };

    match &args.package_name {
        Some(package_name) => {
            package::report(
                &package_name,
                &args.package_version.as_deref(),
                &extension_names,
                &extension_args,
                &config,
                output,
            )?;
        }
        None => {
            fs::report(&extension_names, &extension_args, &config, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::OutputFormat;
    use std::str::FromStr;

    #[test]
    fn output_format_parses_expected_values() {
        assert_eq!(OutputFormat::from_str("table").unwrap(), OutputFormat::Table);
        assert_eq!(OutputFormat::from_str("plain").unwrap(), OutputFormat::Plain);
        assert_eq!(OutputFormat::from_str("json").unwrap(), OutputFormat::Json);
    }
}
