use anyhow::{format_err, Result};
use serde::Deserialize;
use structopt::{self, StructOpt};

use crate::common;

/// Administrative commands for operating a Vouch server.
#[derive(Debug, StructOpt, Clone)]
pub enum Subcommands {
    /// Move a submitted review into server-side quarantine.
    #[structopt(name = "quarantine-review")]
    QuarantineReview(QuarantineReviewArguments),
}

/// Arguments for quarantining one submitted review record.
#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct QuarantineReviewArguments {
    /// Server-assigned review id to quarantine.
    #[structopt(name = "review-id")]
    pub review_id: String,

    /// API base URL. Defaults to core.api-base from local config.
    #[structopt(long = "api-base", value_name = "url")]
    pub api_base: Option<String>,

    /// Admin key. Defaults to the VOUCH_ADMIN_KEY environment variable.
    #[structopt(long = "admin-key", value_name = "key")]
    pub admin_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QuarantineResponse {
    id: String,
    already_quarantined: bool,
}

pub fn run_subcommand(subcommand: &Subcommands) -> Result<()> {
    match subcommand {
        Subcommands::QuarantineReview(args) => {
            log::info!("Running command: admin quarantine-review");
            quarantine_review(args)?;
        }
    }
    Ok(())
}

fn quarantine_review(args: &QuarantineReviewArguments) -> Result<()> {
    let config = common::config::Config::load()?;
    let api_base = args
        .api_base
        .as_deref()
        .unwrap_or(config.core.api_base.as_str());
    let admin_key = admin_key(args)?;

    let client = reqwest::blocking::Client::new();
    let base = common::api::normalize_base(api_base)?;
    let mut url = common::api::join(&base, "v1/reviews/")?;
    url.path_segments_mut()
        .map_err(|_| format_err!("API base URL cannot be used for path segments."))?
        .push(&args.review_id)
        .push("quarantine");

    let response = client
        .post(url)
        .header("User-Agent", common::HTTP_USER_AGENT)
        .bearer_auth(admin_key)
        .send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format_err!(
            "Failed to quarantine review ({}): {}",
            status,
            body
        ));
    }

    let response = response.json::<QuarantineResponse>()?;
    if response.already_quarantined {
        println!("Review already quarantined: {}", response.id);
    } else {
        println!("Quarantined review: {}", response.id);
    }
    Ok(())
}

fn admin_key(args: &QuarantineReviewArguments) -> Result<String> {
    args.admin_key
        .clone()
        .or_else(|| std::env::var("VOUCH_ADMIN_KEY").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format_err!("Admin key is required. Pass --admin-key or set VOUCH_ADMIN_KEY.")
        })
}
