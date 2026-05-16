use anyhow::{format_err, Result};

mod common;
pub mod manage;
mod process;

/// Identify all supported dependencies which are defined in a local file.
///
/// Conducts a parallel search across extensions.
pub fn identify_file_defined_dependencies(
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
    working_directory: &std::path::Path,
) -> Result<Vec<Result<Vec<thirdpass_core::extension::FileDefinedDependencies>>>> {
    let results = crossbeam_utils::thread::scope(|s| {
        let mut threads = Vec::new();
        for extension in extensions {
            let extension_name = extension.name();
            let thread = s.spawn(move |_| {
                extension.identify_file_defined_dependencies(working_directory, extension_args)
            });
            threads.push((extension_name, thread));
        }
        let mut result = Vec::new();
        for (extension_name, thread) in threads {
            result.push(thread.join().unwrap_or_else(|panic| {
                Err(format_err!(
                    "Extension {extension_name} panicked while identifying file-defined \
                     dependencies: {}",
                    common::panic_payload_message(panic.as_ref())
                ))
            }));
        }
        result
    })
    .map_err(|panic| {
        format_err!(
            "Extension file-defined dependency search scope panicked: {}",
            common::panic_payload_message(panic.as_ref())
        )
    })?;

    Ok(results)
}

/// Identify package dependencies.
///
/// Conducts a parallel search across extensions.
pub fn identify_package_dependencies(
    package_name: &str,
    package_version: &Option<&str>,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
) -> Result<Vec<Result<Vec<thirdpass_core::extension::PackageDependencies>>>> {
    let results = crossbeam_utils::thread::scope(|s| {
        let mut threads = Vec::new();
        for extension in extensions {
            let extension_name = extension.name();
            let thread = s.spawn(move |_| {
                extension.identify_package_dependencies(
                    package_name,
                    package_version,
                    extension_args,
                )
            });
            threads.push((extension_name, thread));
        }
        let mut result = Vec::new();
        for (extension_name, thread) in threads {
            result.push(thread.join().unwrap_or_else(|panic| {
                Err(format_err!(
                    "Extension {extension_name} panicked while identifying package \
                     dependencies: {}",
                    common::panic_payload_message(panic.as_ref())
                ))
            }));
        }
        result
    })
    .map_err(|panic| {
        format_err!(
            "Extension package dependency search scope panicked: {}",
            common::panic_payload_message(panic.as_ref())
        )
    })?;

    Ok(results)
}
