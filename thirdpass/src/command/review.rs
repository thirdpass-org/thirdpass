use anyhow::{format_err, Context, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Component, Path, PathBuf};
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;

use super::review_deps;

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Review a package release and submit findings.",
    after_help = "Examples:\n    thirdpass review d3 4.10.0\n    thirdpass review d3 --extension js\n    thirdpass review d3 4.10.0 --file src/index.js --file src/color.js\n    thirdpass review d3 4.10.0 --deps\n    thirdpass review d3 4.10.0 --agent codex --agent-model gpt-5.4 --agent-reasoning-effort high\n    thirdpass review d3 4.10.0 --submit-existing\n    thirdpass review d3 4.10.0 --local-only"
)]
pub struct Arguments {
    /// Package name to review.
    #[structopt(name = "package-name")]
    pub package_name: String,

    /// Package version to review. If omitted, the latest version is used.
    #[structopt(name = "package-version")]
    pub package_version: Option<String>,

    /// Restrict registry lookup to specific extension names (repeatable).
    /// Example values: py, js, rs.
    #[structopt(long = "extension", short = "e", name = "name")]
    pub extension_names: Option<Vec<String>>,

    /// Relative file path within the package to review (repeatable).
    /// If omitted, targets are assigned automatically.
    #[structopt(long = "file", name = "path")]
    pub target_files: Vec<String>,

    /// Review this package and its resolved dependency tree.
    #[structopt(long = "deps")]
    pub deps: bool,

    /// Resolve and prepare the dependency review plan without running reviews.
    #[structopt(long = "plan-only", hidden = crate::command::debug_cli_hidden())]
    pub plan_only: bool,

    /// Run manual review in VS Code instead of an automated agent review.
    #[structopt(long = "manual", hidden = true)]
    pub manual: bool,

    /// Select review agent (`codex` or `claude`). Persists as default.
    #[structopt(long = "agent", value_name = "agent")]
    pub agent: Option<String>,

    /// Set default model for Codex runs. Persists as default.
    #[structopt(long = "agent-model", value_name = "model")]
    pub agent_model: Option<String>,

    /// Set default reasoning effort for Codex runs. Persists as default.
    #[structopt(long = "agent-reasoning-effort", value_name = "effort")]
    pub agent_reasoning_effort: Option<String>,

    /// Submit a matching local review artifact without creating a new one.
    #[structopt(long = "submit-existing")]
    pub submit_existing: bool,

    /// Use local target selection and save the review locally without submission.
    #[structopt(long = "local-only")]
    pub local_only: bool,
}

/// Summary of a completed review command.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ReviewCommandOutcome {
    /// Name of the reviewed package.
    pub(crate) package_name: String,
    /// Version of the reviewed package.
    pub(crate) package_version: String,
    /// Number of package files included in this review.
    pub(crate) target_file_count: usize,
    /// Total number of comments recorded by the review.
    pub(crate) comment_count: usize,
    /// Number of critical comments recorded by the review.
    pub(crate) critical_comment_count: usize,
    /// Number of medium comments recorded by the review.
    pub(crate) medium_comment_count: usize,
    /// Number of low comments recorded by the review.
    pub(crate) low_comment_count: usize,
    /// Whether the review was accepted by the configured API.
    pub(crate) submitted: bool,
}

impl ReviewCommandOutcome {
    fn from_review(review: &review::Review, submitted: bool) -> Self {
        let mut comment_count = 0;
        let mut critical_comment_count = 0;
        let mut medium_comment_count = 0;
        let mut low_comment_count = 0;

        for target in &review.targets {
            for comment in &target.comments {
                comment_count += 1;
                match &comment.security {
                    review::Priority::Critical => critical_comment_count += 1,
                    review::Priority::Medium => medium_comment_count += 1,
                    review::Priority::Low => low_comment_count += 1,
                }
            }
        }

        Self {
            package_name: review.package.name.clone(),
            package_version: review.package.version.clone(),
            target_file_count: review.targets.len(),
            comment_count,
            critical_comment_count,
            medium_comment_count,
            low_comment_count,
            submitted,
        }
    }
}

/// Result of a review command, including any asynchronous submission work.
pub(crate) struct ReviewCommandResult {
    /// Completed review saved by this command.
    pub(crate) review: review::Review,
    /// User-facing review outcome.
    pub(crate) outcome: ReviewCommandOutcome,
    /// Background submission ticket for the saved review.
    pub(crate) submission: Option<review::submission::Ticket>,
}

/// Local file coverage status for one package review target.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct LocalPackageReviewStatus {
    /// Package name.
    pub(crate) package_name: String,
    /// Package version.
    pub(crate) package_version: String,
    /// Number of locally reviewable package files already covered.
    pub(crate) reviewed_file_count: usize,
    /// Number of locally reviewable package files.
    pub(crate) reviewable_file_count: usize,
}

/// Local target batch selected for one package review run.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct LocalPackageReviewBatch {
    /// Local file coverage status before this batch is reviewed.
    pub(crate) status: LocalPackageReviewStatus,
    /// Package-relative target files selected for the next review.
    pub(crate) target_files: Vec<String>,
}

impl LocalPackageReviewStatus {
    /// Return true when there are no remaining local package files to review.
    pub(crate) fn is_complete(&self) -> bool {
        self.reviewed_file_count >= self.reviewable_file_count
    }
}

pub fn run_command(args: &Arguments, extension_args: &[String]) -> Result<()> {
    if args.plan_only {
        crate::command::require_debug_cli("--plan-only")?;
        if !args.deps {
            return Err(format_err!("--plan-only requires --deps."));
        }
    }

    if args.deps {
        if !args.target_files.is_empty() {
            return Err(format_err!("--deps cannot be combined with --file."));
        }
        if args.submit_existing {
            return Err(format_err!(
                "--deps cannot be combined with --submit-existing."
            ));
        }
        return review_deps::run_package_command(args, extension_args);
    }

    let submitter = review::submission::Submitter::start()?;
    let mut result = run_command_with_result(args, Some(&submitter))?;
    if let Some(ticket) = result.submission.take() {
        result.outcome.submitted = review::submission::wait_for_submission(ticket)?;
    }
    Ok(())
}

/// Run a review command and return the review plus any queued submission.
pub(crate) fn run_command_with_result(
    args: &Arguments,
    submitter: Option<&review::submission::Submitter>,
) -> Result<ReviewCommandResult> {
    // TODO: Add gpg signing.

    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    if args.deps {
        return Err(format_err!(
            "--deps can only be used through the review command entry point."
        ));
    }
    if args.plan_only {
        return Err(format_err!("--plan-only requires --deps."));
    }
    if args.submit_existing && args.local_only {
        return Err(format_err!(
            "--submit-existing cannot be combined with --local-only."
        ));
    }
    if args.manual {
        review::tool::check_manual_install(&mut config)?;
    }

    if let Some(model) = args.agent_model.as_ref() {
        config.review_tool.agent_model = Some(model.to_string());
        config.dump()?;
    }
    if let Some(effort) = args.agent_reasoning_effort.as_ref() {
        config.review_tool.agent_reasoning_effort = Some(effort.to_string());
        config.dump()?;
    }

    let override_agent = match args.agent.as_deref() {
        Some(name) => {
            let agent = review::tool::AgentKind::from_name(name).ok_or(format_err!(
                "Unknown agent '{}'. Supported values: codex, claude.",
                name
            ))?;
            Some(agent)
        }
        None => None,
    };

    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;

    let (mut review, workspace_manifest) = setup_review(
        &args.package_name,
        &args.package_version,
        &extension_names,
        &config,
    )?;
    println!(
        "Cached source archive: {}",
        workspace_manifest.artifact_path.display()
    );

    let selected_targets = if !args.target_files.is_empty() {
        thirdpass_core::package::resolve_target_paths(
            &workspace_manifest.workspace_path,
            &args.target_files,
        )?
    } else {
        select_target_files(
            &workspace_manifest.workspace_path,
            &review,
            &config,
            args.local_only,
        )?
    };

    if !args.target_files.is_empty() {
        let files = selected_targets
            .iter()
            .map(|target| target.relative_path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("Selected target files: {}", files);
    }

    if args.submit_existing {
        let target_paths = selected_targets
            .iter()
            .map(|target| target.relative_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let (
            expected_agent_name,
            expected_agent_model,
            expected_agent_reasoning_effort,
            expected_review_strategy,
            expected_scope,
        ) = if args.manual {
            (
                "manual".to_string(),
                Some("".to_string()),
                "manual".to_string(),
                review::tool::review_strategy().to_string(),
                review::ReviewScope::TargetFilePartial,
            )
        } else {
            let expected_agent_name = override_agent
                .map(|agent| agent.name().to_string())
                .or_else(|| config.review_tool.agent.clone())
                .unwrap_or_else(|| "codex".to_string());
            let expected_model = if expected_agent_name == "codex" {
                args.agent_model
                    .as_deref()
                    .or(config.review_tool.agent_model.as_deref())
                    .map(|model| model.to_string())
            } else {
                None
            };
            let expected_effort = if expected_agent_name == "codex" {
                recorded_agent_reasoning_effort(
                    &expected_agent_name,
                    args.agent_reasoning_effort
                        .as_deref()
                        .or(config.review_tool.agent_reasoning_effort.as_deref()),
                )
            } else {
                recorded_agent_reasoning_effort(&expected_agent_name, None)
            };
            (
                expected_agent_name,
                expected_model,
                expected_effort,
                review::tool::review_strategy().to_string(),
                review::ReviewScope::TargetFileFull,
            )
        };

        let criteria = LocalReviewMatchCriteria {
            package_name: &review.package.name,
            package_version: &review.package.version,
            package_hash: &review.package.package_hash,
            target_paths: &target_paths,
            expected_scope,
            expected_agent_name: &expected_agent_name,
            expected_agent_model: expected_agent_model.as_deref(),
            expected_agent_reasoning_effort: &expected_agent_reasoning_effort,
            expected_review_strategy: &expected_review_strategy,
        };

        let existing = match find_matching_local_review(&config, &criteria) {
            Ok(existing) => existing,
            Err(err) => {
                review::workspace::remove(&workspace_manifest)?;
                return Err(err);
            }
        };
        let existing = match existing {
            Some(existing) => existing,
            None => {
                review::workspace::remove(&workspace_manifest)?;
                return Err(format_err!(
                    "No matching local review found for this scope/model/effort. Run without --submit-existing first."
                ));
            }
        };

        if existing.status == review::fs::ReviewStorageStatus::Submitted {
            println!(
                "Matching review already submitted: {}",
                existing.path.display()
            );
            let outcome = ReviewCommandOutcome::from_review(&existing.review, true);
            review::workspace::remove(&workspace_manifest)?;
            return Ok(ReviewCommandResult {
                review: existing.review,
                outcome,
                submission: None,
            });
        }

        let package_label = package_target_label(&existing.review);
        if let Ok(api_base) = submission_api_base(&config) {
            println!(
                "Queueing existing review submission for {} to {}.",
                package_label, api_base
            );
        }
        let package_manifest =
            review::workspace::package_manifest(&workspace_manifest.workspace_path);
        review::workspace::remove(&workspace_manifest)?;

        let package_manifest = match package_manifest {
            Ok(package_manifest) => package_manifest,
            Err(err) => {
                report_submission_failure(&err);
                let outcome = ReviewCommandOutcome::from_review(&existing.review, false);
                return Ok(ReviewCommandResult {
                    review: existing.review,
                    outcome,
                    submission: None,
                });
            }
        };

        let outcome = ReviewCommandOutcome::from_review(&existing.review, false);
        let submission = submitter.map(|submitter| {
            submitter.submit(
                existing.path.clone(),
                existing.review.clone(),
                package_manifest,
                config.clone(),
            )
        });
        return Ok(ReviewCommandResult {
            review: existing.review,
            outcome,
            submission,
        });
    }

    let config_agent_model = config.review_tool.agent_model.clone();
    let config_agent_reasoning_effort = config.review_tool.agent_reasoning_effort.clone();

    let (
        targets,
        agent_name,
        agent_model,
        agent_reasoning_effort,
        review_strategy,
        review_scope,
        agent_summary,
    ) = if args.manual {
        if override_agent.is_some() {
            review::tool::select_agent(&mut config, override_agent)?;
        }
        let comments = run_manual_review(&review, &workspace_manifest.workspace_path, &config)?;
        let targets = build_targets_from_comments(&selected_targets, comments);
        (
            targets,
            "manual".to_string(),
            "".to_string(),
            "manual".to_string(),
            review::tool::review_strategy().to_string(),
            review::ReviewScope::TargetFilePartial,
            String::new(),
        )
    } else {
        let agent = review::tool::select_agent(&mut config, override_agent)?;
        let (effective_agent_model, effective_agent_reasoning_effort) =
            if agent == review::tool::AgentKind::Codex {
                (
                    args.agent_model
                        .as_deref()
                        .or(config_agent_model.as_deref()),
                    args.agent_reasoning_effort
                        .as_deref()
                        .or(config_agent_reasoning_effort.as_deref()),
                )
            } else {
                (None, None)
            };
        let agent_token = format_agent_token(
            agent,
            effective_agent_model,
            effective_agent_reasoning_effort,
        );
        println!("Review agent: {}", agent_token);
        let mut targets = Vec::new();
        let mut agent_model = None::<String>;
        let mut agent_summary = String::new();

        for target in &selected_targets {
            let target_display = target.relative_path.display().to_string();
            let spinner = ProgressBar::new_spinner();
            let spinner_style = ProgressStyle::with_template("{spinner} {msg}")
                .context("Failed to configure review progress indicator.")?
                .tick_strings(&["|", "/", "-", "\\"]);
            spinner.set_style(spinner_style);
            spinner.enable_steady_tick(std::time::Duration::from_millis(120));
            spinner.set_message(format!("Reviewing {}", style(&target_display).dim()));
            let agent_run = review::tool::run_agent(
                agent,
                &workspace_manifest.workspace_path,
                &target_display,
                effective_agent_model,
                effective_agent_reasoning_effort,
            );
            let agent_run = match agent_run {
                Ok(agent_run) => {
                    spinner.finish_with_message(format!("Reviewed {}", target_display));
                    agent_run
                }
                Err(err) => {
                    spinner.abandon_with_message(format!("Failed {}", target_display));
                    return Err(err);
                }
            };
            agent_model = match agent_model {
                None => Some(agent_run.model.clone()),
                Some(current) if current == agent_run.model => Some(current),
                Some(_) => Some("mixed".to_string()),
            };
            let file_agent_summary = agent_run
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
                .map(ToOwned::to_owned);
            if let Some(summary) = file_agent_summary.as_deref() {
                if !agent_summary.is_empty() {
                    agent_summary.push('\n');
                }
                agent_summary.push_str(summary);
            }
            let file_confidence = agent_run.confidence;
            let comments = validate_agent_comments_for_target(
                normalize_comments(agent_run.comments),
                &workspace_manifest.workspace_path,
                &target.relative_path,
            )?;
            let security_summary = review::security_summary_for_comments(&comments);
            targets.push(review::ReviewTarget {
                file_path: target.relative_path.clone(),
                file_hash: Some(target.file_hash.clone()),
                agent_summary: file_agent_summary,
                security_summary: Some(security_summary),
                confidence: file_confidence,
                comments,
            });
        }

        (
            targets,
            agent.name().to_string(),
            agent_model.unwrap_or_else(|| "unknown".to_string()),
            recorded_agent_reasoning_effort(agent.name(), effective_agent_reasoning_effort),
            review::tool::review_strategy().to_string(),
            review::ReviewScope::TargetFileFull,
            agent_summary,
        )
    };

    review.targets = targets;
    review.reviewer_details = build_reviewer_details(
        &config,
        &agent_name,
        &agent_model,
        &agent_reasoning_effort,
        &review_strategy,
        review_scope,
    )?;
    review.agent_summary = agent_summary;
    review.overall_security_summary = review::overall_security_summary(&review)?;
    review.overall_security_confidence = None;

    let pending_review_path = review::store_pending(&review)?;
    println!("Review saved.");

    let mut submission = None;
    if !args.local_only {
        let package_label = package_target_label(&review);
        if let Ok(api_base) = submission_api_base(&config) {
            println!(
                "Queueing review submission for {} to {}.",
                package_label, api_base
            );
        }
        match review::workspace::package_manifest(&workspace_manifest.workspace_path) {
            Ok(package_manifest) => {
                submission = submitter.map(|submitter| {
                    submitter.submit(
                        pending_review_path.clone(),
                        review.clone(),
                        package_manifest,
                        config.clone(),
                    )
                });
            }
            Err(err) => report_submission_failure(&err),
        }
    }

    review::workspace::remove(&workspace_manifest)?;

    let outcome = ReviewCommandOutcome::from_review(&review, false);
    Ok(ReviewCommandResult {
        review,
        outcome,
        submission,
    })
}

/// Select the next bounded local review batch for one package.
pub(crate) fn local_package_review_batch(
    package_name: &str,
    package_version: &str,
    extension_names: &std::collections::BTreeSet<String>,
    config: &common::config::Config,
) -> Result<LocalPackageReviewBatch> {
    let package_version = Some(package_version.to_string());
    let (review, workspace_manifest) =
        setup_review(package_name, &package_version, extension_names, config)?;
    let result = local_package_review_batch_for_workspace(
        &review,
        &workspace_manifest.workspace_path,
        config,
    );
    let cleanup_result = review::workspace::remove(&workspace_manifest);
    match (result, cleanup_result) {
        (Ok(batch), Ok(())) => Ok(batch),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

fn local_package_review_batch_for_workspace(
    review: &review::Review,
    workspace_path: &std::path::Path,
    config: &common::config::Config,
) -> Result<LocalPackageReviewBatch> {
    let candidates = local_candidate_files_for_workspace(review, workspace_path, config)?;
    let status = status_for_candidates(review, &candidates);
    let target_files = if status.is_complete() {
        Vec::new()
    } else {
        select_local_candidate_batch(&candidates)
    };
    Ok(LocalPackageReviewBatch {
        status,
        target_files,
    })
}

fn local_candidate_files_for_workspace(
    review: &review::Review,
    workspace_path: &std::path::Path,
    config: &common::config::Config,
) -> Result<Vec<thirdpass_core::package::CandidateFile>> {
    let analysis = review::workspace::analyse(workspace_path)?;
    let registry = review
        .package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    let locally_reviewed_paths =
        get_locally_reviewed_target_paths(review, config, &registry.host_name)?;
    let mut target_policies = review::remote::review_target_policies(config)?;
    let target_policy = target_policies
        .remove(&registry.host_name)
        .unwrap_or_default();
    Ok(thirdpass_core::package::candidate_files_with_policy(
        &analysis,
        &locally_reviewed_paths,
        &target_policy,
    ))
}

fn status_for_candidates(
    review: &review::Review,
    candidates: &[thirdpass_core::package::CandidateFile],
) -> LocalPackageReviewStatus {
    LocalPackageReviewStatus {
        package_name: review.package.name.clone(),
        package_version: review.package.version.clone(),
        reviewed_file_count: candidates
            .iter()
            .filter(|candidate| candidate.already_reviewed)
            .count(),
        reviewable_file_count: candidates.len(),
    }
}

fn select_local_candidate_batch(
    candidates: &[thirdpass_core::package::CandidateFile],
) -> Vec<String> {
    let mut selected = Vec::new();
    let mut selected_lines = 0usize;
    for candidate in candidates
        .iter()
        .filter(|candidate| !candidate.already_reviewed)
    {
        if selected.len() >= thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_FILES {
            break;
        }
        if !selected.is_empty()
            && selected_lines + candidate.line_count
                > thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_LINES
        {
            break;
        }

        selected.push(candidate.relative_path.display().to_string());
        selected_lines += candidate.line_count;
        if selected_lines >= thirdpass_core::package::DEFAULT_REVIEW_BATCH_MAX_LINES {
            break;
        }
    }
    selected
}

/// Parse user comments from active review file and insert into index.
fn get_comments(
    active_review_file: &std::path::PathBuf,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let comments = review::active::parse(active_review_file)?;
    Ok(normalize_comments(comments))
}

fn run_manual_review(
    review: &review::Review,
    workspace_path: &std::path::Path,
    config: &common::config::Config,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let reviews_directory = review::tool::ensure_reviews_directory(workspace_path)?;
    let active_review_file = review::active::ensure(review, &reviews_directory)?;

    println!("Starting review tool.");
    review::tool::run_manual(workspace_path, config)?;
    if !active_review_file.exists() {
        println!("Review file not found.");
        return Ok(std::collections::BTreeSet::new());
    }
    let comments = get_comments(&active_review_file)?;
    println!(
        "Review tool closed. Found {} review comments.",
        comments.len()
    );
    Ok(comments)
}

fn normalize_comments<I>(comments: I) -> std::collections::BTreeSet<review::comment::Comment>
where
    I: IntoIterator<Item = review::comment::Comment>,
{
    let mut normalized = std::collections::BTreeSet::<review::comment::Comment>::new();
    for mut comment in comments {
        comment.id = 0;
        normalized.insert(comment);
    }
    normalized
}

fn validate_agent_comments_for_target<I>(
    comments: I,
    workspace_path: &Path,
    target_relative_path: &Path,
) -> Result<std::collections::BTreeSet<review::comment::Comment>>
where
    I: IntoIterator<Item = review::comment::Comment>,
{
    let expected_path = normalize_relative_review_path(target_relative_path).ok_or(format_err!(
        "Selected target path is not workspace-relative: {}",
        target_relative_path.display()
    ))?;
    let mut normalized = std::collections::BTreeSet::<review::comment::Comment>::new();

    for mut comment in comments {
        let original_path = comment.path.clone();
        let comment_path = normalize_agent_comment_path(workspace_path, &comment.path)?;
        if comment_path != expected_path {
            return Err(format_err!(
                "Agent reported a finding for {}, but the selected target is {}. Agent review comments must only reference the selected target file.",
                original_path.display(),
                expected_path.display()
            ));
        }
        comment.path = expected_path.clone();
        normalized.insert(comment);
    }

    Ok(normalized)
}

fn normalize_agent_comment_path(workspace_path: &Path, comment_path: &Path) -> Result<PathBuf> {
    let relative_path = if comment_path.is_absolute() {
        comment_path.strip_prefix(workspace_path).map_err(|_| {
            format_err!(
                "Agent reported a finding outside the review workspace: {}",
                comment_path.display()
            )
        })?
    } else {
        comment_path
    };

    normalize_relative_review_path(relative_path).ok_or(format_err!(
        "Agent reported an invalid review path: {}",
        comment_path.display()
    ))
}

fn normalize_relative_review_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if normalized.as_os_str().is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn build_targets_from_comments(
    selected_targets: &[thirdpass_core::package::SelectedTarget],
    comments: std::collections::BTreeSet<review::comment::Comment>,
) -> Vec<review::ReviewTarget> {
    let mut grouped: std::collections::BTreeMap<
        std::path::PathBuf,
        std::collections::BTreeSet<review::comment::Comment>,
    > = std::collections::BTreeMap::new();

    for comment in comments {
        grouped
            .entry(comment.path.clone())
            .or_default()
            .insert(comment);
    }

    let mut targets = Vec::new();
    for target in selected_targets {
        let comments = grouped.remove(&target.relative_path).unwrap_or_default();
        targets.push(review::ReviewTarget {
            file_path: target.relative_path.clone(),
            file_hash: Some(target.file_hash.clone()),
            agent_summary: None,
            security_summary: Some(review::security_summary_for_comments(&comments)),
            confidence: None,
            comments,
        });
    }

    targets.extend(grouped.into_iter().map(|(file_path, comments)| {
        let security_summary = review::security_summary_for_comments(&comments);
        review::ReviewTarget {
            file_path,
            file_hash: None,
            agent_summary: None,
            security_summary: Some(security_summary),
            confidence: None,
            comments,
        }
    }));
    targets
}

fn select_target_files(
    workspace_path: &std::path::Path,
    review: &review::Review,
    config: &common::config::Config,
    local_only: bool,
) -> Result<Vec<thirdpass_core::package::SelectedTarget>> {
    let analysis = review::workspace::analyse(workspace_path)?;
    let registry = review
        .package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    let locally_reviewed_paths =
        get_locally_reviewed_target_paths(review, config, &registry.host_name)?;
    let mut target_policies = review::remote::review_target_policies(config)?;
    let target_policy = target_policies
        .remove(&registry.host_name)
        .unwrap_or_default();

    let candidates = thirdpass_core::package::candidate_files_with_policy(
        &analysis,
        &locally_reviewed_paths,
        &target_policy,
    );
    if candidates.is_empty() {
        return Err(format_err!("No files found to review."));
    }

    if thirdpass_core::package::all_candidates_reviewed(&candidates) {
        println!("All candidate files already reviewed locally; reusing reviewed candidates.");
    }

    let request_candidates = candidates
        .iter()
        .take(50)
        .map(|candidate| review::remote::ReviewCandidate {
            registry_host: registry.host_name.clone(),
            package_name: review.package.name.clone(),
            package_version: review.package.version.clone(),
            file_path: candidate.relative_path.display().to_string(),
            file_paths: Vec::new(),
            package_hash: review.package.package_hash.clone(),
        })
        .collect::<Vec<_>>();

    if !local_only {
        match review::remote::request_target(request_candidates, config) {
            Ok(Some(target)) => {
                let mut selected_targets = Vec::new();
                for target_file in target.target_file_paths() {
                    let target_relative = std::path::PathBuf::from(target_file);
                    let target_path = workspace_path.join(&target_relative);
                    if !target_path.is_file() {
                        log::warn!(
                            "Target file from API not found locally: {}",
                            target_path.display()
                        );
                        selected_targets.clear();
                        break;
                    }
                    selected_targets.push(thirdpass_core::package::selected_target(
                        target_path,
                        target_relative,
                    )?);
                }

                if !selected_targets.is_empty() {
                    let files = selected_targets
                        .iter()
                        .map(|target| target.relative_path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Selected target files: {}", files);
                    return Ok(selected_targets);
                }
            }
            Ok(None) => {}
            Err(err) => {
                if review::remote::is_authentication_required_error(&err) {
                    return Err(err);
                }
                log::warn!("Failed to request target from API: {}", err);
            }
        }
    }

    let target = thirdpass_core::package::select_first_candidate(workspace_path, &candidates)?;
    println!(
        "Selected target file (local order): {}",
        target.relative_path.display()
    );
    Ok(vec![target])
}

fn get_locally_reviewed_target_paths(
    current: &review::Review,
    config: &common::config::Config,
    registry_host_name: &str,
) -> Result<std::collections::BTreeSet<std::path::PathBuf>> {
    let mut reviewed_paths = std::collections::BTreeSet::new();
    for stored in review::fs::list_with_status()? {
        if !matches_current_review_package(
            &stored.review,
            current,
            &config.core.public_user_id,
            registry_host_name,
        ) {
            continue;
        }

        for target in stored.review.targets {
            reviewed_paths.insert(target.file_path);
        }
    }
    Ok(reviewed_paths)
}

fn matches_current_review_package(
    candidate: &review::Review,
    current: &review::Review,
    public_user_id: &str,
    registry_host_name: &str,
) -> bool {
    candidate.reviewer_details.public_user_id == public_user_id
        && candidate.package.name == current.package.name
        && candidate.package.version == current.package.version
        && candidate.package.package_hash == current.package.package_hash
        && candidate
            .package
            .registries
            .iter()
            .any(|registry| registry.host_name == registry_host_name)
}

fn build_reviewer_details(
    config: &common::config::Config,
    agent_name: &str,
    agent_model: &str,
    agent_reasoning_effort: &str,
    review_strategy: &str,
    review_scope: review::ReviewScope,
) -> Result<review::ReviewerDetails> {
    Ok(review::ReviewerDetails {
        public_user_id: config.core.public_user_id.clone(),
        agent_name: agent_name.to_string(),
        agent_model: agent_model.to_string(),
        agent_reasoning_effort: agent_reasoning_effort.to_string(),
        review_strategy: review_strategy.to_string(),
        review_scope,
        created_at: now_epoch_seconds()?,
        thirdpass_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn recorded_agent_reasoning_effort(
    agent_name: &str,
    agent_reasoning_effort: Option<&str>,
) -> String {
    if agent_name == "manual" {
        return "manual".to_string();
    }

    agent_reasoning_effort
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .unwrap_or("default")
        .to_string()
}

fn now_epoch_seconds() -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format_err!("Failed to read system time: {}", err))?;
    Ok(now.as_secs().to_string())
}

fn report_submission_failure(err: &anyhow::Error) {
    eprintln!("Review submission failed; review remains saved locally for retry: {err}");
    log::warn!("Failed to submit review; review remains saved locally for retry: {err}");
}

fn package_target_label(review: &review::Review) -> String {
    format!("{}@{}", review.package.name, review.package.version)
}

fn submission_api_base(config: &common::config::Config) -> Result<String> {
    let base = common::api::normalize_base(&config.core.api_base)?;
    Ok(base.as_str().trim_end_matches('/').to_string())
}

fn format_agent_token(
    agent: review::tool::AgentKind,
    agent_model: Option<&str>,
    agent_reasoning_effort: Option<&str>,
) -> String {
    let mut details = vec![agent.name().to_string()];
    if let Some(model) = agent_model {
        if !model.trim().is_empty() {
            details.push(model.to_string());
        }
    }
    if agent == review::tool::AgentKind::Codex {
        if let Some(effort) = agent_reasoning_effort {
            if !effort.trim().is_empty() {
                details.push(effort.to_string());
            }
        }
    }
    details.join("-")
}

struct LocalReviewMatchCriteria<'a> {
    package_name: &'a str,
    package_version: &'a str,
    package_hash: &'a str,
    target_paths: &'a std::collections::BTreeSet<std::path::PathBuf>,
    expected_scope: review::ReviewScope,
    expected_agent_name: &'a str,
    expected_agent_model: Option<&'a str>,
    expected_agent_reasoning_effort: &'a str,
    expected_review_strategy: &'a str,
}

fn find_matching_local_review(
    config: &common::config::Config,
    criteria: &LocalReviewMatchCriteria,
) -> Result<Option<review::fs::StoredReview>> {
    let stored_reviews = review::fs::list_with_status()?;
    let mut best_submitted: Option<review::fs::StoredReview> = None;
    let mut best_pending: Option<review::fs::StoredReview> = None;

    for stored in stored_reviews {
        let current = &stored.review;
        if current.package.name != criteria.package_name
            || current.package.version != criteria.package_version
            || current.package.package_hash != criteria.package_hash
        {
            continue;
        }
        if current.reviewer_details.public_user_id != config.core.public_user_id {
            continue;
        }
        if current.reviewer_details.agent_name != criteria.expected_agent_name {
            continue;
        }
        if current.reviewer_details.review_scope != criteria.expected_scope {
            continue;
        }
        if current.reviewer_details.review_strategy != criteria.expected_review_strategy {
            continue;
        }
        if let Some(model) = criteria.expected_agent_model {
            if current.reviewer_details.agent_model != model {
                continue;
            }
        }
        if current.reviewer_details.agent_reasoning_effort
            != criteria.expected_agent_reasoning_effort
        {
            continue;
        }

        let stored_target_paths = current
            .targets
            .iter()
            .map(|target| target.file_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if &stored_target_paths != criteria.target_paths {
            continue;
        }

        match stored.status {
            review::fs::ReviewStorageStatus::Submitted => {
                if is_newer_review(&stored, best_submitted.as_ref()) {
                    best_submitted = Some(stored);
                }
            }
            review::fs::ReviewStorageStatus::Pending => {
                if is_newer_review(&stored, best_pending.as_ref()) {
                    best_pending = Some(stored);
                }
            }
        }
    }

    Ok(best_submitted.or(best_pending))
}

fn is_newer_review(
    candidate: &review::fs::StoredReview,
    current_best: Option<&review::fs::StoredReview>,
) -> bool {
    let candidate_ts = parse_created_at(&candidate.review.reviewer_details.created_at);
    let best_ts = current_best
        .map(|review| parse_created_at(&review.review.reviewer_details.created_at))
        .unwrap_or(0);
    candidate_ts > best_ts || current_best.is_none()
}

fn parse_created_at(value: &str) -> u64 {
    value.parse::<u64>().unwrap_or(0)
}

/// Setup review for editing.
fn setup_review(
    package_name: &str,
    package_version: &Option<String>,
    extension_names: &std::collections::BTreeSet<String>,
    config: &common::config::Config,
) -> Result<(review::Review, thirdpass_core::package::Manifest)> {
    let extensions = extension::manage::get_enabled(extension_names, config)?;

    let package_version_was_given = package_version.is_some();

    let mut package_version: Option<String> = package_version.clone();
    let mut registry_metadata: Option<thirdpass_core::extension::RegistryPackageMetadata> = None;
    if package_version.is_none() {
        let (version, r) =
            thirdpass_core::registry::latest_package_metadata(package_name, &extensions)?;
        package_version = Some(version);
        registry_metadata = Some(r);
    }

    let package_version = package_version.ok_or(format_err!(
        "No package version given. Failed to find latest package version."
    ))?;

    if !package_version_was_given {
        println!("Found latest package version: {}", package_version);
    }

    let registry_metadata = match registry_metadata {
        Some(metadata) => metadata,
        None => thirdpass_core::registry::primary_package_metadata(
            package_name,
            &package_version,
            &extensions,
        )?,
    };

    let registry = registry::Registry {
        id: 0,
        host_name: registry_metadata.registry_host_name.clone(),
        human_url: url::Url::parse(&registry_metadata.human_url)?,
        artifact_url: url::Url::parse(&registry_metadata.artifact_url)?,
    };
    let workspace_manifest = review::workspace::ensure(
        package_name,
        &package_version,
        &registry.host_name,
        &registry.artifact_url,
    )?;
    let mut registries = std::collections::BTreeSet::new();
    registries.insert(registry);
    let package = package::Package {
        id: 0,
        name: package_name.to_string(),
        version: package_version,
        registries,
        package_hash: workspace_manifest.package_hash.clone(),
    };
    let peer = peer::public_user_peer(&config.core.public_user_id, &config.core.api_base)?;
    let review = review::Review {
        id: 0,
        peer,
        package,
        targets: Vec::new(),
        reviewer_details: review::ReviewerDetails::default(),
        agent_summary: String::new(),
        overall_security_summary: review::SecuritySummary::default(),
        overall_security_confidence: None,
    };
    Ok((review, workspace_manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_current_review_package_requires_same_public_user_and_package_hash() -> Result<()> {
        let current = stored_review("user-a", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let matching = stored_review("user-a", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let other_user = stored_review("user-b", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let other_hash = stored_review("user-a", "registry.example", "pkg", "1.0.0", "hash-b")?;

        assert!(matches_current_review_package(
            &matching,
            &current,
            "user-a",
            "registry.example"
        ));
        assert!(!matches_current_review_package(
            &other_user,
            &current,
            "user-a",
            "registry.example"
        ));
        assert!(!matches_current_review_package(
            &other_hash,
            &current,
            "user-a",
            "registry.example"
        ));
        Ok(())
    }

    #[test]
    fn format_agent_token_combines_codex_details() {
        assert_eq!(
            format_agent_token(
                review::tool::AgentKind::Codex,
                Some("gpt-5.4"),
                Some("high")
            ),
            "codex-gpt-5.4-high"
        );
    }

    #[test]
    fn local_package_review_status_is_complete_when_all_files_reviewed() {
        assert!(LocalPackageReviewStatus {
            package_name: "pkg".to_string(),
            package_version: "1.0.0".to_string(),
            reviewed_file_count: 2,
            reviewable_file_count: 2,
        }
        .is_complete());
        assert!(!LocalPackageReviewStatus {
            package_name: "pkg".to_string(),
            package_version: "1.0.0".to_string(),
            reviewed_file_count: 1,
            reviewable_file_count: 2,
        }
        .is_complete());
    }

    #[test]
    fn select_local_candidate_batch_skips_reviewed_files_and_caps_lines() {
        let candidates = vec![
            candidate_file("src/large.rs", 800, false),
            candidate_file("src/medium.rs", 250, false),
            candidate_file("src/over-limit.rs", 200, false),
            candidate_file("src/reviewed.rs", 1, true),
        ];

        let batch = select_local_candidate_batch(&candidates);

        assert_eq!(
            batch,
            vec!["src/large.rs".to_string(), "src/medium.rs".to_string()]
        );
    }

    #[test]
    fn select_local_candidate_batch_caps_file_count() {
        let candidates = (1..=6)
            .map(|index| candidate_file(&format!("src/file-{index}.rs"), 1, false))
            .collect::<Vec<_>>();

        let batch = select_local_candidate_batch(&candidates);

        assert_eq!(batch.len(), 5);
        assert_eq!(batch[0], "src/file-1.rs");
        assert_eq!(batch[4], "src/file-5.rs");
    }

    #[test]
    fn select_local_candidate_batch_allows_oversized_first_file() {
        let candidates = vec![
            candidate_file("src/huge.rs", 2_000, false),
            candidate_file("src/small.rs", 10, false),
        ];

        let batch = select_local_candidate_batch(&candidates);

        assert_eq!(batch, vec!["src/huge.rs".to_string()]);
    }

    #[test]
    fn deps_review_rejects_file_targets() {
        let mut args = review_args("axum", Some("0.8.9"));
        args.deps = true;
        args.target_files = vec!["src/lib.rs".to_string()];

        let error = run_command(&args, &[]).expect_err("expected --deps with --file to fail");

        assert_eq!(error.to_string(), "--deps cannot be combined with --file.");
    }

    #[test]
    fn deps_review_rejects_submit_existing() {
        let mut args = review_args("axum", Some("0.8.9"));
        args.deps = true;
        args.submit_existing = true;

        let error =
            run_command(&args, &[]).expect_err("expected --deps with --submit-existing to fail");

        assert_eq!(
            error.to_string(),
            "--deps cannot be combined with --submit-existing."
        );
    }

    #[test]
    fn plan_only_requires_debug_cli_env() {
        let _lock = crate::common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let _env = crate::test_support::ScopedEnv::remove_var(crate::command::DEBUG_CLI_ENV_VAR);
        let mut args = review_args("axum", Some("0.8.9"));
        args.deps = true;
        args.plan_only = true;

        let error = run_command(&args, &[]).expect_err("expected --plan-only to require debug CLI");

        assert_eq!(
            error.to_string(),
            "--plan-only requires THIRDPASS_DEBUG_CLI=1."
        );
    }

    #[test]
    fn plan_only_requires_deps() {
        let _lock = crate::common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let _env = crate::test_support::ScopedEnv::set_var(crate::command::DEBUG_CLI_ENV_VAR, "1");
        let mut args = review_args("axum", Some("0.8.9"));
        args.plan_only = true;

        let error = run_command(&args, &[]).expect_err("expected --plan-only to require --deps");

        assert_eq!(error.to_string(), "--plan-only requires --deps.");
    }

    #[test]
    fn validate_agent_comments_accepts_target_relative_path() -> Result<()> {
        let comments = validate_agent_comments_for_target(
            [agent_comment("src/index.js")],
            Path::new("/workspace"),
            Path::new("src/index.js"),
        )?;

        let comment = comments.iter().next().expect("expected comment");
        assert_eq!(comment.path, PathBuf::from("src/index.js"));
        Ok(())
    }

    #[test]
    fn validate_agent_comments_accepts_workspace_absolute_target_path() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let comments = validate_agent_comments_for_target(
            [agent_comment(workspace.path().join("src/index.js"))],
            workspace.path(),
            Path::new("src/index.js"),
        )?;

        let comment = comments.iter().next().expect("expected comment");
        assert_eq!(comment.path, PathBuf::from("src/index.js"));
        Ok(())
    }

    #[test]
    fn validate_agent_comments_rejects_different_file_path() {
        let err = validate_agent_comments_for_target(
            [agent_comment("src/context.js")],
            Path::new("/workspace"),
            Path::new("src/index.js"),
        )
        .expect_err("expected mismatched file path to fail");

        assert!(err.to_string().contains("selected target is src/index.js"));
    }

    #[test]
    fn validate_agent_comments_rejects_path_traversal() {
        let err = validate_agent_comments_for_target(
            [agent_comment("../outside.js")],
            Path::new("/workspace"),
            Path::new("src/index.js"),
        )
        .expect_err("expected traversal path to fail");

        assert!(err.to_string().contains("invalid review path"));
    }

    #[test]
    fn validate_agent_comments_rejects_absolute_path_outside_workspace() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let err = validate_agent_comments_for_target(
            [agent_comment(outside.path().join("src/index.js"))],
            workspace.path(),
            Path::new("src/index.js"),
        )
        .expect_err("expected outside-workspace path to fail");

        assert!(err.to_string().contains("outside the review workspace"));
        Ok(())
    }

    fn agent_comment(path: impl Into<PathBuf>) -> review::comment::Comment {
        review::comment::Comment {
            id: 0,
            security: review::Priority::Low,
            complexity: review::Priority::Low,
            path: path.into(),
            message: "test comment".to_string(),
            selection: None,
        }
    }

    fn stored_review(
        public_user_id: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
        package_hash: &str,
    ) -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: registry_host_name.to_string(),
            human_url: url::Url::parse("https://registry.example/pkg")?,
            artifact_url: url::Url::parse("https://registry.example/pkg.tgz")?,
        });

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: package_name.to_string(),
                version: package_version.to_string(),
                registries,
                package_hash: package_hash.to_string(),
            },
            targets: Vec::new(),
            reviewer_details: review::ReviewerDetails {
                public_user_id: public_user_id.to_string(),
                ..Default::default()
            },
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }

    fn review_args(package_name: &str, package_version: Option<&str>) -> Arguments {
        Arguments {
            package_name: package_name.to_string(),
            package_version: package_version.map(ToOwned::to_owned),
            extension_names: None,
            target_files: Vec::new(),
            deps: false,
            plan_only: false,
            manual: false,
            agent: None,
            agent_model: None,
            agent_reasoning_effort: None,
            submit_existing: false,
            local_only: false,
        }
    }

    fn candidate_file(
        path: &str,
        line_count: usize,
        already_reviewed: bool,
    ) -> thirdpass_core::package::CandidateFile {
        thirdpass_core::package::CandidateFile {
            relative_path: std::path::PathBuf::from(path),
            line_count,
            already_reviewed,
        }
    }
}
