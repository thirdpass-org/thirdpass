use anyhow::{format_err, Context, Result};

use super::common;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct StaticData {
    pub name: String,
    pub registry_host_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessExtension {
    process_path_: std::path::PathBuf,
    name_: String,
    registry_host_names_: Vec<String>,
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
            let file = std::fs::File::open(&extension_config_path)?;
            let reader = std::io::BufReader::new(file);
            serde_yaml::from_reader(reader)?
        } else {
            let static_data: Box<StaticData> = run_process(&process_path, &vec!["static-data"])?;
            let static_data = *static_data;

            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&extension_config_path)
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

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProcessResult<T> {
    pub ok: Option<T>,
    pub err: Option<String>,
}

fn run_process<'a, T: ?Sized>(process_path: &std::path::PathBuf, args: &Vec<&str>) -> Result<Box<T>>
where
    for<'de> T: serde::Deserialize<'de> + 'a,
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
