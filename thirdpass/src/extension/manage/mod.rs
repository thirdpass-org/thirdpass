use anyhow::{format_err, Result};

use crate::common::config::Config;
use crate::extension::process;

/// Update config with discoverable extensions.
pub fn update_config(config: &mut Config) -> Result<()> {
    log::debug!("Discover extensions and update config.");

    let extensions = process::get_all()?;
    let extension_name_map: std::collections::BTreeMap<_, _> = extensions
        .iter()
        .map(|extension| (extension.name(), extension))
        .collect();

    let all_found_names: std::collections::BTreeSet<_> =
        extension_name_map.keys().cloned().collect();

    let configured_names: std::collections::BTreeSet<_> =
        config.extensions.enabled.keys().cloned().collect();

    let stale_names: Vec<_> = configured_names.difference(&all_found_names).collect();
    let registries_map = config.extensions.registries.clone();
    for name in &stale_names {
        config.extensions.enabled.remove(*name);

        // Update registries map.
        for (registry, extension_name) in &registries_map {
            if *extension_name == **name {
                config.extensions.registries.remove(registry);
            }
        }
    }

    let new_names: Vec<_> = all_found_names.difference(&configured_names).collect();
    for name in &new_names {
        config.extensions.enabled.insert((**name).clone(), true);

        // Update registries map.
        if let Some(extension) = extension_name_map.get(name.as_str()) {
            for registry in extension.registries() {
                config
                    .extensions
                    .registries
                    .insert(registry, (*name).clone());
            }
        }
    }

    if !stale_names.is_empty() || !new_names.is_empty() {
        config.dump()?;
    }
    Ok(())
}

/// Enable extension.
pub fn enable(name: &str, config: &mut Config) -> Result<()> {
    if let Some(enabled_status) = config.extensions.enabled.get_mut(name) {
        *enabled_status = true;
        config.dump()?;
        Ok(())
    } else {
        Err(format_err!("Failed to find extension."))
    }
}

/// Disable extension.
pub fn disable(name: &str, config: &mut Config) -> Result<()> {
    if let Some(enabled_status) = config.extensions.enabled.get_mut(name) {
        *enabled_status = false;
        config.dump()?;
        Ok(())
    } else {
        Err(format_err!("Failed to find extension."))
    }
}

/// Given an extension's name, returns true if the extension is enabled. Otherwise returns false.
pub fn is_enabled(name: &str, config: &Config) -> Result<bool> {
    Ok(*config.extensions.enabled.get(name).unwrap_or(&false))
}

/// Returns enabled extensions.
pub fn get_enabled(
    names: &std::collections::BTreeSet<String>,
    config: &Config,
) -> Result<Vec<Box<dyn thirdpass_core::extension::Extension>>> {
    log::debug!("Identifying enabled extensions.");
    let extensions = process::get_all()?
        .into_iter()
        .filter(|extension| {
            *config
                .extensions
                .enabled
                .get(&extension.name())
                .unwrap_or(&false)
        })
        .filter(|extension| names.contains(&extension.name()))
        .collect();

    Ok(extensions)
}

/// Returns a set of all enabled installed extensions by names.
pub fn get_enabled_names(config: &Config) -> Result<std::collections::BTreeSet<String>> {
    Ok(config
        .extensions
        .enabled
        .iter()
        .filter(|(_name, enabled_flag)| **enabled_flag)
        .map(|(name, _enabled_flag)| name.clone())
        .collect())
}

pub fn get_all_names(config: &Config) -> Result<std::collections::BTreeSet<String>> {
    Ok(config.extensions.enabled.keys().cloned().collect())
}

/// Check given extensions are enabled. If not specified select all enabled extensions.
pub fn handle_extension_names_arg(
    extension_names: &Option<Vec<String>>,
    config: &Config,
) -> Result<std::collections::BTreeSet<String>> {
    let names = match &extension_names {
        Some(extension_names) => {
            let disabled_names: Vec<_> = extension_names
                .iter()
                .filter(|&name| !is_enabled(name, config).unwrap_or(false))
                .cloned()
                .collect();
            if !disabled_names.is_empty() {
                return Err(format_err!(
                    "The following disabled extensions were given: {}",
                    disabled_names.join(", ")
                ));
            } else {
                extension_names.iter().cloned().collect()
            }
        }
        None => get_enabled_names(config)?,
    };
    log::debug!("Using extensions: {:?}", names);
    Ok(names)
}

/// Clean extension name.
///
/// Example: thirdpass-py --> py
pub fn clean_name(name: &str) -> String {
    match &name.strip_prefix(process::EXTENSION_FILE_NAME_PREFIX) {
        Some(name) => name.to_string(),
        None => name.to_string(),
    }
}
