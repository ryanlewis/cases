//! Runs the store vectors, `spec/vectors/store/`, against a store in a
//! SQLite file of its own for each vector, with a clock the vector sets.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

use std::sync::{Arc, Mutex, PoisonError};

use cases_core::Time;
use cases_store_sqlite::{Db, Error};
use cases_vectors::Harness;
use libtest_mimic::Arguments;
use tempfile::TempDir;

/// A store in a directory of its own, and its clock.
struct Sqlite {
    _dir: TempDir,
    db: Db,
    clock: Arc<Mutex<Time>>,
}

impl Sqlite {
    fn new(start: Time) -> Sqlite {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Mutex::new(start));
        let reads = Arc::clone(&clock);
        let db = Db::new(dir.path().join("cases.db"))
            .with_clock(move || *reads.lock().unwrap_or_else(PoisonError::into_inner));
        Sqlite {
            _dir: dir,
            db,
            clock,
        }
    }
}

impl Harness for Sqlite {
    type Store = Db;

    fn store(&self) -> &Db {
        &self.db
    }

    fn set_clock(&self, now: Time) {
        *self.clock.lock().unwrap_or_else(PoisonError::into_inner) = now;
    }

    fn category(&self, error: &Error) -> Option<&'static str> {
        match error {
            Error::Missing { .. } => Some("missing"),
            Error::Unnamed | Error::Doing(_) => Some("invalid"),
            Error::NotListening { .. } => Some("not-listening"),
            _ => None,
        }
    }

    const LOCAL: bool = true;
}

fn main() {
    let args = Arguments::from_args();
    libtest_mimic::run(&args, cases_vectors::trials(Sqlite::new)).exit();
}
