use anyhow::{format_err, Result};
use directories;

/// Filesystem thirdpass config directory absolute paths.
#[derive(Debug)]
pub struct ConfigPaths {
    pub root_directory: std::path::PathBuf,
    pub config_file: std::path::PathBuf,
    pub extensions_directory: std::path::PathBuf,
}

impl ConfigPaths {
    pub fn new() -> Result<Self> {
        let user_directories = directories::ProjectDirs::from("", "", "thirdpass").ok_or(
            format_err!("Failed to obtain a handle on the local user directory."),
        )?;
        let root_directory = user_directories.config_dir();
        Ok(Self {
            root_directory: root_directory.into(),
            config_file: root_directory.join("config.yaml"),
            extensions_directory: root_directory.join("extensions"),
        })
    }
}

/// Filesystem thirdpass data directory absolute paths.
#[derive(Debug)]
pub struct DataPaths {
    pub root_directory: std::path::PathBuf,

    pub reviews_directory: std::path::PathBuf,
    pub pending_reviews_directory: std::path::PathBuf,
    pub ongoing_reviews_directory: std::path::PathBuf,
    pub archives_directory: std::path::PathBuf,
    pub dependency_queues_directory: std::path::PathBuf,
}

impl DataPaths {
    pub fn from_root_directory(root_directory: &std::path::Path) -> Result<Self> {
        Ok(Self {
            root_directory: root_directory.to_path_buf(),

            reviews_directory: root_directory.join("reviews"),
            pending_reviews_directory: root_directory.join("reviews").join(".pending"),
            ongoing_reviews_directory: root_directory.join("reviews").join(".ongoing"),
            archives_directory: root_directory.join("archives"),
            dependency_queues_directory: root_directory.join("dependency-queues"),
        })
    }

    pub fn new() -> Result<Self> {
        let user_directories = directories::ProjectDirs::from("", "", "thirdpass").ok_or(
            format_err!("Failed to obtain a handle on the local user directory."),
        )?;
        let root_directory = user_directories.data_local_dir();
        Self::from_root_directory(root_directory)
    }
}
