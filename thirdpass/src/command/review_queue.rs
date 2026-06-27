use anyhow::{format_err, Context, Result};
use std::collections::BTreeSet;
use std::path::PathBuf;
use structopt::{self, StructOpt};

use super::review as review_command;
use crate::common;
use crate::extension;
use crate::review;

const PACKAGE_NAME_COLUMN: &str = "package_name";
const PACKAGE_VERSION_COLUMN: &str = "package_version";

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Review package-version rows from a CSV queue locally."
)]
pub struct Arguments {
    /// CSV file containing package_name and package_version columns.
    #[structopt(name = "csv-path", parse(from_os_str))]
    pub csv_path: PathBuf,

    /// Restrict registry lookup to specific extension names.
    /// Example values: py, js, rs.
    #[structopt(long = "extension", short = "e", name = "name")]
    pub extension_names: Option<Vec<String>>,

    /// Print remaining queue rows without running reviews.
    #[structopt(long = "plan-only")]
    pub plan_only: bool,

    /// Select review agent (`codex` or `claude`). Persists as default.
    #[structopt(long = "agent", value_name = "agent")]
    pub agent: Option<String>,

    /// Set default model for Codex runs. Persists as default.
    #[structopt(long = "agent-model", value_name = "model")]
    pub agent_model: Option<String>,

    /// Set default reasoning effort for Codex runs. Persists as default.
    #[structopt(long = "agent-reasoning-effort", value_name = "effort")]
    pub agent_reasoning_effort: Option<String>,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    crate::command::require_debug_cli("review-queue")?;

    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;
    let registry_hosts = registry_hosts_for_extensions(&extensions)?;
    let queue = read_queue(&args.csv_path)?;
    let mut reviewed = reviewed_package_keys(
        &review::fs::list_with_status()?,
        &config.core.public_user_id,
        &registry_hosts,
    );

    let remaining = queue
        .iter()
        .filter(|row| !reviewed.contains(&row.key()))
        .count();
    println!("Queue rows: {}", queue.len());
    println!("Already reviewed locally: {}", queue.len() - remaining);
    println!("Remaining: {}", remaining);

    if args.plan_only {
        for row in queue.iter().filter(|row| !reviewed.contains(&row.key())) {
            println!(
                "Would review row {}: {} {}",
                row.row_number, row.package_name, row.package_version
            );
        }
        return Ok(());
    }

    let mut reviewed_now = 0usize;
    let mut skipped = 0usize;
    for row in queue {
        let key = row.key();
        if reviewed.contains(&key) {
            skipped += 1;
            println!(
                "Skipping row {} already reviewed locally: {} {}",
                row.row_number, row.package_name, row.package_version
            );
            continue;
        }

        println!(
            "Reviewing row {}: {} {}",
            row.row_number, row.package_name, row.package_version
        );
        let review_args = review_command::Arguments {
            package_name: row.package_name.clone(),
            package_version: Some(row.package_version.clone()),
            extension_names: args.extension_names.clone(),
            target_files: Vec::new(),
            deps: false,
            plan_only: false,
            manual: false,
            agent: args.agent.clone(),
            agent_model: args.agent_model.clone(),
            agent_reasoning_effort: args.agent_reasoning_effort.clone(),
            submit_existing: false,
            local_only: true,
        };
        review_command::run_command_with_result(&review_args, None)?;
        reviewed.insert(key);
        reviewed_now += 1;
    }

    println!(
        "Review queue complete: {} reviewed, {} skipped.",
        reviewed_now, skipped
    );
    Ok(())
}

fn registry_hosts_for_extensions(
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
) -> Result<BTreeSet<String>> {
    let registry_hosts = extensions
        .iter()
        .flat_map(|extension| extension.registries())
        .collect::<BTreeSet<_>>();
    if registry_hosts.is_empty() {
        return Err(format_err!(
            "No registry hosts found for selected extensions."
        ));
    }
    Ok(registry_hosts)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct QueueRow {
    row_number: usize,
    package_name: String,
    package_version: String,
}

impl QueueRow {
    fn key(&self) -> PackageVersionKey {
        PackageVersionKey {
            package_name: self.package_name.clone(),
            package_version: self.package_version.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct PackageVersionKey {
    package_name: String,
    package_version: String,
}

fn read_queue(path: &PathBuf) -> Result<Vec<QueueRow>> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to read CSV headers from {}", path.display()))?
        .clone();
    let package_name_index = header_index(&headers, PACKAGE_NAME_COLUMN)?;
    let package_version_index = header_index(&headers, PACKAGE_VERSION_COLUMN)?;
    let mut rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let row_number = index + 2;
        let record = record.with_context(|| {
            format!(
                "failed to read CSV row {} from {}",
                row_number,
                path.display()
            )
        })?;
        let package_name = record
            .get(package_name_index)
            .unwrap_or("")
            .trim()
            .to_string();
        let package_version = record
            .get(package_version_index)
            .unwrap_or("")
            .trim()
            .to_string();
        if package_name.is_empty() || package_version.is_empty() {
            return Err(format_err!(
                "CSV row {} must contain non-empty {} and {} values.",
                row_number,
                PACKAGE_NAME_COLUMN,
                PACKAGE_VERSION_COLUMN
            ));
        }
        rows.push(QueueRow {
            row_number,
            package_name,
            package_version,
        });
    }
    Ok(rows)
}

fn header_index(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format_err!("CSV must contain a {} column.", name))
}

fn reviewed_package_keys(
    stored_reviews: &[review::fs::StoredReview],
    public_user_id: &str,
    registry_hosts: &BTreeSet<String>,
) -> BTreeSet<PackageVersionKey> {
    stored_reviews
        .iter()
        .filter(|stored| stored.review.reviewer_details.public_user_id == public_user_id)
        .filter(|stored| review_has_registry(&stored.review, registry_hosts))
        .map(|stored| PackageVersionKey {
            package_name: stored.review.package.name.clone(),
            package_version: stored.review.package.version.clone(),
        })
        .collect()
}

fn review_has_registry(review: &review::Review, registry_hosts: &BTreeSet<String>) -> bool {
    review
        .package
        .registries
        .iter()
        .any(|registry| registry_hosts.contains(&registry.host_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, peer, registry};

    #[test]
    fn read_queue_reads_package_columns() -> Result<()> {
        let temp = tempfile::NamedTempFile::new()?;
        std::fs::write(
            temp.path(),
            "dependency_rank,package_name,package_version\n1,typing-extensions,4.15.0\n",
        )?;

        let rows = read_queue(&temp.path().to_path_buf())?;

        assert_eq!(
            rows,
            vec![QueueRow {
                row_number: 2,
                package_name: "typing-extensions".to_string(),
                package_version: "4.15.0".to_string(),
            }]
        );
        Ok(())
    }

    #[test]
    fn read_queue_rejects_missing_columns() -> Result<()> {
        let temp = tempfile::NamedTempFile::new()?;
        std::fs::write(temp.path(), "name,version\ntyping-extensions,4.15.0\n")?;

        let error = read_queue(&temp.path().to_path_buf())
            .expect_err("queue without package columns should fail");

        assert_eq!(error.to_string(), "CSV must contain a package_name column.");
        Ok(())
    }

    #[test]
    fn reviewed_package_keys_require_current_user_and_selected_registry() -> Result<()> {
        let stored_reviews = vec![
            stored_review("user-a", "pypi.org", "idna", "3.18")?,
            stored_review("user-b", "pypi.org", "certifi", "2026.6.17")?,
            stored_review("user-a", "npmjs.com", "urllib3", "2.7.0")?,
        ];
        let registry_hosts = BTreeSet::from(["pypi.org".to_string()]);

        let keys = reviewed_package_keys(&stored_reviews, "user-a", &registry_hosts);

        assert_eq!(
            keys,
            BTreeSet::from([PackageVersionKey {
                package_name: "idna".to_string(),
                package_version: "3.18".to_string(),
            }])
        );
        Ok(())
    }

    fn stored_review(
        public_user_id: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> Result<review::fs::StoredReview> {
        let mut registries = BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: registry_host_name.to_string(),
            human_url: url::Url::parse("https://registry.example/pkg")?,
            artifact_url: url::Url::parse("https://registry.example/pkg.tgz")?,
        });
        Ok(review::fs::StoredReview {
            path: PathBuf::from("review.json"),
            status: review::fs::ReviewStorageStatus::Pending,
            review: review::Review {
                id: 0,
                peer: peer::Peer::default(),
                package: package::Package {
                    id: 0,
                    name: package_name.to_string(),
                    version: package_version.to_string(),
                    registries,
                    package_hash: "package-hash".to_string(),
                },
                targets: Vec::new(),
                reviewer_details: review::ReviewerDetails {
                    public_user_id: public_user_id.to_string(),
                    ..Default::default()
                },
                agent_summary: String::new(),
                overall_security_summary: review::SecuritySummary::default(),
                overall_security_confidence: None,
            },
        })
    }
}
