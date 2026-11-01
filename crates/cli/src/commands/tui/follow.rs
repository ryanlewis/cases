//! How the TUI follows a remote or forwarded store (TU-04): a thread of its
//! own holds the store's poll, and says when the version moved, so that the
//! screen reads the cases that changed then rather than once a second, and
//! a slow link does not hold up the screen while nothing changes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use cases_core::Version;
use cases_store_remote::{HOLD_MAX, RemoteStore};

/// The first and the longest pause after a held poll fails, before the
/// next; it doubles between them, and starts again at a poll that works.
const PAUSE: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));

/// The thread that holds a remote or forwarded store's poll. The screen gives it the
/// store and the version after each read that works, and asks it whether
/// it woke since.
///
/// The thread is never joined. Dropping this ends it at its next look for
/// a version, and a poll it holds then does not keep the process from
/// ending: quitting neither waits for it nor needs it to answer.
#[derive(Debug)]
pub(crate) struct Follow {
    reads: Sender<(RemoteStore, Version)>,
    woke: Arc<AtomicBool>,
}

impl Follow {
    /// Starts the thread, which holds no poll until it is given a version,
    /// or `None` when no thread can be started: the screen then reads the
    /// store about once a second, as it does a local one.
    pub(crate) fn start() -> Option<Follow> {
        let (reads, read) = mpsc::channel();
        let woke = Arc::new(AtomicBool::new(false));
        let waking = Arc::clone(&woke);
        thread::Builder::new()
            .name("cases-follow".into())
            .spawn(move || follow(&read, &waking, HOLD_MAX))
            .ok()?;
        Some(Follow { reads, woke })
    }

    /// Holds the poll on `store` from `version`, the version the screen
    /// read at, in place of any earlier one.
    pub(crate) fn hold(&self, store: RemoteStore, version: Version) {
        let _ = self.reads.send((store, version));
    }

    /// Whether the store's version moved, or a poll failed, since this was
    /// last asked: either way the screen reads the store again.
    pub(crate) fn woke(&self) -> bool {
        self.woke.swap(false, Ordering::Relaxed)
    }
}

/// The thread: holds the poll from the latest version the screen gave, and
/// wakes the screen when the version moves or a poll fails, then waits for
/// the next version, which the screen gives after it reads. A poll that
/// fails is followed by a pause, so that a server that keeps failing is not
/// asked over and over. It ends when the screen is gone.
fn follow(read: &Receiver<(RemoteStore, Version)>, woke: &AtomicBool, hold: Duration) {
    let mut held: Option<(RemoteStore, Version)> = None;
    let mut pause = Duration::ZERO;
    loop {
        // The latest the screen gave, waiting for one when there is none.
        let given = match &held {
            None => read.recv().ok(),
            Some(_) => match read.try_recv() {
                Ok(given) => Some(given),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            },
        };
        match (given, &held) {
            (Some(given), _) => held = Some(given),
            (None, None) => return,
            (None, Some(_)) => {}
        }
        let Some((store, version)) = &held else {
            continue;
        };
        match store.changes(version, hold) {
            Ok(now) if now == *version => pause = Duration::ZERO,
            Ok(_) => {
                pause = Duration::ZERO;
                woke.store(true, Ordering::Relaxed);
                held = None;
            }
            Err(_) => {
                woke.store(true, Ordering::Relaxed);
                held = None;
                let (first, most) = PAUSE;
                pause = (pause * 2).clamp(first, most);
                // The whole pause, even when the screen reads again at
                // once, then the latest version it gave.
                thread::sleep(pause);
                loop {
                    match read.try_recv() {
                        Ok(given) => held = Some(given),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            }
        }
    }
}

// The remote store crate's fake server, shared by path as the CLI's
// integration tests share it.
#[cfg(test)]
#[cfg(feature = "remote")]
#[path = "../../../../store-remote/tests/fake/mod.rs"]
mod fake;

#[cfg(test)]
#[cfg(feature = "remote")]
mod tests {
    use std::time::Instant;

    use cases_core::{Append, Author, Open, Record, Store as _, Writer};
    use cases_store_remote::Client;

    use super::fake::{Fake, Fault, TOKEN};
    use super::*;

    /// A fake with one store, and the store.
    fn setup() -> (Fake, RemoteStore) {
        let fake = Fake::start();
        let id = fake.add_store("me");
        let client = Client::new(fake.url().parse().unwrap(), TOKEN.parse().unwrap());
        (fake, client.store(&id).unwrap())
    }

    /// The held polls the fake has been sent.
    fn holds(fake: &Fake) -> usize {
        fake.lines()
            .iter()
            .filter(|line| line.contains("/store/changes?after="))
            .count()
    }

    /// Waits up to `within` for `done`, and gives whether it came.
    fn soon(within: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        done()
    }

    fn open(store: &RemoteStore) {
        let mut open = Open::default();
        "fyi".clone_into(&mut open.kind);
        "today".clone_into(&mut open.urgency);
        "Moved".clone_into(&mut open.title);
        let append = Append::new(Author::Agent, &Record::Open(open));
        store.create(&append, &Writer::new(None)).unwrap();
    }

    #[test]
    fn a_change_wakes_the_screen_within_the_hold() {
        let (fake, store) = setup();
        let follow = Follow::start().unwrap();
        assert!(!follow.woke());
        follow.hold(store.clone(), store.version().unwrap());
        assert!(soon(Duration::from_secs(5), || holds(&fake) == 1));
        thread::sleep(Duration::from_millis(100));
        assert!(!follow.woke(), "it woke with nothing changed");

        let start = Instant::now();
        open(&store);
        assert!(soon(Duration::from_secs(2), || follow.woke()));
        assert!(start.elapsed() < Duration::from_secs(2));
        // It holds no poll again until the screen has read.
        thread::sleep(Duration::from_millis(200));
        assert_eq!(holds(&fake), 1);
        follow.hold(store.clone(), store.version().unwrap());
        assert!(soon(Duration::from_secs(5), || holds(&fake) == 2));
    }

    #[test]
    fn a_failed_poll_wakes_the_screen_to_read_again() {
        let (fake, store) = setup();
        let version = store.version().unwrap();
        for _ in 0..3 {
            fake.fault(Fault::Status(500));
        }
        let follow = Follow::start().unwrap();
        follow.hold(store, version);
        assert!(soon(Duration::from_secs(5), || follow.woke()));
    }

    #[test]
    fn dropping_it_does_not_wait_for_a_held_poll() {
        let (fake, store) = setup();
        let follow = Follow::start().unwrap();
        follow.hold(store.clone(), store.version().unwrap());
        assert!(soon(Duration::from_secs(5), || holds(&fake) == 1));
        let start = Instant::now();
        drop(follow);
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    /// The thread, run with `hold` as its longest hold: what the screen
    /// gives it, what it wakes, and the thread.
    fn run(
        hold: Duration,
    ) -> (
        Sender<(RemoteStore, Version)>,
        Arc<AtomicBool>,
        thread::JoinHandle<()>,
    ) {
        let (reads, read) = mpsc::channel();
        let woke = Arc::new(AtomicBool::new(false));
        let waking = Arc::clone(&woke);
        let thread = thread::spawn(move || follow(&read, &waking, hold));
        (reads, woke, thread)
    }

    #[test]
    fn an_unchanged_poll_is_held_again_without_waking_the_screen() {
        let (fake, store) = setup();
        let (reads, woke, _thread) = run(Duration::from_secs(1));
        reads
            .send((store.clone(), store.version().unwrap()))
            .unwrap();
        assert!(soon(Duration::from_secs(5), || holds(&fake) >= 2));
        assert!(
            !woke.load(Ordering::Relaxed),
            "it woke with nothing changed"
        );
    }

    #[test]
    fn a_version_given_while_a_poll_is_held_is_held_next() {
        let (fake, store) = setup();
        let before = store.version().unwrap();
        open(&store);
        let (reads, woke, _thread) = run(Duration::from_secs(1));
        reads
            .send((store.clone(), store.version().unwrap()))
            .unwrap();
        assert!(soon(Duration::from_secs(5), || holds(&fake) == 1));
        // The store has moved on from the version given now: the next poll
        // says so at once.
        reads.send((store, before)).unwrap();
        assert!(soon(Duration::from_secs(5), || woke.load(Ordering::Relaxed)));
    }

    #[test]
    fn it_ends_when_the_screen_is_gone() {
        // Before any version was given.
        let (reads, _woke, thread) = run(Duration::from_secs(1));
        drop(reads);
        assert!(soon(Duration::from_secs(2), || thread.is_finished()));

        // While it holds a poll: at the end of the hold.
        let (fake, store) = setup();
        let (reads, _woke, thread) = run(Duration::from_secs(1));
        reads
            .send((store.clone(), store.version().unwrap()))
            .unwrap();
        assert!(soon(Duration::from_secs(5), || holds(&fake) == 1));
        drop(reads);
        assert!(soon(Duration::from_secs(3), || thread.is_finished()));
        assert_eq!(holds(&fake), 1);

        // While it pauses after a poll that failed: at the end of the pause.
        let version = store.version().unwrap();
        fake.fault(Fault::Status(500));
        let (reads, woke, thread) = run(Duration::from_secs(1));
        reads.send((store, version)).unwrap();
        assert!(soon(Duration::from_secs(5), || woke.load(Ordering::Relaxed)));
        drop(reads);
        assert!(soon(Duration::from_secs(3), || thread.is_finished()));
        assert_eq!(holds(&fake), 2);
    }

    /// After a poll fails it pauses, twice as long each time it fails
    /// again, and then holds the latest version given while it paused.
    #[test]
    fn a_failed_poll_pauses_longer_each_time_then_holds_what_was_given() {
        let (fake, store) = setup();
        let version = store.version().unwrap();
        fake.fault(Fault::Status(500));
        fake.fault(Fault::Status(500));
        let (reads, woke, _thread) = run(Duration::from_secs(1));
        reads.send((store.clone(), version.clone())).unwrap();
        let mut pauses = Vec::new();
        for _ in 0..2 {
            assert!(soon(Duration::from_secs(5), || woke
                .swap(false, Ordering::Relaxed)));
            let failed = Instant::now();
            let polls = holds(&fake);
            // Given while it pauses, as the screen reads at once.
            reads.send((store.clone(), version.clone())).unwrap();
            assert!(soon(Duration::from_secs(5), || holds(&fake) > polls));
            pauses.push(failed.elapsed());
        }
        assert!(pauses[0] >= Duration::from_millis(900), "{pauses:?}");
        assert!(pauses[1] >= Duration::from_millis(1900), "{pauses:?}");
        assert!(
            !woke.load(Ordering::Relaxed),
            "the poll after the pauses failed"
        );
    }
}
