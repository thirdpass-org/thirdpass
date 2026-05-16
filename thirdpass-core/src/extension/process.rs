use anyhow::{format_err, Context, Result};

use super::common;

/// Static metadata advertised by a process-backed extension.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct StaticData {
    /// Extension short name.
    pub name: String,
    /// Registry hosts supported by the extension.
    pub registry_host_names: Vec<String>,
    /// Automatic review target selection policy.
    #[serde(default)]
    pub review_target_policy: common::ReviewTargetPolicy,
}

/// Extension adapter that communicates with an extension executable.
///
/// The adapter caches static extension metadata in a YAML file and invokes the
/// process for each dependency or registry metadata query.
#[derive(Debug, Clone)]
pub struct ProcessExtension {
    process_path_: std::path::PathBuf,
    name_: String,
    registry_host_names_: Vec<String>,
    review_target_policy_: common::ReviewTargetPolicy,
}

impl common::FromProcess for ProcessExtension {
    fn from_process(
        process_path: &std::path::PathBuf,
        extension_config_path: &std::path::PathBuf,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let static_data: StaticData = if extension_config_path.is_file() {
            let file = std::fs::File::open(extension_config_path)?;
            let reader = std::io::BufReader::new(file);
            serde_yaml::from_reader(reader)?
        } else {
            let static_data: Box<StaticData> = run_process(process_path, &["static-data"])?;
            let static_data = *static_data;

            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(extension_config_path)
                .context(format!(
                    "Can't open/create file for writing: {}",
                    extension_config_path.display()
                ))?;
            let writer = std::io::BufWriter::new(file);
            serde_yaml::to_writer(writer, &static_data)?;
            static_data
        };

        Ok(ProcessExtension {
            process_path_: process_path.clone(),
            name_: static_data.name,
            registry_host_names_: static_data.registry_host_names,
            review_target_policy_: static_data.review_target_policy,
        })
    }
}

impl common::Extension for ProcessExtension {
    fn name(&self) -> String {
        self.name_.clone()
    }

    fn registries(&self) -> Vec<String> {
        self.registry_host_names_.clone()
    }

    fn review_target_policy(&self) -> common::ReviewTargetPolicy {
        self.review_target_policy_.clone()
    }

    /// Returns a list of dependencies for the given package.
    ///
    /// Returns one package dependencies structure per registry.
    fn identify_package_dependencies(
        &self,
        package_name: &str,
        package_version: &Option<&str>,
        extension_args: &Vec<String>,
    ) -> Result<Vec<common::PackageDependencies>> {
        let mut args = vec![
            super::commands::identify_package_dependencies::COMMAND_NAME,
            "--package-name",
            package_name,
        ];
        if let Some(package_version) = package_version {
            args.push("--package-version");
            args.push(package_version);
        }
        for extension_arg in extension_args {
            args.push("--extension-args");
            args.push(extension_arg);
        }
        let output: Box<Vec<common::PackageDependencies>> =
            run_process(&self.process_path_, &args)?;
        Ok(*output)
    }

    /// Returns a list of local package dependencies specification files.
    fn identify_file_defined_dependencies(
        &self,
        working_directory: &std::path::PathBuf,
        extension_args: &Vec<String>,
    ) -> Result<Vec<common::FileDefinedDependencies>> {
        let working_directory = working_directory.to_str().ok_or(format_err!(
            "Failed to parse path into string: {}",
            working_directory.display()
        ))?;
        let mut args = vec![
            super::commands::identify_file_defined_dependencies::COMMAND_NAME,
            "--working-directory",
            working_directory,
        ];
        for extension_arg in extension_args {
            args.push("--extension-args");
            args.push(extension_arg);
        }
        let output: Box<Vec<common::FileDefinedDependencies>> =
            run_process(&self.process_path_, &args)?;
        Ok(*output)
    }

    /// Given a package name and version, queries the remote registry for package metadata.
    fn registries_package_metadata(
        &self,
        package_name: &str,
        package_version: &Option<&str>,
    ) -> Result<Vec<common::RegistryPackageMetadata>> {
        let mut args = vec![
            super::commands::registries_package_metadata::COMMAND_NAME,
            package_name,
        ];
        if let Some(package_version) = package_version {
            args.push(*package_version);
        }

        let output: Box<Vec<common::RegistryPackageMetadata>> =
            run_process(&self.process_path_, &args)?;
        Ok(*output)
    }
}

/// JSON envelope used for process extension command responses.
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProcessResult<T> {
    /// Successful command result.
    pub ok: Option<T>,
    /// Command error message.
    pub err: Option<String>,
}

fn run_process<T>(process_path: &std::path::Path, args: &[&str]) -> Result<Box<T>>
where
    for<'de> T: serde::Deserialize<'de>,
{
    log::debug!(
        "Executing extensions process call with arguments\n{:?}",
        args
    );
    let process = process_path.to_str().ok_or(format_err!(
        "Failed to parse string from process path: {}",
        process_path.display()
    ))?;
    let handle = std::process::Command::new(process)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&handle.stdout);
    let process_result: ProcessResult<T> = serde_json::from_str(&stdout)?;

    if let Some(result) = process_result.ok {
        Ok(Box::new(result))
    } else if let Some(result) = process_result.err {
        Err(format_err!(result))
    } else {
        Err(format_err!("Failed to find ok or err result from process."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::common::{Extension, FromProcess, ReviewTargetPolicy};
    use serde_json::json;

    #[test]
    fn static_data_serializes_review_target_policy() {
        let data = StaticData {
            name: "rs".to_string(),
            registry_host_names: vec!["crates.io".to_string()],
            review_target_policy: ReviewTargetPolicy {
                excluded_exact_paths: vec!["Cargo.lock".to_string()],
            },
        };

        let serialized = serde_json::to_value(&data).expect("failed to serialize static data");

        assert_eq!(
            serialized,
            json!({
                "name": "rs",
                "registry_host_names": ["crates.io"],
                "review_target_policy": {
                    "excluded_exact_paths": ["Cargo.lock"]
                }
            })
        );
    }

    #[test]
    fn static_data_defaults_empty_review_target_policy() {
        let data: StaticData = serde_json::from_value(json!({
            "name": "rs",
            "registry_host_names": ["crates.io"]
        }))
        .expect("failed to deserialize static data");

        assert_eq!(data.review_target_policy, ReviewTargetPolicy::default());
    }

    #[test]
    fn process_extension_loads_review_target_policy_from_static_data() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let config_path = tmp.path().join("thirdpass-rs.yaml");
        let process_path = tmp.path().join("thirdpass-rs");
        let data = StaticData {
            name: "rs".to_string(),
            registry_host_names: vec!["crates.io".to_string()],
            review_target_policy: ReviewTargetPolicy {
                excluded_exact_paths: vec!["Cargo.lock".to_string()],
            },
        };
        std::fs::write(
            &config_path,
            serde_yaml::to_string(&data).expect("failed to serialize static data"),
        )
        .expect("failed to write static data");

        let extension = ProcessExtension::from_process(&process_path, &config_path)
            .expect("failed to load extension");

        assert_eq!(extension.name(), "rs");
        assert_eq!(extension.registries(), vec!["crates.io"]);
        assert_eq!(
            extension.review_target_policy(),
            ReviewTargetPolicy {
                excluded_exact_paths: vec!["Cargo.lock".to_string()],
            }
        );
    }
}
