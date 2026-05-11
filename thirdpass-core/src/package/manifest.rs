use anyhow::Result;

/// Build a manifest of regular files in the extracted package workspace.
pub fn package_manifest(
    workspace_directory: &std::path::Path,
) -> Result<crate::schema::PackageManifest> {
    let mut files = Vec::new();
    collect_package_manifest_files(workspace_directory, workspace_directory, &mut files)?;
    files.sort();
    Ok(crate::schema::PackageManifest { files })
}

fn collect_package_manifest_files(
    root: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<crate::schema::PackageManifestFile>,
) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            collect_package_manifest_files(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }

        let relative_path = path.strip_prefix(root)?.to_path_buf();
        files.push(crate::schema::PackageManifestFile {
            path: package_manifest_path(&relative_path),
            size_bytes: metadata.len(),
        });
    }
    Ok(())
}

fn package_manifest_path(path: &std::path::Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manifest_lists_regular_files_with_sizes() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let workspace = tmp.path().to_path_buf();
        std::fs::create_dir_all(workspace.join("lib/core"))?;
        std::fs::write(workspace.join("index.js"), b"12345")?;
        std::fs::write(workspace.join("lib/core/axios.js"), b"123")?;

        let manifest = package_manifest(&workspace)?;

        assert_eq!(
            manifest.files,
            vec![
                crate::schema::PackageManifestFile {
                    path: "index.js".to_string(),
                    size_bytes: 5,
                },
                crate::schema::PackageManifestFile {
                    path: "lib/core/axios.js".to_string(),
                    size_bytes: 3,
                },
            ]
        );
        Ok(())
    }
}
