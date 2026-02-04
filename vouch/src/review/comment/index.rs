use anyhow::{format_err, Result};

use super::common;
use crate::common::StoreTransaction;

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    tx.index_tx().execute(
        r"
        CREATE TABLE IF NOT EXISTS comment (
            id                        INTEGER NOT NULL PRIMARY KEY,
            path                      TEXT NOT NULL,
            summary                   TEXT NOT NULL,
            message                   TEXT,
            selection_start_line      INTEGER,
            selection_start_character INTEGER,
            selection_end_line        INTEGER,
            selection_end_character   INTEGER
        )",
        rusqlite::NO_PARAMS,
    )?;
    Ok(())
}

/// Insert comment into index.
pub fn insert(
    path: &std::path::PathBuf,
    summary: &crate::review::common::Summary,
    message: &str,
    selection: &Option<common::Selection>,
    tx: &StoreTransaction,
) -> Result<common::Comment> {
    tx.index_tx().execute_named(
        r"
            INSERT INTO comment (
                path,
                summary,
                message,
                selection_start_line,
                selection_start_character,
                selection_end_line,
                selection_end_character
            )
            VALUES (
                :path,
                :summary,
                :message,
                :selection_start_line,
                :selection_start_character,
                :selection_end_line,
                :selection_end_character
            )
        ",
        &[
            (
                ":path",
                &path.clone().into_os_string().into_string().map_err(|_| {
                    format_err!("Failed to convert path into String: {}", path.display())
                })?,
            ),
            (":summary", &summary.to_string()),
            (":message", &message.to_string()),
            (
                ":selection_start_line",
                &selection.clone().map(|s| s.start.line),
            ),
            (
                ":selection_start_character",
                &selection.clone().map(|s| s.start.character),
            ),
            (
                ":selection_end_line",
                &selection.clone().map(|s| s.end.line),
            ),
            (
                ":selection_end_character",
                &selection.clone().map(|s| s.end.character),
            ),
        ],
    )?;
    Ok(common::Comment {
        id: tx.index_tx().last_insert_rowid(),
        path: path.clone(),
        summary: summary.clone(),
        message: message.to_string(),
        selection: selection.clone(),
    })
}

#[derive(Debug, Default)]
pub struct Fields<'a> {
    pub id: Option<crate::common::index::ID>,
    pub ids: Option<&'a Vec<crate::common::index::ID>>,
}

/// Get matching comments.
pub fn get(
    fields: &Fields,
    tx: &StoreTransaction,
) -> Result<std::collections::HashSet<common::Comment>> {
    let ids_where_field = crate::common::index::get_ids_where_field(&fields.ids);

    let sql_query = format!(
        "
        SELECT *
        FROM comment
        WHERE
            {ids_where_field}
    ",
        ids_where_field = ids_where_field
    );
    let mut statement = tx.index_tx().prepare(sql_query.as_str())?;
    let mut rows = statement.query_named(&[])?;

    let mut comments = std::collections::HashSet::new();
    while let Some(row) = rows.next()? {
        comments.insert(common::Comment {
            id: row.get(0)?,
            path: std::path::PathBuf::from(&row.get::<_, String>(1)?),
            summary: row.get::<_, String>(2)?.parse()?,
            message: row.get::<_, String>(3)?,
            selection: get_selection_field(row)?,
        });
    }
    Ok(comments)
}

/// Given a comment table row, return a comment selection.
fn get_selection_field(row: &rusqlite::Row<'_>) -> Result<Option<common::Selection>> {
    let selection_fields = [
        row.get::<_, Option<i64>>(4)?, // Start line.
        row.get::<_, Option<i64>>(5)?, // Start character.
        row.get::<_, Option<i64>>(6)?, // End line.
        row.get::<_, Option<i64>>(7)?, // End character.
    ];

    let all_fields_none = selection_fields
        .iter()
        .fold(true, |acc, field| acc && field.is_none());
    let all_fields_some = selection_fields
        .iter()
        .fold(true, |acc, field| acc && field.is_some());

    assert!(
        all_fields_none || all_fields_some,
        "Unexpected Some/None value incoherence in comment selection field."
    );

    if all_fields_none {
        return Ok(None);
    }

    let selection_fields: Vec<i64> = selection_fields
        .iter()
        .map(|x| x.expect("all fields should be some"))
        .collect();

    Ok(Some(common::Selection {
        start: common::Position {
            line: selection_fields[0],
            character: selection_fields[1],
        },
        end: common::Position {
            line: selection_fields[2],
            character: selection_fields[3],
        },
    }))
}

pub fn remove(fields: &Fields, tx: &StoreTransaction) -> Result<()> {
    let id =
        crate::common::index::get_like_clause_param(fields.id.map(|id| id.to_string()).as_deref());
    let ids_where_field = crate::common::index::get_ids_where_field(&fields.ids);
    let sql_query = format!(
        "
        DELETE
        FROM comment
        WHERE
            id LIKE :id ESCAPE '\\'
            AND {ids_where_field}
    ",
        ids_where_field = ids_where_field
    );
    tx.index_tx()
        .execute_named(sql_query.as_str(), &[(":id", &id)])?;
    Ok(())
}
