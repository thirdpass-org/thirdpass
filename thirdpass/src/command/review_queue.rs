use anyhow::{format_err, Context, Result};
use std::path::PathBuf;
use structopt::{self, StructOpt};

use super::review as review_command;
use crate::common;
use crate::extension;

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
    let queue = read_queue(&args.csv_path)?;
    println!("Queue rows: {}", queue.len());

    let mut reviewed_now = 0usize;
    let mut completed_rows = 0usize;
    for row in queue {
        loop {
            let status = review_command::local_package_review_status(
                &row.package_name,
                &row.package_version,
                &extension_names,
                &config,
            )?;
            if status.is_complete() {
                completed_rows += 1;
                println!(
                    "Row {} complete: {} {} ({}/{} files reviewed)",
                    row.row_number,
                    row.package_name,
                    row.package_version,
                    status.reviewed_file_count,
                    status.reviewable_file_count
                );
                break;
            }

            if args.plan_only {
                println!(
                    "Would review row {}: {} {} ({}/{} files reviewed)",
                    row.row_number,
                    row.package_name,
                    row.package_version,
                    status.reviewed_file_count,
                    status.reviewable_file_count
                );
                return Ok(());
            }

            println!(
                "Reviewing row {}: {} {} ({}/{} files reviewed)",
                row.row_number,
                row.package_name,
                row.package_version,
                status.reviewed_file_count,
                status.reviewable_file_count
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
            reviewed_now += 1;
        }
    }

    println!(
        "Review queue complete: {} reviews run, {} rows complete.",
        reviewed_now, completed_rows
    );
    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct QueueRow {
    row_number: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
