//! Helpers for the store's tests: stores in directories of their own, the
//! events of each kind as a writer is handed them, and the store's tables
//! read and written directly, with none of the store's checks, as a hand
//! edit or a newer cases could.

// Each test binary uses part of this module.
#![allow(dead_code)]

use std::path::Path;

use cases_core::{Append, Case, Store};
use cases_store_sqlite::{Db, Key, Writer};
use rusqlite::{Connection, OpenFlags, params};
use tempfile::TempDir;

/// A store at `cases.db` in a directory of its own, which goes when the test
/// ends. There is no file yet.
pub(crate) fn scratch() -> (TempDir, Db) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = Db::new(dir.path().join("cases.db"));
    (dir, db)
}

/// A time, from RFC 3339.
pub(crate) fn at(text: &str) -> cases_core::Time {
    text.parse().expect("the time reads")
}

/// A store at `path` whose clock stays at `now`.
pub(crate) fn fixed(path: &Path, now: &str) -> Db {
    let now = at(now);
    Db::new(path).with_clock(move || now)
}

/// An event to write.
pub(crate) fn append(author: &str, event: &str, data: &str) -> Append {
    Append {
        author: author.into(),
        event: event.into(),
        data: data.into(),
    }
}

/// An agent's event.
pub(crate) fn agent_event(event: &str, data: &str) -> Append {
    append("agent", event, data)
}

/// A human's event.
pub(crate) fn human_event(event: &str, data: &str) -> Append {
    append("human", event, data)
}

/// An open of a case of `kind`, as Go's tests opened one: urgency today, a
/// title and a body, two options for a decision and two rows for an
/// approval.
pub(crate) fn open_of(kind: &str) -> Append {
    let extra = match kind {
        "decision" => r#", "options": ["Pin", "Float"]"#,
        "approval" => {
            r#", "rows": [{"id": "a", "label": "Install deps", "script": "npm ci", "link": "https://example.com/a"}, {"id": "b", "label": "Migrate", "script": "make migrate", "link": "https://example.com/b"}]"#
        }
        _ => "",
    };
    agent_event(
        "open",
        &format!(
            r#"{{"kind": "{kind}", "urgency": "today", "title": "A {kind} case", "body": "Body."{extra}}}"#
        ),
    )
}

/// An answer to a case of `kind`, as Go's tests gave one.
pub(crate) fn answer_of(kind: &str) -> Append {
    let data = match kind {
        "decision" => r#"{"choice": 2}"#,
        "approval" => {
            r#"{"rows": [{"id": "a", "verdict": "approve"}, {"id": "b", "verdict": "hold", "note": "later"}]}"#
        }
        "signoff" => r#"{"signoff": "accept"}"#,
        "stuck" => r#"{"text": "Try the other mirror."}"#,
        "question" => r#"{"text": "The staging one."}"#,
        _ => r#"{"ack": true}"#,
    };
    human_event("answer", data)
}

/// The unnamed writer: the unnamed agent for an agent's event, and the
/// unnamed human for a human's.
pub(crate) fn unnamed() -> Writer {
    Writer::new(None)
}

/// Opens a case of `kind` in `db`.
pub(crate) fn create(db: &Db, kind: &str) -> (Key, Case) {
    db.create(&open_of(kind), &unnamed())
        .expect("the open is stored")
}

/// A connection of the test's own to the store's file, which must be there.
pub(crate) fn sql(db: &Db) -> Connection {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db.path(), flags).expect("the store opens");
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .expect("the busy timeout is set");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("foreign keys are on");
    conn
}

/// A row of the events table, or of the archive's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stored {
    pub(crate) seq: i64,
    pub(crate) author: String,
    pub(crate) event: String,
    pub(crate) at: String,
    pub(crate) data: String,
}

/// The case's rows in the events table, in seq order.
pub(crate) fn stored(db: &Db, key: Key) -> Vec<Stored> {
    table_rows(db, "events", key)
}

/// The case's rows in the archive, in seq order.
pub(crate) fn archived_rows(db: &Db, key: Key) -> Vec<Stored> {
    table_rows(db, "archive_events", key)
}

fn table_rows(db: &Db, table: &str, key: Key) -> Vec<Stored> {
    let conn = sql(db);
    let mut statement = conn
        .prepare(&format!(
            "SELECT seq, author, event, at, data FROM {table} WHERE case_key = ?1 ORDER BY seq"
        ))
        .expect("the query prepares");
    statement
        .query_map([key_sql(key)], |row| {
            let data: Vec<u8> = row.get(4)?;
            Ok(Stored {
                seq: row.get(0)?,
                author: row.get(1)?,
                event: row.get(2)?,
                at: row.get(3)?,
                data: String::from_utf8(data).expect("the data is UTF-8"),
            })
        })
        .expect("the query runs")
        .collect::<Result<_, _>>()
        .expect("the rows read")
}

/// The names the rows had as files, such as `0002-human-answer.json`.
pub(crate) fn names(rows: &[Stored]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{:04}-{}-{}.json", row.seq, row.author, row.event))
        .collect()
}

/// The key as SQLite holds it.
pub(crate) fn key_sql(key: Key) -> i64 {
    i64::try_from(key.get()).expect("a key fits a row id")
}

/// Stores an event row as given on the case `key`, with none of the checks a
/// write makes, as a newer cases or a hand edit could. It adds the case when
/// the store has no case with that key.
pub(crate) fn insert_row(db: &Db, key: i64, seq: i64, author: &str, event: &str, data: &str) {
    let conn = sql(db);
    conn.execute(
        "INSERT OR IGNORE INTO cases (key, opened_at) VALUES (?1, '2026-09-15T09:00:00Z')",
        [key],
    )
    .expect("the case is stored");
    conn.execute(
        "INSERT INTO events (case_key, seq, author, event, at, data) VALUES (?1, ?2, ?3, ?4, '', ?5)",
        params![key, seq, author, event, data.as_bytes()],
    )
    .expect("the row is stored");
}

/// Stores a new case with these rows, as given, and gives its key.
pub(crate) fn insert_case(db: &Db, rows: &[(u64, &str, &str, &str)]) -> Key {
    let conn = sql(db);
    conn.execute("INSERT INTO cases (opened_at) VALUES ('')", [])
        .expect("the case is stored");
    let key = conn.last_insert_rowid();
    for &(seq, author, event, data) in rows {
        conn.execute(
            "INSERT INTO events (case_key, seq, author, event, at, data) VALUES (?1, ?2, ?3, ?4, '', ?5)",
            params![key, i64::try_from(seq).expect("the seq fits"), author, event, data.as_bytes()],
        )
        .expect("the row is stored");
    }
    Key::new(u64::try_from(key).expect("a key is positive")).expect("a key")
}

/// Moves the case `key` from the archive back into the store, in one
/// transaction, as a restore by hand would. There is no restore command.
pub(crate) fn put_back(db: &Db, key: Key) {
    let mut conn = sql(db);
    let tx = conn.transaction().expect("a transaction");
    for statement in [
        "INSERT INTO cases (key, opened_at) SELECT key, opened_at FROM archive_cases WHERE key = ?1",
        "INSERT INTO events (change, case_key, seq, author, event, at, data)
         SELECT change, case_key, seq, author, event, at, data FROM archive_events WHERE case_key = ?1",
        "DELETE FROM archive_events WHERE case_key = ?1",
        "DELETE FROM archive_cases WHERE key = ?1",
    ] {
        tx.execute(statement, [key_sql(key)]).expect("the case moves back");
    }
    tx.commit().expect("the move commits");
}

/// Removes the store's file and its `-wal` and `-shm`, as a person throwing
/// the store away would.
pub(crate) fn throw_away(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{}: {error}", Path::new(&file).display()),
        }
    }
}

/// Throws the store at `path` away and starts it again with one fyi case, as
/// a command in another process would. Nothing in this process may be using
/// the store meanwhile.
pub(crate) fn replace_store(path: &Path) -> Key {
    throw_away(path);
    let again = Db::new(path);
    create(&again, "fyi").0
}
