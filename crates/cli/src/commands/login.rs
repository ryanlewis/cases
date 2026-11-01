//! `cases login`: signs this computer in to an account on a server, and
//! keeps the token (AC-03, AC-05).
//!
//! The server is the argument, else `CASES_SERVER`, else the config file's
//! `server`; there is no built-in one. The sign-in is the loopback (PKCE,
//! RFC 7636): the browser opens the server's approval page and comes back
//! to a listener on `127.0.0.1` with a code, which is traded for a token.
//! `--device` (RFC 8628) prints a page and a code to type in any browser
//! signed in to the account instead. Either way the new token is checked
//! with `GET /v1/cli/whoami` before anything is kept. Then a login writes
//! the account's list of stores, keeps the token ([`credentials`]), and
//! revokes the token it replaced; `--file PATH` writes a second token to
//! PATH and leaves the login alone.
//!
//! Login writes no config. It never prints the token, the code verifier,
//! the code or the device code.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cases_store_remote::{
    Client, Error as RemoteError, Pkce, Server, Signin, SigninError, Token, TokenAnswer, Whoami,
    redirect_uri,
};

use crate::browser;
use crate::commands::login_status;
use crate::config::{self, Config};
use crate::credentials::{self, Login, Place};
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::store::{self, TOKEN_ENV};

/// How long the loopback sign-in waits for the browser to come back.
pub(crate) const LOOPBACK_WAIT: Duration = Duration::from_secs(300);

/// How long the listener waits on one connection for its request.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The most of a request the listener reads.
const REQUEST_MAX: usize = 16 * 1024;

/// How long the browser's callback is held while the code is exchanged and
/// the token checked and kept, before its page says to look at the
/// terminal.
const PAGE_WAIT: Duration = Duration::from_secs(20);

/// The most characters a token's name has.
const NAME_MAX: usize = 100;

/// The CLI's version, which every request names.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The arguments of `cases login`. `status` is a subcommand beside the
/// optional server: the server is always an address, `https://…`, so a
/// first argument of `status` is never one, and flags of login's own
/// cannot come before `status`.
#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Option<Sub>,

    #[arg(
        value_name = "SERVER",
        help = "The server, https://host[:port]. Default: $CASES_SERVER, then the config file's \
                server."
    )]
    server: Option<String>,

    /// Sign in with a code to type in a browser signed in to the account,
    /// for SSH and machines with no browser. The browser is never opened.
    #[arg(long)]
    device: bool,

    /// Print the approval page's address, and do not open the browser.
    #[arg(long, conflicts_with = "device")]
    no_browser: bool,

    #[arg(
        long,
        value_name = "TEXT",
        help = "The token's name, at most 100 characters. Default: \"cases CLI on <computer>\", \
                with the file's name after it for --file."
    )]
    name: Option<String>,

    /// Write a new token to PATH, for an agent or a service, and leave this
    /// computer's login alone: nothing is replaced or revoked. The file is
    /// made readable by its owner alone.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,

    /// With --file: replace a file that is already there.
    #[arg(long, requires = "file")]
    force: bool,
}

/// The subcommands of `cases login`.
#[derive(Debug, clap::Subcommand)]
enum Sub {
    /// Show the token commands use for the server, and what the server says
    /// about it: the account, the token's name, the stores it opens, when
    /// it expires, where it came from, and the default store. Exits 1 when
    /// there is no token, or the server refuses it or cannot be reached.
    Status(login_status::Args),
}

/// Signs in to the server, checks the new token, and keeps it, or writes
/// it to the file.
///
/// # Errors
///
/// When no server is named or set, the flags are not ones, the sign-in is
/// denied, runs out or fails, the new token fails its check, or it cannot
/// be kept. Nothing is kept then, and a token made is revoked as best it
/// can be.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    if let Some(Sub::Status(status)) = &args.command {
        return login_status::run(status, ctx);
    }
    let server = server(args.server.as_deref(), &ctx.config, "cases login")?;
    // The scope comes from the flag alone, never CASES_STORE or the
    // config, so a token's scope is never set by accident (AC-05).
    let stores = scope(ctx.store_flag.as_deref())?;
    let file = args
        .file
        .as_deref()
        .map(|path| target_file(path, args.force))
        .transpose()?;
    let name = match &args.name {
        Some(name) => checked_name(name)?,
        None => default_name(file.as_deref()),
    };
    // Read before the sign-in, so that the token it replaces is known.
    let old = match file {
        Some(_) => None,
        None => stored(&server),
    };
    let signin = Signin::new(server.clone()).with_version(VERSION);
    let (answer, held) = if args.device {
        (device(&signin, &name, stores.as_deref())?, None)
    } else {
        let (answer, held) = loopback(&signin, &name, stores.as_deref(), args.no_browser)?;
        (answer, Some(held))
    };
    let client = Client::new(server.clone(), answer.token.clone()).with_version(VERSION);
    let whoami = check(&client, &answer)?;
    let kept = match &file {
        Some(path) => {
            write_token(path, &answer.token, args.force).map_err(|error| {
                discard(
                    &client,
                    &answer,
                    &format!("{} cannot be written: {error}", path.display()),
                )
            })?;
            None
        }
        None => Some(keep(&client, &answer, &whoami)?),
    };
    if let Some(held) = held {
        held.finish(Page::SignedIn(whoami.account.handle.clone()));
    }
    let scope = scope_text(&answer, &whoami);
    match (&file, kept) {
        (Some(path), _) => {
            let path_text = path.display().to_string();
            let quoted = serde_json::to_string(&path_text).unwrap_or_default();
            writeln!(
                ctx.out,
                "Wrote a token for {} ({}) to {path_text}.\nToken {:?}, for {scope}.\nUse it with \
                 token-file = {quoted}, or {}.",
                whoami.account.handle,
                whoami.account.id,
                answer.name,
                config::TOKEN_FILE_ENV
            )?;
        }
        (None, Some(place)) => {
            if let Some(old) = old {
                revoke_old(&server, old, &answer.token);
            }
            writeln!(
                ctx.out,
                "Signed in to {server} as {} ({}).\nToken {:?}, for {scope}, kept in {place}.\n\
                 Stores: {}.",
                whoami.account.handle,
                whoami.account.id,
                answer.name,
                stores_text(&whoami)
            )?;
        }
        (None, None) => {}
    }
    notes(&server, &ctx.config, file.is_none());
    Ok(())
}

/// The server `command` acts on, `cases login` or another of its kin:
/// `arg`, else `CASES_SERVER`, else the config file's `server`.
///
/// # Errors
///
/// When `arg` is not a server's address, or none is named or set.
pub(crate) fn server(arg: Option<&str>, config: &Config, command: &str) -> Result<Server, Error> {
    if let Some(text) = arg {
        return text
            .trim()
            .parse()
            .map_err(|error: cases_store_remote::Invalid| Error::new(error.to_string()));
    }
    store::server(config)?.ok_or_else(|| {
        Error::new(format!(
            "name the server: {command} https://… (or set {} or the config file's server)",
            config::SERVER_ENV
        ))
    })
}

/// The stores `--store` names for the token to open, split by commas, or
/// `None` for every store. Each must be a store's name: a path, a URL or an
/// id is refused, since the approval page and the device start take names.
fn scope(flag: Option<&str>) -> Result<Option<Vec<String>>, Error> {
    let Some(text) = flag.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let mut names: Vec<String> = Vec::new();
    for name in text.split(',').map(str::trim) {
        if !store_name(name) {
            return Err(Error::new(format!(
                "--store for login takes store names, split by commas, and {name:?} is not one: a \
                 store's name is 1 to 63 lower-case letters, digits and -"
            )));
        }
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }
    Ok(Some(names))
}

/// Whether `name` is a store's name: 1 to 63 lower-case letters, digits
/// and `-` (AC-01).
fn store_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `name`, when it can name a token: 1 to 100 characters, not blank, with
/// no control characters.
fn checked_name(name: &str) -> Result<String, Error> {
    let count = name.chars().count();
    if name.trim().is_empty() || count > NAME_MAX || name.chars().any(char::is_control) {
        return Err(Error::new(format!(
            "--name is the token's name: 1 to {NAME_MAX} characters, not blank, with no control \
             characters"
        )));
    }
    Ok(name.to_owned())
}

/// The token's name when none is given: `cases CLI on <computer>`, and for
/// a file `, <file name>` after it, so the account's token page tells them
/// apart. At most 100 characters, with no control characters.
fn default_name(file: Option<&Path>) -> String {
    let mut name = match host_name() {
        Some(host) => format!("cases CLI on {host}"),
        None => "cases CLI".to_owned(),
    };
    if let Some(file) = file.and_then(Path::file_name) {
        let _ = write!(name, ", {}", file.to_string_lossy());
    }
    clean(&name).chars().take(NAME_MAX).collect()
}

/// The computer's name, up to its first dot, if it has one.
fn host_name() -> Option<String> {
    #[cfg(unix)]
    let host = rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned();
    #[cfg(not(unix))]
    let host = env::var("COMPUTERNAME").unwrap_or_default();
    let host = clean(host.split('.').next().unwrap_or_default());
    let host = host.trim();
    (!host.is_empty()).then(|| host.to_owned())
}

/// `text` with its control characters taken out, for the terminal.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The file `--file` names, as an absolute path, checked before anything
/// is sent: its directory must be there, and it must not be, unless
/// `force`. A symbolic link or a directory is refused even then.
fn target_file(path: &Path, force: bool) -> Result<PathBuf, Error> {
    let absolute = std::path::absolute(path)
        .map_err(|error| Error::new(format!("--file {} is not a path: {error}", path.display())))?;
    if absolute.file_name().is_none() {
        return Err(Error::new(format!(
            "--file {} names no file",
            path.display()
        )));
    }
    let dir = absolute.parent().unwrap_or(Path::new("/"));
    if !dir.is_dir() {
        return Err(Error::new(format!(
            "the directory {} is not there: make it first",
            dir.display()
        )));
    }
    match fs::symlink_metadata(&absolute) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::new(format!(
            "{} is a symbolic link: name the file itself, as cases writes a token only to a file \
             of its own",
            absolute.display()
        ))),
        Ok(meta) if meta.is_dir() => {
            Err(Error::new(format!("{} is a directory", absolute.display())))
        }
        Ok(_) if !force => Err(Error::new(format!(
            "{} is already there: add --force to replace it",
            absolute.display()
        ))),
        Ok(_) => Ok(absolute),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(absolute),
        Err(error) => Err(Error::new(format!(
            "{} cannot be read: {error}",
            absolute.display()
        ))),
    }
}

/// Writes `token` to the file at `path`, one line, readable by its owner
/// alone. The file is made with mode 0600 and `O_EXCL`, which also never
/// follows a symbolic link there. With `force`, it is a temporary file
/// beside it, named with the process id, renamed over the one there; the
/// rename replaces a link, never what it points to. A failed write leaves
/// no copy of the token behind.
fn write_token(path: &Path, token: &Token, force: bool) -> io::Result<()> {
    let line = format!("{}\n", token.expose());
    let write = |at: &Path| {
        let mut out = private_file(at)?;
        let written = out.write_all(line.as_bytes()).and_then(|()| out.sync_all());
        if written.is_err() {
            let _ = fs::remove_file(at);
        }
        written
    };
    if !force {
        return write(path);
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let partial = path.with_file_name(name);
    // One left by an earlier process with this id is stale.
    let _ = fs::remove_file(&partial);
    write(&partial)?;
    fs::rename(&partial, path).inspect_err(|_| {
        let _ = fs::remove_file(&partial);
    })
}

/// A new file at `path`, made with mode 0600, refused when anything is
/// there already.
fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

/// The login kept for `server` now, which a new login replaces. One that
/// cannot be read is warned about, as the new login is kept all the same.
fn stored(server: &Server) -> Option<Login> {
    match credentials::read(server) {
        Ok(login) => login,
        Err(error) => {
            ctx::warn(format_args!(
                "{error}; the token kept for {server} cannot be revoked when this login replaces \
                 it: revoke it at {server}/tokens"
            ));
            None
        }
    }
}

/// Checks the new token with `GET /v1/cli/whoami` against the sign-in's
/// answer (AC-03), and gives what the server said.
///
/// # Errors
///
/// When the server refuses the token, cannot be reached, or says other
/// than the answer did: the token is revoked as best it can be, and the
/// error says whether it was.
fn check(client: &Client, answer: &TokenAnswer) -> Result<Whoami, Error> {
    client
        .whoami()
        .map_err(|error| error.to_string())
        .and_then(|whoami| answer.check(&whoami).map(|()| whoami))
        .map_err(|why| {
            discard(
                client,
                answer,
                &format!("the new token failed its check: {why}"),
            )
        })
}

/// The error for a new token that is not kept, after `why`: the token is
/// revoked as best it can be, and the error says whether it was.
fn discard(client: &Client, answer: &TokenAnswer, why: &str) -> Error {
    let server = client.server();
    let outcome = match client.revoke(&answer.account.id) {
        Ok(()) => "Nothing was kept, and the new token is revoked.".to_owned(),
        Err(error) => format!(
            "Nothing was kept, but the new token could not be revoked ({error}): if \
             {server}/tokens lists it, revoke it there."
        ),
    };
    Error::new(format!("{server}: {why}. {outcome}"))
}

/// Keeps the new login: the account's list of stores from the check's
/// answer, then the token, which replaces the one kept before. A list that
/// cannot be written is warned about, as the next command reads it again.
///
/// # Errors
///
/// When the token cannot be kept: it is revoked as best it can be.
fn keep(client: &Client, answer: &TokenAnswer, whoami: &Whoami) -> Result<Place, Error> {
    let server = client.server();
    if let Err(error) = store::keep_account(server, &answer.token, whoami.clone()) {
        ctx::warn(format_args!(
            "the account's list of stores cannot be kept ({error}): the next command reads it \
             from the server"
        ));
    }
    credentials::write(server, &answer.account.id, &answer.token)
        .map_err(|error| discard(client, answer, &error.to_string()))
}

/// Revokes `old`, the token the new login replaced, as best it can: the
/// server's refusal of it means it is no use either way, and any other
/// failure is warned about, naming the account's token page. Its account
/// is the one kept beside it, else the one the server gives for it.
fn revoke_old(server: &Server, old: Login, new: &Token) {
    if old.token == *new {
        return;
    }
    let client = Client::new(server.clone(), old.token).with_version(VERSION);
    let failed = |error: &RemoteError| {
        ctx::warn(format_args!(
            "the token this login replaced is not revoked ({error}): revoke the old token at \
             {server}/tokens"
        ));
    };
    let account = match old.account {
        Some(account) => account,
        None => match client.whoami() {
            Ok(whoami) => whoami.account.id,
            Err(RemoteError::Auth { .. } | RemoteError::NoToken) => return,
            Err(error) => return failed(&error),
        },
    };
    match client.revoke(&account) {
        Ok(()) | Err(RemoteError::Auth { .. } | RemoteError::NoToken) => {}
        Err(error) => failed(&error),
    }
}

/// What the token opens, for "Signed in": `every store`, or the stores'
/// names and `only`.
fn scope_text(answer: &TokenAnswer, whoami: &Whoami) -> String {
    match &whoami.token.stores {
        None => "every store".to_owned(),
        Some(ids) if ids.is_empty() => "no store".to_owned(),
        Some(ids) => {
            let names: Vec<&str> = ids
                .iter()
                .map(|id| {
                    answer
                        .stores
                        .iter()
                        .find(|store| store.id == *id)
                        .map_or(id.as_str(), |store| store.name.as_str())
                })
                .collect();
            format!("{} only", names.join(", "))
        }
    }
}

/// The stores the token opens, the account's default marked.
pub(crate) fn stores_text(whoami: &Whoami) -> String {
    if whoami.stores.is_empty() {
        return "none".to_owned();
    }
    let names: Vec<String> = whoami
        .stores
        .iter()
        .map(|store| {
            if store.default {
                format!("{} (default)", store.name)
            } else {
                store.name.clone()
            }
        })
        .collect();
    names.join(", ")
}

/// What the human should know after signing in, since login writes no
/// config: that commands use another server, or none, and the line to
/// add; and for a login, that `CASES_TOKEN` or a token file wins over it.
fn notes(server: &Server, config: &Config, login: bool) {
    let path = config.path.display();
    match store::server(config).ok().flatten() {
        None => ctx::warn(format_args!(
            "commands use the local store until a server is set: add server = \"{server}\" to \
             {path}, or set {}",
            config::SERVER_ENV
        )),
        Some(other) if other != *server => {
            let from = match config::server_text(config) {
                Some((_, "env")) => config::SERVER_ENV.to_owned(),
                _ => format!("the config file {path}"),
            };
            ctx::warn(format_args!(
                "commands use {other}, from {from}, not this server: set server = \"{server}\" \
                 in {path}, or {}, to use it",
                config::SERVER_ENV
            ));
            // A token in the environment or a file goes only to the
            // configured server, so it does not hide this login.
            return;
        }
        Some(_) => {}
    }
    if !login {
        return;
    }
    if env::var(TOKEN_ENV).is_ok_and(|token| !token.trim().is_empty()) {
        ctx::warn(format_args!(
            "{TOKEN_ENV} is set, and commands use it rather than this login"
        ));
    } else if let Some((file, from)) = config::token_file_text(config) {
        let from = if from == "env" {
            config::TOKEN_FILE_ENV.to_owned()
        } else {
            format!("the config file's token-file, in {path}")
        };
        ctx::warn(format_args!(
            "the token file {} is set ({from}), and commands use it rather than this login",
            config::expand_home(&file).display()
        ));
    }
}

/// Whether there is no display to open a browser on: an SSH session, with
/// no X display off macOS.
fn no_display() -> bool {
    let set = |name: &str| env::var_os(name).is_some_and(|value| !value.is_empty());
    set("SSH_CONNECTION") && (cfg!(target_os = "macos") || !set("DISPLAY"))
}

/// The loopback sign-in: listens on `127.0.0.1`, sends the browser to the
/// approval page, waits for it to come back with a code, and trades the
/// code for a token. The browser's request is held, to be answered once
/// the token is checked and kept.
fn loopback(
    signin: &Signin,
    name: &str,
    stores: Option<&[String]>,
    no_browser: bool,
) -> Result<(TokenAnswer, Held), Error> {
    let listener = Listener::bind()?;
    let pkce = Arc::new(
        Pkce::new()
            .map_err(|error| Error::new(format!("no random bytes for the sign-in: {error}")))?,
    );
    let redirect = redirect_uri(listener.port);
    let url = signin.authorize_url(&pkce, &redirect, name, stores);
    // The address holds the state and the challenge, which are no secrets.
    if !no_browser && browser::open(&url).is_ok() {
        eprintln!("Approve the sign-in in the browser. If no page opened, open this one:\n  {url}");
    } else {
        eprintln!("Open this page to approve the sign-in:\n  {url}");
    }
    if no_display() {
        eprintln!("No browser on this machine? Run cases login --device instead.");
    }
    let (code, held) = listener.wait(&pkce, LOOPBACK_WAIT)?;
    let server = signin.server();
    let answer = signin.exchange(&code, &pkce, &redirect).map_err(|error| {
        Error::new(match error {
            RemoteError::Lost { .. } => error.to_string(),
            RemoteError::Server { status: 503, .. } => {
                format!("{server}: a store could not be reached; run cases login again")
            }
            RemoteError::Signin {
                kind: SigninError::InvalidGrant,
                message,
            } => format!(
                "{server}: the server refused the sign-in's code ({}): run cases login again",
                clean(&message)
            ),
            error => format!("{server}: {error}"),
        })
    })?;
    Ok((answer, held))
}

/// What a connection to the listener brought, for the sign-in.
enum Event {
    /// A callback with another sign-in's state, refused.
    Stray,
    /// This sign-in's callback, denied, with the error it gave.
    Denied(String),
    /// This sign-in's callback with its code, and the browser's request,
    /// held.
    Code(String, TcpStream),
}

/// The listener on `127.0.0.1` that the browser comes back to.
struct Listener {
    listener: TcpListener,
    port: u16,
}

impl Listener {
    /// Listens on `127.0.0.1`, on a port the system picks: never
    /// `0.0.0.0` or `localhost`, so only this computer can call back.
    fn bind() -> Result<Listener, Error> {
        let fail = |error: io::Error| Error::new(format!("cannot listen on 127.0.0.1: {error}"));
        let listener = TcpListener::bind("127.0.0.1:0").map_err(fail)?;
        let port = listener.local_addr().map_err(fail)?.port();
        Ok(Listener { listener, port })
    }

    /// Waits up to `wait` for the browser's callback with this sign-in's
    /// state, and gives its code with the browser's request, held. Each
    /// connection is answered on a thread of its own, so one that sends
    /// nothing, such as a browser's preconnect, holds nothing up. A
    /// callback with another state is refused, warned about once, and
    /// waited past. The listener takes no more connections after this
    /// sign-in's callback.
    ///
    /// # Errors
    ///
    /// When the sign-in is denied, or nothing comes back in time.
    fn wait(self, pkce: &Arc<Pkce>, wait: Duration) -> Result<(String, Held), Error> {
        let Listener { listener, port } = self;
        listener
            .set_nonblocking(true)
            .map_err(|error| Error::new(format!("cannot listen on 127.0.0.1: {error}")))?;
        let done = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        {
            let (done, pkce) = (Arc::clone(&done), Arc::clone(pkce));
            let host = format!("127.0.0.1:{port}");
            thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (send, done, pkce, host) = (
                                send.clone(),
                                Arc::clone(&done),
                                Arc::clone(&pkce),
                                host.clone(),
                            );
                            thread::spawn(move || connection(stream, &pkce, &host, &done, &send));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(25)),
                    }
                }
            });
        }
        let deadline = Instant::now() + wait;
        let mut warned = false;
        let result = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receive.recv_timeout(left) {
                Ok(Event::Stray) => {
                    if !warned {
                        ctx::warn(
                            "a page came back to the sign-in with another sign-in's state, and \
                             was turned away; still waiting for this one",
                        );
                        warned = true;
                    }
                }
                Ok(Event::Denied(why)) => {
                    break Err(Error::new(format!("the sign-in was denied ({why})")));
                }
                Ok(Event::Code(code, stream)) => break Ok((code, Held::new(stream))),
                Err(_) => {
                    break Err(Error::new(format!(
                        "no approval came back within {}: run cases login again",
                        span(wait)
                    )));
                }
            }
        };
        done.store(true, Ordering::SeqCst);
        result
    }
}

/// `wait` for people: whole minutes, or seconds under one.
fn span(wait: Duration) -> String {
    match wait.as_secs() {
        60 => "a minute".to_owned(),
        secs if secs > 60 => format!("{} minutes", secs / 60),
        1 => "a second".to_owned(),
        secs => format!("{secs} seconds"),
    }
}

/// Answers one connection to the listener. Only `GET /callback?…` with the
/// `Host` of the listener is a callback; anything else gets 404. The state
/// is checked first, in constant time: a callback with another state gets
/// 400, and nothing in it is used, not a code and not an error.
fn connection(
    mut stream: TcpStream,
    pkce: &Pkce,
    host: &str,
    done: &AtomicBool,
    send: &mpsc::Sender<Event>,
) {
    let head = match read_head(&mut stream) {
        Some(Head::Request(head)) => head,
        Some(Head::TooLarge) => return reply(&mut stream, 431, "The request is too large."),
        None => return,
    };
    let Some(query) = callback_query(&head, host) else {
        return reply(&mut stream, 404, "Not found.");
    };
    let fields: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if !field("state").is_some_and(|state| pkce.state_is(&state)) {
        reply(
            &mut stream,
            400,
            "This is not the sign-in the terminal started.",
        );
        let _ = send.send(Event::Stray);
        return;
    }
    let finished = |stream: &mut TcpStream| reply(stream, 400, "This sign-in has finished.");
    if let Some(error) = field("error") {
        if done.swap(true, Ordering::SeqCst) {
            return finished(&mut stream);
        }
        reply(
            &mut stream,
            200,
            "The sign-in was denied. You can close this tab.",
        );
        let why: String = clean(&error).chars().take(200).collect();
        let _ = send.send(Event::Denied(why));
        return;
    }
    let Some(code) = field("code").filter(|code| !code.is_empty()) else {
        return reply(&mut stream, 400, "This is not a sign-in.");
    };
    if done.swap(true, Ordering::SeqCst) {
        return finished(&mut stream);
    }
    let _ = send.send(Event::Code(code, stream));
}

/// A request's head, as read.
enum Head {
    /// The request line and headers, up to the blank line.
    Request(String),
    /// Over [`REQUEST_MAX`] with no end to the head.
    TooLarge,
}

/// Reads a request's head from `stream`: at most [`REQUEST_MAX`] bytes,
/// within [`READ_TIMEOUT`]. `None` when the connection closes or sends
/// too slowly.
fn read_head(stream: &mut TcpStream) -> Option<Head> {
    let started = Instant::now();
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            head.truncate(end);
            return Some(Head::Request(String::from_utf8_lossy(&head).into_owned()));
        }
        if head.len() >= REQUEST_MAX {
            return Some(Head::TooLarge);
        }
        if started.elapsed() >= READ_TIMEOUT {
            return None;
        }
        let room = chunk.len().min(REQUEST_MAX - head.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) | Err(_) => return None,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
}

/// The query of a callback: a `GET /callback?…` over HTTP/1, whose every
/// `Host` header is `host`.
fn callback_query<'a>(head: &'a str, host: &str) -> Option<&'a str> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if method != "GET" || !version.starts_with("HTTP/1.") || first.next().is_some() {
        return None;
    }
    let hosts: Vec<&str> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts.is_empty() || hosts.iter().any(|h| *h != host) {
        return None;
    }
    target.strip_prefix("/callback?")
}

/// Answers `stream` with a small page that says `text`, which is HTML
/// already: no script, nothing from outside, and a policy that lets none
/// run.
fn reply(stream: &mut TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        431 => "Request Header Fields Too Large",
        _ => "",
    };
    let body = format!(
        "<!doctype html>\n<html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width\"><title>cases login</title><p>{text}</p></html>\n"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \
         {}\r\nContent-Security-Policy: default-src 'none'\r\nCache-Control: \
         no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: \
         nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

/// `text` with the characters HTML gives a meaning escaped.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// What the held browser request is told.
enum Page {
    /// Signed in, as the account's handle.
    SignedIn(String),
    /// The sign-in failed.
    Failed,
}

/// The browser's callback, held open while the code is exchanged and the
/// token checked and kept, so that its page tells the truth. After
/// [`PAGE_WAIT`] it is told to look at the terminal. Dropped unfinished,
/// it is told the sign-in failed.
struct Held {
    send: Option<mpsc::Sender<Page>>,
    thread: Option<JoinHandle<()>>,
}

impl Held {
    fn new(mut stream: TcpStream) -> Held {
        let (send, receive) = mpsc::channel();
        let thread = thread::spawn(move || {
            let text = match receive.recv_timeout(PAGE_WAIT) {
                Ok(Page::SignedIn(handle)) => format!(
                    "Signed in as {}. You can close this tab.",
                    escape_html(&handle)
                ),
                Ok(Page::Failed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    "The sign-in failed: see the terminal.".to_owned()
                }
                Err(mpsc::RecvTimeoutError::Timeout) => "Finishing in the terminal.".to_owned(),
            };
            reply(&mut stream, 200, &text);
        });
        Held {
            send: Some(send),
            thread: Some(thread),
        }
    }

    /// Answers the browser with `page`, and waits until it is sent.
    fn finish(mut self, page: Page) {
        self.end(page);
    }

    fn end(&mut self, page: Page) {
        if let Some(send) = self.send.take() {
            let _ = send.send(page);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.end(Page::Failed);
    }
}

/// The device sign-in: starts it, prints the page and the code to type,
/// and polls until a person approves or denies it, or the code runs out.
/// The browser is never opened, and the code is never put in an address.
fn device(signin: &Signin, name: &str, stores: Option<&[String]>) -> Result<TokenAnswer, Error> {
    let server = signin.server();
    let mut device = signin
        .device_start(name, stores)
        .map_err(|error| match error {
            RemoteError::Rate { .. } => Error::new(format!(
                "{server}: too many sign-ins are waiting on the server; try again in a few minutes"
            )),
            error => Error::new(format!("{server}: {error}")),
        })?;
    let minutes = device.expires_in().as_secs().div_ceil(60);
    let plural = if minutes == 1 { "" } else { "s" };
    eprintln!(
        "Open {} in a browser signed in to your account,\nand type the code:\n\n    {}\n\nThe \
         code lasts {minutes} minute{plural}. Type it only if you started this sign-in.",
        clean(device.verification_uri()),
        clean(device.user_code())
    );
    let ran_out =
        || Error::new("the code ran out before it was approved: run cases login --device again");
    let deadline = Instant::now() + device.expires_in();
    let mut wait = device.interval();
    let mut warned = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        thread::sleep(wait.min(left));
        if Instant::now() >= deadline {
            return Err(ran_out());
        }
        let error = match signin.device_poll(&device) {
            Ok(answer) => return Ok(answer),
            Err(error) => error,
        };
        match device.wait_after(&error) {
            Some(next) => {
                if matches!(
                    error,
                    RemoteError::Transport { .. } | RemoteError::Server { .. }
                ) && !warned
                {
                    ctx::warn(format_args!(
                        "{server}: {error}; polling again until the code runs out"
                    ));
                    warned = true;
                }
                wait = next;
            }
            None => {
                return Err(match error {
                    RemoteError::Signin {
                        kind: SigninError::AccessDenied,
                        ..
                    } => Error::new("the sign-in was denied"),
                    RemoteError::Signin {
                        kind: SigninError::ExpiredToken,
                        ..
                    } => ran_out(),
                    RemoteError::Signin {
                        kind: SigninError::InvalidGrant,
                        message,
                    } => Error::new(format!(
                        "{server}: the server refused the sign-in: {}",
                        clean(&message)
                    )),
                    error => Error::new(format!("{server}: {error}")),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sign-in's secrets and its state, as the approval address has it.
    fn secrets() -> (Arc<Pkce>, String) {
        let pkce = Pkce::new().unwrap();
        let signin = Signin::new("https://cases.example".parse().unwrap());
        let url = signin.authorize_url(&pkce, &redirect_uri(1), "t", None);
        let state = url
            .split('&')
            .find_map(|pair| pair.strip_prefix("state="))
            .unwrap()
            .to_owned();
        (Arc::new(pkce), state)
    }

    /// Sends `request` to `port`, and gives the answer, all of it.
    fn send(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    /// A GET of `target` with the `Host` of `port`.
    fn get(port: u16, target: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").into_bytes()
    }

    #[test]
    fn the_listener_is_on_ipv4_loopback_only() {
        let listener = Listener::bind().unwrap();
        let addr = listener.listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback() && addr.is_ipv4(), "{addr}");
        assert_eq!(addr.port(), listener.port);
        assert_ne!(listener.port, 0);
    }

    #[test]
    fn nothing_coming_back_in_time_is_an_error() {
        let (pkce, _) = secrets();
        let started = Instant::now();
        let error = Listener::bind()
            .unwrap()
            .wait(&pkce, Duration::from_millis(300))
            .err()
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert_eq!(
            error.to_string(),
            "no approval came back within 0 seconds: run cases login again"
        );
        assert_eq!(span(LOOPBACK_WAIT), "5 minutes");
        assert_eq!(span(Duration::from_secs(60)), "a minute");
        assert_eq!(span(Duration::from_secs(1)), "a second");
    }

    #[test]
    fn a_preconnect_another_path_another_host_or_a_big_request_is_waited_past() {
        let (pkce, state) = secrets();
        let listener = Listener::bind().unwrap();
        let port = listener.port;
        let waiting = {
            let pkce = Arc::clone(&pkce);
            thread::spawn(move || listener.wait(&pkce, Duration::from_secs(20)))
        };
        // A preconnect that sends nothing holds nothing up.
        let idle = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let callback = format!("/callback?code=c0de&state={state}");
        for (request, status) in [
            (get(port, "/"), "404"),
            (get(port, "/favicon.ico"), "404"),
            (
                get(port, &format!("/elsewhere?code=c0de&state={state}")),
                "404",
            ),
            (
                format!("POST {callback} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").into_bytes(),
                "404",
            ),
            (
                format!("GET {callback} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").into_bytes(),
                "404",
            ),
            (
                format!("GET {callback} HTTP/1.1\r\n\r\n").into_bytes(),
                "404",
            ),
            (
                format!("GET {callback} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: evil\r\n\r\n")
                    .into_bytes(),
                "404",
            ),
            // A head with no end in 16 KiB: exactly that much, so the
            // listener reads it all and its answer is not cut off.
            (
                {
                    let mut request =
                        format!("GET {callback} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX: ")
                            .into_bytes();
                    request.resize(REQUEST_MAX, b'a');
                    request
                },
                "431",
            ),
            (get(port, &format!("/callback?state={state}")), "400"),
        ] {
            let answer = send(port, &request);
            assert!(
                answer.starts_with(&format!("HTTP/1.1 {status} ")),
                "{status}: {answer}"
            );
        }
        // The callback itself still comes through.
        let mut browser = TcpStream::connect(("127.0.0.1", port)).unwrap();
        browser.write_all(&get(port, &callback)).unwrap();
        let (code, held) = waiting.join().unwrap().unwrap();
        assert_eq!(code, "c0de");
        held.finish(Page::SignedIn("<ryan>".into()));
        let mut page = String::new();
        browser.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(
            page.contains("Signed in as &lt;ryan&gt;. You can close this tab."),
            "{page}"
        );
        for header in [
            "Content-Security-Policy: default-src 'none'\r\n",
            "Cache-Control: no-store\r\n",
            "Referrer-Policy: no-referrer\r\n",
            "Connection: close\r\n",
        ] {
            assert!(page.contains(header), "{header}: {page}");
        }
        assert!(!page.contains("<script"), "{page}");
        assert!(!page.contains("c0de"), "{page}");
        drop(idle);
    }

    #[test]
    fn another_state_is_refused_and_the_wait_goes_on() {
        let (pkce, state) = secrets();
        let listener = Listener::bind().unwrap();
        let port = listener.port;
        let waiting = {
            let pkce = Arc::clone(&pkce);
            thread::spawn(move || listener.wait(&pkce, Duration::from_secs(20)))
        };
        let mut other = state.clone();
        other.replace_range(..1, if state.starts_with('A') { "B" } else { "A" });
        for target in [
            format!("/callback?code=stolen&state={other}"),
            "/callback?code=stolen".to_owned(),
            format!("/callback?error=access_denied&state={other}"),
        ] {
            let answer = send(port, &get(port, &target));
            assert!(answer.starts_with("HTTP/1.1 400 "), "{answer}");
            assert!(
                answer.contains("This is not the sign-in the terminal started."),
                "{answer}"
            );
        }
        let mut browser = TcpStream::connect(("127.0.0.1", port)).unwrap();
        browser
            .write_all(&get(port, &format!("/callback?code=mine&state={state}")))
            .unwrap();
        let (code, held) = waiting.join().unwrap().unwrap();
        assert_eq!(code, "mine");
        // Dropped unfinished: the page says the sign-in failed.
        drop(held);
        let mut page = String::new();
        browser.read_to_string(&mut page).unwrap();
        assert!(
            page.contains("The sign-in failed: see the terminal."),
            "{page}"
        );
        // The listener takes no more connections.
        thread::sleep(Duration::from_millis(100));
        let late = TcpStream::connect(("127.0.0.1", port))
            .and_then(|mut stream| {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.write_all(&get(port, "/"))?;
                let mut answer = String::new();
                stream.read_to_string(&mut answer)?;
                Ok(answer)
            })
            .unwrap_or_default();
        assert!(late.is_empty(), "{late}");
    }

    #[test]
    fn a_denial_ends_the_wait_with_its_error_cleaned() {
        let (pkce, state) = secrets();
        let listener = Listener::bind().unwrap();
        let port = listener.port;
        let waiting = {
            let pkce = Arc::clone(&pkce);
            thread::spawn(move || listener.wait(&pkce, Duration::from_secs(20)))
        };
        let answer = send(
            port,
            &get(
                port,
                &format!("/callback?error=access_denied%1B%5B31m&state={state}"),
            ),
        );
        assert!(
            answer.contains("The sign-in was denied. You can close this tab."),
            "{answer}"
        );
        let error = waiting.join().unwrap().err().unwrap();
        assert_eq!(
            error.to_string(),
            "the sign-in was denied (access_denied[31m)"
        );
    }

    #[test]
    fn the_listener_keeps_to_the_design_s_limits() {
        assert_eq!(LOOPBACK_WAIT, Duration::from_secs(300));
        assert_eq!(PAGE_WAIT, Duration::from_secs(20));
        assert_eq!(READ_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REQUEST_MAX, 16 * 1024);
    }

    #[test]
    fn the_scope_is_store_names_split_by_commas() {
        assert_eq!(scope(None).unwrap(), None);
        assert_eq!(scope(Some(" ")).unwrap(), None);
        assert_eq!(
            scope(Some("team-x")).unwrap(),
            Some(vec!["team-x".to_owned()])
        );
        assert_eq!(
            scope(Some("team-x, ops,team-x")).unwrap(),
            Some(vec!["team-x".to_owned(), "ops".to_owned()])
        );
        for bad in [
            "st_0000000000000001",
            "https://cases.example/team-x",
            "./cases.db",
            "Team-X",
            "team-x,",
            &"a".repeat(64),
        ] {
            let error = scope(Some(bad)).unwrap_err().to_string();
            assert!(
                error.starts_with("--store for login takes store names"),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn a_name_is_1_to_100_characters_with_no_control_characters() {
        assert_eq!(checked_name("agent one").unwrap(), "agent one");
        assert!(checked_name(&"é".repeat(100)).is_ok());
        for bad in ["", "  ", "a\nb", "a\u{1b}b", &"a".repeat(101)] {
            assert!(checked_name(bad).is_err(), "{bad:?}");
        }
        let name = default_name(None);
        assert!(name.starts_with("cases CLI"), "{name}");
        let with_file = default_name(Some(Path::new("/tmp/ci.token")));
        assert!(with_file.ends_with(", ci.token"), "{with_file}");
        let long = default_name(Some(Path::new(&"x".repeat(300))));
        assert_eq!(long.chars().count(), NAME_MAX);
    }

    #[cfg(unix)]
    #[test]
    fn a_token_file_is_private_and_never_replaced_unless_forced() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let token: Token = "csu_000000000000000000000000000000000000".parse().unwrap();
        let path = dir.path().join("agent.token");
        assert_eq!(target_file(&path, false).unwrap(), path);
        write_token(&path, &token, false).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{}\n", token.expose())
        );
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(
            target_file(&path, false)
                .unwrap_err()
                .to_string()
                .ends_with("is already there: add --force to replace it")
        );
        assert!(write_token(&path, &token, false).is_err());
        // Forced, it is replaced through a temporary file, left nowhere.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(target_file(&path, true).unwrap(), path);
        write_token(&path, &token, true).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        // A link is refused, forced or not, and so is a missing directory.
        let link = dir.path().join("link.token");
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), &link).unwrap();
        for force in [false, true] {
            let error = target_file(&link, force).unwrap_err().to_string();
            assert!(error.contains("is a symbolic link"), "{error}");
        }
        assert!(write_token(&link, &token, false).is_err());
        assert!(!dir.path().join("elsewhere").exists());
        let error = target_file(&dir.path().join("none/agent.token"), false)
            .unwrap_err()
            .to_string();
        assert!(error.ends_with("is not there: make it first"), "{error}");
        let error = target_file(dir.path(), true).unwrap_err().to_string();
        assert!(error.ends_with("is a directory"), "{error}");
    }

    #[test]
    fn html_is_escaped_on_the_page() {
        assert_eq!(
            escape_html(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }
}
