//! A forwarded store: the store on another machine, reached over ssh, a
//! command of the user's, or HTTP. Each connection runs `ssh host cases
//! dial-stdio`, or the command an `exec:` forward names, which ends in
//! `cases dial-stdio` on the far side; that serves the store routes against
//! the store there, and the store routes go over the pipe, as HTTP/1.1.
//! Auth is ssh's, or the command's: nothing listens on the network, and no
//! token is sent. An `http://` or `https://` forward sends the same store
//! routes to a `cases serve` on the far side, through a proxy such as
//! `tailscale serve`, with the token of `forward-token-file`, if there is
//! one, or else the Tailscale login the proxy names. The client folds the
//! rows with its own copy of the core, so every command works on it as on
//! a local store.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cases_core::{
    ActorRow, Append, Case, Changes, Key, Listening, Row, Store, StoreError, Version, Wait, Writer,
};
use cases_store_remote::{
    Client, DIAL_STORE, Dial, Error as RemoteError, Hold, RemoteStore, Server, Token,
};

use crate::error::Error;
use crate::store::Failure;

/// The scheme of a forward that runs a command: `exec:<command>`.
const EXEC: &str = "exec:";

/// Where a forwarded store is, checked: `ssh://[user@]host[:port]`, with
/// the scheme in lower case and no `/` at the end, or `exec:<command>`, a
/// command that gives a pipe to `cases dial-stdio` on the far side, such as
/// `exec:docker exec -i box cases dial-stdio`, or `http://host[:port]` or
/// `https://host[:port]`, a `cases serve` on the far side, through a proxy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Forward {
    /// Over ssh, to a host.
    Ssh {
        user: Option<String>,
        host: String,
        port: Option<u16>,
    },
    /// Over a command, run by `sh -c`.
    Exec(String),
    /// Over HTTP, to a `cases serve`.
    Http(Server),
}

impl Forward {
    /// Whether `text` is meant as a forward: it starts `ssh://` or
    /// `exec:`, in any case. An `http://` or `https://` forward is taken
    /// from the config file's `forward` or `CASES_FORWARD` alone: a store
    /// given as an `https://` URL is a hosted store's.
    pub(crate) fn is_forward(text: &str) -> bool {
        text.split_once("://")
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("ssh"))
            || exec_command(text).is_some()
    }

    /// How each connection is made.
    ///
    /// Over ssh: `ssh -T -o BatchMode=yes -o ConnectTimeout=10 [-p port] --
    /// [user@]host cases dial-stdio`, with the ssh on the PATH and the
    /// user's ssh config. `-T` keeps a tty off the pipe, which would garble
    /// the HTTP stream even with `RequestTTY force` in that config, and
    /// `BatchMode` makes a host-key or password prompt fail the dial rather
    /// than hang it or break into the terminal. `ConnectTimeout` limits
    /// connecting and ssh's handshake to 10 s a try, rather than the
    /// request's whole timeout; on the command line, it wins over one in
    /// that config.
    ///
    /// Over a command: `sh -c <command>`, so that it is quoted, and may set
    /// variables, as it would be at a shell. A shell runs the last command
    /// of `-c` in its own place, so the pipe is that command's, and it ends
    /// when its stdin does, as ssh's does.
    ///
    /// Over HTTP, there is none: `None`.
    pub(crate) fn dial(&self) -> Option<Dial> {
        let (user, host, port) = match self {
            Forward::Exec(command) => {
                return Some(Dial::new("sh", self.to_string()).args(["-c", command.as_str()]));
            }
            Forward::Http(_) => return None,
            Forward::Ssh { user, host, port } => (user, host, port),
        };
        let mut args: Vec<String> = ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
            .map(str::to_owned)
            .into();
        if let Some(port) = port {
            args.push("-p".to_owned());
            args.push(port.to_string());
        }
        args.push("--".to_owned());
        // ssh takes an address bare, `::1`, not in the brackets a URL
        // gives it, which it would look up as a name.
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        args.push(match user {
            Some(user) => format!("{user}@{host}"),
            None => host.to_owned(),
        });
        args.push("cases".to_owned());
        args.push("dial-stdio".to_owned());
        Some(Dial::new("ssh", self.to_string()).args(args))
    }
}

/// The command of `text`, if it starts `exec:` in any case.
fn exec_command(text: &str) -> Option<&str> {
    let scheme = text.get(..EXEC.len())?;
    scheme
        .eq_ignore_ascii_case(EXEC)
        .then(|| &text[EXEC.len()..])
}

impl fmt::Display for Forward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (user, host, port) = match self {
            Forward::Exec(command) => return write!(f, "{EXEC}{command}"),
            Forward::Http(server) => return server.fmt(f),
            Forward::Ssh { user, host, port } => (user, host, port),
        };
        f.write_str("ssh://")?;
        if let Some(user) = user {
            write!(f, "{user}@")?;
        }
        f.write_str(host)?;
        if let Some(port) = port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

impl FromStr for Forward {
    type Err = Error;

    fn from_str(text: &str) -> Result<Forward, Error> {
        let bad = |why: &str| {
            Error::new(format!(
                "{text:?} is not a forward: {why}; it is ssh://[user@]host[:port], \
                 exec:<command> for a command that runs cases dial-stdio on the far side, or \
                 http://host[:port] for a cases serve there"
            ))
        };
        if let Some(command) = exec_command(text) {
            let command = command.trim();
            if command.is_empty() {
                return Err(bad("exec: has no command"));
            }
            return Ok(Forward::Exec(command.to_owned()));
        }
        let rest = match text.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("ssh") => rest,
            Some((scheme, _))
                if scheme.eq_ignore_ascii_case("https") && !cfg!(feature = "remote") =>
            {
                return Err(Error::new(format!(
                    "{text:?}: this build of cases has no TLS, so it cannot reach an https:// \
                     forward; use http:// to a host on a tailnet, whose WireGuard encrypts it \
                     (tailscale serve --http=PORT on the far side), or a build with the remote \
                     feature"
                )));
            }
            Some((scheme, _))
                if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
            {
                return Server::forward(text)
                    .map(Forward::Http)
                    .map_err(|error| bad(&error.to_string()));
            }
            _ => return Err(bad("the scheme is not ssh, exec, http or https")),
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['/', '?', '#']) {
            return Err(bad("it may not have a path, a query or a fragment"));
        }
        let (user, host) = match rest.rsplit_once('@') {
            Some((user, host)) => (Some(user), host),
            None => (None, rest),
        };
        if let Some(user) = user
            && !name(user)
        {
            return Err(bad("the user is not one"));
        }
        let (host, port) = match host.rsplit_once(':') {
            Some((host, port)) if !host.starts_with('[') || host.ends_with(']') => {
                (host, Some(port))
            }
            _ => (host, None),
        };
        let port = match port {
            None => None,
            Some(port) => match port.parse::<u16>() {
                Ok(port) if port > 0 => Some(port),
                _ => return Err(bad("the port is not one")),
            },
        };
        if !host_name(host) {
            return Err(bad("the host is not one"));
        }
        Ok(Forward::Ssh {
            user: user.map(str::to_owned),
            host: host.to_owned(),
            port,
        })
    }
}

/// Whether `text` is a user's name: letters, digits, `.`, `_` and `-`, not
/// starting with `-`, so that ssh never reads it as an option.
fn name(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Whether `text` is a host: a name of letters, digits, `.`, `_` and `-`
/// not starting with `-` or `.`, or an address in brackets.
fn host_name(text: &str) -> bool {
    if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return !inner.is_empty() && inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
    }
    name(text) && !text.starts_with('.')
}

/// A forwarded store: the store routes over a pipe that [`Forward::dial`]
/// opens, with the forward it was named by for its errors.
#[derive(Debug)]
pub(crate) struct Forwarded {
    forward: Forward,
    /// In a mutex for [`Forwarded::until`], which gives it a deadline.
    store: Mutex<RemoteStore>,
}

/// Locks `mutex`, whatever a panic left in it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Forwarded {
    /// The store on the machine `forward` names, sending `token` over
    /// HTTP. Nothing is run or sent until a command reads or writes it.
    pub(crate) fn new(forward: &Forward, token: Option<Token>) -> Forwarded {
        match (forward, forward.dial()) {
            (Forward::Http(server), _) => {
                let client =
                    Client::forward(server.clone(), token).with_version(env!("CARGO_PKG_VERSION"));
                Forwarded {
                    forward: forward.clone(),
                    store: Mutex::new(dial_store(&client)),
                }
            }
            (_, Some(dial)) => Forwarded::with_dial(forward, dial),
            (_, None) => unreachable!("only an HTTP forward has no dial"),
        }
    }

    /// The store `dial` reaches, named as `forward`.
    pub(crate) fn with_dial(forward: &Forward, dial: Dial) -> Forwarded {
        let client = Client::dial(dial).with_version(env!("CARGO_PKG_VERSION"));
        Forwarded {
            forward: forward.clone(),
            store: Mutex::new(dial_store(&client)),
        }
    }

    /// The store now.
    pub(crate) fn store(&self) -> RemoteStore {
        lock(&self.store).clone()
    }

    /// Keeps every later request, and its retries, within `deadline` (see
    /// [`Client::with_deadline`]).
    pub(crate) fn until(&self, deadline: Option<Instant>) {
        let mut store = lock(&self.store);
        let client = store.client().clone().with_deadline(deadline);
        *store = dial_store(&client);
    }

    /// The held poll (see [`RemoteStore::changes`]).
    ///
    /// # Errors
    ///
    /// When the request fails.
    pub(crate) fn changes(&self, after: &Version, hold: Duration) -> Result<Version, Failure> {
        self.store()
            .changes(after, hold)
            .map_err(|error| self.fail(error))
    }

    /// The link of the inbox a serve on the far side gives, an allowed
    /// host's (IN-11), or `None` when there is none or the far side is too
    /// old to say (see [`RemoteStore::inbox_link`]).
    ///
    /// # Errors
    ///
    /// When the request fails.
    pub(crate) fn inbox_link(&self) -> Result<Option<String>, Failure> {
        self.store().inbox_link().map_err(|error| self.fail(error))
    }

    /// Takes a pickup wait's locks on the far side, held until the
    /// [`FarLocks`] are dropped (see [`RemoteStore::hold`]), or a signal
    /// lets them go (see [`on_signal`]).
    ///
    /// # Errors
    ///
    /// When another wait holds one of them there, the request fails, or
    /// the signals cannot be caught.
    pub(crate) fn hold(&self, scopes: &[(&'static str, String)]) -> Result<FarLocks, Failure> {
        let locks: Vec<(&str, &str)> = scopes
            .iter()
            .map(|(on, text)| (*on, text.as_str()))
            .collect();
        // Caught before the locks are asked for, and the slot held until
        // they are in it, so that a signal on the way lets go of them too.
        #[cfg(unix)]
        let slot = on_signal()
            .map_err(|error| Failure::Named(format!("cannot catch signals: {error}")))?;
        #[cfg(unix)]
        let mut slot = lock(&slot);
        let hold = self
            .store()
            .hold(&locks)
            .map_err(|error| self.fail(error))?;
        let shared = Arc::new(Shared {
            hold,
            writing: Mutex::new(()),
            owed: Mutex::new(Vec::new()),
        });
        #[cfg(unix)]
        {
            *slot = Some(Arc::clone(&shared));
        }
        Ok(FarLocks {
            shared,
            forward: self.forward.to_string(),
        })
    }

    /// `error` as a [`Failure`] of this store.
    fn fail(&self, error: RemoteError) -> Failure {
        Failure::Forward {
            error,
            forward: self.forward.to_string(),
        }
    }

    /// A write's error as a [`StoreError`] of this store.
    fn write_fail(&self, error: StoreError<RemoteError>) -> StoreError<Failure> {
        match error {
            StoreError::Refused(error) => StoreError::Refused(error),
            StoreError::NoCase => StoreError::NoCase,
            StoreError::Store(error) => StoreError::Store(self.fail(error)),
            other => StoreError::Store(Failure::Named(other.to_string())),
        }
    }
}

/// A pickup wait's locks on a forwarded store's machine, held until they
/// are dropped, which lets them go there at once, or their lease there runs
/// out.
#[derive(Debug)]
pub(crate) struct FarLocks {
    shared: Arc<Shared>,
    forward: String,
}

/// What a [`FarLocks`] shares with the thread that lets them go on a
/// signal ([`on_signal`]).
#[derive(Debug)]
struct Shared {
    hold: Hold,
    /// Held while the wait writes under the locks, such as a pickup, and
    /// by the signal's thread from before it lets them go, so that they are
    /// never let go with a write on its way.
    writing: Mutex<()>,
    /// The cases the wait took, picked up, and has not printed yet: a
    /// signal names them on stderr before it ends the wait, so that none is
    /// taken and lost unsaid.
    owed: Mutex<Vec<Key>>,
}

impl Drop for FarLocks {
    fn drop(&mut self) {
        self.shared.hold.release();
    }
}

impl FarLocks {
    /// Begins a write under the locks, such as a pickup: they are not let
    /// go on a signal until the [`Writing`] it gives is dropped.
    ///
    /// # Errors
    ///
    /// When they may have been let go.
    pub(crate) fn write(&self) -> Result<Writing<'_>, Failure> {
        let writing = lock(&self.shared.writing);
        self.check()?;
        Ok(Writing {
            _writing: writing,
            owed: &self.shared.owed,
        })
    }

    /// The cases taken and not printed yet (see [`Writing::owe`]), which a
    /// signal names on stderr. One is taken out once its line is printed,
    /// so a signal while it is printed names it too: it may name one that
    /// was printed, but never misses one that was not, and never waits on a
    /// stdout that nobody reads.
    pub(crate) fn owed(&self) -> MutexGuard<'_, Vec<Key>> {
        lock(&self.shared.owed)
    }

    /// How long the locks are held for still, if they are on a lease (see
    /// [`Hold::left`]).
    ///
    /// # Errors
    ///
    /// When they may have been let go.
    pub(crate) fn left(&self) -> Result<Option<Duration>, Failure> {
        self.shared.hold.left().map_err(|error| Failure::Forward {
            error,
            forward: self.forward.clone(),
        })
    }

    /// Whether the locks are held still.
    ///
    /// # Errors
    ///
    /// When they may have been let go.
    pub(crate) fn check(&self) -> Result<(), Failure> {
        self.left().map(|_| ())
    }
}

/// The far locks a signal lets go of, once they are taken.
type Slot = Arc<Mutex<Option<Arc<Shared>>>>;

/// Lets go of the far locks put in the slot it gives when SIGINT, SIGTERM or
/// SIGHUP (its terminal closed) comes, each unless it is left alone (see
/// [`left_alone`]), names on stderr each case the wait
/// took and did not print, and then dies by the signal, as it would have
/// with no handler, so that a shell script around it stops too: once a
/// write under them on its way has ended, and before another. The cases are
/// named, not printed: stdout is the wait's, which may be in the middle of
/// a line. A signal that comes while the slot is
/// held, as the locks are asked for, waits for them. A second signal ends it
/// at once, in case letting go is stuck. A Ctrl-C at the terminal reaches this process alone, not the
/// connection that holds the locks (see [`cases_store_remote::Dial`]), so
/// that it is there to let go over.
///
/// # Errors
///
/// When the signals cannot be caught.
#[cfg(unix)]
fn on_signal() -> std::io::Result<Slot> {
    use std::sync::atomic::{AtomicBool, Ordering};

    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use signal_hook::flag;
    use signal_hook::iterator::Signals;

    let once = Arc::new(AtomicBool::new(false));
    let caught: Vec<_> = [SIGINT, SIGTERM, SIGHUP]
        .into_iter()
        .filter(|&signal| !left_alone(signal))
        .collect();
    for &signal in &caught {
        flag::register_conditional_default(signal, Arc::clone(&once))?;
    }
    let mut signals = Signals::new(caught)?;
    let slot = Slot::default();
    let catching = Arc::clone(&slot);
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            once.store(true, Ordering::SeqCst);
            if let Some(shared) = lock(&catching).as_ref() {
                let _writing = lock(&shared.writing);
                shared.hold.release();
                for key in lock(&shared.owed).iter() {
                    crate::ctx::warn(format_args!(
                        "{key}: picked up, but not printed: a signal ended the wait"
                    ));
                }
            }
            // Its default ends the process; were it to fail, exit as a
            // shell says a process the signal ended did.
            let _ = signal_hook::low_level::emulate_default_handler(signal);
            std::process::exit(128 + signal);
        }
    });
    Ok(slot)
}

/// Whether `signal` is left as the process started with it, not caught: on
/// Linux, one it started with ignored, as `nohup` starts it with SIGHUP and
/// a shell a job it runs in the background with SIGINT, which a handler
/// would undo. Elsewhere, or with no `/proc` to read it from, the start
/// cannot be read without unsafe code, so SIGHUP, the one `nohup` ignores,
/// is left alone, as it was before it was caught.
#[cfg(unix)]
fn left_alone(signal: i32) -> bool {
    // The mask of the ignored signals, in hex, with signal n at bit n - 1.
    #[cfg(target_os = "linux")]
    let ignored = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            let mask = status
                .lines()
                .find_map(|line| line.strip_prefix("SigIgn:"))?;
            u64::from_str_radix(mask.trim(), 16).ok()
        });
    #[cfg(not(target_os = "linux"))]
    let ignored: Option<u64> = None;
    match ignored {
        Some(mask) => (mask >> (signal - 1)) & 1 == 1,
        None => signal == signal_hook::consts::SIGHUP,
    }
}

/// A write under a pickup wait's far locks, on its way: they are not let go
/// on a signal until it is dropped.
#[derive(Debug)]
pub(crate) struct Writing<'a> {
    _writing: MutexGuard<'a, ()>,
    owed: &'a Mutex<Vec<Key>>,
}

impl Writing<'_> {
    /// Records that the write took the case `key`, which the wait now owes
    /// a line: a signal before it is printed names it (see
    /// [`FarLocks::owed`]).
    pub(crate) fn owe(&self, key: Key) {
        lock(self.owed).push(key);
    }
}

/// The one store of `client`, the store `cases dial-stdio` serves.
fn dial_store(client: &Client) -> RemoteStore {
    client
        .store(DIAL_STORE)
        .unwrap_or_else(|_| unreachable!("DIAL_STORE is a store's id"))
}

impl Store for Forwarded {
    type Key = Key;
    type Writer = Writer;
    type Error = Failure;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Failure>> {
        self.store()
            .create(open, writer)
            .map_err(|e| self.write_fail(e))
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Failure>> {
        self.store()
            .append(key, append, writer, at_revision)
            .map_err(|e| self.write_fail(e))
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Failure> {
        self.store().rows(key).map_err(|e| self.fail(e))
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Failure> {
        self.store().list().map_err(|e| self.fail(e))
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        self.store()
            .archive(key, at_revision)
            .map_err(|e| self.write_fail(e))
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Failure>> {
        self.store()
            .delete(key, at_revision)
            .map_err(|e| self.write_fail(e))
    }

    fn version(&self) -> Result<Version, Failure> {
        self.store().version().map_err(|e| self.fail(e))
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Failure> {
        self.store().rows_since(since).map_err(|e| self.fail(e))
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Failure> {
        self.store().actors().map_err(|e| self.fail(e))
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Failure> {
        self.store()
            .listen(wait, worker, labels, kinds, doing)
            .map_err(|e| self.fail(e))
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Failure> {
        self.store()
            .set_doing(worker, text)
            .map_err(|e| self.fail(e))
    }

    fn waits(&self) -> Result<Vec<Wait>, Failure> {
        self.store().waits().map_err(|e| self.fail(e))
    }

    fn listeners(&self) -> Result<Vec<Listening>, Failure> {
        self.store().listeners().map_err(|e| self.fail(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(text: &str) -> Forward {
        text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn args(forward: &Forward) -> Vec<String> {
        forward
            .dial()
            .unwrap()
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_forward_is_ssh_a_host_and_maybe_a_user_and_a_port() {
        for (text, shown, dial) in [
            ("ssh://devbox", "ssh://devbox", vec!["--", "devbox"]),
            ("SSH://devbox/", "ssh://devbox", vec!["--", "devbox"]),
            (
                "ssh://ryan@devbox.example.ts.net:2222",
                "ssh://ryan@devbox.example.ts.net:2222",
                vec!["-p", "2222", "--", "ryan@devbox.example.ts.net"],
            ),
            (
                "ssh://[::1]:22",
                "ssh://[::1]:22",
                vec!["-p", "22", "--", "::1"],
            ),
            ("ssh://[::1]", "ssh://[::1]", vec!["--", "::1"]),
            (
                "ssh://ryan@[fe80::1]",
                "ssh://ryan@[fe80::1]",
                vec!["--", "ryan@fe80::1"],
            ),
            ("ssh://my_host", "ssh://my_host", vec!["--", "my_host"]),
        ] {
            let forward = forward(text);
            assert_eq!(forward.to_string(), shown);
            let mut want: Vec<String> = ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
                .into_iter()
                .chain(dial)
                .map(str::to_owned)
                .collect();
            want.extend(["cases".to_owned(), "dial-stdio".to_owned()]);
            assert_eq!(args(&forward), want, "{text}");
            assert_eq!(forward.dial().unwrap().program(), "ssh");
            assert_eq!(forward.dial().unwrap().to_string(), shown);
        }
    }

    #[test]
    fn what_is_not_a_forward_is_refused() {
        for text in [
            "devbox",
            "ftp://devbox",
            "http://devbox/path",
            "http://devbox?x",
            "http://user@devbox",
            "http://devbox:port",
            "ssh://",
            "ssh://-oProxyCommand=x",
            "ssh://-devbox",
            "ssh://.devbox",
            "ssh://devbox/path",
            "ssh://devbox?x",
            "ssh://devbox#x",
            "ssh://devbox:0",
            "ssh://devbox:65536",
            "ssh://devbox:port",
            "ssh://devbox:",
            "ssh://@devbox",
            "ssh://-l@devbox",
            "ssh://a b@devbox",
            "ssh://be rt",
            "ssh://[]",
            "ssh://[zz]",
            "ssh://devbox;rm",
            "exec:",
            "EXEC:   ",
        ] {
            assert!(text.parse::<Forward>().is_err(), "{text}");
        }
    }

    #[test]
    fn a_refused_forward_says_why() {
        let tail = "; it is ssh://[user@]host[:port], exec:<command> for a command that runs \
                    cases dial-stdio on the far side, or http://host[:port] for a cases serve \
                    there";
        for (text, why) in [
            ("devbox", "the scheme is not ssh, exec, http or https"),
            ("ftp://devbox", "the scheme is not ssh, exec, http or https"),
            ("exec:", "exec: has no command"),
            ("EXEC:   ", "exec: has no command"),
            (
                "ssh://devbox/path",
                "it may not have a path, a query or a fragment",
            ),
            (
                "ssh://devbox?x",
                "it may not have a path, a query or a fragment",
            ),
            (
                "ssh://devbox#x",
                "it may not have a path, a query or a fragment",
            ),
            ("ssh://@devbox", "the user is not one"),
            ("ssh://-l@devbox", "the user is not one"),
            ("ssh://devbox:0", "the port is not one"),
            ("ssh://devbox:65536", "the port is not one"),
            ("ssh://devbox:", "the port is not one"),
            ("ssh://", "the host is not one"),
            ("ssh://.devbox", "the host is not one"),
            ("ssh://[zz]", "the host is not one"),
        ] {
            let error = text.parse::<Forward>().unwrap_err().to_string();
            assert_eq!(error, format!("{text:?} is not a forward: {why}{tail}"));
        }
        // An http forward's refusal carries the address parser's reason.
        let error = "http://devbox/path"
            .parse::<Forward>()
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("\"http://devbox/path\" is not a forward: ") && error.ends_with(tail),
            "{error}"
        );
    }

    #[test]
    fn what_is_taken_for_a_user_and_a_port() {
        // The last `@` and the last `:` split the user and the port off.
        assert_eq!(
            forward("ssh://a.b-c_d@host:65535"),
            Forward::Ssh {
                user: Some("a.b-c_d".to_owned()),
                host: "host".to_owned(),
                port: Some(65535),
            }
        );
        assert_eq!(
            forward("ssh://host:1"),
            Forward::Ssh {
                user: None,
                host: "host".to_owned(),
                port: Some(1),
            }
        );
        // A bare address in brackets keeps its colons; only a port after
        // the closing bracket is one.
        assert_eq!(
            forward("ssh://[fe80::1]:2200"),
            Forward::Ssh {
                user: None,
                host: "[fe80::1]".to_owned(),
                port: Some(2200),
            }
        );
        assert!("ssh://[fe80::1".parse::<Forward>().is_err());
        assert!("ssh://fe80::1".parse::<Forward>().is_err());
    }

    #[test]
    fn a_forward_is_known_by_its_scheme_alone() {
        assert!(Forward::is_forward("ssh://devbox"));
        assert!(Forward::is_forward("SSH://devbox"));
        assert!(Forward::is_forward("ssh://not a host"));
        assert!(!Forward::is_forward("https://devbox"));
        assert!(!Forward::is_forward("devbox"));
        assert!(!Forward::is_forward("./ssh/cases.db"));
        assert!(Forward::is_forward(
            "exec:docker exec -i box cases dial-stdio"
        ));
        assert!(Forward::is_forward("Exec:"));
        assert!(!Forward::is_forward("exec"));
        assert!(!Forward::is_forward("./exec:cases.db"));
    }

    #[test]
    fn an_http_forward_is_a_serve_s_address() {
        for (text, shown) in [
            (
                "http://devbox.example.ts.net:7338",
                "http://devbox.example.ts.net:7338",
            ),
            ("HTTP://Devbox:7338/", "http://devbox:7338"),
        ] {
            let forward = forward(text);
            assert!(matches!(forward, Forward::Http(_)), "{text}");
            assert_eq!(forward.to_string(), shown);
            assert!(forward.dial().is_none());
        }
        // Taken from forward alone: --store https://… is a hosted store.
        assert!(!Forward::is_forward("http://devbox:7338"));
        let https = "https://devbox.example.ts.net".parse::<Forward>();
        if cfg!(feature = "remote") {
            assert!(matches!(https, Ok(Forward::Http(_))));
        } else {
            let error = https.unwrap_err().to_string();
            assert!(
                error.contains("no TLS") && error.contains("http://"),
                "{error}"
            );
        }
    }

    #[test]
    fn an_exec_forward_runs_its_command_by_sh() {
        for (text, shown, command) in [
            (
                "exec:docker exec -i box cases dial-stdio",
                "exec:docker exec -i box cases dial-stdio",
                "docker exec -i box cases dial-stdio",
            ),
            (
                "EXEC:  ssh -p 2 'my host' cases dial-stdio ",
                "exec:ssh -p 2 'my host' cases dial-stdio",
                "ssh -p 2 'my host' cases dial-stdio",
            ),
        ] {
            let forward = forward(text);
            assert_eq!(forward, Forward::Exec(command.to_owned()));
            assert_eq!(forward.to_string(), shown);
            assert_eq!(forward.dial().unwrap().program(), "sh");
            assert_eq!(args(&forward), ["-c", command]);
            assert_eq!(forward.dial().unwrap().to_string(), shown);
        }
    }

    #[test]
    fn a_deadline_is_kept_on_the_store_until_it_is_taken_away() {
        let forward = forward("exec:true");
        let forwarded = Forwarded::new(&forward, None);
        let shown = |f: &Forwarded| format!("{:?}", f.store().client());
        assert!(shown(&forwarded).contains("deadline: None"));
        forwarded.until(Some(Instant::now() + Duration::from_secs(60)));
        assert!(shown(&forwarded).contains("deadline: Some("));
        forwarded.until(None);
        assert!(shown(&forwarded).contains("deadline: None"));
    }

    /// The signals a process started with ignored are left alone. Rust's
    /// runtime starts every process, a test's too, with SIGPIPE ignored.
    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn a_signal_the_process_started_ignoring_is_left_alone() {
        use signal_hook::consts::{SIGPIPE, SIGSYS, SIGUSR2};
        assert!(left_alone(SIGPIPE));
        assert!(!left_alone(SIGUSR2));
        // Above every signal that is ignored, nothing is left to shift in.
        assert!(!left_alone(SIGSYS));
    }
}
