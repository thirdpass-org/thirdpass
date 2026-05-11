use anyhow::Result;
use crossbeam_utils;

mod common;
pub mod manage;
mod process;

/// Identify all supported dependencies which are defined in a local file.
///
/// Conducts a parallel search across extensions.
pub fn identify_file_defined_dependencies(
    extensions: &Vec<Box<dyn thirdpass_core::extension::Extension>>,
    extension_args: &Vec<String>,
    working_directory: &std::path::PathBuf,
) -> Result<Vec<Result<Vec<thirdpass_core::extension::FileDefinedDependencies>>>> {
    crossbeam_utils::thread::scope(|s| {
        let mut threads = Vec::new();
        for extension in extensions {
            threads.push(s.spawn(move |_| {
                extension.identify_file_defined_dependencies(&working_directory, &extension_args)
            }));
        }
        let mut result = Vec::new();
        for thread in threads {
            result.push(thread.join().unwrap());
        }
        Ok(result)
    })
    .unwrap()
}

/// Identify package dependencies.
///
/// Conducts a parallel search across extensions.
pub fn identify_package_dependencies(
    package_name: &str,
    package_version: &Option<&str>,
    extensions: &Vec<Box<dyn thirdpass_core::extension::Extension>>,
    extension_args: &Vec<String>,
) -> Result<Vec<Result<Vec<thirdpass_core::extension::PackageDependencies>>>> {
    crossbeam_utils::thread::scope(|s| {
        let mut threads = Vec::new();
        for extension in extensions {
            threads.push(s.spawn(move |_| {
                extension.identify_package_dependencies(
                    &package_name,
                    &package_version,
                    &extension_args,
                )
            }));
        }
        let mut result = Vec::new();
        for thread in threads {
            result.push(thread.join().unwrap());
        }
        Ok(result)
    })
    .unwrap()
}
