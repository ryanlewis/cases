//! The poller over the store: the core's, reading with the store's own read.

use cases_core::{Changes, Key, Version};

use crate::db::Db;
use crate::error::Error;
use crate::read::changes;

/// Reads the store over and over, as `cases wait` and `cases serve` do, and
/// folds again only the cases that changed: the core's
/// [`Poller`](cases_core::Poller) over a [`Db`].
///
/// A poll reads what changed in one read transaction, so what it gives is
/// one moment of the store. When nothing changed, it gives the cases it has,
/// without reading a row. When the store gives every case, such as a store
/// made again at the path or one a case was put back in by hand, it folds
/// them all. A poll gives [`Error::Missing`] when there is no store, where
/// [`Store::rows_since`](cases_core::Store::rows_since) gives no cases.
pub type Poller<'a> = cases_core::Poller<'a, Db>;

impl Db {
    /// A poller over the store.
    #[must_use]
    pub fn poller(&self) -> Poller<'_> {
        Poller::with_read(self, read)
    }
}

/// What changed since `since`, in one read transaction, or
/// [`Error::Missing`] when there is no store.
fn read(db: &Db, since: Option<&Version>) -> Result<Changes<Key>, Error> {
    db.read(|tx, _| changes(tx, db.path(), since))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use cases_core::{Cases, Store};
    use rusqlite::limits::Limit;

    use super::*;
    use crate::key::RowId;
    use crate::testing::{create, note, scratch, sql};

    thread_local! {
        /// How many cases the reads of a [`counting`] poller have given,
        /// which is how many it has folded.
        static FOLDS: Cell<usize> = const { Cell::new(0) };
    }

    /// The store's poller, counting in [`FOLDS`] each case it folds.
    fn counting(db: &Db) -> Poller<'_> {
        FOLDS.set(0);
        Poller::with_read(db, |db, since| {
            let changes = read(db, since)?;
            let n = match &changes {
                Changes::Unchanged { .. } => 0,
                Changes::Since { cases, .. } | Changes::All { cases, .. } => cases.len(),
            };
            FOLDS.set(FOLDS.get() + n);
            Ok(changes)
        })
    }

    /// How many cases the [`counting`] poller has folded.
    fn folds() -> usize {
        FOLDS.get()
    }

    /// With nothing written, a poll folds nothing; a write to one case, or a
    /// new case, is folded alone; a case taken out is dropped unfolded.
    #[test]
    fn a_poll_folds_again_only_the_cases_that_changed() {
        let (_dir, db) = scratch();
        let first = create(&db);
        let second = create(&db);
        let mut poller = counting(&db);
        assert_eq!(poller.poll().unwrap().len(), 2);
        assert_eq!(folds(), 2);
        poller.poll().unwrap();
        assert_eq!(folds(), 2, "an unchanged store was folded again");
        note(&db, second);
        let revision = |cases: &Cases, key| cases[&key].as_ref().unwrap().revision();
        assert_eq!(revision(poller.poll().unwrap(), second), 2);
        assert_eq!(folds(), 3);
        let third = create(&db);
        assert_eq!(poller.poll().unwrap().len(), 3);
        assert_eq!(folds(), 4);
        db.delete(&first, None).unwrap();
        let keys: Vec<Key> = poller.poll().unwrap().keys().copied().collect();
        assert_eq!((keys, folds()), (vec![second, third], 4));
    }

    /// A listener's writes are not events: a poll after one folds nothing,
    /// so serve's store version stays, and no inbox refreshes on a
    /// heartbeat.
    #[test]
    fn a_listener_moves_no_poll() {
        let (_dir, db) = scratch();
        create(&db);
        let mut poller = counting(&db);
        poller.poll().unwrap();
        db.listen("w1", "bun-pins", &[], &[], Some("rebasing"))
            .unwrap();
        db.set_doing("bun-pins", "testing").unwrap();
        poller.poll().unwrap();
        assert_eq!(folds(), 1, "a listener's write was folded");
    }

    /// Stores `n` more fyi cases and a note on each of the first `noted`,
    /// in one transaction.
    fn fill(db: &Db, n: usize, noted: &[Key]) {
        let mut conn = sql(db);
        let tx = conn.transaction().unwrap();
        for _ in 0..n {
            tx.execute("INSERT INTO cases (opened_at) VALUES ('')", [])
                .unwrap();
            tx.execute(
                "INSERT INTO events (case_key, seq, author, event, at, data)
                 VALUES (last_insert_rowid(), 1, 'agent', 'open', '',
                 CAST('{\"kind\":\"fyi\",\"urgency\":\"today\",\"title\":\"T\"}' AS BLOB))",
                [],
            )
            .unwrap();
        }
        for key in noted {
            tx.execute(
                "INSERT INTO events (case_key, seq, author, event, at, data)
                 SELECT ?1, max(seq) + 1, 'agent', 'note', '', CAST('{\"body\":\"More.\"}' AS BLOB)
                 FROM events WHERE case_key = ?1",
                [key.sql()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    /// A poll folds only the changed cases, however many changed.
    #[test]
    fn a_poll_folds_only_the_changed_cases_however_many() {
        const MANY: usize = 150;
        let (_dir, db) = scratch();
        create(&db);
        fill(&db, MANY, &[]);
        let mut poller = counting(&db);
        let keys: Vec<Key> = poller.poll().unwrap().keys().copied().collect();
        assert_eq!((keys.len(), folds()), (MANY + 1, MANY + 1));

        fill(&db, 0, &keys[..MANY]);
        let cases = poller.poll().unwrap();
        assert!(cases.values().all(Result::is_ok));
        assert_eq!(cases.len(), MANY + 1);
        assert_eq!(folds(), 2 * MANY + 1, "the changed cases");
    }

    /// A poll that fails forgets the version it read at, and the next poll
    /// reads every case again. The failure is SQLite's: with its limit on
    /// bound parameters lowered to 0, the poll's read of the version fails.
    #[test]
    fn a_poll_after_a_failed_one_reads_every_case() {
        let (_dir, db) = scratch();
        create(&db);
        create(&db);
        let archived = create(&db);
        let mut poller = db.poller();
        assert_eq!(poller.poll().unwrap().len(), 3);

        db.archive(&archived, None).unwrap();
        let added = [create(&db), create(&db)];
        let limit = |n| {
            db.with_connection(|conn, _| {
                conn.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, n)
                    .unwrap()
            })
        };
        let was = limit(0);
        assert!(poller.poll().is_err(), "a poll with no parameter allowed");
        limit(was);

        for key in added {
            db.delete(&key, None).unwrap();
        }
        let mut conn = sql(&db);
        let tx = conn.transaction().unwrap();
        for statement in [
            "INSERT INTO cases (key, opened_at) SELECT key, opened_at FROM archive_cases WHERE key = ?1",
            "INSERT INTO events (change, case_key, seq, author, event, at, data)
             SELECT change, case_key, seq, author, event, at, data FROM archive_events WHERE case_key = ?1",
            "DELETE FROM archive_events WHERE case_key = ?1",
            "DELETE FROM archive_cases WHERE key = ?1",
        ] {
            tx.execute(statement, [archived.sql()]).unwrap();
        }
        tx.commit().unwrap();
        let cases = poller.poll().unwrap();
        assert_eq!(cases.len(), 3);
        assert!(cases.values().all(Result::is_ok));
    }
}
