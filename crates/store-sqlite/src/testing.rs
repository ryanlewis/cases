//! Helpers for the unit tests, which reach into the store.

use cases_core::{Append, Store};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::{Db, Key, Writer};

/// A store at `cases.db` in a directory of its own, where no file is yet.
pub(crate) fn scratch() -> (TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path().join("cases.db"));
    (dir, db)
}

/// The unnamed writer.
pub(crate) fn unnamed() -> Writer {
    Writer::new(None)
}

pub(crate) fn event(author: &str, event: &str, data: &str) -> Append {
    Append {
        author: author.into(),
        event: event.into(),
        data: data.into(),
    }
}

/// Opens an fyi case.
pub(crate) fn create(db: &Db) -> Key {
    let open = event(
        "agent",
        "open",
        r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
    );
    db.create(&open, &unnamed()).unwrap().0
}

/// Adds a note to the case.
pub(crate) fn note(db: &Db, key: Key) {
    let note = event("agent", "note", r#"{"body": "More."}"#);
    db.append(&key, &note, &unnamed(), None).unwrap();
}

/// A connection of the test's own to the store's file.
pub(crate) fn sql(db: &Db) -> Connection {
    let conn = Connection::open(db.path()).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn
}

/// The number of rows in `table`.
pub(crate) fn count(db: &Db, table: &str) -> i64 {
    sql(db)
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
