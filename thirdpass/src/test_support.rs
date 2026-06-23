use anyhow::Result;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use crate::{common, package, peer, registry, review};

/// Environment variable override that restores previous values on drop.
pub(crate) struct ScopedEnv {
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

    /// Set one environment variable and restore its previous value on drop.
    pub(crate) fn set_var(name: &'static str, value: impl Into<OsString>) -> Self {
        let previous = vec![(name, std::env::var_os(name))];
        std::env::set_var(name, value.into());
        Self { previous }
    }

    /// Remove one environment variable and restore its previous value on drop.
    pub(crate) fn remove_var(name: &'static str) -> Self {
        let previous = vec![(name, std::env::var_os(name))];
        std::env::remove_var(name);
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

/// Shared fixture for command tests that need one dependency package.
pub(crate) struct DependencyReviewFixture {
    root: tempfile::TempDir,
    project: PathBuf,
    dependency_file: PathBuf,
    registry_host_name: String,
    package_name: String,
    package_version: String,
    package_hash: String,
    files: Vec<FixturePackageFile>,
}

impl DependencyReviewFixture {
    /// Create a project with one dependency file and one package.
    pub(crate) fn new(prefix: &str) -> Result<Self> {
        let root = tempfile::Builder::new().prefix(prefix).tempdir()?;
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
            package_hash: "fixture-package-hash".to_string(),
            files: vec![
                FixturePackageFile {
                    path: PathBuf::from("README.md"),
                    contents: b"# Fixture\n".to_vec(),
                },
                FixturePackageFile {
                    path: PathBuf::from("src/lib.rs"),
                    contents: b"pub fn fixture() {}\n".to_vec(),
                },
            ],
        })
    }

    /// Enter isolated client storage rooted under this fixture tempdir.
    pub(crate) fn enter_client_environment(&self) -> ScopedEnv {
        let client_root = self.root.path().join("client");
        ScopedEnv::set(&[
            ("HOME", client_root.join("home")),
            ("XDG_CONFIG_HOME", client_root.join("xdg-config")),
            ("XDG_DATA_HOME", client_root.join("xdg-data")),
        ])
    }

    /// Project root containing dependency files and committed reviews.
    pub(crate) fn project_root(&self) -> &Path {
        &self.project
    }

    /// Dependency file path for this fixture project.
    pub(crate) fn dependency_file(&self) -> &Path {
        &self.dependency_file
    }

    /// Registry host used by the fixture extension.
    pub(crate) fn registry_host_name(&self) -> &str {
        &self.registry_host_name
    }

    /// Package name used by the fixture dependency.
    pub(crate) fn package_name(&self) -> &str {
        &self.package_name
    }

    /// Package version used by the fixture dependency.
    pub(crate) fn package_version(&self) -> &str {
        &self.package_version
    }

    /// Dependency discovery result for this fixture project.
    pub(crate) fn file_defined_dependencies(
        &self,
    ) -> thirdpass_core::extension::FileDefinedDependencies {
        thirdpass_core::extension::FileDefinedDependencies {
            path: self.dependency_file.clone(),
            registry_host_name: self.registry_host_name.clone(),
            dependencies: vec![thirdpass_core::extension::Dependency {
                name: self.package_name.clone(),
                version: Ok(self.package_version.clone()),
            }],
        }
    }

    /// Write the cached workspace manifest used by dependency package analysis.
    pub(crate) fn prepare_cached_workspace(&self) -> Result<()> {
        let data_paths = common::fs::DataPaths::new()?;
        let package_path = thirdpass_core::package::unique_package_path(
            &self.package_name,
            &self.package_version,
            &self.registry_host_name,
        )?;
        let package_directory = data_paths.ongoing_reviews_directory.join(package_path);
        let workspace_path =
            package_directory.join(format!("{}-{}", self.package_name, self.package_version));
        std::fs::create_dir_all(&workspace_path)?;
        for file in &self.files {
            let path = workspace_path.join(&file.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &file.contents)?;
        }

        let archive_path = package_directory.join("archive.tar.gz");
        std::fs::write(&archive_path, b"stand-in archive bytes")?;
        let manifest = thirdpass_core::package::Manifest {
            workspace_path,
            manifest_path: package_directory.join("manifest.json"),
            artifact_path: archive_path,
            package_hash: self.package_hash.clone(),
        };
        let mut manifest_file = std::fs::File::create(&manifest.manifest_path)?;
        manifest_file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        Ok(())
    }

    /// Commit a matching project review for all fixture package files.
    pub(crate) fn write_project_review(&self) -> Result<()> {
        self.write_project_review_with_package_hash(&self.package_hash)
    }

    /// Commit a project review with an explicit package hash.
    pub(crate) fn write_project_review_with_package_hash(&self, package_hash: &str) -> Result<()> {
        let all_paths = self.all_file_paths();
        review::project::store_dependency_review(
            self.project_root(),
            &self.review(package_hash.to_string(), "committed-reviewer", &all_paths)?,
        )?;
        Ok(())
    }

    /// Store a matching review in the client-wide review store.
    pub(crate) fn write_global_review(&self, public_user_id: &str) -> Result<std::path::PathBuf> {
        let all_paths = self.all_file_paths();
        review::store_submitted(&self.review_for_files(public_user_id, &all_paths)?)
    }

    /// Store a matching review for specific fixture files in the client-wide review store.
    pub(crate) fn write_global_review_for_files(
        &self,
        public_user_id: &str,
        paths: &[&str],
    ) -> Result<std::path::PathBuf> {
        review::store_submitted(&self.review_for_files(public_user_id, paths)?)
    }

    /// Build a matching review for specific fixture package files.
    pub(crate) fn review_for_files(
        &self,
        public_user_id: &str,
        paths: &[&str],
    ) -> Result<review::Review> {
        self.review(self.package_hash.clone(), public_user_id, paths)
    }

    fn all_file_paths(&self) -> Vec<&str> {
        self.files
            .iter()
            .map(|file| {
                file.path
                    .to_str()
                    .expect("fixture paths should be UTF-8 strings")
            })
            .collect()
    }

    fn review(
        &self,
        package_hash: String,
        public_user_id: &str,
        paths: &[&str],
    ) -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: self.registry_host_name.clone(),
            human_url: url::Url::parse("https://fixture.registry/fixture-package")?,
            artifact_url: url::Url::parse("https://fixture.registry/fixture-package-1.0.0.tar.gz")?,
        });

        let targets = self
            .files
            .iter()
            .filter(|file| {
                paths
                    .iter()
                    .any(|path| file.path == std::path::Path::new(path))
            })
            .map(|file| {
                Ok(review::ReviewTarget {
                    file_path: file.path.clone(),
                    file_hash: Some(file.hash()),
                    agent_summary: None,
                    security_summary: Some(review::SecuritySummary::None),
                    confidence: None,
                    comments: std::collections::BTreeSet::new(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: self.package_name.clone(),
                version: self.package_version.clone(),
                registries,
                package_hash,
            },
            targets,
            reviewer_details: review::ReviewerDetails {
                public_user_id: public_user_id.to_string(),
                ..review::ReviewerDetails::default()
            },
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::None,
            overall_security_confidence: None,
        })
    }
}

struct FixturePackageFile {
    path: PathBuf,
    contents: Vec<u8>,
}

impl FixturePackageFile {
    fn hash(&self) -> thirdpass_core::schema::FileHash {
        let digest = blake3::hash(&self.contents).to_hex().as_str().to_string();
        thirdpass_core::schema::FileHash::blake3(digest)
    }
}

/// Fake extension that serves the shared dependency fixture.
pub(crate) struct FixtureExtension {
    registry_host_name: String,
    package_name: String,
    package_version: String,
}

impl FixtureExtension {
    /// Create an extension bound to the supplied dependency fixture.
    pub(crate) fn new(fixture: &DependencyReviewFixture) -> Self {
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

/// Minimal review API server that returns no remote reviews.
pub(crate) struct EmptyReviewServer {
    /// Base URL accepted by the client API config.
    pub(crate) api_base: String,
    handle: Option<JoinHandle<std::io::Result<()>>>,
}

impl EmptyReviewServer {
    /// Start an HTTP server that responds to one request with an empty list.
    pub(crate) fn start() -> Result<Self> {
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

    /// Wait for the server request handler to finish.
    pub(crate) fn join(mut self) -> Result<()> {
        let handle = self.handle.take().expect("server handle should exist");
        match handle.join() {
            Ok(result) => Ok(result?),
            Err(_) => anyhow::bail!("empty review server panicked"),
        }
    }
}
