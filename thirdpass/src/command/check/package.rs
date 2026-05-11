use anyhow::Result;

use crate::common;
use crate::extension;

use super::output;
use super::report;
use super::OutputFormat;

/// Prints a report for a specific package.
pub fn report(
    package_name: &str,
    package_version: &Option<&str>,
    extension_names: &std::collections::BTreeSet<String>,
    extension_args: &Vec<String>,
    config: &common::config::Config,
    output_format: OutputFormat,
) -> Result<()> {
    let extensions = extension::manage::get_enabled(&extension_names, &config)?;

    let mut dependencies_found = false;
    let all_extensions_results = extension::identify_package_dependencies(
        &package_name,
        &package_version,
        &extensions,
        &extension_args,
    )?;

    let mut groups = Vec::new();
    for (extension, extension_all_dependencies) in
        extensions.iter().zip(all_extensions_results.into_iter())
    {
        log::debug!(
            "Inspecting dependencies supported by extension: {}",
            extension.name()
        );

        let extension_all_package_dependencies = match extension_all_dependencies {
            Ok(d) => d,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };

        for (_index, package_dependencies) in extension_all_package_dependencies.iter().enumerate()
        {
            let dependency_group =
                report_dependencies(&package_name, &package_dependencies, &config, true)?;
            dependencies_found = true;
            groups.push(dependency_group);
        }
    }

    if !dependencies_found {
        println!("No dependencies found.")
    } else {
        output::print(&groups, output_format)?;
    }
    Ok(())
}

fn report_dependencies(
    package_name: &str,
    package_dependencies: &thirdpass_core::extension::PackageDependencies,
    config: &common::config::Config,
    first_row_separate: bool,
) -> Result<output::DependencyGroup> {
    log::info!("Generating report for package dependencies.");
    let dependencies = &package_dependencies.dependencies;

    let mut dependency_reports = vec![];
    let target_package_dependency_report = report::get_dependency_report(
        &thirdpass_core::extension::Dependency {
            name: package_name.to_string(),
            version: package_dependencies.package_version.clone(),
        },
        &package_dependencies.registry_host_name,
        config,
    )?;
    dependency_reports.push(target_package_dependency_report);
    for dependency in dependencies {
        let dependency_report = report::get_dependency_report(
            &dependency,
            &package_dependencies.registry_host_name,
            config,
        )?;
        dependency_reports.push(dependency_report);
    }

    log::info!("Number of dependencies found: {}", dependency_reports.len());
    Ok(output::DependencyGroup {
        registry_host_name: package_dependencies.registry_host_name.clone(),
        source_path: None,
        dependencies: dependency_reports,
        first_row_separate,
    })
}
