use anyhow::{format_err, Result};

use std::collections::HashSet;

use super::comment;
use super::common;
use crate::common::StoreTransaction;
use crate::package;
use crate::peer;

#[derive(Debug, Default)]
pub struct Fields<'a> {
    pub id: Option<crate::common::index::ID>,
    pub peer: Option<&'a peer::Peer>,

    pub package_name: Option<&'a str>,
    pub package_version: Option<&'a str>,

    // Filters match for any in set.
    pub registry_host_names: Option<std::collections::BTreeSet<&'a str>>,
}

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    comment::index::setup(&tx)?;

    tx.index_tx().execute(
        r"
        CREATE TABLE IF NOT EXISTS review (
            id                    INTEGER NOT NULL PRIMARY KEY,
            peer_id               INTEGER NOT NULL,
            package_id            INTEGER NOT NULL,
            comment_ids           BLOB,

            UNIQUE(peer_id, package_id)
            FOREIGN KEY(peer_id) REFERENCES peer(id)
            CONSTRAINT fk_package
                FOREIGN KEY (package_id)
                REFERENCES package(id)
                ON DELETE CASCADE
        )",
        rusqlite::NO_PARAMS,
    )?;
    Ok(())
}

pub fn insert(
    comments: &std::collections::BTreeSet<comment::Comment>,
    peer: &crate::peer::Peer,
    package: &crate::package::Package,
    tx: &StoreTransaction,
) -> Result<common::Review> {
    let comment_ids: Vec<crate::common::index::ID> = comments.into_iter().map(|c| c.id).collect();
    let comment_ids = if !comment_ids.is_empty() {
        Some(bincode::serialize(&comment_ids)?)
    } else {
        None
    };

    tx.index_tx().execute_named(
        r"
            INSERT INTO review (
                peer_id,
                package_id,
                comment_ids
            )
            VALUES (
                :peer_id,
                :package_id,
                :comment_ids
            )
        ",
        &[
            (":peer_id", &peer.id),
            (":package_id", &package.id),
            (":comment_ids", &comment_ids),
        ],
    )?;
    Ok(common::Review {
        id: tx.index_tx().last_insert_rowid(),
        peer: peer.clone(),
        package: package.clone(),
        comments: comments.clone(),
        metadata: common::ReviewMetadata::default(),
        target_file: None,
        overall_security_summary: common::SecuritySummary::default(),
    })
}

pub fn update(review: &common::Review, tx: &StoreTransaction) -> Result<()> {
    remove_stale_comments(&review, &tx)?;

    tx.index_tx().execute_named(
        r"
            UPDATE review
            SET
                peer_id = :peer_id,
                package_id = :package_id,
                comment_ids = :comment_ids
            WHERE
                id = :id
        ",
        &[
            (":id", &review.id),
            (":peer_id", &review.peer.id),
            (":package_id", &review.package.id),
            (
                ":comment_ids",
                &bincode::serialize(&review.comments.iter().map(|c| c.id).collect::<Vec<_>>())?,
            ),
        ],
    )?;
    Ok(())
}

fn remove_stale_comments(review: &common::Review, tx: &StoreTransaction) -> Result<()> {
    let current_reviews = get(
        &Fields {
            id: Some(review.id),
            ..Default::default()
        },
        &tx,
    )?;
    let current_review = match current_reviews.first() {
        Some(current_review) => current_review,
        None => {
            // No current review, no stale comments to remove.
            return Ok(());
        }
    };

    let current_comments = current_review
        .comments
        .clone()
        .into_iter()
        .collect::<HashSet<_>>();
    let new_comments = review.comments.clone().into_iter().collect::<HashSet<_>>();
    let stale_comments =
        crate::common::index::get_difference_sans_id(&current_comments, &new_comments)?;

    for comment in stale_comments {
        comment::index::remove(
            &comment::index::Fields {
                id: Some(comment.id),
                ..Default::default()
            },
            &tx,
        )?;
    }
    Ok(())
}

pub fn get(fields: &Fields, tx: &StoreTransaction) -> Result<Vec<common::Review>> {
    let review_id =
        crate::common::index::get_like_clause_param(fields.id.map(|id| id.to_string()).as_deref());

    let package_name = crate::common::index::get_like_clause_param(fields.package_name);
    let package_version = crate::common::index::get_like_clause_param(fields.package_version);

    let peer_id = crate::common::index::get_like_clause_param(
        fields.peer.map(|peer| peer.id.to_string()).as_deref(),
    );

    let mut statement = tx.index_tx().prepare(
        r"
        SELECT
            review.id,
            peer.id,
            package.id,
            review.comment_ids
        FROM review
        JOIN peer
            ON review.peer_id = peer.id
        JOIN package
            ON review.package_id = package.id
        WHERE
            review.id LIKE :review_id ESCAPE '\'
            AND package.name LIKE :name ESCAPE '\'
            AND package.version LIKE :version ESCAPE '\'
            AND peer.id LIKE :peer_id ESCAPE '\'
        ",
    )?;
    let mut rows = statement.query_named(&[
        (":review_id", &review_id),
        (":name", &package_name),
        (":version", &package_version),
        (":peer_id", &peer_id),
    ])?;

    let mut reviews = Vec::new();
    while let Some(row) = rows.next()? {
        let peer = peer::index::get(
            &peer::index::Fields {
                id: row.get(1)?,
                ..Default::default()
            },
            &tx,
        )?
        .into_iter()
        .next()
        .ok_or(format_err!("Failed to find review peer in index."))?;

        let package = package::index::get(
            &package::index::Fields {
                id: row.get(2)?,
                ..Default::default()
            },
            &tx,
        )?
        .into_iter()
        .next()
        .ok_or(format_err!("Failed to find review package in index."))?;

        // Skip review if associated package registries do not match on given registry host names.
        if let Some(registry_host_names) = &fields.registry_host_names {
            if !package
                .registries
                .iter()
                .any(|registry| registry_host_names.contains(registry.host_name.as_str()))
            {
                continue;
            }
        }

        let comment_ids: Option<Result<Vec<crate::common::index::ID>>> = row
            .get::<_, Option<Vec<u8>>>(3)?
            .map(|x| Ok(bincode::deserialize(&x)?));
        let comments = match comment_ids {
            Some(comment_ids) => {
                let comment_ids = comment_ids?;
                comment::index::get(
                    &comment::index::Fields {
                        ids: Some(&comment_ids),
                        ..Default::default()
                    },
                    &tx,
                )?
                .into_iter()
                .collect()
            }
            None => std::collections::BTreeSet::<comment::Comment>::new(),
        };

        let mut review = common::Review {
            id: row.get(0)?,
            peer,
            package,
            comments,
            metadata: common::ReviewMetadata::default(),
            target_file: None,
            overall_security_summary: common::SecuritySummary::default(),
        };
        review.overall_security_summary = crate::review::overall_security_summary(&review)?;
        reviews.push(review);
    }
    Ok(reviews)
}
