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
