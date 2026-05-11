use anyhow::Result;

use crate::common;
use crate::extension;

use super::output;
use super::report;
use super::OutputFormat;

pub fn report(
    extension_names: &std::collections::BTreeSet<String>,
    extension_args: &Vec<String>,
    config: &common::config::Config,
    output_format: OutputFormat,
) -> Result<()> {
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;
    let working_directory = std::env::current_dir()?;
    log::debug!("Current working directory: {}", working_directory.display());

    let mut dependencies_found = false;
    let all_dependencies_specs = extension::identify_file_defined_dependencies(
        &extensions,
        &extension_args,
        &working_directory,
    )?;
    let mut groups = Vec::new();
    for (extension, extension_all_dependencies) in
        extensions.iter().zip(all_dependencies_specs.into_iter())
    {
        log::info!(
            "Inspecting dependencies supported by extension: {}",
            extension.name()
        );

        let extension_all_dependencies = match extension_all_dependencies {
            Ok(d) => d,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };
        for (_index, fs_dependencies) in extension_all_dependencies.iter().enumerate() {
            let dependency_group = report_dependencies(&fs_dependencies, &config, false)?;
            if let Some(dependency_group) = dependency_group {
                dependencies_found = true;
                groups.push(dependency_group);
            }
        }
    }

    if !dependencies_found {
        println!(
            "No dependency specification files found in \
            working directory or parent directories."
        )
    } else {
        output::print(&groups, output_format)?;
    }
    Ok(())
}

fn report_dependencies(
    package_dependencies: &thirdpass_core::extension::FileDefinedDependencies,
    config: &common::config::Config,
    first_row_separate: bool,
) -> Result<Option<output::DependencyGroup>> {
    log::info!(
        "Generating report for dependencies specification file: {}",
        package_dependencies.path.display()
    );
    let dependencies = &package_dependencies.dependencies;

    let dependency_reports: Result<Vec<report::DependencyReport>> = dependencies
        .into_iter()
        .map(|dependency| -> Result<report::DependencyReport> {
            Ok(report::get_dependency_report(
                &dependency,
                &package_dependencies.registry_host_name,
                config,
            )?)
        })
        .collect();
    let dependency_reports = dependency_reports?;

    log::info!("Number of dependencies found: {}", dependency_reports.len());
    if dependency_reports.is_empty() {
        return Ok(None);
    }

    Ok(Some(output::DependencyGroup {
        registry_host_name: package_dependencies.registry_host_name.clone(),
        source_path: Some(package_dependencies.path.clone()),
        dependencies: dependency_reports,
        first_row_separate,
    }))
}
