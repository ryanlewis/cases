//! A store kept in memory: the smallest store the trait allows, written from
//! its docs. It answers `rows_since` with what changed, from a record of
//! what it took out, as `cases-store-sqlite` does from its log of removals.
//! The store vectors run against it, and so do the poller's tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cases_core::{
    Actor, ActorRow, Append, Author, Case, Changes, Error, Key, Listening, Row, Store, StoreError,
    Time, Version, Wait, Writer, doing_line, heard_in_time,
};

/// What goes wrong in the store itself.
#[derive(Debug)]
pub(crate) enum Fault {
    /// No case was ever opened: the store is not made.
    Missing,
    /// A blank worker.
    Unnamed,
    /// A doing line that is not one.
    Doing(Error),
    /// A doing line for a worker not listening.
    NotListening,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Missing => f.write_str("no store"),
            Fault::Unnamed => f.write_str("a blank worker"),
            Fault::Doing(error) => error.fmt(f),
            Fault::NotListening => f.write_str("not listening"),
        }
    }
}

/// A doing line and when it was set.
type Doing = (Option<String>, Option<Time>);

#[derive(Default)]
struct Inner {
    made: bool,
    last_key: u64,
    /// Each case's rows, and the change of its last row.
    cases: BTreeMap<Key, (Vec<Row>, u64)>,
    /// Raised by every event, archive and delete.
    changes: u64,
    /// Each case taken out, by the change that took it out.
    removed: Vec<(u64, Key)>,
    actors: Vec<ActorRow>,
    /// Each wait listening, by worker and id.
    waits: BTreeMap<(String, String), Wait>,
    doing: BTreeMap<String, Doing>,
}

pub(crate) struct Memory {
    inner: RefCell<Inner>,
    /// The store's clock, which the caller sets.
    pub(crate) now: Cell<Time>,
}

impl Memory {
    /// An empty store, not made yet, with its clock at `start`.
    pub(crate) fn new(start: Time) -> Memory {
        Memory {
            inner: RefCell::default(),
            now: Cell::new(start),
        }
    }

    /// The actor for `writer` writing as `author`, and whether it is new.
    fn actor(
        inner: &Inner,
        author: &str,
        writer: &Writer,
        now: Time,
    ) -> Option<(Actor, Option<ActorRow>)> {
        let kind = author.parse::<Author>().ok()?.as_str();
        let name = writer.name().map(str::to_owned);
        let mut actor = Actor::default();
        if let Some(row) = inner
            .actors
            .iter()
            .find(|a| a.kind == kind && a.name == name)
        {
            actor.id.clone_from(&row.id);
            return Some((actor, None));
        }
        // 22 base64url characters, as an actor's id is.
        actor.id = format!("{:0>22}", inner.actors.len() + 1);
        let row = ActorRow::new(actor.id.clone(), kind.into(), name, now.to_string());
        Some((actor, Some(row)))
    }

    fn take(&self, key: Key, at_revision: Option<u64>) -> Result<(), StoreError<Fault>> {
        let mut inner = self.inner.borrow_mut();
        let (rows, _) = inner.cases.get(&key).ok_or(StoreError::NoCase)?;
        let revision = rows.len() as u64;
        if let Some(read) = at_revision
            && read != revision
        {
            return Err(Error::stale(read, revision).into());
        }
        inner.cases.remove(&key);
        inner.changes += 1;
        let change = inner.changes;
        inner.removed.push((change, key));
        Ok(())
    }

    /// Drops the waits that have lapsed, and the doing lines of workers
    /// with none left.
    fn drop_lapsed(inner: &mut Inner, now: Time) {
        inner.waits.retain(|_, wait| heard_in_time(wait.heard, now));
        let live: BTreeSet<String> = inner.waits.keys().map(|(w, _)| w.clone()).collect();
        inner.doing.retain(|worker, _| live.contains(worker));
    }

    fn live_waits(&self) -> Vec<Wait> {
        let now = self.now.get();
        let inner = self.inner.borrow();
        inner
            .waits
            .values()
            .filter(|wait| heard_in_time(wait.heard, now))
            .cloned()
            .collect()
    }
}

/// The doing line after `line` is set on `was`: the same line keeps its
/// time, and a new one takes `now`.
fn next_doing(was: &Doing, line: Option<String>, now: Time) -> Doing {
    let kept = (was.0 == line).then_some(was.1).flatten();
    let at = line.as_ref().map(|_| kept.unwrap_or(now));
    (line, at)
}

impl Store for Memory {
    type Key = Key;
    type Writer = Writer;
    type Error = Fault;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Fault>> {
        let now = self.now.get();
        let mut inner = self.inner.borrow_mut();
        let actor = Memory::actor(&inner, &open.author, writer, now);
        let (case, row) = Case::open(open, now, actor.as_ref().map(|(a, _)| a))?;
        if let Some((_, Some(new))) = actor {
            inner.actors.push(new);
        }
        inner.made = true;
        inner.last_key += 1;
        let key = Key::new(inner.last_key).unwrap();
        inner.changes += 1;
        let change = inner.changes;
        inner.cases.insert(key, (vec![row], change));
        Ok((key, case))
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Fault>> {
        let now = self.now.get();
        let mut inner = self.inner.borrow_mut();
        let actor = Memory::actor(&inner, &append.author, writer, now);
        let (rows, _) = inner.cases.get(key).ok_or(StoreError::NoCase)?;
        let mut case = Case::fold(rows)?;
        let row = case.append(append, now, actor.as_ref().map(|(a, _)| a), at_revision)?;
        if let Some((_, Some(new))) = actor {
            inner.actors.push(new);
        }
        inner.changes += 1;
        let change = inner.changes;
        let (rows, last) = inner.cases.get_mut(key).unwrap();
        rows.push(row);
        *last = change;
        Ok(case)
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Fault> {
        Ok(self
            .inner
            .borrow()
            .cases
            .get(key)
            .map(|(rows, _)| rows.clone()))
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Fault> {
        let inner = self.inner.borrow();
        Ok(inner
            .cases
            .iter()
            .map(|(k, (rows, _))| (*k, rows.clone()))
            .collect())
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Fault>> {
        self.take(*key, at_revision)
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Fault>> {
        self.take(*key, at_revision)
    }

    fn version(&self) -> Result<Version, Fault> {
        Ok(Version::new(self.inner.borrow().changes.to_string()))
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Fault> {
        let version = self.version()?;
        let inner = self.inner.borrow();
        let from = since.and_then(|v| v.as_str().parse::<u64>().ok());
        match from {
            _ if since == Some(&version) => Ok(Changes::Unchanged { version }),
            Some(from) if from <= inner.changes => Ok(Changes::Since {
                version,
                cases: inner
                    .cases
                    .iter()
                    .filter(|(_, (_, last))| *last > from)
                    .map(|(k, (rows, _))| (*k, rows.clone()))
                    .collect(),
                removed: inner
                    .removed
                    .iter()
                    .filter(|(change, _)| *change > from)
                    .map(|(_, key)| *key)
                    .collect(),
            }),
            _ => Ok(Changes::All {
                version,
                cases: self.list()?,
            }),
        }
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Fault> {
        let inner = self.inner.borrow();
        if !inner.made {
            return Err(Fault::Missing);
        }
        Ok(inner.actors.clone())
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Fault> {
        if worker.trim().is_empty() {
            return Err(Fault::Unnamed);
        }
        let doing = doing.map(doing_line).transpose().map_err(Fault::Doing)?;
        let now = self.now.get();
        let mut inner = self.inner.borrow_mut();
        if !inner.made {
            return Err(Fault::Missing);
        }
        Memory::drop_lapsed(&mut inner, now);
        let since = inner
            .waits
            .values()
            .filter(|w| w.worker == worker)
            .map(|w| w.since)
            .min()
            .unwrap_or(now);
        let labels: BTreeSet<String> = labels
            .iter()
            .filter(|l| !l.trim().is_empty())
            .cloned()
            .collect();
        let kinds: BTreeSet<String> = kinds.iter().cloned().collect();
        let row = Wait::new(
            wait.into(),
            worker.into(),
            labels.into_iter().collect(),
            kinds.into_iter().collect(),
            since,
            now,
        );
        inner.waits.insert((worker.into(), wait.into()), row);
        if let Some(line) = doing {
            let was = inner.doing.get(worker).cloned().unwrap_or_default();
            inner
                .doing
                .insert(worker.into(), next_doing(&was, line, now));
        }
        Ok(())
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Fault> {
        let line = doing_line(text).map_err(Fault::Doing)?;
        let now = self.now.get();
        let mut inner = self.inner.borrow_mut();
        if !inner.made {
            return Err(Fault::Missing);
        }
        let listening = inner
            .waits
            .values()
            .any(|w| w.worker == worker && heard_in_time(w.heard, now));
        if !listening {
            return Err(Fault::NotListening);
        }
        let was = inner.doing.get(worker).cloned().unwrap_or_default();
        inner
            .doing
            .insert(worker.into(), next_doing(&was, line, now));
        Ok(())
    }

    fn waits(&self) -> Result<Vec<Wait>, Fault> {
        if !self.inner.borrow().made {
            return Err(Fault::Missing);
        }
        Ok(self.live_waits())
    }

    fn listeners(&self) -> Result<Vec<Listening>, Fault> {
        let waits = self.waits()?;
        let inner = self.inner.borrow();
        let mut workers: BTreeMap<String, Listening> = BTreeMap::new();
        for wait in waits {
            let (line, at) = inner.doing.get(&wait.worker).cloned().unwrap_or_default();
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
            .map(|mut l| {
                l.labels.sort();
                l.labels.dedup();
                l
            })
            .collect())
    }
}
