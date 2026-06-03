//! Package archive preparation, analysis, and review target selection.
//!
//! The package API covers the local workflow used by the CLI and coordinator:
//! download or reuse a package artifact with [`crate::package::ensure`],
//! inspect the extracted workspace with [`crate::package::analyse`] or
//! [`crate::package::package_manifest`], choose review targets with helpers
//! such as [`crate::package::candidate_files_with_policy`], and clean up
//! extracted workspaces with [`crate::package::remove`].
//!
//! The module re-exports package helpers from a flat namespace so callers do
//! not need to depend on the crate's internal module layout.

mod analysis;
mod archive;
mod batch;
mod manifest;
mod target;
mod workspace;

pub use analysis::{analyse, file_blake3_digest, Analysis, PathAnalysis, PathType};
pub use archive::{download, extract, ArchiveType};
pub use batch::{
    build_review_batches, ReviewBatch, ReviewBatchConfig, ReviewBatchFile, ReviewBatchInput,
    ReviewBatchPackage, ReviewableFile, DEFAULT_REVIEW_BATCH_MAX_FILES,
    DEFAULT_REVIEW_BATCH_MAX_LINES,
};
pub use manifest::package_manifest;
pub use target::{
    all_candidates_reviewed, candidate_files, candidate_files_with_policy, resolve_target_path,
    resolve_target_paths, select_first_candidate, selected_target, sort_candidates, CandidateFile,
    SelectedTarget,
};
pub use workspace::{ensure, get_existing, remove, unique_package_path, Manifest, WorkspacePaths};
