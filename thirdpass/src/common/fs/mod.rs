use anyhow::{format_err, Result};
use directories;

pub mod archive;

pub fn ensure_extensions_bin_directory() -> Result<Option<std::path::PathBuf>> {
    // Attempt to create an extensions directory in the users home directory.
    let extensions_directory = get_extensions_default_directory();

    // Use user local bin if previous path is None.
    let extensions_directory = extensions_directory.or(dirs::executable_dir());

    // Ensure directory exists.
    if let Some(extensions_directory) = &extensions_directory {
        if !extensions_directory.exists() {
            log::debug!(
                "Creating Thirdpass extensions bin directory: {}",
                extensions_directory.display()
            );
            std::fs::create_dir_all(&extensions_directory)?;
            set_directory_hidden_windows(&extensions_directory);
        }
    }
    Ok(extensions_directory)
}

/// Does not create the directory.
/// Returns None if home directory does not exist.
pub fn get_extensions_default_directory() -> Option<std::path::PathBuf> {
    let extensions_directory_name = ".thirdpass_extensions";

    match dirs::home_dir() {
        Some(home_directory) => {
            if !home_directory.exists() {
                None
            } else {
                let extensions_directory = home_directory.join(extensions_directory_name);
                Some(extensions_directory)
            }
        }
        None => None,
    }
}

#[cfg(windows)]
fn set_directory_hidden_windows(directory: &std::path::PathBuf) {
    // TODO: Hide directory on Windows.
    // winapi::um::fileapi::SetFileAttributesA()
}

#[cfg(not(windows))]
fn set_directory_hidden_windows(_directory: &std::path::PathBuf) {}

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
}

impl DataPaths {
    pub fn from_root_directory(root_directory: &std::path::PathBuf) -> Result<Self> {
        Ok(Self {
            root_directory: root_directory.clone(),

            reviews_directory: root_directory.join("reviews"),
            pending_reviews_directory: root_directory.join("reviews").join(".pending"),
            ongoing_reviews_directory: root_directory.join("reviews").join(".ongoing"),
            archives_directory: root_directory.join("archives"),
        })
    }

    pub fn new() -> Result<Self> {
        let user_directories = directories::ProjectDirs::from("", "", "thirdpass").ok_or(
            format_err!("Failed to obtain a handle on the local user directory."),
        )?;
        let root_directory = user_directories.data_local_dir();
        Self::from_root_directory(&root_directory.into())
    }

    /// Returns true if the given absolute path is protected from deletion, otherwise false.
    pub fn is_protected(&self, absolute_path: &std::path::PathBuf) -> bool {
        absolute_path == &self.root_directory
            || absolute_path == &self.reviews_directory
            || absolute_path == &self.pending_reviews_directory
            || absolute_path == &self.ongoing_reviews_directory
            || absolute_path == &self.archives_directory
    }
}

/// Remove empty directories along relative path.
pub fn remove_empty_directories(
    relative_path: &std::path::PathBuf,
    working_directory: &std::path::PathBuf,
) -> Result<()> {
    let paths = DataPaths::new()?;

    let mut absolute_path = working_directory.join(relative_path);
    while &absolute_path != working_directory {
        if paths.is_protected(&absolute_path) {
            break;
        }
        if !absolute_path.exists() {
            absolute_path.pop();
            continue;
        }
        if std::fs::remove_dir(&absolute_path).is_err() {
            // Found first non-empty directory.
            break;
        }
        absolute_path.pop();
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum PathType {
    File,
    Directory,
}

fn blake3_digest<R: std::io::Read>(mut reader: R) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hasher.finalize().to_hex().as_str().to_string())
}

fn hash_file(path: &std::path::PathBuf) -> Result<String> {
    let input = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(input);
    Ok(blake3_digest(reader)?)
}

pub fn hash(path: &std::path::PathBuf) -> Result<(String, PathType)> {
    if path.is_file() {
        return Ok((hash_file(&path)?, PathType::File));
    } else {
        unimplemented!("Only file hashing is currently implemented.");
    }
}
