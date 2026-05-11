use anyhow::{format_err, Result};

type RegistryMetadataResult = Result<Vec<crate::extension::RegistryPackageMetadata>>;

/// Search package registries via extensions for package metadata.
pub fn search_registries(
    package_name: &str,
    package_version: Option<&str>,
    extensions: &[Box<dyn crate::extension::Extension>],
) -> Result<Vec<crate::extension::RegistryPackageMetadata>> {
    log::debug!("Querying extensions for package metadata from registries.");
    type SearchResults = Result<Vec<RegistryMetadataResult>>;
    let search_results: SearchResults = crossbeam_utils::thread::scope(|s| {
        let threads: Vec<_> = extensions
            .iter()
            .map(|extension| {
                s.spawn(move |_| {
                    extension.registries_package_metadata(package_name, &package_version)
                })
            })
            .collect();
        Ok(threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect())
    })
    .unwrap();

    let extensions_search_results = search_results.map(|search_result| {
        search_result
            .into_iter()
            .zip(
                extensions
                    .iter()
                    .map(|extension| extension.as_ref() as &dyn crate::extension::Extension),
            )
            .collect()
    })?;
    select_search_result(extensions_search_results)
}

fn select_search_result(
    extensions_search_results: Vec<(RegistryMetadataResult, &dyn crate::extension::Extension)>,
) -> Result<Vec<crate::extension::RegistryPackageMetadata>> {
    let mut selection = Err(format_err!(
        "Extensions have failed to find package in package registries."
    ));
    let mut ok_extension_names = Vec::<_>::new();

    for (search_result, extension) in extensions_search_results.into_iter() {
        if search_result.is_err() {
            log::debug!(
                "Extension {} returned error:\n{:?}",
                extension.name(),
                search_result
            );
            continue;
        }

        ok_extension_names.push(extension.name());
        selection = search_result;
    }

    if ok_extension_names.len() > 1 {
        Err(format_err!(
            "Found multiple matching candidate packages.\n\
        Limit registry lookup to one extension.\n\
        Matching extensions: {}",
            ok_extension_names.join(", ")
        ))
    } else {
        selection
    }
}

/// Resolve the latest package version and its primary registry metadata.
pub fn latest_package_metadata(
    package_name: &str,
    extensions: &[Box<dyn crate::extension::Extension>],
) -> Result<(String, crate::extension::RegistryPackageMetadata)> {
    let remote_package_metadata = search_registries(package_name, None, extensions)?;
    let primary_registry = select_primary_metadata(&remote_package_metadata)?;
    let package_version = primary_registry.package_version.clone();
    Ok((package_version, primary_registry))
}

/// Resolve primary registry metadata for a specific package version.
pub fn primary_package_metadata(
    package_name: &str,
    package_version: &str,
    extensions: &[Box<dyn crate::extension::Extension>],
) -> Result<crate::extension::RegistryPackageMetadata> {
    let remote_package_metadata =
        search_registries(package_name, Some(package_version), extensions)?;
    select_primary_metadata(&remote_package_metadata)
}

fn select_primary_metadata(
    remote_package_metadata: &[crate::extension::RegistryPackageMetadata],
) -> Result<crate::extension::RegistryPackageMetadata> {
    remote_package_metadata
        .iter()
        .find(|registry_metadata| registry_metadata.is_primary)
        .ok_or(format_err!(
            "Failed to find primary registry metadata from extension."
        ))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_metadata_selection_returns_primary_registry() -> Result<()> {
        let metadata = vec![
            registry_metadata(false, "1.0.0"),
            registry_metadata(true, "2.0.0"),
        ];

        let selected = select_primary_metadata(&metadata)?;

        assert_eq!(selected.package_version, "2.0.0");
        Ok(())
    }

    #[test]
    fn primary_metadata_selection_requires_primary_registry() {
        let metadata = vec![registry_metadata(false, "1.0.0")];

        assert!(select_primary_metadata(&metadata).is_err());
    }

    fn registry_metadata(
        is_primary: bool,
        package_version: &str,
    ) -> crate::extension::RegistryPackageMetadata {
        crate::extension::RegistryPackageMetadata {
            registry_host_name: "registry.example".to_string(),
            human_url: "https://registry.example/packages/pkg".to_string(),
            artifact_url: "https://registry.example/packages/pkg.tgz".to_string(),
            is_primary,
            package_version: package_version.to_string(),
        }
    }
}
