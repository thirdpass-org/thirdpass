use std::hash::Hash;

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct Registry {
    #[serde(skip)]
    pub id: crate::common::index::ID,
    pub host_name: String,
    pub human_url: url::Url,
    pub artifact_url: url::Url,
}

impl crate::common::HashSansId for Registry {
    fn hash_sans_id<H: std::hash::Hasher>(&self, state: &mut H) {
        self.host_name.hash(state);
        self.human_url.hash(state);
        self.artifact_url.hash(state);
    }
}
