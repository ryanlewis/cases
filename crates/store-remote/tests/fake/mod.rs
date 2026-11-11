//! A fake of the hosted server's agent API, for the remote store's tests:
//! plain HTTP/1.1 on `127.0.0.1`, one thread per connection, and one
//! answer per connection. Behind it each store is a `cases-store-sqlite`
//! store of its own, with a clock the test sets.
//!
//! It speaks the store routes of spec/api-agents.md, `GET /v1/cli/whoami`,
//! the bearer check, the held poll and the error shapes. It can be told to
//! answer the next requests with a fault ([`Fault`]), and it keeps every
//! request as it came, so a test can check what went on the wire.
//!
//! It also speaks the sign-in routes as the hosted Worker does: the
//! loopback exchange (`POST /v1/cli/token`), the device start and poll
//! (`POST /v1/cli/device`, `POST /v1/cli/device/token`) and the revoke
//! (`DELETE /v1/accounts/{account}/cli/token`). [`Fake::approve`] and
//! [`Fake::deny`] play the approval page and the person behind it, and
//! [`Fake::approve_device`] and its kin the `/device` page. Each token it
//! mints is kept with its scope, and checked on every route as the fixed
//! [`TOKEN`] is.
//!
//! It uses no crate of the remote store's, so the unit tests and the
//! integration tests can both take it, and so can the CLI's tests
//! (`crates/cli/tests/remote.rs`), which run the `cases` binary against it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "the fake panics to fail the test that runs it"
)]
// Each test binary uses part of this module.
#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Append, Changes, Key, Row, Store, StoreError, Time};
use cases_store_sqlite::{Db, Error as DbError, Writer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// The token the fake takes from the start, named `tests`, for every store.
pub(crate) const TOKEN: &str = "csu_fAkEfAkEfAkEfAkEfAkEfAkEfAkEfAkEfAkE";

/// The fake's one account.
pub(crate) const ACCOUNT: &str = "ac_fake";

/// When every token expires, as whoami says.
const EXPIRES: &str = "2026-12-31T10:00:00Z";

/// How often a device may poll. The Worker's is 5 s.
pub(crate) const DEVICE_INTERVAL: Duration = Duration::from_secs(1);

/// How long a device code lasts, in seconds.
const DEVICE_LIFE: u64 = 600;

/// The most device sign-ins that may wait at once, as the Worker has it.
const DEVICES_WAITING: usize = 10;

/// The time each store's clock starts at.
pub(crate) const START: &str = "2026-09-15T09:00:00Z";

/// The most a request's body may hold, as the server has it.
const BODY_LIMIT: usize = 1024 * 1024;

/// What the fake answers a request with, in place of its own answer.
#[derive(Clone, Debug)]
pub(crate) enum Fault {
    /// This status, with an error body of category `store`.
    Status(u16),
    /// 429, with `Retry-After` of these seconds when given.
    Rate(Option<u64>),
    /// 302, to another path on the fake.
    Redirect,
    /// The fake's own answer, after this wait.
    Slow(Duration),
    /// 200, with a body of this many bytes that is not JSON.
    Oversized(usize),
    /// 401, as for a revoked token.
    Revoked,
    /// 410, as for a removed store.
    Gone,
    /// The connection closes with no answer.
    Hangup,
    /// 200, with a body that is not the API's.
    Garbage,
    /// 200, with this body.
    Body(String),
    /// 503 `unavailable`, as when a store could not take a new token: the
    /// code stays usable.
    Unavailable,
    /// The fake's own answer is made (a code is spent, a token minted), and
    /// then the connection closes with no answer.
    Lost,
    /// This status, with an error body of this category, such as 400
    /// `invalid_grant`.
    Refuse(u16, &'static str),
}

/// A token the fake takes: the fixed [`TOKEN`], or one it minted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Minted {
    pub(crate) token: String,
    pub(crate) name: String,
    /// The ids of the stores it opens, or `None` for every store.
    pub(crate) stores: Option<Vec<String>>,
    pub(crate) revoked: bool,
    pub(crate) expired: bool,
}

/// An approved loopback sign-in, waiting for its exchange.
struct Grant {
    challenge: String,
    redirect: String,
    name: String,
    stores: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeviceState {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// A device sign-in.
struct Device {
    code: String,
    user_code: String,
    name: String,
    /// The store names it asks for, or `None` for every store.
    names: Option<Vec<String>>,
    /// The ids they were resolved to on approval.
    stores: Option<Vec<String>>,
    state: DeviceState,
    polled: Option<Instant>,
}

/// A store on the fake.
struct Entry {
    name: String,
    default: bool,
    gone: bool,
    db: Arc<Db>,
    clock: Arc<Mutex<Time>>,
}

#[derive(Default)]
struct State {
    stores: Mutex<BTreeMap<String, Entry>>,
    faults: Mutex<VecDeque<Fault>>,
    /// Faults for the next writes of an event, ahead of `faults`.
    event_faults: Mutex<VecDeque<Fault>>,
    /// Whether every request but a write of an event fails with a 503.
    fail_reads: Mutex<bool>,
    /// Faults for the requests straight after a write of an event that met
    /// a fault of `event_faults`, ahead of `faults`.
    after_event: Mutex<VecDeque<Fault>>,
    requests: Mutex<Vec<String>>,
    tokens: Mutex<Vec<Minted>>,
    grants: Mutex<BTreeMap<String, Grant>>,
    devices: Mutex<Vec<Device>>,
    /// Counts what is made: tokens, codes, device codes.
    made: Mutex<u64>,
    /// Raised by every write, for the held poll.
    ticks: Mutex<u64>,
    changed: Condvar,
}

/// The fake server, which stops when the test process ends.
pub(crate) struct Fake {
    addr: SocketAddr,
    state: Arc<State>,
    dir: Dir,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a fake keeps its stores.
enum Dir {
    Temp(TempDir),
    Kept(std::path::PathBuf),
}

impl Dir {
    fn path(&self) -> &std::path::Path {
        match self {
            Dir::Temp(dir) => dir.path(),
            Dir::Kept(path) => path,
        }
    }
}

impl Fake {
    /// A fake on a port of its own, with no stores.
    pub(crate) fn start() -> Fake {
        Fake::start_in(Dir::Temp(tempfile::tempdir().unwrap()))
    }

    /// Like [`start`](Fake::start), for a fake kept in a `static`, which is
    /// never dropped: the stores go under `target/tmp/fake-<name>`, which is
    /// cleared first, so a run leaves one run's files and not a temp dir in
    /// `/tmp`. `name` must be unique to the test binary.
    pub(crate) fn start_kept(name: &str) -> Fake {
        // Only integration tests get the variable; elsewhere it is a temp dir.
        let Some(base) = option_env!("CARGO_TARGET_TMPDIR") else {
            return Fake::start();
        };
        let path = std::path::Path::new(base).join(format!("fake-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Fake::start_in(Dir::Kept(path))
    }

    fn start_in(dir: Dir) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(State::default());
        lock(&state.tokens).push(Minted {
            token: TOKEN.to_owned(),
            name: "tests".to_owned(),
            stores: None,
            revoked: false,
            expired: false,
        });
        let serving = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&serving);
                thread::spawn(move || serve(&state, addr, stream));
            }
        });
        Fake { addr, state, dir }
    }

    /// The fake's address, `http://127.0.0.1:port`.
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// A new, empty store named `name`, and its id. The account's first
    /// store is its default.
    pub(crate) fn add_store(&self, name: &str) -> String {
        let mut stores = lock(&self.state.stores);
        let id = format!("st_{:016}", stores.len() + 1);
        let start: Time = START.parse().unwrap();
        let clock = Arc::new(Mutex::new(start));
        let reads = Arc::clone(&clock);
        let db =
            Db::new(self.dir.path().join(format!("{id}.db"))).with_clock(move || *lock(&reads));
        let default = stores.is_empty();
        stores.insert(
            id.clone(),
            Entry {
                name: name.to_owned(),
                default,
                gone: false,
                db: Arc::new(db),
                clock,
            },
        );
        id
    }

    /// Sets the clock of the store `id`.
    pub(crate) fn set_clock(&self, id: &str, now: Time) {
        let stores = lock(&self.state.stores);
        *lock(&stores[id].clock) = now;
    }

    /// The store `id`'s own store, to read or write with none of the
    /// server's checks.
    pub(crate) fn db(&self, id: &str) -> Arc<Db> {
        Arc::clone(&lock(&self.state.stores)[id].db)
    }

    /// Wakes the held polls, after a write straight to a store's [`Db`].
    pub(crate) fn wake(&self) {
        *lock(&self.state.ticks) += 1;
        self.state.changed.notify_all();
    }

    /// Renames the store `id` to `name`.
    pub(crate) fn rename(&self, id: &str, name: &str) {
        name.clone_into(&mut lock(&self.state.stores).get_mut(id).unwrap().name);
    }

    /// Makes the store `id` the account's default, or not.
    pub(crate) fn set_default(&self, id: &str, default: bool) {
        lock(&self.state.stores).get_mut(id).unwrap().default = default;
    }

    /// Removes the store `id`: every route of it answers 410 after.
    pub(crate) fn remove(&self, id: &str) {
        lock(&self.state.stores).get_mut(id).unwrap().gone = true;
    }

    /// Adds a fault for the next write of an event, whatever is sent
    /// before it.
    pub(crate) fn fault_event(&self, fault: Fault) {
        lock(&self.state.event_faults).push_back(fault);
    }

    /// Adds a fault for the requests straight after the next write of an
    /// event that meets a fault.
    #[allow(
        dead_code,
        reason = "only some of the tests that share the fake use it"
    )]
    pub(crate) fn fault_after_event(&self, fault: Fault) {
        lock(&self.state.after_event).push_back(fault);
    }

    /// Fails every request from now on but a write of an event.
    #[allow(
        dead_code,
        reason = "only some of the tests that share the fake use it"
    )]
    pub(crate) fn fail_reads(&self) {
        *lock(&self.state.fail_reads) = true;
    }

    /// Takes away every fault queued.
    #[allow(
        dead_code,
        reason = "only some of the tests that share the fake use it"
    )]
    pub(crate) fn clear_faults(&self) {
        lock(&self.state.faults).clear();
    }

    /// Answers the next request with `fault`, after any queued before it.
    pub(crate) fn fault(&self, fault: Fault) {
        lock(&self.state.faults).push_back(fault);
    }

    /// Every request so far, as it came: its request line, its headers and
    /// its body.
    pub(crate) fn requests(&self) -> Vec<String> {
        lock(&self.state.requests).clone()
    }

    /// Plays the approval page and the person who approves: checks the
    /// query of `authorize_url` as the Worker does, keeps a grant for its
    /// code, and gives the address the browser is sent back to, the CLI's
    /// redirect with `code` and `state`. A test then GETs it on the CLI's
    /// port, as the browser would.
    ///
    /// # Errors
    ///
    /// Gives what the approval page would say for an ask that is not one.
    pub(crate) fn approve(&self, authorize_url: &str) -> Result<String, String> {
        let ask = self.ask(authorize_url)?;
        let stores = match &ask.stores {
            None => None,
            Some(names) => Some(store_ids(&self.state, names)?),
        };
        let code = format!("{ACCOUNT}.code{}", made(&self.state));
        lock(&self.state.grants).insert(
            code.clone(),
            Grant {
                challenge: ask.challenge,
                redirect: ask.redirect.clone(),
                name: ask.name,
                stores,
            },
        );
        Ok(format!(
            "{}?code={}&state={}",
            ask.redirect,
            encode(&code),
            encode(&ask.state)
        ))
    }

    /// As [`Fake::approve`], for a person who denies: the redirect with
    /// `error=access_denied` and the state.
    ///
    /// # Errors
    ///
    /// Gives what the approval page would say for an ask that is not one.
    pub(crate) fn deny(&self, authorize_url: &str) -> Result<String, String> {
        let ask = self.ask(authorize_url)?;
        Ok(format!(
            "{}?error=access_denied&state={}",
            ask.redirect,
            encode(&ask.state)
        ))
    }

    /// The ask in `authorize_url`, checked as the Worker's `/cli/authorize`
    /// does.
    fn ask(&self, authorize_url: &str) -> Result<Ask, String> {
        let prefix = format!("{}/cli/authorize?", self.url());
        let query = authorize_url
            .strip_prefix(&prefix)
            .ok_or_else(|| format!("not this fake's approval page: {authorize_url}"))?;
        let pairs = pairs(query);
        let get = |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        if get("code_challenge_method") != "S256" {
            return Err("The sign-in must use PKCE with S256.".into());
        }
        let challenge = get("code_challenge");
        if !challenge_ok(&challenge) {
            return Err("The sign-in's code challenge is not one.".into());
        }
        let redirect = get("redirect_uri");
        if !redirect_ok(&redirect) {
            return Err("The sign-in may only go back to http://127.0.0.1:<port>/callback.".into());
        }
        let state = get("state");
        if state.is_empty() || state.len() > 512 {
            return Err("The sign-in has no state.".into());
        }
        let name = get("name");
        if !name_ok(&name) {
            return Err("The token's name is 1 to 100 characters.".into());
        }
        let stores = get("stores");
        let stores =
            (!stores.is_empty()).then(|| stores.split(',').map(str::to_owned).collect::<Vec<_>>());
        if stores
            .as_ref()
            .is_some_and(|names| names.iter().any(|n| !store_name(n)))
        {
            return Err("A store name is not one.".into());
        }
        Ok(Ask {
            challenge,
            state,
            redirect,
            name,
            stores,
        })
    }

    /// The user codes of the device sign-ins that wait, as shown
    /// (`BCDF-GHJK`), oldest first.
    pub(crate) fn user_codes(&self) -> Vec<String> {
        lock(&self.state.devices)
            .iter()
            .filter(|d| d.state == DeviceState::Pending)
            .map(|d| d.user_code.clone())
            .collect()
    }

    /// Approves the device sign-in shown as `user_code`, as `/device`
    /// does: its store names become the ids of the account's stores.
    ///
    /// # Errors
    ///
    /// Gives why, for a code that waits for nothing or a store name the
    /// account does not have.
    pub(crate) fn approve_device(&self, user_code: &str) -> Result<(), String> {
        let mut devices = lock(&self.state.devices);
        let device = pending(&mut devices, user_code)?;
        device.stores = match &device.names {
            None => None,
            Some(names) => Some(store_ids(&self.state, names)?),
        };
        device.state = DeviceState::Approved;
        Ok(())
    }

    /// Denies the device sign-in shown as `user_code`.
    ///
    /// # Errors
    ///
    /// Gives why, for a code that waits for nothing.
    pub(crate) fn deny_device(&self, user_code: &str) -> Result<(), String> {
        pending(&mut lock(&self.state.devices), user_code)?.state = DeviceState::Denied;
        Ok(())
    }

    /// Makes the device sign-in shown as `user_code` run out.
    ///
    /// # Errors
    ///
    /// Gives why, for a code that waits for nothing.
    pub(crate) fn expire_device(&self, user_code: &str) -> Result<(), String> {
        pending(&mut lock(&self.state.devices), user_code)?.state = DeviceState::Expired;
        Ok(())
    }

    /// Every token the fake takes, the fixed [`TOKEN`] first, then each it
    /// minted in turn.
    pub(crate) fn tokens(&self) -> Vec<Minted> {
        lock(&self.state.tokens).clone()
    }

    /// Revokes `token`, as `/tokens` does.
    pub(crate) fn revoke(&self, token: &str) {
        token_mut(&self.state, token, |t| t.revoked = true);
    }

    /// Makes `token` expire.
    pub(crate) fn expire(&self, token: &str) {
        token_mut(&self.state, token, |t| t.expired = true);
    }

    /// The request lines so far, such as `GET /v1/cli/whoami HTTP/1.1`.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|r| r.lines().next().unwrap_or_default().to_owned())
            .collect()
    }
}

/// An answer: a status, its headers and its body.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn json(status: u16, value: &Value) -> Answer {
        Answer {
            status,
            headers: Vec::new(),
            body: value.to_string().into_bytes(),
        }
    }

    fn ok(value: &Value) -> Answer {
        Answer::json(200, value)
    }

    fn refuse(status: u16, category: &str, message: &str) -> Answer {
        Answer::json(
            status,
            &json!({"error": {"category": category, "message": message}}),
        )
    }
}

/// A request, as read.
struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

fn serve(state: &State, addr: SocketAddr, stream: TcpStream) {
    let Some((request, raw)) = read(&stream) else {
        return;
    };
    let event = raw
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST ") && line.contains("/events"));
    let event_fault = event
        .then(|| lock(&state.event_faults).pop_front())
        .flatten();
    if event_fault.is_some() {
        let mut after = lock(&state.after_event);
        let mut faults = lock(&state.faults);
        while let Some(fault) = after.pop_back() {
            faults.push_front(fault);
        }
    }
    let fault = event_fault.or_else(|| {
        if !event && *lock(&state.fail_reads) {
            return Some(Fault::Status(503));
        }
        lock(&state.faults).pop_front()
    });
    lock(&state.requests).push(raw);
    let answer = match fault {
        Some(Fault::Status(status)) => Answer::refuse(status, "store", "the fake failed"),
        Some(Fault::Unavailable) => Answer::refuse(
            503,
            "unavailable",
            "a store could not be reached; try again",
        ),
        Some(Fault::Refuse(status, category)) => {
            Answer::refuse(status, category, "the fake refused")
        }
        Some(Fault::Lost) => {
            handle(state, addr, &request);
            return;
        }
        Some(Fault::Rate(after)) => {
            let mut answer = Answer::refuse(429, "rate", "too many requests");
            if let Some(after) = after {
                answer
                    .headers
                    .push(("Retry-After".into(), after.to_string()));
            }
            answer
        }
        Some(Fault::Redirect) => Answer {
            status: 302,
            headers: vec![("Location".into(), format!("http://{addr}/elsewhere"))],
            body: Vec::new(),
        },
        Some(Fault::Slow(wait)) => {
            thread::sleep(wait);
            handle(state, addr, &request)
        }
        Some(Fault::Oversized(size)) => Answer {
            status: 200,
            headers: Vec::new(),
            body: vec![b'x'; size],
        },
        Some(Fault::Revoked) => Answer::refuse(401, "auth", "revoked"),
        Some(Fault::Gone) => Answer::refuse(410, "gone", "the store was removed"),
        Some(Fault::Hangup) => return,
        Some(Fault::Garbage) => Answer {
            status: 200,
            headers: Vec::new(),
            body: b"<html>not the API</html>".to_vec(),
        },
        Some(Fault::Body(body)) => Answer {
            status: 200,
            headers: Vec::new(),
            body: body.into_bytes(),
        },
        None => handle(state, addr, &request),
    };
    write(stream, &answer);
}

/// Reads one request, and gives it with its text as it came.
fn read(stream: &TcpStream) -> Option<(Request, String)> {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
    }
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let query = pairs(query);
    let raw = format!("{head}{}", String::from_utf8_lossy(&body));
    let request = Request {
        method,
        path: path.to_owned(),
        query,
        headers,
        body,
    };
    Some((request, raw))
}

/// The pairs of a query, decoded.
fn pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(n), decode(v))
        })
        .collect()
}

/// Percent-encodes all of `text` but RFC 3986's unreserved characters.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Percent-decodes `text`.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write(mut stream: TcpStream, answer: &Answer) {
    let mut head = format!(
        "HTTP/1.1 {} Fake\r\nContent-Length: {}\r\nConnection: close\r\n",
        answer.status,
        answer.body.len()
    );
    if !answer.body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    for (name, value) in &answer.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    // The client may give up on a big body part-way.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer.body);
    let _ = stream.flush();
}

/// The fake's own answer to `request`, in the server's order of checks:
/// the sign-in routes, which take no token; then the token, the store, the
/// body, the case, and then the core.
fn handle(state: &State, addr: SocketAddr, request: &Request) -> Answer {
    let parts: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
    match (request.method.as_str(), parts.as_slice()) {
        ("POST", ["v1", "cli", "token"]) => return exchange(state, request),
        ("POST", ["v1", "cli", "device"]) => return device_start(state, addr, request),
        ("POST", ["v1", "cli", "device", "token"]) => return device_token(state, request),
        ("DELETE", ["v1", "accounts", id, "cli", "token"]) if *id != ACCOUNT => {
            return Answer::refuse(404, "request", "no such account");
        }
        _ => {}
    }
    // The scheme is case-insensitive, as RFC 7235 has it and as `serve`
    // takes it: `bearer` is taken too. One space follows it.
    let bearer = request
        .header("authorization")
        .and_then(|v| v.split_at_checked("Bearer ".len()))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer "))
        .map(|(_, token)| token);
    let token = bearer.and_then(|bearer| {
        lock(&state.tokens)
            .iter()
            .find(|t| t.token == bearer)
            .cloned()
    });
    let Some(token) = token else {
        return Answer::refuse(401, "auth", "the token is not one");
    };
    // As the Worker: a revoked or expired token is refused here, on the
    // revoke too.
    if token.revoked {
        return Answer::refuse(401, "auth", "the token is revoked");
    }
    if token.expired {
        return Answer::refuse(401, "auth", "the token is expired");
    }
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["v1", "cli", "whoami"]) => whoami(state, &token),
        ("DELETE", ["v1", "accounts", _, "cli", "token"]) => {
            token_mut(state, &token.token, |t| t.revoked = true);
            Answer::ok(&json!({}))
        }
        (_, ["v1", "stores", id, "store", rest @ ..]) => {
            let found = {
                let stores = lock(&state.stores);
                stores.get(*id).map(|e| (e.gone, Arc::clone(&e.db)))
            };
            match found {
                None => Answer::refuse(404, "request", "no such store"),
                Some((true, _)) => Answer::refuse(410, "gone", "the store was removed"),
                Some(_) if !opens(&token, id) => {
                    Answer::refuse(401, "auth", "the token is not for this store")
                }
                Some((false, db)) => {
                    let answer = route(state, &db, request, rest);
                    if request.method != "GET" && answer.status == 200 {
                        *lock(&state.ticks) += 1;
                        state.changed.notify_all();
                    }
                    answer
                }
            }
        }
        _ => Answer::refuse(404, "request", "no such route"),
    }
}

/// Whether `token` opens the store `id`.
fn opens(token: &Minted, id: &str) -> bool {
    token
        .stores
        .as_ref()
        .is_none_or(|ids| ids.iter().any(|i| i == id))
}

fn whoami(state: &State, token: &Minted) -> Answer {
    let stores: Vec<Value> = lock(&state.stores)
        .iter()
        .filter(|(id, e)| !e.gone && opens(token, id))
        .map(|(id, e)| json!({"id": id, "name": e.name, "default": e.default}))
        .collect();
    Answer::ok(&json!({
        "account": {"id": ACCOUNT, "handle": "ryan"},
        "token": {"name": token.name, "stores": token.stores, "expires": EXPIRES},
        "stores": stores,
    }))
}

/// The next number for something made.
fn made(state: &State) -> u64 {
    let mut made = lock(&state.made);
    *made += 1;
    *made
}

/// Changes the kept `token` with `change`.
fn token_mut(state: &State, token: &str, change: impl FnOnce(&mut Minted)) {
    if let Some(found) = lock(&state.tokens).iter_mut().find(|t| t.token == token) {
        change(found);
    }
}

/// What the CLI asks for in an approval address.
struct Ask {
    challenge: String,
    state: String,
    redirect: String,
    name: String,
    stores: Option<Vec<String>>,
}

/// Whether `challenge` is an S256 challenge: 32 bytes as base64url.
fn challenge_ok(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `verifier` is a PKCE code verifier (RFC 7636, 4.1).
fn verifier_ok(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

/// The S256 challenge of `verifier`, worked out here rather than with the
/// client's code.
fn challenge_of(verifier: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let digest = Sha256::digest(verifier.as_bytes());
    let mut bits = 0u32;
    let mut held = 0;
    let mut out = String::new();
    for byte in digest {
        bits = (bits << 8) | u32::from(byte);
        held += 8;
        while held >= 6 {
            held -= 6;
            out.push(char::from(ALPHABET[((bits >> held) & 63) as usize]));
        }
    }
    if held > 0 {
        out.push(char::from(ALPHABET[((bits << (6 - held)) & 63) as usize]));
    }
    out
}

/// Whether `redirect` is exactly `http://127.0.0.1:<port>/callback`.
fn redirect_ok(redirect: &str) -> bool {
    redirect
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix("/callback"))
        .is_some_and(|port| {
            !port.is_empty()
                && !port.starts_with('0')
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok()
        })
}

/// Whether `name` can name a token: 1 to 100 characters, none a control.
fn name_ok(name: &str) -> bool {
    (1..=100).contains(&name.chars().count()) && !name.chars().any(char::is_control)
}

/// Whether `name` is a store's name: 1 to 63 lower-case letters, digits
/// and `-`.
fn store_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The ids of the live stores named `names`.
fn store_ids(state: &State, names: &[String]) -> Result<Vec<String>, String> {
    let stores = lock(&state.stores);
    names
        .iter()
        .map(|name| {
            stores
                .iter()
                .find(|(_, e)| !e.gone && e.name == *name)
                .map(|(id, _)| id.clone())
                .ok_or_else(|| format!("There is no store named {name}."))
        })
        .collect()
}

/// The device sign-in that waits with `user_code`, typed with or without
/// its dash.
fn pending<'a>(devices: &'a mut [Device], user_code: &str) -> Result<&'a mut Device, String> {
    let typed = user_code.replace('-', "").to_ascii_uppercase();
    devices
        .iter_mut()
        .find(|d| d.state == DeviceState::Pending && d.user_code.replace('-', "") == typed)
        .ok_or_else(|| format!("no sign-in waits with the code {user_code}"))
}

/// The 400 of OAuth's `invalid_grant`.
fn invalid_grant(message: &str) -> Answer {
    Answer::refuse(400, "invalid_grant", message)
}

/// Mints a token named `name` for the stores `stores` (ids, or `None` for
/// every store), and gives the token answer.
fn mint(state: &State, name: &str, stores: Option<Vec<String>>) -> Answer {
    let token = format!("csu_M{:035}", made(state));
    let opened: Vec<(String, String, bool)> = lock(&state.stores)
        .iter()
        .filter(|(id, e)| {
            !e.gone
                && stores
                    .as_ref()
                    .is_none_or(|ids| ids.iter().any(|i| i == *id))
        })
        .map(|(id, e)| (id.clone(), e.name.clone(), e.default))
        .collect();
    lock(&state.tokens).push(Minted {
        token: token.clone(),
        name: name.to_owned(),
        stores,
        revoked: false,
        expired: false,
    });
    let default = opened
        .iter()
        .find(|(_, _, default)| *default)
        .map(|(id, name, _)| json!({"id": id, "name": name}));
    let opened: Vec<Value> = opened
        .iter()
        .map(|(id, name, _)| json!({"id": id, "name": name}))
        .collect();
    Answer::ok(&json!({
        "token": token,
        "name": name,
        "account": {"id": ACCOUNT, "handle": "ryan"},
        "stores": opened,
        "default_store": default,
    }))
}

/// `POST /v1/cli/token` `{code, code_verifier, redirect_uri}`: spends the
/// code on a success, a bad verifier or another redirect.
fn exchange(state: &State, request: &Request) -> Answer {
    let bad = "the body is not {code, code_verifier, redirect_uri}";
    let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
        return invalid_grant(bad);
    };
    let (Some(code), Some(verifier), Some(redirect)) = (
        body["code"].as_str(),
        body["code_verifier"].as_str(),
        body["redirect_uri"].as_str(),
    ) else {
        return invalid_grant(bad);
    };
    let Some(grant) = lock(&state.grants).remove(code) else {
        return invalid_grant("no such code");
    };
    if grant.redirect != redirect
        || !verifier_ok(verifier)
        || challenge_of(verifier) != grant.challenge
    {
        return invalid_grant("the verifier or the redirect does not match");
    }
    mint(state, &grant.name, grant.stores)
}

/// `POST /v1/cli/device` `{name, stores?}`.
fn device_start(state: &State, addr: SocketAddr, request: &Request) -> Answer {
    let bad = |message: &str| Answer::refuse(400, "request", message);
    let shape = "the body is not {name, stores?}";
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(&request.body) else {
        return bad(shape);
    };
    if body.keys().any(|key| key != "name" && key != "stores") {
        return bad(shape);
    }
    let Some(name) = body.get("name").and_then(Value::as_str) else {
        return bad(shape);
    };
    if !name_ok(name) {
        return bad("a token's name is 1 to 100 characters");
    }
    let names = match body.get("stores") {
        None | Some(Value::Null) => None,
        Some(Value::Array(names)) => {
            let names: Option<Vec<String>> = names
                .iter()
                .map(|n| n.as_str().map(str::to_owned))
                .collect();
            match names {
                Some(names) if names.is_empty() => {
                    return bad("a sign-in for some stores names at least one");
                }
                Some(names) if names.iter().all(|n| store_name(n)) => Some(names),
                _ => return bad("a store name is 1 to 63 lower-case letters, digits and -"),
            }
        }
        Some(_) => return bad(shape),
    };
    let mut devices = lock(&state.devices);
    let waiting = devices
        .iter()
        .filter(|d| d.state == DeviceState::Pending)
        .count();
    if waiting >= DEVICES_WAITING {
        return Answer::refuse(
            429,
            "rate",
            "too many device sign-ins are waiting; try again later",
        );
    }
    let n = made(state);
    let letters = b"BCDFGHJKLMNPQRSTVWXZ";
    let mut user_code = String::new();
    let mut rest = n;
    for i in 0..8 {
        if i == 4 {
            user_code.insert(0, '-');
        }
        user_code.insert(0, char::from(letters[usize::try_from(rest % 20).unwrap()]));
        rest /= 20;
    }
    let code = format!("{ACCOUNT}.device{n}");
    devices.push(Device {
        code: code.clone(),
        user_code: user_code.clone(),
        name: name.to_owned(),
        names,
        stores: None,
        state: DeviceState::Pending,
        polled: None,
    });
    Answer::ok(&json!({
        "device_code": code,
        "user_code": user_code,
        "verification_uri": format!("http://{addr}/device"),
        "interval": DEVICE_INTERVAL.as_secs(),
        "expires_in": DEVICE_LIFE,
    }))
}

/// `POST /v1/cli/device/token` `{device_code}`: RFC 8628's answers, or
/// once, when the code is approved, the token answer.
fn device_token(state: &State, request: &Request) -> Answer {
    let code = serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|body| body["device_code"].as_str().map(str::to_owned));
    let Some(code) = code else {
        return invalid_grant("the body is not {device_code}");
    };
    let mut devices = lock(&state.devices);
    let Some(at) = devices.iter().position(|d| d.code == code) else {
        return invalid_grant("no such device code");
    };
    match devices[at].state {
        DeviceState::Expired => {
            devices.remove(at);
            return Answer::refuse(400, "expired_token", "the device code has expired");
        }
        DeviceState::Denied => {
            devices.remove(at);
            return Answer::refuse(400, "access_denied", "the sign-in was denied");
        }
        DeviceState::Pending | DeviceState::Approved => {}
    }
    let device = &mut devices[at];
    let early = device
        .polled
        .is_some_and(|at| at.elapsed() < DEVICE_INTERVAL);
    if device.state == DeviceState::Pending || early {
        device.polled = Some(Instant::now());
        return if early {
            Answer::refuse(400, "slow_down", "poll no more than every second")
        } else {
            Answer::refuse(
                400,
                "authorization_pending",
                "the sign-in waits for approval",
            )
        };
    }
    let device = devices.remove(at);
    drop(devices);
    mint(state, &device.name, device.stores)
}

fn rows_json(rows: &[Row]) -> Value {
    rows.iter()
        .map(|r| json!({"seq": r.seq, "author": r.author, "event": r.event, "data": r.data}))
        .collect()
}

fn case_json(key: Key, rows: &[Row]) -> Value {
    json!({"key": key.get(), "rows": rows_json(rows)})
}

fn cases_json(cases: &[(Key, Vec<Row>)]) -> Value {
    cases.iter().map(|(k, rows)| case_json(*k, rows)).collect()
}

/// The body as JSON, or the refusal for one that is not.
fn body(request: &Request) -> Result<Value, Answer> {
    if request.header("content-type") != Some("application/json") {
        return Err(Answer::refuse(400, "request", "the body is not JSON"));
    }
    if request.body.len() > BODY_LIMIT {
        return Err(Answer::refuse(400, "request", "the body is over 1 MiB"));
    }
    serde_json::from_slice(&request.body)
        .map_err(|_| Answer::refuse(400, "request", "the body is not JSON"))
}

/// The writer and the append of a write's body.
fn writer_append(body: &Value) -> Result<(Writer, Append), Answer> {
    let bad = || Answer::refuse(400, "request", "the body is not a write");
    let writer = match &body["writer"] {
        Value::Null => None,
        Value::String(name) => Some(name.as_str()),
        _ => return Err(bad()),
    };
    let field = |name: &str| {
        body["append"][name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(bad)
    };
    let append = Append {
        author: field("author")?,
        event: field("event")?,
        data: field("data")?,
    };
    Ok((Writer::new(writer), append))
}

fn revision(body: &Value) -> Result<Option<u64>, Answer> {
    match &body["revision"] {
        Value::Null => Ok(None),
        value => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| Answer::refuse(400, "request", "the revision is not one")),
    }
}

/// The answer to a write the store refused.
fn refused(error: &StoreError<DbError>) -> Answer {
    match error {
        StoreError::Refused(error) => {
            let category = error.category().as_str();
            let status = match category {
                "malformed" => 400,
                "stale" => 409,
                _ => 422,
            };
            Answer::refuse(status, category, &error.to_string())
        }
        StoreError::NoCase | StoreError::Store(DbError::Missing { .. }) => {
            Answer::refuse(404, "nocase", "no such case")
        }
        error => Answer::refuse(500, "store", &error.to_string()),
    }
}

fn failed(error: &DbError) -> Answer {
    Answer::refuse(500, "store", &error.to_string())
}

/// The case key in a path.
fn key(text: &str) -> Result<Key, Answer> {
    text.parse::<u64>()
        .ok()
        .and_then(Key::new)
        .ok_or_else(|| Answer::refuse(404, "request", "not a key"))
}

fn route(state: &State, db: &Db, request: &Request, rest: &[&str]) -> Answer {
    let result = match (request.method.as_str(), rest) {
        ("GET", ["cases"]) => list(db, request.query("since")),
        ("GET", ["cases", k]) => key(k).map(|k| match db.rows(&k) {
            Ok(Some(rows)) => Answer::ok(&case_json(k, &rows)),
            Ok(None) | Err(DbError::Missing { .. }) => {
                Answer::refuse(404, "nocase", "no such case")
            }
            Err(error) => failed(&error),
        }),
        ("POST", ["cases"]) => body(request).and_then(|b| {
            let (writer, append) = writer_append(&b)?;
            Ok(match db.create(&append, &writer) {
                Ok((k, _)) => written(db, k),
                Err(error) => refused(&error),
            })
        }),
        ("POST", ["cases", k, "events"]) => key(k).and_then(|k| {
            let b = body(request)?;
            let (writer, append) = writer_append(&b)?;
            let at = revision(&b)?;
            Ok(match db.append(&k, &append, &writer, at) {
                Ok(_) => written(db, k),
                Err(error) => refused(&error),
            })
        }),
        ("POST", ["cases", k, what @ ("archive" | "delete")]) => key(k).and_then(|k| {
            let at = revision(&body(request)?)?;
            let done = if *what == "archive" {
                db.archive(&k, at)
            } else {
                db.delete(&k, at)
            };
            Ok(match done {
                Ok(()) => Answer::ok(&json!({})),
                Err(error) => refused(&error),
            })
        }),
        ("GET", ["actors"]) => Ok(match db.actors() {
            Ok(actors) => Answer::ok(&json!({"list": actors
                .iter()
                .map(|a| json!({"id": a.id, "kind": a.kind, "name": a.name, "created": a.created}))
                .collect::<Vec<_>>()})),
            Err(DbError::Missing { .. }) => Answer::ok(&json!({"list": []})),
            Err(error) => failed(&error),
        }),
        ("GET", ["changes"]) => Ok(changes(state, db, request)),
        ("GET", ["waits"]) => Ok(match db.waits() {
            Ok(waits) => {
                Answer::ok(&json!({"list": waits.iter().map(wait_json).collect::<Vec<_>>()}))
            }
            Err(DbError::Missing { .. }) => Answer::ok(&json!({"list": []})),
            Err(error) => failed(&error),
        }),
        ("GET", ["listeners"]) => Ok(match db.listeners() {
            Ok(listeners) => Answer::ok(&json!({"list": listeners
                .iter()
                .map(|l| {
                    let mut value = json!({
                        "worker": l.worker,
                        "labels": l.labels,
                        "since": l.since,
                        "heard": l.heard,
                        "waits": l.waits.iter().map(wait_json).collect::<Vec<_>>(),
                    });
                    // Left out when the worker has none, as the server does.
                    if let Some(doing) = &l.doing {
                        value["doing"] = json!(doing);
                    }
                    if let Some(at) = &l.doing_at {
                        value["doing_at"] = json!(at);
                    }
                    value
                })
                .collect::<Vec<_>>()})),
            Err(DbError::Missing { .. }) => Answer::ok(&json!({"list": []})),
            Err(error) => failed(&error),
        }),
        ("PUT", ["doing", worker]) => body(request).map(|b| {
            let Some(text) = b["text"].as_str() else {
                return Answer::refuse(400, "request", "no text");
            };
            match db.set_doing(&decode(worker), text) {
                Ok(()) => Answer::ok(&json!({})),
                Err(DbError::NotListening { .. } | DbError::Missing { .. }) => {
                    Answer::refuse(404, "not-listening", "the worker is not listening")
                }
                Err(DbError::Doing(why)) => Answer::refuse(400, "request", &why),
                Err(error) => failed(&error),
            }
        }),
        _ => Ok(Answer::refuse(404, "request", "no such route")),
    };
    result.unwrap_or_else(|answer| answer)
}

fn wait_json(w: &cases_core::Wait) -> Value {
    json!({"id": w.id, "worker": w.worker, "labels": w.labels, "kinds": w.kinds, "since": w.since, "heard": w.heard})
}

/// The answer to a write that was stored: the case's rows now.
fn written(db: &Db, key: Key) -> Answer {
    match db.rows(&key) {
        Ok(Some(rows)) => Answer::ok(&case_json(key, &rows)),
        Ok(None) => Answer::refuse(500, "store", "the case is gone"),
        Err(error) => failed(&error),
    }
}

fn list(db: &Db, since: Option<&str>) -> Result<Answer, Answer> {
    let since = since.map(cases_core::Version::new);
    let changes = db.rows_since(since.as_ref()).map_err(|e| failed(&e))?;
    Ok(Answer::ok(&match changes {
        Changes::Unchanged { version } => {
            json!({"version": version.as_str(), "cases": [], "removed": []})
        }
        Changes::Since {
            version,
            cases,
            removed,
        } => json!({
            "version": version.as_str(),
            "cases": cases_json(&cases),
            "removed": removed.iter().map(|k| k.get()).collect::<Vec<_>>(),
        }),
        Changes::All { version, cases } => {
            json!({"version": version.as_str(), "cases": cases_json(&cases)})
        }
    }))
}

/// The held poll: the version as soon as it is not `after`, or after
/// `timeout` seconds (25 by default, at most 300).
fn changes(state: &State, db: &Db, request: &Request) -> Answer {
    let version = || db.version().map(|v| v.as_str().to_owned());
    let Some(after) = request.query("after").filter(|a| !a.is_empty()) else {
        return match version() {
            Ok(v) => Answer::ok(&json!({"version": v})),
            Err(error) => failed(&error),
        };
    };
    let secs = match request.query("timeout") {
        None => 25,
        Some(text) => match text.parse::<u64>() {
            Ok(secs) if secs <= 300 => secs,
            _ => return Answer::refuse(400, "request", "the timeout is not one"),
        },
    };
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut ticks = lock(&state.ticks);
    loop {
        let now = match version() {
            Ok(v) => v,
            Err(error) => return failed(&error),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if now != after || left.is_zero() {
            return Answer::ok(&json!({"version": now}));
        }
        let wait = left.min(Duration::from_millis(50));
        ticks = state
            .changed
            .wait_timeout(ticks, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}
