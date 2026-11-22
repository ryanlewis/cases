//! The store's file: finding it, making it, and one connection to it.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use cases_core::{StoreError, Time};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{Error, is_busy};
use crate::schema::{self, BUSY_TIMEOUT};

/// The cases store in the SQLite file at a path.
///
/// Only creating a case makes the file, and its directory. Until then, and
/// while the file is empty, every read and write finds no store,
/// [`Error::Missing`], and changes nothing. On unix what it makes is for the
/// owner alone: the file `0600`, and each directory `0700`. A file or
/// directory that was there keeps its mode.
///
/// The store keeps one connection to the file, and looks at the file before
/// each read and write. When the file at the path has been removed or
/// replaced, it opens the file now there, so a long-running `cases serve` or
/// `cases wait` follows a store that was thrown away and started again, and
/// never writes to the old file.
///
/// A `Db` is safe to share between threads, which take turns with its
/// connection. Writers with their own `Db`, in this process or another, take
/// turns through SQLite's lock on the file: each write holds it for one fold,
/// check and insert, and a write that waits more than 5 seconds for it fails
/// with [`Error::Busy`] and writes nothing.
///
/// Its writes of cases are the [`Store`](cases_core::Store) trait's. It also
/// writes the listeners, which are not events: [`Db::listen`] and
/// [`Db::set_doing`].
pub struct Db {
    path: PathBuf,
    clock: Box<dyn Fn() -> Option<Time> + Send + Sync>,
    busy_timeout: Duration,
    state: Mutex<State>,
    /// Makes the next commits fail, for a test.
    #[cfg(test)]
    pub(crate) fail_commit: AtomicBool,
}

/// The connection, if one is open.
#[derive(Default)]
struct State {
    open: Option<Open>,
    /// How many connections the store has opened: the generation of the
    /// newest.
    opened: u64,
}

/// A connection, and the file it has open.
struct Open {
    conn: Connection,
    file: FileId,
    generation: u64,
}

/// A file, whatever path names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileId {
    device: u64,
    inode: u64,
}

impl Db {
    /// The store in the SQLite file at `path`. It does not touch the file.
    ///
    /// A relative path is taken from the current directory now.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Db {
        let path = path.into();
        let path = std::path::absolute(&path).unwrap_or(path);
        Db {
            path,
            clock: Box::new(system_time),
            busy_timeout: BUSY_TIMEOUT,
            state: Mutex::default(),
            #[cfg(test)]
            fail_commit: AtomicBool::new(false),
        }
    }

    /// The store with `clock` in place of the system's, such as a clock that
    /// never moves, for a test. The store reads it once a write holds the
    /// lock, and after it reads the listeners.
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Time + Send + Sync + 'static) -> Db {
        self.clock = Box::new(move || Some(clock()));
        self
    }

    /// The store with a shorter wait for another writer's lock, for a test.
    #[cfg(test)]
    pub(crate) fn with_busy_timeout(mut self, busy_timeout: Duration) -> Db {
        self.busy_timeout = busy_timeout;
        self
    }

    /// Runs `f` with the store's connection and its generation, for a test.
    #[cfg(test)]
    pub(crate) fn with_connection<T>(&self, f: impl FnOnce(&Connection, u64) -> T) -> T {
        let mut state = self.lock();
        let open = self.connect(&mut state, false).unwrap();
        f(&open.conn, open.generation)
    }

    /// Runs `f` with the store's connection to change, for a test.
    #[cfg(test)]
    pub(crate) fn with_connection_mut<T>(&self, f: impl FnOnce(&mut Connection) -> T) -> T {
        let mut state = self.lock();
        let open = self.connect(&mut state, false).unwrap();
        f(&mut open.conn)
    }

    /// Where the store is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The time for a write that holds the lock, or for a read of the
    /// listeners that is done.
    pub(crate) fn now(&self) -> Result<Time, Error> {
        (self.clock)().ok_or(Error::Clock)
    }

    /// Runs `read` in a read transaction, which sees one moment of the
    /// store, and gives it the generation of the connection. A missing store
    /// is [`Error::Missing`], and nothing is made.
    pub(crate) fn read<T>(
        &self,
        read: impl FnOnce(&Transaction<'_>, u64) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut state = self.lock();
        let run = || {
            let open = self.connect(&mut state, false)?;
            let tx = open.conn.transaction()?;
            schema::check_version(&tx, &self.path)?;
            let value = read(&tx, open.generation)?;
            tx.commit()?;
            Ok(value)
        };
        run().map_err(|error| self.busy(error))
    }

    /// Runs `write` in a write transaction and commits only when it
    /// succeeds. The transaction begins immediate: it takes the store's lock
    /// before its first read, so writers, in this process or another, run
    /// one at a time, and each reads the case as the writer before it left
    /// it. With `create`, a missing store is made; without it, it is
    /// [`Error::Missing`]. A write of a case fails with a [`StoreError`], and
    /// any other write, such as a listener's, with an [`Error`].
    pub(crate) fn write<T, E: Failure>(
        &self,
        create: bool,
        write: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut state = self.lock();
        let run = || {
            let open = self.connect(&mut state, create)?;
            let tx = open
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(Error::from)?;
            schema::check_version(&tx, &self.path)?;
            let value = write(&tx)?;
            #[cfg(test)]
            if self.fail_commit.load(Ordering::SeqCst) {
                // Dropped, the transaction rolls back, as after a failed
                // commit.
                return Err(Error::from(io::Error::other("disk on fire")).into());
            }
            tx.commit().map_err(Error::from)?;
            Ok(value)
        };
        run().map_err(|error: E| error.busy(self))
    }

    /// `error`, or [`Error::Busy`] when SQLite gave up waiting for the lock.
    fn busy(&self, error: Error) -> Error {
        match error {
            Error::Sqlite(error) if is_busy(&error) => Error::Busy {
                path: self.path.clone(),
            },
            error => error,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A thread that panicked with the lock left no transaction open: the
        // transaction rolls back as it is dropped.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The connection to the file now at the path. A connection to a file
    /// that is no longer there is closed, and the file there is opened; with
    /// `create`, it is made, with its directory and the schema, when it is
    /// missing or empty.
    fn connect<'s>(&self, state: &'s mut State, create: bool) -> Result<&'s mut Open, Error> {
        let found = file_at(&self.path)?;
        let open = match state.open.take() {
            Some(open) if Some(open.file) == found => open,
            stale => {
                // Closing a connection to a file that was removed does not
                // delete the -wal of a file now at the path: SQLite sees that
                // its file has moved.
                drop(stale);
                let (conn, file) = self.open(found, create)?;
                state.opened += 1;
                Open {
                    conn,
                    file,
                    generation: state.opened,
                }
            }
        };
        Ok(state.open.insert(open))
    }

    /// Opens a connection to the file at the path, found as `found`.
    fn open(&self, found: Option<FileId>, create: bool) -> Result<(Connection, FileId), Error> {
        let missing = || Error::Missing {
            path: self.path.clone(),
        };
        if found.is_none() {
            if !create {
                return Err(missing());
            }
            check_leftovers(&self.path)?;
            make_file(&self.path)?;
        }
        // The connection is kept only when the file at the path is the same
        // before and after it connects. Otherwise it may be on a file that is
        // no longer there, while later looks at the path find the file it
        // recorded; so it is closed and opened again.
        for _ in 0..3 {
            let before = file_at(&self.path)?.ok_or_else(missing)?;
            let mut conn = connect_to(&self.path, self.busy_timeout)?;
            if !schema::setup(&mut conn, &self.path, create, self.busy_timeout)? {
                return Err(missing());
            }
            if file_at(&self.path)? == Some(before) {
                return Ok((conn, before));
            }
        }
        Err(Error::Replaced {
            path: self.path.clone(),
        })
    }
}

/// How a write fails: with the store's [`Error`], which a write of a case
/// wraps in a [`StoreError`] beside its refusals.
pub(crate) trait Failure: From<Error> {
    /// The failure, or [`Error::Busy`] in place of SQLite giving up waiting
    /// for the lock.
    fn busy(self, db: &Db) -> Self;
}

impl Failure for Error {
    fn busy(self, db: &Db) -> Error {
        db.busy(self)
    }
}

impl Failure for StoreError<Error> {
    fn busy(self, db: &Db) -> StoreError<Error> {
        match self {
            StoreError::Store(error) => StoreError::Store(db.busy(error)),
            error => error,
        }
    }
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The system's clock.
fn system_time() -> Option<Time> {
    let since = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    Time::from_unix(i64::try_from(since.as_secs()).ok()?, since.subsec_nanos())
}

/// The file at `path`, or `None` when there is none.
fn file_at(path: &Path) -> Result<Option<FileId>, Error> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => Err(Error::Directory {
            path: path.to_owned(),
        }),
        Ok(meta) => Ok(Some(FileId::of(&meta))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl FileId {
    /// The file `meta` is of. Off unix, the store cannot tell one file from
    /// another, and keeps its connection while the path names a file.
    fn of(meta: &fs::Metadata) -> FileId {
        #[cfg(not(unix))]
        let _ = meta;
        FileId {
            #[cfg(unix)]
            device: std::os::unix::fs::MetadataExt::dev(meta),
            #[cfg(unix)]
            inode: std::os::unix::fs::MetadataExt::ino(meta),
            #[cfg(not(unix))]
            device: 0,
            #[cfg(not(unix))]
            inode: 0,
        }
    }
}

/// Refuses to make a store at `path` while a `-wal` or `-shm` file of a store
/// that was there is still beside it. Those are left when the store was
/// removed on its own while a process, such as `cases serve`, had it open. A
/// new store would pair with them by name, and share them with that
/// process, and its writes could be lost.
fn check_leftovers(path: &Path) -> Result<(), Error> {
    for suffix in ["-wal", "-shm"] {
        let mut leftover = path.as_os_str().to_owned();
        leftover.push(suffix);
        let leftover = PathBuf::from(leftover);
        if !leftover.try_exists()? {
            continue;
        }
        // Another writer may have made the store, and these files with it,
        // since this one found the store missing.
        if path.try_exists()? {
            return Ok(());
        }
        return Err(Error::Leftover {
            path: path.to_owned(),
            leftover,
        });
    }
    Ok(())
}

/// Makes an empty file at `path`, and its directory, for the owner alone:
/// the store holds what agents asked and what the human answered. SQLite
/// gives the `-wal` and `-shm` files it makes beside the store the store's
/// mode. An empty file is an empty SQLite database. A file that is there
/// already is left alone: another writer made it first. So is a directory
/// that is there already.
fn make_file(path: &Path) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// A connection to the file at `path`, which must be there: the connection
/// never makes it. `synchronous = FULL` syncs the log on every commit, and
/// `fullfsync` makes that an `F_FULLFSYNC` on macOS, so a write that has
/// returned survives a power loss.
fn connect_to(path: &Path, busy_timeout: Duration) -> Result<Connection, Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE.union(OpenFlags::SQLITE_OPEN_NO_MUTEX);
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(busy_timeout)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA synchronous = FULL;
         PRAGMA fullfsync = ON;",
    )?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use cases_core::Store;

    use super::*;
    use crate::Writer;
    use crate::testing::{count, create, event, note, scratch, sql, unnamed};

    /// A create whose commit fails keeps no case, no actor and no key: the
    /// next create takes the key it did not keep.
    #[test]
    fn a_failed_create_keeps_nothing() {
        let (_dir, db) = scratch();
        create(&db);
        db.fail_commit.store(true, Ordering::SeqCst);
        let open = event(
            "agent",
            "open",
            r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
        );
        let failed = db.create(&open, &Writer::new(Some("new")));
        assert!(
            matches!(&failed, Err(StoreError::Store(Error::Io(e))) if e.to_string() == "disk on fire"),
            "{failed:?}"
        );
        db.fail_commit.store(false, Ordering::SeqCst);
        assert_eq!((count(&db, "cases"), count(&db, "actors")), (1, 1));
        assert_eq!(create(&db).get(), 2);
    }

    /// A write whose commit fails leaves the case as it was and lets the
    /// lock go, and the next write takes the seq it did not keep.
    #[test]
    fn a_failed_commit_writes_nothing() {
        let (_dir, db) = scratch();
        let key = create(&db);
        let human = Writer::new(None);
        let answer = event("human", "answer", r#"{"ack": true}"#);
        db.fail_commit.store(true, Ordering::SeqCst);
        assert!(db.append(&key, &answer, &human, None).is_err());
        db.fail_commit.store(false, Ordering::SeqCst);
        assert_eq!(count(&db, "events"), 1);
        // Another store on the file can write, so the lock was let go.
        let other = Db::new(db.path());
        let answered = other.append(&key, &answer, &human, None).unwrap();
        let last = answered.events().last().unwrap();
        assert_eq!(
            (last.file().as_str(), answered.revision()),
            ("0002-human-answer.json", 2)
        );
    }

    /// A write that waits longer than the busy timeout for another writer's
    /// lock fails, and writes nothing.
    #[test]
    fn a_write_that_waits_too_long_for_the_lock_writes_nothing() {
        let (_dir, db) = scratch();
        let key = create(&db);
        let db = Db::new(db.path()).with_busy_timeout(Duration::from_millis(100));
        let mut other = sql(&db);
        let tx = other
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let started = Instant::now();
        let more = event("agent", "note", r#"{"body": "More."}"#);
        let outcome = db.append(&key, &more, &unnamed(), None);
        assert!(started.elapsed() >= Duration::from_millis(90));
        let Err(StoreError::Store(error @ Error::Busy { .. })) = outcome else {
            panic!("{outcome:?}");
        };
        assert!(error.to_string().contains("is locked"), "{error}");
        tx.rollback().unwrap();
        assert_eq!(count(&db, "events"), 1);
        note(&db, key);
    }

    /// The connection syncs every commit, with `F_FULLFSYNC` on macOS,
    /// checks foreign keys, and waits 5 seconds for another writer.
    #[test]
    fn the_connection_syncs_every_commit_and_waits_for_the_lock() {
        let (_dir, db) = scratch();
        create(&db);
        db.with_connection(|conn, _| {
            let pragma = |name: &str| -> i64 {
                conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
                    .unwrap()
            };
            assert_eq!(pragma("synchronous"), 2);
            assert_eq!(pragma("fullfsync"), 1);
            assert_eq!(pragma("foreign_keys"), 1);
            assert_eq!(pragma("busy_timeout"), 5000);
            let mode: String = conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        });
    }

    /// A writer that makes the store waits for another writer's lock on the
    /// new file only as long as a write waits for the lock, and then fails as
    /// busy.
    #[test]
    fn making_a_store_waits_only_as_long_as_a_write() {
        let (_dir, db) = scratch();
        fs::write(db.path(), "").unwrap();
        let db = Db::new(db.path()).with_busy_timeout(Duration::from_millis(100));
        let mut other = sql(&db);
        std::thread::scope(|scope| {
            let (held, locked) = std::sync::mpsc::channel();
            // The other writer lets go after a second, so a writer that
            // waited for ever would make the store rather than hang.
            scope.spawn(move || {
                let tx = other
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                held.send(()).unwrap();
                std::thread::sleep(Duration::from_secs(1));
                tx.rollback().unwrap();
            });
            locked.recv().unwrap();
            let open = event(
                "agent",
                "open",
                r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
            );
            let outcome = db.create(&open, &unnamed());
            assert!(
                matches!(outcome, Err(StoreError::Store(Error::Busy { .. }))),
                "{outcome:?}"
            );
        });
        create(&db);
    }

    #[test]
    fn a_store_shows_as_its_path() {
        let db = Db::new("/tmp/cases.db");
        assert_eq!(format!("{db:?}"), r#"Db { path: "/tmp/cases.db", .. }"#);
    }

    /// The store keeps its one connection while the file at the path is the
    /// one it has open, and opens the file there in its place when it is
    /// not.
    #[test]
    fn the_connection_is_opened_again_only_for_another_file() {
        let (_dir, db) = scratch();
        create(&db);
        let generation = |db: &Db| db.with_connection(|_, generation| generation);
        assert_eq!(generation(&db), 1);
        db.list().unwrap();
        assert_eq!(generation(&db), 1);
        for suffix in ["", "-wal", "-shm"] {
            let mut file = db.path().as_os_str().to_owned();
            file.push(suffix);
            fs::remove_file(file).unwrap();
        }
        create(&Db::new(db.path()));
        assert_eq!(generation(&db), 2);
        assert_eq!(db.list().unwrap().len(), 1);
    }

    /// Making the file where one opener has already made it, as when two
    /// openers race to create the store, is not an error.
    #[test]
    fn making_the_file_where_it_is_already_there_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        make_file(&path).unwrap();
        make_file(&path).unwrap();
    }
}
