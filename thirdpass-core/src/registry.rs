use anyhow::{format_err, Result};

/// Search package registries via extensions for package metadata.
pub fn search_registries(
    package_name: &str,
    package_version: Option<&str>,
    extensions: &[Box<dyn crate::extension::Extension>],
) -> Result<Vec<crate::extension::RegistryPackageMetadata>> {
    log::debug!("Querying extensions for package metadata from registries.");
    type SearchResults = Result<Vec<Result<Vec<crate::extension::RegistryPackageMetadata>>>>;
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

    let extensions_search_results = search_results
        .map(|search_result| search_result.into_iter().zip(extensions.iter()).collect())?;
    select_search_result(extensions_search_results)
}

fn select_search_result<'a>(
    extensions_search_results: Vec<(
        Result<Vec<crate::extension::RegistryPackageMetadata>>,
        &'a Box<dyn crate::extension::Extension>,
    )>,
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
    let primary_registry = remote_package_metadata
        .iter()
        .find(|registry_metadata| registry_metadata.is_primary)
        .ok_or(format_err!(
            "Failed to find primary registry metadata from extension."
        ))?;
    let package_version = primary_registry.package_version.clone();
    Ok((package_version, primary_registry.clone()))
}

/// Resolve primary registry metadata for a specific package version.
pub fn primary_package_metadata(
    package_name: &str,
    package_version: &str,
    extensions: &[Box<dyn crate::extension::Extension>],
) -> Result<crate::extension::RegistryPackageMetadata> {
    let remote_package_metadata =
        search_registries(package_name, Some(package_version), extensions)?;
    let primary_registry = remote_package_metadata
        .iter()
        .find(|registry_metadata| registry_metadata.is_primary)
        .ok_or(format_err!(
            "Failed to find primary registry metadata from extension."
        ))?;
    Ok(primary_registry.clone())
}
