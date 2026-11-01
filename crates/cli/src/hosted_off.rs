//! The hosted half of [`crate::store`] in a build without the `remote`
//! feature, which is how a release is built while hosted is parked. Its
//! types have no values, so no store on a server can be named or reached:
//! every path that would reach one is code the compiler knows cannot run,
//! and the binary has no HTTP client. A store's address, or a server set in
//! `CASES_SERVER` or the config file, is refused with a plain message.

use std::fmt;
use std::time::{Duration, Instant};

use cases_core::{
    ActorRow, Append, Case, Changes, Key, Listening, Row, Store, StoreError, Version, Wait, Writer,
};

use crate::config::{self, Config};
use crate::error::Error;
use crate::store::{AnyStore, Failure, Selection, Target};

/// The longest a held poll holds, as the remote store's.
pub(crate) const HOLD_MAX: Duration = Duration::from_secs(25);

/// A server: there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Server {}

impl Server {
    /// Its host.
    pub(crate) fn host(&self) -> &str {
        match *self {}
    }

    /// Its address.
    pub(crate) fn as_str(&self) -> &str {
        match *self {}
    }
}

impl fmt::Display for Server {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}

/// No value at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Never {}

/// A store on a server: there is none, as it holds a [`Never`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreRef {
    /// `st_…`.
    pub(crate) id: String,
    /// Its name in the account.
    pub(crate) name: Option<String>,
    never: Never,
}

impl fmt::Display for StoreRef {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.never {}
    }
}

/// Which store on a server: there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Which {}

/// A remote store's handle: there is none.
#[derive(Clone, Debug)]
pub(crate) enum RemoteStore {}

/// A store on a server: there is none.
#[derive(Debug)]
pub(crate) enum Remote {}

impl Remote {
    /// The store now, and how it was named.
    pub(crate) fn current(&self) -> (RemoteStore, StoreRef) {
        match *self {}
    }

    /// The server.
    pub(crate) fn server(&self) -> &Server {
        match *self {}
    }

    /// The held poll.
    pub(crate) fn changes(&self, _: &Version, _: Duration) -> Result<Version, Failure> {
        match *self {}
    }

    /// Keeps later requests within a deadline.
    pub(crate) fn until(&self, _: Option<Instant>) {
        match *self {}
    }
}

impl Store for Remote {
    type Key = Key;
    type Writer = Writer;
    type Error = Failure;

    fn create(&self, _: &Append, _: &Writer) -> Result<(Key, Case), StoreError<Failure>> {
        match *self {}
    }

    fn append(
        &self,
        _: &Key,
        _: &Append,
        _: &Writer,
        _: Option<u64>,
    ) -> Result<Case, StoreError<Failure>> {
        match *self {}
    }

    fn rows(&self, _: &Key) -> Result<Option<Vec<Row>>, Failure> {
        match *self {}
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Failure> {
        match *self {}
    }

    fn archive(&self, _: &Key, _: Option<u64>) -> Result<(), StoreError<Failure>> {
        match *self {}
    }

    fn delete(&self, _: &Key, _: Option<u64>) -> Result<(), StoreError<Failure>> {
        match *self {}
    }

    fn version(&self) -> Result<Version, Failure> {
        match *self {}
    }

    fn rows_since(&self, _: Option<&Version>) -> Result<Changes<Key>, Failure> {
        match *self {}
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Failure> {
        match *self {}
    }

    fn listen(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &[String],
        _: Option<&str>,
    ) -> Result<(), Failure> {
        match *self {}
    }

    fn set_doing(&self, _: &str, _: &str) -> Result<(), Failure> {
        match *self {}
    }

    fn waits(&self) -> Result<Vec<Wait>, Failure> {
        match *self {}
    }

    fn listeners(&self) -> Result<Vec<Listening>, Failure> {
        match *self {}
    }
}

/// The store on a server, which there is none of.
#[allow(
    clippy::needless_pass_by_value,
    reason = "it takes what the remote feature's takes"
)]
pub(crate) fn on_server(server: Server, _: Option<&str>) -> Selection {
    match server {}
}

/// Why `what` cannot be used.
fn refused(what: &str) -> Error {
    Error::new(format!(
        "{what}: this cases is built without remote stores, so it uses only a local store \
         file"
    ))
}

/// The hosted inbox of a server.
pub(crate) fn inbox_url(server: &Server) -> String {
    match *server {}
}

/// The page of a case in the hosted inbox.
pub(crate) fn case_url(server: &Server, _: &StoreRef, _: &str) -> String {
    match *server {}
}

/// The server of remote stores, which none may be.
///
/// # Errors
///
/// When `CASES_SERVER` or the config file's `server` sets one.
pub(crate) fn server(config: &Config) -> Result<Option<Server>, Error> {
    match config::server_text(config) {
        Some((_, "env")) => Err(refused(config::SERVER_ENV)),
        Some(_) => Err(refused("the config file's server")),
        None => Ok(None),
    }
}

/// A one-off URL, which no store may be.
///
/// # Errors
///
/// When `text` is an `http` or `https` URL.
pub(crate) fn one_off(text: &str) -> Result<Option<Selection>, Error> {
    match text.split_once("://") {
        Some((scheme, _))
            if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") =>
        {
            Err(refused(&format!("{text:?}")))
        }
        _ => Ok(None),
    }
}

/// Opens a store on a server, which there is none of.
#[allow(
    clippy::needless_pass_by_value,
    reason = "it takes what the remote feature's takes"
)]
pub(crate) fn connect(server: Server, _: Which, _: &Config) -> Result<(Target, AnyStore), Error> {
    match server {}
}
