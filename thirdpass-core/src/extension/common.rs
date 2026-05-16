use anyhow::Result;

/// Error value used when an extension cannot resolve a dependency version.
#[derive(
    Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct VersionError(String);

impl VersionError {
    /// Build an error for dependency metadata that omits a version.
    pub fn from_missing_version() -> Self {
        Self("Missing version number".to_string())
    }

    /// Build an error for a dependency version string that could not be parsed.
    pub fn from_parse_error(raw_version_number: &str) -> Self {
        Self(format!("Version parse error: {}", raw_version_number))
    }

    /// Return the human-readable error message.
    pub fn message(&self) -> String {
        self.0.clone()
    }
}

/// Parsed dependency version or a version parsing error.
pub type VersionParseResult = std::result::Result<String, VersionError>;

/// A dependency as specified within a dependencies definition file.
#[derive(
    Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Dependency {
    /// Dependency package name.
    pub name: String,
    /// Parsed dependency version or a captured parse failure.
    pub version: VersionParseResult,
}

/// Common view over dependency collections returned by extensions.
pub trait DependenciesCollection: Sized {
    /// Registry host that owns the dependencies.
    fn registry_host_name(&self) -> &String;
    /// Dependencies found for the registry or file.
    fn dependencies(&self) -> &Vec<Dependency>;
}

/// Package dependencies found by querying a registry.
#[derive(Clone, Debug, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PackageDependencies {
    /// Package version, included when the extension resolves a missing version.
    pub package_version: VersionParseResult,

    /// Dependencies registry host name.
    pub registry_host_name: String,

    /// Dependencies specified within the dependencies specification file.
    pub dependencies: Vec<Dependency>,
}

impl DependenciesCollection for PackageDependencies {
    fn registry_host_name(&self) -> &String {
        &self.registry_host_name
    }
    fn dependencies(&self) -> &Vec<Dependency> {
        &self.dependencies
    }
}

/// A dependencies specification file found from inspecting the local filesystem.
#[derive(Clone, Debug, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileDefinedDependencies {
    /// Absolute file path for dependencies specification file.
    pub path: std::path::PathBuf,

    /// Dependencies registry host name.
    pub registry_host_name: String,

    /// Dependencies specified within the dependencies specification file.
    pub dependencies: Vec<Dependency>,
}

impl DependenciesCollection for FileDefinedDependencies {
    fn registry_host_name(&self) -> &String {
        &self.registry_host_name
    }
    fn dependencies(&self) -> &Vec<Dependency> {
        &self.dependencies
    }
}

/// Metadata returned by an extension for a package in a registry.
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegistryPackageMetadata {
    /// Registry host name such as `crates.io` or `npmjs.com`.
    pub registry_host_name: String,
    /// Human-readable package URL.
    pub human_url: String,
    /// Download URL for the package source artifact.
    pub artifact_url: String,
    /// True when this metadata describes the primary registry hit.
    pub is_primary: bool,
    /// Package version, included when the extension resolves a missing version.
    pub package_version: String,
}

/// Policy for selecting automatic review target files.
#[derive(Debug, Clone, Default, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewTargetPolicy {
    /// Exact package-relative paths to exclude from automatic target selection.
    pub excluded_exact_paths: Vec<String>,
}

impl ReviewTargetPolicy {
    /// Return true when this policy excludes the exact package-relative path.
    pub fn excludes_exact_path(&self, package_relative_path: &str) -> bool {
        self.excluded_exact_paths
            .iter()
            .any(|excluded_path| excluded_path == package_relative_path)
    }

    /// Return true when this policy excludes the package-relative path.
    pub fn excludes_path(&self, package_relative_path: &std::path::Path) -> bool {
        self.excludes_exact_path(&package_relative_path_string(package_relative_path))
    }
}

fn package_relative_path_string(package_relative_path: &std::path::Path) -> String {
    if package_relative_path.as_os_str().is_empty() {
        return ".".to_string();
    }

    package_relative_path
        .iter()
        .map(|component| component.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Extension implementation that is compiled directly into the current process.
pub trait FromLib: Extension + Send + Sync {
    /// Initialize extension from a library.
    fn new() -> Self
    where
        Self: Sized;
}

/// Extension implementation that is loaded by invoking a process.
pub trait FromProcess: Extension + Send + Sync {
    /// Initialize extension from a process.
    fn from_process(
        process_path: &std::path::Path,
        extension_config_path: &std::path::Path,
    ) -> Result<Self>
    where
        Self: Sized;
}

/// Registry and dependency behavior implemented by every Thirdpass extension.
pub trait Extension: Send + Sync {
    /// Return the extension short name.
    fn name(&self) -> String;

    /// Return registry host names supported by this extension.
    fn registries(&self) -> Vec<String>;

    /// Return automatic review-target selection policy for this extension.
    fn review_target_policy(&self) -> ReviewTargetPolicy {
        ReviewTargetPolicy::default()
    }

    /// Identify specific package dependencies.
    fn identify_package_dependencies(
        &self,
        package_name: &str,
        package_version: &Option<&str>,
        extension_args: &[String],
    ) -> Result<Vec<PackageDependencies>>;

    /// Identify file defined dependencies.
    fn identify_file_defined_dependencies(
        &self,
        working_directory: &std::path::Path,
        extension_args: &[String],
    ) -> Result<Vec<FileDefinedDependencies>>;

    /// Query package registries for package metadata.
    fn registries_package_metadata(
        &self,
        package_name: &str,
        package_version: &Option<&str>,
    ) -> Result<Vec<RegistryPackageMetadata>>;
}
