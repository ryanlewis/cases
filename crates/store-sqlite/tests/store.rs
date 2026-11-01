//! The store's file and schema, and the Store trait end to end. The Go
//! store's `store_test.go`, as `PARITY.md` maps it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fs;
use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::Duration;

use cases_core::{Case, Category, State, Store, StoreError};
use cases_store_sqlite::{Db, Error, Key, UnknownId};
use common::{
    agent_event, create, human_event, names, open_of, replace_store, scratch, sql, stored,
    throw_away, unnamed,
};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

/// Runs a case through every method of a store, as a caller of the trait
/// does, and checks what each gives.
fn lifecycle<S>(
    store: &S,
    agent: &S::Writer,
    human: &S::Writer,
) -> Vec<(S::Key, Vec<cases_core::Row>)>
where
    S: Store,
    S::Key: Copy + std::fmt::Debug,
    S::Error: std::fmt::Debug,
{
    let (stuck, _) = store.create(&open_of("stuck"), agent).unwrap();
    let (decision, _) = store.create(&open_of("decision"), agent).unwrap();
    let steps = [
        (
            decision,
            agent,
            agent_event("amend", r#"{"labels": ["deps"]}"#),
            None,
            State::Open,
        ),
        (
            decision,
            human,
            human_event("answer", r#"{"choice": 1}"#),
            Some(2),
            State::Answered,
        ),
        (
            decision,
            agent,
            agent_event("pickup", r#"{"by": "test"}"#),
            None,
            State::PickedUp,
        ),
        (
            decision,
            agent,
            agent_event("note", r#"{"body": "Which patch?", "reopen": true}"#),
            None,
            State::Open,
        ),
        (
            decision,
            human,
            human_event("answer", r#"{"choice": 2}"#),
            None,
            State::Answered,
        ),
        (
            decision,
            agent,
            agent_event("pickup", "{}"),
            None,
            State::PickedUp,
        ),
        (
            decision,
            agent,
            agent_event("close", r#"{"outcome": "Floated."}"#),
            None,
            State::Closed,
        ),
        (stuck, human, human_event("park", "{}"), None, State::Parked),
        (stuck, human, human_event("resume", "{}"), None, State::Open),
        (
            stuck,
            human,
            human_event("withdraw", "{}"),
            None,
            State::Withdrawn,
        ),
    ];
    for (i, (key, writer, event, at, state)) in steps.into_iter().enumerate() {
        let case = store.append(&key, &event, writer, at).unwrap();
        assert_eq!(case.state(), state, "step {i}");
        // The case a write gives is the case as stored.
        let rows = store.rows(&key).unwrap().unwrap();
        assert_eq!(Case::fold(&rows).unwrap(), case, "step {i}");
    }
    let rows = store.rows(&decision).unwrap().unwrap();
    let case = Case::fold(&rows).unwrap();
    assert_eq!((case.state(), case.revision()), (State::Closed, 8));
    let files: Vec<String> = case.events().iter().map(cases_core::Event::file).collect();
    assert_eq!(
        files,
        [
            "0001-agent-open.json",
            "0002-agent-amend.json",
            "0003-human-answer.json",
            "0004-agent-pickup.json",
            "0005-agent-note.json",
            "0006-human-answer.json",
            "0007-agent-pickup.json",
            "0008-agent-close.json",
        ]
    );
    store.list().unwrap()
}

#[test]
fn the_store_contract() {
    let (_dir, db) = scratch();
    let listed = lifecycle(&db, &unnamed(), &unnamed());
    let keys: Vec<u64> = listed.iter().map(|(key, _)| key.get()).collect();
    assert_eq!(keys, [1, 2]);
    assert_eq!(db.poller().poll().unwrap().len(), 2);

    // The errors callers branch on.
    let (key, _) = create(&db, "fyi");
    let missing = Key::new(900).unwrap();
    assert_eq!(db.rows(&missing).unwrap(), None);
    let pickup = agent_event("pickup", "{}");
    assert!(matches!(
        db.append(&missing, &pickup, &unnamed(), None),
        Err(StoreError::NoCase)
    ));
    // Text that is no id names no case: a path, say.
    assert_eq!("../elsewhere".parse::<Key>(), Err(UnknownId));
    let ack = human_event("answer", r#"{"ack": true}"#);
    let stale = db.append(&key, &ack, &unnamed(), Some(0));
    assert!(matches!(stale, Err(StoreError::Refused(e)) if e.category() == Category::Stale));
    let close = agent_event("close", r#"{"outcome": "done"}"#);
    let early = db.append(&key, &close, &unnamed(), None);
    assert!(matches!(early, Err(StoreError::Refused(e)) if e.category() == Category::Transition));

    // A store nothing has written to.
    let (_other, absent) = scratch();
    assert!(matches!(absent.list(), Err(Error::Missing { .. })));
    assert!(matches!(absent.rows(&key), Err(Error::Missing { .. })));
    assert!(matches!(absent.poller().poll(), Err(Error::Missing { .. })));
    assert!(matches!(
        absent.append(&key, &ack, &unnamed(), None),
        Err(StoreError::Store(Error::Missing { .. }))
    ));
}

/// Reads, and writes to a case, never make the file; the first create makes
/// it and its directory.
#[test]
fn only_create_makes_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new").join("dir").join("cases.db");
    let db = Db::new(&path);
    let key = Key::new(1).unwrap();
    assert!(matches!(db.list(), Err(Error::Missing { .. })));
    assert!(matches!(db.rows(&key), Err(Error::Missing { .. })));
    assert!(matches!(db.actors(), Err(Error::Missing { .. })));
    assert!(matches!(db.poller().poll(), Err(Error::Missing { .. })));
    let pickup = agent_event("pickup", "{}");
    assert!(db.append(&key, &pickup, &unnamed(), None).is_err());
    assert!(db.archive(&key, None).is_err());
    assert!(db.delete(&key, None).is_err());
    assert!(
        !path.parent().unwrap().exists(),
        "a read made the store's directory"
    );
    create(&db, "fyi");
    let meta = fs::metadata(&path).unwrap();
    assert!(meta.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o644);
    }
}

/// A file that exists but that no cases has written to, such as one another
/// writer is making, reads as no store and is left as it was.
#[test]
fn an_empty_file_reads_as_no_store() {
    let (_dir, db) = scratch();
    fs::write(db.path(), "").unwrap();
    assert!(matches!(db.list(), Err(Error::Missing { .. })));
    assert_eq!(
        fs::metadata(db.path()).unwrap().len(),
        0,
        "the read wrote to the file"
    );
    create(&db, "fyi");
    assert_eq!(db.list().unwrap().len(), 1);
}

/// Two writers making the store at once: one switches the new file to
/// write-ahead logging while the other holds the file's write lock. SQLite
/// refuses the switch at once rather than waiting, so create must wait for
/// the other writer itself.
#[test]
fn create_waits_for_another_writer_making_the_store() {
    let (_dir, db) = scratch();
    fs::write(db.path(), "").unwrap();
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut other = Connection::open_with_flags(db.path(), flags).unwrap();
    thread::scope(|scope| {
        let (held, locked) = mpsc::channel();
        scope.spawn(move || {
            let tx = other
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            held.send(()).unwrap();
            thread::sleep(Duration::from_millis(200));
            tx.rollback().unwrap();
        });
        locked.recv().unwrap();
        create(&db, "fyi");
    });
}

/// Writers on one missing path, each with a connection of its own, make one
/// store between them, and each opens its case in it.
#[test]
fn creators_racing_to_make_the_store_make_one() {
    let (_dir, db) = scratch();
    let n = 6;
    let start = Barrier::new(n);
    let path = db.path();
    let keys: Vec<u64> = thread::scope(|scope| {
        let creators: Vec<_> = (0..n)
            .map(|_| {
                scope.spawn(|| {
                    let db = Db::new(path);
                    start.wait();
                    create(&db, "fyi").0.get()
                })
            })
            .collect();
        creators.into_iter().map(|c| c.join().unwrap()).collect()
    });
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=6).collect::<Vec<u64>>());
    let listed = db.list().unwrap();
    assert_eq!(listed.len(), n);
    let conn = sql(&db);
    let versions: i64 = conn
        .query_row("SELECT count(*) FROM meta", [], |row| row.get(0))
        .unwrap();
    assert_eq!(versions, 1);
    let unnamed_agents: i64 = conn
        .query_row("SELECT count(*) FROM actors", [], |row| row.get(0))
        .unwrap();
    assert_eq!(unnamed_agents, 1);
}

/// A store removed on its own while a process still has it open, as when
/// cases.db is deleted while cases serve runs, leaves its -wal and -shm files
/// behind. A new store must not pair with them.
#[test]
fn create_refuses_the_files_of_a_removed_store() {
    let (_dir, serving) = scratch();
    create(&serving, "fyi");
    serving.poller().poll().unwrap();
    fs::remove_file(serving.path()).unwrap();

    let agent = Db::new(serving.path());
    let refused = agent.create(&open_of("fyi"), &common::unnamed());
    match refused {
        Err(StoreError::Store(error @ Error::Leftover { .. })) => assert!(
            error
                .to_string()
                .contains("-wal from an earlier store is still there"),
            "{error}"
        ),
        other => panic!("create beside a removed store's files: {other:?}"),
    }
    assert!(
        !serving.path().exists(),
        "the refused create made the store"
    );
    throw_away(serving.path());
    create(&agent, "fyi");
}

/// A serve or wait that is running when a newer cases changes the schema
/// stops writing and reading there, as a command started afterwards would.
#[test]
fn a_running_store_refuses_a_newer_schema() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let mut poller = db.poller();
    poller.poll().unwrap();
    sql(&db)
        .execute("UPDATE meta SET schema_version = 2", [])
        .unwrap();
    let refusal = "has schema version 2, newer than this cases reads (1)";
    let ack = human_event("answer", r#"{"ack": true}"#);
    match db.append(&key, &ack, &unnamed(), None) {
        Err(StoreError::Store(error @ Error::Newer { version: 2, .. })) => {
            assert!(error.to_string().contains(refusal), "{error}");
        }
        other => panic!("answer: {other:?}"),
    }
    assert!(matches!(
        poller.poll(),
        Err(Error::Newer { version: 2, .. })
    ));
    assert!(matches!(db.rows(&key), Err(Error::Newer { .. })));
    assert_eq!(stored(&db, key).len(), 1);
}

#[test]
fn the_schema() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let conn = sql(&db);
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let version: i64 = conn
        .query_row("SELECT schema_version FROM meta", [], |row| row.get(0))
        .unwrap();
    assert_eq!((mode.as_str(), version), ("wal", 1));
    let fails = |statement: &str, why: &str| {
        let error = conn.execute(statement, []).unwrap_err().to_string();
        assert!(error.contains(why), "{statement}: {error}, want {why}");
    };
    let row = |key: &str, seq: &str, data: &str| {
        format!(
            "INSERT INTO events (case_key, seq, author, event, at, data)
             VALUES ({key}, {seq}, 'agent', 'note', '', {data})"
        )
    };
    let key = common::key_sql(key);
    // A row needs its case, one number each, from 1.
    fails(&row("900", "2", "x'7b7d'"), "FOREIGN KEY");
    fails(&row(&key.to_string(), "1", "x'7b7d'"), "UNIQUE");
    fails(&row(&key.to_string(), "0", "x'7b7d'"), "CHECK");
    // The tables are strict: the record is bytes.
    fails(
        &row(&key.to_string(), "2", "'{}'"),
        "cannot store TEXT value in BLOB column",
    );
    // One actor for each kind and name, and one unnamed actor for each kind.
    let actor = |id: &str, kind: &str, name: &str| {
        format!(
            "INSERT INTO actors (id, kind, name, created) VALUES ('{id}', '{kind}', {name}, '')"
        )
    };
    fails(&actor("AAAAAAAAAAAAAAAAAAAAAA", "agent", "NULL"), "UNIQUE");
    conn.execute(&actor("BBBBBBBBBBBBBBBBBBBBBB", "agent", "'w1'"), [])
        .unwrap();
    fails(&actor("CCCCCCCCCCCCCCCCCCCCCC", "agent", "'w1'"), "UNIQUE");
    conn.execute(&actor("DDDDDDDDDDDDDDDDDDDDDD", "human", "'w1'"), [])
        .unwrap();
    conn.execute(&actor("EEEEEEEEEEEEEEEEEEEEEE", "human", "NULL"), [])
        .unwrap();
    // A case's key is never below 1.
    fails("INSERT INTO cases (key, opened_at) VALUES (0, '')", "CHECK");
    // One row for each worker of each wait, and the times are text.
    let wait = |id: &str, worker: &str, heard: &str| {
        format!(
            "INSERT INTO waits (id, worker, labels, kinds, since, heard) VALUES ('{id}', '{worker}', '[]', '[]', '', {heard})"
        )
    };
    conn.execute(&wait("w1", "bun-pins", "''"), []).unwrap();
    conn.execute(&wait("w1", "lead", "''"), []).unwrap();
    conn.execute(&wait("w2", "bun-pins", "''"), []).unwrap();
    fails(&wait("w1", "bun-pins", "''"), "UNIQUE");
    fails(
        &wait("w3", "lead", "x'00'"),
        "cannot store BLOB value in TEXT column",
    );
    // One doing line for each worker.
    let doing = "INSERT INTO doing (worker, doing, doing_at) VALUES ('bun-pins', 'x', '')";
    conn.execute(doing, []).unwrap();
    fails(doing, "UNIQUE");
}

#[test]
fn a_store_refuses_what_is_not_this_store() {
    // A newer schema, for a store opened afresh, as the next command would.
    let (_dir, db) = scratch();
    create(&db, "fyi");
    sql(&db)
        .execute("UPDATE meta SET schema_version = 2", [])
        .unwrap();
    let again = Db::new(db.path());
    let newer = "has schema version 2, newer than this cases reads (1): update cases";
    let listed = again.list().unwrap_err();
    assert!(listed.to_string().contains(newer), "{listed}");
    match again.create(&open_of("fyi"), &unnamed()) {
        Err(StoreError::Store(error)) => assert!(error.to_string().contains(newer), "{error}"),
        other => panic!("create: {other:?}"),
    }

    // A directory.
    let dir = tempfile::tempdir().unwrap();
    let at_dir = Db::new(dir.path());
    let listed = at_dir.list().unwrap_err();
    assert!(matches!(listed, Error::Directory { .. }), "{listed}");
    assert!(listed.to_string().contains("is a directory"), "{listed}");
    assert!(matches!(
        at_dir.create(&open_of("fyi"), &unnamed()),
        Err(StoreError::Store(Error::Directory { .. }))
    ));

    // Another database.
    let path = dir.path().join("other.db");
    Connection::open(&path)
        .unwrap()
        .execute("CREATE TABLE notes (body TEXT)", [])
        .unwrap();
    match Db::new(&path).create(&open_of("fyi"), &unnamed()) {
        Err(StoreError::Store(error @ Error::NotAStore { .. })) => {
            assert!(
                error.to_string().contains("is not a cases store"),
                "{error}"
            );
        }
        other => panic!("create in another database: {other:?}"),
    }

    // Not a database: SQLite's own error, and not a missing store.
    let path = dir.path().join("cases.db");
    fs::write(&path, "these are notes, not a database\n").unwrap();
    let listed = Db::new(&path).list().unwrap_err();
    assert!(matches!(listed, Error::Sqlite(_)), "{listed:?}");

    // A refused create writes nothing to either file (ST-07).
    let notes = fs::read(&path).unwrap();
    assert!(Db::new(&path).create(&open_of("fyi"), &unnamed()).is_err());
    assert_eq!(fs::read(&path).unwrap(), notes);
    let other = dir.path().join("other.db");
    let before = fs::read(&other).unwrap();
    assert!(Db::new(&other).create(&open_of("fyi"), &unnamed()).is_err());
    assert_eq!(fs::read(&other).unwrap(), before);
    let tables: i64 = Connection::open(&other)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 1, "the refused create made tables");
}

/// A store whose schema version no cases writes is damaged, and refused.
#[test]
fn a_damaged_store_is_refused() {
    for (change, what) in [
        ("UPDATE meta SET schema_version = 0", "schema version 0"),
        ("DELETE FROM meta", "its meta table has no schema version"),
    ] {
        let (_dir, db) = scratch();
        create(&db, "fyi");
        sql(&db).execute(change, []).unwrap();
        let listed = db.list().unwrap_err();
        assert!(matches!(listed, Error::Damaged { .. }), "{listed:?}");
        assert!(
            listed.to_string().ends_with(&format!("is damaged: {what}")),
            "{listed}"
        );
    }
}

/// A path the file system cannot follow, such as one through a file, is an
/// error of the file system, and not a missing store.
#[test]
fn a_path_through_a_file_is_an_error_not_a_missing_store() {
    let (_dir, db) = scratch();
    fs::write(db.path(), "").unwrap();
    let under = Db::new(db.path().join("cases.db"));
    assert!(matches!(under.list(), Err(Error::Io(_))));
    assert!(matches!(
        under.create(&open_of("fyi"), &unnamed()),
        Err(StoreError::Store(Error::Io(_)))
    ));
}

/// Whether the tests run as a user the file system refuses, as root is not.
#[cfg(unix)]
fn permissions_hold(dir: &std::path::Path) -> bool {
    fs::write(dir.join("probe"), "").is_err()
}

/// A store in a directory the store may not write to says so, and makes
/// nothing.
#[cfg(unix)]
#[test]
fn a_store_that_cannot_be_made_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    if permissions_hold(dir.path()) {
        let db = Db::new(dir.path().join("cases.db"));
        match db.create(&open_of("fyi"), &unnamed()) {
            Err(StoreError::Store(Error::Io(error))) => {
                assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("{other:?}"),
        }
        assert!(!db.path().exists());
    }
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
}

/// An empty file the store may not write to is refused at once, not waited
/// on: SQLite opens it read-only, and will not switch it to write-ahead
/// logging.
#[cfg(unix)]
#[test]
fn a_read_only_file_is_refused_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, db) = scratch();
    fs::write(db.path(), "").unwrap();
    fs::set_permissions(db.path(), fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    if permissions_hold(dir.path()) {
        let (done, outcome) = mpsc::channel();
        let path = db.path().to_owned();
        // A detached thread, so a store that waits for ever fails the test
        // rather than hangs it.
        thread::spawn(move || {
            let _ = done.send(Db::new(path).create(&open_of("fyi"), &unnamed()));
        });
        match outcome.recv_timeout(Duration::from_secs(2)) {
            Ok(Err(StoreError::Store(Error::Sqlite(error)))) => assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ReadOnly)
            ),
            other => panic!("{other:?}"),
        }
    }
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
}

/// A serve or wait that outlives its store follows the file at the path: a
/// store thrown away reads as missing, and one started again in its place is
/// read and written, not the old file the store had open.
#[test]
fn the_store_follows_the_file_at_the_path() {
    let (_dir, db) = scratch();
    let (old, _) = create(&db, "fyi");
    let mut poller = db.poller();
    assert_eq!(poller.poll().unwrap().len(), 1);

    throw_away(db.path());
    assert!(matches!(db.rows(&old), Err(Error::Missing { .. })));
    assert!(matches!(poller.poll(), Err(Error::Missing { .. })));
    let ack = human_event("answer", r#"{"ack": true}"#);
    assert!(matches!(
        db.append(&old, &ack, &unnamed(), None),
        Err(StoreError::Store(Error::Missing { .. }))
    ));

    // Another process starts the store again, and its first case has the key
    // the old one had.
    let other = Db::new(db.path());
    let started = agent_event(
        "open",
        r#"{"kind": "stuck", "urgency": "today", "title": "Started again"}"#,
    );
    let (fresh, _) = other.create(&started, &unnamed()).unwrap();
    drop(other);
    assert_eq!(fresh, old);
    let cases = poller.poll().unwrap();
    let titles: Vec<&str> = cases
        .values()
        .map(|case| case.as_ref().unwrap().title())
        .collect();
    assert_eq!(titles, ["Started again"]);
    db.append(&fresh, &human_event("park", "{}"), &unnamed(), None)
        .unwrap();
    let rows = Db::new(db.path()).rows(&fresh).unwrap().unwrap();
    assert_eq!(Case::fold(&rows).unwrap().state(), State::Parked);
}

/// The store keeps working after the file is replaced again and again: each
/// call finds the file now at the path.
#[test]
fn a_store_replaced_again_and_again_is_followed_each_time() {
    let (_dir, db) = scratch();
    create(&db, "fyi");
    for _ in 0..3 {
        let key = replace_store(db.path());
        let listed = db.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(names(&stored(&db, key)), ["0001-agent-open.json"]);
    }
}
