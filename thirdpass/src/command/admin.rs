use anyhow::{format_err, Result};
use serde::Deserialize;
use structopt::{self, StructOpt};

use crate::common;

/// Administrative commands for operating a Thirdpass server.
#[derive(Debug, StructOpt, Clone)]
pub enum Subcommands {
    /// Move a submitted review into server-side quarantine.
    #[structopt(name = "quarantine-review")]
    QuarantineReview(QuarantineReviewArguments),

    /// Restore a quarantined review to active server-side storage.
    #[structopt(name = "unquarantine-review")]
    UnquarantineReview(UnquarantineReviewArguments),
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

    /// Admin key. Defaults to the THIRDPASS_ADMIN_KEY environment variable.
    #[structopt(long = "admin-key", value_name = "key")]
    pub admin_key: Option<String>,
}

/// Arguments for restoring one quarantined review record.
#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct UnquarantineReviewArguments {
    /// Server-assigned review id to restore.
    #[structopt(name = "review-id")]
    pub review_id: String,

    /// API base URL. Defaults to core.api-base from local config.
    #[structopt(long = "api-base", value_name = "url")]
    pub api_base: Option<String>,

    /// Admin key. Defaults to the THIRDPASS_ADMIN_KEY environment variable.
    #[structopt(long = "admin-key", value_name = "key")]
    pub admin_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QuarantineResponse {
    id: String,
    already_quarantined: bool,
}

#[derive(Debug, Deserialize)]
struct UnquarantineResponse {
    id: String,
    already_active: bool,
}

pub fn run_subcommand(subcommand: &Subcommands) -> Result<()> {
    match subcommand {
        Subcommands::QuarantineReview(args) => {
            log::info!("Running command: admin quarantine-review");
            quarantine_review(args)?;
        }
        Subcommands::UnquarantineReview(args) => {
            log::info!("Running command: admin unquarantine-review");
            unquarantine_review(args)?;
        }
    }
    Ok(())
}

fn quarantine_review(args: &QuarantineReviewArguments) -> Result<()> {
    let response = post_admin_review_action(
        &args.review_id,
        "quarantine",
        args.api_base.as_deref(),
        admin_key(args)?,
    )?;
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

fn unquarantine_review(args: &UnquarantineReviewArguments) -> Result<()> {
    let response = post_admin_review_action(
        &args.review_id,
        "unquarantine",
        args.api_base.as_deref(),
        admin_key(args)?,
    )?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format_err!(
            "Failed to unquarantine review ({}): {}",
            status,
            body
        ));
    }

    let response = response.json::<UnquarantineResponse>()?;
    if response.already_active {
        println!("Review already active: {}", response.id);
    } else {
        println!("Unquarantined review: {}", response.id);
    }
    Ok(())
}

fn post_admin_review_action(
    review_id: &str,
    action: &str,
    api_base: Option<&str>,
    admin_key: String,
) -> Result<reqwest::blocking::Response> {
    let config = common::config::Config::load()?;
    let api_base = api_base.unwrap_or(config.core.api_base.as_str());
    let client = reqwest::blocking::Client::new();
    let base = common::api::normalize_base(api_base)?;
    let mut url = common::api::join(&base, "v1/reviews")?;
    url.path_segments_mut()
        .map_err(|_| format_err!("API base URL cannot be used for path segments."))?
        .push(review_id)
        .push(action);

    Ok(common::api::with_client_headers(client.post(url), &config)
        .bearer_auth(admin_key)
        .send()?)
}

trait AdminArguments {
    fn admin_key(&self) -> Option<&str>;
}

impl AdminArguments for QuarantineReviewArguments {
    fn admin_key(&self) -> Option<&str> {
        self.admin_key.as_deref()
    }
}

impl AdminArguments for UnquarantineReviewArguments {
    fn admin_key(&self) -> Option<&str> {
        self.admin_key.as_deref()
    }
}

fn admin_key(args: &impl AdminArguments) -> Result<String> {
    args.admin_key()
        .map(str::to_string)
        .or_else(|| std::env::var("THIRDPASS_ADMIN_KEY").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format_err!("Admin key is required. Pass --admin-key or set THIRDPASS_ADMIN_KEY.")
        })
}
