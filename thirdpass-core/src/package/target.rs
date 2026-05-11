use anyhow::{format_err, Result};

use crate::package::analysis::{self, Analysis, PathType};

/// A concrete file selected for review.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SelectedTarget {
    /// Absolute path to the selected file on local disk.
    pub absolute_path: std::path::PathBuf,
    /// Path to the selected file relative to the package workspace.
    pub relative_path: std::path::PathBuf,
    /// Blake3 file hash for the selected file.
    pub file_hash: crate::schema::FileHash,
}

/// A package file that may be useful to review.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CandidateFile {
    /// Path to the candidate file relative to the package workspace.
    pub relative_path: std::path::PathBuf,
    /// Line count reported by package analysis.
    pub line_count: usize,
    /// Whether this exact file has already been reviewed locally.
    pub already_reviewed: bool,
}

/// Resolve a user-provided file path into a selected package target.
pub fn resolve_target_path(
    workspace_path: &std::path::Path,
    target_file: &str,
) -> Result<SelectedTarget> {
    let target_path = std::path::PathBuf::from(target_file);
    let target_path = if target_path.is_absolute() {
        target_path
    } else {
        workspace_path.join(target_path)
    };
    if !target_path.is_file() {
        return Err(format_err!(
            "Target file not found: {}",
            target_path.display()
        ));
    }
    let target_relative = target_path
        .strip_prefix(workspace_path)
        .unwrap_or(target_path.as_path())
        .to_path_buf();
    selected_target(target_path, target_relative)
}

/// Build selected package targets from user-provided file paths.
pub fn resolve_target_paths(
    workspace_path: &std::path::Path,
    target_files: &[String],
) -> Result<Vec<SelectedTarget>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut targets = Vec::new();
    for target_file in target_files {
        let target = resolve_target_path(workspace_path, target_file)?;
        if seen.insert(target.relative_path.clone()) {
            targets.push(target);
        }
    }
    Ok(targets)
}

/// Build a selected target from absolute and workspace-relative paths.
pub fn selected_target(
    absolute_path: std::path::PathBuf,
    relative_path: std::path::PathBuf,
) -> Result<SelectedTarget> {
    if !absolute_path.is_file() {
        return Err(format_err!(
            "Target path is not a file: {}",
            absolute_path.display()
        ));
    }
    let hash = analysis::file_blake3_digest(&absolute_path)?;
    Ok(SelectedTarget {
        absolute_path,
        relative_path,
        file_hash: crate::schema::FileHash::blake3(hash),
    })
}

/// Convert workspace analysis into sorted candidate review files.
pub fn candidate_files(
    analysis: &Analysis,
    already_reviewed_paths: &std::collections::BTreeSet<std::path::PathBuf>,
) -> Vec<CandidateFile> {
    let mut candidates = Vec::new();
    for (path, entry) in analysis.iter() {
        if matches!(entry.path_type, PathType::File) {
            candidates.push(CandidateFile {
                relative_path: path.clone(),
                line_count: entry.line_count,
                already_reviewed: already_reviewed_paths.contains(path),
            });
        }
    }
    sort_candidates(&mut candidates);
    candidates
}

/// Sort candidates by review usefulness.
pub fn sort_candidates(candidates: &mut [CandidateFile]) {
    candidates.sort_by(|a, b| {
        a.already_reviewed
            .cmp(&b.already_reviewed)
            .then_with(|| b.line_count.cmp(&a.line_count))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
}

/// Return true when every candidate has already been reviewed.
pub fn all_candidates_reviewed(candidates: &[CandidateFile]) -> bool {
    !candidates.is_empty()
        && candidates
            .iter()
            .all(|candidate| candidate.already_reviewed)
}

/// Select the first locally ranked candidate as a target.
pub fn select_first_candidate(
    workspace_path: &std::path::Path,
    candidates: &[CandidateFile],
) -> Result<SelectedTarget> {
    let candidate = candidates
        .first()
        .ok_or(format_err!("No files found to review."))?;
    let target_relative = candidate.relative_path.clone();
    let target_path = workspace_path.join(&target_relative);
    selected_target(target_path, target_relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_candidates_prefers_unreviewed_files() {
        let mut candidates = vec![
            candidate_file("already-reviewed-large.js", 300, true),
            candidate_file("unreviewed-small.js", 50, false),
            candidate_file("unreviewed-large.js", 200, false),
            candidate_file("already-reviewed-small.js", 20, true),
        ];

        sort_candidates(&mut candidates);

        let paths = candidates
            .iter()
            .map(|candidate| candidate.relative_path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "unreviewed-large.js",
                "unreviewed-small.js",
                "already-reviewed-large.js",
                "already-reviewed-small.js",
            ]
        );
    }

    #[test]
    fn resolve_target_path_records_blake3_file_hash() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let workspace = tmp.path().to_path_buf();
        let contents = b"console.log('review me');\n";
        std::fs::write(workspace.join("index.js"), contents)?;

        let target = resolve_target_path(&workspace, "index.js")?;
        let expected_hash = blake3::hash(contents).to_hex().as_str().to_string();

        assert_eq!(target.relative_path, std::path::PathBuf::from("index.js"));
        assert_eq!(
            target.file_hash,
            crate::schema::FileHash::blake3(expected_hash)
        );
        Ok(())
    }

    fn candidate_file(path: &str, line_count: usize, already_reviewed: bool) -> CandidateFile {
        CandidateFile {
            relative_path: std::path::PathBuf::from(path),
            line_count,
            already_reviewed,
        }
    }
}
