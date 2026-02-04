use anyhow::Result;
use std::collections::HashSet;

use super::common;
use crate::common::StoreTransaction;
use crate::registry;

#[derive(Debug, Default)]
pub struct Fields<'a> {
    pub id: Option<crate::common::index::ID>,
    pub package_name: Option<&'a str>,
    pub package_version: Option<&'a str>,

    // Filters match for any in set.
    pub registry_host_names: Option<std::collections::BTreeSet<&'a str>>,
}

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    tx.index_tx().execute(
        r"
        CREATE TABLE IF NOT EXISTS package (
            id                         INTEGER NOT NULL PRIMARY KEY,
            name                       TEXT NOT NULL,
            version                    TEXT NOT NULL,
            registry_ids               BLOB NOT NULL,
            artifact_hash              TEXT NOT NULL,

            UNIQUE(name, version, artifact_hash)
        )",
        rusqlite::NO_PARAMS,
    )?;
    Ok(())
}

pub fn insert(
    package_name: &str,
    package_version: &str,
    registries: &std::collections::BTreeSet<registry::Registry>,
    artifact_hash: &str,
    tx: &StoreTransaction,
) -> Result<common::Package> {
    assert!(
        !registries.is_empty(),
        "At least one registry must be assigned to a package before index insert."
    );
    let registry_ids: Vec<crate::common::index::ID> =
        registries.into_iter().map(|c| c.id).collect();
    let registry_ids = bincode::serialize(&registry_ids)?;

    tx.index_tx().execute_named(
        r"
            INSERT INTO package (
                name,
                version,
                registry_ids,
                artifact_hash
            )
            VALUES (
                :name,
                :version,
                :registry_ids,
                :artifact_hash
            )
        ",
        rusqlite::named_params! {
            ":name": package_name,
            ":version": package_version,
            ":registry_ids": registry_ids,
            ":artifact_hash": artifact_hash,
        },
    )?;
    Ok(common::Package {
        id: tx.index_tx().last_insert_rowid(),
        name: package_name.to_string(),
        version: package_version.to_string(),
        registries: registries.clone(),
        artifact_hash: artifact_hash.to_string(),
    })
}

pub fn get(fields: &Fields, tx: &StoreTransaction) -> Result<HashSet<common::Package>> {
    let id =
        crate::common::index::get_like_clause_param(fields.id.map(|id| id.to_string()).as_deref());
    let package_name = crate::common::index::get_like_clause_param(fields.package_name);
    let package_version = crate::common::index::get_like_clause_param(fields.package_version);

    let mut statement = tx.index_tx().prepare(
        r"
            SELECT *
            FROM package
            WHERE
                package.id LIKE :package_id ESCAPE '\'
                AND name LIKE :name ESCAPE '\'
                AND version LIKE :version ESCAPE '\'
        ",
    )?;
    let mut rows = statement.query_named(&[
        (":package_id", &id),
        (":name", &package_name),
        (":version", &package_version),
    ])?;

    let mut packages = HashSet::new();
    while let Some(row) = rows.next()? {
        let registry_ids: Option<Result<Vec<crate::common::index::ID>>> = row
            .get::<_, Option<Vec<u8>>>(3)?
            .map(|x| Ok(bincode::deserialize(&x)?));
        let registries = match registry_ids {
            Some(registry_ids) => {
                let registry_ids = registry_ids?;
                registry::index::get(
                    &registry::index::Fields {
                        ids: Some(&registry_ids),
                        ..Default::default()
                    },
                    &tx,
                )?
                .into_iter()
                .collect()
            }
            None => std::collections::BTreeSet::<registry::Registry>::new(),
        };

        // Skip package if none of the given registry host names match to any registry.
        if let Some(registry_host_names) = &fields.registry_host_names {
            let mut found_match = false;
            for registry_host_name in registry_host_names {
                found_match |= registries
                    .iter()
                    .any(|registry| &registry.host_name.as_str() == registry_host_name);
            }
            if !found_match {
                continue;
            }
        }

        let package = common::Package {
            id: row.get(0)?,
            name: row.get(1)?,
            version: row.get(2)?,
            registries: registries,
            artifact_hash: row.get(4)?,
        };
        packages.insert(package);
    }
    Ok(packages)
}

