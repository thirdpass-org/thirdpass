use crate::review::common::Priority;

pub use thirdpass_core::schema::{Position, Selection};

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    #[serde(skip)]
    pub id: crate::common::index::ID,
    #[serde(default)]
    pub security: Priority,
    #[serde(default)]
    pub complexity: Priority,
    #[serde(rename = "file")]
    pub path: std::path::PathBuf,
    #[serde(rename = "description")]
    pub message: String,
    pub selection: Option<Selection>,
}

impl Ord for Comment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.security,
            &self.complexity,
            &self.path,
            &self.message,
            &self.selection,
            &self.id,
        )
            .cmp(&(
                &other.security,
                &other.complexity,
                &other.path,
                &other.message,
                &other.selection,
                &other.id,
            ))
    }
}

impl PartialOrd for Comment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
