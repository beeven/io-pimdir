//! # Schema
//!
//! The migration runner (STORAGE §6) and the checks refusing a store this
//! crate does not read: a newer version, disagreeing stamps, an earlier
//! draft's shape, a foreign hash.

use alloc::{format, string::String, vec::Vec};

use std::{collections::BTreeMap, path::Path};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params, params};

use crate::{
    client::{PimdirError, PimdirNotCurrent, blobs::PimdirBlobs, busy_or_sql, rows},
    codec,
    hash::PimdirHashAlgo,
    object::PimdirHash,
    sql,
    summary::{PimdirSummary, calendar, mail},
};

/// The core tables and triggers every draft's store holds: one missing is
/// a damaged store, refused before anything is reconciled, where any other
/// difference from the canonical schema is an earlier draft's, reconciled
/// on open (§6). The triggers are named since the last drafts moved the
/// feed into them, and a store lacking one stamps nothing.
const SCHEMA: [&str; 16] = [
    "bindings",
    "collections",
    "collections_restamp_items",
    "contact_summary",
    "event_summary",
    "item_address",
    "items",
    "items_stamp_request",
    "journal_summary",
    "mail_summary",
    "objects",
    "objects_count_collect",
    "queue",
    "sources",
    "store_meta",
    "task_summary",
];

/// Runs every migration above `user_version` in order, each in its own
/// transaction setting the version it reaches (§6), the first one
/// seeding `store_meta` (§4.2); then checks the store as [`check`] does.
/// A store above the current version is refused.
pub(crate) fn init(
    conn: &mut Connection,
    hash: PimdirHashAlgo,
    dir: &Path,
) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > sql::VERSION {
        return Err(PimdirError::Version { found: version });
    }

    for (index, migration) in sql::MIGRATIONS.iter().enumerate() {
        let reached = index as i64 + 1;
        if reached <= version {
            continue;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute_batch(migration)?;
        if reached == 1 {
            tx.execute(
                sql::INIT_STORE_META,
                named_params! { ":version": reached, ":hash_algo": hash.as_str() },
            )?;
        } else {
            tx.execute(
                "UPDATE store_meta SET version = :version WHERE id = 1",
                named_params! { ":version": reached },
            )?;
        }
        tx.pragma_update(None, "user_version", reached)?;
        tx.commit().map_err(busy_or_sql)?;
    }

    // NOTE: a core table or trigger missing is a damaged store, not an
    // earlier one: refused before anything is reconciled.
    check(conn)?;
    reconcile(conn, &PimdirBlobs::open(dir, hash))?;
    check(conn)
}

/// One table, index or trigger as `sqlite_schema` declares it.
struct PimdirSchemaObject {
    /// `table`, `index` or `trigger`.
    kind: String,
    /// The table it belongs to.
    table: String,
    /// Its declaring statement as stored.
    sql: String,
}

/// Reconciles the store against the canonical schema's own text (§6): the
/// canonical migrations are applied to an empty in-memory database, and
/// every table, index and trigger whose stored text differs from its
/// canonical one, comments dropped and whitespace collapsed, is brought to
/// it in one transaction; a store already current is left untouched.
fn reconcile(conn: &mut Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let canonical = Connection::open_in_memory()?;
    for migration in sql::MIGRATIONS {
        canonical.execute_batch(migration)?;
    }
    let wanted = objects(&canonical)?;
    let held = objects(conn)?;
    if first_difference(&wanted, &held).is_none() {
        return Ok(());
    }

    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON;")?;
    let reconciled = rebuild(conn, &canonical, &wanted, &held, blobs);
    conn.execute_batch("PRAGMA legacy_alter_table = OFF; PRAGMA foreign_keys = ON;")?;
    reconciled
}

/// The four steps of §6 in one transaction: the refcounts settled, every
/// table whose text differs rebuilt from the canonical one, what is
/// missing created, what differs recreated, what the canonical schema
/// lacks dropped; then the columns a rebuild added backfilled, and no
/// foreign key left dangling.
fn rebuild(
    conn: &mut Connection,
    canonical: &Connection,
    wanted: &BTreeMap<String, PimdirSchemaObject>,
    held: &BTreeMap<String, PimdirSchemaObject>,
    blobs: &PimdirBlobs,
) -> Result<(), PimdirError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    let without_invitation =
        has_table(&tx, "mail_summary")? && !has_column(&tx, "mail_summary", "invitation")?;
    let without_organisation =
        has_table(&tx, "task_summary")? && !has_column(&tx, "task_summary", "priority")?;
    let without_shared_object =
        has_table(&tx, "bindings")? && !has_column(&tx, "bindings", "shared_object")?;
    let pointers = ["objects", "items", "bindings", "queue"];
    if pointers.iter().all(|table| held.contains_key(*table)) {
        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
    }

    for (name, object) in wanted.iter().filter(|(_, o)| o.kind == "table") {
        let Some(stored) = held.get(name) else {
            continue;
        };
        if normalised(&stored.sql) == normalised(&object.sql) {
            continue;
        }
        for (attached, other) in held {
            if other.table == *name && other.kind != "table" {
                tx.execute_batch(&format!("DROP {} {attached};", other.kind.to_uppercase()))?;
            }
        }
        let columns: Vec<String> = rows(
            &tx,
            "SELECT name FROM pragma_table_info(?1)",
            [name],
            |row| row.get(0),
        )?;
        let canonical_columns: Vec<String> = rows(
            canonical,
            "SELECT name FROM pragma_table_info(?1)",
            [name],
            |row| row.get(0),
        )?;
        let shared: Vec<&String> = columns
            .iter()
            .filter(|column| canonical_columns.contains(column))
            .collect();
        let shared = shared
            .iter()
            .map(|column| column.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        tx.execute_batch(&format!(
            "ALTER TABLE {name} RENAME TO {name}_aside;\n{};\n\
             INSERT INTO {name} ({shared}) SELECT {shared} FROM {name}_aside;",
            object.sql
        ))?;
        if has_table(&tx, "sqlite_sequence")? {
            tx.execute_batch(&format!(
                "UPDATE sqlite_sequence SET seq = max(seq, (SELECT seq FROM sqlite_sequence \
                 WHERE name = '{name}_aside')) WHERE name = '{name}';\n\
                 INSERT INTO sqlite_sequence(name, seq) SELECT '{name}', seq FROM sqlite_sequence \
                 WHERE name = '{name}_aside' \
                 AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = '{name}');"
            ))?;
        }
        tx.execute_batch(&format!("DROP TABLE {name}_aside;"))?;
    }

    for (name, object) in wanted.iter().filter(|(_, o)| o.kind == "table") {
        if !held.contains_key(name) {
            tx.execute_batch(&object.sql)?;
        }
    }
    for name in held
        .iter()
        .filter(|(_, o)| o.kind == "table")
        .map(|(name, _)| name)
    {
        if !wanted.contains_key(name) {
            tx.execute_batch(&format!("DROP TABLE {name};"))?;
        }
    }

    let now = objects(&tx)?;
    for (name, object) in now.iter().filter(|(_, o)| o.kind != "table") {
        let stale = wanted
            .get(name)
            .is_none_or(|wanted| normalised(&wanted.sql) != normalised(&object.sql));
        if stale {
            tx.execute_batch(&format!("DROP {} {name};", object.kind.to_uppercase()))?;
        }
    }
    let now = objects(&tx)?;
    for (name, object) in wanted.iter().filter(|(_, o)| o.kind != "table") {
        if !now.contains_key(name) {
            tx.execute_batch(&object.sql)?;
        }
    }

    if without_shared_object {
        tx.execute(sql::BACKFILL_SHARED_OBJECT, [])?;
    }
    if without_invitation {
        backfill_invitation(&tx, blobs)?;
    }
    if without_organisation {
        backfill_organisation(&tx, blobs)?;
    }

    let dangling = tx
        .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
        .optional()?;
    if dangling.is_some() {
        return Err(PimdirError::Stale {
            missing: "a foreign key the reconciliation left dangling",
        });
    }
    tx.commit().map_err(busy_or_sql)
}

/// Re-derives `mail_summary.invitation` from every held body, a page of
/// `list_held_mail` at a time, then records the invitations they imply
/// (`link_invitations_of` over every mail): a held message is never read
/// again (§6, Annex A.1). A body the blob directory does not hold leaves
/// its row `NULL`.
fn backfill_invitation(conn: &Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let mut after: Option<(String, String)> = None;
    loop {
        let page: Vec<(String, String, String)> = rows(
            conn,
            sql::LIST_HELD_MAIL,
            named_params! {
                ":after_collection": after.as_ref().map(|(collection, _)| collection.as_str()),
                ":after_link_id": after.as_ref().map(|(_, link_id)| link_id.as_str()),
                ":limit": 500,
            },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
        )?;
        let Some((collection, link_id, _)) = page.last() else {
            break;
        };
        after = Some((collection.clone(), link_id.clone()));
        for (collection, link_id, hash) in page {
            let Some(body) = blobs.get(&PimdirHash(hash))? else {
                continue;
            };
            let Some(PimdirSummary::Mail(mail)) = mail::derive(&body).summary else {
                continue;
            };
            conn.execute(
                "UPDATE mail_summary SET invitation = ?3 WHERE collection = ?1 AND link_id = ?2",
                params![collection, link_id, mail.invitation],
            )?;
        }
    }
    conn.execute(
        sql::LINK_INVITATIONS_OF,
        named_params! { ":link_id": None::<String> },
    )?;
    Ok(())
}

/// Re-derives `task_summary`'s `priority`, `categories`, `parent` and `mid`
/// from every held task body, a page of `list_held_tasks` at a time, then
/// records the references they imply (§6, §14.2): the four task rules run
/// over every item of their end. A body the blob directory does not hold
/// leaves its row empty.
fn backfill_organisation(conn: &Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let mut after: Option<(String, String)> = None;
    loop {
        let page: Vec<(String, String, String)> = rows(
            conn,
            sql::LIST_HELD_TASKS,
            named_params! {
                ":after_collection": after.as_ref().map(|(collection, _)| collection.as_str()),
                ":after_link_id": after.as_ref().map(|(_, link_id)| link_id.as_str()),
                ":limit": 500,
            },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
        )?;
        let Some((collection, link_id, _)) = page.last() else {
            break;
        };
        after = Some((collection.clone(), link_id.clone()));
        for (collection, link_id, hash) in page {
            let Some(body) = blobs.get(&PimdirHash(hash))? else {
                continue;
            };
            let Some(PimdirSummary::Task(task)) = calendar::derive(&body).summary else {
                continue;
            };
            conn.execute(
                "UPDATE task_summary SET priority = ?3, categories = ?4, parent = ?5, mid = ?6 \
                 WHERE collection = ?1 AND link_id = ?2",
                params![
                    collection,
                    link_id,
                    task.priority,
                    codec::ids_to_json(&task.categories),
                    task.parent,
                    task.mid,
                ],
            )?;
        }
    }
    for rule in [
        sql::LINK_PARENTS_OF,
        sql::LINK_SOURCES_OF,
        sql::LINK_PARENTS_TO,
        sql::LINK_SOURCES_TO,
    ] {
        conn.execute(rule, named_params! { ":link_id": None::<String> })?;
    }
    Ok(())
}

/// Every table, index and trigger a database declares, by name; the
/// automatic indexes, which declare no statement, and SQLite's own tables
/// left out.
fn objects(conn: &Connection) -> Result<BTreeMap<String, PimdirSchemaObject>, PimdirError> {
    let declared = rows(
        conn,
        "SELECT name, type, tbl_name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rowid",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                PimdirSchemaObject {
                    kind: row.get(1)?,
                    table: row.get(2)?,
                    sql: row.get(3)?,
                },
            ))
        },
    )?;
    Ok(declared.into_iter().collect())
}

/// The first table, index or trigger by name whose stored text differs
/// from its canonical one, or that only one side holds: what [`reconcile`]
/// would rebuild, create or drop (§6); `None` for a current store.
fn first_difference(
    wanted: &BTreeMap<String, PimdirSchemaObject>,
    held: &BTreeMap<String, PimdirSchemaObject>,
) -> Option<String> {
    let differs = wanted.iter().find(|(name, object)| {
        held.get(*name)
            .is_none_or(|held| normalised(&held.sql) != normalised(&object.sql))
    });
    let extra = || held.keys().find(|name| !wanted.contains_key(*name));
    differs.map(|(name, _)| name).or_else(extra).cloned()
}

/// Checks, changing nothing, that the store is the one this crate writes
/// as it stands: at the current `user_version`, every core table present
/// with agreeing stamps ([`check`]), and its schema text the canonical one,
/// so that [`init`] would migrate and reconcile nothing (§6).
pub(crate) fn current(conn: &Connection) -> Result<(), PimdirError> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found != sql::VERSION {
        return Err(PimdirError::NotCurrent(PimdirNotCurrent::Version { found }));
    }
    check(conn)?;

    let canonical = Connection::open_in_memory()?;
    for migration in sql::MIGRATIONS {
        canonical.execute_batch(migration)?;
    }
    match first_difference(&objects(&canonical)?, &objects(conn)?) {
        Some(object) => Err(PimdirError::NotCurrent(PimdirNotCurrent::Shape { object })),
        None => Ok(()),
    }
}

/// A declaring statement as §6 compares it: every `--` comment dropped and
/// whitespace collapsed to single spaces.
fn normalised(sql: &str) -> String {
    let mut out = String::new();
    for line in sql.lines() {
        let code = line.split_once("--").map_or(line, |(code, _)| code);
        for word in code.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// Whether the store holds `table`: one a later draft added is
/// missing from a store whose owner predates it.
pub(crate) fn has_table(conn: &Connection, table: &str) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether `table` has `column`: `collections.role` is missing from a
/// store its owner has not reconciled yet (§6), which a reader reads as
/// `NULL`.
pub(crate) fn has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The `task_summary` columns a later draft added (Annex A.4), which a
/// store its owner has not reconciled yet lacks (§6): the shape an owner
/// at io-pimdir 3b4bed8 keeps.
pub(crate) const TASK_ORGANISATION: [&str; 4] = ["priority", "categories", "parent", "mid"];

/// The columns among `columns` that `table` of the database `schema`
/// (`main`, or an attached one) lacks, in the order given.
pub(crate) fn missing_columns(
    conn: &Connection,
    schema: &str,
    table: &str,
    columns: &[&'static str],
) -> Result<Vec<&'static str>, PimdirError> {
    let held: Vec<String> = rows(
        conn,
        "SELECT name FROM pragma_table_info(?1, ?2)",
        [table, schema],
        |row| row.get(0),
    )?;
    Ok(columns
        .iter()
        .copied()
        .filter(|column| !held.iter().any(|held| held == column))
        .collect())
}

/// A statement reading every one of `columns` as `NULL`, so a reader
/// meeting a store that lacks them runs no statement naming them (§6).
///
/// Each identifier equal to one of `columns`, bare or qualified by a table
/// alias (`s.priority`), becomes `NULL`; a bound parameter (`:mid`) and
/// any other word are kept.
pub(crate) fn nulled(statement: &str, columns: &[&str]) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(statement.len());
    let mut rest = statement;
    let mut previous: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let starts = word(c) && !previous.is_some_and(|p| word(p) || matches!(p, ':' | '@' | '$'));
        if !starts {
            out.push(c);
            previous = Some(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let first = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        let mut end = first;
        let mut column = &rest[..first];
        if let Some(after) = rest[first..].strip_prefix('.')
            && after.starts_with(word)
        {
            let second = after.find(|c: char| !word(c)).unwrap_or(after.len());
            column = &after[..second];
            end = first + 1 + second;
        }
        if columns.contains(&column) {
            out.push_str("NULL");
        } else {
            out.push_str(&rest[..end]);
        }
        previous = rest[..end].chars().last();
        rest = &rest[end..];
    }
    out
}

/// Refuses a store stamped at the current version that this crate does
/// not read: an earlier draft's shape, or disagreeing stamps (§4.2).
///
/// The shape is checked first, so a store lacking `store_meta` is named
/// as stale rather than failing the read of its stamp. The spec is a
/// draft with no migration path, so a store missing a table is recreated
/// by its owner, never reconciled here.
pub(crate) fn check(conn: &Connection) -> Result<(), PimdirError> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'trigger')")?;
    let declared: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(missing) = SCHEMA
        .iter()
        .find(|name| !declared.iter().any(|d| d == *name))
    {
        return Err(PimdirError::Stale { missing });
    }

    let stamped: Option<i64> = conn
        .query_row("SELECT version FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    if let Some(store_meta) = stamped
        && store_meta != sql::VERSION
    {
        return Err(PimdirError::VersionMismatch {
            user_version: sql::VERSION,
            store_meta,
        });
    }

    Ok(())
}

/// Opens an existing database read-only or as a producer, refusing what
/// [`check`] refuses plus an unstamped or foreign version.
pub(crate) fn check_version(conn: &Connection) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        version if version == sql::VERSION => check(conn),
        0 => Err(PimdirError::Uncreated),
        found => Err(PimdirError::Version { found }),
    }
}

/// The algorithm the store records, checked against the one declared: a
/// handle computing another would name bodies no reader finds (§5).
pub(crate) fn hash_algo(
    conn: &Connection,
    declared: Option<PimdirHashAlgo>,
) -> Result<PimdirHashAlgo, PimdirError> {
    let stored: Option<String> = conn
        .query_row("SELECT hash_algo FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;

    let Some(stored) = stored else {
        return Ok(declared.unwrap_or_default());
    };
    let Some(algo) = PimdirHashAlgo::parse(&stored) else {
        return Err(PimdirError::HashAlgo {
            found: stored,
            declared: declared.map(|a| a.as_str()),
        });
    };
    match declared {
        Some(declared) if declared != algo => Err(PimdirError::HashAlgo {
            found: stored,
            declared: Some(declared.as_str()),
        }),
        _ => Ok(algo),
    }
}

#[cfg(test)]
mod tests {
    use super::nulled;

    #[test]
    fn a_missing_column_reads_as_null_bare_or_qualified() {
        let columns = ["priority", "mid"];
        assert_eq!(
            nulled("SELECT s.until, s.priority, s.mid FROM t s", &columns),
            "SELECT s.until, NULL, NULL FROM t s"
        );
        assert_eq!(
            nulled("SELECT until, priority, mid\nFROM t -- §6", &columns),
            "SELECT until, NULL, NULL\nFROM t -- §6"
        );
    }

    #[test]
    fn a_parameter_or_a_longer_word_is_kept() {
        let columns = ["mid"];
        assert_eq!(
            nulled(
                "WHERE amid = :mid AND mid_x = @mid AND x.mid IS NULL",
                &columns
            ),
            "WHERE amid = :mid AND mid_x = @mid AND NULL IS NULL"
        );
    }
}
