//! The storage surface a store implements.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::case::{Append, Case, Row};
use crate::error::Error;
use crate::key::Key;
use crate::listening::{Listening, Wait};

/// Where cases are kept: what the CLI and the web inbox need from a store.
///
/// A store keeps each case's rows as the writer made them, and never a case's
/// state: a reader folds the rows with [`Case::fold`]. A write runs the
/// writer, [`Case::open`] or [`Case::append`], inside the store's own lock,
/// with the store's clock for `now` and the actor the store keeps for who
/// writes. So the revision the writer checks is still the case's when the
/// row is stored, a time the writer stamps follows the order of storage, and
/// each event records who wrote it.
///
/// Beside the cases, a store keeps what is not an event: the table of
/// actors, which names who wrote each event ([`Store::actors`]); the
/// sessions listening for messages and what each is doing (ST-16,
/// [`Store::listen`], [`Store::set_doing`], [`Store::waits`],
/// [`Store::listeners`]); and a version that moves on every event, archive
/// and delete, and never on listening ([`Store::version`]), with the cases
/// changed since one ([`Store::rows_since`]). A reader that polls a store,
/// as `cases wait` does, reads the version and reads cases again only when
/// it has moved.
///
/// The methods are synchronous. A local store is one SQLite file, and a
/// hosted store keeps its cases in a Durable Object, whose SQL calls are
/// synchronous too, and a client of a hosted store can block on HTTP. An
/// asynchronous trait can be added beside this one if a store needs it.
///
/// # Example
///
/// A store that keeps its cases in memory, with a clock that never moves.
/// Who writes is an actor the caller hands it:
///
/// ```
/// use std::cell::{Cell, RefCell};
/// use std::convert::Infallible;
///
/// use cases_core::{
///     Actor, ActorRow, Answer, Append, Author, Case, Category, Changes, Error, Listening, Open,
///     Record, Row, State, Store, StoreError, Time, Version, Wait,
/// };
///
/// struct Memory {
///     /// Each case's rows, by key: `None` for a case taken out.
///     cases: RefCell<Vec<Option<Vec<Row>>>>,
///     archive: RefCell<Vec<(usize, Vec<Row>)>>,
///     /// Raised by every event, archive and delete.
///     changes: Cell<u64>,
///     now: Time,
/// }
///
/// impl Memory {
///     /// Takes the case `key` out, if it is still at `at_revision`.
///     fn take(&self, key: usize, at_revision: Option<u64>) -> Result<Vec<Row>, StoreError<Infallible>> {
///         let mut cases = self.cases.borrow_mut();
///         let case = cases.get_mut(key).ok_or(StoreError::NoCase)?;
///         let revision = case.as_ref().ok_or(StoreError::NoCase)?.len() as u64;
///         if let Some(read) = at_revision
///             && read != revision
///         {
///             return Err(Error::stale(read, revision).into());
///         }
///         self.changes.set(self.changes.get() + 1);
///         Ok(case.take().unwrap_or_default())
///     }
/// }
///
/// impl Store for Memory {
///     type Key = usize;
///     type Writer = Actor;
///     type Error = Infallible;
///
///     fn create(&self, open: &Append, writer: &Actor) -> Result<(usize, Case), StoreError<Infallible>> {
///         let (case, row) = Case::open(open, self.now, Some(writer))?;
///         let mut cases = self.cases.borrow_mut();
///         cases.push(Some(vec![row]));
///         self.changes.set(self.changes.get() + 1);
///         Ok((cases.len() - 1, case))
///     }
///
///     fn append(
///         &self,
///         key: &usize,
///         append: &Append,
///         writer: &Actor,
///         at_revision: Option<u64>,
///     ) -> Result<Case, StoreError<Infallible>> {
///         // The borrow is the lock: no other write comes between the fold
///         // and the stored row.
///         let mut cases = self.cases.borrow_mut();
///         let rows = cases.get_mut(*key).and_then(Option::as_mut).ok_or(StoreError::NoCase)?;
///         let mut case = Case::fold(rows)?;
///         rows.push(case.append(append, self.now, Some(writer), at_revision)?);
///         self.changes.set(self.changes.get() + 1);
///         Ok(case)
///     }
///
///     fn rows(&self, key: &usize) -> Result<Option<Vec<Row>>, Infallible> {
///         Ok(self.cases.borrow().get(*key).cloned().flatten())
///     }
///
///     fn list(&self) -> Result<Vec<(usize, Vec<Row>)>, Infallible> {
///         let cases = self.cases.borrow();
///         let live = cases.iter().enumerate();
///         Ok(live.filter_map(|(key, rows)| Some((key, rows.clone()?))).collect())
///     }
///
///     fn archive(&self, key: &usize, at_revision: Option<u64>) -> Result<(), StoreError<Infallible>> {
///         let rows = self.take(*key, at_revision)?;
///         self.archive.borrow_mut().push((*key, rows));
///         Ok(())
///     }
///
///     fn delete(&self, key: &usize, at_revision: Option<u64>) -> Result<(), StoreError<Infallible>> {
///         self.take(*key, at_revision).map(drop)
///     }
///
///     fn version(&self) -> Result<Version, Infallible> {
///         Ok(Version::new(self.changes.get().to_string()))
///     }
///
///     // Every case when anything moved: a store may always give them all.
///     fn rows_since(&self, since: Option<&Version>) -> Result<Changes<usize>, Infallible> {
///         let version = self.version()?;
///         if since == Some(&version) {
///             return Ok(Changes::Unchanged { version });
///         }
///         Ok(Changes::All { version, cases: self.list()? })
///     }
///
///     // Nobody listens to a store in memory, and its actors are the ones
///     // the caller holds.
///     fn actors(&self) -> Result<Vec<ActorRow>, Infallible> {
///         Ok(Vec::new())
///     }
///
///     fn listen(&self, _: &str, _: &str, _: &[String], _: &[String], _: Option<&str>) -> Result<(), Infallible> {
///         Ok(())
///     }
///
///     fn set_doing(&self, _: &str, _: &str) -> Result<(), Infallible> {
///         Ok(())
///     }
///
///     fn waits(&self) -> Result<Vec<Wait>, Infallible> {
///         Ok(Vec::new())
///     }
///
///     fn listeners(&self) -> Result<Vec<Listening>, Infallible> {
///         Ok(Vec::new())
///     }
/// }
///
/// let store = Memory {
///     cases: RefCell::default(),
///     archive: RefCell::default(),
///     changes: Cell::new(0),
///     now: "2026-09-15T09:01:00Z".parse().unwrap(),
/// };
/// let mut agent = Actor::default();
/// agent.id = "hV3kQ9mZr2pX7wLc0tBnYA".into();
/// let mut human = Actor::default();
/// human.id = "Q2xbTPWyJh8kVs3dNr6aLg".into();
///
/// let mut open = Open::default();
/// open.kind = "fyi".into();
/// open.urgency = "today".into();
/// open.title = "Nightly build moved".into();
/// let open = Append::new(Author::Agent, &Record::Open(open));
/// let before = store.version().unwrap();
/// let (key, case) = store.create(&open, &agent).unwrap();
/// assert_eq!(case.actor(), Some(&agent));
/// // The open moved the version, so a reader at the old one reads again.
/// let now = store.version().unwrap();
/// assert_ne!(now, before);
/// assert_eq!(store.rows_since(Some(&before)).unwrap().cases().len(), 1);
/// assert!(store.rows_since(Some(&now)).unwrap().cases().is_empty());
///
/// let mut answer = Answer::default();
/// answer.ack = true;
/// let answer = Append::new(Author::Human, &Record::Answer(answer));
/// let case = store.append(&key, &answer, &human, Some(case.revision())).unwrap();
/// assert_eq!(case.state(), State::Answered);
/// assert_eq!(case.answer().unwrap().actor.as_ref(), Some(&human));
///
/// // A second answer from the same read is stale, and stores nothing.
/// let Err(StoreError::Refused(error)) = store.append(&key, &answer, &human, Some(1)) else {
///     panic!("the stale answer was stored");
/// };
/// assert_eq!(error.category(), Category::Stale);
/// assert_eq!(store.rows(&key).unwrap().unwrap().len(), 2);
///
/// // Archived, the case is out of sight, and its rows are kept.
/// store.archive(&key, Some(2)).unwrap();
/// assert_eq!(store.rows(&key).unwrap(), None);
/// assert_eq!(store.archive.borrow()[0].1.len(), 2);
/// ```
pub trait Store {
    /// Names a case in the store, such as its numeric primary key. The store
    /// and the CLI map it to the case's public id.
    type Key;
    /// Who writes, as the store tells writers apart, such as an agent by
    /// its worker name. The store keeps an actor for each writer, and gives
    /// it to each record the writer writes.
    type Writer;
    /// What can go wrong in the store itself, such as a file that does not
    /// open.
    type Error;

    /// Opens a new case: runs [`Case::open`] on `open`, with the actor for
    /// `writer`, stores the row it gives, and returns the new case's key and
    /// the case.
    ///
    /// # Errors
    ///
    /// Gives [`StoreError::Refused`], with the category [`Case::open`] gives,
    /// when the open is refused, and [`StoreError::Store`] when the store
    /// fails. Either way nothing is stored.
    fn create(
        &self,
        open: &Append,
        writer: &Self::Writer,
    ) -> Result<(Self::Key, Case), StoreError<Self::Error>>;

    /// Appends an event to the case `key`: folds the case's rows, runs
    /// [`Case::append`] with `append`, the actor for `writer` and
    /// `at_revision`, and stores the row it gives, all under the store's
    /// lock. Returns the case after the event. A refused event stores
    /// nothing.
    ///
    /// # Errors
    ///
    /// Gives [`StoreError::NoCase`] when the store has no case `key`;
    /// [`StoreError::Refused`] when the case's rows do not fold (as
    /// [`Category::NoOpen`](crate::Category::NoOpen)) or [`Case::append`]
    /// refuses the event, with the category it gives; and
    /// [`StoreError::Store`] when the store fails. Either way nothing is
    /// stored.
    fn append(
        &self,
        key: &Self::Key,
        append: &Append,
        writer: &Self::Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Self::Error>>;

    /// The rows of the case `key`, in seq order, or `None` when the store has
    /// no such case.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store fails.
    fn rows(&self, key: &Self::Key) -> Result<Option<Vec<Row>>, Self::Error>;

    /// Every case in the store, as its key and its rows, in the order the
    /// cases were opened.
    ///
    /// Filters, such as by state, kind or label, run on the folded cases,
    /// since a case's state is never stored. For now a store lists every
    /// case. A later version of this trait may let a store filter or page the
    /// list itself, so that a hosted store need not read every case.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store fails.
    #[expect(
        clippy::type_complexity,
        reason = "each case's key beside its rows, spelt out, reads more plainly than an alias"
    )]
    fn list(&self) -> Result<Vec<(Self::Key, Vec<Row>)>, Self::Error>;

    /// Moves the case `key` and its rows into the store's archive, under the
    /// store's lock. The store no longer lists the case or gives its rows,
    /// keeps the rows as written, and never gives its key to another case.
    ///
    /// With `at_revision`, the revision the case was read at, the case is
    /// moved only if it still has that revision, as for [`Store::append`].
    ///
    /// # Errors
    ///
    /// Gives [`StoreError::NoCase`] when the store has no case `key`;
    /// [`StoreError::Refused`], as
    /// [`Category::Stale`](crate::Category::Stale), when the case's revision
    /// is not `at_revision`; and [`StoreError::Store`] when the store fails.
    /// Either way the case stays where it is.
    fn archive(
        &self,
        key: &Self::Key,
        at_revision: Option<u64>,
    ) -> Result<(), StoreError<Self::Error>>;

    /// Removes the case `key` and its rows from the store, under the store's
    /// lock. It cannot be undone, and the key is never given to another case.
    ///
    /// With `at_revision`, the case is removed only if it still has that
    /// revision, as for [`Store::archive`].
    ///
    /// # Errors
    ///
    /// Gives the errors [`Store::archive`] gives, when it would. Either way
    /// the case stays where it is.
    fn delete(
        &self,
        key: &Self::Key,
        at_revision: Option<u64>,
    ) -> Result<(), StoreError<Self::Error>>;

    /// The store's version: it moves on every event stored, and on every
    /// case archived or deleted, and never on listening (ST-16), nor on a
    /// write that is refused or fails, which stores nothing.
    ///
    /// It is opaque: a reader only asks whether two versions are the same.
    /// So a store must never give one version for two states of its cases
    /// that differ: a reader that sees the version it last read takes it
    /// that nothing changed, and reads nothing again. A store that has
    /// never stored a case, or is not made yet, may give `0`.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store fails.
    fn version(&self) -> Result<Version, Self::Error>;

    /// The cases changed since the version `since`, with the version now,
    /// for a reader that keeps the cases it read at `since` and wants only
    /// what moved. The answer is one of three:
    ///
    /// - **Nothing changed** ([`Changes::Unchanged`]): `since` is the
    ///   version now.
    /// - **What changed** ([`Changes::Since`]): `cases` holds each case with
    ///   a row stored after `since`, with all its rows, and `removed` the key
    ///   of each case archived or deleted after `since`. `removed` may name a
    ///   case opened after `since`, which the reader never had; a key is
    ///   never in both, as a store never gives a key twice.
    /// - **Every case** ([`Changes::All`]): as [`Store::list`] gives them,
    ///   and the reader drops every case it holds that is not among them.
    ///
    /// Every case is always a correct answer, so a store that cannot work
    /// out what changed, or no longer knows (it kept no record of what was
    /// removed, or `since` is older than the record it keeps, or is a
    /// version it never gave), gives every case. With no `since`, it gives
    /// every case. The cases are in the order they were opened.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store fails.
    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Self::Key>, Self::Error>;

    /// Every actor in the store, in the order the store made them. A reader
    /// looks up an event's actor id here for its kind and name; an id that
    /// is not here is one whose row was deleted (ST-12).
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store is not made yet, or fails.
    fn actors(&self) -> Result<Vec<ActorRow>, Self::Error>;

    /// Records the wait `wait` of `worker` as listening now, on `labels`
    /// and `kinds`, with the doing line `doing` when given: the local
    /// store's heartbeat, which a listening wait sends every [`HEARTBEAT`]
    /// (ST-16). A worker still listening keeps when it began and its doing
    /// line. A listener never makes a store: a store not made yet records
    /// nothing, and gives its error.
    ///
    /// A hosted store counts a worker as listening while one of its waits
    /// holds a poll open (ST-16), and learns the wait's worker, labels,
    /// kinds and start from the poll, which is outside this trait. It may
    /// take `listen` as recording nothing; its [`Store::waits`] and
    /// [`Store::listeners`] then give the waits whose polls are open, and
    /// [`Store::set_doing`] takes a worker as listening while one is.
    ///
    /// [`HEARTBEAT`]: crate::HEARTBEAT
    ///
    /// # Errors
    ///
    /// Gives the store's error when `worker` is blank, `doing` is not a
    /// doing line (AG-17), the store is not made yet, or the store fails.
    /// Nothing is recorded then.
    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Self::Error>;

    /// Sets what `worker` says it is doing to `text`: blank clears it, and
    /// the same line again keeps the time it was first set (AG-17).
    ///
    /// A doing line is `text` trimmed: at most 80 characters, with no line
    /// break or other control character, and none that sets the direction
    /// of the text after it (U+2028, U+2029, U+202A to U+202E, U+2066 to
    /// U+2069), which a page showing the line would carry on past its end.
    /// The same rule holds for `listen`'s `doing`. A line that is not one is
    /// refused as the core's [`doing_line`](crate::doing_line) refuses it,
    /// before anything else is checked: whether the store is made, and
    /// whether the worker listens.
    ///
    /// # Errors
    ///
    /// Gives the store's error when `text` is not a doing line, `worker` is
    /// not listening now, the store is not made yet, or the store fails.
    /// Nothing is written then.
    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Self::Error>;

    /// The waits listening now, by the store's clock, by worker and then by
    /// id.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store is not made yet, or fails.
    fn waits(&self) -> Result<Vec<Wait>, Self::Error>;

    /// The workers listening now, by name: each with the labels of all its
    /// waits listening now, when the first of them began, when one was last
    /// heard, and its doing line.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the store is not made yet, or fails.
    fn listeners(&self) -> Result<Vec<Listening>, Self::Error>;
}

/// A store's version, as [`Store::version`] gives it: opaque text, which
/// moves on every event, archive and delete. Two versions are compared only
/// for being the same.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version(String);

impl Version {
    /// The version `text`, as a store writes it.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Version {
        Version(text.into())
    }

    /// The version as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The cases changed since a version, as [`Store::rows_since`] gives them:
/// one of three answers, each with the store's version now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changes<K> {
    /// Nothing changed: the store is still at the version asked about.
    Unchanged {
        /// The store's version now.
        version: Version,
    },
    /// What changed since the version asked about.
    Since {
        /// The store's version now.
        version: Version,
        /// Each case with a row stored since, with all its rows, in the
        /// order the cases were opened.
        cases: Vec<(K, Vec<Row>)>,
        /// The key of each case archived or deleted since. It may name a
        /// case opened since, which the reader never had.
        removed: Vec<K>,
    },
    /// Every case in the store: the reader drops each case it holds that is
    /// not among them.
    All {
        /// The store's version now.
        version: Version,
        /// Every case, with its rows, in the order the cases were opened.
        cases: Vec<(K, Vec<Row>)>,
    },
}

impl<K> Changes<K> {
    /// The store's version now.
    #[must_use]
    pub fn version(&self) -> &Version {
        match self {
            Changes::Unchanged { version }
            | Changes::Since { version, .. }
            | Changes::All { version, .. } => version,
        }
    }

    /// The cases the answer gives: the changed ones, or every one, or none.
    #[must_use]
    pub fn cases(&self) -> &[(K, Vec<Row>)] {
        match self {
            Changes::Unchanged { .. } => &[],
            Changes::Since { cases, .. } | Changes::All { cases, .. } => cases,
        }
    }
}

/// Each case in a store, by key, folded: the case, or why it does not fold.
pub type Cases = BTreeMap<Key, Result<Case, Error>>;

/// Who writes: a name, or none.
///
/// The event's author says whether the writer is an agent or a human. A
/// store keeps one actor for each of those kinds and each name, made the
/// first time the writer writes, and one unnamed actor for each kind. Each
/// event it writes carries its writer's actor, as `actor: {id}` (ST-12). An
/// agent is named by its worker, and a human by their name (AG-01, HU-06).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Writer {
    name: Option<String>,
}

impl Writer {
    /// The writer named `name`. No name, or an empty one, is the unnamed
    /// writer.
    #[must_use]
    pub fn new(name: Option<&str>) -> Writer {
        Writer {
            name: name.filter(|name| !name.is_empty()).map(String::from),
        }
    }

    /// The writer's name, if it has one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// An actor, as a store's table of actors keeps it (ST-12).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActorRow {
    /// The id events carry: 22 base64url characters, from 128 random bits.
    pub id: String,
    /// `agent` or `human`.
    pub kind: String,
    /// The actor's name, or `None` for its kind's unnamed actor.
    pub name: Option<String>,
    /// When the store made the actor: an RFC 3339 time in UTC.
    pub created: String,
}

impl ActorRow {
    /// The actor `id`, of `kind`, named `name`, made at `created`.
    #[must_use]
    pub fn new(id: String, kind: String, name: Option<String>, created: String) -> ActorRow {
        ActorRow {
            id,
            kind,
            name,
            created,
        }
    }
}

/// Why a store did not make a write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError<E> {
    /// The writer refused the event, the case's rows do not fold, or the
    /// case changed since the revision the write was made at. Nothing was
    /// stored.
    Refused(Error),
    /// The store has no case with this key.
    NoCase,
    /// The store failed. Nothing was stored.
    Store(E),
}

impl<E> From<Error> for StoreError<E> {
    fn from(error: Error) -> StoreError<E> {
        StoreError::Refused(error)
    }
}

impl<E: fmt::Display> fmt::Display for StoreError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Refused(error) => error.fmt(f),
            StoreError::NoCase => f.write_str("the store has no such case"),
            StoreError::Store(error) => error.fmt(f),
        }
    }
}

/// Displays as the error it holds, and gives that error's source.
impl<E: core::error::Error> core::error::Error for StoreError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            StoreError::Refused(error) => core::error::Error::source(error),
            StoreError::NoCase => None,
            StoreError::Store(error) => error.source(),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;
    use core::error::Error as _;

    use super::*;

    #[derive(Debug)]
    struct Disk;

    impl fmt::Display for Disk {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the disk is full")
        }
    }

    impl core::error::Error for Disk {}

    #[derive(Debug)]
    struct Write(Disk);

    impl fmt::Display for Write {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the row was not written")
        }
    }

    impl core::error::Error for Write {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn a_writer_s_blank_name_is_no_name() {
        assert_eq!(Writer::new(Some("lead")).name(), Some("lead"));
        assert_eq!(Writer::new(Some("")), Writer::new(None));
        assert_eq!(Writer::new(None).name(), None);
    }

    #[test]
    fn changes_give_their_version_and_cases() {
        let row = Row {
            seq: 1,
            author: "agent".into(),
            event: "open".into(),
            data: "{}".into(),
        };
        let v = Version::new("7");
        assert_eq!(v.as_str(), "7");
        assert_eq!(v.to_string(), "7");
        let none: Changes<u64> = Changes::Unchanged { version: v.clone() };
        assert_eq!((none.version(), none.cases().len()), (&v, 0));
        let every = Changes::All {
            version: v.clone(),
            cases: vec![(1, vec![row.clone()])],
        };
        assert_eq!((every.version(), every.cases().len()), (&v, 1));
        let some = Changes::Since {
            version: Version::new("8"),
            cases: vec![(2, vec![row.clone()]), (3, vec![row])],
            removed: vec![1],
        };
        assert_eq!(some.version().as_str(), "8");
        assert_eq!(some.cases().len(), 2);
        let actor = ActorRow::new("id".into(), "agent".into(), None, "t".into());
        assert_eq!((actor.id.as_str(), actor.kind.as_str()), ("id", "agent"));
        assert_eq!((actor.name, actor.created.as_str()), (None, "t"));
    }

    #[test]
    fn a_store_error_displays_as_the_error_it_holds() {
        let refused: StoreError<Write> = Error::invalid("the title is blank").into();
        assert_eq!(refused.to_string(), "the title is blank");
        assert_eq!(
            StoreError::<Write>::NoCase.to_string(),
            "the store has no such case"
        );
        let failed = StoreError::Store(Write(Disk));
        assert_eq!(failed.to_string(), "the row was not written");
        assert_eq!(failed.source().unwrap().to_string(), "the disk is full");
        assert!(refused.source().is_none());
    }
}
