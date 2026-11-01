//! The hosted half of [`crate::store`]: a store on a server (ST-11, SV-01,
//! AC-04), found by its id or by its name in the account's list of stores,
//! which is kept in the state directory, and the token for the server.
//! Hosted is parked, so this is built only with the `remote` feature, and a
//! release has no HTTP client in it.

use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cases_core::{
    ActorRow, Append, Case, Changes, Key, Listening, Row, Store, StoreError, Version, Wait, Writer,
};
use cases_store_remote::{Client, Error as RemoteError, Token, Whoami, store_id};

pub(crate) use cases_store_remote::Server;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{self, Config};
use crate::credentials::{self, Place};
use crate::error::Error;
use crate::store::{AnyStore, Failure, Selection, Target};

use crate::{instance, output};
pub(crate) use cases_store_remote::{HOLD_MAX, RemoteStore};
/// The environment variable that holds the token for a server.
pub(crate) const TOKEN_ENV: &str = "CASES_TOKEN";
/// Which store on a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    /// By its id, which is never looked up.
    Id(String),
    /// By its name in the account.
    Name(String),
    /// None named: the config's `default-store`, else the account's
    /// default, else the one store the token opens.
    Default,
}

impl Which {
    /// The store `text` names: its id when it is `st_` and 16 letters and
    /// digits, and its name otherwise.
    pub(crate) fn of(text: &str) -> Which {
        if store_id(text) {
            Which::Id(text.to_owned())
        } else {
            Which::Name(text.to_owned())
        }
    }
}
/// A store on a server: its id, and its name when the account's list gives
/// one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreRef {
    /// `st_…`.
    pub(crate) id: String,
    /// Its name in the account.
    pub(crate) name: Option<String>,
}

impl fmt::Display for StoreRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} ({})", self.id),
            None => f.write_str(&self.id),
        }
    }
}
/// The hosted inbox of `server`, `{server}/`.
pub(crate) fn inbox_url(server: &Server) -> String {
    format!("{server}/")
}

/// The page of the case `titled`, its id and title slug, in the hosted
/// inbox: `{server}/cases/{store id}-{store name}/{case id}-{title slug}`
/// (IN-02). The server routes by the ids alone, so a store whose name is not
/// known has its id alone.
pub(crate) fn case_url(server: &Server, store: &StoreRef, titled: &str) -> String {
    let store = match &store.name {
        Some(name) => format!("{}-{}", store.id, output::slug(name)),
        None => store.id.clone(),
    };
    format!("{server}/cases/{store}/{titled}")
}
/// The store on `server` that `text` names, by its id or its name, or the
/// default when it names none.
pub(crate) fn on_server(server: Server, text: Option<&str>) -> Selection {
    Selection::Remote {
        server,
        which: text.map_or(Which::Default, Which::of),
    }
}

/// The server of remote stores, `CASES_SERVER` else the config file's
/// `server`, when one is set.
///
/// # Errors
///
/// When the one set is not a server's address.
pub(crate) fn server(config: &Config) -> Result<Option<Server>, Error> {
    config::server_text(config)
        .map(|(text, source)| {
            text.parse::<Server>().map_err(|error| {
                let from = if source == "env" {
                    config::SERVER_ENV
                } else {
                    "the config file's server"
                };
                Error::new(format!("{from}: {error}"))
            })
        })
        .transpose()
}

/// The store a one-off URL names, `https://host/<name or id>`, or `None`
/// when `text` is not an `http` or `https` URL.
pub(crate) fn one_off(text: &str) -> Result<Option<Selection>, Error> {
    let Some((scheme, rest)) = text.split_once("://") else {
        return Ok(None);
    };
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return Ok(None);
    }
    let bad = || {
        Error::new(format!(
            "{text:?} is not a store's address: it is https://host/<store name or id>"
        ))
    };
    let (host, store) = rest.split_once('/').ok_or_else(bad)?;
    let store = store.strip_suffix('/').unwrap_or(store);
    if store.is_empty() || store.contains(['/', '?', '#']) {
        return Err(bad());
    }
    let server = format!("{scheme}://{host}")
        .parse::<Server>()
        .map_err(|error| Error::new(error.to_string()))?;
    Ok(Some(Selection::Remote {
        server,
        which: Which::of(store),
    }))
}
/// Opens the store `which` on `server`, with the token for the server, by
/// its id, or by its name or the default from the account's list of
/// stores, read from the state directory or the server.
///
/// # Errors
///
/// When there is no token for the server, the list cannot be read, the
/// name is not in it, or no store is named and the account has no single
/// store to use.
pub(crate) fn connect(
    server: Server,
    which: Which,
    config: &Config,
) -> Result<(Target, AnyStore), Error> {
    let accounts = Accounts::new(&server, config)?;
    let client = accounts.client.clone();
    let origin = accounts.origin.clone();
    let (store, again) = accounts.resolve(&which, false)?;
    let remote = client
        .store(&store.id)
        .map_err(|error| Error::new(error.to_string()))?;
    let target = Target::Remote {
        server: server.clone(),
        store: store.clone(),
    };
    let remote = Remote {
        server,
        origin,
        current: Mutex::new((remote, store)),
        again: Mutex::new(again.then_some((accounts, which))),
    };
    Ok((target, AnyStore::Remote(Box::new(remote))))
}

/// The config file's `default-store`: the store to use on a server when
/// none is named.
pub(crate) fn default_store(config: &Config) -> Option<String> {
    config
        .get("default-store")
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The account on `server` and the stores the token opens, read afresh
/// from the server with the token for it, and kept in the state directory
/// in place of the list there (`cases stores`).
///
/// # Errors
///
/// When there is no token for the server, or the server cannot be read or
/// refuses the token.
pub(crate) fn account(server: &Server, config: &Config) -> Result<Whoami, Error> {
    Accounts::new(server, config)?.whoami()
}

/// Where the token in use came from (SV-01's order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// `CASES_TOKEN`.
    Env,
    /// The file `CASES_TOKEN_FILE` names.
    EnvFile(PathBuf),
    /// The file the config file's `token-file` names.
    ConfigFile(PathBuf),
    /// The login `cases login` kept for the server.
    Login(Place),
}

impl Source {
    /// The token, for a sentence: `the token in CASES_TOKEN`.
    pub(crate) fn the_token(&self) -> String {
        match self {
            Source::Env => format!("the token in {TOKEN_ENV}"),
            Source::EnvFile(path) => format!(
                "the token in {} ({})",
                path.display(),
                config::TOKEN_FILE_ENV
            ),
            Source::ConfigFile(path) => format!("the token in the token file {}", path.display()),
            Source::Login(place) => format!("the token cases login kept in {place}"),
        }
    }

    /// What `cases login status --json` gives as `from`: `env`,
    /// `env-file`, `token-file`, `keychain` or `credentials`.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::EnvFile(_) => "env-file",
            Source::ConfigFile(_) => "token-file",
            Source::Login(Place::Keychain) => "keychain",
            Source::Login(Place::File(_)) => "credentials",
        }
    }

    /// Where the token came from, for `cases login status`: `CASES_TOKEN`,
    /// `CASES_TOKEN_FILE <path>`, `token-file <path>`, `the macOS keychain
    /// (cases login)` or `<credentials file> (cases login)`.
    pub(crate) fn described(&self) -> String {
        match self {
            Source::Env => TOKEN_ENV.to_owned(),
            Source::EnvFile(path) => format!("{} {}", config::TOKEN_FILE_ENV, path.display()),
            Source::ConfigFile(path) => format!("token-file {}", path.display()),
            Source::Login(place) => format!("{place} (cases login)"),
        }
    }
}

/// The token for `server` and where it came from, with what the server
/// says of it, `GET /v1/cli/whoami`, which also keeps the account's list of
/// stores (`cases login status`). The outer error is for no token at all;
/// the inner one, for a token the server refuses or no answer, names where
/// the token came from.
///
/// # Errors
///
/// When there is no token for the server, or it cannot be read.
pub(crate) fn status(
    server: &Server,
    config: &Config,
) -> Result<(Source, Result<Whoami, Error>), Error> {
    let accounts = Accounts::new(server, config)?;
    Ok((accounts.origin.source.clone(), accounts.whoami()))
}

/// Where a token was refused: the server, and where the token came from.
/// Only a refusal of the stored login suggests `cases login`, since a new
/// login would not change which token is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    /// The server.
    pub(crate) server: Server,
    /// Where the token came from.
    pub(crate) source: Source,
}

impl Origin {
    /// What to do about a refusal: sign in again, for the stored login.
    pub(crate) fn hint(&self) -> String {
        match self.source {
            Source::Login(_) => format!(". Run cases login {}", self.server),
            _ => String::new(),
        }
    }
}

/// The token for `server` (SV-01), and where it came from, in this order:
///
/// 1. `CASES_TOKEN`, when it is set and not empty;
/// 2. a token file, one line, trimmed: the one `CASES_TOKEN_FILE` names,
///    else the config file's `token-file`;
/// 3. the login `cases login` kept for `server`: the keychain, then the
///    credentials file.
///
/// The first two go only to the configured server, or to any server when
/// none is configured: a one-off URL to another server gets its stored
/// login alone, so a token is never sent to a server it was not made for.
/// A token file that others can read is warned about. Nothing here ever
/// prints the token.
///
/// # Errors
///
/// When none gives a token, the configured server is not one, a token file
/// or the stored login cannot be read, or what one holds is not a token.
pub(crate) fn token(server: &Server, config: &Config) -> Result<(Token, Source), Error> {
    let configured = self::server(config)?;
    let ours = configured
        .as_ref()
        .is_none_or(|configured| configured == server);
    if ours {
        if let Some(text) = std::env::var(TOKEN_ENV)
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
        {
            let token = text
                .parse()
                .map_err(|error| Error::new(format!("{TOKEN_ENV}: {error}")))?;
            return Ok((token, Source::Env));
        }
        if let Some((file, from)) = config::token_file_text(config) {
            let path = config::expand_home(&file);
            let token = token_file(&path)?;
            let source = if from == "env" {
                Source::EnvFile(path)
            } else {
                Source::ConfigFile(path)
            };
            return Ok((token, source));
        }
    }
    if let Some(login) = credentials::read(server)? {
        return Ok((login.token, Source::Login(login.place)));
    }
    Err(Error::new(match configured {
        Some(configured) if !ours => format!(
            "not signed in to {server}: run cases login {server} ({TOKEN_ENV} and a token file \
             go only to {configured})"
        ),
        _ => format!(
            "not signed in to {server}: run cases login {server}, or set {TOKEN_ENV} or a token \
             file ({}, or the config file's token-file)",
            config::TOKEN_FILE_ENV
        ),
    }))
}

/// The token in the token file at `path`, one line, trimmed. One that
/// others can read is warned about.
fn token_file(path: &Path) -> Result<Token, Error> {
    let text = fs::read_to_string(path).map_err(|error| {
        Error::new(format!(
            "the token file {} cannot be read: {error}",
            path.display()
        ))
    })?;
    warn_if_readable(path);
    text.trim().parse().map_err(|error| {
        Error::new(format!(
            "the token file {} does not hold a token: {error}",
            path.display()
        ))
    })
}

/// Warns when others than the owner can read the token file at `path`.
#[cfg(unix)]
fn warn_if_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        crate::ctx::warn(format_args!(
            "the token file {} can be read by others: make it private with chmod 600 {}",
            path.display(),
            path.display()
        ));
    }
}

/// Off unix, a file's mode says nothing to warn about.
#[cfg(not(unix))]
fn warn_if_readable(_path: &Path) {}

/// The SHA-256 of the token, in hex: what the kept list of stores records
/// it was read with, with no secret in it.
fn hash(token: &Token) -> String {
    Sha256::digest(token.expose().as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The account's list of stores, as kept in the state directory at
/// `accounts/<host>.json`: the account, each store, when the list was read,
/// and the SHA-256 of the token it was read with. It holds no secret.
#[derive(Debug, Serialize, Deserialize)]
struct List {
    server: String,
    account: ListAccount,
    stores: Vec<ListStore>,
    read_at: String,
    token_sha256: String,
}

/// The account, by id and handle.
#[derive(Debug, Serialize, Deserialize)]
struct ListAccount {
    id: String,
    handle: String,
}

/// A store of the account.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ListStore {
    id: String,
    name: String,
    default: bool,
}

/// How a server's account and its list of stores are read: through the
/// client, and from the state directory.
#[derive(Debug)]
struct Accounts {
    client: Client,
    /// Where the token came from.
    origin: Arc<Origin>,
    /// The SHA-256 of the token, which the kept list must match.
    hash: String,
    /// The config's `default-store`.
    default_store: Option<String>,
}

impl Accounts {
    /// How to read the account on `server`, with the token for it.
    fn new(server: &Server, config: &Config) -> Result<Accounts, Error> {
        let (token, source) = token(server, config)?;
        let hash = hash(&token);
        let client = Client::new(server.clone(), token).with_version(env!("CARGO_PKG_VERSION"));
        Ok(Accounts {
            client,
            origin: Arc::new(Origin {
                server: server.clone(),
                source,
            }),
            hash,
            default_store: default_store(config),
        })
    }

    /// The store `which` names, and whether it was found from the list
    /// (by name or as the default), so that a 401 or a 410 reads the list
    /// again (AC-04). An id is never looked up: its name comes from the
    /// kept list alone, when it has one. A list read from the state
    /// directory that does not have the name, or gives no store to use, is
    /// read again from the server, once; `fresh` reads it from the server
    /// at once.
    fn resolve(&self, which: &Which, fresh: bool) -> Result<(StoreRef, bool), Error> {
        let which = match (which, &self.default_store) {
            (Which::Default, Some(text)) => Which::of(text),
            (which, _) => which.clone(),
        };
        if let Which::Id(id) = &which {
            let name = self.kept().and_then(|list| {
                list.stores
                    .into_iter()
                    .find(|store| store.id == *id)
                    .map(|store| store.name)
            });
            return Ok((
                StoreRef {
                    id: id.clone(),
                    name,
                },
                false,
            ));
        }
        let (list, read) = match self.kept().filter(|_| !fresh) {
            Some(list) => (list, false),
            None => (self.read()?, true),
        };
        match pick(&list, &which, self.client.server()) {
            Ok(store) => Ok((store, true)),
            Err(_) if !read => {
                let list = self.read()?;
                pick(&list, &which, self.client.server()).map(|store| (store, true))
            }
            Err(error) => Err(error),
        }
    }

    /// Where the list for this server is kept.
    fn path(&self) -> Result<PathBuf, Error> {
        list_path(self.client.server())
    }

    /// The list kept in the state directory, if there is one for this
    /// server, read with this token.
    fn kept(&self) -> Option<List> {
        let text = fs::read_to_string(self.path().ok()?).ok()?;
        let list: List = serde_json::from_str(&text).ok()?;
        (list.token_sha256 == self.hash && list.server == self.client.server().as_str())
            .then_some(list)
    }

    /// The list from the server, `GET /v1/cli/whoami`, kept in the state
    /// directory for the next command. A list that cannot be kept is read
    /// again next time.
    fn read(&self) -> Result<List, Error> {
        let whoami = self.whoami()?;
        Ok(self.list(whoami))
    }

    /// `GET /v1/cli/whoami`, as the server gives it, with the list kept in
    /// the state directory as [`Accounts::read`] keeps it.
    fn whoami(&self) -> Result<Whoami, Error> {
        let server = self.client.server();
        let whoami = self.client.whoami().map_err(|error| {
            Error::new(match error {
                RemoteError::Auth { message } => format!(
                    "{server}: the server refused {}: {message}{}",
                    self.origin.source.the_token(),
                    self.origin.hint()
                ),
                error => format!("{server}: {error}"),
            })
        })?;
        let _ = self.keep(&self.list(whoami.clone()));
        Ok(whoami)
    }

    /// The list to keep from what `whoami` gave.
    fn list(&self, whoami: Whoami) -> List {
        list_of(self.client.server(), &self.hash, whoami)
    }

    /// Writes `list` to the state directory, in place of the one there.
    fn keep(&self, list: &List) -> io::Result<()> {
        keep_list(self.client.server(), list)
    }
}

/// The list to keep for `server` from what `whoami` gave, read with the
/// token whose SHA-256 is `hash`.
fn list_of(server: &Server, hash: &str, whoami: Whoami) -> List {
    List {
        server: server.as_str().to_owned(),
        account: ListAccount {
            id: whoami.account.id,
            handle: whoami.account.handle,
        },
        stores: whoami
            .stores
            .into_iter()
            .map(|store| ListStore {
                id: store.id,
                name: store.name,
                default: store.default,
            })
            .collect(),
        read_at: output::stamp(output::now().ok()),
        token_sha256: hash.to_owned(),
    }
}

/// Where the account's list for `server` is kept: `accounts/<host>.json`
/// in the state directory.
fn list_path(server: &Server) -> Result<PathBuf, Error> {
    Ok(instance::dir()?
        .join("accounts")
        .join(format!("{}.json", server.host())))
}

/// Writes `list` for `server` to the state directory, in place of the one
/// there.
fn keep_list(server: &Server, list: &List) -> io::Result<()> {
    let path = list_path(server).map_err(|error| io::Error::other(error.to_string()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(list)?;
    let partial = path.with_extension(format!("json.{}", std::process::id()));
    fs::write(&partial, text + "\n")?;
    fs::rename(&partial, &path)
}

/// Keeps what `whoami` gave for `token` on `server` as the account's list
/// of stores, in place of the one there, as `cases login` does with its
/// check's answer, so the first command after it needs no request to find
/// a store's name or the default (AC-03, SV-01).
///
/// # Errors
///
/// When the state directory cannot be found or written.
pub(crate) fn keep_account(server: &Server, token: &Token, whoami: Whoami) -> io::Result<()> {
    keep_list(server, &list_of(server, &hash(token), whoami))
}

/// The kept list for `server`, when it is there and was read with `token`.
fn kept_list(server: &Server, token: &Token) -> Option<List> {
    let text = fs::read_to_string(list_path(server).ok()?).ok()?;
    let list: List = serde_json::from_str(&text).ok()?;
    (list.token_sha256 == hash(token) && list.server == server.as_str()).then_some(list)
}

/// The account, its id and handle, that the kept list for `server` names,
/// when the list was read with `token`: so `cases logout` knows the account
/// with no request.
pub(crate) fn kept_account(server: &Server, token: &Token) -> Option<(String, String)> {
    kept_list(server, token).map(|list| (list.account.id, list.account.handle))
}

/// Removes the kept list for `server` when it was read with `token`, as
/// `cases logout` does, and gives whether there was one. A list read with
/// another token is that token's, and is left.
///
/// # Errors
///
/// When the list is there and cannot be removed.
pub(crate) fn forget_account(server: &Server, token: &Token) -> Result<bool, Error> {
    if kept_list(server, token).is_none() {
        return Ok(false);
    }
    let path = list_path(server)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::new(format!(
            "the account's list of stores, {}, cannot be removed: {error}",
            path.display()
        ))),
    }
}

/// The store `which` names in `list`, by name or as the default.
fn pick(list: &List, which: &Which, server: &Server) -> Result<StoreRef, Error> {
    let of = |store: &ListStore| StoreRef {
        id: store.id.clone(),
        name: Some(store.name.clone()),
    };
    let account = &list.account.handle;
    match which {
        Which::Id(id) => Ok(StoreRef {
            id: id.clone(),
            name: None,
        }),
        Which::Name(name) => list
            .stores
            .iter()
            .find(|store| store.name == *name)
            .map(of)
            .ok_or_else(|| {
                Error::new(format!(
                    "no store named {name} in account {account} on {server}"
                ))
            }),
        Which::Default => {
            if let Some(store) = list.stores.iter().find(|store| store.default) {
                return Ok(of(store));
            }
            match list.stores.as_slice() {
                [store] => Ok(of(store)),
                [] => Err(Error::new(format!(
                    "account {account} on {server} has no stores"
                ))),
                stores => {
                    let width = stores.iter().map(|s| s.name.len()).max().unwrap_or(0);
                    let names: Vec<String> = stores
                        .iter()
                        .map(|s| format!("  {:width$}  {}", s.name, s.id))
                        .collect();
                    Err(Error::new(format!(
                        "no store given, and account {account} on {server} has {} stores and no \
                         default: name one with --store, CASES_STORE or the config file's \
                         default-store\n{}",
                        stores.len(),
                        names.join("\n")
                    )))
                }
            }
        }
    }
}
/// A store on a server, which follows a rename: when it was found by name,
/// or as the default, from the account's list, a 401 or a 410 reads the
/// list again, once in the command, and when the name now has another id,
/// sends the request again to it. Nothing was read or written on a 401 or a
/// 410, so that is safe for a write too (AC-04).
#[derive(Debug)]
pub(crate) struct Remote {
    server: Server,
    /// Where the token came from.
    origin: Arc<Origin>,
    current: Mutex<(RemoteStore, StoreRef)>,
    /// How to find the store again, until it has been tried.
    again: Mutex<Option<(Accounts, Which)>>,
}

/// Locks `mutex`, whatever a panic left in it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An error a remote store gives, which may say the store moved.
trait Moved {
    /// The error, with what it says as a [`Failure`].
    type Out;
    /// Whether it is a 401 or a 410.
    fn moved(&self) -> bool;
    /// It, as a [`Failure`] of the store `store`, whose token came from
    /// `origin`.
    fn fail(self, store: String, origin: &Arc<Origin>) -> Self::Out;
}

impl Moved for RemoteError {
    type Out = Failure;
    fn moved(&self) -> bool {
        matches!(self, RemoteError::Auth { .. } | RemoteError::Gone)
    }
    fn fail(self, store: String, origin: &Arc<Origin>) -> Failure {
        Failure::Remote {
            error: self,
            store,
            origin: Arc::clone(origin),
        }
    }
}

impl Moved for StoreError<RemoteError> {
    type Out = StoreError<Failure>;
    fn moved(&self) -> bool {
        matches!(self, StoreError::Store(error) if error.moved())
    }
    fn fail(self, store: String, origin: &Arc<Origin>) -> StoreError<Failure> {
        match self {
            StoreError::Refused(error) => StoreError::Refused(error),
            StoreError::NoCase => StoreError::NoCase,
            StoreError::Store(error) => StoreError::Store(error.fail(store, origin)),
            other => StoreError::Store(Failure::Named(other.to_string())),
        }
    }
}

impl Remote {
    /// The store now, and how it was named.
    pub(crate) fn current(&self) -> (RemoteStore, StoreRef) {
        lock(&self.current).clone()
    }

    /// The server.
    pub(crate) fn server(&self) -> &Server {
        &self.server
    }

    /// The held poll (see [`RemoteStore::changes`]): the store's version as
    /// soon as it is not `after`, or after `hold`, at most 25 s.
    ///
    /// # Errors
    ///
    /// When the request fails.
    pub(crate) fn changes(&self, after: &Version, hold: Duration) -> Result<Version, Failure> {
        self.call(|store| store.changes(after, hold))
    }

    /// Keeps every later request within `deadline` (see
    /// [`Client::with_deadline`]): the store's, and the account's when the
    /// store is found again.
    pub(crate) fn until(&self, deadline: Option<Instant>) {
        let mut current = lock(&self.current);
        let client = current.0.client().clone().with_deadline(deadline);
        if let Ok(store) = client.store(current.0.id()) {
            current.0 = store;
        }
        if let Some((accounts, _)) = lock(&self.again).as_mut() {
            accounts.client = accounts.client.clone().with_deadline(deadline);
        }
    }

    /// Runs `call` on the store, and again on the store the name now has
    /// when the server says the store moved.
    fn call<T, E: Moved<Out = O>, O: From<Failure>>(
        &self,
        call: impl Fn(&RemoteStore) -> Result<T, E>,
    ) -> Result<T, O> {
        let (store, named) = self.current();
        let label = |named: &StoreRef| format!("{named} on {}", self.server);
        match call(&store) {
            Err(error) if error.moved() => {
                let Some((accounts, which)) = lock(&self.again).take() else {
                    return Err(error.fail(label(&named), &self.origin));
                };
                let again = match accounts.resolve(&which, true) {
                    Ok((again, _)) => again,
                    // The name is gone from the account: say so.
                    Err(why) if matches!(which, Which::Name(_)) => {
                        return Err(O::from(Failure::Named(why.to_string())));
                    }
                    Err(_) => return Err(error.fail(label(&named), &self.origin)),
                };
                if again.id == named.id {
                    return Err(error.fail(label(&named), &self.origin));
                }
                let Ok(moved) = store.client().store(&again.id) else {
                    return Err(error.fail(label(&named), &self.origin));
                };
                *lock(&self.current) = (moved.clone(), again.clone());
                call(&moved).map_err(|error| error.fail(label(&again), &self.origin))
            }
            result => result.map_err(|error| error.fail(label(&named), &self.origin)),
        }
    }
}

impl From<Failure> for StoreError<Failure> {
    fn from(failure: Failure) -> StoreError<Failure> {
        StoreError::Store(failure)
    }
}

impl Store for Remote {
    type Key = Key;
    type Writer = Writer;
    type Error = Failure;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Failure>> {
        self.call(|store| store.create(open, writer))
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Failure>> {
        self.call(|store| store.append(key, append, writer, at_revision))
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Failure> {
        self.call(|store| store.rows(key))
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Failure> {
        self.call(Store::list)
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        self.call(|store| store.archive(key, at_revision))
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        self.call(|store| store.delete(key, at_revision))
    }

    fn version(&self) -> Result<Version, Failure> {
        self.call(Store::version)
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Failure> {
        self.call(|store| store.rows_since(since))
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Failure> {
        self.call(Store::actors)
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Failure> {
        self.call(|store| store.listen(wait, worker, labels, kinds, doing))
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Failure> {
        self.call(|store| store.set_doing(worker, text))
    }

    fn waits(&self) -> Result<Vec<Wait>, Failure> {
        self.call(Store::waits)
    }

    fn listeners(&self) -> Result<Vec<Listening>, Failure> {
        self.call(Store::listeners)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        "https://cases.sh".parse().unwrap()
    }

    fn list(stores: &[(&str, &str, bool)]) -> List {
        List {
            server: "https://cases.sh".into(),
            account: ListAccount {
                id: "ac_x".into(),
                handle: "ryan".into(),
            },
            stores: stores
                .iter()
                .map(|(id, name, default)| ListStore {
                    id: (*id).into(),
                    name: (*name).into(),
                    default: *default,
                })
                .collect(),
            read_at: String::new(),
            token_sha256: String::new(),
        }
    }

    #[test]
    fn text_is_an_id_only_in_an_id_s_shape() {
        assert_eq!(
            Which::of("st_4kQ9mZr2pX7wLc0t"),
            Which::Id("st_4kQ9mZr2pX7wLc0t".into())
        );
        assert_eq!(
            Which::of("st_4kQ9mZr2pX7wLc0"),
            Which::Name("st_4kQ9mZr2pX7wLc0".into())
        );
        assert_eq!(Which::of("work"), Which::Name("work".into()));
    }

    #[test]
    fn a_one_off_url_names_a_server_and_a_store() {
        let Ok(Some(Selection::Remote { server, which })) = one_off("HTTPS://Cases.sh/work/")
        else {
            panic!("not a store's address")
        };
        assert_eq!(server.as_str(), "https://cases.sh");
        assert_eq!(which, Which::Name("work".into()));
        for text in ["work", "./a.db", "file:///x.db", "ftp://cases.sh/x"] {
            assert!(matches!(one_off(text), Ok(None)), "{text}");
        }
        for text in [
            "https://cases.sh",
            "https://cases.sh/",
            "https://cases.sh/a/b",
            "https://cases.sh/a?b",
            "http://cases.sh/work",
        ] {
            assert!(one_off(text).is_err(), "{text}");
        }
    }

    #[test]
    fn the_default_is_the_account_s_then_the_only_store() {
        let s = server();
        let two = list(&[("st_1", "me", false), ("st_2", "team", true)]);
        assert_eq!(pick(&two, &Which::Default, &s).unwrap().id, "st_2");
        let one = list(&[("st_1", "me", false)]);
        assert_eq!(pick(&one, &Which::Default, &s).unwrap().id, "st_1");
        let none = list(&[("st_1", "me", false), ("st_2", "team", false)]);
        let error = pick(&none, &Which::Default, &s).unwrap_err().to_string();
        assert!(error.contains("  me    st_1\n  team  st_2"), "{error}");
        let empty = list(&[]);
        assert!(pick(&empty, &Which::Default, &s).is_err());
        let error = pick(&two, &Which::Name("x".into()), &s)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "no store named x in account ryan on https://cases.sh"
        );
    }

    #[test]
    fn a_refused_token_names_its_source_and_only_a_login_s_suggests_login() {
        let file = PathBuf::from("/c/credentials.toml");
        let cases = [
            (Source::Env, "env", "the token in CASES_TOKEN", ""),
            (
                Source::EnvFile("/t".into()),
                "env-file",
                "the token in /t (CASES_TOKEN_FILE)",
                "",
            ),
            (
                Source::ConfigFile("/t".into()),
                "token-file",
                "the token in the token file /t",
                "",
            ),
            (
                Source::Login(Place::Keychain),
                "keychain",
                "the token cases login kept in the macOS keychain",
                ". Run cases login https://cases.sh",
            ),
            (
                Source::Login(Place::File(file)),
                "credentials",
                "the token cases login kept in /c/credentials.toml",
                ". Run cases login https://cases.sh",
            ),
        ];
        for (source, name, the_token, hint) in cases {
            assert_eq!(source.name(), name);
            assert_eq!(source.the_token(), the_token);
            let origin = Origin {
                server: server(),
                source,
            };
            assert_eq!(origin.hint(), hint);
        }
    }

    #[test]
    fn a_hosted_case_s_page_names_its_store() {
        let named = StoreRef {
            id: "st_1".into(),
            name: Some("Team X".into()),
        };
        assert_eq!(
            case_url(&server(), &named, "aB3xQ-pin"),
            format!(
                "https://cases.sh/cases/st_1-{}/aB3xQ-pin",
                output::slug("Team X")
            )
        );
        let bare = StoreRef {
            id: "st_1".into(),
            name: None,
        };
        assert_eq!(
            case_url(&server(), &bare, "aB3xQ"),
            "https://cases.sh/cases/st_1/aB3xQ"
        );
    }
}
