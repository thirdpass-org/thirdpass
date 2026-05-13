use anyhow::{format_err, Context, Result};
use std::convert::TryFrom;
use std::io::Write;

/// Archive formats that ThirdPass can download and unpack.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ArchiveType {
    /// A `.zip` archive.
    Zip,
    /// A `.tar.gz` archive.
    TarGz,
    /// A `.tgz` archive.
    Tgz,
    /// An unsupported or unknown archive format.
    Unknown,
}

impl std::convert::TryFrom<&std::path::PathBuf> for ArchiveType {
    type Error = anyhow::Error;

    fn try_from(path: &std::path::PathBuf) -> Result<Self, Self::Error> {
        Self::try_from(path.as_path())
    }
}

impl std::convert::TryFrom<&std::path::Path> for ArchiveType {
    type Error = anyhow::Error;

    fn try_from(path: &std::path::Path) -> Result<Self, Self::Error> {
        Ok(match get_file_extension(path)?.as_str() {
            "zip" => Self::Zip,
            "tar.gz" | "crate" => Self::TarGz,
            "tgz" => Self::Tgz,
            _ => Self::Unknown,
        })
    }
}

impl ArchiveType {
    /// Return the file extension normally used for this archive type.
    pub fn try_to_string(&self) -> Result<String> {
        Ok(match self {
            ArchiveType::Zip => "zip",
            ArchiveType::TarGz => "tar.gz",
            ArchiveType::Tgz => "tgz",
            ArchiveType::Unknown => {
                return Err(format_err!(
                    "Failed to convert unknown archive type into string."
                ))
            }
        }
        .to_string())
    }
}

fn get_file_extension(path: &std::path::Path) -> Result<String> {
    if path
        .to_str()
        .ok_or(format_err!("Failed to parse URL path as str."))?
        .ends_with(".tar.gz")
    {
        return Ok("tar.gz".to_string());
    }

    match path.extension() {
        Some(extension) => extension.to_str().map(ToOwned::to_owned).ok_or(format_err!(
            "Failed to parse file extension unicode characters."
        )),
        None => Ok(String::new()),
    }
}

/// Extract an archive into the destination directory and return its root.
pub fn extract(
    archive_path: &std::path::PathBuf,
    destination_directory: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    log::debug!("Extracting archive: {}", archive_path.display());
    let archive_type = ArchiveType::try_from(archive_path)?;
    let workspace_directory = match archive_type {
        ArchiveType::Zip => extract_zip(archive_path, destination_directory)?,
        ArchiveType::Tgz | ArchiveType::TarGz => {
            extract_tar_gz(archive_path, destination_directory)?
        }
        ArchiveType::Unknown => {
            return Err(format_err!(
                "Archive extraction failed. Unsupported archive file type: {}",
                archive_path.display()
            ));
        }
    };
    log::debug!(
        "Archive extraction complete. Workspace directory: {}",
        workspace_directory.display()
    );
    Ok(workspace_directory)
}

fn extract_zip(
    archive_path: &std::path::PathBuf,
    destination_directory: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let file = std::fs::File::open(archive_path).context(format!(
        "Can't open zip archive: {}",
        archive_path.display()
    ))?;
    let mut archive = zip::ZipArchive::new(file)?;

    let extracted_directory =
        destination_directory.join(archive.by_index(0)?.enclosed_name().ok_or(format_err!(
            "Archive is unexpectedly empty: {}",
            archive_path.display()
        ))?);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let output_path = match file.enclosed_name() {
            Some(path) => path.to_owned(),
            None => continue,
        };
        let output_path = destination_directory.join(output_path);

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&output_path)?;
        } else {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut output_file = std::fs::File::create(&output_path)?;
            std::io::copy(&mut file, &mut output_file)?;
        }
    }
    Ok(extracted_directory)
}

fn extract_tar_gz(
    archive_path: &std::path::PathBuf,
    destination_directory: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    let root_layout = get_tar_root_layout(archive_path)?;

    let file = std::fs::File::open(archive_path).context(format!(
        "Can't open tar archive: {}",
        archive_path.display()
    ))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(destination_directory).context(format!(
        "Can't unpack archive into destination directory: {}",
        destination_directory.display()
    ))?;

    let workspace_directory = if let TarRootLayout::TopDirectory(top_directory_name) = root_layout {
        log::debug!(
            "Found archive top level directory name: {}",
            top_directory_name
        );
        destination_directory.join(top_directory_name)
    } else {
        log::debug!("Archive top level directory not found. Creating stand-in.");

        let uuid = uuid::Uuid::new_v4();
        let mut encode_buffer = uuid::Uuid::encode_buffer();
        let uuid = uuid.to_hyphenated().encode_lower(&mut encode_buffer);
        let workspace_directory_name = "thirdpass-workspace-".to_string() + uuid;

        let workspace_directory = destination_directory.join(workspace_directory_name);
        std::fs::create_dir(&workspace_directory)?;

        let paths = std::fs::read_dir(destination_directory)?;
        for path in paths {
            let file_name = path?.file_name();
            let path = destination_directory.join(&file_name);
            if path == workspace_directory || &path == archive_path {
                continue;
            }
            std::fs::rename(&path, workspace_directory.join(&file_name))?;
        }

        workspace_directory
    };

    log::debug!(
        "Using workspace directory: {}",
        workspace_directory.display()
    );

    Ok(workspace_directory)
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TarRootLayout {
    TopDirectory(String),
    Flat,
}

fn get_tar_root_layout(archive_path: &std::path::PathBuf) -> Result<TarRootLayout> {
    let file = std::fs::File::open(archive_path).context(format!(
        "Can't open tar archive: {}",
        archive_path.display()
    ))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut top_directory_name = None::<String>;
    let mut saw_child_path = false;

    for entry in archive.entries()? {
        let entry = entry?;
        let path = (*entry.path()?).to_path_buf();
        let mut components = path.components();
        let first_component = match components.next() {
            Some(std::path::Component::Normal(component)) => component
                .to_str()
                .ok_or(format_err!("Failed to parse archive's first path."))?
                .to_string(),
            Some(_) => return Ok(TarRootLayout::Flat),
            None => continue,
        };

        if top_directory_name
            .as_ref()
            .map_or(false, |expected| expected != &first_component)
        {
            return Ok(TarRootLayout::Flat);
        }
        if components.next().is_some() || entry.header().entry_type().is_dir() {
            saw_child_path = true;
        }
        top_directory_name.get_or_insert(first_component);
    }

    match (top_directory_name, saw_child_path) {
        (Some(top_directory_name), true) => Ok(TarRootLayout::TopDirectory(top_directory_name)),
        (Some(_), false) => Ok(TarRootLayout::Flat),
        (None, _) => Err(format_err!("Archive empty.")),
    }
}

/// Download a package archive to the requested local path.
pub fn download(target_url: &url::Url, destination_path: &std::path::PathBuf) -> Result<()> {
    log::debug!(
        "Downloading archive to destination path: {}",
        destination_path.display()
    );

    let response = reqwest::blocking::get(target_url.clone())?
        .error_for_status()
        .context(format!(
            "Failed to download package archive: {}",
            target_url
        ))?;
    let mut file = std::fs::File::create(destination_path).context(format!(
        "Can't create archive destination file: {}",
        destination_path.display()
    ))?;
    let content = response.bytes()?;
    file.write_all(&content)?;
    file.sync_all()?;

    log::debug!("Finished writing archive.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correct_extension_extracted_for_tar_gz() -> Result<()> {
        let result = get_file_extension(&std::path::PathBuf::from("/d3/d3-4.10.0.tar.gz"))?;
        let expected = "tar.gz".to_string();
        assert_eq!(result, expected);
        Ok(())
    }

    #[test]
    fn test_crate_archives_are_treated_as_tar_gz() -> Result<()> {
        let result = ArchiveType::try_from(&std::path::PathBuf::from("/serde/serde-1.0.0.crate"))?;
        assert_eq!(result, ArchiveType::TarGz);
        Ok(())
    }

    #[test]
    fn test_extensionless_archives_are_unknown() -> Result<()> {
        let result = ArchiveType::try_from(&std::path::PathBuf::from("/downloads/archive"))?;
        assert_eq!(result, ArchiveType::Unknown);
        Ok(())
    }

    #[test]
    fn tar_gz_with_top_directory_uses_that_directory_as_root() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive_path = tmp.path().join("package.tar.gz");
        let destination = tmp.path().join("out");
        std::fs::create_dir_all(&destination)?;
        write_tar_gz(
            &archive_path,
            &[
                ("package-1.0.0/src/lib.rs", b"pub fn lib() {}\n"),
                ("package-1.0.0/README.md", b"# package\n"),
            ],
        )?;

        let root = extract_tar_gz(&archive_path, &destination)?;

        assert_eq!(root, destination.join("package-1.0.0"));
        assert!(root.is_dir());
        assert!(root.join("src/lib.rs").is_file());
        Ok(())
    }

    #[test]
    fn flat_tar_gz_uses_stand_in_directory_as_root() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive_path = tmp.path().join("collection.tar.gz");
        let destination = tmp.path().join("out");
        std::fs::create_dir_all(&destination)?;
        write_tar_gz(
            &archive_path,
            &[
                ("FILES.json", b"{}\n"),
                ("plugins/lookup/protonpass.py", b"print('lookup')\n"),
                ("meta/runtime.yml", b"requires_ansible: '>=2.15'\n"),
            ],
        )?;

        let root = extract_tar_gz(&archive_path, &destination)?;

        assert!(root.is_dir());
        assert!(root.join("FILES.json").is_file());
        assert!(root.join("plugins/lookup/protonpass.py").is_file());
        assert!(!destination.join("FILES.json").exists());
        Ok(())
    }

    #[test]
    fn single_file_tar_gz_uses_stand_in_directory_as_root() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive_path = tmp.path().join("single-file.tar.gz");
        let destination = tmp.path().join("out");
        std::fs::create_dir_all(&destination)?;
        write_tar_gz(&archive_path, &[("package-1.0.0", b"plain file\n")])?;

        let root = extract_tar_gz(&archive_path, &destination)?;

        assert!(root.is_dir());
        assert!(root.join("package-1.0.0").is_file());
        Ok(())
    }

    fn write_tar_gz(archive_path: &std::path::Path, entries: &[(&str, &[u8])]) -> Result<()> {
        let file = std::fs::File::create(archive_path)?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (path, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, *contents)?;
        }
        archive.finish()?;
        let encoder = archive.into_inner()?;
        encoder.finish()?;
        Ok(())
    }
}
