//! The far side of a forwarded store: the store routes of the agent API,
//! `/v1/stores/{id}/store/…`, served as HTTP/1.1 over one connection, such
//! as `cases dial-stdio`'s stdin and stdout, against a local store. The one
//! store is [`DIAL_STORE`]. It is what `cases-store-remote`'s test fake
//! serves, without the accounts, the tokens or the sign-in, and with the
//! local store's own errors: a store not made yet is `missing`, as a local
//! store's is, listening is recorded (`POST store/listen`), and the link of
//! the inbox a serve on the store gives is had from its instance file
//! (`GET store/inbox`).
//!
//! A write's answer is the case's rows as the write's own transaction left
//! them, so a write that was stored is never answered with an error, nor
//! with a later writer's rows. A pickup wait's locks (AG-05) are taken here,
//! on the store's own machine, by `POST store/lock`, and held until the
//! connection ends. A wait that asks for a lease has them only while it
//! renews it (`POST store/lock/renew`): a peer that stops renewing, as a
//! laptop asleep does while its ssh connection stays open, loses them once
//! the lease runs out. A wait that also says it will let them go itself
//! (`release`) has them kept past the connection's end until the lease
//! runs out, unless it let them go first (`POST store/lock/release`): a
//! connection that dies may leave that wait's pickup still on its way here
//! over another, which a second wait must not race.
//!
//! The connection is the peer's: whoever started the child could open the
//! store's file, so no token is asked for. Requests are served one at a
//! time, each answered with its length, and the connection is kept until
//! the peer closes it, or a request cannot be read.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Changes, Key, Row, Store, StoreError, Version};
use cases_store_remote::DIAL_STORE;
use cases_store_sqlite::{Db, Error as DbError, Writer};

use crate::commands::wait::lock;
use crate::instance;
use serde_json::{Value, json};

/// The most a request's head may hold, its request line and headers.
const HEAD_LIMIT: usize = 64 * 1024;

/// The most a request's body may hold, as the server has it.
const BODY_LIMIT: usize = cases_store_remote::REQUEST_LIMIT;

/// The longest hold of a held poll the far side takes, in seconds.
const HOLD_LIMIT: u64 = 300;

/// The hold of a held poll that names none, in seconds.
const HOLD_DEFAULT: u64 = 25;

/// How often a held poll reads the store's version: other processes write
/// it, so nothing here is told of a write.
const HOLD_EVERY: Duration = Duration::from_millis(200);

/// How long a pickup wait's lease on its locks lasts from its last
/// renewal: the wait renews it a third of the way in.
pub(crate) const LEASE: Duration = Duration::from_secs(30);

/// How long a pickup wait's lease on its locks lasts: [`LEASE`] or, in a
/// debug build alone, the milliseconds `CASES_TEST_LEASE` gives, so that
/// the tests see a lease run out in a second. One shorter than
/// [`cases_store_remote::LEASE_MIN`] is taken as that, the shortest a
/// client keeps. `dial-stdio` and serve's store routes both hold locks on
/// it.
pub(crate) fn lease() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("CASES_TEST_LEASE")
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
    {
        return Duration::from_millis(ms).max(cases_store_remote::LEASE_MIN);
    }
    LEASE
}

/// Serves requests read from `input` with answers written to `output`,
/// against `db`, until `input` ends or a request cannot be read. Locks
/// taken on a lease are let go once `lease` passes with no renewal.
///
/// # Errors
///
/// When `input` or `output` fails.
pub(crate) fn serve(
    db: &Db,
    lease: Duration,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    // The locks taken on this connection, let go as it ends, or as their
    // lease runs out, or, for a wait that lets them go itself, as it does
    // or their lease runs out.
    let locks = Locks::new(lease);
    thread::scope(|scope| {
        let _ending = Ending(&locks);
        scope.spawn(|| locks.watch());
        serve_on(db, &locks, input, output)
    })
}

/// Ends the watch of a connection's locks, and lets them go, when the
/// connection ends, however it ends: at once, or once their lease runs out
/// when the wait said it would let them go itself and has not.
struct Ending<'a>(&'a Locks);

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// Serves the connection, its locks in `locks`.
fn serve_on(
    db: &Db,
    locks: &Locks,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    loop {
        let request = match read(input)? {
            Read::End => return Ok(()),
            Read::Bad(why) => {
                // What follows a request that cannot be read cannot be
                // either: answer, and close.
                write(output, &Answer::refuse(400, "request", why))?;
                return Ok(());
            }
            Read::Request(request) => request,
        };
        let answer = skew(handle(db, &request, locks), request.agent.as_deref());
        write(output, &answer)?;
        if request.close {
            return Ok(());
        }
    }
}

/// The locks a connection has taken, and the lease they are on, if any,
/// shared with the watch that lets them go when it runs out.
#[derive(Debug)]
struct Locks {
    taken: Mutex<Taken>,
    /// Told when the lease is renewed or the connection ends.
    changed: Condvar,
    /// How long a lease lasts from its last renewal.
    lease: Duration,
}

/// What a connection holds.
#[derive(Debug, Default)]
struct Taken {
    held: Vec<lock::Held>,
    /// When the lease runs out unless it is renewed, while there is one.
    until: Option<Instant>,
    /// Set once a lease ran out and its locks were let go.
    lapsed: bool,
    /// Set while the wait has said it lets its locks go itself (`release`
    /// on a leased lock): its connection's end does not let them go before
    /// the lease runs out.
    releases: bool,
    /// Set as the connection ends, so that the watch stops.
    ended: bool,
}

impl Locks {
    fn new(lease: Duration) -> Locks {
        Locks {
            taken: Mutex::new(Taken::default()),
            changed: Condvar::new(),
            lease,
        }
    }

    fn taken(&self) -> MutexGuard<'_, Taken> {
        self.taken.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets the locks go once the lease runs out with no renewal, until
    /// the connection ends.
    fn watch(&self) {
        let mut taken = self.taken();
        while !taken.ended {
            taken = match taken.until {
                None => self
                    .changed
                    .wait(taken)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        taken.held.clear();
                        taken.until = None;
                        taken.lapsed = true;
                        continue;
                    }
                    self.changed
                        .wait_timeout(taken, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
    }

    /// The connection has ended: its locks go, and the watch stops. Leased
    /// locks whose wait said it lets them go itself, and has not, are kept
    /// until the lease runs out: the connection may have died with that
    /// wait's pickup still on its way over another.
    fn end(&self) {
        let mut taken = self.taken();
        while taken.releases
            && let Some(until) = taken.until
        {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            taken = self
                .changed
                .wait_timeout(taken, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        taken.ended = true;
        taken.held.clear();
        taken.until = None;
        self.changed.notify_all();
    }

    /// Waits until the locks are let go: released, or their lease run out
    /// with no renewal, which lets them go. For locks held by id over HTTP,
    /// which no connection's end lets go.
    fn until_let_go(&self) {
        let mut taken = self.taken();
        while let Some(until) = taken.until {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                taken.held.clear();
                taken.until = None;
                taken.lapsed = true;
                return;
            }
            taken = self
                .changed
                .wait_timeout(taken, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        taken.held.clear();
    }

    /// The answer that gives the lease's length.
    fn leased(&self) -> Answer {
        let ms = u64::try_from(self.lease.as_millis()).unwrap_or(u64::MAX);
        Answer::ok(json!({ "lease_ms": ms }))
    }
}

/// A request, as read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    content_type: Option<String>,
    body: Vec<u8>,
    /// Whether the peer asked for the connection to close after it.
    close: bool,
    /// Its `User-Agent`, such as `cases/0.1.0`.
    agent: Option<String>,
}

impl Request {
    /// A request that arrived some other way than over a pipe, such as to
    /// `cases serve`: its method, its path and its query, undecoded, its
    /// `Content-Type` and `User-Agent`, and its body.
    pub(crate) fn new(
        method: &str,
        path: &str,
        query: &str,
        content_type: Option<&str>,
        agent: Option<&str>,
        body: Vec<u8>,
    ) -> Request {
        Request {
            method: method.to_owned(),
            path: path.to_owned(),
            query: pairs(query),
            content_type: content_type.map(str::to_owned),
            body,
            close: false,
            agent: agent.map(str::to_owned),
        }
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What reading a request gave.
#[derive(Debug, PartialEq, Eq)]
enum Read {
    Request(Request),
    /// The input ended before a request began, or part-way through one.
    End,
    /// A request that cannot be read, and why.
    Bad(&'static str),
}

/// Reads one request.
fn read(input: &mut impl BufRead) -> io::Result<Read> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let left = HEAD_LIMIT.saturating_sub(head.len()) + 1;
        let n = io::Read::take(&mut *input, u64::try_from(left).unwrap_or(u64::MAX))
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(Read::End);
        }
        if !line.ends_with(b"\n") {
            if head.len() + line.len() > HEAD_LIMIT {
                return Ok(Read::Bad("the request's head is over 64 KiB"));
            }
            return Ok(Read::End);
        }
        // A blank line before the request line is skipped (RFC 9112, 2.2).
        if head.is_empty() && (line == b"\r\n" || line == b"\n") {
            continue;
        }
        let end = line == b"\r\n" || line == b"\n";
        head.extend_from_slice(&line);
        if end {
            break;
        }
    }
    let Ok(head) = String::from_utf8(head) else {
        return Ok(Read::Bad("the request's head is not UTF-8"));
    };
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (first.next(), first.next(), first.next(), first.next())
    else {
        return Ok(Read::Bad("the request line is not one"));
    };
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Ok(Read::Bad("the request line is not one"));
    }
    let mut length = 0;
    let mut content_type = None;
    let mut close = version == "HTTP/1.0";
    let mut agent = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Ok(Read::Bad("a header is not one"));
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            match value.parse::<usize>() {
                Ok(n) if n <= BODY_LIMIT => length = n,
                Ok(_) => return Ok(Read::Bad("the body is over 1 MiB")),
                Err(_) => return Ok(Read::Bad("the body's length is not one")),
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Ok(Read::Bad("a body is sent with its length"));
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("user-agent") {
            agent = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("connection") {
            close = value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"));
        }
    }
    let mut body = vec![0; length];
    if let Err(error) = input.read_exact(&mut body) {
        return match error.kind() {
            io::ErrorKind::UnexpectedEof => Ok(Read::End),
            _ => Err(error),
        };
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Ok(Read::Request(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: pairs(query),
        content_type,
        body,
        close,
        agent,
    }))
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

/// Percent-decodes `text`. A `%` not followed by two hex digits is kept.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
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

/// An answer: a status and its JSON body.
#[derive(Debug, PartialEq)]
pub(crate) struct Answer {
    status: u16,
    body: Value,
}

impl Answer {
    /// Its status.
    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    /// Its body.
    pub(crate) fn body(&self) -> &Value {
        &self.body
    }

    fn ok(body: Value) -> Answer {
        Answer { status: 200, body }
    }

    fn refuse(status: u16, category: &str, message: &str) -> Answer {
        Answer {
            status,
            body: json!({"error": {"category": category, "message": message}}),
        }
    }

    /// The answer for an error of the store's own: `missing` for a store
    /// not made yet, 503 for one whose lock another writer held too long,
    /// 500 `newer` for one made by a newer cases than this one, so that the
    /// near side can end as it does on a local store (TU-01), and 500 for
    /// the rest. The newer store's message names this side, the far side,
    /// as the one to update.
    fn failed(error: &DbError) -> Answer {
        match error {
            DbError::Missing { .. } => Answer::refuse(404, "missing", "the store is not made yet"),
            DbError::Newer { path, version } => Answer::refuse(
                500,
                "newer",
                &format!(
                    "store {} has schema version {version}, newer than the cases on the far \
                     side reads: update cases there",
                    path.display()
                ),
            ),
            DbError::Busy { .. } => Answer::refuse(503, "store", &error.to_string()),
            error => Answer::refuse(500, "store", &error.to_string()),
        }
    }
}

/// `answer`, with the versions on each side named when it is `unknown`: an
/// event, kind or urgency the cases here does not know, which a newer cases
/// at the other end can send.
fn skew(mut answer: Answer, agent: Option<&str>) -> Answer {
    let Some(error) = answer
        .body
        .get_mut("error")
        .filter(|error| error["category"] == "unknown")
    else {
        return answer;
    };
    let here = env!("CARGO_PKG_VERSION");
    let message = error["message"].as_str().unwrap_or_default();
    let message = match agent.and_then(|a| a.strip_prefix("cases/")) {
        Some(there) if there != here => format!(
            "{message}: the cases on the far side is {here}, and this one {there}; update the older"
        ),
        _ => format!("{message}: the cases on the far side is {here}"),
    };
    error["message"] = Value::String(message);
    answer
}

/// Writes `answer`, with its length.
fn write(output: &mut impl Write, answer: &Answer) -> io::Result<()> {
    let body = answer.body.to_string();
    let reason = match answer.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Content",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let mut head = String::new();
    let _ = write!(
        head,
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        answer.status,
        body.len()
    );
    output.write_all(head.as_bytes())?;
    output.write_all(body.as_bytes())?;
    output.flush()
}

/// The answer to `request`: the store's route, or a refusal for a path
/// that is not one of the store routes of [`DIAL_STORE`]. Locks it takes
/// go in `locks`.
fn handle(db: &Db, request: &Request, locks: &Locks) -> Answer {
    handle_with(db, request, Holds::Connection(locks))
}

/// The answer to `request`, sent over HTTP to `cases serve`, its pickup
/// wait's locks held in `leases` by id (see [`Leases`]), with the versions
/// named on an `unknown` refusal, as over a pipe.
pub(crate) fn handle_http(db: &Db, request: &Request, leases: &Leases) -> Answer {
    skew(
        handle_with(db, request, Holds::Leases(leases)),
        request.agent.as_deref(),
    )
}

/// Where a request's pickup wait's locks are held.
#[derive(Clone, Copy)]
enum Holds<'a> {
    /// On the connection, over a pipe.
    Connection(&'a Locks),
    /// By id, on a lease, over HTTP.
    Leases(&'a Leases),
}

fn handle_with(db: &Db, request: &Request, holds: Holds<'_>) -> Answer {
    let parts: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["v1", "stores", id, "store", rest @ ..] if *id == DIAL_STORE => {
            route(db, request, rest, holds)
        }
        ["v1", "stores", _, "store", ..] => Answer::refuse(404, "request", "no such store"),
        _ => Answer::refuse(404, "request", "no such route"),
    }
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
    if request.content_type.as_deref() != Some("application/json") {
        return Err(Answer::refuse(400, "request", "the body is not JSON"));
    }
    serde_json::from_slice(&request.body)
        .map_err(|_| Answer::refuse(400, "request", "the body is not JSON"))
}

/// A string field of `body`, or the refusal for one that is not there.
fn text<'a>(body: &'a Value, name: &str) -> Result<&'a str, Answer> {
    body[name]
        .as_str()
        .ok_or_else(|| Answer::refuse(400, "request", "the body is not a write"))
}

/// A list of strings in `body`, empty when it is not there.
fn texts(body: &Value, name: &str) -> Result<Vec<String>, Answer> {
    match &body[name] {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or_else(|| Answer::refuse(400, "request", "a list is not of strings")),
        _ => Err(Answer::refuse(400, "request", "a list is not of strings")),
    }
}

/// The writer and the append of a write's body.
fn writer_append(body: &Value) -> Result<(Writer, cases_core::Append), Answer> {
    let writer = match &body["writer"] {
        Value::Null => None,
        Value::String(name) => Some(name.as_str()),
        _ => return Err(Answer::refuse(400, "request", "the body is not a write")),
    };
    let append = &body["append"];
    let append = cases_core::Append {
        author: text(append, "author")?.to_owned(),
        event: text(append, "event")?.to_owned(),
        data: text(append, "data")?.to_owned(),
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
        StoreError::NoCase => Answer::refuse(404, "nocase", "no such case"),
        StoreError::Store(error) => Answer::failed(error),
        error => Answer::refuse(500, "store", &error.to_string()),
    }
}

/// The case key in a path.
fn key(text: &str) -> Result<Key, Answer> {
    text.parse::<u64>()
        .ok()
        .and_then(Key::new)
        .ok_or_else(|| Answer::refuse(404, "request", "not a key"))
}

/// The answer to a store route, `rest` being the path after `store/`.
fn route(db: &Db, request: &Request, rest: &[&str], holds: Holds<'_>) -> Answer {
    let result = match (request.method.as_str(), rest) {
        ("GET", ["cases"]) => Ok(list(db, request.query("since"))),
        ("GET", ["cases", k]) => key(k).map(|k| match db.rows(&k) {
            Ok(Some(rows)) => Answer::ok(case_json(k, &rows)),
            Ok(None) => Answer::refuse(404, "nocase", "no such case"),
            Err(error) => Answer::failed(&error),
        }),
        ("POST", ["cases"]) => body(request).and_then(|b| {
            let (writer, append) = writer_append(&b)?;
            Ok(match db.create_rows(&append, &writer) {
                Ok((k, rows)) => Answer::ok(case_json(k, &rows)),
                Err(error) => refused(&error),
            })
        }),
        ("POST", ["cases", k, "events"]) => key(k).and_then(|k| {
            let b = body(request)?;
            let (writer, append) = writer_append(&b)?;
            let at = revision(&b)?;
            Ok(match db.append_rows(&k, &append, &writer, at) {
                Ok(rows) => Answer::ok(case_json(k, &rows)),
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
                Ok(()) => Answer::ok(json!({})),
                Err(error) => refused(&error),
            })
        }),
        ("GET", ["actors"]) => Ok(match db.actors() {
            Ok(actors) => Answer::ok(json!({"list": actors
                .iter()
                .map(|a| json!({"id": a.id, "kind": a.kind, "name": a.name, "created": a.created}))
                .collect::<Vec<_>>()})),
            Err(error) => Answer::failed(&error),
        }),
        ("GET", ["changes"]) => Ok(changes(db, request)),
        ("GET", ["waits"]) => Ok(match db.waits() {
            Ok(waits) => {
                Answer::ok(json!({"list": waits.iter().map(wait_json).collect::<Vec<_>>()}))
            }
            Err(error) => Answer::failed(&error),
        }),
        ("GET", ["listeners"]) => Ok(match db.listeners() {
            Ok(listeners) => Answer::ok(json!({"list": listeners
                .iter()
                .map(listener_json)
                .collect::<Vec<_>>()})),
            Err(error) => Answer::failed(&error),
        }),
        ("GET", ["inbox"]) => Ok(inbox(db)),
        ("POST", ["listen"]) => body(request).and_then(|b| listen(db, &b)),
        ("POST", ["lock"]) => body(request).and_then(|b| match holds {
            Holds::Connection(locks) => take_locks(db, &b, locks),
            Holds::Leases(leases) => leases.take(db, &b),
        }),
        ("POST", ["lock", "renew"]) => Ok(match holds {
            Holds::Connection(locks) => renew(locks),
            Holds::Leases(_) => no_connection(),
        }),
        ("POST", ["lock", "release"]) => Ok(match holds {
            Holds::Connection(locks) => release(locks),
            Holds::Leases(_) => no_connection(),
        }),
        ("POST", ["lock", id, "renew"]) => Ok(match holds {
            Holds::Leases(leases) => leases.renew(id),
            Holds::Connection(_) => Answer::refuse(404, "request", "no such route"),
        }),
        ("POST", ["lock", id, "release"]) => Ok(match holds {
            Holds::Leases(leases) => leases.release(id),
            Holds::Connection(_) => Answer::refuse(404, "request", "no such route"),
        }),
        ("PUT", ["doing", worker]) => body(request).and_then(|b| {
            let text = text(&b, "text")?;
            Ok(match db.set_doing(&decode(worker), text) {
                Ok(()) => Answer::ok(json!({})),
                Err(DbError::NotListening { .. }) => {
                    Answer::refuse(404, "not-listening", "the worker is not listening")
                }
                Err(DbError::Doing(why)) => Answer::refuse(400, "request", &why),
                Err(error) => Answer::failed(&error),
            })
        }),
        _ => Ok(Answer::refuse(404, "request", "no such route")),
    };
    result.unwrap_or_else(|answer| answer)
}

/// `GET store/inbox`: `{link}`, the link of the inbox a serve on this
/// store gives, an allowed host's (IN-11), or `null` when no serve is
/// running here or the one running has none: its loopback address is no
/// use on the peer's machine.
fn inbox(db: &Db) -> Answer {
    match instance::running(db.path()) {
        Ok(Some(info)) if !info.link.is_empty() => Answer::ok(json!({ "link": info.link })),
        Ok(_) => Answer::ok(json!({ "link": null })),
        Err(error) => Answer::refuse(500, "store", &error.to_string()),
    }
}

/// `POST store/listen` `{wait, worker, labels, kinds, doing}`.
fn listen(db: &Db, body: &Value) -> Result<Answer, Answer> {
    let wait = text(body, "wait")?;
    let worker = text(body, "worker")?;
    let labels = texts(body, "labels")?;
    let kinds = texts(body, "kinds")?;
    let doing = match &body["doing"] {
        Value::Null => None,
        Value::String(doing) => Some(doing.as_str()),
        _ => return Err(Answer::refuse(400, "request", "the doing line is not text")),
    };
    Ok(match db.listen(wait, worker, &labels, &kinds, doing) {
        Ok(()) => Answer::ok(json!({})),
        Err(DbError::Unnamed) => Answer::refuse(400, "request", "a worker has a name"),
        Err(DbError::Doing(why)) => Answer::refuse(400, "request", &why),
        Err(error) => Answer::failed(&error),
    })
}

fn wait_json(w: &cases_core::Wait) -> Value {
    json!({"id": w.id, "worker": w.worker, "labels": w.labels, "kinds": w.kinds, "since": w.since, "heard": w.heard})
}

fn listener_json(l: &cases_core::Listening) -> Value {
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
}

/// `POST store/lock` `{locks: [{on, text}], lease, release}`: takes a
/// pickup wait's locks on each worker, label or id (`on`), as `cases wait
/// --pickup` takes them on a local store, and keeps them in `locks` until
/// the connection ends. With `lease` true, they are kept only while the
/// lease is renewed, and the answer gives its length, `{lease_ms}`. With
/// `release` true as well, the wait says it lets them go itself (`POST
/// store/lock/release`), and they are kept past the connection's end until
/// the lease runs out unless it has. A store not made yet
/// takes none, as a local one does not. One that another wait holds is
/// refused, 409 `held`, naming that wait.
fn take_locks(db: &Db, body: &Value, locks: &Locks) -> Result<Answer, Answer> {
    let not_locks = || Answer::refuse(400, "request", "the body is not a list of locks");
    let scopes = body["locks"]
        .as_array()
        .ok_or_else(not_locks)?
        .iter()
        .map(|item| {
            let on = item["on"]
                .as_str()
                .and_then(lock::what)
                .ok_or_else(not_locks)?;
            let text = item["text"].as_str().ok_or_else(not_locks)?;
            Ok((on, text.to_owned()))
        })
        .collect::<Result<Vec<lock::Scope>, Answer>>()?;
    let flag = |name: &str| match &body[name] {
        Value::Null => Ok(false),
        Value::Bool(flag) => Ok(*flag),
        _ => Err(Answer::refuse(
            400,
            "request",
            &format!("the {name} is not true or false"),
        )),
    };
    let lease = flag("lease")?;
    let releases = flag("release")?;
    let answer = |taken: &mut Taken| {
        if !lease {
            return Answer::ok(json!({}));
        }
        taken.until = Some(Instant::now() + locks.lease);
        taken.lapsed = false;
        taken.releases = releases;
        locks.changed.notify_all();
        locks.leased()
    };
    if !db.path().exists() {
        return Ok(answer(&mut locks.taken()));
    }
    Ok(match lock::take(db.path(), &scopes) {
        Ok(held) => {
            let mut taken = locks.taken();
            taken.held.push(held);
            answer(&mut taken)
        }
        Err(lock::Refusal::Held(error)) => Answer::refuse(409, "held", &error.to_string()),
        Err(lock::Refusal::Failed(error)) => Answer::refuse(500, "store", &error.to_string()),
    })
}

/// `POST store/lock/renew`: renews the lease of the locks this connection
/// took, for as long again, and gives its length. One that has run out,
/// its locks let go, or that there is none of, is refused, 409 `lapsed`.
fn renew(locks: &Locks) -> Answer {
    let mut taken = locks.taken();
    if taken.until.is_none() {
        let why = if taken.lapsed {
            "the lease ran out before it was renewed, and its locks were let go"
        } else {
            "this connection holds no lease"
        };
        return Answer::refuse(409, "lapsed", why);
    }
    taken.until = Some(Instant::now() + locks.lease);
    locks.changed.notify_all();
    drop(taken);
    locks.leased()
}

/// `POST store/lock/release`: lets go of the locks this connection took,
/// at once, as a wait that has ended does, so that its connection's end
/// does not keep them until their lease runs out.
fn release(locks: &Locks) -> Answer {
    let mut taken = locks.taken();
    taken.held.clear();
    taken.until = None;
    taken.releases = false;
    locks.changed.notify_all();
    Answer::ok(json!({}))
}

/// The refusal of a renewal or release of a connection's locks over HTTP,
/// where they are held by id.
fn no_connection() -> Answer {
    Answer::refuse(
        409,
        "lapsed",
        "over HTTP, locks are held by the id their lock gave: renew them at lock/<id>/renew",
    )
}

/// The pickup waits' locks taken over HTTP, through `cases serve`, each by
/// the id its `POST store/lock` gave. A proxy between, such as `tailscale
/// serve`, may send each request on another connection, so none is held
/// while a connection lasts: each is held on a lease, which its wait
/// renews (`POST store/lock/<id>/renew`), and let go once it runs out with
/// no renewal, or once the wait lets go (`POST store/lock/<id>/release`).
#[derive(Debug)]
pub(crate) struct Leases {
    lease: Duration,
    held: Arc<Mutex<HashMap<String, Arc<Locks>>>>,
}

impl Leases {
    /// None held yet, each to be held on a lease of `lease`.
    pub(crate) fn new(lease: Duration) -> Leases {
        Leases {
            lease,
            held: Arc::default(),
        }
    }

    fn held(&self) -> MutexGuard<'_, HashMap<String, Arc<Locks>>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `POST store/lock` over HTTP: as over a pipe, but on a lease the wait
    /// lets go itself alone (`lease` and `release` true), and the answer
    /// gives the hold's id as well, `{lease_ms, hold}`.
    fn take(&self, db: &Db, body: &Value) -> Result<Answer, Answer> {
        if body["lease"] != Value::Bool(true) || body["release"] != Value::Bool(true) {
            return Err(Answer::refuse(
                400,
                "request",
                "over HTTP, locks are taken on a lease the wait lets go itself: send lease and \
                 release true",
            ));
        }
        let locks = Arc::new(Locks::new(self.lease));
        let mut answer = take_locks(db, body, &locks)?;
        if answer.status != 200 {
            return Ok(answer);
        }
        let id = match hold_id() {
            Ok(id) => id,
            Err(error) => return Ok(Answer::refuse(500, "store", &error)),
        };
        answer.body["hold"] = json!(id);
        self.held().insert(id.clone(), Arc::clone(&locks));
        let held = Arc::clone(&self.held);
        thread::spawn(move || {
            locks.until_let_go();
            held.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        });
        Ok(answer)
    }

    /// `POST store/lock/<id>/renew`: renews that hold's lease, as over a
    /// pipe. One let go, run out, or never taken is refused, 409 `lapsed`.
    fn renew(&self, id: &str) -> Answer {
        let locks = self.held().get(id).cloned();
        match locks {
            Some(locks) => renew(&locks),
            None => Answer::refuse(
                409,
                "lapsed",
                "no locks are held by this id: their lease ran out, or they were let go",
            ),
        }
    }

    /// `POST store/lock/<id>/release`: lets go of that hold's locks at
    /// once. One already let go is let go again, as nothing.
    fn release(&self, id: &str) -> Answer {
        let locks = self.held().remove(id);
        match locks {
            Some(locks) => release(&locks),
            None => Answer::ok(json!({})),
        }
    }
}

/// A new hold's id: 32 hex digits of the system's random bytes, which no
/// other wait can guess.
fn hold_id() -> Result<String, String> {
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|error| format!("no random bytes: {error}"))?;
    Ok(random.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// `GET store/cases`, every case or what changed `since` a version.
fn list(db: &Db, since: Option<&str>) -> Answer {
    let since = since.map(Version::new);
    match db.rows_since(since.as_ref()) {
        Ok(Changes::Unchanged { version }) => {
            Answer::ok(json!({"version": version.as_str(), "cases": [], "removed": []}))
        }
        Ok(Changes::Since {
            version,
            cases,
            removed,
        }) => Answer::ok(json!({
            "version": version.as_str(),
            "cases": cases_json(&cases),
            "removed": removed.iter().map(|k| k.get()).collect::<Vec<_>>(),
        })),
        Ok(Changes::All { version, cases }) => {
            Answer::ok(json!({"version": version.as_str(), "cases": cases_json(&cases)}))
        }
        Err(error) => Answer::failed(&error),
    }
}

/// The held poll, `GET store/changes?after=V&timeout=S`: the version as
/// soon as it is not `after`, or after `timeout` seconds (25 by default,
/// at most 300). With no `after`, the version at once.
fn changes(db: &Db, request: &Request) -> Answer {
    let version = || db.version().map(|v| v.as_str().to_owned());
    let Some(after) = request.query("after").filter(|a| !a.is_empty()) else {
        return match version() {
            Ok(v) => Answer::ok(json!({"version": v})),
            Err(error) => Answer::failed(&error),
        };
    };
    let secs = match request.query("timeout") {
        None => HOLD_DEFAULT,
        Some(text) => match text.parse::<u64>() {
            Ok(secs) if secs <= HOLD_LIMIT => secs,
            _ => return Answer::refuse(400, "request", "the timeout is not one"),
        },
    };
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let now = match version() {
            Ok(v) => v,
            Err(error) => return Answer::failed(&error),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if now != after || left.is_zero() {
            return Answer::ok(json!({"version": now}));
        }
        thread::sleep(left.min(HOLD_EVERY));
    }
}

/// In a debug build alone, the store with its clock read from the file
/// `CASES_TEST_CLOCK` names, when it is set, so that the tests can run the
/// vectors through `dial-stdio`, and serve's store routes, at the vectors' times. A file that does not
/// hold a time gives the system's. A release build has no such clock.
#[cfg(debug_assertions)]
pub(crate) fn test_clock(db: &cases_store_sqlite::Db) -> cases_store_sqlite::Db {
    use std::time::{SystemTime, UNIX_EPOCH};

    use cases_core::Time;

    let db = cases_store_sqlite::Db::new(db.path());
    let Some(path) = std::env::var_os("CASES_TEST_CLOCK") else {
        return db;
    };
    db.with_clock(move || {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse::<Time>().ok())
            .unwrap_or_else(|| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                Time::from_unix(
                    i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
                    now.subsec_nanos(),
                )
                .unwrap_or_else(|| unreachable!("the system's clock is in range"))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> Read {
        read(&mut text.as_bytes()).unwrap()
    }

    #[test]
    fn a_request_is_read_with_its_body_and_query() {
        let Read::Request(got) = request(
            "\r\nPOST /v1/x?since=a%20b&n HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\ncontent-length: 2\r\n\r\n{}",
        ) else {
            panic!("not read");
        };
        assert_eq!(
            got,
            Request {
                method: "POST".into(),
                path: "/v1/x".into(),
                query: vec![("since".into(), "a b".into()), ("n".into(), String::new())],
                content_type: Some("application/json".into()),
                body: b"{}".to_vec(),
                close: false,
                agent: None,
            }
        );
        let Read::Request(got) = request("GET / HTTP/1.1\nConnection: keep-alive, Close\n\n")
        else {
            panic!("not read");
        };
        assert!(got.close);
        let Read::Request(got) = request("GET / HTTP/1.0\r\n\r\n") else {
            panic!("not read");
        };
        assert!(got.close);
    }

    #[test]
    fn two_requests_are_read_in_turn() {
        let mut input = "GET /a HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n".as_bytes();
        for path in ["/a", "/b"] {
            match read(&mut input).unwrap() {
                Read::Request(r) => assert_eq!(r.path, path),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(read(&mut input).unwrap(), Read::End);
    }

    #[test]
    fn an_end_part_way_is_the_end() {
        for text in [
            "",
            "GET / HTTP/1.1",
            "GET / HTTP/1.1\r\nHost: x\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n{}",
        ] {
            assert_eq!(request(text), Read::End, "{text:?}");
        }
    }

    #[test]
    fn what_is_not_a_request_is_bad() {
        let big = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(HEAD_LIMIT));
        let long_line = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(HEAD_LIMIT + 1));
        let over = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            BODY_LIMIT + 1
        );
        for text in [
            "GET\r\n\r\n",
            "GET / HTTP/1.1 x\r\n\r\n",
            "GET x HTTP/1.1\r\n\r\n",
            " / HTTP/1.1\r\n\r\n",
            "GET / HTTP/2\r\n\r\n",
            "GET / HTTP/1.1\r\nno colon\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: two\r\n\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            &over,
            &big,
            &long_line,
        ] {
            assert!(
                matches!(request(text), Read::Bad(_)),
                "{}",
                &text[..text.len().min(60)]
            );
        }
        assert!(matches!(
            read(&mut &b"GET /\xff HTTP/1.1\r\n\r\n"[..]).unwrap(),
            Read::Bad(_)
        ));
        // A body of exactly the limit is taken.
        let at = format!(
            "POST / HTTP/1.1\r\nContent-Length: {BODY_LIMIT}\r\n\r\n{}",
            " ".repeat(BODY_LIMIT)
        );
        assert!(matches!(request(&at), Read::Request(_)));
        // A head of exactly the limit is taken.
        let line = "GET / HTTP/1.1\r\n";
        let pad = HEAD_LIMIT - line.len() - "X: \r\n".len() - 2;
        let at = format!("{line}X: {}\r\n\r\n", "a".repeat(pad));
        assert_eq!(at.len(), HEAD_LIMIT);
        assert!(matches!(request(&at), Read::Request(_)));
    }

    #[test]
    fn percent_decoding_keeps_what_is_not_an_escape() {
        assert_eq!(decode("a%20b%2fc"), "a b/c");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%C3%A9"), "é");
    }

    /// The answer `serve` gives to `input`, as text.
    fn served(db: &Db, input: &str) -> String {
        let mut out = Vec::new();
        serve(db, LEASE, &mut input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn each_answer_has_its_length_and_a_bad_request_ends_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let path = format!("/v1/stores/{DIAL_STORE}/store/changes");
        let got = served(
            &db,
            &format!(
                "GET {path} HTTP/1.1\r\n\r\nGET {path} HTTP/1.1\r\n\r\nBAD\r\n\r\nGET {path} HTTP/1.1\r\n\r\n"
            ),
        );
        let one = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n{\"version\":\"0\"}";
        let bad = r#"{"error":{"category":"request","message":"the request line is not one"}}"#;
        assert_eq!(
            got,
            format!(
                "{one}{one}HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{bad}",
                bad.len()
            )
        );
        // Connection: close ends it after the answer.
        let got = served(
            &db,
            &format!("GET {path} HTTP/1.1\r\nConnection: close\r\n\r\nGET {path} HTTP/1.1\r\n\r\n"),
        );
        assert_eq!(got, one);
    }

    #[test]
    fn only_the_dial_store_s_routes_are_served() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let get = |path: &str| {
            let Read::Request(request) = request(&format!("GET {path} HTTP/1.1\r\n\r\n")) else {
                panic!("not read");
            };
            handle(&db, &request, &Locks::new(LEASE))
        };
        let category = |answer: &Answer| answer.body["error"]["category"].clone();
        let other = get("/v1/stores/st_0000000000000001/store/changes");
        assert_eq!((other.status, category(&other)), (404, json!("request")));
        let other = get("/v1/cli/whoami");
        assert_eq!((other.status, category(&other)), (404, json!("request")));
        let other = get(&format!("/v1/stores/{DIAL_STORE}/store/nothing"));
        assert_eq!((other.status, category(&other)), (404, json!("request")));
        // A store not made yet is missing, as a local one is, where the
        // local store says so; its cases are none.
        let missing = get(&format!("/v1/stores/{DIAL_STORE}/store/actors"));
        assert_eq!(
            (missing.status, category(&missing)),
            (404, json!("missing"))
        );
        let none = get(&format!("/v1/stores/{DIAL_STORE}/store/cases"));
        assert_eq!(none, Answer::ok(json!({"version": "0", "cases": []})));
        let bad = get(&format!(
            "/v1/stores/{DIAL_STORE}/store/changes?after=1&timeout=301"
        ));
        assert_eq!((bad.status, category(&bad)), (400, json!("request")));
        let at_once = get(&format!(
            "/v1/stores/{DIAL_STORE}/store/changes?after=1&timeout=300"
        ));
        assert_eq!(at_once, Answer::ok(json!({"version": "0"})));
    }

    /// `POST store/lock` with `body`, its locks kept in `locks`.
    fn lock_with(db: &Db, body: &str, locks: &Locks) -> Answer {
        post(db, "lock", body, locks)
    }

    /// [`lock_with`], for a test that has just let go of the locks in this
    /// process: a refusal, `held`, is tried again for up to 2 s, as one comes
    /// for a moment while a child another test starts has the lock's file
    /// open (see [`lock::take_freed`]).
    fn lock_freed(db: &Db, body: &str, locks: &Locks) -> Answer {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let answer = lock_with(db, body, locks);
            if answer.status != 409 || Instant::now() >= until {
                return answer;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// `POST store/<route>` with `body`, on the connection of `locks`.
    fn post(db: &Db, route: &str, body: &str, locks: &Locks) -> Answer {
        let Read::Request(request) = request(&format!(
            "POST /v1/stores/{DIAL_STORE}/store/{route} HTTP/1.1\r\nContent-Type: \
             application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )) else {
            panic!("not read");
        };
        handle(db, &request, locks)
    }

    fn held_count(locks: &Locks) -> usize {
        locks.taken().held.len()
    }

    /// The category of a refusal.
    fn refusal(answer: &Answer) -> Value {
        answer.body["error"]["category"].clone()
    }

    /// `POST store/<route>` with `body` over HTTP, its locks held by id
    /// in `leases`.
    fn post_http(db: &Db, route: &str, body: &str, leases: &Leases) -> Answer {
        let path = format!("/v1/stores/{DIAL_STORE}/store/{route}");
        let request = Request::new(
            "POST",
            &path,
            "",
            Some("application/json"),
            Some("cases/0.0.0"),
            body.as_bytes().to_vec(),
        );
        handle_http(db, &request, leases)
    }

    #[test]
    fn over_http_locks_are_held_by_id_on_a_lease() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        db.create(
            &cases_core::Append {
                author: "agent".into(),
                event: "open".into(),
                data: r#"{"kind":"fyi","urgency":"today","title":"t"}"#.into(),
            },
            &Writer::new(None),
        )
        .unwrap();
        let leases = Leases::new(Duration::from_millis(300));
        let deploy =
            r#"{"locks": [{"on": "label", "text": "deploy"}], "lease": true, "release": true}"#;
        // Only on a lease the wait lets go itself: no connection holds them.
        for body in [
            r#"{"locks": [{"on": "label", "text": "deploy"}]}"#,
            r#"{"locks": [{"on": "label", "text": "deploy"}], "lease": true}"#,
        ] {
            let refused = post_http(&db, "lock", body, &leases);
            assert_eq!(refused.status, 400, "{body}: {refused:?}");
        }
        let taken = post_http(&db, "lock", deploy, &leases);
        assert_eq!(taken.status, 200, "{taken:?}");
        assert_eq!(taken.body["lease_ms"], 300);
        let id = taken.body["hold"].as_str().unwrap().to_owned();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        // Another wait is refused while it is held.
        let second = post_http(&db, "lock", deploy, &leases);
        assert_eq!((second.status, refusal(&second)), (409, json!("held")));
        // Renewed by its id, it outlives the lease.
        for _ in 0..4 {
            thread::sleep(Duration::from_millis(100));
            let renewed = post_http(&db, &format!("lock/{id}/renew"), "", &leases);
            assert_eq!(renewed, Answer::ok(json!({"lease_ms": 300})));
        }
        // The connection's routes are not there over HTTP.
        let renew = post_http(&db, "lock/renew", "", &leases);
        assert_eq!((renew.status, refusal(&renew)), (409, json!("lapsed")));
        let unknown = post_http(&db, "lock/0000/renew", "", &leases);
        assert_eq!((unknown.status, refusal(&unknown)), (409, json!("lapsed")));
        // Let go by its id, at once, and again as nothing.
        let released = post_http(&db, &format!("lock/{id}/release"), "", &leases);
        assert_eq!(released, Answer::ok(json!({})));
        let again = post_http(&db, &format!("lock/{id}/release"), "", &leases);
        assert_eq!(again, Answer::ok(json!({})));
        let gone = post_http(&db, &format!("lock/{id}/renew"), "", &leases);
        assert_eq!((gone.status, refusal(&gone)), (409, json!("lapsed")));
        // One left alone lapses once its lease runs out, and is let go.
        let until = Instant::now() + Duration::from_secs(2);
        let id = loop {
            let taken = post_http(&db, "lock", deploy, &leases);
            if taken.status == 200 {
                break taken.body["hold"].as_str().unwrap().to_owned();
            }
            assert!(Instant::now() < until, "{taken:?}");
            thread::sleep(Duration::from_millis(10));
        };
        thread::sleep(Duration::from_millis(600));
        let lapsed = post_http(&db, &format!("lock/{id}/renew"), "", &leases);
        assert_eq!((lapsed.status, refusal(&lapsed)), (409, json!("lapsed")));
        assert!(leases.held().is_empty());
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let taken = post_http(&db, "lock", deploy, &leases);
            if taken.status == 200 {
                break;
            }
            assert!(Instant::now() < until, "{taken:?}");
            thread::sleep(Duration::from_millis(10));
        }
        // A pipe's connection has no routes by id.
        let by_id = post(&db, &format!("lock/{id}/renew"), "", &Locks::new(LEASE));
        assert_eq!(by_id.status, 404);
    }

    #[test]
    fn a_pickup_wait_s_locks_are_held_until_the_connection_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let db = Db::new(&path);
        let deploy = r#"{"locks": [{"on": "label", "text": "deploy"}]}"#;
        // A store not made yet takes none, and makes no directory of them.
        let first = Locks::new(LEASE);
        assert_eq!(lock_with(&db, deploy, &first), Answer::ok(json!({})));
        assert_eq!(held_count(&first), 0);
        assert!(!dir.path().join("cases.db-waits").exists());
        std::fs::write(&path, "").unwrap();
        assert_eq!(lock_with(&db, deploy, &first), Answer::ok(json!({})));
        assert_eq!(held_count(&first), 1);
        // A second connection is refused while the first holds them.
        let second = Locks::new(LEASE);
        let both =
            r#"{"locks": [{"on": "worker", "text": "w"}, {"on": "label", "text": "deploy"}]}"#;
        let refused = lock_with(&db, both, &second);
        assert_eq!(refused.status, 409);
        assert_eq!(refused.body["error"]["category"], "held");
        let message = refused.body["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("another wait is already picking up label \"deploy\" (pid "),
            "{message}"
        );
        assert_eq!(held_count(&second), 0);
        // Once the first ends, they are free.
        first.end();
        assert_eq!(lock_freed(&db, both, &second), Answer::ok(json!({})));
        // A body that is not a list of locks, or names something else, is
        // refused.
        for body in [
            "{}",
            r#"{"locks": {}}"#,
            r#"{"locks": [{"on": "label"}]}"#,
            r#"{"locks": [{"on": "../x", "text": "deploy"}]}"#,
            r#"{"locks": [], "lease": "yes"}"#,
            r#"{"locks": [], "lease": true, "release": 1}"#,
        ] {
            let bad = lock_with(&db, body, &Locks::new(LEASE));
            assert_eq!(
                (bad.status, bad.body["error"]["category"].clone()),
                (400, json!("request")),
                "{body}"
            );
        }
    }

    /// A request to `route` with `body`, as the peer sends it.
    fn sent(route: &str, body: &str, close: bool) -> String {
        let close = if close { "Connection: close\r\n" } else { "" };
        format!(
            "POST /v1/stores/{DIAL_STORE}/store/{route} HTTP/1.1\r\nContent-Type: \
             application/json\r\n{close}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn locks_a_wait_lets_go_itself_outlast_its_connection_until_it_does_or_their_lease_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        std::fs::write(&path, "").unwrap();
        let db = Db::new(&path);
        let releases =
            r#"{"locks": [{"on": "worker", "text": "lead"}], "lease": true, "release": true}"#;
        let leased = r#"{"locks": [{"on": "worker", "text": "lead"}], "lease": true}"#;
        let plain = r#"{"locks": [{"on": "worker", "text": "lead"}]}"#;
        let lease = Duration::from_millis(600);
        // Its connection ends with no release, as one that dies does: the
        // locks are kept until the lease runs out, and then let go.
        let start = Instant::now();
        thread::scope(|scope| {
            let serving = scope.spawn(|| {
                let mut out = Vec::new();
                serve(
                    &db,
                    lease,
                    &mut sent("lock", releases, false).as_bytes(),
                    &mut out,
                )
                .unwrap();
                Instant::now()
            });
            thread::sleep(Duration::from_millis(200));
            let second = Locks::new(lease);
            let refused = lock_with(&db, plain, &second);
            assert_eq!(
                (refused.status, refused.body["error"]["category"].clone()),
                (409, json!("held"))
            );
            let ended = serving.join().unwrap();
            assert!(ended - start >= lease, "{:?}", ended - start);
            assert_eq!(lock_freed(&db, plain, &second), Answer::ok(json!({})));
            second.end();
        });
        // Let go first, they go at once, and the connection ends with the
        // release's answer. The lease is long, so a wait for it would show.
        let start = Instant::now();
        let got = served(
            &db,
            &format!(
                "{}{}",
                sent("lock", releases, false),
                sent("lock/release", "", true)
            ),
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(got.matches("HTTP/1.1 200 OK").count(), 2, "{got}");
        let second = Locks::new(lease);
        assert_eq!(lock_freed(&db, plain, &second), Answer::ok(json!({})));
        second.end();
        // A wait that did not say it lets them go itself, as one older than
        // the release is, has them let go as its connection ends.
        let start = Instant::now();
        served(&db, &sent("lock", leased, false));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        let second = Locks::new(lease);
        assert_eq!(lock_freed(&db, plain, &second), Answer::ok(json!({})));
        second.end();
    }

    #[test]
    fn leased_locks_are_held_while_the_lease_is_renewed_and_let_go_when_it_runs_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        std::fs::write(&path, "").unwrap();
        let db = Db::new(&path);
        let deploy = r#"{"locks": [{"on": "label", "text": "deploy"}], "lease": true}"#;
        let lease = Duration::from_millis(300);
        let first = Locks::new(lease);
        let second = Locks::new(lease);
        thread::scope(|scope| {
            scope.spawn(|| first.watch());
            scope.spawn(|| second.watch());
            assert_eq!(
                lock_with(&db, deploy, &first),
                Answer::ok(json!({"lease_ms": 300}))
            );
            // Renewed, for three leases' time, they are kept.
            let until = Instant::now() + lease * 3;
            while Instant::now() < until {
                thread::sleep(lease / 3);
                assert_eq!(
                    post(&db, "lock/renew", "{}", &first),
                    Answer::ok(json!({"lease_ms": 300}))
                );
                assert_eq!(lock_with(&db, deploy, &second).status, 409);
            }
            // Not renewed, they are let go once it runs out, though the
            // connection is open, and a renewal after is refused.
            thread::sleep(lease * 2);
            assert_eq!(held_count(&first), 0);
            let lapsed = post(&db, "lock/renew", "{}", &first);
            assert_eq!(
                (lapsed.status, lapsed.body["error"]["category"].clone()),
                (409, json!("lapsed"))
            );
            assert_eq!(
                lock_freed(&db, deploy, &second),
                Answer::ok(json!({"lease_ms": 300}))
            );
            first.end();
            second.end();
        });
        // A connection with no lease has none to renew, and one that took
        // its locks without a lease keeps them however long it waits.
        let unleased = Locks::new(Duration::from_millis(50));
        let refused = post(&db, "lock/renew", "{}", &unleased);
        assert_eq!(
            (refused.status, refused.body["error"]["category"].clone()),
            (409, json!("lapsed"))
        );
        thread::scope(|scope| {
            scope.spawn(|| unleased.watch());
            let plain = r#"{"locks": [{"on": "label", "text": "deploy"}]}"#;
            assert_eq!(lock_freed(&db, plain, &unleased), Answer::ok(json!({})));
            thread::sleep(Duration::from_millis(200));
            assert_eq!(held_count(&unleased), 1);
            unleased.end();
        });
    }

    #[test]
    fn a_write_s_answer_is_the_rows_it_stored() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let post = |path: &str, body: &Value| {
            let body = body.to_string();
            let Read::Request(request) = request(&format!(
                "POST /v1/stores/{DIAL_STORE}/store/{path} HTTP/1.1\r\nContent-Type: \
                 application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )) else {
                panic!("not read");
            };
            handle(&db, &request, &Locks::new(LEASE))
        };
        let opened = post(
            "cases",
            &json!({"writer": null, "append": {"author": "agent", "event": "open",
                "data": r#"{"kind":"fyi","urgency":"today","title":"x"}"#}}),
        );
        assert_eq!(opened.status, 200, "{opened:?}");
        let key = opened.body["key"].as_u64().unwrap();
        let k = Key::new(key).unwrap();
        assert_eq!(opened.body, case_json(k, &db.rows(&k).unwrap().unwrap()));
        let noted = post(
            &format!("cases/{key}/events"),
            &json!({"writer": null, "revision": 1, "append": {"author": "agent",
                "event": "note", "data": r#"{"body":"more"}"#}}),
        );
        assert_eq!(noted.status, 200, "{noted:?}");
        assert_eq!(noted.body, case_json(k, &db.rows(&k).unwrap().unwrap()));
        assert_eq!(noted.body["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_held_poll_ends_at_its_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let Read::Request(request) = request(&format!(
            "GET /v1/stores/{DIAL_STORE}/store/changes?after=0&timeout=1 HTTP/1.1\r\n\r\n"
        )) else {
            panic!("not read");
        };
        let start = Instant::now();
        assert_eq!(
            handle(&db, &request, &Locks::new(LEASE)),
            Answer::ok(json!({"version": "0"}))
        );
        let took = start.elapsed();
        assert!(
            took >= Duration::from_secs(1) && took < Duration::from_secs(3),
            "{took:?}"
        );
    }

    #[test]
    fn a_store_s_own_errors_are_missing_busy_newer_or_failed() {
        let path = std::path::PathBuf::from("/x");
        let missing = Answer::failed(&DbError::Missing { path: path.clone() });
        assert_eq!(missing.status, 404);
        assert_eq!(missing.body["error"]["category"], "missing");
        assert_eq!(
            Answer::failed(&DbError::Busy { path: path.clone() }).status,
            503
        );
        let newer = Answer::failed(&DbError::Newer {
            path: path.clone(),
            version: 9,
        });
        assert_eq!(newer.status, 500);
        assert_eq!(newer.body["error"]["category"], "newer");
        let message = newer.body["error"]["message"].as_str().unwrap();
        assert!(message.contains("update cases there"), "{message}");
        let failed = Answer::failed(&DbError::Clock);
        assert_eq!(failed.status, 500);
        assert_eq!(failed.body["error"]["category"], "store");
    }

    #[test]
    fn each_status_has_its_reason() {
        for (status, reason) in [
            (200, "OK"),
            (400, "Bad Request"),
            (404, "Not Found"),
            (409, "Conflict"),
            (422, "Unprocessable Content"),
            (503, "Service Unavailable"),
            (500, "Internal Server Error"),
        ] {
            let mut out = Vec::new();
            write(
                &mut out,
                &Answer {
                    status,
                    body: json!({}),
                },
            )
            .unwrap();
            let out = String::from_utf8(out).unwrap();
            assert!(
                out.starts_with(&format!("HTTP/1.1 {status} {reason}\r\n")),
                "{out}"
            );
        }
    }

    #[test]
    fn an_unknown_kind_names_the_versions_on_each_side() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let body = json!({"writer": null, "append": {"author": "agent", "event": "open",
            "data": r#"{"kind":"memo","urgency":"today","title":"x","worker":"lead"}"#}})
        .to_string();
        let open = |agent: &str| {
            served(
                &db,
                &format!(
                    "POST /v1/stores/{DIAL_STORE}/store/cases HTTP/1.1\r\n{agent}Content-Type: \
                     application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
            )
        };
        let here = env!("CARGO_PKG_VERSION");
        let got = open("User-Agent: cases/9.9.9\r\n");
        assert!(got.starts_with("HTTP/1.1 422 "), "{got}");
        assert!(
            got.contains(&format!(
                r#""category":"unknown","message":"\"memo\" is not a kind: the cases on the far side is {here}, and this one 9.9.9; update the older""#
            )),
            "{got}"
        );
        for agent in [String::new(), format!("User-Agent: cases/{here}\r\n")] {
            let got = open(&agent);
            assert!(
                got.contains(&format!(
                    r#""message":"\"memo\" is not a kind: the cases on the far side is {here}""#
                )),
                "{got}"
            );
        }
        // Any other answer is left as it is.
        let other = Answer::refuse(409, "stale", "why");
        assert_eq!(
            skew(other, Some("cases/9.9.9")),
            Answer::refuse(409, "stale", "why")
        );
    }
}
