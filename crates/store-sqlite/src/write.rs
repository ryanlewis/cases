//! Writes: the [`Store`] trait for [`Db`].

use cases_core::{
    ActorRow, Append, Case, Changes, Event, Key, Listening, Row, Store, StoreError, Time, Version,
    Wait, Writer,
};
use rusqlite::{Transaction, params};

use crate::actors::actor_for;
use crate::db::Db;
use crate::error::Error;
use crate::key::{RowId, from_sql};
use crate::read::load_cases;

impl From<Error> for StoreError<Error> {
    fn from(error: Error) -> StoreError<Error> {
        StoreError::Store(error)
    }
}

impl Db {
    /// Opens a new case as [`Store::create`] does, and gives its key and its
    /// rows, read in the write's own transaction: what the open stored, with
    /// no later writer's rows and no second read that can fail after the
    /// open was stored.
    ///
    /// # Errors
    ///
    /// As [`Store::create`]; either way nothing is stored.
    pub fn create_rows(
        &self,
        open: &Append,
        writer: &Writer,
    ) -> Result<(Key, Vec<Row>), StoreError<Error>> {
        self.create_then(open, writer, |tx, key, _| Ok((key, self.rows_in(tx, key)?)))
    }

    /// Appends an event as [`Store::append`] does, and gives the case's rows,
    /// read in the write's own transaction, as [`Db::create_rows`] does.
    ///
    /// # Errors
    ///
    /// As [`Store::append`]; either way nothing is stored.
    pub fn append_rows(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Vec<Row>, StoreError<Error>> {
        self.append_then(*key, append, writer, at_revision, |tx, _| {
            self.rows_in(tx, *key)
        })
    }

    /// The rows of the case `key`, read in `tx`, which has just stored one.
    fn rows_in(&self, tx: &Transaction<'_>, key: Key) -> Result<Vec<Row>, StoreError<Error>> {
        let (_, rows) = load_cases(tx, self.path(), Some(&[key]))?
            .pop()
            .ok_or(StoreError::NoCase)?;
        Ok(rows)
    }

    /// Opens a new case, and gives what `then` makes of it in the same
    /// transaction, before it commits.
    fn create_then<T>(
        &self,
        open: &Append,
        writer: &Writer,
        then: impl FnOnce(&Transaction<'_>, Key, Case) -> Result<T, StoreError<Error>>,
    ) -> Result<T, StoreError<Error>> {
        // Only a case the writer takes makes the store (AG-01, ST-08). The
        // time and the actor it is written with do not change whether it is
        // taken, so it is checked with a stand-in time and no actor before
        // the file is touched, and the clock is read only under the lock.
        let stand_in = Time::from_unix(0, 0).ok_or(Error::Clock)?;
        Case::open(open, stand_in, None)?;
        self.write(true, |tx| {
            let now = self.now()?;
            let actor = actor_for(tx, &open.author, writer, now)?;
            let (case, row) = Case::open(open, now, actor.as_ref())?;
            let opened_at = case.opened_at().map(|at| at.to_string());
            tx.execute(
                "INSERT INTO cases (opened_at) VALUES (?1)",
                [opened_at.unwrap_or_default()],
            )
            .map_err(Error::from)?;
            let key = from_sql(tx.last_insert_rowid()).ok_or_else(|| Error::Damaged {
                path: self.path().to_owned(),
                what: "it has no key left for another case".into(),
            })?;
            self.insert(tx, key, &row, &case)?;
            then(tx, key, case)
        })
    }

    /// Appends an event to the case `key`, and gives what `then` makes of
    /// the case after it in the same transaction, before it commits.
    fn append_then<T>(
        &self,
        key: Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
        then: impl FnOnce(&Transaction<'_>, Case) -> Result<T, StoreError<Error>>,
    ) -> Result<T, StoreError<Error>> {
        self.write(false, |tx| {
            let (_, rows) = load_cases(tx, self.path(), Some(&[key]))?
                .pop()
                .ok_or(StoreError::NoCase)?;
            let mut case = Case::fold(&rows)?;
            // Stamped once the write holds the store's lock, so events are
            // timed in the order they are stored, however long the wait for
            // the lock: a wait since an event's time sees every event stored
            // after it.
            let now = self.now()?;
            let actor = actor_for(tx, &append.author, writer, now)?;
            let row = case.append(append, now, actor.as_ref(), at_revision)?;
            self.insert(tx, key, &row, &case)?;
            then(tx, case)
        })
    }
}

impl Store for Db {
    type Key = Key;
    type Writer = Writer;
    type Error = Error;

    /// Opens a new case with the next key. The case and its first row are
    /// stored in one transaction, which makes the store first when there is
    /// none. An open the writer refuses does not make the store.
    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Error>> {
        self.create_then(open, writer, |_, key, case| Ok((key, case)))
    }

    /// Appends an event to the case `key`, in a transaction that holds the
    /// store's lock from its first read: the case is folded, the event
    /// stamped, checked and stored under it.
    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Error>> {
        self.append_then(*key, append, writer, at_revision, |_, case| Ok(case))
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Error> {
        self.read(|tx, _| {
            let case = load_cases(tx, self.path(), Some(&[*key]))?.pop();
            Ok(case.map(|(_, rows)| rows))
        })
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Error> {
        self.read(|tx, _| load_cases(tx, self.path(), None))
    }

    /// The version is made from the one counter every event and every
    /// removal raise, with the store's id. A store not made yet is `0`.
    fn version(&self) -> Result<Version, Error> {
        match self.read(|tx, _| crate::read::version(tx)) {
            Err(Error::Missing { .. }) => Ok(Version::new("0")),
            other => other,
        }
    }

    /// What changed since `since`, from the rows stored and the removals
    /// logged since, or every case when `since` is not a version of this
    /// store it can still read changes from.
    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Error> {
        match self.read(|tx, _| crate::read::changes(tx, self.path(), since)) {
            Err(Error::Missing { .. }) => {
                let version = Version::new("0");
                if since == Some(&version) {
                    Ok(Changes::Unchanged { version })
                } else {
                    Ok(Changes::All {
                        version,
                        cases: Vec::new(),
                    })
                }
            }
            other => other,
        }
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Error> {
        Db::actors(self)
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Error> {
        Db::listen(self, wait, worker, labels, kinds, doing)
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Error> {
        Db::set_doing(self, worker, text)
    }

    fn waits(&self) -> Result<Vec<Wait>, Error> {
        Db::waits(self)
    }

    fn listeners(&self) -> Result<Vec<Listening>, Error> {
        Db::listeners(self)
    }

    /// Moves the case `key` and its rows into the archive tables, in one
    /// transaction, with the time it was archived. A case whose key the
    /// archive holds already, as it can after a case is put back from the
    /// archive by hand, is refused with [`Error::Archived`] and left where it
    /// is.
    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Error>> {
        self.remove(*key, at_revision, true)
    }

    /// Removes the case `key` and its rows, in one transaction.
    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Error>> {
        self.remove(*key, at_revision, false)
    }
}

impl Db {
    /// Stores `row`, the new last row of `case`, as a row of the case `key`,
    /// with its event's time.
    fn insert(&self, tx: &Transaction<'_>, key: Key, row: &Row, case: &Case) -> Result<(), Error> {
        let seq = i64::try_from(row.seq).map_err(|_| Error::Damaged {
            path: self.path().to_owned(),
            what: format!("case {key} has no seq left for another row"),
        })?;
        let at = case
            .events()
            .last()
            .and_then(Event::at)
            .map(|at| at.to_string());
        tx.execute(
            "INSERT INTO events (case_key, seq, author, event, at, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key.sql(),
                seq,
                row.author,
                row.event,
                at.unwrap_or_default(),
                row.data.as_bytes()
            ],
        )?;
        Ok(())
    }

    /// Archives or deletes the case `key`, if it is at `at_revision`.
    fn remove(
        &self,
        key: Key,
        at_revision: Option<u64>,
        archive: bool,
    ) -> Result<(), StoreError<Error>> {
        self.write(false, |tx| {
            let (found, revision): (bool, i64) = tx
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM cases WHERE key = ?1),
                            (SELECT count(*) FROM events WHERE case_key = ?1)",
                    [key.sql()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(Error::from)?;
            if !found {
                return Err(StoreError::NoCase);
            }
            // A count is never below 0.
            let revision = u64::try_from(revision).unwrap_or_default();
            if let Some(read) = at_revision
                && read != revision
            {
                return Err(cases_core::Error::stale(read, revision).into());
            }
            if archive {
                let archived: bool = tx
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM archive_cases WHERE key = ?1)",
                        [key.sql()],
                        |row| row.get(0),
                    )
                    .map_err(Error::from)?;
                if archived {
                    return Err(Error::Archived { key }.into());
                }
                let archived_at = self.now()?.to_string();
                tx.execute(
                    "INSERT INTO archive_cases (key, opened_at, archived_at)
                     SELECT key, opened_at, ?2 FROM cases WHERE key = ?1",
                    params![key.sql(), archived_at],
                )
                .map_err(Error::from)?;
                tx.execute(
                    "INSERT INTO archive_events (change, case_key, seq, author, event, at, data)
                     SELECT change, case_key, seq, author, event, at, data
                     FROM events WHERE case_key = ?1",
                    [key.sql()],
                )
                .map_err(Error::from)?;
            }
            for statement in [
                "DELETE FROM events WHERE case_key = ?1",
                "DELETE FROM cases WHERE key = ?1",
            ] {
                tx.execute(statement, [key.sql()]).map_err(Error::from)?;
            }
            Ok(())
        })
    }
}
