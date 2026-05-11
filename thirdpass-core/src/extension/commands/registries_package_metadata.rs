use super::common;
use crate::extension::common::Extension;
use anyhow::Result;
use structopt::{self, StructOpt};

pub const COMMAND_NAME: &str = "registries-package-metadata";

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct Arguments {
    /// Package name.
    #[structopt(name = "package-name")]
    pub package_name: String,

    /// Package version.
    #[structopt(name = "package-version")]
    pub package_version: Option<String>,
}

pub fn run_command<T: Extension + std::fmt::Debug>(
    args: &Arguments,
    extension: &mut T,
) -> Result<()> {
    let local_dependencies =
        extension.registries_package_metadata(&args.package_name, &args.package_version.as_deref());
    common::communicate_result(local_dependencies)?;
    Ok(())
}
