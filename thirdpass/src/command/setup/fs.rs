use anyhow::Result;

use crate::common;
use crate::extension;

fn handle_nonempty_data_directory(directory_path: &std::path::PathBuf, force: bool) -> Result<()> {
    let target_directory_empty = directory_path.read_dir()?.next().is_none();
    if force && !target_directory_empty {
        // Delete directory contents so setup can start cleanly.
        std::fs::remove_dir_all(&directory_path)?;
        std::fs::create_dir_all(&directory_path)?;
    }
    Ok(())
}

fn setup_data_directory_contents(paths: &common::fs::DataPaths) -> Result<()> {
    std::fs::create_dir_all(&paths.reviews_directory)?;
    std::fs::File::create(&paths.reviews_directory.join(".gitkeep"))?;

    std::fs::create_dir_all(&paths.pending_reviews_directory)?;
    std::fs::File::create(&paths.pending_reviews_directory.join(".gitkeep"))?;

    std::fs::create_dir_all(&paths.ongoing_reviews_directory)?;
    std::fs::File::create(&paths.ongoing_reviews_directory.join(".gitkeep"))?;

    std::fs::create_dir_all(&paths.archives_directory)?;
    std::fs::File::create(&paths.archives_directory.join(".gitkeep"))?;

    // TODO: Populate README.md with reasonable message, stats, links.
    let readme_file_path = paths.root_directory.join("README.md");
    if !readme_file_path.is_file() {
        std::fs::File::create(&readme_file_path)?;
    }
    Ok(())
}

/// Setup config directory.
///
/// If config file exists and force is false, file will not be modified.
fn setup_config(paths: &common::fs::ConfigPaths, force: bool) -> Result<()> {
    std::fs::create_dir_all(&paths.root_directory)?;
    std::fs::create_dir_all(&paths.extensions_directory)?;

    if force || !paths.config_file.is_file() {
        log::debug!("Generating config file: {}", paths.config_file.display());
        let mut config = crate::common::config::Config::default();

        config.core.api_base = "https://thirdpass.dev/api".to_string();
        config.core.client_id = uuid::Uuid::new_v4().to_hyphenated().to_string();
        config.review_tool.name = "agent".to_string();
        config.review_tool.install_check = false;
        config.review_tool.agent = Some("codex".to_string());
        config.review_tool.agent_model = Some("gpt-5.4".to_string());
        config.review_tool.agent_reasoning_effort = Some("high".to_string());
        extension::manage::update_config(&mut config)?;
        config.dump()?;
    } else {
        log::debug!(
            "Not overwriting existing config file (--force: {:?}): {}",
            force,
            paths.config_file.display()
        );
    }
    Ok(())
}

pub fn setup(force: bool) -> Result<()> {
    let data_paths = common::fs::DataPaths::new()?;
    log::debug!("Using data paths: {:#?}", data_paths);

    let config_paths = common::fs::ConfigPaths::new()?;
    log::debug!("Using config paths: {:#?}", config_paths);
    setup_config(&config_paths, force)?;
    log::debug!("Config setup complete.");

    log::debug!("Ensuring root data directory exists.");
    std::fs::create_dir_all(&data_paths.root_directory)?;

    handle_nonempty_data_directory(&data_paths.root_directory, force)?;

    setup_data_directory_contents(&data_paths)?;

    Ok(())
}

/// Returns true if setup is complete, otherwise returns false.
///
/// Checks for existence of config file and for reviews directory.
pub fn is_complete() -> Result<bool> {
    let config_paths = common::fs::ConfigPaths::new()?;
    let data_paths = common::fs::DataPaths::new()?;
    Ok(config_paths.config_file.is_file() && data_paths.reviews_directory.is_dir())
}
