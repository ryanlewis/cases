//! What a forwarded store's far side holds for a client while a connection
//! of its own lasts, such as a pickup wait's locks, and the lease it may
//! hold them on.
//!
//! A far side that gives the locks as a lease lets them go once the lease
//! runs out with no renewal, even while the connection stays open, as an
//! ssh connection does for a while to a laptop that went to sleep. The
//! hold renews it on a thread of its own, over its connection, a third of
//! the way through from when it sent the last renewal, by either clock. It counts itself held only until the lease from the last
//! renewal it saw answered would end, less a third: it counts from when it
//! sent the renewal, which is before the far side had it, and it reads both
//! the monotonic clock and the wall clock, so that time asleep, which the
//! first may not count, still counts. So a wait that checks its hold stops
//! before the far side lets go.
//!
//! Such a far side keeps the locks past the connection's end until the
//! lease runs out, as the hold asks it to, unless the hold let them go
//! first ([`Hold::release`], and its drop): a connection that dies may
//! leave the wait's pickup still on its way over another, which a second
//! wait must not race. A far side too old to be let go of this way lets go
//! as the connection ends.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::client::{Client, classify, protocol_of};
use crate::error::Error;
use crate::pipe::Pipe;

/// The most an answer to a renewal may hold.
const RENEW_LIMIT: u64 = 65_536;

/// The longest lease a hold takes from a far side: one longer is taken
/// as this.
const LEASE_MAX: Duration = Duration::from_secs(3600);

/// The shortest lease a hold keeps: one shorter, which only a bad far side
/// or a test's gives, is refused, as it could run out before a renewal
/// reached it.
pub const LEASE_MIN: Duration = Duration::from_millis(100);

/// How often the renewing thread looks to see whether the connection has
/// ended between renewals.
const WATCH_EVERY: Duration = Duration::from_millis(100);

/// A connection to a forwarded store kept open for what the far side holds
/// while it lasts, such as a pickup wait's locks
/// ([`RemoteStore::hold`](crate::RemoteStore::hold)). Dropped, it lets them
/// go ([`Hold::release`]) and ends the child. When the far side gave them
/// on a lease, the hold renews it until it is let go, and [`Hold::check`]
/// says once it may have run out.
#[derive(Debug)]
pub struct Hold {
    kept: Kept,
}

#[derive(Debug)]
enum Kept {
    /// Held while the connection lasts.
    Open(#[allow(dead_code, reason = "kept for its drop, which ends the child")] Pipe),
    /// Held while the lease is renewed, by a thread that has the connection.
    Lease {
        standing: Arc<Mutex<Standing>>,
        /// Tells the thread to let go, and then stop, which ends the
        /// child: it answers on the sender it is given once it has.
        release: Mutex<Option<Sender<Sender<()>>>>,
    },
}

/// Until when a hold on a lease is good, by each clock, or why it is not.
#[derive(Debug)]
struct Standing {
    until: Instant,
    wall: SystemTime,
    lost: Option<String>,
}

impl Standing {
    /// Good for `lease` from `sent`, less a third.
    fn from(sent: (Instant, SystemTime), lease: Duration) -> Standing {
        let good = lease.saturating_sub(lease / 3);
        Standing {
            until: sent.0 + good,
            wall: sent.1 + good,
            lost: None,
        }
    }

    /// How long it is good for still, or why it is not.
    fn left(&self) -> Result<Duration, String> {
        if let Some(why) = &self.lost {
            return Err(why.clone());
        }
        let left = self.until.saturating_duration_since(Instant::now()).min(
            self.wall
                .duration_since(SystemTime::now())
                .unwrap_or_default(),
        );
        if left.is_zero() {
            return Err("it was not renewed in time".to_owned());
        }
        Ok(left)
    }
}

/// The answer to a lock or a renewal that gives a lease.
#[derive(Deserialize)]
struct Lease {
    lease_ms: u64,
}

/// The lease `body` gives, if it gives one.
///
/// # Errors
///
/// Why, when it gives one shorter than [`LEASE_MIN`].
pub(crate) fn lease_of(body: &[u8]) -> Result<Option<Duration>, String> {
    let Ok(lease) = serde_json::from_slice::<Lease>(body) else {
        return Ok(None);
    };
    let lease = Duration::from_millis(lease.lease_ms);
    if lease < LEASE_MIN {
        return Err(format!(
            "the far side gave a lease of {} ms, shorter than the {} ms a wait can keep",
            lease.as_millis(),
            LEASE_MIN.as_millis()
        ));
    }
    Ok(Some(lease.min(LEASE_MAX)))
}

/// The answer to a lock over HTTP: the hold's id.
#[derive(Deserialize)]
struct Id {
    hold: String,
}

/// The id of the hold `body` gives, `{hold}`, when it is 1 to 64 ASCII
/// letters and digits, which go in a path as they are.
pub(crate) fn id_of(body: &[u8]) -> Option<String> {
    let id = serde_json::from_slice::<Id>(body).ok()?.hold;
    ((1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(id)
}

/// The time on each clock, as a request is sent.
pub(crate) fn now() -> (Instant, SystemTime) {
    (Instant::now(), SystemTime::now())
}

fn guard(standing: &Mutex<Standing>) -> MutexGuard<'_, Standing> {
    standing.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hold {
    /// Held while `pipe` lasts.
    pub(crate) fn open(pipe: Pipe) -> Hold {
        Hold {
            kept: Kept::Open(pipe),
        }
    }

    /// Held on a lease of `lease`, given in answer to a request sent at
    /// `sent`, renewed and let go over `link`.
    pub(crate) fn leased(link: Link, lease: Duration, sent: (Instant, SystemTime)) -> Hold {
        let standing = Arc::new(Mutex::new(Standing::from(sent, lease)));
        let (release, released) = mpsc::channel();
        let renewing = Arc::clone(&standing);
        // Not joined: it ends the child once it ends, or the process's end
        // does. [`Hold::release`] waits for it to let go.
        thread::spawn(move || renew_until(link, lease, sent, &renewing, &released));
        Hold {
            kept: Kept::Lease {
                standing,
                release: Mutex::new(Some(release)),
            },
        }
    }

    /// Lets go of what the far side holds, at once, and ends the child, as
    /// a wait that has ended does, whether it ended well, failed or was
    /// stopped by a signal. Once it has, [`Hold::check`] fails. A far side
    /// that gave no lease, or is too old to be let go of this way, lets go
    /// as the connection ends; one that gave a lease would otherwise keep
    /// it until the lease runs out. It waits for a renewal on its way, and
    /// for the far side's answer, each for no longer than a third of the
    /// lease, unless the hold may have lapsed already: then nothing waits
    /// on it, and the far side lets go once the lease runs out, if it has
    /// not had the release before. Letting go again does nothing, once the
    /// first has let go: a second, such as a signal's as the wait ends,
    /// waits for it.
    pub fn release(&self) {
        let Kept::Lease { release, standing } = &self.kept else {
            return;
        };
        let lapsed = guard(standing).left().is_err();
        // Held until it has let go, so that a second waits for it.
        let mut sender = release.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(release) = sender.take() else {
            return;
        };
        let (done, released) = mpsc::channel();
        // A thread that has stopped, its connection lost, has dropped its
        // end: there is nothing to let go over.
        if release.send(done).is_ok() && !lapsed {
            let _ = released.recv();
        }
        drop(sender);
    }

    /// Whether what the far side holds is held still.
    ///
    /// # Errors
    ///
    /// [`Error::Lapsed`] when it was given on a lease that may have run
    /// out: a renewal failed, or none was answered in time.
    pub fn check(&self) -> Result<(), Error> {
        self.left().map(|_| ())
    }

    /// How long what the far side holds is held for still, if it was given
    /// on a lease, so that a wait can check again before then; `None` when
    /// it is held while the connection lasts.
    ///
    /// # Errors
    ///
    /// [`Error::Lapsed`], as [`Hold::check`] gives it.
    pub fn left(&self) -> Result<Option<Duration>, Error> {
        match &self.kept {
            Kept::Open(_) => Ok(None),
            Kept::Lease { standing: s, .. } => guard(s)
                .left()
                .map(Some)
                .map_err(|reason| Error::Lapsed { reason }),
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.release();
    }
}

/// How a hold on a lease renews it and lets go.
#[derive(Debug)]
pub(crate) enum Link {
    /// Over a connection of its own, a pipe.
    Pipe {
        pipe: Pipe,
        /// Renews the lease.
        renew: Vec<u8>,
        /// Lets go of what the far side holds, and closes the connection.
        release: Vec<u8>,
    },
    /// Over HTTP, a request of its own each time, to these paths.
    Http {
        client: Client,
        renew: String,
        release: String,
    },
}

impl Link {
    /// Renews the lease, within `timeout`: the answer's status and body.
    fn renew(&mut self, timeout: Duration) -> Result<(u16, Vec<u8>), ureq::Error> {
        match self {
            Link::Pipe { pipe, renew, .. } => pipe.exchange(renew, timeout, RENEW_LIMIT),
            Link::Http { client, renew, .. } => client.post_empty(renew, timeout, RENEW_LIMIT),
        }
    }

    /// Lets go, within `timeout`, whatever the answer.
    fn release(self, timeout: Duration) {
        match self {
            Link::Pipe {
                mut pipe, release, ..
            } => {
                let _ = pipe.exchange(&release, timeout, RENEW_LIMIT);
            }
            Link::Http {
                client, release, ..
            } => {
                let _ = client.post_empty(&release, timeout, RENEW_LIMIT);
            }
        }
    }

    /// Why the connection ended between renewals, if it has: a pipe's.
    /// Over HTTP each request is on its own, so there is none to end.
    fn ended(&mut self) -> Option<String> {
        match self {
            Link::Pipe { pipe, .. } => (!pipe.is_open()).then(|| why(&pipe.closed())),
            Link::Http { .. } => None,
        }
    }
}

/// Renews the lease, given in answer to a request sent at `sent`, over
/// `link` a third of the way through each, until `release` says to let go,
/// or a renewal fails, or the connection ends, which it says in `standing`.
/// Each is due a third of a lease after the last was sent, by whichever
/// clock says so first: a renewal due while asleep is sent on waking.
fn renew_until(
    mut link: Link,
    mut lease: Duration,
    mut sent: (Instant, SystemTime),
    standing: &Mutex<Standing>,
    release: &Receiver<Sender<()>>,
) {
    let lose = |why: String| guard(standing).lost = Some(why);
    loop {
        let every = lease / 3;
        loop {
            let left = (sent.0 + every)
                .saturating_duration_since(Instant::now())
                .min(
                    (sent.1 + every)
                        .duration_since(SystemTime::now())
                        .unwrap_or_default(),
                );
            if left.is_zero() {
                break;
            }
            match release.recv_timeout(left.min(WATCH_EVERY)) {
                Err(RecvTimeoutError::Timeout) => {}
                Ok(done) => {
                    lose("this wait let them go".to_owned());
                    // Its answer is not needed: a far side too old to know
                    // the route refuses it, 404, and lets go as the
                    // connection ends, as it does once this has.
                    link.release(every);
                    let _ = done.send(());
                    return;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if let Some(why) = link.ended() {
                return lose(why);
            }
        }
        sent = now();
        match link.renew(every) {
            Ok((200, body)) => match lease_of(&body) {
                Ok(Some(renewed)) => {
                    lease = renewed;
                    *guard(standing) = Standing::from(sent, lease);
                }
                Ok(None) => return lose("the answer to its renewal gave no lease".to_owned()),
                Err(why) => return lose(why),
            },
            Ok((status, body)) => {
                return lose(match protocol_of(&classify(status, None, &body)) {
                    Error::Lapsed { reason } => reason,
                    error => error.to_string(),
                });
            }
            Err(error) => return lose(why(&error)),
        }
    }
}

/// Why a renewal got no answer, without the HTTP client's prefix. The
/// pipe's own errors are all of a connection that ended, and say why.
fn why(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Io(io) => format!("its connection ended: {io}"),
        ureq::Error::Timeout(_) => "its renewal got no answer in time".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    use crate::Token;
    use crate::client::{Client, Limits};
    use crate::shape::Server;

    /// A server that answers a lock with a lease of `lease_ms` and the id
    /// `h1`, and each renewal by `renew` (the status and body), recording
    /// each request's method, path and bearer: its address and the record.
    fn far(lease_ms: u64, renew: (u16, &'static str)) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let record = Arc::clone(&record);
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut stream = stream;
                    loop {
                        let mut first = String::new();
                        if reader.read_line(&mut first).unwrap_or(0) == 0 {
                            return;
                        }
                        let (mut length, mut bearer) = (0, String::new());
                        loop {
                            let mut line = String::new();
                            reader.read_line(&mut line).unwrap();
                            if line == "\r\n" {
                                break;
                            }
                            let (name, value) = line.split_once(':').unwrap();
                            let value = value.trim().to_owned();
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.parse().unwrap();
                            } else if name.eq_ignore_ascii_case("authorization") {
                                bearer = value;
                            }
                        }
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).unwrap();
                        let line: Vec<&str> = first.split(' ').collect();
                        let path = line[1].to_owned();
                        record
                            .lock()
                            .unwrap()
                            .push(format!("{} {path} {bearer}", line[0]));
                        let (status, answer) = if path.ends_with("/lock") {
                            (200, format!(r#"{{"lease_ms": {lease_ms}, "hold": "h1"}}"#))
                        } else if path.ends_with("/renew") {
                            (renew.0, renew.1.replace('@', &lease_ms.to_string()))
                        } else {
                            (200, "{}".to_owned())
                        };
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n{answer}",
                            answer.len()
                        );
                    }
                });
            }
        });
        (addr, seen)
    }

    fn client(addr: &str) -> Client {
        let token = Token::forward(&"t".repeat(32)).unwrap();
        Client::forward_with_limits(
            Server::forward(addr).unwrap(),
            Some(token),
            Limits::default(),
        )
    }

    #[test]
    fn over_http_a_hold_is_renewed_and_let_go_by_its_id_with_the_token() {
        let (addr, seen) = far(300, (200, r#"{"lease_ms": @}"#));
        let hold = client(&addr).hold("/s/lock", b"{}").unwrap();
        // Renewed a third of the way through each lease, it outlives them.
        thread::sleep(Duration::from_millis(700));
        assert!(hold.left().unwrap().is_some());
        hold.release();
        assert!(hold.check().is_err());
        let seen = seen.lock().unwrap().clone();
        let bearer = format!("Bearer {}", "t".repeat(32));
        assert_eq!(seen[0], format!("POST /s/lock {bearer}"));
        let renewals = seen
            .iter()
            .filter(|r| *r == &format!("POST /s/lock/h1/renew {bearer}"));
        assert!(renewals.count() >= 3, "{seen:?}");
        assert_eq!(
            seen.last().unwrap(),
            &format!("POST /s/lock/h1/release {bearer}")
        );
    }

    #[test]
    fn over_http_a_hold_whose_renewal_is_refused_has_lapsed() {
        let refusal = r#"{"error": {"category": "lapsed", "message": "no locks by this id"}}"#;
        let (addr, _) = far(300, (409, refusal));
        let hold = client(&addr).hold("/s/lock", b"{}").unwrap();
        thread::sleep(Duration::from_millis(300));
        match hold.check() {
            Err(Error::Lapsed { reason }) => assert_eq!(reason, "no locks by this id"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_hold_s_id_is_letters_and_digits_that_go_in_a_path() {
        let id = |body: &str| id_of(body.as_bytes());
        assert_eq!(
            id(r#"{"hold": "0f3a", "lease_ms": 30000}"#).as_deref(),
            Some("0f3a")
        );
        let long = "a".repeat(64);
        assert_eq!(id(&format!(r#"{{"hold": "{long}"}}"#)), Some(long));
        for body in [
            r#"{"lease_ms": 30000}"#,
            r#"{"hold": ""}"#,
            r#"{"hold": "../x"}"#,
            r#"{"hold": "a/renew"}"#,
            r#"{"hold": 7}"#,
            "not json",
        ] {
            assert_eq!(id(body), None, "{body}");
        }
        assert_eq!(id(&format!(r#"{{"hold": "{}"}}"#, "a".repeat(65))), None);
    }
}
