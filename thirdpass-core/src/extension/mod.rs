//! Extension interfaces for package registry integrations.
//!
//! Extensions implement [`crate::extension::Extension`] to report supported
//! registries, discover dependency metadata, and return package artifact
//! metadata. Extensions can be compiled into a client with
//! [`crate::extension::FromLib`] or invoked through a process with
//! [`crate::extension::ProcessExtension`].
//!
//! Process-backed extension binaries should call
//! [`crate::extension::run_command`] from their `main` function to expose the
//! standard Thirdpass extension subcommands.

#[doc(hidden)]
pub mod commands;
#[doc(hidden)]
pub mod common;
mod process;

pub use commands::run as run_command;
pub use common::{
    DependenciesCollection, Dependency, Extension, FileDefinedDependencies, FromLib, FromProcess,
    PackageDependencies, RegistryPackageMetadata, ReviewTargetPolicy, VersionError,
    VersionParseResult,
};
pub use process::ProcessExtension;
