use anyhow::Result;
use structopt::{self, StructOpt};

use crate::common;
use crate::review;
use crate::store;

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
    pub package_version: String,

    /// Target file path within the package.
    #[structopt(long = "file", name = "path")]
    pub target_file: String,

    /// Registry host name to filter results.
    #[structopt(long = "registry-host")]
    pub registry_host: Option<String>,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    let config = common::config::Config::load()?;

    let query = review::remote::ReviewQuery {
        registry_host: args.registry_host.clone(),
        package_name: Some(args.package_name.clone()),
        package_version: Some(args.package_version.clone()),
        file_path: Some(args.target_file.clone()),
    };

    let records = review::remote::fetch(&query, &config)?;

    let mut store = store::Store::from_root()?;
    let tx = store.get_transaction()?;
    let stored = review::remote::store_records(records, &config, &tx)?;
    tx.commit("Pull reviews from central API.")?;

    println!("Pulled {} reviews.", stored);
    Ok(())
}
