use crate::review::common::{Priority, Summary};
use std::hash::Hash;

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct Position {
    pub line: i64,
    pub character: i64,
}

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    #[serde(skip)]
    pub id: crate::common::index::ID,
    #[serde(default)]
    pub security: Priority,
    #[serde(default)]
    pub complexity: Priority,
    #[serde(skip_serializing, default)]
    pub summary: Option<Summary>,
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
            &self.summary,
            &self.path,
            &self.message,
            &self.selection,
            &self.id,
        )
            .cmp(&(
                &other.security,
                &other.complexity,
                &other.summary,
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

impl Comment {
    pub fn apply_legacy_summary(&mut self) {
        if let Some(summary) = &self.summary {
            self.security = match summary {
                Summary::Fail => Priority::Critical,
                Summary::Warn => Priority::Medium,
                Summary::Pass => Priority::Low,
                Summary::Todo => Priority::Low,
            };
        }
    }
}
