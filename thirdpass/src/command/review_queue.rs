use anyhow::{format_err, Context, Result};
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
    about = "Review package-version rows from a CSV queue."
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

    /// Save reviews locally without submitting them.
    #[structopt(long = "local-only")]
    pub local_only: bool,

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
    let submitter = if args.local_only || args.plan_only {
        None
    } else {
        Some(review::submission::Submitter::start_without_pending_scan()?)
    };
    let queue = read_queue(&args.csv_path)?;
    let queue_total = queue.len();
    println!("Queue rows: {}", queue_total);

    let mut reviewed_now = 0usize;
    let mut completed_rows = 0usize;
    let mut queued_submissions = 0usize;
    let mut submission_tickets = Vec::new();
    for (queue_index, row) in queue.into_iter().enumerate() {
        let queue_position = queue_index + 1;
        let pending_before_row = if submitter.is_some() {
            pending_review_paths_for_row(&row, &config).with_context(|| {
                format!(
                    "Failed to inspect pending reviews for queue row {}/{}: {} {}.",
                    queue_position, queue_total, row.package_name, row.package_version
                )
            })?
        } else {
            Vec::new()
        };

        loop {
            let batch = review_command::local_package_review_batch(
                &row.package_name,
                &row.package_version,
                &extension_names,
                &config,
            )
            .with_context(|| {
                format!(
                    "Failed to prepare review batch for queue row {}/{}: {} {}.",
                    queue_position, queue_total, row.package_name, row.package_version
                )
            })?;
            let status = &batch.status;
            if status.is_complete() {
                completed_rows += 1;
                println!(
                    "Row {}/{} complete: {} {} ({}/{} files reviewed)",
                    queue_position,
                    queue_total,
                    row.package_name,
                    row.package_version,
                    status.reviewed_file_count,
                    status.reviewable_file_count
                );
                if let Some(submitter) = submitter.as_ref() {
                    let queued = queue_pending_paths_for_submission(
                        submitter,
                        &pending_before_row,
                        &mut submission_tickets,
                    );
                    if queued > 0 {
                        println!(
                            "Queued {} pending review submission{} for row {}/{}.",
                            queued,
                            plural_suffix(queued),
                            queue_position,
                            queue_total
                        );
                    }
                    queued_submissions += queued;
                }
                break;
            }

            if args.plan_only {
                println!(
                    "Would review row {}/{}: {} {} ({}/{} files reviewed); next batch: {}",
                    queue_position,
                    queue_total,
                    row.package_name,
                    row.package_version,
                    status.reviewed_file_count,
                    status.reviewable_file_count,
                    format_target_files(&batch.target_files)
                );
                return Ok(());
            }

            if batch.target_files.is_empty() {
                return Err(format_err!(
                    "Row {} has no selected target files but is not complete.",
                    row.row_number
                ));
            }

            println!(
                "Reviewing row {}/{}: {} {} ({}/{} files reviewed); batch: {}",
                queue_position,
                queue_total,
                row.package_name,
                row.package_version,
                status.reviewed_file_count,
                status.reviewable_file_count,
                format_target_files(&batch.target_files)
            );
            let review_args = review_command::Arguments {
                package_name: row.package_name.clone(),
                package_version: Some(row.package_version.clone()),
                extension_names: args.extension_names.clone(),
                target_files: batch.target_files,
                deps: false,
                plan_only: false,
                manual: false,
                agent: args.agent.clone(),
                agent_model: args.agent_model.clone(),
                agent_reasoning_effort: args.agent_reasoning_effort.clone(),
                submit_existing: false,
                local_only: args.local_only,
            };
            let mut result =
                review_command::run_command_with_result(&review_args, submitter.as_ref())
                    .with_context(|| {
                        format!(
                            "Failed while reviewing queue row {}/{}: {} {}.",
                            queue_position, queue_total, row.package_name, row.package_version
                        )
                    })?;
            if let Some(ticket) = result.submission.take() {
                submission_tickets.push(ticket);
                queued_submissions += 1;
            }
            reviewed_now += 1;
        }
    }

    let submission_summary = review::submission::wait_for_submissions(submission_tickets)
        .context("Failed while waiting for queued review submissions.")?;

    println!(
        "Review queue complete: {} reviews run, {} submission{} queued, {} submitted, {} pending, {} rows complete.",
        reviewed_now,
        queued_submissions,
        plural_suffix(queued_submissions),
        submission_summary.submitted,
        submission_summary.failed,
        completed_rows
    );
    Ok(())
}

fn pending_review_paths_for_row(
    row: &QueueRow,
    config: &common::config::Config,
) -> Result<Vec<PathBuf>> {
    let mut pending = review::fs::list_with_status()?
        .into_iter()
        .filter(|stored| stored.status == review::fs::ReviewStorageStatus::Pending)
        .filter(|stored| review_matches_row(&stored.review, row, &config.core.public_user_id))
        .map(|stored| stored.path)
        .collect::<Vec<_>>();
    pending.sort();
    Ok(pending)
}

fn review_matches_row(review: &review::Review, row: &QueueRow, public_user_id: &str) -> bool {
    review.package.name == row.package_name
        && review.package.version == row.package_version
        && review_matches_public_user(review, public_user_id)
}

fn review_matches_public_user(review: &review::Review, public_user_id: &str) -> bool {
    let review_public_user = review.reviewer_details.public_user_id.as_str();
    if public_user_id.is_empty() {
        return review_public_user.is_empty();
    }
    review_public_user.is_empty() || review_public_user == public_user_id
}

fn queue_pending_paths_for_submission(
    submitter: &review::submission::Submitter,
    pending_paths: &[PathBuf],
    submission_tickets: &mut Vec<review::submission::Ticket>,
) -> usize {
    for path in pending_paths {
        submission_tickets.push(submitter.submit_pending_path(path.clone()));
    }
    pending_paths.len()
}

fn format_target_files(target_files: &[String]) -> String {
    if target_files.is_empty() {
        return "no files".to_string();
    }
    target_files.join(", ")
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
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
