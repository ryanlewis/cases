//! Listening: the agent sessions whose `cases wait` takes messages, one row
//! for each wait that listens, and what each worker says it is doing. They
//! are not events and not part of the decision log: a row is kept up to date
//! in place, and lapses when its wait stops refreshing it.

use std::collections::{BTreeMap, BTreeSet};

use cases_core::{Listening, Time, Wait, heard_in_time};
use rusqlite::types::ValueRef;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::db::Db;
use crate::error::Error;

/// The tables of listening: `waits`, one row for each worker of each wait
/// that listens, by the id the wait made when it started, with the labels
/// and kinds it waits on; and `doing`, what each worker listening says it is
/// doing. A store an older cases made lacks them, and gets them from the
/// first write that needs them. An older cases never reads them, so the
/// schema version stays as it was. An older cases kept one row for each
/// worker, with the labels of all its waits, in a table named `listeners`,
/// which an older `cases wait` or `serve` still running keeps using. This
/// cases reads a live row of it as one wait of a worker with no wait of its
/// own, and sets such a worker's doing line on that row, but never makes,
/// adds to or deletes from it.
pub(crate) const LISTENERS: &str = "
CREATE TABLE IF NOT EXISTS waits (
    id     TEXT NOT NULL,
    worker TEXT NOT NULL,
    labels TEXT NOT NULL,
    kinds  TEXT NOT NULL,
    since  TEXT NOT NULL,
    heard  TEXT NOT NULL,
    PRIMARY KEY (id, worker)
) STRICT;

CREATE TABLE IF NOT EXISTS doing (
    worker   TEXT NOT NULL PRIMARY KEY,
    doing    TEXT NOT NULL,
    doing_at TEXT NOT NULL
) STRICT;
";

/// The columns of a wait's row, in the order [`read_wait`] reads them.
const COLUMNS: &str = "id, worker, labels, kinds, since, heard";

/// A new wait's id: 128 random bits from the operating system, in
/// base64url, as an actor's id is.
///
/// # Errors
///
/// [`Error::Io`] when the operating system gives no random bits.
pub fn new_wait() -> Result<String, Error> {
    crate::actors::new_id()
}

/// `text` as a doing line, as the core's [`cases_core::doing_line`] reads
/// it: trimmed, or `None` when that leaves nothing, which clears the line.
///
/// # Errors
///
/// [`Error::Doing`], with the core's reason, when it is not a doing line.
pub fn doing_line(text: &str) -> Result<Option<String>, Error> {
    cases_core::doing_line(text).map_err(|error| Error::Doing(error.to_string()))
}

/// A worker's doing line and when it was set, as the `doing` table keeps it.
type Doing = (Option<String>, Option<Time>);

impl Db {
    /// Records that the wait `wait`, by the id it made when it started, is
    /// listening for `worker` on `labels` and `kinds`, now: as a listening
    /// wait does when it starts and at each [`HEARTBEAT`](cases_core::HEARTBEAT). A wait still
    /// listening keeps when it began; a new one for a worker that another
    /// wait listens for takes that one's, so the worker is listed as
    /// listening since then. A blank label, which no message goes to, is left
    /// out. With `doing`, it also sets the worker's doing line, as
    /// [`Db::set_doing`] does. A heartbeat passes no `doing`, which keeps the
    /// line as it is: passing the line the wait started with again would
    /// undo any [`Db::set_doing`] since. A worker none of whose waits is
    /// listening has no line, so one that listens again starts afresh.
    ///
    /// Rows that have lapsed are deleted first. Nothing is written to a
    /// store that is not there yet: a listener never makes one.
    ///
    /// # Errors
    ///
    /// [`Error::Unnamed`] for a blank `worker`, [`Error::Doing`] for a
    /// `doing` that is not a doing line, [`Error::Missing`] when there is no
    /// store, and any other error of the store. Nothing is written then.
    pub fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Error> {
        if worker.trim().is_empty() {
            return Err(Error::Unnamed);
        }
        let doing = doing.map(doing_line).transpose()?;
        self.write_listeners(|tx, now| {
            drop_lapsed(tx, now)?;
            // Each wait of a worker listening takes the since of the first,
            // so the earliest is this wait's own, if it has a row.
            let since = waits_of(tx, worker)?
                .iter()
                .map(|other| other.since)
                .min()
                .unwrap_or(now);
            let labels: BTreeSet<&String> = labels
                .iter()
                .filter(|label| !label.trim().is_empty())
                .collect();
            let kinds: BTreeSet<&String> = kinds.iter().collect();
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO waits ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                ),
                params![
                    wait,
                    worker,
                    encode(&labels),
                    encode(&kinds),
                    since.to_string(),
                    now.to_string(),
                ],
            )?;
            if let Some(line) = doing {
                let was = doing_of(tx, worker)?;
                write_doing(tx, worker, next_doing(&was, line, now))?;
            }
            Ok(())
        })
    }

    /// Sets what `worker` says it is doing to `text`, as [`doing_line`]
    /// reads it: blank clears it. A wait of the worker must be listening.
    /// The same line again keeps the time it was first set. It is not a sign
    /// that the worker is listening, so no wait's row is refreshed. A worker
    /// listening only by an older cases's wait has the line set on that
    /// wait's row, which both cases read.
    ///
    /// # Errors
    ///
    /// [`Error::Doing`] for text that is not a doing line,
    /// [`Error::NotListening`] when the worker is not listening,
    /// [`Error::Missing`] when there is no store, and any other error of the
    /// store. Nothing is written then.
    pub fn set_doing(&self, worker: &str, text: &str) -> Result<(), Error> {
        let line = doing_line(text)?;
        self.write_listeners(|tx, now| {
            let listening = waits_of(tx, worker)?
                .iter()
                .any(|wait| heard_in_time(wait.heard, now));
            if listening {
                let was = doing_of(tx, worker)?;
                return write_doing(tx, worker, next_doing(&was, line, now));
            }
            let old = if has_table(tx, "listeners")? {
                tx.query_row(
                    &format!("SELECT {OLD_COLUMNS} FROM listeners WHERE worker = ?1"),
                    [worker],
                    read_old,
                )
                .optional()?
                .flatten()
                .filter(|(wait, _)| heard_in_time(wait.heard, now))
            } else {
                None
            };
            let Some((_, was)) = old else {
                return Err(Error::NotListening {
                    worker: worker.into(),
                });
            };
            let (line, at) = next_doing(&was, line, now);
            tx.execute(
                "UPDATE listeners SET doing = ?1, doing_at = ?2 WHERE worker = ?3",
                params![line, at.map(|at| at.to_string()), worker],
            )?;
            Ok(())
        })
    }

    /// The waits listening now, by the store's clock, by worker and then by
    /// id, with a live row of an older cases's `listeners` table as one wait
    /// of every kind, on the labels it heard in time, for a worker with no
    /// wait of this cases. A store an older cases made, which has no
    /// listeners yet, has none.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when there is no store, [`Error::Clock`] when the
    /// clock cannot be read, and any other error of the store.
    pub fn waits(&self) -> Result<Vec<Wait>, Error> {
        Ok(self.listening()?.0)
    }

    /// The workers listening now, by the store's clock, by name: each with
    /// the labels of all its waits listening now, when the first of them
    /// began, when one was last heard, and its doing line, from its waits as
    /// [`Db::waits`] gives them. A store an older cases made, which has no
    /// listeners yet, has none.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when there is no store, [`Error::Clock`] when the
    /// clock cannot be read, and any other error of the store.
    pub fn listeners(&self) -> Result<Vec<Listening>, Error> {
        let (waits, mut doing) = self.listening()?;
        let mut workers: BTreeMap<String, Listening> = BTreeMap::new();
        for wait in waits {
            let (line, at) = doing.remove(&wait.worker).unwrap_or_default();
            let listening = workers.entry(wait.worker.clone()).or_insert_with(|| {
                Listening::new(
                    wait.worker.clone(),
                    Vec::new(),
                    wait.since,
                    wait.heard,
                    line,
                    at,
                    Vec::new(),
                )
            });
            listening.since = listening.since.min(wait.since);
            listening.heard = listening.heard.max(wait.heard);
            listening.labels.extend(wait.labels.iter().cloned());
            listening.waits.push(wait);
        }
        Ok(workers
            .into_values()
            .map(|mut listening| {
                listening.labels.sort();
                listening.labels.dedup();
                listening
            })
            .collect())
    }

    /// The waits listening now, by worker and then by id, and the doing
    /// lines of their workers.
    fn listening(&self) -> Result<(Vec<Wait>, BTreeMap<String, Doing>), Error> {
        let (waits, doing, old) = self.read(|tx, _| {
            let (mut waits, mut doing, mut old) = (Vec::new(), Vec::new(), Vec::new());
            if has_listeners(tx)? {
                let mut statement = tx.prepare(&format!("SELECT {COLUMNS} FROM waits"))?;
                waits = statement
                    .query_map([], read_wait)?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut statement = tx.prepare("SELECT worker, doing, doing_at FROM doing")?;
                doing = statement
                    .query_map([], read_doing)?
                    .collect::<Result<Vec<_>, _>>()?;
            }
            if has_table(tx, "listeners")? {
                let mut statement = tx.prepare(&format!("SELECT {OLD_COLUMNS} FROM listeners"))?;
                old = statement
                    .query_map([], read_old)?
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Ok((waits, doing, old))
        })?;
        let now = self.now()?;
        let mut waits: Vec<Wait> = waits
            .into_iter()
            .flatten()
            .filter(|wait| heard_in_time(wait.heard, now))
            .collect();
        let mut doing: BTreeMap<String, Doing> = doing.into_iter().flatten().collect();
        // A worker listening only by an older cases's wait, which keeps
        // writing its one row per worker until it restarts, is one wait on
        // the labels that row has heard in time, of every kind.
        for (wait, line) in old.into_iter().flatten() {
            if !heard_in_time(wait.heard, now) || waits.iter().any(|w| w.worker == wait.worker) {
                continue;
            }
            let wait = wait.into_wait(now);
            doing.insert(wait.worker.clone(), line);
            waits.push(wait);
        }
        waits.sort_by(|a, b| (&a.worker, &a.id).cmp(&(&b.worker, &b.id)));
        Ok((waits, doing))
    }

    /// Runs `write` on the listeners in a write transaction, with the time
    /// now, making the tables in a store an older cases made. The tables are
    /// made in the same transaction, so a write that fails makes none.
    fn write_listeners<T>(
        &self,
        write: impl FnOnce(&Transaction<'_>, Time) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.write(false, |tx| {
            tx.execute_batch(LISTENERS)?;
            let now = self.now()?;
            write(tx, now)
        })
    }
}

/// The doing line and its time after `line` is set on the line `was`: the
/// same line as before keeps its time, and a new one takes `now`.
fn next_doing(was: &Doing, line: Option<String>, now: Time) -> Doing {
    let kept = (was.0 == line).then_some(was.1).flatten();
    let at = line.as_ref().map(|_| kept.unwrap_or(now));
    (line, at)
}

/// Writes `worker`'s doing line, or deletes it when there is none.
fn write_doing(tx: &Transaction<'_>, worker: &str, (line, at): Doing) -> Result<(), Error> {
    match (line, at) {
        (Some(line), Some(at)) => tx.execute(
            "INSERT OR REPLACE INTO doing (worker, doing, doing_at) VALUES (?1, ?2, ?3)",
            params![worker, line, at.to_string()],
        )?,
        _ => tx.execute("DELETE FROM doing WHERE worker = ?1", [worker])?,
    };
    Ok(())
}

/// Whether the store has the tables of listening: one an older cases made
/// has not, until a listener writes to them.
fn has_listeners(tx: &Transaction<'_>) -> Result<bool, Error> {
    Ok(has_table(tx, "waits")? && has_table(tx, "doing")?)
}

/// Whether the store has the table `name`.
fn has_table(tx: &Transaction<'_>, name: &str) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )?)
}

/// The columns of an older cases's row, one per worker, in the order
/// [`read_old`] reads them.
const OLD_COLUMNS: &str = "worker, labels, since, heard, doing, doing_at";

/// An older cases's row: a worker, its labels with when a wait last
/// listened on each, since and heard.
struct OldWait {
    worker: String,
    labels: BTreeMap<String, Time>,
    since: Time,
    heard: Time,
}

impl OldWait {
    /// The row as one wait at `now`, of every kind, on the labels heard in
    /// time, by the id "listeners", which no wait of this cases makes.
    fn into_wait(self, now: Time) -> Wait {
        Wait::new(
            "listeners".into(),
            self.worker,
            self.labels
                .into_iter()
                .filter(|(_, heard)| heard_in_time(*heard, now))
                .map(|(name, _)| name)
                .collect(),
            Vec::new(),
            self.since,
            self.heard,
        )
    }
}

/// An older cases's row, from [`OLD_COLUMNS`], with its doing line, or
/// `None` when it does not read: a column is not UTF-8, or its labels (an
/// object of each label and when it was heard) or times do not.
fn read_old(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<(OldWait, Doing)>> {
    let Some([worker, labels, since, heard]) = texts::<4>(row)? else {
        return Ok(None);
    };
    let optional = |index: usize| -> rusqlite::Result<Option<Option<&str>>> {
        Ok(match row.get_ref(index)? {
            ValueRef::Null => Some(None),
            ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok().map(Some),
            _ => None,
        })
    };
    let (Some(line), Some(at)) = (optional(4)?, optional(5)?) else {
        return Ok(None);
    };
    let labels: Option<BTreeMap<String, Time>> =
        serde_json::from_str::<BTreeMap<String, String>>(labels)
            .ok()
            .and_then(|labels| {
                labels
                    .into_iter()
                    .map(|(name, heard)| Some((name, heard.parse().ok()?)))
                    .collect()
            });
    let (Some(labels), Ok(since), Ok(heard), Ok(at)) = (
        labels,
        since.parse(),
        heard.parse(),
        at.map(str::parse::<Time>).transpose(),
    ) else {
        return Ok(None);
    };
    let wait = OldWait {
        worker: worker.to_owned(),
        labels,
        since,
        heard,
    };
    Ok(Some((wait, (line.map(str::to_owned), at))))
}

/// The rows of `worker`'s waits that read, lapsed or not.
fn waits_of(tx: &Transaction<'_>, worker: &str) -> Result<Vec<Wait>, Error> {
    let mut statement = tx.prepare(&format!("SELECT {COLUMNS} FROM waits WHERE worker = ?1"))?;
    let waits = statement
        .query_map([worker], read_wait)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(waits.into_iter().flatten().collect())
}

/// `worker`'s doing line, if the table has one that reads.
fn doing_of(tx: &Transaction<'_>, worker: &str) -> Result<Doing, Error> {
    let found = tx
        .query_row(
            "SELECT worker, doing, doing_at FROM doing WHERE worker = ?1",
            [worker],
            read_doing,
        )
        .optional()?;
    Ok(found.flatten().map(|(_, doing)| doing).unwrap_or_default())
}

/// Deletes the waits that no longer count as listening at `now`, and any
/// that do not read, then the doing lines of workers with no wait left, and
/// any that do not read. They are deleted by rowid, since the worker of a
/// row that does not read may not read either.
fn drop_lapsed(tx: &Transaction<'_>, now: Time) -> Result<(), Error> {
    // The rowid comes after the columns read_wait reads, at 6.
    let mut statement = tx.prepare(&format!("SELECT {COLUMNS}, rowid FROM waits"))?;
    let rows: Vec<(Option<Wait>, i64)> = statement
        .query_map([], |row| Ok((read_wait(row)?, row.get(6)?)))?
        .collect::<Result<_, _>>()?;
    let mut workers = BTreeSet::new();
    for (wait, rowid) in rows {
        match wait.filter(|wait| heard_in_time(wait.heard, now)) {
            Some(wait) => {
                workers.insert(wait.worker);
            }
            None => {
                tx.execute("DELETE FROM waits WHERE rowid = ?1", [rowid])?;
            }
        }
    }
    let mut statement = tx.prepare("SELECT worker, doing, doing_at, rowid FROM doing")?;
    let rows: Vec<(Option<String>, i64)> = statement
        .query_map([], |row| {
            Ok((read_doing(row)?.map(|(worker, _)| worker), row.get(3)?))
        })?
        .collect::<Result<_, _>>()?;
    for (worker, rowid) in rows {
        if !worker.is_some_and(|worker| workers.contains(&worker)) {
            tx.execute("DELETE FROM doing WHERE rowid = ?1", [rowid])?;
        }
    }
    Ok(())
}

/// The first `N` columns of `row` as text, or `None` when one does not read,
/// as no cases writes it: text that is not UTF-8, which only another writer
/// can store.
fn texts<'r, const N: usize>(row: &'r rusqlite::Row<'_>) -> rusqlite::Result<Option<[&'r str; N]>> {
    let mut columns = [""; N];
    for (index, column) in columns.iter_mut().enumerate() {
        // The tables are STRICT, and every column NOT NULL: a column holds
        // text.
        let ValueRef::Text(bytes) = row.get_ref(index)? else {
            return Ok(None);
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Ok(None);
        };
        *column = text;
    }
    Ok(Some(columns))
}

/// A wait's row, from [`COLUMNS`], or `None` when it does not read, as no
/// cases writes it: a column is not UTF-8, or its labels, kinds or times do
/// not read.
fn read_wait(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Wait>> {
    let Some([id, worker, labels, kinds, since, heard]) = texts::<6>(row)? else {
        return Ok(None);
    };
    let (Some(labels), Some(kinds), Ok(since), Ok(heard)) =
        (decode(labels), decode(kinds), since.parse(), heard.parse())
    else {
        return Ok(None);
    };
    Ok(Some(Wait::new(
        id.to_owned(),
        worker.to_owned(),
        labels,
        kinds,
        since,
        heard,
    )))
}

/// A doing line's row, of its worker, line and time, or `None` when it does
/// not read.
fn read_doing(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<(String, Doing)>> {
    let Some([worker, line, at]) = texts::<3>(row)? else {
        return Ok(None);
    };
    let Ok(at) = at.parse::<Time>() else {
        return Ok(None);
    };
    Ok(Some((worker.to_owned(), (Some(line.to_owned()), Some(at)))))
}

/// Labels or kinds as the table keeps them: a JSON list, in name order.
#[expect(clippy::expect_used, reason = "a list of strings serializes")]
fn encode(names: &BTreeSet<&String>) -> String {
    serde_json::to_string(names).expect("a list of strings serializes")
}

/// Labels or kinds from the table, or `None` when they do not read: not a
/// list of strings.
fn decode(text: &str) -> Option<Vec<String>> {
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use tempfile::TempDir;

    use super::*;
    use crate::testing::{count, create, scratch, sql};

    const SECOND: i64 = 1_000_000_000;

    /// A store with an fyi case, so that it is there, and a clock the test
    /// moves: nanoseconds since 2026-09-15T09:00:00Z.
    fn clocked() -> (TempDir, Db, Arc<AtomicI64>) {
        let (dir, db) = scratch();
        let nanos = Arc::new(AtomicI64::new(0));
        let clock = Arc::clone(&nanos);
        let db = Db::new(db.path()).with_clock(move || at(clock.load(Ordering::SeqCst)));
        create(&db);
        (dir, db, nanos)
    }

    /// The time `nanos` after 2026-09-15T09:00:00Z.
    fn at(nanos: i64) -> Time {
        let start = "2026-09-15T09:00:00Z".parse::<Time>().unwrap().unix_secs();
        let subsec = u32::try_from(nanos.rem_euclid(SECOND)).unwrap();
        Time::from_unix(start + nanos.div_euclid(SECOND), subsec).unwrap()
    }

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// Records `worker` as listening on `labels`, by a wait of the
    /// worker's name, which takes every kind: one wait for each worker.
    fn listen(db: &Db, worker: &str, labels: &[String], doing: Option<&str>) -> Result<(), Error> {
        db.listen(worker, worker, labels, &[], doing)
    }

    fn only(db: &Db) -> Listening {
        let mut listeners = db.listeners().unwrap();
        assert_eq!(listeners.len(), 1, "{listeners:?}");
        listeners.remove(0)
    }

    #[test]
    fn a_listener_is_recorded_with_its_labels_and_times() {
        let (_dir, db, _clock) = clocked();
        listen(&db, "bun-pins", &labels(&["release"]), None).unwrap();
        assert_eq!(
            only(&db),
            Listening::new(
                "bun-pins".into(),
                labels(&["release"]),
                at(0),
                at(0),
                None,
                None,
                vec![Wait::new(
                    "bun-pins".into(),
                    "bun-pins".into(),
                    labels(&["release"]),
                    Vec::new(),
                    at(0),
                    at(0),
                )],
            )
        );
    }

    #[test]
    fn a_heartbeat_keeps_when_its_wait_began() {
        let (_dir, db, clock) = clocked();
        listen(&db, "bun-pins", &labels(&["b"]), None).unwrap();
        clock.store(15 * SECOND, Ordering::SeqCst);
        listen(&db, "bun-pins", &labels(&["b"]), None).unwrap();
        let listening = only(&db);
        assert_eq!(listening.labels, labels(&["b"]));
        assert_eq!((listening.since, listening.heard), (at(0), at(15 * SECOND)));
        assert_eq!(count(&db, "waits"), 1);
    }

    /// Two waits of one worker are two rows, each with its own labels and
    /// kinds, and each lapses on its own. The worker is listed once, with
    /// the labels of the waits listening, since the first of them began.
    #[test]
    fn each_wait_of_a_worker_is_its_own_row_and_lapses_on_its_own() {
        let (_dir, db, clock) = clocked();
        let kinds = labels(&["message", "decision"]);
        db.listen("w1", "bun-pins", &labels(&["release"]), &[], None)
            .unwrap();
        clock.store(30 * SECOND, Ordering::SeqCst);
        db.listen("w2", "bun-pins", &labels(&["docs", "docs"]), &kinds, None)
            .unwrap();
        let wait = |id: &str, labels: Vec<String>, kinds: Vec<String>, heard: Time| {
            Wait::new(id.into(), "bun-pins".into(), labels, kinds, at(0), heard)
        };
        assert_eq!(
            db.waits().unwrap(),
            [
                wait("w1", labels(&["release"]), vec![], at(0)),
                // A wait that starts while another listens takes its time,
                // and its kinds are stored in name order, as its labels are.
                wait(
                    "w2",
                    labels(&["docs"]),
                    labels(&["decision", "message"]),
                    at(30 * SECOND)
                ),
            ]
        );
        clock.store(45 * SECOND, Ordering::SeqCst);
        assert_eq!(only(&db).labels, labels(&["docs", "release"]));
        clock.store(45 * SECOND + 1, Ordering::SeqCst);
        let listening = only(&db);
        assert_eq!(listening.labels, labels(&["docs"]));
        assert_eq!((listening.since, listening.heard), (at(0), at(30 * SECOND)));
        assert_eq!(db.waits().unwrap().len(), 1);
        db.listen("w2", "bun-pins", &labels(&["docs"]), &kinds, None)
            .unwrap();
        assert_eq!(count(&db, "waits"), 1, "the lapsed wait was not deleted");
        assert_eq!(only(&db).since, at(0));
    }

    /// A wait's id is new each time, as long as an actor's.
    #[test]
    fn each_new_wait_has_an_id_of_its_own() {
        let (a, b) = (new_wait().unwrap(), new_wait().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 22, "{a}");
    }

    /// One wait for two workers, as `wait --worker a --worker b` is, is a
    /// row for each.
    #[test]
    fn a_wait_for_two_workers_is_a_row_for_each() {
        let (_dir, db, _clock) = clocked();
        for worker in ["a", "b"] {
            db.listen("w1", worker, &[], &[], None).unwrap();
        }
        let workers: Vec<String> = db.waits().unwrap().into_iter().map(|w| w.worker).collect();
        assert_eq!(workers, ["a", "b"]);
    }

    /// No message goes to a blank worker or label: a listener needs a worker,
    /// and leaves a blank label out.
    #[test]
    fn a_listener_needs_a_worker_and_leaves_out_a_blank_label() {
        let (_dir, db, _clock) = clocked();
        for worker in ["", "  "] {
            let error = listen(&db, worker, &[], None).unwrap_err();
            assert!(matches!(error, Error::Unnamed), "{worker:?}: {error:?}");
        }
        assert_eq!(db.listeners().unwrap(), []);
        listen(&db, "bun-pins", &labels(&["", " ", "release"]), None).unwrap();
        assert_eq!(only(&db).labels, labels(&["release"]));
    }

    /// A row counts as listening until 45 s after its last refresh, and no
    /// longer; the next listener's write deletes it.
    #[test]
    fn a_row_lapses_45_seconds_after_its_last_heartbeat() {
        let (_dir, db, clock) = clocked();
        listen(&db, "bun-pins", &[], None).unwrap();
        clock.store(45 * SECOND, Ordering::SeqCst);
        assert_eq!(only(&db).worker, "bun-pins");
        clock.store(45 * SECOND + 1, Ordering::SeqCst);
        assert_eq!(db.listeners().unwrap(), []);
        assert_eq!(count(&db, "waits"), 1, "a read deleted the row");
        listen(&db, "lead", &[], None).unwrap();
        assert_eq!(count(&db, "waits"), 1);
        assert_eq!(only(&db).worker, "lead");
    }

    #[test]
    fn a_worker_that_listens_again_after_its_row_lapsed_starts_afresh() {
        let (_dir, db, clock) = clocked();
        listen(&db, "bun-pins", &labels(&["a"]), Some("rebasing")).unwrap();
        clock.store(60 * SECOND, Ordering::SeqCst);
        listen(&db, "bun-pins", &labels(&["b"]), None).unwrap();
        let listening = only(&db);
        assert_eq!(listening.since, at(60 * SECOND));
        assert_eq!(listening.labels, labels(&["b"]));
        assert_eq!((listening.doing, listening.doing_at), (None, None));
    }

    /// The doing line: the same line again keeps its time, a new one takes
    /// the time now, and a blank one clears it. A wait without one leaves it.
    #[test]
    fn a_doing_line_keeps_its_time_until_it_changes() {
        let (_dir, db, clock) = clocked();
        let doing = |db: &Db| {
            let listening = only(db);
            (listening.doing, listening.doing_at)
        };
        listen(&db, "bun-pins", &[], Some(" rebasing ")).unwrap();
        assert_eq!(doing(&db), (Some("rebasing".into()), Some(at(0))));
        clock.store(30 * SECOND, Ordering::SeqCst);
        listen(&db, "bun-pins", &[], None).unwrap();
        assert_eq!(doing(&db), (Some("rebasing".into()), Some(at(0))));
        listen(&db, "bun-pins", &[], Some("rebasing")).unwrap();
        assert_eq!(doing(&db), (Some("rebasing".into()), Some(at(0))));
        clock.store(31 * SECOND, Ordering::SeqCst);
        db.set_doing("bun-pins", "testing").unwrap();
        assert_eq!(doing(&db), (Some("testing".into()), Some(at(31 * SECOND))));
        listen(&db, "bun-pins", &[], Some("  ")).unwrap();
        assert_eq!(doing(&db), (None, None));
    }

    /// `set_doing` needs a worker that is listening, and is no sign that it
    /// is: the row is not refreshed.
    #[test]
    fn a_doing_line_needs_a_listening_worker() {
        let (_dir, db, clock) = clocked();
        let refused = |db: &Db| match db.set_doing("bun-pins", "rebasing") {
            Err(error @ Error::NotListening { .. }) => error.to_string(),
            other => panic!("{other:?}"),
        };
        assert!(refused(&db).starts_with("\"bun-pins\" is not listening"));
        listen(&db, "bun-pins", &[], None).unwrap();
        clock.store(10 * SECOND, Ordering::SeqCst);
        db.set_doing("bun-pins", "rebasing").unwrap();
        let listening = only(&db);
        assert_eq!(listening.heard, at(0), "set_doing refreshed the row");
        assert_eq!(listening.doing_at, Some(at(10 * SECOND)));
        clock.store(46 * SECOND, Ordering::SeqCst);
        refused(&db);
    }

    #[test]
    fn a_doing_line_is_one_line_of_at_most_80_characters() {
        assert_eq!(doing_line("  rebasing  ").unwrap(), Some("rebasing".into()));
        for blank in ["", "   ", "\u{a0}"] {
            assert_eq!(doing_line(blank).unwrap(), None, "{blank:?}");
        }
        let longer = "é".repeat(cases_core::DOING_MAX + 1);
        let error = doing_line(&longer).unwrap_err().to_string();
        assert_eq!(
            error,
            "a doing line holds at most 80 characters, and this one has 81"
        );
        assert!(matches!(doing_line("a\nb"), Err(Error::Doing(_))));
        // A listener with a bad line writes nothing.
        let (_dir, db, _clock) = clocked();
        assert!(listen(&db, "bun-pins", &[], Some(&longer)).is_err());
        assert_eq!(db.listeners().unwrap(), []);
    }

    /// A store an older cases made has no tables of listening. A read finds
    /// no listeners, a refused write makes no table, and the first listener
    /// makes them, at the same schema version. An older cases's table of
    /// listeners, one row per worker, is left alone, and its lapsed row is
    /// not listed.
    #[test]
    fn a_store_an_older_cases_made_gets_the_tables_from_its_first_listener() {
        let (_dir, db, _clock) = clocked();
        sql(&db)
            .execute_batch(
                "DROP TABLE waits; DROP TABLE doing;
                 CREATE TABLE listeners (worker TEXT NOT NULL PRIMARY KEY, labels TEXT NOT NULL,
                     since TEXT NOT NULL, heard TEXT NOT NULL, doing TEXT, doing_at TEXT) STRICT;",
            )
            .unwrap();
        // Lapsed, so that it is not listed: how a live one is, is
        // an_older_cases_s_listener_is_one_wait_until_its_worker_has_new_ones.
        let old = format!(
            "INSERT INTO listeners (worker, labels, since, heard) VALUES ('old', '{{}}', '{0}', '{0}')",
            at(-100 * SECOND)
        );
        sql(&db).execute(&old, []).unwrap();
        let tables = |db: &Db| -> i64 {
            sql(db)
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name IN ('waits', 'doing')",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(db.listeners().unwrap(), []);
        assert!(db.set_doing("bun-pins", "rebasing").is_err());
        assert_eq!(tables(&db), 0, "a refused write made the tables");
        listen(&db, "bun-pins", &[], Some("rebasing")).unwrap();
        assert_eq!((tables(&db), only(&db).worker.as_str()), (2, "bun-pins"));
        assert_eq!(count(&db, "listeners"), 1, "the older table was changed");
        let version: i64 = sql(&db)
            .query_row("SELECT schema_version FROM meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    /// Makes an older cases's table of listeners, one row per worker, and
    /// gives a writer of a row to it for `worker`, heard at `heard`, with
    /// `labels` as that table keeps them and a doing line.
    fn older(db: &Db) -> impl Fn(&str, &str, Time) {
        sql(db)
            .execute_batch(
                "CREATE TABLE listeners (worker TEXT NOT NULL PRIMARY KEY, labels TEXT NOT NULL,
                     since TEXT NOT NULL, heard TEXT NOT NULL, doing TEXT, doing_at TEXT) STRICT;",
            )
            .unwrap();
        |worker: &str, labels: &str, heard: Time| {
            sql(db)
                .execute(
                    "INSERT OR REPLACE INTO listeners (worker, labels, since, heard, doing, doing_at)
                     VALUES (?1, ?2, ?3, ?4, 'rebasing', ?3)",
                    params![worker, labels, at(0).to_string(), heard.to_string()],
                )
                .unwrap();
        }
    }

    /// A worker listening only by an older cases's wait, which writes one
    /// row per worker until it restarts, is listed as one wait of every kind
    /// on the labels that row heard in time, with its doing line. A worker
    /// with a wait of this cases is listed by those alone, and a lapsed row
    /// is not listed.
    #[test]
    fn an_older_cases_s_listener_is_one_wait_until_its_worker_has_new_ones() {
        let (_dir, db, clock) = clocked();
        let old = older(&db);
        clock.store(30 * SECOND, Ordering::SeqCst);
        let heard = format!(
            r#"{{"release": "{}", "stale": "{}"}}"#,
            at(30 * SECOND),
            at(0)
        );
        old("homeday", &heard, at(30 * SECOND));
        old("gone", "{}", at(0));
        clock.store(46 * SECOND, Ordering::SeqCst);
        assert_eq!(
            db.waits().unwrap(),
            [Wait::new(
                "listeners".into(),
                "homeday".into(),
                labels(&["release"]),
                Vec::new(),
                at(0),
                at(30 * SECOND),
            )]
        );
        let listening = only(&db);
        assert_eq!(
            (listening.labels, listening.doing, listening.doing_at),
            (labels(&["release"]), Some("rebasing".into()), Some(at(0)))
        );
        // A wait of this cases for the worker: its own rows alone.
        db.listen("w1", "homeday", &labels(&["homeday"]), &[], None)
            .unwrap();
        let waits: Vec<(String, Vec<String>)> = db
            .waits()
            .unwrap()
            .into_iter()
            .map(|w| (w.id, w.labels))
            .collect();
        assert_eq!(waits, [("w1".to_owned(), labels(&["homeday"]))]);
        assert_eq!(only(&db).doing, None);
        // `cases doing` sets an older cases's wait's line on its row, while
        // the worker has no wait of this cases.
        old("triage", "{}", at(46 * SECOND));
        db.set_doing("triage", "sorting mail").unwrap();
        let line: (String, String) = sql(&db)
            .query_row(
                "SELECT doing, doing_at FROM listeners WHERE worker = 'triage'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            line,
            ("sorting mail".to_owned(), at(46 * SECOND).to_string())
        );
        db.set_doing("triage", "sorting mail").unwrap();
        let listed = db.listeners().unwrap();
        let triage = listed.iter().find(|l| l.worker == "triage").unwrap();
        assert_eq!(
            triage.doing_at,
            Some(at(46 * SECOND)),
            "the same line kept its time"
        );
        assert_eq!(
            count(&db, "doing"),
            0,
            "the line went to this cases's table"
        );
        // One with no doing line is listed, with none.
        sql(&db)
            .execute(
                "UPDATE listeners SET doing = NULL, doing_at = NULL WHERE worker = 'triage'",
                [],
            )
            .unwrap();
        let listed = db.listeners().unwrap();
        let triage = listed.iter().find(|l| l.worker == "triage").unwrap();
        assert_eq!((triage.doing.as_deref(), triage.doing_at), (None, None));
        // A lapsed row of an older cases is not listed.
        clock.store(200 * SECOND, Ordering::SeqCst);
        old("homeday", "{}", at(100 * SECOND));
        assert_eq!(db.waits().unwrap(), []);
        assert!(matches!(
            db.set_doing("triage", "x"),
            Err(Error::NotListening { .. })
        ));
        // A row of an older cases that does not read is skipped.
        old("bad", "not json", at(200 * SECOND));
        assert_eq!(db.waits().unwrap(), []);
    }

    /// A store with one of the tables and not the other, which no cases
    /// makes, has no listeners rather than failing to read, and the next
    /// listener makes the other.
    #[test]
    fn a_store_with_half_the_tables_has_no_listeners() {
        let (_dir, db, _clock) = clocked();
        listen(&db, "bun-pins", &[], None).unwrap();
        sql(&db).execute("DROP TABLE doing", []).unwrap();
        assert_eq!(db.listeners().unwrap(), []);
        assert_eq!(db.waits().unwrap(), []);
        listen(&db, "bun-pins", &[], None).unwrap();
        assert_eq!(only(&db).worker, "bun-pins");
    }

    #[test]
    fn a_listener_never_makes_the_store() {
        let (_dir, db) = scratch();
        assert!(matches!(
            listen(&db, "bun-pins", &[], None),
            Err(Error::Missing { .. })
        ));
        assert!(matches!(db.listeners(), Err(Error::Missing { .. })));
        assert!(!db.path().exists());
    }

    /// A row no cases writes, such as one whose labels are not a list, is
    /// not a listener, and the next listener's write deletes it. So is a row
    /// with text that is not UTF-8 in any column, its worker's included,
    /// which a STRICT table takes from another writer. The same goes for a
    /// doing line, and for one whose worker has no wait listening.
    #[test]
    fn a_row_that_does_not_read_is_skipped_and_deleted() {
        let (_dir, db, _clock) = clocked();
        let since = at(0).to_string();
        let wait = "INSERT INTO waits (id, worker, labels, kinds, since, heard)
                    VALUES (?1, ?1, ?2, ?3, ?4, ?4)";
        for (worker, labels, kinds, time) in [
            ("bad-labels", "not json", "[]", since.as_str()),
            ("bad-kinds", "[]", "{}", since.as_str()),
            ("bad-time", "[]", "[]", "yesterday"),
            ("labels-as-an-object", "{}", "[]", since.as_str()),
            ("a-label-not-text", "[1]", "[]", since.as_str()),
        ] {
            sql(&db)
                .execute(wait, params![worker, labels, kinds, time])
                .unwrap();
        }
        for column in ["id", "worker", "labels", "kinds", "since", "heard"] {
            let worker = format!("bad-{column}-bytes");
            let conn = sql(&db);
            conn.execute(wait, params![worker, "[]", "[]", since])
                .unwrap();
            conn.execute(
                &format!("UPDATE waits SET {column} = CAST(x'ff' AS TEXT) WHERE id = ?1"),
                [&worker],
            )
            .unwrap();
        }
        let doing = "INSERT INTO doing (worker, doing, doing_at) VALUES (?1, 'x', ?2)";
        sql(&db)
            .execute(doing, params!["bad-doing-time", "soon"])
            .unwrap();
        sql(&db)
            .execute(doing, params!["not-listening", since])
            .unwrap();
        sql(&db).execute(doing, params!["lead", since]).unwrap();
        for column in ["worker", "doing", "doing_at"] {
            sql(&db)
                .execute(
                    &format!(
                        "UPDATE doing SET {column} = CAST(x'ff' AS TEXT) WHERE worker = 'lead'"
                    ),
                    [],
                )
                .unwrap();
            if column == "worker" {
                sql(&db).execute(doing, params!["lead", since]).unwrap();
            }
        }
        assert_eq!(db.listeners().unwrap(), []);
        listen(&db, "lead", &[], None).unwrap();
        assert_eq!(count(&db, "waits"), 1);
        assert_eq!(count(&db, "doing"), 0);
        let listening = only(&db);
        assert_eq!((listening.worker.as_str(), listening.doing), ("lead", None));
    }

    #[test]
    fn listeners_are_listed_by_worker() {
        let (_dir, db, _clock) = clocked();
        for worker in ["lead", "bun-pins", "docs"] {
            listen(&db, worker, &[], None).unwrap();
        }
        let workers: Vec<String> = db
            .listeners()
            .unwrap()
            .into_iter()
            .map(|listening| listening.worker)
            .collect();
        assert_eq!(workers, ["bun-pins", "docs", "lead"]);
    }
}
