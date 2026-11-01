//! The TUI's reads and writes of the store, on a thread of their own
//! (TU-04): the screen asks for a read or a write and takes what came of it
//! once it is ready, so that a slow store, such as one forwarded over ssh on
//! a slow link, never holds up the keys or the drawing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cases_core::{
    Append, Author, Case, Category, Listening, Record, Stats, Store as _, StoreError, Time,
};
use cases_store_sqlite::{Key, Writer};

use super::app::Refusal;
use super::follow::Follow;
use crate::ctx;
use crate::error::Error;
use crate::output::{self, Actors};
use crate::store::{AnyStore, Poller};

/// What one read of the store gave.
#[derive(Debug)]
pub(crate) enum Snapshot {
    /// Every case that folded, how many did not, the actors and who
    /// listens, at `now`, and the stats of the cases (TU-13).
    Read {
        cases: Vec<(Key, Case)>,
        unfolded: usize,
        actors: Actors,
        listeners: Vec<Listening>,
        now: Time,
        stats: Stats,
    },
    /// The store could not be read, and why: tried again on a later read.
    Unreadable { why: String, now: Time },
    /// A store from a newer cases, here or on a forward's far side, or a
    /// clock out of range: the end of the program (TU-01).
    Fatal(Error),
}

/// Reads `db` with `poller` once. A local store not made yet has no cases
/// (ST-08).
pub(crate) fn snapshot(poller: &mut Poller<'_>, db: &AnyStore) -> Snapshot {
    let now = match output::now() {
        Ok(now) => now,
        Err(error) => return Snapshot::Fatal(error),
    };
    let polled = poller.poll().map(|cases| {
        let mut folded: Vec<(Key, Case)> = Vec::new();
        let mut unfolded = 0;
        for (key, case) in cases {
            match case {
                Ok(case) => folded.push((*key, case.clone())),
                Err(_) => unfolded += 1,
            }
        }
        (folded, unfolded)
    });
    match polled {
        Ok((cases, unfolded)) => match Actors::of(db) {
            // Who is listening is a convenience: a store that cannot say
            // has no one.
            Ok(actors) => Snapshot::Read {
                stats: stats(&cases, now),
                cases,
                unfolded,
                actors,
                listeners: db.listeners().unwrap_or_default(),
                now,
            },
            Err(error) => Snapshot::Unreadable {
                why: error.to_string(),
                now,
            },
        },
        Err(error) if error.is_missing() => Snapshot::Read {
            cases: Vec::new(),
            unfolded: 0,
            actors: Actors::default(),
            listeners: Vec::new(),
            now,
            stats: stats(&[], now),
        },
        Err(error) if error.is_newer() => Snapshot::Fatal(error.into()),
        Err(error) => Snapshot::Unreadable {
            why: error.to_string(),
            now,
        },
    }
}

/// The stats strip's counts of `cases` at `now` (TU-13), worked out here
/// with the read so that the screen only draws them.
fn stats(cases: &[(Key, Case)], now: Time) -> Stats {
    cases_core::stats(cases.iter().map(|(_, case)| case), now, super::stats::DAYS)
}

/// A write the human asked for.
#[derive(Debug)]
pub(crate) enum Write {
    /// The human's event on a case, at the revision the form was drawn at
    /// (IN-05).
    Append {
        key: Key,
        record: Record,
        revision: u64,
    },
    /// A new message (HU-08).
    Create(Record),
}

/// How a write went.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Wrote {
    Append(Result<(), Refusal>),
    /// The new message's key, and whether a session listens for it now; or
    /// why it was refused.
    Create(Result<(Key, bool), String>),
}

impl Wrote {
    /// Whether the write was recorded.
    pub(crate) fn landed(&self) -> bool {
        match self {
            Wrote::Append(result) => result.is_ok(),
            Wrote::Create(result) => result.is_ok(),
        }
    }
}

impl Write {
    /// Sends the write to `db` as `writer`.
    pub(crate) fn send(&self, db: &AnyStore, writer: &Writer) -> Wrote {
        match self {
            Write::Append {
                key,
                record,
                revision,
            } => {
                let append = Append::new(Author::Human, record);
                Wrote::Append(match db.append(key, &append, writer, Some(*revision)) {
                    Ok(_) => Ok(()),
                    Err(StoreError::Refused(error)) if error.category() == Category::Stale => {
                        Err(Refusal::Stale)
                    }
                    Err(StoreError::Refused(error)) => Err(Refusal::Other(error.to_string())),
                    Err(StoreError::NoCase) => Err(Refusal::Other(format!("no case {key}"))),
                    Err(error) => Err(Refusal::Other(error.to_string())),
                })
            }
            Write::Create(record) => {
                let append = Append::new(Author::Human, record);
                Wrote::Create(
                    db.create(&append, writer)
                        .map(|(key, case)| {
                            (key, ctx::listened_for(db, case.worker(), case.labels()))
                        })
                        .map_err(|e| e.to_string()),
                )
            }
        }
    }
}

/// What the screen asks of the thread.
#[derive(Debug)]
enum Job {
    /// Read `n`.
    Read(u64),
    /// Send the write, then read `n`.
    Write(Box<Write>, u64),
}

/// What the thread gives back: a read, or how a write went, numbered as
/// the read after it.
#[derive(Debug)]
enum Landed {
    Read(u64, Snapshot),
    Wrote(u64, Wrote),
}

/// The thread that reads and writes the store. The screen asks for a read,
/// which is numbered, and takes each read as it lands. Asks made while a
/// read runs are answered by one read after it. A write is sent in the
/// order asked, after the reads asked before it and before those asked
/// after it, and is followed by a read of its own, so that how it went is
/// told with the case as it left it. After each read that works, a remote
/// or forwarded store's poll is held from the version read (`follow`).
///
/// Dropping this ends the thread once what it runs is done; nothing waits
/// for that.
#[derive(Debug)]
pub(crate) struct Reader {
    asks: Sender<Job>,
    reads: Receiver<Landed>,
    /// The number of the last read asked for.
    asked: u64,
    /// The number of the last read taken.
    taken: u64,
    /// How the last write went, and the number of the read after it, once
    /// it has landed and until it is given by [`Reader::wrote`].
    wrote: Option<(u64, Wrote)>,
    #[cfg_attr(not(test), allow(dead_code, reason = "joined only in a test"))]
    thread: JoinHandle<()>,
}

impl Reader {
    /// Starts the thread on `db`, writing as `writer`.
    ///
    /// # Errors
    ///
    /// When no thread can be started.
    pub(crate) fn start(
        db: Arc<AnyStore>,
        writer: Writer,
        follow: Option<Arc<Follow>>,
    ) -> Result<Reader, Error> {
        let (asks, jobs) = mpsc::channel::<Job>();
        let (landed, reads) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("cases-read".into())
            .spawn(move || {
                let mut poller = db.poller();
                // A write met while passing over asks, run next.
                let mut next = None;
                loop {
                    let Some(job) = next.take().or_else(|| jobs.recv().ok()) else {
                        return;
                    };
                    let n = match job {
                        Job::Read(mut n) => {
                            // One read for every ask made while the last one
                            // ran, up to a write.
                            while let Ok(later) = jobs.try_recv() {
                                match later {
                                    Job::Read(m) => n = m,
                                    write @ Job::Write(..) => {
                                        next = Some(write);
                                        break;
                                    }
                                }
                            }
                            n
                        }
                        Job::Write(write, n) => {
                            let wrote = write.send(&db, &writer);
                            if landed.send(Landed::Wrote(n, wrote)).is_err() {
                                return;
                            }
                            n
                        }
                    };
                    let read = snapshot(&mut poller, &db);
                    if matches!(read, Snapshot::Read { .. })
                        && let (Some(follow), Some(store), Some(version)) =
                            (&follow, db.followed(), poller.version())
                    {
                        follow.hold(store, version);
                    }
                    if landed.send(Landed::Read(n, read)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|error| Error::new(format!("cannot start the reads: {error}")))?;
        Ok(Reader {
            asks,
            reads,
            asked: 0,
            taken: 0,
            wrote: None,
            thread,
        })
    }

    /// Asks for a read, and gives its number.
    pub(crate) fn ask(&mut self) -> u64 {
        self.asked += 1;
        let _ = self.asks.send(Job::Read(self.asked));
        self.asked
    }

    /// Asks for `write` to be sent, then the store read, and gives the
    /// number of that read: [`Reader::wrote`] gives how the write went once
    /// it has been taken.
    pub(crate) fn write(&mut self, write: Write) -> u64 {
        self.asked += 1;
        let _ = self.asks.send(Job::Write(Box::new(write), self.asked));
        self.asked
    }

    /// How the last write went, once the read after it has been taken; once.
    pub(crate) fn wrote(&mut self) -> Option<Wrote> {
        match &self.wrote {
            Some((n, _)) if *n <= self.taken => self.wrote.take().map(|(_, wrote)| wrote),
            _ => None,
        }
    }

    /// Whether a read asked for has not been taken yet.
    pub(crate) fn busy(&self) -> bool {
        self.taken < self.asked
    }

    /// The latest read that has landed, if one has since the last taken.
    pub(crate) fn take(&mut self) -> Option<Snapshot> {
        let mut latest = None;
        loop {
            match self.reads.try_recv() {
                Ok(Landed::Read(n, read)) => {
                    self.taken = n;
                    latest = Some(read);
                }
                Ok(Landed::Wrote(n, wrote)) => self.wrote = Some((n, wrote)),
                Err(TryRecvError::Empty) => return latest,
                // The thread is gone, having panicked: no more reads land.
                Err(TryRecvError::Disconnected) => {
                    self.taken = self.asked;
                    return latest.or_else(|| {
                        Some(Snapshot::Fatal(Error::new(
                            "the reads of the store stopped",
                        )))
                    });
                }
            }
        }
    }

    /// Waits for read `n`, or a later one, and gives it; reads asked for
    /// before it are passed over, as they may be from before a write, unless
    /// they end the program. Gives `None` once `stop` is set, looking every
    /// `tick`.
    pub(crate) fn wait(&mut self, n: u64, stop: &AtomicBool, tick: Duration) -> Option<Snapshot> {
        self.wait_until(n, stop, tick, None)
    }

    /// Waits as [`Reader::wait`] for the last read asked for, which follows
    /// a write being sent, if one is; for no longer than `within`. Gives
    /// `None` at once when every read asked for has been taken.
    pub(crate) fn settle(
        &mut self,
        stop: &AtomicBool,
        tick: Duration,
        within: Duration,
    ) -> Option<Snapshot> {
        if !self.busy() {
            return None;
        }
        self.wait_until(self.asked, stop, tick, Some(Instant::now() + within))
    }

    /// [`Reader::wait`], giving `None` at `deadline` too.
    fn wait_until(
        &mut self,
        n: u64,
        stop: &AtomicBool,
        tick: Duration,
        deadline: Option<Instant>,
    ) -> Option<Snapshot> {
        loop {
            match self.reads.recv_timeout(tick) {
                Ok(Landed::Read(m, read)) => {
                    self.taken = m;
                    if m >= n || matches!(read, Snapshot::Fatal(_)) {
                        return Some(read);
                    }
                }
                Ok(Landed::Wrote(m, wrote)) => self.wrote = Some((m, wrote)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.taken = self.asked;
                    return Some(Snapshot::Fatal(Error::new(
                        "the reads of the store stopped",
                    )));
                }
            }
            if stop.load(Ordering::Relaxed) || deadline.is_some_and(|at| Instant::now() >= at) {
                return None;
            }
        }
    }

    /// Ends the thread, waiting for what it runs, so that the store is the
    /// caller's alone again.
    #[cfg(test)]
    pub(crate) fn finish(self) {
        drop(self.asks);
        let _ = self.thread.join();
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Author, Open, Record};
    use cases_store_sqlite::{Db, Writer};

    use super::*;

    fn open(db: &AnyStore, title: &str) {
        let mut open = Open::default();
        "fyi".clone_into(&mut open.kind);
        "today".clone_into(&mut open.urgency);
        title.clone_into(&mut open.title);
        "worker".clone_into(&mut open.worker);
        let append = Append::new(Author::Agent, &Record::Open(open));
        db.create(&append, &Writer::new(Some("worker"))).unwrap();
    }

    fn titles(read: &Snapshot) -> Vec<String> {
        match read {
            Snapshot::Read { cases, .. } => {
                cases.iter().map(|(_, c)| c.title().to_string()).collect()
            }
            other => panic!("not a read: {other:?}"),
        }
    }

    /// A read asked for lands on its own, and a wait for a later one passes
    /// over any read asked before it, which may be from before a write.
    #[test]
    fn a_read_lands_and_a_wait_takes_only_one_asked_after() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(AnyStore::Local(Db::new(dir.path().join("cases.db"))));
        let mut reader = Reader::start(Arc::clone(&db), Writer::new(None), None).unwrap();
        let stop = AtomicBool::new(false);
        assert!(!reader.busy());
        reader.ask();
        assert!(reader.busy());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let first = loop {
            if let Some(read) = reader.take() {
                break read;
            }
            assert!(std::time::Instant::now() < deadline, "no read landed");
            thread::sleep(Duration::from_millis(10));
        };
        // A store not made yet has no cases.
        assert!(titles(&first).is_empty());
        assert!(!reader.busy());

        reader.ask();
        open(&db, "Written");
        let after = reader.ask();
        let read = reader
            .wait(after, &stop, Duration::from_millis(10))
            .unwrap();
        assert_eq!(titles(&read), ["Written"]);
        assert!(!reader.busy());

        // A write is sent in order with the reads, and how it went is given
        // once the read after it is taken.
        let mut answer = cases_core::Answer::default();
        answer.ack = true;
        let key = match &read {
            Snapshot::Read { cases, .. } => cases[0].0,
            other => panic!("not a read: {other:?}"),
        };
        let before = reader.ask();
        let n = reader.write(Write::Append {
            key,
            record: Record::Answer(answer),
            revision: 1,
        });
        assert!(n > before);
        assert_eq!(reader.wrote(), None);
        let read = reader.wait(n, &stop, Duration::from_millis(10)).unwrap();
        match &read {
            Snapshot::Read { cases, .. } => {
                assert_eq!(cases[0].1.state(), cases_core::State::Answered);
            }
            other => panic!("not a read: {other:?}"),
        }
        assert_eq!(reader.wrote(), Some(Wrote::Append(Ok(()))));
        assert_eq!(reader.wrote(), None, "given twice");

        // A stop ends a wait for a read that never comes.
        stop.store(true, Ordering::Relaxed);
        assert!(
            reader
                .wait(n + 1, &stop, Duration::from_millis(10))
                .is_none()
        );
        reader.finish();
        assert_eq!(Arc::strong_count(&db), 1, "the thread kept the store");
    }
}
