use std::collections::BTreeSet;

use anyhow::{format_err, Result};
use common::StoreTransaction;
use structopt::{self, StructOpt};

use crate::common;
use crate::extension;
use crate::package;
use crate::peer;
use crate::registry;
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

    let mut store = store::Store::from_root()?;
    let tx = store.get_transaction()?;

    let (mut review, edit_mode, workspace_manifest) = setup_review(
        &args.package_name,
        &args.package_version,
        &extension_names,
        &config,
        &tx,
    )?;

    let target_file = args
        .target_file
        .as_ref()
        .ok_or(format_err!("--file is required"))?;
    let target_path = resolve_target_path(&workspace_manifest.workspace_path, target_file)?;
    let target_relative = target_path
        .strip_prefix(&workspace_manifest.workspace_path)
        .unwrap_or(target_path.as_path())
        .to_path_buf();
    let target_display = target_relative.display().to_string();

    // TODO: Make use of workspace analysis in review.
    review::workspace::analyse(&workspace_manifest.workspace_path)?;

    let (comments, agent_name, agent_model, prompt_version) = if args.manual {
        let comments = run_manual_review(&review, &workspace_manifest.workspace_path, &config, &tx)?;
        (
            comments,
            "manual".to_string(),
            "".to_string(),
            "manual".to_string(),
        )
    } else {
        let agent = review::tool::select_agent()?;
        let file_contents = std::fs::read_to_string(&target_path)?;
        let agent_run =
            review::tool::run_agent(agent, &target_path, &target_display, &file_contents)?;
        let comments = insert_comments(agent_run.comments, &tx)?;
        (
            comments,
            agent.name().to_string(),
            agent_run.model,
            review::tool::agent_prompt_version().to_string(),
        )
    };

    review.comments = comments;
    review.target_file = Some(target_relative);
    review.metadata = build_metadata(
        &config,
        &agent_name,
        &agent_model,
        &prompt_version,
    )?;
    review.overall_security_summary = review::overall_security_summary(&review)?;

    review::store(&review, &tx)?;
    let commit_message = get_commit_message(&review.package, &review.target_file, &edit_mode)?;
    tx.commit(&commit_message)?;
    println!("Review committed.");

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
    tx: &StoreTransaction,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let comments = review::active::parse(&active_review_file)?;
    insert_comments(comments, tx)
}

fn insert_comments<I>(
    comments: I,
    tx: &StoreTransaction,
) -> Result<std::collections::BTreeSet<review::comment::Comment>>
where
    I: IntoIterator<Item = review::comment::Comment>,
{
    let mut inserted_comments = std::collections::BTreeSet::<_>::new();
    for comment in comments {
        let mut comment = comment;
        comment.apply_legacy_summary();
        let comment = review::comment::index::insert(&comment, &tx)?;
        inserted_comments.insert(comment);
    }
    Ok(inserted_comments)
}

fn run_manual_review(
    review: &review::Review,
    workspace_path: &std::path::PathBuf,
    config: &common::config::Config,
    tx: &StoreTransaction,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let reviews_directory = review::tool::ensure_reviews_directory(&workspace_path)?;
    let active_review_file = review::active::ensure(&review, &reviews_directory)?;

    println!("Starting review tool.");
    review::tool::run_manual(&workspace_path, &config)?;
    if !active_review_file.exists() {
        println!("Review file not found.");
        return Ok(std::collections::BTreeSet::new());
    }
    let comments = get_comments(&active_review_file, &tx)?;
    println!(
        "Review tool closed. Found {} review comments.",
        comments.len()
    );
    Ok(comments)
}

fn resolve_target_path(
    workspace_path: &std::path::PathBuf,
    target_file: &str,
) -> Result<std::path::PathBuf> {
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
    Ok(target_path)
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

/// Review edit mode.
enum ReviewEditMode {
    Create,
    Update,
}

/// Setup review for editing.
fn setup_review(
    package_name: &str,
    package_version: &Option<String>,
    extension_names: &std::collections::BTreeSet<String>,
    config: &common::config::Config,
    tx: &StoreTransaction,
) -> Result<(review::Review, ReviewEditMode, review::workspace::Manifest)> {
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;

    let package_version_was_given = package_version.is_some();

    // Get latest package version if none given.
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

    if let Some((review, workspace_manifest)) = setup_existing_review(
        &package_name,
        &package_version,
        &extension_names,
        &config,
        &tx,
    )? {
        println!("Selecting previously committed review for editing.");
        Ok((review, ReviewEditMode::Update, workspace_manifest))
    } else {
        println!("Editing local uncommitted review.");
        let (review, workspace_directory) = setup_new_review(
            &package_name,
            &package_version,
            &registry_metadata,
            &extension_names,
            &config,
            &tx,
        )?;
        Ok((review, ReviewEditMode::Create, workspace_directory))
    }
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

// Setup existing review for editing.
fn setup_existing_review(
    package_name: &str,
    package_version: &str,
    extension_names: &BTreeSet<String>,
    config: &common::config::Config,
    tx: &StoreTransaction,
) -> Result<Option<(review::Review, review::workspace::Manifest)>> {
    log::debug!("Checking index for existing reviewer review.");
    let reviewer_peer = get_reviewer_peer(config, &tx)?;
    let reviews = review::index::get(
        &review::index::Fields {
            package_name: Some(&package_name),
            package_version: Some(&package_version),
            peer: Some(&reviewer_peer),
            ..Default::default()
        },
        &tx,
    )?;

    // TODO: Include filter in above get call.

    log::debug!("Count existing matching reviews: {}", reviews.len());
    let reviews = filter_on_ecosystems(&reviews, &extension_names, &config)?;
    log::debug!(
        "Count existing matching reviews post filtering: {}",
        reviews.len()
    );

    // TODO: count number of different ecosystems in found reviews.

    if reviews.len() > 1 {
        multiple_matching_ecosystems(&reviews, &config)?;
        return Ok(None);
    }

    let review = match reviews.first() {
        Some(review) => review,
        None => return Ok(None),
    };

    log::debug!("Setting up review workspace using existing review package metadata.");
    let registry = get_primary_registry(&review.package)?;
    let workspace_manifest = review::workspace::ensure(
        &review.package.name,
        &review.package.version,
        &registry.host_name,
        &registry.artifact_url,
    )?;
    Ok(Some((review.clone(), workspace_manifest)))
}

// TODO: Replace with method on Package.
fn get_primary_registry<'a>(package: &'a package::Package) -> Result<&'a registry::Registry> {
    let registry = package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    Ok(registry)
}

/// Filter reviews on given extension.
fn filter_on_ecosystems(
    reviews: &Vec<review::Review>,
    target_extension_names: &BTreeSet<String>,
    config: &common::config::Config,
) -> Result<Vec<review::Review>> {
    // Find registry host names which are handled by the given extensions.
    let enabled_registries: std::collections::BTreeSet<String> = config
        .extensions
        .registries
        .iter()
        .filter(|(_registry_host_name, extension_name)| {
            target_extension_names.contains(extension_name.as_str())
        })
        .map(|(registry_host_name, _extension_name)| registry_host_name.clone())
        .collect();

    Ok(reviews
        .iter()
        .filter(|review| {
            review
                .package
                .registries
                .iter()
                .any(|registry| enabled_registries.contains(&registry.host_name))
        })
        .cloned()
        .collect())
}

/// Request extension specification when multiple matching reviews found.
fn multiple_matching_ecosystems(
    reviews: &Vec<review::Review>,
    config: &common::config::Config,
) -> Result<()> {
    assert!(reviews.len() > 1);

    let registry_host_names: std::collections::BTreeSet<String> = reviews
        .iter()
        .map(|review| {
            review
                .package
                .registries
                .iter()
                .map(|registry| registry.host_name.clone())
        })
        .flatten()
        .collect();
    let extension_names: std::collections::BTreeSet<String> = config
        .extensions
        .registries
        .iter()
        .filter(|(registry_host_name, _extension_name)| {
            registry_host_names.contains(registry_host_name.as_str())
        })
        .map(|(_registry_host_name, extension_name)| extension_name.clone())
        .collect();
    let extension_names: Vec<String> = extension_names.into_iter().collect();

    return Err(format_err!(
        "Found multiple matching candidate packages.\n\
        Please specify an extension using --extension (-e).\n\
        Matching extensions: {}",
        extension_names.join(", ")
    ));
}

/// Setup new review for editing.
fn setup_new_review(
    package_name: &str,
    package_version: &str,
    registry_metadata: &Option<vouch_lib::extension::RegistryPackageMetadata>,
    extension_names: &BTreeSet<String>,
    config: &common::config::Config,
    tx: &StoreTransaction,
) -> Result<(review::Review, review::workspace::Manifest)> {
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;
    let (package, workspace_manifest) = ensure_package_setup(
        &package_name,
        &package_version,
        &registry_metadata,
        &extensions,
        &tx,
    )?;
    let review = get_insert_empty_review(&package, config, &tx)?;
    Ok((review, workspace_manifest))
}

/// Attempt to retrieve package from index.
/// Add package metadata using extension(s) if missing.
fn ensure_package_setup(
    package_name: &str,
    package_version: &str,
    registry_metadata: &Option<vouch_lib::extension::RegistryPackageMetadata>,
    extensions: &Vec<Box<dyn vouch_lib::extension::Extension>>,
    tx: &common::StoreTransaction,
) -> Result<(package::Package, review::workspace::Manifest)> {
    // Don't query registries again if results already found.
    let registry_metadata = match registry_metadata {
        Some(r) => r.clone(),
        None => {
            let all_registries_metadata =
                extension::search_registries(&package_name, &Some(package_version), &extensions)?;
            all_registries_metadata
                .iter()
                .find(|registry_metadata| registry_metadata.is_primary)
                .ok_or(format_err!(
                    "Failed to find primary registry metadata from extension."
                ))?
                .clone()
        }
    };

    // Get package version from found metadata incase given version was unknown.
    let package_version = registry_metadata.package_version.clone();

    let package = package::index::get(
        &package::index::Fields {
            package_name: Some(&package_name),
            package_version: Some(&package_version),
            registry_host_names: Some(
                maplit::btreeset! {registry_metadata.registry_host_name.as_str()},
            ),
            ..Default::default()
        },
        &tx,
    )?
    .into_iter()
    .next();

    let package = match package {
        Some(package) => {
            let registry = get_primary_registry(&package)?;
            let workspace_manifest = review::workspace::ensure(
                &package.name,
                &package.version,
                &registry.host_name,
                &registry.artifact_url,
            )?;
            (package, workspace_manifest)
        }
        None => {
            let registry = registry::index::ensure(
                &registry_metadata.registry_host_name,
                &url::Url::parse(&registry_metadata.human_url)?,
                &url::Url::parse(&registry_metadata.artifact_url)?,
                &tx,
            )?;
            let workspace_manifest = review::workspace::ensure(
                &package_name,
                &package_version,
                &registry.host_name,
                &registry.artifact_url,
            )?;
            let package = package::index::insert(
                &package_name,
                &package_version,
                &maplit::btreeset! {registry},
                &workspace_manifest.artifact_hash,
                &tx,
            )?;
            (package, workspace_manifest)
        }
    };
    Ok(package)
}

fn get_insert_empty_review(
    package: &package::Package,
    config: &common::config::Config,
    tx: &common::StoreTransaction,
) -> Result<review::Review> {
    let reviewer_peer = get_reviewer_peer(config, tx)?;
    let unset_review = review::index::insert(
        &std::collections::BTreeSet::<review::comment::Comment>::new(),
        &reviewer_peer,
        &package,
        &tx,
    )?;
    Ok(unset_review)
}

fn get_reviewer_peer(
    config: &common::config::Config,
    tx: &common::StoreTransaction,
) -> Result<peer::Peer> {
    peer::index::ensure_reviewer_peer(
        &config.core.reviewer_uuid,
        &config.core.api_base,
        tx,
    )
}

fn get_commit_message(
    package: &package::Package,
    target_file: &Option<std::path::PathBuf>,
    editing_mode: &ReviewEditMode,
) -> Result<String> {
    let message_prefix = match editing_mode {
        ReviewEditMode::Create => "Creating",
        ReviewEditMode::Update => "Updating",
    };
    let registry = get_primary_registry(&package)?;
    let target_file = target_file
        .as_ref()
        .map(|path| format!(" {}", path.display()))
        .unwrap_or_default();
    Ok(format!(
        "{message_prefix} review: {registry_host_name}/{package_name}/{package_version}{target_file}",
        message_prefix = message_prefix,
        registry_host_name = registry.host_name,
        package_name = package.name,
        package_version = package.version,
        target_file = target_file,
    ))
}
