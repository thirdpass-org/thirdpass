use crate::common;
use crate::registry;
use std::hash::Hash;

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Package {
    #[serde(skip)]
    pub id: common::index::ID,

    pub name: String,
    pub version: String,
    pub registries: std::collections::BTreeSet<registry::Registry>,
    pub artifact_hash: String,
}

impl Ord for Package {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.name,
            &self.version,
            &self.registries,
            &self.artifact_hash,
            &self.id,
        )
            .cmp(&(
                &other.name,
                &other.version,
                &other.registries,
                &other.artifact_hash,
                &other.id,
            ))
    }
}

impl PartialOrd for Package {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl crate::common::HashSansId for Package {
    fn hash_sans_id<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.version.hash(state);
        self.artifact_hash.hash(state);
    }
}
