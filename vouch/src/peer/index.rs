use anyhow::Result;
use std::collections::HashSet;
use std::convert::TryFrom;

use super::common;
use crate::common::StoreTransaction;

#[derive(Debug, Default)]
pub struct Fields<'a> {
    pub id: Option<crate::common::index::ID>,
    pub alias: Option<&'a str>,
    pub git_url: Option<&'a crate::common::GitUrl>,
    pub parent_id: Option<crate::common::index::ID>,
}

/// Returns the root peer.
pub fn get_root(tx: &StoreTransaction) -> Result<Option<common::Peer>> {
    Ok(get(
        &Fields {
            alias: Some(common::ROOT_ALIAS),
            ..Default::default()
        },
        &tx,
    )?
    .into_iter()
    .next()
    .map(|x| x.clone()))
}

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    tx.index_tx().execute(
        "
    CREATE TABLE IF NOT EXISTS peer (
        id              INTEGER NOT NULL PRIMARY KEY,
        alias           TEXT NOT NULL UNIQUE,
        git_url         TEXT NOT NULL UNIQUE,
        parent_id       INTEGER,

        FOREIGN KEY(parent_id) REFERENCES peer(id)
    )",
        rusqlite::NO_PARAMS,
    )?;

    // Insert root peer if absent.
    let found_root_peer = !get(
        &Fields {
            alias: Some(common::ROOT_ALIAS),
            ..Default::default()
        },
        &tx,
    )?
    .is_empty();
    if !found_root_peer {
        let git_url = crate::common::GitUrl::try_from(common::ROOT_DEFAULT_GIT_URL)?;
        log::debug!(
            "Failed to find root peer. Inserting: {alias} ({git_url})",
            alias = common::ROOT_ALIAS,
            git_url = git_url
        );
        insert(common::ROOT_ALIAS, &git_url, None, tx)?;
    }
    Ok(())
}

fn insert(
    alias: &str,
    git_url: &crate::common::GitUrl,
    parent_id: Option<crate::common::index::ID>,
    tx: &StoreTransaction,
) -> Result<common::Peer> {
    tx.index_tx().execute(
        "
        INSERT INTO peer (alias, git_url, parent_id)
            VALUES (?1, ?2, ?3)
        ",
        rusqlite::params![alias, git_url.to_string(), parent_id],
    )?;
    let new_peer = common::Peer {
        id: tx.index_tx().last_insert_rowid(),
        alias: alias.to_string(),
        git_url: git_url.clone(),
        parent_id: parent_id,
    };
    Ok(new_peer)
}

/// Get matching peers.
pub fn get(fields: &Fields, tx: &StoreTransaction) -> Result<HashSet<common::Peer>> {
    let id =
        crate::common::index::get_like_clause_param(fields.id.map(|id| id.to_string()).as_deref());
    let alias = crate::common::index::get_like_clause_param(fields.alias);
    let git_url =
        crate::common::index::get_like_clause_param(fields.git_url.map(|url| url.as_str()));
    let parent_id = crate::common::index::get_like_clause_param(
        fields.parent_id.map(|id| id.to_string()).as_deref(),
    );

    let sql_query = r"
        SELECT id, alias, git_url, parent_id
        FROM peer
        WHERE
            id LIKE :id ESCAPE '\'
            AND alias LIKE :alias ESCAPE '\'
            AND git_url LIKE :git_url ESCAPE '\'
            AND ifnull(parent_id, '') LIKE :parent_id ESCAPE '\'
    ";
    let mut statement = tx.index_tx().prepare(sql_query)?;
    let mut rows = statement.query_named(&[
        (":id", &id),
        (":alias", &alias),
        (":git_url", &git_url),
        (":parent_id", &parent_id),
    ])?;
    let mut peers = HashSet::new();
    while let Some(row) = rows.next()? {
        let git_url = crate::common::GitUrl::try_from(&row.get::<_, String>(2)?)?;
        peers.insert(common::Peer {
            id: row.get(0)?,
            alias: row.get(1)?,
            git_url,
            parent_id: row.get(3)?,
        });
    }
    Ok(peers)
}
