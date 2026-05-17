use anyhow::{format_err, Result};
use std::collections::HashMap;
use thirdpass_core::extension::{FromLib, FromProcess};

use crate::extension::common;

pub static EXTENSION_FILE_NAME_PREFIX: &str = "thirdpass-";

const RESERVED_PROCESS_NAMES: &[&str] = &["admin", "server"];

/// Return handles to all known extensions.
pub fn get_all() -> Result<Vec<Box<dyn thirdpass_core::extension::Extension>>> {
    log::debug!("Identifying all extensions.");

    let mut all_extensions = vec![
        Box::new(thirdpass_py_lib::PyExtension::new())
            as Box<dyn thirdpass_core::extension::Extension>,
        Box::new(thirdpass_js_lib::JsExtension::new())
            as Box<dyn thirdpass_core::extension::Extension>,
        Box::new(thirdpass_rs_lib::RsExtension::new())
            as Box<dyn thirdpass_core::extension::Extension>,
    ];

    for extension in get_process_extensions()? {
        all_extensions.push(Box::new(extension) as Box<dyn thirdpass_core::extension::Extension>);
    }

    Ok(all_extensions)
}

/// Discovers and loads process extensions.
fn get_process_extensions() -> Result<Vec<thirdpass_core::extension::ProcessExtension>> {
    let extension_paths = get_extension_paths()?;

    let mut threads = vec![];
    for (name, path) in extension_paths.iter() {
        let extension_config_path = common::get_config_path(name)?;
        let extension_name = name.clone();
        let process_path = path.clone();
        let process_path_for_thread = process_path.clone();

        let thread = std::thread::spawn(move || {
            thirdpass_core::extension::ProcessExtension::from_process(
                &process_path_for_thread,
                &extension_config_path,
            )
        });
        threads.push((extension_name, process_path, thread));
    }

    let mut valid_extensions = Vec::new();
    for (extension_name, process_path, thread) in threads {
        let extension = thread.join().unwrap_or_else(|panic| {
            Err(format_err!(
                "Extension {extension_name} panicked while loading {}: {}",
                process_path.display(),
                common::panic_payload_message(panic.as_ref())
            ))
        });

        match extension {
            Ok(v) => {
                valid_extensions.push(v);
            }
            Err(e) => {
                eprintln!(
                    "{extension_name}: Failed to load extension.\n{error}",
                    extension_name = process_path.display(),
                    error = e
                );
            }
        };
    }
    Ok(valid_extensions)
}

pub fn get_extension_paths() -> Result<HashMap<String, std::path::PathBuf>> {
    let mut result: HashMap<String, std::path::PathBuf> = HashMap::new();
    for path in get_candidate_extension_paths()? {
        // Skip non-valid paths.
        if !path.is_dir() && !path.is_file() {
            continue;
        }

        if path.is_file() {
            let name = match get_extension_name(&path)? {
                Some(name) => name,
                None => {
                    continue;
                }
            };
            result.insert(name, path);
            continue;
        }

        // Inspect file in directory. Does not investigate child directories.
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_file() {
                let name = match get_extension_name(&path)? {
                    Some(name) => name,
                    None => {
                        continue;
                    }
                };
                result.insert(name, path);
            }
        }
    }
    Ok(result)
}

fn get_candidate_extension_paths() -> Result<Vec<std::path::PathBuf>> {
    let env_path_value =
        std::env::var_os("PATH").ok_or(format_err!("Failed to read PATH environment variable."))?;
    let mut paths = std::env::split_paths(&env_path_value).collect::<Vec<_>>();

    let config_paths = crate::common::fs::ConfigPaths::new()?;
    if config_paths.extensions_directory.exists() {
        paths.push(config_paths.extensions_directory);
    }

    if let Some(extensions_home_directory) = crate::common::fs::get_extensions_default_directory() {
        if extensions_home_directory.exists() {
            paths.push(extensions_home_directory);
        }
    }
    Ok(paths)
}

fn get_extension_name(file_path: &std::path::Path) -> Result<Option<String>> {
    let file_name = file_path
        .file_name()
        .ok_or(format_err!("Failed to parse path file name."))?
        .to_str()
        .ok_or(format_err!("Failed to parse path file name into string."))?
        .to_string();

    let captures = match regex::Regex::new(&format!(
        "^{extension_file_name_prefix}([a-z]+).*",
        extension_file_name_prefix = EXTENSION_FILE_NAME_PREFIX
    ))?
    .captures(file_name.as_str())
    {
        Some(v) => v,
        None => {
            return Ok(None);
        }
    };

    let name = match captures.get(1) {
        Some(v) => v,
        None => {
            return Ok(None);
        }
    }
    .as_str();
    if RESERVED_PROCESS_NAMES.contains(&name) {
        return Ok(None);
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_name_reads_process_extension_file_name() -> Result<()> {
        assert_eq!(extension_name("/tmp/thirdpass-py")?, Some("py".to_string()));
        assert_eq!(
            extension_name("/tmp/thirdpass-py.d")?,
            Some("py".to_string())
        );
        Ok(())
    }

    #[test]
    fn extension_name_skips_core_thirdpass_binaries() -> Result<()> {
        assert_eq!(extension_name("/tmp/thirdpass-admin")?, None);
        assert_eq!(extension_name("/tmp/thirdpass-server")?, None);
        Ok(())
    }

    #[test]
    fn extension_name_ignores_non_extension_file_name() -> Result<()> {
        assert_eq!(extension_name("/tmp/thirdparty-py")?, None);
        Ok(())
    }

    fn extension_name(path: &str) -> Result<Option<String>> {
        get_extension_name(&std::path::PathBuf::from(path))
    }
}
