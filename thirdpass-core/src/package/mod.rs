//! Package archive preparation, analysis, and review target selection.

mod analysis;
mod archive;
mod manifest;
mod target;
mod workspace;

pub use analysis::{analyse, file_blake3_digest, Analysis, PathAnalysis, PathType};
pub use archive::{download, extract, ArchiveType};
pub use manifest::package_manifest;
pub use target::{
    all_candidates_reviewed, candidate_files, candidate_files_with_policy, resolve_target_path,
    resolve_target_paths, select_first_candidate, selected_target, sort_candidates, CandidateFile,
    SelectedTarget,
};
pub use workspace::{ensure, get_existing, remove, unique_package_path, Manifest, WorkspacePaths};
