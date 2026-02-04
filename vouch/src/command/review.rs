use anyhow::{format_err, Result};
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
    global_settings = &[structopt::clap::AppSettings::DisableVersion]
)]
pub struct Arguments {
    /// Package name.
    #[structopt(name = "package-name")]
    pub package_name: String,

    /// Package version.
    #[structopt(name = "package-version")]
    pub package_version: Option<String>,

    /// Specify an extension for handling the package.
    /// Example values: py, js, rs
    #[structopt(long = "extension", short = "e", name = "name")]
    pub extension_names: Option<Vec<String>>,

    /// Target file path within the package.
    #[structopt(long = "file", name = "path")]
    pub target_file: Option<String>,

    /// Use manual review via VSCode.
    #[structopt(long = "manual")]
    pub manual: bool,

    /// Skip submission to the central API.
    #[structopt(long = "no-submit")]
    pub no_submit: bool,
}

pub fn run_command(args: &Arguments) -> Result<()> {
    // TODO: Add gpg signing.

    let mut config = common::config::Config::load()?;
    extension::manage::update_config(&mut config)?;
    if args.manual {
        review::tool::check_manual_install(&mut config)?;
    }
    let config = config;

    let extension_names =
        extension::manage::handle_extension_names_arg(&args.extension_names, &config)?;

    let (mut review, workspace_manifest) = setup_review(
        &args.package_name,
        &args.package_version,
        &extension_names,
        &config,
    )?;

    let (target_path, target_relative) = match args.target_file.as_ref() {
        Some(target_file) => resolve_target_path(&workspace_manifest.workspace_path, target_file)?,
        None => select_target_file(
            &workspace_manifest.workspace_path,
            &review,
            &config,
        )?,
    };
    let target_display = target_relative.display().to_string();

    let (comments, agent_name, agent_model, prompt_version) = if args.manual {
        let comments = run_manual_review(&review, &workspace_manifest.workspace_path, &config)?;
        (
            comments,
            "manual".to_string(),
            "".to_string(),
            "manual".to_string(),
        )
    } else {
        let agent = review::tool::select_agent()?;
        let file_contents = std::fs::read_to_string(&target_path)?;
        let agent_run = review::tool::run_agent(
            agent,
            &workspace_manifest.workspace_path,
            &target_display,
            &file_contents,
        )?;
        let comments = normalize_comments(agent_run.comments);
        (
            comments,
            agent.name().to_string(),
            agent_run.model,
            review::tool::agent_prompt_version().to_string(),
        )
    };

    review.comments = comments;
    review.target_file = Some(target_relative.clone());
    review.metadata = build_metadata(
        &config,
        &agent_name,
        &agent_model,
        &prompt_version,
    )?;
    review.overall_security_summary = review::overall_security_summary(&review)?;

    review::store(&review)?;
    println!("Review saved.");

    let submit_result = if args.no_submit {
        Ok(())
    } else {
        println!("Submitting review to central API.");
        review::remote::submit(&review, &config)
    };

    review::workspace::remove(&workspace_manifest)?;
    submit_result
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

fn resolve_target_path(
    workspace_path: &std::path::PathBuf,
    target_file: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
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
    Ok((target_path, target_relative))
}

fn select_target_file(
    workspace_path: &std::path::PathBuf,
    review: &review::Review,
    config: &common::config::Config,
) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let analysis = review::workspace::analyse(workspace_path)?;
    let mut candidates = Vec::new();
    for (path, entry) in analysis.iter() {
        if let common::fs::PathType::File = entry.path_type {
            candidates.push((path.clone(), entry.line_count));
        }
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    if candidates.is_empty() {
        return Err(format_err!("No files found to review."));
    }

    let registry = review
        .package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;

    let request_candidates = candidates
        .iter()
        .take(50)
        .map(|(path, _)| review::remote::ReviewTarget {
            registry_host: registry.host_name.clone(),
            package_name: review.package.name.clone(),
            package_version: review.package.version.clone(),
            file_path: path.display().to_string(),
            artifact_hash: review.package.artifact_hash.clone(),
        })
        .collect::<Vec<_>>();

    if let Ok(Some(target)) = review::remote::request_target(request_candidates, config) {
        let target_relative = std::path::PathBuf::from(target.file_path);
        let target_path = workspace_path.join(&target_relative);
        if target_path.is_file() {
            println!("Selected target file: {}", target_relative.display());
            return Ok((target_path, target_relative));
        }
        log::warn!(
            "Target file from API not found locally: {}",
            target_path.display()
        );
    }

    let target_relative = candidates[0].0.clone();
    let target_path = workspace_path.join(&target_relative);
    println!(
        "Selected target file (local order): {}",
        target_relative.display()
    );
    Ok((target_path, target_relative))
}

fn build_metadata(
    config: &common::config::Config,
    agent_name: &str,
    agent_model: &str,
    prompt_version: &str,
) -> Result<review::ReviewMetadata> {
    Ok(review::ReviewMetadata {
        reviewer_uuid: config.core.reviewer_uuid.clone(),
        agent_name: agent_name.to_string(),
        agent_model: agent_model.to_string(),
        prompt_version: prompt_version.to_string(),
        created_at: now_epoch_seconds()?,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn now_epoch_seconds() -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format_err!("Failed to read system time: {}", err))?;
    Ok(now.as_secs().to_string())
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
        artifact_hash: workspace_manifest.artifact_hash.clone(),
    };
    let peer = peer::reviewer_peer(&config.core.reviewer_uuid, &config.core.api_base)?;
    let review = review::Review {
        id: 0,
        peer,
        package,
        comments: std::collections::BTreeSet::new(),
        metadata: review::ReviewMetadata::default(),
        target_file: None,
        overall_security_summary: review::SecuritySummary::default(),
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
