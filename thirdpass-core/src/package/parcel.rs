use anyhow::{format_err, Result};
use rand::{seq::SliceRandom, SeedableRng};

/// Default maximum total line count for one review parcel.
pub const DEFAULT_REVIEW_PARCEL_MAX_LINES: usize = 1_200;

/// Default maximum number of files for one review parcel.
pub const DEFAULT_REVIEW_PARCEL_MAX_FILES: usize = 5;

/// Controls how package files are grouped into review parcels.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewParcelConfig {
    /// Maximum total line count to include in one parcel.
    pub max_lines: usize,
    /// Maximum number of files to include in one parcel.
    pub max_files: usize,
    /// Optional seed used to make file shuffling reproducible.
    pub shuffle_seed: Option<u64>,
}

impl Default for ReviewParcelConfig {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_REVIEW_PARCEL_MAX_LINES,
            max_files: DEFAULT_REVIEW_PARCEL_MAX_FILES,
            shuffle_seed: None,
        }
    }
}

impl ReviewParcelConfig {
    fn validate(&self) -> Result<()> {
        if self.max_lines == 0 {
            return Err(format_err!(
                "max lines per parcel must be greater than zero"
            ));
        }
        if self.max_files == 0 {
            return Err(format_err!(
                "max files per parcel must be greater than zero"
            ));
        }
        Ok(())
    }
}

/// Package identity carried by generated review parcels.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewParcelPackage {
    /// Registry host name such as `crates.io` or `npmjs.com`.
    pub registry_host: String,
    /// Package name in the registry.
    pub package_name: String,
    /// Package version in the registry.
    pub package_version: String,
    /// Hash identifying the analyzed package artifact.
    pub package_hash: String,
}

/// One package file that can be considered for review parceling.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewableFile {
    /// Package-relative file path.
    pub path: String,
    /// Hash of the file contents.
    pub file_hash: crate::schema::FileHash,
    /// File size in bytes.
    pub size_bytes: u64,
    /// File extension without the leading dot, when known.
    pub extension: Option<String>,
    /// Line count for text files; files without a line count are not parceled.
    pub line_count: Option<usize>,
}

/// Input package and files for review parcel generation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewParcelInput {
    /// Package identity copied into each generated parcel.
    pub package: ReviewParcelPackage,
    /// Package files to filter, shuffle, and group.
    pub files: Vec<ReviewableFile>,
    /// Registry-specific automatic review target policy.
    pub target_policy: crate::extension::ReviewTargetPolicy,
}

/// One file included in a generated review parcel.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewParcelFile {
    /// Package-relative file path.
    pub path: String,
    /// Hash of the file contents.
    pub file_hash: crate::schema::FileHash,
    /// File size in bytes.
    pub size_bytes: u64,
    /// File extension without the leading dot, when known.
    pub extension: Option<String>,
    /// Line count used for parcel sizing.
    pub line_count: usize,
    /// Stable rank among reviewable files before shuffling.
    pub file_rank: usize,
}

/// A bounded group of package files to review together.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewParcel {
    /// Package identity for this parcel.
    pub package: ReviewParcelPackage,
    /// One-based parcel rank within the package.
    pub package_parcel_rank: usize,
    /// Total lines across files in this parcel.
    pub total_lines: usize,
    /// Files included in this parcel.
    pub files: Vec<ReviewParcelFile>,
}

/// Build bounded review parcels for one package.
///
/// The builder filters files using the supplied target policy, ignores files
/// without line counts, assigns file ranks by package-relative path, shuffles
/// the reviewable files, and groups them by configured line and file limits.
pub fn build_review_parcels(
    input: ReviewParcelInput,
    config: ReviewParcelConfig,
) -> Result<Vec<ReviewParcel>> {
    config.validate()?;

    let mut files = reviewable_files(input.files, &input.target_policy);
    shuffle_reviewable_files(&mut files, config.shuffle_seed);
    Ok(group_reviewable_files(input.package, files, &config))
}

fn reviewable_files(
    mut files: Vec<ReviewableFile>,
    target_policy: &crate::extension::ReviewTargetPolicy,
) -> Vec<ReviewParcelFile> {
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut reviewable_files = Vec::new();
    for file in files {
        if target_policy.excludes_exact_path(&file.path) {
            continue;
        }
        let Some(line_count) = file.line_count else {
            continue;
        };
        reviewable_files.push(ReviewParcelFile {
            path: file.path,
            file_hash: file.file_hash,
            size_bytes: file.size_bytes,
            extension: file.extension,
            line_count,
            file_rank: reviewable_files.len() + 1,
        });
    }
    reviewable_files
}

fn shuffle_reviewable_files(files: &mut [ReviewParcelFile], shuffle_seed: Option<u64>) {
    match shuffle_seed {
        Some(seed) => {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            files.shuffle(&mut rng);
        }
        None => {
            let mut rng = rand::thread_rng();
            files.shuffle(&mut rng);
        }
    }
}

fn group_reviewable_files(
    package: ReviewParcelPackage,
    files: Vec<ReviewParcelFile>,
    config: &ReviewParcelConfig,
) -> Vec<ReviewParcel> {
    let mut parcels = Vec::new();
    let mut current_files = Vec::new();
    let mut current_lines = 0usize;

    for file in files {
        if !current_files.is_empty()
            && (current_files.len() >= config.max_files
                || current_lines + file.line_count > config.max_lines)
        {
            parcels.push(review_parcel_for_files(
                &package,
                parcels.len() + 1,
                &current_files,
                current_lines,
            ));
            current_files.clear();
            current_lines = 0;
        }

        current_lines += file.line_count;
        current_files.push(file);

        if current_files.len() >= config.max_files || current_lines >= config.max_lines {
            parcels.push(review_parcel_for_files(
                &package,
                parcels.len() + 1,
                &current_files,
                current_lines,
            ));
            current_files.clear();
            current_lines = 0;
        }
    }

    if !current_files.is_empty() {
        parcels.push(review_parcel_for_files(
            &package,
            parcels.len() + 1,
            &current_files,
            current_lines,
        ));
    }

    parcels
}

fn review_parcel_for_files(
    package: &ReviewParcelPackage,
    package_parcel_rank: usize,
    files: &[ReviewParcelFile],
    total_lines: usize,
) -> ReviewParcel {
    ReviewParcel {
        package: package.clone(),
        package_parcel_rank,
        total_lines,
        files: files.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_parcels_filter_shuffle_and_group_files() -> Result<()> {
        let input = ReviewParcelInput {
            package: package(),
            files: vec![
                file("src/c.rs", Some(500), "hash-c"),
                file("static/logo.bin", None, "hash-logo"),
                file("Cargo.lock", Some(100), "hash-lock"),
                file("src/a.rs", Some(100), "hash-a"),
                file("src/b.rs", Some(100), "hash-b"),
            ],
            target_policy: crate::extension::ReviewTargetPolicy {
                excluded_exact_paths: vec!["Cargo.lock".to_string()],
            },
        };

        let parcels = build_review_parcels(
            input,
            ReviewParcelConfig {
                max_lines: 1_000,
                max_files: 2,
                shuffle_seed: Some(7),
            },
        )?;

        assert_eq!(parcels.len(), 2);
        assert_eq!(
            parcels
                .iter()
                .map(|parcel| parcel.total_lines)
                .sum::<usize>(),
            700
        );
        assert_eq!(
            parcels
                .iter()
                .map(|parcel| parcel.files.len())
                .sum::<usize>(),
            3
        );
        let mut files = parcels
            .iter()
            .flat_map(|parcel| parcel.files.iter())
            .map(|file| (file.path.as_str(), file.line_count, file.file_rank))
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("src/a.rs", 100, 1),
                ("src/b.rs", 100, 2),
                ("src/c.rs", 500, 3),
            ]
        );
        Ok(())
    }

    #[test]
    fn seeded_shuffle_is_repeatable() -> Result<()> {
        let config = ReviewParcelConfig {
            max_lines: 1_000,
            max_files: 10,
            shuffle_seed: Some(42),
        };
        let first = build_review_parcels(input_with_numbered_files(10), config.clone())?;
        let second = build_review_parcels(input_with_numbered_files(10), config)?;

        assert_eq!(first, second);
        assert_ne!(
            first[0]
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>(),
            (0..10)
                .map(|index| format!("src/file-{index}.rs"))
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn oversized_file_becomes_its_own_parcel() -> Result<()> {
        let parcels = build_review_parcels(
            ReviewParcelInput {
                package: package(),
                files: vec![file("src/large.rs", Some(2_000), "hash-large")],
                target_policy: crate::extension::ReviewTargetPolicy::default(),
            },
            ReviewParcelConfig {
                max_lines: 1_000,
                max_files: 5,
                shuffle_seed: Some(1),
            },
        )?;

        assert_eq!(parcels.len(), 1);
        assert_eq!(parcels[0].total_lines, 2_000);
        assert_eq!(parcels[0].files[0].path, "src/large.rs");
        Ok(())
    }

    #[test]
    fn zero_limits_are_rejected() {
        let err = build_review_parcels(
            input_with_numbered_files(1),
            ReviewParcelConfig {
                max_lines: 0,
                max_files: 1,
                shuffle_seed: Some(1),
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("max lines per parcel"));
    }

    fn input_with_numbered_files(file_count: usize) -> ReviewParcelInput {
        ReviewParcelInput {
            package: package(),
            files: (0..file_count)
                .map(|index| file(&format!("src/file-{index}.rs"), Some(10), "hash"))
                .collect(),
            target_policy: crate::extension::ReviewTargetPolicy::default(),
        }
    }

    fn package() -> ReviewParcelPackage {
        ReviewParcelPackage {
            registry_host: "crates.io".to_string(),
            package_name: "serde".to_string(),
            package_version: "1.0.0".to_string(),
            package_hash: "package-hash".to_string(),
        }
    }

    fn file(path: &str, line_count: Option<usize>, hash: &str) -> ReviewableFile {
        ReviewableFile {
            path: path.to_string(),
            file_hash: crate::schema::FileHash::blake3(hash),
            size_bytes: 1024,
            extension: path
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_string()),
            line_count,
        }
    }
}
