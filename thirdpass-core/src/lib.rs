//! Shared types and helpers for the Thirdpass package review system.
//!
//! `thirdpass-core` is the library boundary between the CLI, registry
//! extensions, the coordinator, and the server-facing review API. Most users
//! should start with the wire types in [`schema`] and the extension contracts in
//! [`extension`].
//!
//! # Feature Flags
//!
//! - `package` enables archive download, extraction, workspace preparation,
//!   manifest generation, file hashing, line-count analysis, and review target
//!   selection and batching helpers in [`package`].
//! - `registry` enables registry lookup helpers in [`registry`].
//! - The default feature set keeps the crate limited to schema and extension
//!   contracts.
//!
//! # Public API Shape
//!
//! The crate intentionally exposes a flat package API such as
//! [`package::ensure`] and [`package::SelectedTarget`] instead of exposing the
//! internal module layout. Extension authors should use [`extension::Extension`]
//! and [`extension::run_command`] when implementing process-backed extensions.

/// Extension traits, process adapters, and extension command helpers.
pub mod extension;
/// Package archive and workspace helpers.
#[cfg(feature = "package")]
pub mod package;
/// Registry metadata lookup helpers.
#[cfg(feature = "registry")]
pub mod registry;
/// Wire-format schema shared by the CLI and server API.
pub mod schema;
