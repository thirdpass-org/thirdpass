use anyhow::Result;

use crate::common;
use crate::extension;
use crate::review;

use super::output;
use super::report;
use super::OutputFormat;

pub fn report(
    extension_names: &std::collections::BTreeSet<String>,
    extension_args: &[String],
    config: &common::config::Config,
    output_format: OutputFormat,
) -> Result<()> {
    let extensions = extension::manage::get_enabled(extension_names, config)?;
    let working_directory = std::env::current_dir()?;
    log::debug!("Current working directory: {}", working_directory.display());
    let project_reviews = review::project::list_dependency_reviews(&working_directory)?;
    let mut local_reviews = review::fs::list()?;

    let mut dependencies_found = false;
    let all_dependencies_specs = extension::identify_file_defined_dependencies(
        &extensions,
        extension_args,
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
        for fs_dependencies in extension_all_dependencies.iter() {
            let dependency_group = report_dependencies(
                fs_dependencies,
                extension.as_ref(),
                &project_reviews,
                &mut local_reviews,
                config,
                false,
            )?;
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
    extension: &dyn thirdpass_core::extension::Extension,
    project_reviews: &[review::Review],
    local_reviews: &mut Vec<review::Review>,
    config: &common::config::Config,
    first_row_separate: bool,
) -> Result<Option<output::DependencyGroup>> {
    log::info!(
        "Generating report for dependencies specification file: {}",
        package_dependencies.path.display()
    );
    let dependencies = &package_dependencies.dependencies;

    let mut dependency_reports = Vec::new();
    for dependency in dependencies {
        let mut report_context = report::DependencyReportContext {
            extension,
            project_reviews,
            local_reviews,
        };
        dependency_reports.push(report::get_dependency_report(
            dependency,
            &package_dependencies.registry_host_name,
            config,
            &mut report_context,
        )?);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{DependencyReviewFixture, EmptyReviewServer, FixtureExtension};

    #[test]
    fn check_reuses_matching_committed_project_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-check-project-reviews-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review()?;
        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        let server = EmptyReviewServer::start()?;
        let mut config = common::config::Config::default();
        config.core.api_base = server.api_base.clone();
        let mut local_reviews = review::fs::list()?;

        let group = report_dependencies(
            &fixture.file_defined_dependencies(),
            &FixtureExtension::new(&fixture),
            &project_reviews,
            &mut local_reviews,
            &config,
            false,
        )?
        .expect("dependency group should be reported");
        server.join()?;

        assert_eq!(group.dependencies.len(), 1);
        assert_eq!(group.dependencies[0].review_count, Some(1));
        assert_eq!(
            group.dependencies[0].committed_reviews,
            Some(report::CommittedReviewReport {
                matching_count: 1,
                mismatch_count: 0,
                covered_file_count: 2,
                total_file_count: 2,
            })
        );
        assert_eq!(group.dependencies[0].note, None);
        Ok(())
    }

    #[test]
    fn check_reports_mismatched_committed_project_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = DependencyReviewFixture::new("thirdpass-check-project-reviews-")?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review_with_package_hash("mismatched-package-hash")?;
        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        let server = EmptyReviewServer::start()?;
        let mut config = common::config::Config::default();
        config.core.api_base = server.api_base.clone();
        let mut local_reviews = review::fs::list()?;

        let group = report_dependencies(
            &fixture.file_defined_dependencies(),
            &FixtureExtension::new(&fixture),
            &project_reviews,
            &mut local_reviews,
            &config,
            false,
        )?
        .expect("dependency group should be reported");
        server.join()?;

        assert_eq!(group.dependencies.len(), 1);
        assert_eq!(group.dependencies[0].review_count, Some(0));
        assert_eq!(
            group.dependencies[0].committed_reviews,
            Some(report::CommittedReviewReport {
                matching_count: 0,
                mismatch_count: 1,
                covered_file_count: 0,
                total_file_count: 2,
            })
        );
        assert_eq!(
            group.dependencies[0].note.as_deref(),
            Some("committed review mismatch (1)")
        );
        Ok(())
    }
}
