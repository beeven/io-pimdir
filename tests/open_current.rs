//! An owner open that creates, migrates and reconciles nothing (STORAGE
//! §6, §8): `PimdirStore::open_current` takes the owner lock on a store
//! exactly as this crate writes it, and refuses any other, leaving it as it
//! was, where `PimdirStore::open` would bring it to this crate's schema.

use std::fs;

use io_pimdir::client::{
    PimdirError, PimdirNotCurrent, PimdirStore, TIME_COLLECTION, reader::PimdirReader,
};

const ENTRY: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
BEGIN:VEVENT\r\nUID:entry-1@example.org\r\nDTSTAMP:20261028T090000Z\r\n\
DTSTART:20261028T090000Z\r\nDTEND:20261028T100000Z\r\nSUMMARY:Paint\r\n\
TRANSP:TRANSPARENT\r\nRELATED-TO:todo-1@example.org\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

fn columns(dir: &std::path::Path, table: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap();
    stmt.query_map([table], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn a_store_this_crate_created_opens_and_takes_writes() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());

    let mut store = PimdirStore::open_current(dir.path()).unwrap();
    store.ensure_time_collection().unwrap();
    let (_, seq) = store.put_item(TIME_COLLECTION, ENTRY).unwrap();
    drop(store);

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert!(reader.get_item(TIME_COLLECTION, seq).unwrap().is_some());
    drop(reader);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

#[test]
fn an_absent_store_is_refused_and_not_created() {
    let dir = tempfile::tempdir().unwrap();

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(
            refused,
            Some(PimdirError::NotCurrent(PimdirNotCurrent::Absent))
        ),
        "{refused:?}"
    );
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nothing written"
    );
}

#[test]
fn another_schema_version_is_refused_and_not_migrated() {
    for version in [0, 2] {
        let dir = tempfile::tempdir().unwrap();
        drop(PimdirStore::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);

        let refused = PimdirStore::open_current(dir.path()).err();
        assert!(
            matches!(
                refused,
                Some(PimdirError::NotCurrent(PimdirNotCurrent::Version { found })) if found == version
            ),
            "{refused:?}"
        );
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        let after: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(after, version, "left as it was");
    }
}

/// A store an owner at io-pimdir 3b4bed8 reconciled to its own schema: its
/// `task_summary` lacks the task organisation columns.
#[test]
fn a_store_needing_a_reconcile_is_refused_and_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "DROP INDEX task_summary_by_parent;
         DROP INDEX task_summary_by_mid;
         ALTER TABLE task_summary DROP COLUMN mid;
         ALTER TABLE task_summary DROP COLUMN parent;
         ALTER TABLE task_summary DROP COLUMN categories;
         ALTER TABLE task_summary DROP COLUMN priority;",
    )
    .unwrap();
    drop(conn);
    let before = columns(dir.path(), "task_summary");

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(
            &refused,
            Some(PimdirError::NotCurrent(PimdirNotCurrent::Shape { object })) if object == "task_summary"
        ),
        "{refused:?}"
    );
    assert_eq!(columns(dir.path(), "task_summary"), before, "not rebuilt");
    assert!(!before.iter().any(|column| column == "priority"));

    // NOTE: the lock went with the refusal, and open reconciles.
    drop(PimdirStore::open(dir.path()).unwrap());
    assert!(
        columns(dir.path(), "task_summary")
            .iter()
            .any(|column| column == "priority")
    );
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

#[test]
fn an_extra_index_is_a_difference_too() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("CREATE INDEX zz_extra ON items(sort_key);")
        .unwrap();
    drop(conn);

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(
            &refused,
            Some(PimdirError::NotCurrent(PimdirNotCurrent::Shape { object })) if object == "zz_extra"
        ),
        "{refused:?}"
    );
}

/// Held the way another process would: through a file description this
/// crate's own registry knows nothing about.
#[test]
fn a_store_another_process_owns_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("owner.lock"))
        .unwrap();
    fs4::FileExt::try_lock(&file).unwrap();

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(refused, Some(PimdirError::Owned(_))),
        "{refused:?}"
    );

    drop(file);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}
