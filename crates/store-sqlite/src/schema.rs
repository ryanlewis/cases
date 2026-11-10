//! The store's tables, and the checks that a file holds them.

use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Error, is_busy};
use crate::listen::LISTENERS;

/// The version of [`SCHEMA`]. A store with a higher version was made by a
/// newer cases, and is refused.
///
/// The version moves when an older cases would read or write the store
/// wrongly. A table an older cases never reads, such as the tables of
/// listening, is added without a new version: a new store has it from the start, and a
/// store an older cases made gets it from the first write that needs it.
/// An older cases leaves such a table alone.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// The store's tables.
///
/// `cases.key` is a case's key, which its public id encodes. Being
/// AUTOINCREMENT, it is never given twice, even after the case with the
/// highest key is archived or deleted. `events.change` numbers every row
/// across the store in the same way, so it only goes up, and a reader can
/// ask for the rows stored since the highest it has seen. `(case_key, seq)`
/// is unique, and `data` holds the record's bytes as the writer made them.
///
/// `actors` names who writes events: one actor for each kind and name, and
/// one unnamed actor for each kind. Events hold only an actor's id, and
/// nothing refers to the table, so a row can be deleted and the events keep
/// the bare id.
///
/// The archive tables have the columns of `cases` and `events`, and hold the
/// cases an archive moves out of sight, with their keys and changes.
///
/// The tables of listening, [`LISTENERS`], and the log of removals,
/// [`REMOVALS`], are made with them. `meta.store_id` is 128 random bits in
/// base64url, made with the store, which tells it from any other store,
/// such as one made again at the same path.
const SCHEMA: &str = "
CREATE TABLE meta (
    schema_version INTEGER NOT NULL,
    store_id       TEXT
) STRICT;

CREATE TABLE actors (
    id      TEXT NOT NULL PRIMARY KEY,
    kind    TEXT NOT NULL,
    name    TEXT,
    created TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX actors_named ON actors (kind, name) WHERE name IS NOT NULL;
CREATE UNIQUE INDEX actors_unnamed ON actors (kind) WHERE name IS NULL;

CREATE TABLE cases (
    key       INTEGER PRIMARY KEY AUTOINCREMENT CHECK (key >= 1),
    opened_at TEXT NOT NULL
) STRICT;

CREATE TABLE events (
    change   INTEGER PRIMARY KEY AUTOINCREMENT,
    case_key INTEGER NOT NULL REFERENCES cases (key),
    seq      INTEGER NOT NULL CHECK (seq >= 1),
    author   TEXT NOT NULL,
    event    TEXT NOT NULL,
    at       TEXT NOT NULL,
    data     BLOB NOT NULL,
    UNIQUE (case_key, seq)
) STRICT;

CREATE TABLE archive_cases (
    key         INTEGER PRIMARY KEY CHECK (key >= 1),
    opened_at   TEXT NOT NULL,
    archived_at TEXT NOT NULL
) STRICT;

CREATE TABLE archive_events (
    change   INTEGER PRIMARY KEY,
    case_key INTEGER NOT NULL REFERENCES archive_cases (key),
    seq      INTEGER NOT NULL CHECK (seq >= 1),
    author   TEXT NOT NULL,
    event    TEXT NOT NULL,
    at       TEXT NOT NULL,
    data     BLOB NOT NULL,
    UNIQUE (case_key, seq)
) STRICT;
";

/// The log of removals: each case taken out of `cases`, by an archive, a
/// delete, or a hand edit, with a change from the events' own counter, so
/// that one counter moves on every event and every removal, and the time
/// by SQLite's clock; and the change and time of each event taken out and
/// not kept in the archive, in `gone`, so that a version's row can still be
/// found once a delete has taken it. Triggers keep them, so a cases from before the log
/// keeps it too, and the schema version stays: an older cases never reads
/// the log, and its archives and deletes are logged all the same. A store
/// an older cases made gets the log, and a store id, when this cases first
/// opens it ([`upgrade`]).
pub(crate) const REMOVALS: &str = "
CREATE TABLE IF NOT EXISTS removed (
    change INTEGER PRIMARY KEY,
    key    INTEGER NOT NULL,
    at     TEXT NOT NULL
) STRICT;

CREATE TRIGGER IF NOT EXISTS removed_log AFTER DELETE ON cases BEGIN
    UPDATE sqlite_sequence SET seq = seq + 1 WHERE name = 'events';
    INSERT INTO removed (change, key, at) VALUES (
        (SELECT seq FROM sqlite_sequence WHERE name = 'events'),
        OLD.key,
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    );
END;

CREATE TABLE IF NOT EXISTS gone (
    change INTEGER PRIMARY KEY,
    at     TEXT NOT NULL
) STRICT;

CREATE TRIGGER IF NOT EXISTS gone_log AFTER DELETE ON events
WHEN NOT EXISTS (SELECT 1 FROM archive_events WHERE change = OLD.change) BEGIN
    INSERT OR IGNORE INTO gone (change, at) VALUES (OLD.change, OLD.at);
END;
";

/// How long a connection waits for another writer's lock before it fails
/// with `SQLITE_BUSY`. A write holds the lock for one fold, check and insert.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Checks the schema in the file and, with `create`, makes it in an empty
/// file. Says whether the file holds a cases store: an empty one does not.
pub(crate) fn setup(
    conn: &mut Connection,
    path: &Path,
    create: bool,
    busy_timeout: Duration,
) -> Result<bool, Error> {
    if check_schema(conn, path)? {
        upgrade_or_wait(conn, path)?;
        return Ok(true);
    }
    if !create {
        return Ok(false);
    }
    use_wal(conn, path, busy_timeout)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another writer may have made the schema while this one waited.
    if check_schema(&tx, path)? {
        tx.commit()?;
        upgrade_or_wait(conn, path)?;
        return Ok(true);
    }
    tx.execute_batch(SCHEMA)?;
    tx.execute_batch(LISTENERS)?;
    tx.execute_batch(REMOVALS)?;
    tx.execute(
        "INSERT INTO meta (schema_version, store_id) VALUES (?1, ?2)",
        rusqlite::params![SCHEMA_VERSION, crate::actors::new_id()?],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Upgrades the store, or, when it cannot be written now, because the file
/// is read-only or another writer holds the lock past the busy timeout,
/// leaves it for the next open. Reads go on meanwhile: the store's version
/// then has no id and no tag, and `rows_since` gives every case.
fn upgrade_or_wait(conn: &mut Connection, path: &Path) -> Result<(), Error> {
    match upgrade(conn, path) {
        Err(Error::Sqlite(error))
            if is_busy(&error)
                || error.sqlite_error_code() == Some(rusqlite::ErrorCode::ReadOnly) =>
        {
            Ok(())
        }
        other => other,
    }
}

/// Gives a store an older cases made what this one adds without a new
/// schema version: a store id in `meta`, and the log of removals. It runs
/// once, the first time this cases opens the store, in one write
/// transaction, and leaves the store as it was when it fails. An older
/// cases goes on reading and writing the store after, as before.
fn upgrade(conn: &mut Connection, path: &Path) -> Result<(), Error> {
    if upgraded(conn)? {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another cases may have upgraded it while this one waited.
    if !upgraded(&tx)? {
        check_version(&tx, path)?;
        if !has_store_id(&tx)? {
            tx.execute_batch("ALTER TABLE meta ADD COLUMN store_id TEXT")?;
        }
        tx.execute(
            "UPDATE meta SET store_id = ?1 WHERE store_id IS NULL",
            [crate::actors::new_id()?],
        )?;
        tx.execute_batch(REMOVALS)?;
    }
    tx.commit()?;
    Ok(())
}

/// Whether the store has its id and the log of removals.
pub(crate) fn upgraded(conn: &Connection) -> Result<bool, Error> {
    let logged: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE (type, name) IN
         (VALUES ('table', 'removed'), ('trigger', 'removed_log'),
                 ('table', 'gone'), ('trigger', 'gone_log'))",
        [],
        |row| row.get(0),
    )?;
    if logged < 4 || !has_store_id(conn)? {
        return Ok(false);
    }
    let id: Option<String> =
        conn.query_row("SELECT max(store_id) FROM meta", [], |row| row.get(0))?;
    Ok(id.is_some())
}

/// Whether `meta` has the column for the store id.
fn has_store_id(conn: &Connection) -> Result<bool, Error> {
    Ok(conn.query_row(
        "SELECT count(*) FROM pragma_table_info('meta') WHERE name = 'store_id'",
        [],
        |row| row.get(0),
    )?)
}

/// Says whether the file holds the cases schema at a version this cases
/// reads. A file with no tables is new, and holds no store. One with tables
/// but no meta table is not a cases store.
fn check_schema(conn: &Connection, path: &Path) -> Result<bool, Error> {
    let (tables, meta): (i64, bool) = conn.query_row(
        r"SELECT count(*), coalesce(sum(name = 'meta'), 0) FROM sqlite_master
          WHERE type = 'table' AND name NOT LIKE 'sqlite\_%' ESCAPE '\'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if tables == 0 {
        return Ok(false);
    }
    if !meta {
        return Err(Error::NotAStore {
            path: path.to_owned(),
        });
    }
    check_version(conn, path)?;
    Ok(true)
}

/// Refuses a store whose schema version this cases does not read. Every read
/// and write checks it again in its transaction, so a `cases serve` or
/// `cases wait` that is running when a newer cases changes the schema stops
/// there.
pub(crate) fn check_version(conn: &Connection, path: &Path) -> Result<(), Error> {
    let version: Option<i64> =
        conn.query_row("SELECT max(schema_version) FROM meta", [], |row| row.get(0))?;
    let damaged = |what: String| Error::Damaged {
        path: path.to_owned(),
        what,
    };
    match version {
        None => Err(damaged("its meta table has no schema version".into())),
        Some(version) if version > SCHEMA_VERSION => Err(Error::Newer {
            path: path.to_owned(),
            version,
        }),
        Some(version) if version < 1 => Err(damaged(format!("schema version {version}"))),
        Some(_) => Ok(()),
    }
}

/// Switches the file to write-ahead logging, which lets reads go on while a
/// write happens. The mode is kept in the file, so it is set once, before the
/// first table.
///
/// The switch turns a read lock into a write lock, which SQLite refuses at
/// once with `SQLITE_BUSY` rather than wait out the busy timeout. So a writer
/// that races another to make the store tries again until the timeout. Once
/// the other writer has switched, the mode is set.
fn use_wal(conn: &Connection, path: &Path, busy_timeout: Duration) -> Result<(), Error> {
    let deadline = Instant::now() + busy_timeout;
    loop {
        match conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        {
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
            Ok(mode) => {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "store {}: cannot use write-ahead logging: the journal mode is {mode}",
                        path.display()
                    ),
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Changes, Store};
    use rusqlite::Connection;

    use crate::Db;
    use crate::testing::{create, note, scratch, sql};

    /// The schema of a store the cases of v0.0.1-alpha.1 made: no store id,
    /// and no log of removals.
    const V1: &str = "
CREATE TABLE meta (
    schema_version INTEGER NOT NULL
) STRICT;

CREATE TABLE actors (
    id      TEXT NOT NULL PRIMARY KEY,
    kind    TEXT NOT NULL,
    name    TEXT,
    created TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX actors_named ON actors (kind, name) WHERE name IS NOT NULL;
CREATE UNIQUE INDEX actors_unnamed ON actors (kind) WHERE name IS NULL;

CREATE TABLE cases (
    key       INTEGER PRIMARY KEY AUTOINCREMENT CHECK (key >= 1),
    opened_at TEXT NOT NULL
) STRICT;

CREATE TABLE events (
    change   INTEGER PRIMARY KEY AUTOINCREMENT,
    case_key INTEGER NOT NULL REFERENCES cases (key),
    seq      INTEGER NOT NULL CHECK (seq >= 1),
    author   TEXT NOT NULL,
    event    TEXT NOT NULL,
    at       TEXT NOT NULL,
    data     BLOB NOT NULL,
    UNIQUE (case_key, seq)
) STRICT;

CREATE TABLE archive_cases (
    key         INTEGER PRIMARY KEY CHECK (key >= 1),
    opened_at   TEXT NOT NULL,
    archived_at TEXT NOT NULL
) STRICT;

CREATE TABLE archive_events (
    change   INTEGER PRIMARY KEY,
    case_key INTEGER NOT NULL REFERENCES archive_cases (key),
    seq      INTEGER NOT NULL CHECK (seq >= 1),
    author   TEXT NOT NULL,
    event    TEXT NOT NULL,
    at       TEXT NOT NULL,
    data     BLOB NOT NULL,
    UNIQUE (case_key, seq)
) STRICT;
";

    /// A store as an older cases left it: its tables, with a case, a case
    /// archived and a listener, then the log of removals and the store id
    /// taken out again.
    fn older(db: &Db) -> Connection {
        let one = create(db);
        create(db);
        note(db, one);
        db.archive(&one, None).unwrap();
        db.listen("w1", "lead", &[], &[], Some("reading")).unwrap();
        let conn = sql(db);
        conn.execute_batch(
            "DROP TRIGGER removed_log; DROP TABLE removed;
             DROP TRIGGER gone_log; DROP TABLE gone;
             ALTER TABLE meta DROP COLUMN store_id;",
        )
        .unwrap();
        conn
    }

    /// The schema of the store's tables, as SQLite keeps it, without the
    /// ones this cases adds.
    fn tables(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare(
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL
                 AND name NOT IN ('removed', 'removed_log', 'gone', 'gone_log', 'meta',
                                  'sqlite_sequence')
                 ORDER BY name",
            )
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn a_store_an_older_cases_made_has_that_cases_s_tables() {
        // The tables this cases makes, but for meta, are the older cases's:
        // a store it makes is one an older cases reads.
        let (_dir, db) = scratch();
        let conn = older(&db);
        let old = Connection::open_in_memory().unwrap();
        old.execute_batch(V1).unwrap();
        old.execute_batch(crate::listen::LISTENERS).unwrap();
        assert_eq!(tables(&conn), tables(&old));
    }

    /// The first open of an older cases's store gives it an id and the log,
    /// keeps its schema version and everything in it, and gives changes
    /// from then on.
    #[test]
    fn an_older_store_is_upgraded_when_first_opened() {
        let (dir, db) = scratch();
        let conn = older(&db);
        let before: Vec<(i64, String)> = conn
            .prepare("SELECT change, at FROM events ORDER BY change")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(conn);
        drop(db);

        let db = Db::new(dir.path().join("cases.db"));
        let cases = db.list().unwrap();
        assert_eq!(cases.len(), 1);
        let conn = sql(&db);
        let (version, id): (i64, String) = conn
            .query_row("SELECT schema_version, store_id FROM meta", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((version, id.len()), (1, 22));
        let after: Vec<(i64, String)> = conn
            .prepare("SELECT change, at FROM events ORDER BY change")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(after, before, "the events are as they were");
        assert_eq!(db.listeners().unwrap()[0].doing.as_deref(), Some("reading"));
        assert_eq!(
            db.version().unwrap().as_str().split('.').next(),
            Some(id.as_str())
        );

        // Opened again, it is as it was: the same id.
        drop(conn);
        let again = Db::new(dir.path().join("cases.db"));
        let then = again.version().unwrap();
        assert_eq!(then, db.version().unwrap());

        // An older cases's delete, as it writes one, is logged all the same.
        let key = cases[0].0;
        let conn = sql(&again);
        conn.execute_batch(&format!(
            "DELETE FROM events WHERE case_key = {k}; DELETE FROM cases WHERE key = {k};",
            k = key.get()
        ))
        .unwrap();
        let Changes::Since { removed, cases, .. } = again.rows_since(Some(&then)).unwrap() else {
            panic!("not what changed");
        };
        assert_eq!((removed, cases.len()), (vec![key], 0));
    }

    /// Several cases opening an older store at once: one upgrades it, the
    /// others wait for it or find it done, and none fails. There is one
    /// store id after.
    #[test]
    fn an_older_store_opened_by_several_at_once_is_upgraded_once() {
        use std::sync::{Arc, Barrier};

        for _ in 0..5 {
            let (dir, db) = scratch();
            let conn = older(&db);
            drop(conn);
            drop(db);
            let path = dir.path().join("cases.db");
            let barrier = Arc::new(Barrier::new(8));
            let ids: Vec<String> = (0..8)
                .map(|_| {
                    let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                    std::thread::spawn(move || {
                        let db = Db::new(path);
                        barrier.wait();
                        let version = db.version().unwrap();
                        assert_eq!(db.list().unwrap().len(), 1);
                        version.as_str().split('.').next().unwrap().to_owned()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            assert!(
                ids.iter().all(|id| *id == ids[0] && id.len() == 22),
                "{ids:?}"
            );
            let rows: i64 = sql(&Db::new(&path))
                .query_row("SELECT count(*) FROM meta", [], |row| row.get(0))
                .unwrap();
            assert_eq!(rows, 1);
        }
    }

    /// A store upgraded already is not written to when it is opened: it
    /// opens, and reads, while another connection holds the write lock.
    #[test]
    fn an_upgraded_store_is_opened_without_a_write() {
        let (dir, db) = scratch();
        create(&db);
        drop(db);
        let mut lock = sql(&Db::new(dir.path().join("cases.db")));
        let tx = lock
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let db = Db::new(dir.path().join("cases.db"))
            .with_busy_timeout(std::time::Duration::from_millis(50));
        assert_eq!(db.list().unwrap().len(), 1);
        drop(tx);
    }

    /// A store with only part of what the upgrade adds gets the rest.
    #[test]
    fn a_store_with_part_of_the_upgrade_gets_the_rest() {
        for drop_it in [
            "UPDATE meta SET store_id = NULL",
            "DROP TRIGGER gone_log; DROP TABLE gone",
            "DROP TRIGGER removed_log",
        ] {
            let (dir, db) = scratch();
            create(&db);
            sql(&db).execute_batch(drop_it).unwrap();
            drop(db);
            let db = Db::new(dir.path().join("cases.db"));
            db.list().unwrap();
            let conn = sql(&db);
            let (id, logs): (Option<String>, i64) = conn
                .query_row(
                    "SELECT (SELECT store_id FROM meta), (SELECT count(*) FROM sqlite_master
                     WHERE name IN ('removed', 'removed_log', 'gone', 'gone_log'))",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!((id.map(|id| id.len()), logs), (Some(22), 4), "{drop_it}");
        }
    }

    /// An older store that cannot be written when it is opened, read-only
    /// or locked by another writer past the busy timeout, is still read:
    /// with no store id in its version, and every case from `rows_since`. The
    /// next open that can write upgrades it.
    #[test]
    fn an_older_store_that_cannot_be_written_now_is_read_without_the_upgrade() {
        use std::os::unix::fs::PermissionsExt as _;

        let (dir, db) = scratch();
        drop(older(&db));
        drop(db);
        let path = dir.path().join("cases.db");

        // Locked by another writer.
        let mut lock = Connection::open(&path).unwrap();
        let tx = lock
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let db = Db::new(&path).with_busy_timeout(std::time::Duration::from_millis(50));
        assert_eq!(db.list().unwrap().len(), 1);
        let version = db.version().unwrap();
        assert!(version.as_str().starts_with('.'), "{version:?}");
        assert!(matches!(
            db.rows_since(Some(&version)).unwrap(),
            Changes::Unchanged { .. }
        ));
        drop(tx);
        drop(lock);

        // Written through the open that read it, which does not upgrade it
        // again: what changed is every case, not a read of the log of
        // removals the store does not have yet.
        let key = db.list().unwrap()[0].0;
        note(&db, key);
        let changes = db.rows_since(Some(&version)).unwrap();
        let Changes::All {
            version: now,
            cases,
        } = changes
        else {
            panic!("not every case: {changes:?}");
        };
        assert!(now.as_str().starts_with('.'), "{now:?}");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].0, key);
        assert_eq!(
            cases[0].1.last().map(|row| row.event.as_str()),
            Some("note")
        );
        drop(db);

        // No longer locked: the next open upgrades it.
        let db = Db::new(&path);
        db.list().unwrap();
        let id = db.version().unwrap();
        assert_eq!(id.as_str().split('.').next().map(str::len), Some(22));
        drop(db);

        // Read-only: SQLite opens it for reading, and it reads, with no id.
        let (dir, db) = scratch();
        drop(older(&db));
        drop(db);
        let path = dir.path().join("cases.db");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let db = Db::new(&path);
        let (cases, version) = (db.list(), db.version());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(cases.unwrap().len(), 1);
        assert!(version.unwrap().as_str().starts_with('.'));
    }

    /// An upgrade that fails leaves the store as it was.
    #[test]
    fn an_upgrade_that_fails_changes_nothing() {
        let (dir, db) = scratch();
        let conn = older(&db);
        drop(db);
        // A trigger that refuses the store id: the upgrade fails after it
        // has added its column, and rolls that back.
        conn.execute_batch(
            "CREATE TRIGGER no_id BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        assert!(db.list().is_err());
        let has_id: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('meta') WHERE name = 'store_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let removed: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('removed', 'removed_log')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((has_id, removed), (0, 0));
    }
}
