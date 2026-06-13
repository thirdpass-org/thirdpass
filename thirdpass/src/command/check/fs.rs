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
    config: &common::config::Config,
    first_row_separate: bool,
) -> Result<Option<output::DependencyGroup>> {
    log::info!(
        "Generating report for dependencies specification file: {}",
        package_dependencies.path.display()
    );
    let dependencies = &package_dependencies.dependencies;

    let dependency_reports: Result<Vec<report::DependencyReport>> = dependencies
        .iter()
        .map(|dependency| -> Result<report::DependencyReport> {
            let project_review_context = report::ProjectReviewContext {
                extension,
                reviews: project_reviews,
            };
            report::get_dependency_report(
                dependency,
                &package_dependencies.registry_host_name,
                config,
                Some(&project_review_context),
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, peer, registry};
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::thread::JoinHandle;

    #[test]
    fn check_reuses_matching_committed_project_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = CheckFixture::new()?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review("fixture-package-hash", &fixture.readme_hash()?)?;
        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        let server = EmptyReviewServer::start()?;
        let mut config = common::config::Config::default();
        config.core.api_base = server.api_base.clone();

        let group = report_dependencies(
            &fixture.file_defined_dependencies(),
            &FixtureExtension::new(&fixture),
            &project_reviews,
            &config,
            false,
        )?
        .expect("dependency group should be reported");
        server.join()?;

        assert_eq!(group.dependencies.len(), 1);
        assert_eq!(group.dependencies[0].review_count, Some(1));
        assert_eq!(
            group.dependencies[0].note.as_deref(),
            Some("project reviews (1)")
        );
        Ok(())
    }

    #[test]
    fn check_reports_stale_committed_project_reviews() -> Result<()> {
        let _lock = common::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let fixture = CheckFixture::new()?;
        let _env = fixture.enter_client_environment();
        fixture.prepare_cached_workspace()?;
        fixture.write_project_review("stale-package-hash", &fixture.readme_hash()?)?;
        let project_reviews = review::project::list_dependency_reviews(fixture.project_root())?;
        let server = EmptyReviewServer::start()?;
        let mut config = common::config::Config::default();
        config.core.api_base = server.api_base.clone();

        let group = report_dependencies(
            &fixture.file_defined_dependencies(),
            &FixtureExtension::new(&fixture),
            &project_reviews,
            &config,
            false,
        )?
        .expect("dependency group should be reported");
        server.join()?;

        assert_eq!(group.dependencies.len(), 1);
        assert_eq!(group.dependencies[0].review_count, Some(0));
        assert_eq!(
            group.dependencies[0].note.as_deref(),
            Some("project reviews stale")
        );
        Ok(())
    }

    struct CheckFixture {
        root: tempfile::TempDir,
        project: PathBuf,
        dependency_file: PathBuf,
        registry_host_name: String,
        package_name: String,
        package_version: String,
    }

    impl CheckFixture {
        fn new() -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix("thirdpass-check-project-reviews-")
                .tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir_all(&project)?;
            let dependency_file = project.join("deps.lock");
            std::fs::write(&dependency_file, "fixture-package 1.0.0\n")?;

            Ok(Self {
                root,
                project,
                dependency_file,
                registry_host_name: "fixture.registry".to_string(),
                package_name: "fixture-package".to_string(),
                package_version: "1.0.0".to_string(),
            })
        }

        fn enter_client_environment(&self) -> ScopedEnv {
            let client_root = self.root.path().join("client");
            ScopedEnv::set(&[
                ("HOME", client_root.join("home")),
                ("XDG_CONFIG_HOME", client_root.join("xdg-config")),
                ("XDG_DATA_HOME", client_root.join("xdg-data")),
            ])
        }

        fn project_root(&self) -> &Path {
            &self.project
        }

        fn file_defined_dependencies(&self) -> thirdpass_core::extension::FileDefinedDependencies {
            thirdpass_core::extension::FileDefinedDependencies {
                path: self.dependency_file.clone(),
                registry_host_name: self.registry_host_name.clone(),
                dependencies: vec![thirdpass_core::extension::Dependency {
                    name: self.package_name.clone(),
                    version: Ok(self.package_version.clone()),
                }],
            }
        }

        fn prepare_cached_workspace(&self) -> Result<()> {
            let data_paths = common::fs::DataPaths::new()?;
            let package_path = thirdpass_core::package::unique_package_path(
                &self.package_name,
                &self.package_version,
                &self.registry_host_name,
            )?;
            let package_directory = data_paths.ongoing_reviews_directory.join(package_path);
            let workspace_path =
                package_directory.join(format!("{}-{}", self.package_name, self.package_version));
            std::fs::create_dir_all(workspace_path.join("src"))?;
            std::fs::write(workspace_path.join("README.md"), b"# Fixture\n")?;
            std::fs::write(workspace_path.join("src/lib.rs"), b"pub fn fixture() {}\n")?;

            let archive_path = package_directory.join("archive.tar.gz");
            std::fs::write(&archive_path, b"stand-in archive bytes")?;
            let manifest = thirdpass_core::package::Manifest {
                workspace_path,
                manifest_path: package_directory.join("manifest.json"),
                artifact_path: archive_path,
                package_hash: "fixture-package-hash".to_string(),
            };
            let mut manifest_file = std::fs::File::create(&manifest.manifest_path)?;
            manifest_file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
            Ok(())
        }

        fn write_project_review(&self, package_hash: &str, readme_hash: &str) -> Result<()> {
            review::project::store_dependency_review(
                self.project_root(),
                &self.review(package_hash, readme_hash)?,
            )?;
            Ok(())
        }

        fn readme_hash(&self) -> Result<String> {
            thirdpass_core::package::file_blake3_digest(
                &self.cached_workspace_path()?.join("README.md"),
            )
        }

        fn cached_workspace_path(&self) -> Result<PathBuf> {
            let data_paths = common::fs::DataPaths::new()?;
            Ok(data_paths
                .ongoing_reviews_directory
                .join(thirdpass_core::package::unique_package_path(
                    &self.package_name,
                    &self.package_version,
                    &self.registry_host_name,
                )?)
                .join(format!("{}-{}", self.package_name, self.package_version)))
        }

        fn review(&self, package_hash: &str, readme_hash: &str) -> Result<review::Review> {
            let mut registries = std::collections::BTreeSet::new();
            registries.insert(registry::Registry {
                id: 0,
                host_name: self.registry_host_name.clone(),
                human_url: url::Url::parse("https://fixture.registry/fixture-package")?,
                artifact_url: url::Url::parse(
                    "https://fixture.registry/fixture-package-1.0.0.tar.gz",
                )?,
            });

            Ok(review::Review {
                id: 0,
                peer: peer::Peer::default(),
                package: package::Package {
                    id: 0,
                    name: self.package_name.clone(),
                    version: self.package_version.clone(),
                    registries,
                    package_hash: package_hash.to_string(),
                },
                targets: vec![review::ReviewTarget {
                    file_path: PathBuf::from("README.md"),
                    file_hash: Some(thirdpass_core::schema::FileHash::blake3(readme_hash)),
                    agent_summary: None,
                    security_summary: Some(review::SecuritySummary::None),
                    confidence: None,
                    comments: std::collections::BTreeSet::new(),
                }],
                reviewer_details: review::ReviewerDetails {
                    public_user_id: "committed-reviewer".to_string(),
                    ..review::ReviewerDetails::default()
                },
                agent_summary: String::new(),
                overall_security_summary: review::SecuritySummary::None,
                overall_security_confidence: None,
            })
        }
    }

    struct FixtureExtension {
        registry_host_name: String,
        package_name: String,
        package_version: String,
    }

    impl FixtureExtension {
        fn new(fixture: &CheckFixture) -> Self {
            Self {
                registry_host_name: fixture.registry_host_name.clone(),
                package_name: fixture.package_name.clone(),
                package_version: fixture.package_version.clone(),
            }
        }
    }

    impl thirdpass_core::extension::Extension for FixtureExtension {
        fn name(&self) -> String {
            "fixture".to_string()
        }

        fn registries(&self) -> Vec<String> {
            vec![self.registry_host_name.clone()]
        }

        fn identify_package_dependencies(
            &self,
            _package_name: &str,
            _package_version: &Option<&str>,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::PackageDependencies>> {
            Ok(Vec::new())
        }

        fn identify_file_defined_dependencies(
            &self,
            _working_directory: &Path,
            _extension_args: &[String],
        ) -> Result<Vec<thirdpass_core::extension::FileDefinedDependencies>> {
            Ok(Vec::new())
        }

        fn registries_package_metadata(
            &self,
            package_name: &str,
            package_version: &Option<&str>,
        ) -> Result<Vec<thirdpass_core::extension::RegistryPackageMetadata>> {
            assert_eq!(package_name, self.package_name);
            assert_eq!(*package_version, Some(self.package_version.as_str()));
            Ok(vec![thirdpass_core::extension::RegistryPackageMetadata {
                registry_host_name: self.registry_host_name.clone(),
                human_url: "https://fixture.registry/fixture-package".to_string(),
                artifact_url: "https://fixture.registry/fixture-package-1.0.0.tar.gz".to_string(),
                is_primary: true,
                package_version: self.package_version.clone(),
            }])
        }
    }

    struct EmptyReviewServer {
        api_base: String,
        handle: Option<JoinHandle<std::io::Result<()>>>,
    }

    impl EmptyReviewServer {
        fn start() -> Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let api_base = format!("http://{}", listener.local_addr()?);
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept()?;
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer)?;
                stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
                )?;
                stream.flush()?;
                Ok(())
            });

            Ok(Self {
                api_base,
                handle: Some(handle),
            })
        }

        fn join(mut self) -> Result<()> {
            let handle = self.handle.take().expect("server handle should exist");
            match handle.join() {
                Ok(result) => Ok(result?),
                Err(_) => anyhow::bail!("empty review server panicked"),
            }
        }
    }

    struct ScopedEnv {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl ScopedEnv {
        fn set(values: &[(&'static str, PathBuf)]) -> Self {
            let previous = values
                .iter()
                .map(|(name, _)| (*name, std::env::var_os(name)))
                .collect::<Vec<_>>();
            for (name, value) in values {
                std::env::set_var(name, value);
            }
            Self { previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (name, value) in self.previous.iter().rev() {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
