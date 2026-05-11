use anyhow::Result;

pub use thirdpass_core::package::analysis::analyse;
pub use thirdpass_core::package::manifest::package_manifest;
pub use thirdpass_core::package::workspace::Manifest;

fn workspace_paths() -> Result<thirdpass_core::package::workspace::WorkspacePaths> {
    let data_paths = crate::common::fs::DataPaths::new()?;
    Ok(thirdpass_core::package::workspace::WorkspacePaths::new(
        data_paths.ongoing_reviews_directory,
        data_paths.archives_directory,
    ))
}

pub fn ensure(
    package_name: &str,
    package_version: &str,
    registry_host_name: &str,
    artifact_url: &url::Url,
) -> Result<Manifest> {
    thirdpass_core::package::workspace::ensure(
        &workspace_paths()?,
        package_name,
        package_version,
        registry_host_name,
        artifact_url,
    )
}

pub fn remove(workspace_manifest: &Manifest) -> Result<()> {
    thirdpass_core::package::workspace::remove(&workspace_paths()?, workspace_manifest)
}
