use anyhow::{format_err, Result};
use rand::{seq::SliceRandom, SeedableRng};

/// Default maximum total line count for one review batch.
pub const DEFAULT_REVIEW_BATCH_MAX_LINES: usize = 1_200;

/// Default maximum number of files for one review batch.
pub const DEFAULT_REVIEW_BATCH_MAX_FILES: usize = 5;

/// Approximate bytes represented by one review-weight unit for binary files.
pub const BINARY_REVIEW_WEIGHT_BYTES_PER_LINE: u64 = 80;

/// Controls how package files are grouped into review batches.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewBatchConfig {
    /// Maximum total line count to include in one batch.
    pub max_lines: usize,
    /// Maximum number of files to include in one batch.
    pub max_files: usize,
    /// Optional seed used to make file shuffling reproducible.
    pub shuffle_seed: Option<u64>,
}

impl Default for ReviewBatchConfig {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_REVIEW_BATCH_MAX_LINES,
            max_files: DEFAULT_REVIEW_BATCH_MAX_FILES,
            shuffle_seed: None,
        }
    }
}

impl ReviewBatchConfig {
    fn validate(&self) -> Result<()> {
        if self.max_lines == 0 {
            return Err(format_err!("max lines per batch must be greater than zero"));
        }
        if self.max_files == 0 {
            return Err(format_err!("max files per batch must be greater than zero"));
        }
        Ok(())
    }
}

/// Package identity carried by generated review batches.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewBatchPackage {
    /// Registry host name such as `crates.io` or `npmjs.com`.
    pub registry_host: String,
    /// Package name in the registry.
    pub package_name: String,
    /// Package version in the registry.
    pub package_version: String,
    /// Hash identifying the analyzed package artifact.
    pub package_hash: String,
}

/// One package file that can be considered for review batching.
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
    /// Line count for text files; files without a line count are weighted by bytes.
    pub line_count: Option<usize>,
}

/// Input package and files for review batch generation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewBatchInput {
    /// Package identity copied into each generated batch.
    pub package: ReviewBatchPackage,
    /// Package files to filter, shuffle, and group.
    pub files: Vec<ReviewableFile>,
    /// Registry-specific automatic review target policy.
    pub target_policy: crate::extension::ReviewTargetPolicy,
}

/// One file included in a generated review batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewBatchFile {
    /// Package-relative file path.
    pub path: String,
    /// Hash of the file contents.
    pub file_hash: crate::schema::FileHash,
    /// File size in bytes.
    pub size_bytes: u64,
    /// File extension without the leading dot, when known.
    pub extension: Option<String>,
    /// Line count used for batch sizing.
    pub line_count: usize,
    /// Stable rank among reviewable files before shuffling.
    pub file_rank: usize,
}

/// A bounded group of package files to review together.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewBatch {
    /// Package identity for this batch.
    pub package: ReviewBatchPackage,
    /// One-based batch rank within the package.
    pub package_batch_rank: usize,
    /// Total lines across files in this batch.
    pub total_lines: usize,
    /// Files included in this batch.
    pub files: Vec<ReviewBatchFile>,
}

/// Build bounded review batches for one package.
///
/// The builder filters files using the supplied target policy, weights files
/// without line counts by byte size, assigns file ranks by package-relative
/// path, shuffles the reviewable files, and groups them by configured line and
/// file limits.
pub fn build_review_batches(
    input: ReviewBatchInput,
    config: ReviewBatchConfig,
) -> Result<Vec<ReviewBatch>> {
    config.validate()?;

    let mut files = reviewable_files(input.files, &input.target_policy);
    shuffle_reviewable_files(&mut files, config.shuffle_seed);
    Ok(group_reviewable_files(input.package, files, &config))
}

fn reviewable_files(
    mut files: Vec<ReviewableFile>,
    target_policy: &crate::extension::ReviewTargetPolicy,
) -> Vec<ReviewBatchFile> {
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut reviewable_files = Vec::new();
    for file in files {
        if target_policy.excludes_exact_path(&file.path) {
            continue;
        }
        let line_count = file
            .line_count
            .unwrap_or_else(|| byte_review_weight(file.size_bytes));
        reviewable_files.push(ReviewBatchFile {
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

fn byte_review_weight(size_bytes: u64) -> usize {
    let weight = size_bytes.saturating_add(BINARY_REVIEW_WEIGHT_BYTES_PER_LINE - 1)
        / BINARY_REVIEW_WEIGHT_BYTES_PER_LINE;
    if weight > usize::MAX as u64 {
        usize::MAX
    } else {
        weight.max(1) as usize
    }
}

fn shuffle_reviewable_files(files: &mut [ReviewBatchFile], shuffle_seed: Option<u64>) {
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
    package: ReviewBatchPackage,
    files: Vec<ReviewBatchFile>,
    config: &ReviewBatchConfig,
) -> Vec<ReviewBatch> {
    let mut batches = Vec::new();
    let mut current_files = Vec::new();
    let mut current_lines = 0usize;

    for file in files {
        if !current_files.is_empty()
            && (current_files.len() >= config.max_files
                || current_lines + file.line_count > config.max_lines)
        {
            batches.push(review_batch_for_files(
                &package,
                batches.len() + 1,
                &current_files,
                current_lines,
            ));
            current_files.clear();
            current_lines = 0;
        }

        current_lines += file.line_count;
        current_files.push(file);

        if current_files.len() >= config.max_files || current_lines >= config.max_lines {
            batches.push(review_batch_for_files(
                &package,
                batches.len() + 1,
                &current_files,
                current_lines,
            ));
            current_files.clear();
            current_lines = 0;
        }
    }

    if !current_files.is_empty() {
        batches.push(review_batch_for_files(
            &package,
            batches.len() + 1,
            &current_files,
            current_lines,
        ));
    }

    batches
}

fn review_batch_for_files(
    package: &ReviewBatchPackage,
    package_batch_rank: usize,
    files: &[ReviewBatchFile],
    total_lines: usize,
) -> ReviewBatch {
    ReviewBatch {
        package: package.clone(),
        package_batch_rank,
        total_lines,
        files: files.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_batches_filter_shuffle_and_group_files() -> Result<()> {
        let input = ReviewBatchInput {
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

        let batches = build_review_batches(
            input,
            ReviewBatchConfig {
                max_lines: 1_000,
                max_files: 2,
                shuffle_seed: Some(7),
            },
        )?;

        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches.iter().map(|batch| batch.total_lines).sum::<usize>(),
            713
        );
        assert_eq!(
            batches.iter().map(|batch| batch.files.len()).sum::<usize>(),
            4
        );
        let mut files = batches
            .iter()
            .flat_map(|batch| batch.files.iter())
            .map(|file| (file.path.as_str(), file.line_count, file.file_rank))
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("src/a.rs", 100, 1),
                ("src/b.rs", 100, 2),
                ("src/c.rs", 500, 3),
                ("static/logo.bin", 13, 4),
            ]
        );
        Ok(())
    }

    #[test]
    fn files_without_line_counts_are_weighted_by_size() -> Result<()> {
        let batches = build_review_batches(
            ReviewBatchInput {
                package: package(),
                files: vec![
                    file_with_size("empty.bin", None, 0, "hash-empty"),
                    file_with_size("payload.bin", None, 161, "hash-payload"),
                ],
                target_policy: crate::extension::ReviewTargetPolicy::default(),
            },
            ReviewBatchConfig {
                max_lines: 1_000,
                max_files: 5,
                shuffle_seed: Some(1),
            },
        )?;

        let mut files = batches[0]
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.line_count))
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(files, vec![("empty.bin", 1), ("payload.bin", 3)]);
        Ok(())
    }

    #[test]
    fn seeded_shuffle_is_repeatable() -> Result<()> {
        let config = ReviewBatchConfig {
            max_lines: 1_000,
            max_files: 10,
            shuffle_seed: Some(42),
        };
        let first = build_review_batches(input_with_numbered_files(10), config.clone())?;
        let second = build_review_batches(input_with_numbered_files(10), config)?;

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
    fn oversized_file_becomes_its_own_batch() -> Result<()> {
        let batches = build_review_batches(
            ReviewBatchInput {
                package: package(),
                files: vec![file("src/large.rs", Some(2_000), "hash-large")],
                target_policy: crate::extension::ReviewTargetPolicy::default(),
            },
            ReviewBatchConfig {
                max_lines: 1_000,
                max_files: 5,
                shuffle_seed: Some(1),
            },
        )?;

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].total_lines, 2_000);
        assert_eq!(batches[0].files[0].path, "src/large.rs");
        Ok(())
    }

    #[test]
    fn zero_limits_are_rejected() {
        let err = build_review_batches(
            input_with_numbered_files(1),
            ReviewBatchConfig {
                max_lines: 0,
                max_files: 1,
                shuffle_seed: Some(1),
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("max lines per batch"));
    }

    fn input_with_numbered_files(file_count: usize) -> ReviewBatchInput {
        ReviewBatchInput {
            package: package(),
            files: (0..file_count)
                .map(|index| file(&format!("src/file-{index}.rs"), Some(10), "hash"))
                .collect(),
            target_policy: crate::extension::ReviewTargetPolicy::default(),
        }
    }

    fn package() -> ReviewBatchPackage {
        ReviewBatchPackage {
            registry_host: "crates.io".to_string(),
            package_name: "serde".to_string(),
            package_version: "1.0.0".to_string(),
            package_hash: "package-hash".to_string(),
        }
    }

    fn file(path: &str, line_count: Option<usize>, hash: &str) -> ReviewableFile {
        file_with_size(path, line_count, 1024, hash)
    }

    fn file_with_size(
        path: &str,
        line_count: Option<usize>,
        size_bytes: u64,
        hash: &str,
    ) -> ReviewableFile {
        ReviewableFile {
            path: path.to_string(),
            file_hash: crate::schema::FileHash::blake3(hash),
            size_bytes,
            extension: path
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_string()),
            line_count,
        }
    }
}
