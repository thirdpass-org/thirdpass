use anyhow::{format_err, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;

#[derive(Debug, StructOpt, Clone)]
#[structopt(
    name = "no_version",
    no_version,
    global_settings = &[structopt::clap::AppSettings::DisableVersion],
    about = "Review a package release and submit findings.",
    after_help = "Examples:\n    vouch review d3 4.10.0\n    vouch review d3 --extension js\n    vouch review d3 4.10.0 --file src/index.js --file src/color.js\n    vouch review d3 4.10.0 --agent codex --agent-model gpt-5.5 --agent-reasoning-effort high\n    vouch review d3 4.10.0 --submit-existing\n    vouch review d3 4.10.0 --skip-coordination"
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

    /// Skip central API coordination (no target assignment or submission).
    #[structopt(long = "skip-coordination", alias = "no-submit")]
    pub skip_coordination: bool,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    // TODO: Add gpg signing.

    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    if args.submit_existing && args.skip_coordination {
        return Err(format_err!(
            "--submit-existing cannot be combined with --skip-coordination."
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
        resolve_target_paths(&workspace_manifest.workspace_path, &args.target_files)?
    } else {
        select_target_files(
            &workspace_manifest.workspace_path,
            &review,
            &config,
            args.skip_coordination,
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

        let existing = match find_matching_local_review(
            &config,
            &review.package.name,
            &review.package.version,
            &review.package.package_hash,
            &target_paths,
            expected_scope,
            &expected_agent_name,
            expected_agent_model.as_deref(),
            &expected_agent_reasoning_effort,
            &expected_review_strategy,
        ) {
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
            review::workspace::remove(&workspace_manifest)?;
            return Ok(());
        }

        let package_label = package_target_label(&existing.review);
        let api_base = submission_api_base(&config)?;
        println!(
            "Submitting existing review for {} to {}.",
            package_label, api_base
        );
        let submit_result = review::remote::submit(&existing.review, &config);
        review::workspace::remove(&workspace_manifest)?;

        if let Err(err) = submit_result {
            if is_network_error(&err) {
                log::warn!(
                    "Failed to submit review due to network error: {}. Use --skip-coordination to skip.",
                    err
                );
                return Ok(());
            }
            return Err(err);
        }

        review::promote_pending(&existing.review, &existing.path)?;
        println!("Review submitted.");
        return Ok(());
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
        overall_security_confidence,
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
            None,
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
        let mut confidence_values = Vec::new();

        for target in &selected_targets {
            let target_display = target.relative_path.display().to_string();
            let file_contents = std::fs::read_to_string(&target.absolute_path)?;
            let spinner = ProgressBar::new_spinner();
            let spinner_style = ProgressStyle::with_template("{spinner} {msg}")
                .unwrap()
                .tick_strings(&["|", "/", "-", "\\"]);
            spinner.set_style(spinner_style);
            spinner.enable_steady_tick(std::time::Duration::from_millis(120));
            spinner.set_message(format!("Reviewing {}", style(&target_display).dim()));
            let agent_run = review::tool::run_agent(
                agent,
                &workspace_manifest.workspace_path,
                &target_display,
                &file_contents,
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
            if let Some(summary) = agent_run.summary.as_deref() {
                let summary = summary.trim();
                if !summary.is_empty() {
                    if !agent_summary.is_empty() {
                        agent_summary.push('\n');
                    }
                    agent_summary.push_str(summary);
                }
            }
            if let Some(confidence) = agent_run.confidence {
                confidence_values.push(confidence);
            }
            let comments = normalize_comments(agent_run.comments)
                .into_iter()
                .map(|mut comment| {
                    comment.path = target.relative_path.clone();
                    comment
                })
                .collect::<std::collections::BTreeSet<_>>();
            targets.push(review::ReviewTarget {
                file_path: target.relative_path.clone(),
                file_hash: Some(target.file_hash.clone()),
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
            aggregate_confidence(&confidence_values),
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
    review.overall_security_confidence = overall_security_confidence;

    let pending_review_path = review::store_pending(&review)?;
    println!("Review saved.");

    let submit_result = if args.skip_coordination {
        Ok(false)
    } else {
        let package_label = package_target_label(&review);
        let api_base = submission_api_base(&config)?;
        println!("Submitting review for {} to {}.", package_label, api_base);
        review::remote::submit(&review, &config).map(|_| true)
    };

    review::workspace::remove(&workspace_manifest)?;

    let submitted_review = match submit_result {
        Ok(submitted_review) => submitted_review,
        Err(err) => {
            if is_network_error(&err) {
                log::warn!(
                    "Failed to submit review due to network error: {}. Use --skip-coordination to skip.",
                    err
                );
                return Ok(());
            }
            return Err(err);
        }
    };

    if !args.skip_coordination {
        review::promote_pending(&review, &pending_review_path)?;
        if submitted_review {
            println!("Review submitted.");
        }
    }

    Ok(())
}

/// Parse user comments from active review file and insert into index.
fn get_comments(
    active_review_file: &std::path::PathBuf,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let comments = review::active::parse(&active_review_file)?;
    Ok(normalize_comments(comments))
}

fn run_manual_review(
    review: &review::Review,
    workspace_path: &std::path::PathBuf,
    config: &common::config::Config,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let reviews_directory = review::tool::ensure_reviews_directory(&workspace_path)?;
    let active_review_file = review::active::ensure(&review, &reviews_directory)?;

    println!("Starting review tool.");
    review::tool::run_manual(&workspace_path, &config)?;
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
        comment.apply_legacy_summary();
        comment.id = 0;
        normalized.insert(comment);
    }
    normalized
}

fn build_targets_from_comments(
    selected_targets: &[SelectedTarget],
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
            comments,
        });
    }

    targets.extend(
        grouped
            .into_iter()
            .map(|(file_path, comments)| review::ReviewTarget {
                file_path,
                file_hash: None,
                comments,
            }),
    );
    targets
}

#[derive(Debug, Clone)]
struct SelectedTarget {
    absolute_path: std::path::PathBuf,
    relative_path: std::path::PathBuf,
    file_hash: vouch_lib::schema::FileHash,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ReviewCandidateFile {
    relative_path: std::path::PathBuf,
    line_count: usize,
    already_reviewed: bool,
}

fn resolve_target_path(
    workspace_path: &std::path::PathBuf,
    target_file: &str,
) -> Result<SelectedTarget> {
    let target_path = std::path::PathBuf::from(target_file);
    let target_path = if target_path.is_absolute() {
        target_path
    } else {
        workspace_path.join(target_path)
    };
    if !target_path.is_file() {
        return Err(format_err!(
            "Target file not found: {}",
            target_path.display()
        ));
    }
    let target_relative = target_path
        .strip_prefix(workspace_path)
        .unwrap_or(target_path.as_path())
        .to_path_buf();
    selected_target(target_path, target_relative)
}

fn selected_target(
    absolute_path: std::path::PathBuf,
    relative_path: std::path::PathBuf,
) -> Result<SelectedTarget> {
    let (hash, path_type) = common::fs::hash(&absolute_path)?;
    if !matches!(path_type, common::fs::PathType::File) {
        return Err(format_err!(
            "Target path is not a file: {}",
            absolute_path.display()
        ));
    }
    Ok(SelectedTarget {
        absolute_path,
        relative_path,
        file_hash: vouch_lib::schema::FileHash::blake3(hash),
    })
}

fn resolve_target_paths(
    workspace_path: &std::path::PathBuf,
    target_files: &[String],
) -> Result<Vec<SelectedTarget>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut targets = Vec::new();
    for target_file in target_files {
        let target = resolve_target_path(workspace_path, target_file)?;
        if seen.insert(target.relative_path.clone()) {
            targets.push(target);
        }
    }
    Ok(targets)
}

fn select_target_files(
    workspace_path: &std::path::PathBuf,
    review: &review::Review,
    config: &common::config::Config,
    skip_coordination: bool,
) -> Result<Vec<SelectedTarget>> {
    let analysis = review::workspace::analyse(workspace_path)?;
    let registry = review
        .package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    let locally_reviewed_paths =
        get_locally_reviewed_target_paths(review, config, &registry.host_name)?;

    let mut candidates = Vec::new();
    for (path, entry) in analysis.iter() {
        if let common::fs::PathType::File = entry.path_type {
            candidates.push(ReviewCandidateFile {
                relative_path: path.clone(),
                line_count: entry.line_count,
                already_reviewed: locally_reviewed_paths.contains(path),
            });
        }
    }
    sort_review_candidates(&mut candidates);
    if candidates.is_empty() {
        return Err(format_err!("No files found to review."));
    }

    if candidates
        .iter()
        .all(|candidate| candidate.already_reviewed)
    {
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
            package_hash: review.package.package_hash.clone(),
        })
        .collect::<Vec<_>>();

    if !skip_coordination {
        if let Ok(Some(target)) = review::remote::request_target(request_candidates, config) {
            let target_relative = std::path::PathBuf::from(target.file_path);
            let target_path = workspace_path.join(&target_relative);
            if target_path.is_file() {
                println!("Selected target file: {}", target_relative.display());
                return Ok(vec![selected_target(target_path, target_relative)?]);
            }
            log::warn!(
                "Target file from API not found locally: {}",
                target_path.display()
            );
        }
    }

    let target_relative = candidates[0].relative_path.clone();
    let target_path = workspace_path.join(&target_relative);
    println!(
        "Selected target file (local order): {}",
        target_relative.display()
    );
    Ok(vec![selected_target(target_path, target_relative)?])
}

fn sort_review_candidates(candidates: &mut Vec<ReviewCandidateFile>) {
    candidates.sort_by(|a, b| {
        a.already_reviewed
            .cmp(&b.already_reviewed)
            .then_with(|| b.line_count.cmp(&a.line_count))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
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
            &config.core.reviewer_uuid,
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
    reviewer_uuid: &str,
    registry_host_name: &str,
) -> bool {
    candidate.reviewer_details.reviewer_uuid == reviewer_uuid
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
        reviewer_uuid: config.core.reviewer_uuid.clone(),
        agent_name: agent_name.to_string(),
        agent_model: agent_model.to_string(),
        agent_reasoning_effort: agent_reasoning_effort.to_string(),
        review_strategy: review_strategy.to_string(),
        review_scope,
        created_at: now_epoch_seconds()?,
        vouch_version: env!("CARGO_PKG_VERSION").to_string(),
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

fn is_network_error(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        return reqwest_err.is_connect() || reqwest_err.is_timeout();
    }
    false
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

fn aggregate_confidence(
    confidence_values: &[review::ReviewConfidence],
) -> Option<review::ReviewConfidence> {
    if confidence_values.is_empty() {
        return None;
    }
    if confidence_values
        .iter()
        .any(|value| matches!(value, review::ReviewConfidence::Low))
    {
        return Some(review::ReviewConfidence::Low);
    }
    if confidence_values
        .iter()
        .any(|value| matches!(value, review::ReviewConfidence::Medium))
    {
        return Some(review::ReviewConfidence::Medium);
    }
    Some(review::ReviewConfidence::High)
}

fn find_matching_local_review(
    config: &common::config::Config,
    package_name: &str,
    package_version: &str,
    package_hash: &str,
    target_paths: &std::collections::BTreeSet<std::path::PathBuf>,
    expected_scope: review::ReviewScope,
    expected_agent_name: &str,
    expected_agent_model: Option<&str>,
    expected_agent_reasoning_effort: &str,
    expected_review_strategy: &str,
) -> Result<Option<review::fs::StoredReview>> {
    let stored_reviews = review::fs::list_with_status()?;
    let mut best_submitted: Option<review::fs::StoredReview> = None;
    let mut best_pending: Option<review::fs::StoredReview> = None;

    for stored in stored_reviews {
        let current = &stored.review;
        if current.package.name != package_name
            || current.package.version != package_version
            || current.package.package_hash != package_hash
        {
            continue;
        }
        if current.reviewer_details.reviewer_uuid != config.core.reviewer_uuid {
            continue;
        }
        if current.reviewer_details.agent_name != expected_agent_name {
            continue;
        }
        if current.reviewer_details.review_scope != expected_scope {
            continue;
        }
        if current.reviewer_details.review_strategy != expected_review_strategy {
            continue;
        }
        if let Some(model) = expected_agent_model {
            if current.reviewer_details.agent_model != model {
                continue;
            }
        }
        if current.reviewer_details.agent_reasoning_effort != expected_agent_reasoning_effort {
            continue;
        }

        let stored_target_paths = current
            .targets
            .iter()
            .map(|target| target.file_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if &stored_target_paths != target_paths {
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
) -> Result<(review::Review, review::workspace::Manifest)> {
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;

    let package_version_was_given = package_version.is_some();

    let mut package_version: Option<String> = package_version.clone();
    let mut registry_metadata: Option<vouch_lib::extension::RegistryPackageMetadata> = None;
    if package_version.is_none() {
        let (version, r) = get_latest_package_version(package_name, &extensions)?;
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
        None => get_primary_registry_metadata(package_name, &package_version, &extensions)?,
    };

    let registry = registry::Registry {
        id: 0,
        host_name: registry_metadata.registry_host_name.clone(),
        human_url: url::Url::parse(&registry_metadata.human_url)?,
        artifact_url: url::Url::parse(&registry_metadata.artifact_url)?,
    };
    let workspace_manifest = review::workspace::ensure(
        &package_name,
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
    let peer = peer::reviewer_peer(&config.core.reviewer_uuid, &config.core.api_base)?;
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

fn get_latest_package_version(
    package_name: &str,
    extensions: &Vec<Box<dyn vouch_lib::extension::Extension>>,
) -> Result<(String, vouch_lib::extension::RegistryPackageMetadata)> {
    let remote_package_metadata = extension::search_registries(&package_name, &None, &extensions)?;
    let primary_registry = remote_package_metadata
        .iter()
        .find(|registry_metadata| registry_metadata.is_primary)
        .ok_or(format_err!(
            "Failed to find primary registry metadata from extension."
        ))?;
    let package_version = primary_registry.package_version.clone();
    Ok((package_version, primary_registry.clone()))
}

fn get_primary_registry_metadata(
    package_name: &str,
    package_version: &str,
    extensions: &Vec<Box<dyn vouch_lib::extension::Extension>>,
) -> Result<vouch_lib::extension::RegistryPackageMetadata> {
    let remote_package_metadata =
        extension::search_registries(&package_name, &Some(package_version), &extensions)?;
    let primary_registry = remote_package_metadata
        .iter()
        .find(|registry_metadata| registry_metadata.is_primary)
        .ok_or(format_err!(
            "Failed to find primary registry metadata from extension."
        ))?;
    Ok(primary_registry.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_review_candidates_prefers_unreviewed_files() {
        let mut candidates = vec![
            candidate_file("already-reviewed-large.js", 300, true),
            candidate_file("unreviewed-small.js", 50, false),
            candidate_file("unreviewed-large.js", 200, false),
            candidate_file("already-reviewed-small.js", 20, true),
        ];

        sort_review_candidates(&mut candidates);

        let paths = candidates
            .iter()
            .map(|candidate| candidate.relative_path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "unreviewed-large.js",
                "unreviewed-small.js",
                "already-reviewed-large.js",
                "already-reviewed-small.js",
            ]
        );
    }

    #[test]
    fn matches_current_review_package_requires_same_reviewer_and_package_hash() -> Result<()> {
        let current = stored_review("reviewer-a", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let matching = stored_review("reviewer-a", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let other_reviewer =
            stored_review("reviewer-b", "registry.example", "pkg", "1.0.0", "hash-a")?;
        let other_hash = stored_review("reviewer-a", "registry.example", "pkg", "1.0.0", "hash-b")?;

        assert!(matches_current_review_package(
            &matching,
            &current,
            "reviewer-a",
            "registry.example"
        ));
        assert!(!matches_current_review_package(
            &other_reviewer,
            &current,
            "reviewer-a",
            "registry.example"
        ));
        assert!(!matches_current_review_package(
            &other_hash,
            &current,
            "reviewer-a",
            "registry.example"
        ));
        Ok(())
    }

    #[test]
    fn resolve_target_path_records_blake3_file_hash() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let workspace = tmp.path().to_path_buf();
        let contents = b"console.log('review me');\n";
        std::fs::write(workspace.join("index.js"), contents)?;

        let target = resolve_target_path(&workspace, "index.js")?;
        let expected_hash = blake3::hash(contents).to_hex().as_str().to_string();

        assert_eq!(target.relative_path, std::path::PathBuf::from("index.js"));
        assert_eq!(
            target.file_hash,
            vouch_lib::schema::FileHash::blake3(expected_hash)
        );
        Ok(())
    }

    #[test]
    fn format_agent_token_combines_codex_details() {
        assert_eq!(
            format_agent_token(
                review::tool::AgentKind::Codex,
                Some("gpt-5.5"),
                Some("high")
            ),
            "codex-gpt-5.5-high"
        );
    }

    fn candidate_file(
        path: &str,
        line_count: usize,
        already_reviewed: bool,
    ) -> ReviewCandidateFile {
        ReviewCandidateFile {
            relative_path: std::path::PathBuf::from(path),
            line_count,
            already_reviewed,
        }
    }

    fn stored_review(
        reviewer_uuid: &str,
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
                reviewer_uuid: reviewer_uuid.to_string(),
                ..Default::default()
            },
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
