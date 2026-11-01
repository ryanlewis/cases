//! What can go wrong in the store itself.

use std::fmt;
use std::io;
use std::path::PathBuf;

use cases_core::Key;

use crate::schema::{BUSY_TIMEOUT, SCHEMA_VERSION};

/// What went wrong in the store itself: its file, SQLite or the clock. A
/// write that fails with one stored nothing.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// There is no store at the path: no file, or an empty one. Only
    /// creating a case makes the store.
    Missing {
        /// Where the store would be.
        path: PathBuf,
    },
    /// The path is a directory, not a file.
    Directory {
        /// The directory.
        path: PathBuf,
    },
    /// There is no store at the path, but the `-wal` or `-shm` file of an
    /// earlier store is still beside it. The store was removed while a
    /// process had it open, and a new store would pair with the files and
    /// could lose its writes.
    Leftover {
        /// Where the store would be.
        path: PathBuf,
        /// The earlier store's file.
        leftover: PathBuf,
    },
    /// The store was made by a newer cases: its schema version is higher
    /// than this cases reads.
    Newer {
        /// The store.
        path: PathBuf,
        /// The store's schema version.
        version: i64,
    },
    /// The file is an SQLite database, but not a cases store.
    NotAStore {
        /// The file.
        path: PathBuf,
    },
    /// The store holds what no cases writes, such as a schema version below
    /// 1.
    Damaged {
        /// The store.
        path: PathBuf,
        /// What is wrong with it.
        what: String,
    },
    /// The file at the path was replaced each time the store opened it.
    Replaced {
        /// The store.
        path: PathBuf,
    },
    /// Another writer held the store's lock for longer than a write waits
    /// for it, 5 seconds. Nothing was written, and the write can be made
    /// again.
    Busy {
        /// The store.
        path: PathBuf,
    },
    /// The case is not archived: the archive holds its key already, as it
    /// can after a case is put back from the archive by hand.
    Archived {
        /// The case.
        key: Key,
    },
    /// A doing line that is not one: it has a line break or another
    /// control character, or is too long.
    Doing(String),
    /// A worker that is not listening was given a doing line.
    NotListening {
        /// The worker.
        worker: String,
    },
    /// A listener was given a blank worker, which no message goes to.
    Unnamed,
    /// The clock gave a time outside the years 1970 to 9999.
    Clock,
    /// SQLite failed.
    Sqlite(rusqlite::Error),
    /// The file system failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Missing { path } => write!(f, "no store at {}", path.display()),
            Error::Directory { path } => {
                write!(f, "store {} is a directory, not a file", path.display())
            }
            Error::Leftover { path, leftover } => {
                let path = path.display();
                write!(
                    f,
                    "no store at {path}, but {} from an earlier store is still there: \
                     stop cases serve and any cases wait, then delete {path}-wal and \
                     {path}-shm, or put the store back",
                    leftover.display()
                )
            }
            Error::Newer { path, version } => write!(
                f,
                "store {} has schema version {version}, newer than this cases reads \
                 ({SCHEMA_VERSION}): update cases on this machine",
                path.display()
            ),
            Error::NotAStore { path } => write!(
                f,
                "store {} is not a cases store: it has tables but no meta table",
                path.display()
            ),
            Error::Damaged { path, what } => {
                write!(f, "store {} is damaged: {what}", path.display())
            }
            Error::Replaced { path } => write!(
                f,
                "store {} was replaced three times while it was being opened",
                path.display()
            ),
            Error::Busy { path } => write!(
                f,
                "store {} is locked: another writer held it for more than {} s, and \
                 nothing was written",
                path.display(),
                BUSY_TIMEOUT.as_secs()
            ),
            Error::Archived { key } => write!(f, "case {key} is in the archive already"),
            Error::Doing(why) => f.write_str(why),
            Error::Unnamed => f.write_str("a listener needs a worker name that is not blank"),
            Error::NotListening { worker } => write!(
                f,
                "{worker:?} is not listening: a doing line needs a running \
                 cases wait --pickup --worker {worker:?}"
            ),
            Error::Clock => f.write_str("the clock is outside the years 1970 to 9999"),
            Error::Sqlite(error) => error.fmt(f),
            Error::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sqlite(error) => Some(error),
            Error::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Error {
        Error::Sqlite(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        Error::Io(error)
    }
}

/// Whether SQLite gave up waiting for another connection's lock.
pub(crate) fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn an_error_gives_the_error_of_sqlite_or_the_file_system_as_its_source() {
        let sqlite = Error::Sqlite(rusqlite::Error::InvalidQuery);
        assert_eq!(sqlite.source().unwrap().to_string(), sqlite.to_string());
        let io = Error::Io(io::Error::other("disk on fire"));
        assert_eq!(io.source().unwrap().to_string(), "disk on fire");
        assert!(Error::Clock.source().is_none());
    }

    /// Variants the store cannot be made to give from a test, or only by
    /// racing another process, are built here. Each message names the one
    /// thing a reader needs; the wording is not pinned.
    #[test]
    fn a_message_names_what_the_reader_needs() {
        let path = PathBuf::from("/data/cases.db");
        let replaced = Error::Replaced { path: path.clone() };
        assert!(replaced.to_string().contains("/data/cases.db"));
        assert!(replaced.source().is_none());
        assert!(Error::Unnamed.to_string().contains("worker"));
        assert!(Error::Clock.to_string().contains("clock"));
        let io = Error::from(io::Error::other("disk on fire"));
        assert_eq!(io.to_string(), "disk on fire");
        let sqlite = Error::from(rusqlite::Error::InvalidQuery);
        assert!(matches!(sqlite, Error::Sqlite(_)));
    }
}
