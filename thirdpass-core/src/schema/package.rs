use serde::{Deserialize, Serialize};

/// Package release that a review or assignment refers to.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ReviewTarget {
    /// Registry host that identifies the package ecosystem.
    pub registry_host: String,
    /// Package name inside the registry.
    pub package_name: String,
    /// Package version inside the registry.
    pub package_version: String,
    /// Content hash for the package source artifact.
    pub package_hash: String,
}

/// File inventory for a package archive.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PackageManifest {
    /// Regular files found in the extracted package archive.
    pub files: Vec<PackageManifestFile>,
}

/// Metadata for a regular file in a package archive.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct PackageManifestFile {
    /// Path of the file relative to the package root.
    pub path: String,
    /// Size of the file contents in bytes.
    pub size_bytes: u64,
}

/// Content hash for a file included in a review.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FileHash {
    /// Algorithm used to produce the hash digest.
    pub algorithm: FileHashAlgorithm,
    /// Lowercase hexadecimal hash digest.
    pub value: String,
}

impl FileHash {
    /// Build a Blake3 file hash from a lowercase hexadecimal digest.
    pub fn blake3(value: impl Into<String>) -> Self {
        Self {
            algorithm: FileHashAlgorithm::Blake3,
            value: value.into(),
        }
    }
}

/// Supported content hash algorithms for reviewed files.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "lowercase")]
pub enum FileHashAlgorithm {
    /// The Blake3 cryptographic hash algorithm.
    Blake3,
}
