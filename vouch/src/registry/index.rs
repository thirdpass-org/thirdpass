use anyhow::Result;

use super::common;
use crate::common::StoreTransaction;
use std::collections::HashSet;

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    tx.index_tx().execute(
        "CREATE TABLE IF NOT EXISTS registry (
        id                         INTEGER NOT NULL PRIMARY KEY,
        host_name                  TEXT NOT NULL,
        human_url                  TEXT NOT NULL,
        artifact_url               TEXT NOT NULL UNIQUE
    )",
        rusqlite::NO_PARAMS,
    )?;
    Ok(())
}

pub fn insert(
    host_name: &str,
    human_url: &url::Url,
    artifact_url: &url::Url,
    tx: &StoreTransaction,
) -> Result<common::Registry> {
    tx.index_tx().execute_named(
        "INSERT INTO registry (
                host_name,
                human_url,
                artifact_url
            )
            VALUES (
                :host_name,
                :human_url,
                :artifact_url
            )",
        rusqlite::named_params! {
            ":host_name": host_name,
            ":human_url": human_url.to_string(),
            ":artifact_url": artifact_url.to_string(),
        },
    )?;
    Ok(common::Registry {
        id: tx.index_tx().last_insert_rowid(),
        host_name: host_name.to_string(),
        human_url: human_url.clone(),
        artifact_url: artifact_url.clone(),
    })
}

#[derive(Debug, Default)]
pub struct Fields<'a> {
    pub id: Option<crate::common::index::ID>,
    pub ids: Option<&'a Vec<crate::common::index::ID>>,
    pub host_name: Option<&'a str>,
    pub human_url: Option<&'a str>,
    pub artifact_url: Option<&'a str>,
}

pub fn get(fields: &Fields, tx: &StoreTransaction) -> Result<HashSet<common::Registry>> {
    let id =
        crate::common::index::get_like_clause_param(fields.id.map(|id| id.to_string()).as_deref());
    let ids_where_field = crate::common::index::get_ids_where_field(&fields.ids);
    let host_name = crate::common::index::get_like_clause_param(fields.host_name);
    let human_url = crate::common::index::get_like_clause_param(fields.human_url);
    let artifact_url = crate::common::index::get_like_clause_param(fields.artifact_url);

    let sql_query = format!(
        r"
            SELECT *
            FROM registry
            WHERE
                id LIKE :id ESCAPE '\'
                AND {ids_where_field}
                AND host_name LIKE :host_name ESCAPE '\'
                AND human_url LIKE :human_url ESCAPE '\'
                AND artifact_url LIKE :artifact_url ESCAPE '\'
        ",
        ids_where_field = ids_where_field
    );
    let mut statement = tx.index_tx().prepare(&sql_query)?;
    let mut rows = statement.query_named(&[
        (":id", &id),
        (":host_name", &host_name),
        (":human_url", &human_url),
        (":artifact_url", &artifact_url),
    ])?;
    let mut registries = HashSet::new();
    while let Some(row) = rows.next()? {
        registries.insert(common::Registry {
            id: row.get(0)?,
            host_name: row.get(1)?,
            human_url: url::Url::parse(row.get::<_, String>(2)?.as_str())?,
            artifact_url: url::Url::parse(row.get::<_, String>(3)?.as_str())?,
        });
    }
    Ok(registries)
}

pub fn ensure(
    host_name: &str,
    human_url: &url::Url,
    artifact_url: &url::Url,
    tx: &StoreTransaction,
) -> Result<common::Registry> {
    let existing = get(
        &Fields {
            artifact_url: Some(artifact_url.as_str()),
            ..Default::default()
        },
        &tx,
    )?;
    if let Some(registry) = existing.into_iter().next() {
        return Ok(registry);
    }
    insert(host_name, human_url, artifact_url, &tx)
}
