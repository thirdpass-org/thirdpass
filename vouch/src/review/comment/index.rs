use anyhow::{format_err, Result};

use super::common;
use crate::common::StoreTransaction;
use crate::review::common::{Priority, Summary};

pub fn setup(tx: &StoreTransaction) -> Result<()> {
    tx.index_tx().execute(
        r"
        CREATE TABLE IF NOT EXISTS comment (
            id                        INTEGER NOT NULL PRIMARY KEY,
            path                      TEXT NOT NULL,
            message                   TEXT,
            selection_start_line      INTEGER,
            selection_start_character INTEGER,
            selection_end_line        INTEGER,
            selection_end_character   INTEGER,
            summary                   TEXT,
            security                  TEXT,
            complexity                TEXT
        )",
        rusqlite::NO_PARAMS,
    )?;
    ensure_columns(tx)?;
    Ok(())
}

/// Insert comment into index.
pub fn insert(comment: &common::Comment, tx: &StoreTransaction) -> Result<common::Comment> {
    let summary = comment
        .summary
        .clone()
        .unwrap_or_else(|| summary_from_security(&comment.security));

    tx.index_tx().execute_named(
        r"
            INSERT INTO comment (
                path,
                summary,
                message,
                selection_start_line,
                selection_start_character,
                selection_end_line,
                selection_end_character,
                security,
                complexity
            )
            VALUES (
                :path,
                :summary,
                :message,
                :selection_start_line,
                :selection_start_character,
                :selection_end_line,
                :selection_end_character,
                :security,
                :complexity
            )
        ",
        &[
            (
                ":path",
                &comment
                    .path
                    .clone()
                    .into_os_string()
                    .into_string()
                    .map_err(|_| {
                        format_err!(
                            "Failed to convert path into String: {}",
                            comment.path.display()
                        )
                    })?,
            ),
            (":summary", &summary.to_string()),
            (":message", &comment.message.to_string()),
            (
                ":selection_start_line",
                &comment.selection.clone().map(|s| s.start.line),
            ),
            (
                ":selection_start_character",
                &comment.selection.clone().map(|s| s.start.character),
            ),
            (
                ":selection_end_line",
                &comment.selection.clone().map(|s| s.end.line),
            ),
            (
                ":selection_end_character",
                &comment.selection.clone().map(|s| s.end.character),
            ),
            (":security", &comment.security.to_string()),
            (":complexity", &comment.complexity.to_string()),
        ],
    )?;
    Ok(common::Comment {
        id: tx.index_tx().last_insert_rowid(),
        security: comment.security.clone(),
        complexity: comment.complexity.clone(),
        summary: Some(summary),
        path: comment.path.clone(),
        message: comment.message.to_string(),
        selection: comment.selection.clone(),
    })
}

fn summary_from_security(security: &Priority) -> Summary {
    match security {
        Priority::Critical => Summary::Fail,
        Priority::Medium => Summary::Warn,
        Priority::Low => Summary::Pass,
    }
}

fn ensure_columns(tx: &StoreTransaction) -> Result<()> {
    add_column_if_missing(tx, "security", "TEXT")?;
    add_column_if_missing(tx, "complexity", "TEXT")?;
    Ok(())
}

fn add_column_if_missing(tx: &StoreTransaction, column: &str, column_type: &str) -> Result<()> {
    if has_column(tx, column)? {
        return Ok(());
    }
    let sql = format!("ALTER TABLE comment ADD COLUMN {} {}", column, column_type);
    tx.index_tx().execute(sql.as_str(), rusqlite::NO_PARAMS)?;
    Ok(())
}

fn has_column(tx: &StoreTransaction, column: &str) -> Result<bool> {
    let mut statement = tx.index_tx().prepare("PRAGMA table_info(comment)")?;
    let mut rows = statement.query(rusqlite::NO_PARAMS)?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
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
        SELECT
            id,
            path,
            message,
            selection_start_line,
            selection_start_character,
            selection_end_line,
            selection_end_character,
            summary,
            security,
            complexity
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
        let summary: Option<Summary> = match row.get::<_, Option<String>>(7)? {
            Some(value) => Some(value.parse()?),
            None => None,
        };
        let security: Priority = match row.get::<_, Option<String>>(8)? {
            Some(value) => value.parse()?,
            None => summary
                .as_ref()
                .map(security_from_summary)
                .unwrap_or_default(),
        };
        let complexity: Priority = match row.get::<_, Option<String>>(9)? {
            Some(value) => value.parse()?,
            None => Priority::default(),
        };

        comments.insert(common::Comment {
            id: row.get(0)?,
            path: std::path::PathBuf::from(&row.get::<_, String>(1)?),
            security,
            complexity,
            summary,
            message: row.get::<_, String>(2)?,
            selection: get_selection_field(row, 3)?,
        });
    }
    Ok(comments)
}

fn security_from_summary(summary: &Summary) -> Priority {
    match summary {
        Summary::Fail => Priority::Critical,
        Summary::Warn => Priority::Medium,
        Summary::Pass => Priority::Low,
        Summary::Todo => Priority::Low,
    }
}

/// Given a comment table row, return a comment selection.
fn get_selection_field(
    row: &rusqlite::Row<'_>,
    base_index: usize,
) -> Result<Option<common::Selection>> {
    let selection_fields = [
        row.get::<_, Option<i64>>(base_index)?, // Start line.
        row.get::<_, Option<i64>>(base_index + 1)?, // Start character.
        row.get::<_, Option<i64>>(base_index + 2)?, // End line.
        row.get::<_, Option<i64>>(base_index + 3)?, // End character.
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
