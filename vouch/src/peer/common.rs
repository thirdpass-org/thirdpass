//! A module for data structures which are available to all super modules.
//!
//! This module contains data structures which are available to all super modules.
//! The number of data structures in this module should be minimized. The data structures
//! should be as simple as possible.
//!
//! Print statements are prohibited within this module. Logging is allowed.

use anyhow::Result;
use std::convert::TryFrom;
use std::hash::Hash;
pub static ROOT_ALIAS: &str = "root";
pub static ROOT_DEFAULT_GIT_URL: &str = "https://localhost";

#[derive(Debug, Clone, Hash, Eq, PartialEq)] //, Ord, PartialOrd)]
pub struct Peer {
    pub id: crate::common::index::ID,
    pub alias: String,
    pub git_url: crate::common::GitUrl,

    // Only the root peer can have None parent ID.
    pub parent_id: Option<crate::common::index::ID>,
}

impl Peer {
    pub fn is_root(&self) -> bool {
        self.alias.as_str() == ROOT_ALIAS && self.parent_id.is_none()
    }
}

impl Ord for Peer {
    /// Order on root status then git_url (which is unique).
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.is_root() && !other.is_root() {
            std::cmp::Ordering::Less
        } else if self.is_root() && other.is_root() {
            std::cmp::Ordering::Equal
        } else if !self.is_root() && other.is_root() {
            std::cmp::Ordering::Greater
        } else {
            self.git_url.cmp(&other.git_url)
        }
    }
}

impl PartialOrd for Peer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl crate::common::HashSansId for Peer {
    fn hash_sans_id<H: std::hash::Hasher>(&self, state: &mut H) {
        self.alias.hash(state);
        self.git_url.hash(state);
    }
}

impl Default for Peer {
    fn default() -> Self {
        Peer {
            id: 0,
            alias: "".to_string(),
            git_url: crate::common::GitUrl::try_from(ROOT_DEFAULT_GIT_URL).unwrap(),
            parent_id: None,
        }
    }
}

pub fn reviewer_peer(reviewer_uuid: &str, api_base: &str) -> Result<Peer> {
    let alias = format!("reviewer-{}", reviewer_uuid);
    let base = crate::common::api::normalize_base(api_base)?;
    let reviewer_url = crate::common::api::join(&base, &format!("reviewers/{}", reviewer_uuid))?;
    let git_url = crate::common::GitUrl::try_from(reviewer_url.as_str())?;
    Ok(Peer {
        id: 0,
        alias,
        git_url,
        parent_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_root_peer_greater_ordering() -> Result<()> {
        let root_peer = Peer {
            id: 0,
            alias: ROOT_ALIAS.to_string(),
            git_url: crate::common::GitUrl::try_from("http://localhost")?,
            parent_id: None,
        };
        let other_peer = Peer {
            id: 0,
            alias: "aA-other_peer".to_string(),
            git_url: crate::common::GitUrl::try_from("http://aA-localhost")?,
            parent_id: Some(42),
        };
        assert!(root_peer < other_peer);
        Ok(())
    }

    #[test]
    fn test_nonroot_peer_git_url_ordering() -> Result<()> {
        let peer_1 = Peer {
            id: 0,
            alias: "peer".to_string(),
            git_url: crate::common::GitUrl::try_from("http://localhost")?,
            parent_id: Some(42),
        };
        let peer_2 = Peer {
            id: 0,
            alias: "peer".to_string(),
            git_url: crate::common::GitUrl::try_from("http://aA-localhost")?,
            parent_id: Some(42),
        };
        assert!(peer_1 > peer_2);
        Ok(())
    }
}
