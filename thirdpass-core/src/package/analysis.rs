use anyhow::Result;

/// The filesystem kind for a package analysis entry.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PathType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
}

/// Line-count analysis for one package-relative path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PathAnalysis {
    /// Whether the path is a file or directory.
    pub path_type: PathType,
    /// Total line count for this file or directory subtree.
    pub line_count: usize,
}

/// Mapping from package-relative paths to their line-count analysis.
pub type Analysis = std::collections::BTreeMap<std::path::PathBuf, PathAnalysis>;

/// Compute the lowercase Blake3 digest for a file.
pub fn file_blake3_digest(path: &std::path::PathBuf) -> Result<String> {
    let input = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(input);
    blake3_digest(reader)
}

fn blake3_digest<R: std::io::Read>(mut reader: R) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hasher.finalize().to_hex().as_str().to_string())
}

fn get_file_line_counts(
    workspace_directory: &std::path::PathBuf,
) -> Result<std::collections::BTreeMap<std::path::PathBuf, usize>> {
    let paths = &[workspace_directory];
    let excluded = &[];
    let config = tokei::Config {
        hidden: Some(true),
        no_ignore: Some(true),
        ..tokei::Config::default()
    };
    let mut languages = tokei::Languages::new();
    languages.get_statistics(paths, excluded, &config);

    let mut file_line_counts = std::collections::BTreeMap::new();

    for (_language_type, language) in &languages {
        for report in &language.reports {
            let file_path = report.name.clone();
            let total_line_count = report.stats.lines();
            *file_line_counts.entry(file_path).or_insert(0) += total_line_count;
        }
    }
    Ok(file_line_counts)
}

fn get_directory_line_counts(
    file_line_counts: &std::collections::BTreeMap<std::path::PathBuf, usize>,
    workspace_directory: &std::path::PathBuf,
) -> Result<std::collections::BTreeMap<std::path::PathBuf, usize>> {
    let mut directory_line_counts = std::collections::BTreeMap::new();
    for (file_path, line_count) in file_line_counts.iter() {
        let mut path = file_path.clone();
        while path.pop() {
            *directory_line_counts.entry(path.clone()).or_insert(0) += line_count;
            if path == *workspace_directory {
                break;
            }
        }
    }
    Ok(directory_line_counts)
}

/// Analyze a package workspace and return file and directory line counts.
pub fn analyse(workspace_directory: &std::path::PathBuf) -> Result<Analysis> {
    let file_line_counts = get_file_line_counts(workspace_directory)?;
    let directory_line_counts = get_directory_line_counts(&file_line_counts, workspace_directory)?;

    let mut analysis = std::collections::BTreeMap::new();
    for (path_type, line_counts) in vec![
        (PathType::File, file_line_counts),
        (PathType::Directory, directory_line_counts),
    ] {
        for (path, line_count) in line_counts.into_iter() {
            let path = path.strip_prefix(workspace_directory)?.to_path_buf();
            analysis.insert(
                path,
                PathAnalysis {
                    path_type,
                    line_count,
                },
            );
        }
    }
    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correct_directory_line_counts() -> Result<()> {
        let workspace_directory = std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0");
        let mut file_line_counts = std::collections::BTreeMap::new();
        file_line_counts.insert(
            std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0/file_1.js"),
            22,
        );
        file_line_counts.insert(
            std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0/build/file_2.js"),
            37,
        );
        file_line_counts.insert(
            std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0/build/file_3.js"),
            5,
        );

        let result = get_directory_line_counts(&file_line_counts, &workspace_directory)?;
        let mut expected = std::collections::BTreeMap::new();
        expected.insert(
            std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0"),
            64,
        );
        expected.insert(
            std::path::PathBuf::from("/npmjs.com/d3/4.10.0/d3-4.10.0/build"),
            42,
        );
        assert_eq!(result, expected);
        Ok(())
    }
}
