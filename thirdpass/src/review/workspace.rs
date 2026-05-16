use anyhow::Result;

pub use thirdpass_core::package::analyse;
pub use thirdpass_core::package::package_manifest;
pub use thirdpass_core::package::Manifest;

fn workspace_paths() -> Result<thirdpass_core::package::WorkspacePaths> {
    let data_paths = crate::common::fs::DataPaths::new()?;
    Ok(thirdpass_core::package::WorkspacePaths::new(
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
    thirdpass_core::package::ensure(
        &workspace_paths()?,
        package_name,
        package_version,
        registry_host_name,
        artifact_url,
    )
}

pub fn remove(workspace_manifest: &Manifest) -> Result<()> {
    thirdpass_core::package::remove(&workspace_paths()?, workspace_manifest)
}
