use anyhow::Result;

use crate::common;
use crate::extension;

use super::report;
use super::table;

pub fn report(
    extension_names: &std::collections::BTreeSet<String>,
    extension_args: &Vec<String>,
    config: &common::config::Config,
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
        for (index, fs_dependencies) in extension_all_dependencies.iter().enumerate() {
            dependencies_found |= !fs_dependencies.dependencies.is_empty();
            report_dependencies(&fs_dependencies, &config)?;
            let is_last = index == extension_all_dependencies.len() - 1;
            if !is_last {
                println!("");
            }
        }
    }

    if !dependencies_found {
        println!(
            "No dependency specification files found in \
            working directory or parent directories."
        )
    }
    Ok(())
}

fn report_dependencies(
    package_dependencies: &vouch_lib::extension::FileDefinedDependencies,
    config: &common::config::Config,
) -> Result<()> {
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
        return Ok(());
    }

    let table = table::get(&dependency_reports, false)?;
    println!(
        "Registry: {name}\n{path}",
        name = package_dependencies.registry_host_name,
        path = package_dependencies.path.display(),
    );
    table.printstd();
    Ok(())
}
