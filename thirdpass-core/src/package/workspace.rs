use anyhow::{format_err, Context, Result};
use std::convert::TryFrom;
use std::io::Write;

use crate::package::{analysis, archive};

static MANIFEST_FILE_NAME: &str = "manifest.json";
static CACHED_ARCHIVE_FILE_NAME: &str = "archive";

/// Local directories used to cache package archives and workspaces.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspacePaths {
    /// Directory containing extracted review workspaces.
    pub ongoing_reviews_directory: std::path::PathBuf,
    /// Directory containing cached package archives.
    pub archives_directory: std::path::PathBuf,
}

impl WorkspacePaths {
    /// Build workspace paths from explicit ongoing-review and archive directories.
    pub fn new(
        ongoing_reviews_directory: std::path::PathBuf,
        archives_directory: std::path::PathBuf,
    ) -> Self {
        Self {
            ongoing_reviews_directory,
            archives_directory,
        }
    }
}

/// Manifest for a prepared package workspace.
#[derive(
    Debug, Clone, Default, Ord, PartialOrd, Eq, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct Manifest {
    /// Extracted package workspace root.
    pub workspace_path: std::path::PathBuf,
    /// Path of the local workspace manifest file.
    pub manifest_path: std::path::PathBuf,
    /// Cached source archive path.
    pub artifact_path: std::path::PathBuf,
    /// Blake3 digest of the cached source archive.
    pub package_hash: String,
}

/// Return the stable relative package path used for local storage.
pub fn unique_package_path(
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<std::path::PathBuf> {
    let registry_host_name = std::path::PathBuf::from(registry_host_name);
    Ok(registry_host_name.join(package_name).join(package_version))
}

fn archive_file_name(archive_type: archive::ArchiveType) -> Result<String> {
    let uuid = uuid::Uuid::new_v4();
    let mut encode_buffer = uuid::Uuid::encode_buffer();
    let uuid = uuid.to_hyphenated().encode_lower(&mut encode_buffer);
    Ok(format!(
        "archive-{}.{}",
        uuid,
        archive_type.try_to_string()?
    ))
}

fn cached_archive_path(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
    archive_type: archive::ArchiveType,
) -> Result<std::path::PathBuf> {
    let package_path = unique_package_path(package_name, package_version, registry_host_name)?;
    let file_name = format!(
        "{}.{}",
        CACHED_ARCHIVE_FILE_NAME,
        archive_type.try_to_string()?
    );
    Ok(paths.archives_directory.join(package_path).join(file_name))
}

fn find_cached_archive(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<Option<std::path::PathBuf>> {
    let package_path = unique_package_path(package_name, package_version, registry_host_name)?;
    let archive_directory = paths.archives_directory.join(package_path);
    if !archive_directory.is_dir() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&archive_directory)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(file_name) => file_name,
            None => continue,
        };
        if file_name.starts_with(&format!("{}.", CACHED_ARCHIVE_FILE_NAME)) {
            candidates.push(path);
        }
    }

    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort();
    Ok(candidates.pop())
}

fn ensure_cached_archive_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context(format!(
            "Can't create archive cache directory: {}",
            parent.display()
        ))?;
    }
    Ok(())
}

/// Ensure a package archive is cached and extracted into a review workspace.
pub fn ensure(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
    artifact_url: &url::Url,
) -> Result<Manifest> {
    if let Some(workspace_manifest) =
        get_existing(paths, package_name, package_version, registry_host_name)?
    {
        return Ok(workspace_manifest);
    }

    let package_unique_directory =
        setup_unique_package_directory(paths, package_name, package_version, registry_host_name)?;
    let cached_archive =
        match find_cached_archive(paths, package_name, package_version, registry_host_name)? {
            Some(path) => path,
            None => {
                let archive_type =
                    archive::ArchiveType::try_from(std::path::Path::new(artifact_url.path()))?;
                if archive_type == archive::ArchiveType::Unknown {
                    return Err(format_err!(
                        "Unsupported archive file type: {}",
                        artifact_url
                    ));
                }

                let cached_archive = cached_archive_path(
                    paths,
                    package_name,
                    package_version,
                    registry_host_name,
                    archive_type,
                )?;

                if !cached_archive.is_file() {
                    let download_path =
                        package_unique_directory.join(archive_file_name(archive_type)?);
                    archive::download(artifact_url, &download_path)?;
                    ensure_cached_archive_parent(&cached_archive)?;
                    std::fs::copy(&download_path, &cached_archive).context(format!(
                        "Can't copy archive into cache: {}",
                        cached_archive.display()
                    ))?;
                    std::fs::remove_file(&download_path).context(format!(
                        "Can't remove temporary archive download: {}",
                        download_path.display()
                    ))?;
                }
                cached_archive
            }
        };

    let package_hash = analysis::file_blake3_digest(&cached_archive)?;

    let workspace_directory = archive::extract(&cached_archive, &package_unique_directory)?;

    let workspace_directory = normalize_workspace_directory_name(
        &workspace_directory,
        &package_unique_directory,
        package_name,
        package_version,
    )?;

    let workspace_manifest = Manifest {
        workspace_path: workspace_directory,
        manifest_path: get_manifest_path(&package_unique_directory),
        artifact_path: cached_archive,
        package_hash,
    };
    write_manifest(&workspace_manifest)?;
    Ok(workspace_manifest)
}

fn get_manifest_path(package_unique_directory: &std::path::Path) -> std::path::PathBuf {
    package_unique_directory.join(MANIFEST_FILE_NAME)
}

fn write_manifest(workspace_manifest: &Manifest) -> Result<()> {
    log::debug!(
        "Writing workspace manifest: {}",
        workspace_manifest.manifest_path.display()
    );
    let path = &workspace_manifest.manifest_path;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .append(false)
        .create(true)
        .truncate(true)
        .open(path)
        .context(format!(
            "Can't open/create file for writing: {}",
            path.display()
        ))?;
    file.write_all(serde_json::to_string_pretty(workspace_manifest)?.as_bytes())?;
    Ok(())
}

fn read_manifest(path: &std::path::PathBuf) -> Result<Manifest> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    // serde_yaml can read the JSON manifests written by current clients and
    // any older YAML manifests left in local workspaces.
    Ok(serde_yaml::from_reader(reader)?)
}

/// Return an existing prepared workspace manifest when present.
pub fn get_existing(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<Option<Manifest>> {
    let package_unique_directory =
        get_unique_package_directory(paths, package_name, package_version, registry_host_name)?;
    let manifest_path = get_manifest_path(&package_unique_directory);
    if manifest_path.is_file() {
        Ok(Some(read_manifest(&manifest_path)?))
    } else {
        Ok(None)
    }
}

fn get_unique_package_directory(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<std::path::PathBuf> {
    let package_unique_directory = paths.ongoing_reviews_directory.join(unique_package_path(
        package_name,
        package_version,
        registry_host_name,
    )?);
    Ok(package_unique_directory)
}

fn setup_unique_package_directory(
    paths: &WorkspacePaths,
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
) -> Result<std::path::PathBuf> {
    let package_unique_directory =
        get_unique_package_directory(paths, package_name, package_version, registry_host_name)?;
    std::fs::create_dir_all(&package_unique_directory).context(format!(
        "Can't create directory: {}",
        package_unique_directory.display()
    ))?;
    Ok(package_unique_directory)
}

fn get_workspace_directory_name(package_name: &str, package_version: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}-{}", package_name, package_version))
}

fn normalize_workspace_directory_name(
    workspace_directory: &std::path::PathBuf,
    parent_directory: &std::path::Path,
    package_name: &str,
    package_version: &str,
) -> Result<std::path::PathBuf> {
    let target_directory =
        parent_directory.join(get_workspace_directory_name(package_name, package_version));
    log::debug!(
        "Normalize workspace directory name: {}, {}",
        workspace_directory.display(),
        target_directory.display(),
    );
    std::fs::rename(workspace_directory, &target_directory).context(format!(
        "Can't normalize workspace directory from {} to {}",
        workspace_directory.display(),
        target_directory.display()
    ))?;
    Ok(target_directory)
}

/// Remove an extracted workspace and any empty workspace parent directories.
pub fn remove(paths: &WorkspacePaths, workspace_manifest: &Manifest) -> Result<()> {
    log::debug!(
        "Removing workspace directory: {}",
        workspace_manifest.workspace_path.display()
    );
    std::fs::remove_dir_all(&workspace_manifest.workspace_path).context(format!(
        "Can't remove workspace directory: {}",
        workspace_manifest.workspace_path.display()
    ))?;

    if workspace_manifest.manifest_path.is_file() {
        log::debug!(
            "Removing workspace manifest file: {}",
            workspace_manifest.manifest_path.display()
        );
        std::fs::remove_file(&workspace_manifest.manifest_path).context(format!(
            "Can't remove workspace manifest file: {}",
            workspace_manifest.manifest_path.display()
        ))?;
    }

    remove_empty_workspace_directories(
        &workspace_manifest.workspace_path,
        &paths.ongoing_reviews_directory,
    )?;
    Ok(())
}

fn remove_empty_workspace_directories(
    workspace_path: &std::path::Path,
    working_directory: &std::path::PathBuf,
) -> Result<()> {
    let mut absolute_path = if workspace_path.is_absolute() {
        workspace_path.to_path_buf()
    } else {
        working_directory.join(workspace_path)
    };
    while absolute_path.starts_with(working_directory) && &absolute_path != working_directory {
        if !absolute_path.exists() {
            absolute_path.pop();
            continue;
        }
        if std::fs::remove_dir(&absolute_path).is_err() {
            break;
        }
        absolute_path.pop();
    }
    Ok(())
}
