//! Which store a command uses (ST-11, SV-01, AC-04): a local file, a store
//! on another machine reached over ssh or a command (a forwarded store), or
//! a store on a server, found by its id or by its name in the account's
//! list of stores, which is kept in the state directory. Each kind is reached through the
//! core's [`Store`] trait, as [`AnyStore`].

use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(feature = "remote")]
use std::sync::Arc;
use std::time::{Duration, Instant};

use cases_core::{
    ActorRow, Append, Case, Cases, Changes, Key, Listening, Row, Store, StoreError, Version, Wait,
    Writer,
};
use cases_store_remote::Error as RemoteError;
use cases_store_sqlite::{Db, Error as DbError};

use crate::config::{self, Config};
use crate::error::Error;
use crate::forward::{Forward, Forwarded};
pub(crate) use crate::hosted::*;
use crate::tokens;

/// The store a command was pointed at, before anything is read: a file, or
/// a store on a server, named by its id or its name, or left to the
/// account's default.
#[derive(Debug)]
pub(crate) enum Selection {
    /// A local store: its file.
    Local(PathBuf),
    /// The store on another machine, over ssh or a command.
    Forward(Forward),
    /// A store on `server`. Without the `remote` feature there is none.
    #[cfg_attr(not(feature = "remote"), allow(dead_code))]
    Remote {
        /// The server.
        server: Server,
        /// Which of the account's stores.
        which: Which,
    },
}

/// Where a command's store is.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// A local store's file.
    Local {
        /// The file, as given, with a leading `~` expanded.
        path: PathBuf,
    },
    /// The store on another machine, over ssh or a command.
    Forward {
        /// Where it is.
        forward: Forward,
    },
    /// A store on a server. Without the `remote` feature there is none.
    #[cfg_attr(not(feature = "remote"), allow(dead_code))]
    Remote {
        /// The server.
        server: Server,
        /// The store.
        store: StoreRef,
    },
    /// A store on a server that the command does not reach, so it was not
    /// looked up and needed no token: `config`, `skill`, `serve` and
    /// `service`.
    Server {
        /// The server.
        server: Server,
    },
}

/// The store for people: a local store's file, or a remote store's name and
/// id on its server.
impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Local { path } => path.display().fmt(f),
            Target::Forward { forward } => write!(f, "the store on {forward}"),
            Target::Remote { server, store } => write!(f, "{store} on {server}"),
            Target::Server { server } => write!(f, "a store on {server}"),
        }
    }
}

impl Target {
    /// The local store's file, if the store is local.
    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Target::Local { path } => Some(path),
            Target::Forward { .. } | Target::Remote { .. } | Target::Server { .. } => None,
        }
    }
}

/// The store `flag`, `CASES_STORE`, the config file's `store`, the forward
/// (`CASES_FORWARD`, the config file's `forward`), the server
/// (`CASES_SERVER`, the config file's `server`) and `default-store` point
/// at.
///
/// A forward is taken before a store given at the same level or below:
/// `CASES_FORWARD` before `CASES_STORE` and the config file's `store`, and
/// the config file's `forward` before its `store`. `--store` is always
/// taken, and `CASES_STORE` before the config file's `forward`. A forward
/// is taken before a server.
///
/// The store given is then read in ST-11's order:
///
/// 1. a forward, `ssh://[user@]host[:port]` or `exec:<command>`: the
///    store on that machine;
/// 2. a one-off URL, `https://host/<name or id>` (scheme in any case,
///    `http` only for a loopback host): that store on that server;
/// 3. a path, with a `/` or ending in `.db`: a local store, even with a
///    server set;
/// 4. with a server set, a store in the account, by id or name;
/// 5. otherwise a path.
///
/// With no store given, it is the account's default with a server set, and
/// the default local file without.
///
/// # Errors
///
/// When a forward, a URL or the server is not one, or no store is given and
/// the default file cannot be found.
pub(crate) fn select(flag: Option<&str>, config: &Config) -> Result<Selection, Error> {
    let server = || server(config);
    let store = config::store_text(flag, config);
    if let Some((text, from)) = config::forward_text(config) {
        let taken = match &store {
            Some((_, "flag")) => false,
            Some((_, "env")) => from == "env",
            // The config file's store, or none.
            _ => true,
        };
        if taken {
            let forward = text.parse().map_err(|error: Error| {
                let key = if from == "env" {
                    config::FORWARD_ENV
                } else {
                    "the config file's forward"
                };
                Error::new(format!("{key}: {error}"))
            })?;
            return Ok(Selection::Forward(forward));
        }
    }
    let Some((text, _)) = store else {
        return Ok(match server()? {
            Some(server) => on_server(server, None),
            None => Selection::Local(config::default_store_path()?),
        });
    };
    if Forward::is_forward(&text) {
        return Ok(Selection::Forward(text.parse()?));
    }
    if let Some(selection) = one_off(&text)? {
        return Ok(selection);
    }
    if config::is_path(&text) {
        return Ok(Selection::Local(config::expand_home(&text)));
    }
    Ok(match server()? {
        Some(server) => on_server(server, Some(&text)),
        None => Selection::Local(config::expand_home(&text)),
    })
}

/// The local store `flag`, `CASES_STORE` or the config file's `store`
/// names, else the default file: never a forward or a server, for
/// `dial-stdio`.
///
/// # Errors
///
/// When the store given is an address, not a file, or none is given and the
/// default file cannot be found.
pub(crate) fn local_path(flag: Option<&str>, config: &Config) -> Result<PathBuf, Error> {
    match config::store_text(flag, config) {
        Some((text, _)) if text.contains("://") || Forward::is_forward(&text) => Err(Error::new(
            format!("{text:?} is not a local store's file: dial-stdio serves a local store"),
        )),
        Some((text, _)) => Ok(config::expand_home(&text)),
        None => config::default_store_path(),
    }
}

/// Opens what `selection` names: a local store at once, since nothing is
/// read until a command reads it, and a remote one with the token for its
/// server, by its id, or by its name or the default from the account's list
/// of stores, read from the state directory or the server.
///
/// # Errors
///
/// When there is no token for the server, the list cannot be read, the
/// name is not in it, or no store is named and the account has no single
/// store to use.
pub(crate) fn connect(selection: Selection, config: &Config) -> Result<(Target, AnyStore), Error> {
    match selection {
        Selection::Local(path) => {
            let db = Db::new(&path);
            Ok((Target::Local { path }, AnyStore::Local(db)))
        }
        Selection::Forward(forward) => {
            let token = match (&forward, config::forward_token_file_text(config)) {
                (Forward::Http(_), Some((path, from))) => {
                    let key = if from == "env" {
                        config::FORWARD_TOKEN_FILE_ENV
                    } else {
                        "the config file's forward-token-file"
                    };
                    tokens::read(&path, key)?.into_iter().next()
                }
                _ => None,
            };
            let store = Forwarded::new(&forward, token);
            Ok((
                Target::Forward { forward },
                AnyStore::Forward(Box::new(store)),
            ))
        }
        Selection::Remote { server, which } => crate::hosted::connect(server, which, config),
    }
}

/// What went wrong in a store of either kind.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The local store's.
    Local(DbError),
    /// The remote store's, with the store it was asked of.
    #[cfg(feature = "remote")]
    Remote {
        /// What went wrong.
        error: RemoteError,
        /// The store, as `name (id) on server`.
        store: String,
        /// The server, and where its token came from.
        origin: Arc<Origin>,
    },
    /// The forwarded store's, with where it is.
    Forward {
        /// What went wrong.
        error: RemoteError,
        /// Where the store is, `ssh://host` or `exec:<command>`.
        forward: String,
    },
    /// The store was named, and the account's list, read again after the
    /// server refused it, no longer has the name.
    Named(String),
}

impl Failure {
    /// Whether the local store is not made yet, which a read takes as no
    /// cases (ST-08).
    pub(crate) fn is_missing(&self) -> bool {
        matches!(
            self,
            Failure::Local(DbError::Missing { .. })
                | Failure::Forward {
                    error: RemoteError::Missing,
                    ..
                }
        )
    }

    /// Whether the store was made by a newer cases than the one that reads
    /// it: this one, or for a forwarded store the one on the far side
    /// (ST-07, TU-01).
    pub(crate) fn is_newer(&self) -> bool {
        matches!(
            self,
            Failure::Local(DbError::Newer { .. })
                | Failure::Forward {
                    error: RemoteError::Newer { .. },
                    ..
                }
        )
    }

    /// The error of a forwarded or remote store, if it is one.
    fn remote_error(&self) -> Option<&RemoteError> {
        match self {
            Failure::Forward { error, .. } => Some(error),
            #[cfg(feature = "remote")]
            Failure::Remote { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Whether the remote or forwarded store says the worker is not
    /// listening.
    pub(crate) fn not_listening(&self) -> bool {
        matches!(self.remote_error(), Some(RemoteError::NotListening))
    }

    /// Whether the same read may work if it is tried again later: the
    /// remote or forwarded store gave no answer, limited requests, or
    /// answered 502, 503 or 504.
    pub(crate) fn is_transient(&self) -> bool {
        self.remote_error().is_some_and(RemoteError::is_transient)
    }

    /// Whether the remote or forwarded store gave no answer at all, so that
    /// a write may or may not have been stored.
    pub(crate) fn no_answer(&self) -> bool {
        matches!(self.remote_error(), Some(RemoteError::Transport { .. }))
    }

    /// How long the remote store asked to be left, when it limited
    /// requests and said.
    pub(crate) fn retry_after(&self) -> Option<Duration> {
        match self.remote_error() {
            Some(RemoteError::Rate { retry_after }) => *retry_after,
            _ => None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Local(error) => error.fmt(f),
            #[cfg(feature = "remote")]
            Failure::Remote {
                error: RemoteError::Auth { message },
                store,
                origin,
            } => write!(
                f,
                "the server refused {} for the store {store}: {message}{}",
                origin.source.the_token(),
                origin.hint()
            ),
            #[cfg(feature = "remote")]
            Failure::Remote {
                error: RemoteError::Gone,
                store,
                ..
            } => write!(f, "the store {store} was removed"),
            #[cfg(feature = "remote")]
            Failure::Remote {
                error: RemoteError::NoStore,
                store,
                ..
            } => write!(f, "there is no store {store}"),
            #[cfg(feature = "remote")]
            Failure::Remote { error, .. } => error.fmt(f),
            Failure::Forward {
                error: RemoteError::Transport { reason, .. } | RemoteError::Dial { reason, .. },
                forward,
            } => write!(f, "could not reach the store on {forward}: {reason}"),
            // Over HTTP, the cases serve on the far side let the request
            // in only with a token or a listed login, and says which.
            Failure::Forward {
                error: RemoteError::Auth { message },
                forward,
            } => write!(
                f,
                "the cases serve on {forward} refused this request: {message}"
            ),
            Failure::Forward {
                error: RemoteError::Missing,
                forward,
            } => write!(
                f,
                "the store on {forward} is not made yet: opening a case makes it"
            ),
            // The far side serves one store, whose id never changes: a 404
            // `request` is a route it does not have, from a cases older than
            // this one.
            Failure::Forward {
                error: RemoteError::NoStore,
                forward,
            } => write!(
                f,
                "the cases on {forward} does not serve this request: it may be older than this \
                 one ({}); update it there",
                env!("CARGO_PKG_VERSION")
            ),
            Failure::Forward {
                error: RemoteError::Server { message, .. },
                forward,
            } if !message.is_empty() => write!(f, "the store on {forward} failed: {message}"),
            Failure::Forward { error, forward } => write!(f, "{error} (the store on {forward})"),
            Failure::Named(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Failure {}

/// A store of either kind: what every command reads and writes through.
#[derive(Debug)]
pub(crate) enum AnyStore {
    /// A local store.
    Local(Db),
    /// A store on another machine, over ssh or a command.
    Forward(Box<Forwarded>),
    /// A store on a server. Without the `remote` feature there is none.
    #[cfg_attr(not(feature = "remote"), allow(dead_code))]
    Remote(Box<Remote>),
}

/// A poller over a store of either kind (see [`cases_core::Poller`]).
#[derive(Debug)]
pub(crate) enum Poller<'a> {
    /// Over a local store: a store not made yet is an error, which the
    /// caller takes as no cases.
    Local(cases_store_sqlite::Poller<'a>),
    /// Over a forwarded store.
    Forward(cases_core::Poller<'a, Forwarded>),
    /// Over a remote store.
    Remote(cases_core::Poller<'a, Remote>),
}

impl Poller<'_> {
    /// The version the cases were last read at, or `None` before the first
    /// read and after one that failed.
    pub(crate) fn version(&self) -> Option<Version> {
        match self {
            Poller::Local(poller) => poller.version().cloned(),
            Poller::Forward(poller) => poller.version().cloned(),
            Poller::Remote(poller) => poller.version().cloned(),
        }
    }

    /// Every case in the store, each folded, reading only what changed.
    ///
    /// # Errors
    ///
    /// When the read fails, or a local store is not made yet.
    pub(crate) fn poll(&mut self) -> Result<&Cases, Failure> {
        match self {
            Poller::Local(poller) => poller.poll().map_err(Failure::Local),
            Poller::Forward(poller) => poller.poll(),
            Poller::Remote(poller) => poller.poll(),
        }
    }
}

impl AnyStore {
    /// A remote store's server and the store, as it is now: after a rename
    /// was followed, the store the name has now. Without the `remote`
    /// feature there is none, and the arm is code that cannot run.
    #[cfg_attr(not(feature = "remote"), allow(unreachable_code))]
    pub(crate) fn hosted(&self) -> Option<(Server, StoreRef)> {
        match self {
            AnyStore::Local(_) | AnyStore::Forward(_) => None,
            AnyStore::Remote(remote) => Some((remote.server().clone(), remote.current().1)),
        }
    }

    /// The store a held poll can follow from a thread of its own: a
    /// forwarded store, or a remote one as it is now, after a rename was
    /// followed. A local store has none. Without the `remote` feature no
    /// store is remote, and the arm is code that cannot run.
    pub(crate) fn followed(&self) -> Option<cases_store_remote::RemoteStore> {
        match self {
            AnyStore::Local(_) => None,
            AnyStore::Forward(forwarded) => Some(forwarded.store()),
            #[cfg(feature = "remote")]
            AnyStore::Remote(remote) => Some(remote.current().0),
            #[cfg(not(feature = "remote"))]
            AnyStore::Remote(remote) => match **remote {},
        }
    }

    /// Whether the store has a held poll ([`AnyStore::changes`]): a remote
    /// or a forwarded one.
    pub(crate) fn follows(&self) -> bool {
        !matches!(self, AnyStore::Local(_))
    }

    /// The held poll of a remote or a forwarded store: its version as soon
    /// as it is not `after`, or after `hold`, at most 25 s. A local store
    /// has none, and gives its version at once.
    ///
    /// # Errors
    ///
    /// When the request fails.
    pub(crate) fn changes(&self, after: &Version, hold: Duration) -> Result<Version, Failure> {
        match self {
            AnyStore::Local(db) => db.version().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.changes(after, hold),
            AnyStore::Remote(remote) => remote.changes(after, hold),
        }
    }

    /// Keeps a remote or forwarded store's requests, and their retries,
    /// within `deadline`, such as a wait's timeout. A local store has no
    /// requests.
    pub(crate) fn until(&self, deadline: Option<Instant>) {
        match self {
            AnyStore::Local(_) => {}
            AnyStore::Forward(forwarded) => forwarded.until(deadline),
            AnyStore::Remote(remote) => remote.until(deadline),
        }
    }

    /// A poller over the store.
    pub(crate) fn poller(&self) -> Poller<'_> {
        match self {
            AnyStore::Local(db) => Poller::Local(db.poller()),
            AnyStore::Forward(forwarded) => Poller::Forward(cases_core::Poller::new(&**forwarded)),
            AnyStore::Remote(remote) => Poller::Remote(cases_core::Poller::new(&**remote)),
        }
    }
}

/// A local store's write error, as either kind's.
fn local<T>(result: Result<T, StoreError<DbError>>) -> Result<T, StoreError<Failure>> {
    result.map_err(|error| match error {
        StoreError::Refused(error) => StoreError::Refused(error),
        StoreError::NoCase => StoreError::NoCase,
        StoreError::Store(error) => StoreError::Store(Failure::Local(error)),
        other => StoreError::Store(Failure::Named(other.to_string())),
    })
}

impl Store for AnyStore {
    type Key = Key;
    type Writer = Writer;
    type Error = Failure;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Failure>> {
        match self {
            AnyStore::Local(db) => local(db.create(open, writer)),
            AnyStore::Forward(forwarded) => forwarded.create(open, writer),
            AnyStore::Remote(remote) => remote.create(open, writer),
        }
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Failure>> {
        match self {
            AnyStore::Local(db) => local(db.append(key, append, writer, at_revision)),
            AnyStore::Forward(forwarded) => forwarded.append(key, append, writer, at_revision),
            AnyStore::Remote(remote) => remote.append(key, append, writer, at_revision),
        }
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Failure> {
        match self {
            AnyStore::Local(db) => db.rows(key).map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.rows(key),
            AnyStore::Remote(remote) => remote.rows(key),
        }
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Failure> {
        match self {
            AnyStore::Local(db) => db.list().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.list(),
            AnyStore::Remote(remote) => remote.list(),
        }
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        match self {
            AnyStore::Local(db) => local(db.archive(key, at_revision)),
            AnyStore::Forward(forwarded) => forwarded.archive(key, at_revision),
            AnyStore::Remote(remote) => remote.archive(key, at_revision),
        }
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        match self {
            AnyStore::Local(db) => local(db.delete(key, at_revision)),
            AnyStore::Forward(forwarded) => forwarded.delete(key, at_revision),
            AnyStore::Remote(remote) => remote.delete(key, at_revision),
        }
    }

    fn version(&self) -> Result<Version, Failure> {
        match self {
            AnyStore::Local(db) => db.version().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.version(),
            AnyStore::Remote(remote) => remote.version(),
        }
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Failure> {
        match self {
            AnyStore::Local(db) => db.rows_since(since).map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.rows_since(since),
            AnyStore::Remote(remote) => remote.rows_since(since),
        }
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Failure> {
        match self {
            AnyStore::Local(db) => db.actors().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.actors(),
            AnyStore::Remote(remote) => remote.actors(),
        }
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Failure> {
        match self {
            AnyStore::Local(db) => db
                .listen(wait, worker, labels, kinds, doing)
                .map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.listen(wait, worker, labels, kinds, doing),
            AnyStore::Remote(remote) => remote.listen(wait, worker, labels, kinds, doing),
        }
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Failure> {
        match self {
            AnyStore::Local(db) => db.set_doing(worker, text).map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.set_doing(worker, text),
            AnyStore::Remote(remote) => remote.set_doing(worker, text),
        }
    }

    fn waits(&self) -> Result<Vec<Wait>, Failure> {
        match self {
            AnyStore::Local(db) => db.waits().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.waits(),
            AnyStore::Remote(remote) => remote.waits(),
        }
    }

    fn listeners(&self) -> Result<Vec<Listening>, Failure> {
        match self {
            AnyStore::Local(db) => db.listeners().map_err(Failure::Local),
            AnyStore::Forward(forwarded) => forwarded.listeners(),
            AnyStore::Remote(remote) => remote.listeners(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local store in `dir`, not made yet.
    fn local_store(dir: &tempfile::TempDir) -> AnyStore {
        AnyStore::Local(Db::new(dir.path().join("cases.db")))
    }

    fn open() -> Append {
        Append {
            author: "agent".into(),
            event: "open".into(),
            data: r#"{"kind":"fyi","urgency":"today","title":"Heads up"}"#.into(),
        }
    }

    #[test]
    fn a_local_store_gives_its_version_and_what_changed_since_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = local_store(&dir);
        // A store not made yet is at version 0, with nothing in it.
        let empty = store.version().unwrap();
        assert_eq!(empty.as_str(), "0");
        assert!(matches!(
            store.rows_since(None).unwrap(),
            Changes::All { version, cases } if version == empty && cases.is_empty()
        ));
        let (key, _) = store.create(&open(), &Writer::new(None)).unwrap();
        let first = store.version().unwrap();
        let Changes::All { version, cases } = store.rows_since(None).unwrap() else {
            panic!("a first read is every case");
        };
        assert_eq!(version, first);
        assert_eq!(cases.iter().map(|(key, _)| *key).collect::<Vec<_>>(), [key]);
        assert!(matches!(
            store.rows_since(Some(&first)).unwrap(),
            Changes::Unchanged { version } if version == first
        ));
        let (other, _) = store.create(&open(), &Writer::new(None)).unwrap();
        assert_ne!(store.version().unwrap(), first);
        let Changes::Since { cases, removed, .. } = store.rows_since(Some(&first)).unwrap() else {
            panic!("a read after a write is what changed");
        };
        assert_eq!(
            cases.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            [other]
        );
        assert!(removed.is_empty());
    }

    #[test]
    fn a_write_to_no_case_is_no_case() {
        let dir = tempfile::tempdir().unwrap();
        let store = local_store(&dir);
        let (key, _) = store.create(&open(), &Writer::new(None)).unwrap();
        let none = Key::new(key.get() + 1).unwrap();
        let resume = Append {
            author: "human".into(),
            event: "resume".into(),
            data: "{}".into(),
        };
        assert!(matches!(
            store.append(&none, &resume, &Writer::new(None), None),
            Err(StoreError::NoCase)
        ));
    }

    #[test]
    fn a_local_failure_is_no_remote_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let missing = Failure::Local(DbError::Missing { path: path.clone() });
        assert!(missing.is_missing());
        assert!(!missing.is_transient());
        assert!(!missing.not_listening());
        assert!(!missing.no_answer());
        let named = Failure::Named("the store moved".into());
        assert!(!named.is_missing());
        assert_eq!(named.to_string(), "the store moved");
        let target = Target::Local { path: path.clone() };
        assert_eq!(target.to_string(), path.display().to_string());
        assert_eq!(target.path(), Some(path.as_path()));
    }

    #[test]
    fn a_local_store_passes_each_call_to_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = local_store(&dir);
        let mut poller = store.poller();
        assert!(poller.version().is_none(), "no read yet");
        let (archived, _) = store.create(&open(), &Writer::new(None)).unwrap();
        let (deleted, _) = store.create(&open(), &Writer::new(None)).unwrap();
        assert_eq!(poller.poll().unwrap().len(), 2);
        assert_eq!(poller.version(), Some(store.version().unwrap()));
        drop(poller);
        store.archive(&archived, Some(1)).unwrap();
        store.delete(&deleted, Some(1)).unwrap();
        assert!(store.rows(&archived).unwrap().is_none());
        assert!(store.rows(&deleted).unwrap().is_none());
        assert!(store.list().unwrap().is_empty());

        store
            .listen("w1", "bun-pins", &["release".to_owned()], &[], None)
            .unwrap();
        store.set_doing("bun-pins", "rebasing").unwrap();
        let waits = store.waits().unwrap();
        assert_eq!(
            waits
                .iter()
                .map(|wait| wait.worker.as_str())
                .collect::<Vec<_>>(),
            ["bun-pins"]
        );
        let listeners = store.listeners().unwrap();
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].doing.as_deref(), Some("rebasing"));
    }

    #[test]
    fn a_route_the_far_side_does_not_serve_says_to_update_it_there() {
        let failure = Failure::Forward {
            error: RemoteError::NoStore,
            forward: "ssh://devbox".into(),
        };
        assert_eq!(
            failure.to_string(),
            format!(
                "the cases on ssh://devbox does not serve this request: it may be older than this \
                 one ({}); update it there",
                env!("CARGO_PKG_VERSION")
            )
        );
        assert!(!failure.is_transient() && !failure.no_answer());
    }
}
