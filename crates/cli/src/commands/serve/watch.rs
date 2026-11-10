//! The store as serve follows it (IN-06, IN-08).
//!
//! Every read goes through one poller, which folds again only the cases that
//! changed. A read that finds the store moved publishes a new snapshot, which
//! wakes the event streams, and shows the notifier the cases, so that what
//! landed on the human is queued for the browser. serve reads the store about
//! once a second by itself, and each API read reads it again, so a page never
//! sees older cases than the stream told it of.
//!
//! The poller runs on a thread of its own, the reader, which holds the store
//! for as long as the watch lives: a poller borrows its store, and the
//! reader's thread is where that borrow can live. When the watch drops, the
//! reader stops, and the store's connection closes with serve's other one,
//! so SQLite folds its write-ahead log back into the store's file.

use std::collections::HashSet;
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use cases_core::{Case, Refolded};
use cases_store_sqlite::{Cases, Db, Key, Poller};
use tokio::sync::watch;

use super::log::Log;
use super::notify::{Engine, Feed, Item};
use crate::instance;
use crate::output::{self, Actors, CaseJson};

/// The store's cases as one read found them, and their version.
#[derive(Debug)]
pub(crate) struct Snapshot {
    /// The cases that fold, in the order they were opened. A case the
    /// poller did not fold again since the snapshot before is the one that
    /// snapshot held, shared, not a copy. After a read of every case, such
    /// as one after a failed read, so is each case equal to the one that
    /// snapshot held; the rest are new copies.
    pub(crate) cases: Vec<(Key, Arc<Case>)>,
    /// The store's version, as the poller read it with the cases, in the
    /// same read transaction: so it names this moment of the store and no
    /// other. Every event written moves it, as does a case taken out of the
    /// store or put back, or another store put at the path, even one copied
    /// from this one whose cases have the same keys, open times and
    /// revisions, where the copy's last row was written at another time: the
    /// store's version is a guard there, not a proof (see
    /// `cases_store_sqlite`'s `version`). It is [`MISSING`] while there is
    /// no store. It is the same in every run of serve (IN-06).
    pub(crate) version: String,
}

/// The version of a store that is not there, or not read yet: never a
/// store's own, which has its parts joined by dots.
const MISSING: &str = "missing";

/// What a read gives.
type Read = Result<Arc<Snapshot>, cases_store_sqlite::Error>;

/// The store serve follows, and what it learns on each read.
#[derive(Debug)]
pub(crate) struct Watch {
    /// Asks the reader for a read: each request is the channel its answer
    /// goes back on. `None` once the watch has stopped the reader.
    requests: Option<mpsc::Sender<mpsc::SyncSender<Read>>>,
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
}

/// What the watch and its reader share.
#[derive(Debug)]
struct Shared {
    snapshots: watch::Sender<Arc<Snapshot>>,
    feed: Mutex<Feed>,
}

/// The reader: the poller, and what the reads learn, on the reader's
/// thread.
struct Reader<'a> {
    db: &'a Db,
    poller: Poller<'a>,
    /// The inbox's address, for the `url` of the case in a notification.
    url: String,
    log: Arc<Log>,
    shared: Arc<Shared>,
    /// Whether a read has found the store, or found it missing.
    read: bool,
    engine: Engine,
    /// The warnings logged, each once: a case that does not fold is read
    /// again on every change.
    warned: HashSet<String>,
    /// The keys of the cases folded again since the snapshot published
    /// last, by every poll since, which the next snapshot holds anew; the
    /// rest it shares with that snapshot. `None` when every case was, so
    /// that the next snapshot shares only the cases that came out equal.
    refolded: Option<HashSet<Key>>,
}

impl Watch {
    /// Follows the store `db` for the inbox at `url`, with `boot` as the
    /// notification feed's boot id. Until the first read, the store counts
    /// as empty.
    ///
    /// # Errors
    ///
    /// When the reader's thread cannot be started.
    pub(crate) fn new(db: Arc<Db>, boot: String, url: String, log: Arc<Log>) -> io::Result<Watch> {
        let empty = Snapshot {
            cases: Vec::new(),
            version: MISSING.into(),
        };
        let shared = Arc::new(Shared {
            snapshots: watch::Sender::new(Arc::new(empty)),
            feed: Mutex::new(Feed::new(boot)),
        });
        let (requests, received) = mpsc::channel::<mpsc::SyncSender<Read>>();
        let reading = Arc::clone(&shared);
        let reader = thread::Builder::new().name("store".into()).spawn(move || {
            let mut reader = Reader {
                db: &db,
                poller: db.poller(),
                url,
                log,
                shared: reading,
                read: false,
                engine: Engine::default(),
                warned: HashSet::new(),
                refolded: None,
            };
            for answer in received {
                // A reader that asked and went away needs no answer.
                let _ = answer.send(reader.read());
            }
        })?;
        Ok(Watch {
            requests: Some(requests),
            shared,
            reader: Some(reader),
        })
    }

    /// Reads the store: the snapshot as it is now. A store not made yet has
    /// no cases. It waits for the store, so an async caller runs it on a
    /// blocking thread.
    ///
    /// # Errors
    ///
    /// When the store fails. The snapshot then stays as it was.
    pub(crate) fn read(&self) -> Read {
        let stopped = || {
            cases_store_sqlite::Error::Io(io::Error::other(
                "serve's reader of the store has stopped",
            ))
        };
        let (answer, answered) = mpsc::sync_channel(1);
        self.requests
            .as_ref()
            .ok_or_else(stopped)?
            .send(answer)
            .map_err(|_| stopped())?;
        answered.recv().map_err(|_| stopped())?
    }

    /// Each snapshot from now on, as reads publish them.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.shared.snapshots.subscribe()
    }

    /// The notifications after `after`, as the feed's JSON.
    ///
    /// # Errors
    ///
    /// When the page does not serialize, which it always does.
    pub(crate) fn notifications(&self, after: u64) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(&self.shared.feed().after(after))
    }

    /// How many notifications have been queued since serve started.
    pub(crate) fn notified(&self) -> u64 {
        self.shared.feed().latest()
    }
}

impl Drop for Watch {
    /// Stops the reader and waits for it, so that the store's connection it
    /// holds is closed once the watch is gone.
    fn drop(&mut self) {
        drop(self.requests.take());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Shared {
    fn feed(&self) -> MutexGuard<'_, Feed> {
        // A reader that panicked while it queued has left what it queued.
        self.feed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Reader<'_> {
    /// Reads the store, and publishes what moved.
    fn read(&mut self) -> Read {
        let polled = self.poller.poll().map(|_| ());
        // Taken from a failed poll too, which folds every case again next.
        match self.poller.refolded() {
            Refolded::Changed { folded, .. } => {
                if let Some(refolded) = &mut self.refolded {
                    refolded.extend(folded);
                }
            }
            _ => self.refolded = None,
        }
        let missing = match polled {
            Ok(()) => false,
            Err(cases_store_sqlite::Error::Missing { .. }) => true,
            Err(error) => return Err(error),
        };
        // A poll that fails leaves the poller with no cases.
        let cases = self.poller.cases();
        // The version the poll read the cases at, not a second read of the
        // store, which could find it moved since.
        let version = match self.poller.version() {
            Some(version) => version.to_string(),
            None => MISSING.into(),
        };
        let current = self.shared.snapshots.borrow().clone();
        if self.read && version == current.version {
            return Ok(current);
        }
        for (key, case) in cases {
            if let Err(error) = case {
                let warning = format!("warning: {key}: {error}");
                if self.warned.insert(warning.clone()) {
                    self.log.line(warning);
                }
            }
        }
        // A store that goes missing leaves the engine as it was: were the
        // file put back, its cases would all be new again. A first read
        // that finds none still counts, so the first case to come is new.
        if !missing || !self.read {
            let landed = self.engine.observe(cases);
            if !landed.is_empty() {
                queue(self.db, &self.url, &self.shared, cases, &landed);
            }
        }
        self.read = true;
        // Each case the poller did not fold again since the last snapshot
        // is the one that snapshot holds: shared, rather than copied with
        // all its events, and not looked at. When every case was folded
        // again since that snapshot, one that came out equal is shared too:
        // comparing it allocates nothing, where a copy clones its events.
        let refolded = self.refolded.replace(HashSet::new());
        // Both the snapshot and the poller's cases are in key order.
        let mut before = current.cases.iter().peekable();
        let snapshot = Arc::new(Snapshot {
            cases: cases
                .iter()
                .filter_map(|(key, case)| {
                    let case = case.as_ref().ok()?;
                    while before.next_if(|(was, _)| was < key).is_some() {}
                    let was = before.next_if(|(was, _)| was == key).map(|(_, was)| was);
                    let kept = |was: &Case| match &refolded {
                        Some(refolded) => !refolded.contains(key),
                        None => was == case,
                    };
                    let shared = match was {
                        Some(was) if kept(was) => Arc::clone(was),
                        _ => Arc::new(case.clone()),
                    };
                    Some((*key, shared))
                })
                .collect(),
            version,
        });
        self.shared.snapshots.send_replace(Arc::clone(&snapshot));
        Ok(snapshot)
    }
}

/// Queues a notification for each case in `landed`, with the case as
/// `show --json` gives it, its page at the inbox at `url`.
fn queue(db: &Db, url: &str, shared: &Shared, cases: &Cases, landed: &[super::notify::Landed]) {
    // Without a clock inside 1970 to 9999 there is no time to queue at.
    let Ok(now) = output::now() else { return };
    let actors = Actors::read(db).unwrap_or_default();
    let mut feed = shared.feed();
    for landed in landed {
        let Some(Ok(case)) = cases.get(&landed.key) else {
            continue;
        };
        let Some(event) = case.events().iter().find(|event| event.seq == landed.seq) else {
            continue;
        };
        let page = instance::case_url(url, &output::titled(landed.key, case.title()));
        let json = CaseJson::new(landed.key, case, &actors)
            .ok()
            .and_then(|json| serde_json::to_value(json.with_url(page)).ok())
            .unwrap_or_default();
        feed.add(Item::new(landed.key, case, event, landed.name, now, json));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cases_core::{Append, Changes, Store, Version};
    use cases_store_sqlite::Writer;

    use super::*;

    fn watch(path: &std::path::Path) -> (Arc<Db>, Watch) {
        watch_db(Db::new(path))
    }

    fn watch_db(db: Db) -> (Arc<Db>, Watch) {
        let db = Arc::new(db);
        let log = Arc::new(Log::new(false, 0));
        let url = "http://127.0.0.1:7337/".into();
        let watch = Watch::new(Arc::clone(&db), "boot".into(), url, log).unwrap();
        (db, watch)
    }

    fn fyi(db: &Db) -> Key {
        let open = Append {
            author: "agent".into(),
            event: "open".into(),
            data: r#"{"kind":"fyi","urgency":"today","title":"Heads up"}"#.into(),
        };
        db.create(&open, &Writer::new(None)).unwrap().0
    }

    /// A clock stopped at `at`, so that a store writes the time the test
    /// chooses. Two stores amended apart need times of their own: at the
    /// same time, they have the same version (a limit the store documents).
    fn at(at: &'static str) -> impl Fn() -> cases_core::Time + Send + Sync + 'static {
        move || at.parse().unwrap()
    }

    fn question(db: &Db, body: &str) -> Key {
        let open = Append {
            author: "agent".into(),
            event: "open".into(),
            data: format!(
                r#"{{"kind":"question","urgency":"today","title":"Which?","body":"{body}"}}"#
            ),
        };
        db.create(&open, &Writer::new(None)).unwrap().0
    }

    fn amend(db: &Db, key: Key, body: &str) {
        let amend = Append {
            author: "agent".into(),
            event: "amend".into(),
            data: format!(r#"{{"body":"{body}"}}"#),
        };
        db.append(&key, &amend, &Writer::new(None), None).unwrap();
    }

    fn body(snapshot: &Snapshot) -> Vec<(Key, u64, String)> {
        snapshot
            .cases
            .iter()
            .map(|(key, case)| (*key, case.revision(), case.body().to_owned()))
            .collect()
    }

    /// A copy of the store, amended apart from it, put at its path: the
    /// copy's case has the same key, open time and revision as the case
    /// serve read, and another body. The next read shows the copy's body,
    /// and publishes it (finding 1 of the code-health review of 1 Nov).
    #[test]
    fn a_copy_of_the_store_amended_apart_and_put_at_its_path_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let copy = dir.path().join("copy.db");
        let db = Db::new(&path).with_clock(at("2026-11-01T09:00:00Z"));
        let key = question(&db, "original");
        // A consistent copy, as `sqlite3 .backup` makes, with the store's
        // id, keys and change counter.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("VACUUM INTO ?1", [copy.to_str().unwrap()])
            .unwrap();
        // Both amended at a time of their own, as two writers would.
        let other = Db::new(&copy).with_clock(at("2026-11-01T09:05:00Z"));
        amend(&other, key, "branch B");
        drop(other);
        let db = db.with_clock(at("2026-11-01T09:04:00Z"));
        amend(&db, key, "branch A");

        let (_db, watch) = watch_db(db);
        let mut snapshots = watch.subscribe();
        let before = watch.read().unwrap();
        assert_eq!(body(&before), [(key, 2, "branch A".into())]);
        snapshots.borrow_and_update();

        // The store, its log and its shared memory moved aside together,
        // and the copy put at the path, all before the next read.
        for suffix in ["", "-wal", "-shm"] {
            let from = dir.path().join(format!("cases.db{suffix}"));
            if from.exists() {
                std::fs::rename(&from, dir.path().join(format!("aside.db{suffix}"))).unwrap();
            }
        }
        std::fs::rename(&copy, &path).unwrap();

        let after = watch.read().unwrap();
        assert_eq!(body(&after), [(key, 2, "branch B".into())]);
        assert_ne!(after.version, before.version);
        assert!(
            snapshots.has_changed().unwrap(),
            "the copy was not published"
        );
        assert_eq!(body(&snapshots.borrow_and_update()), body(&after));
    }

    /// A read after a write shares each case that did not change with the
    /// snapshot before, and holds the changed one anew; a case added is
    /// new, and a case taken out is gone.
    #[test]
    fn a_read_shares_the_cases_that_did_not_change() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("cases.db"));
        let keys: Vec<Key> = (0..3)
            .map(|n| question(&db, &format!("body {n}")))
            .collect();
        let before = watch.read().unwrap();
        amend(&db, keys[1], "amended");
        let after = watch.read().unwrap();
        let shared = |snapshot: &Snapshot, key: Key| {
            let (_, case) = snapshot.cases.iter().find(|(k, _)| *k == key).unwrap();
            Arc::clone(case)
        };
        assert!(Arc::ptr_eq(
            &shared(&before, keys[0]),
            &shared(&after, keys[0])
        ));
        assert!(Arc::ptr_eq(
            &shared(&before, keys[2]),
            &shared(&after, keys[2])
        ));
        assert!(!Arc::ptr_eq(
            &shared(&before, keys[1]),
            &shared(&after, keys[1])
        ));
        assert_eq!(shared(&after, keys[1]).body(), "amended");
        let added = fyi(&db);
        db.archive(&keys[0], None).unwrap();
        let last = watch.read().unwrap();
        assert!(Arc::ptr_eq(
            &shared(&after, keys[1]),
            &shared(&last, keys[1])
        ));
        assert!(Arc::ptr_eq(
            &shared(&after, keys[2]),
            &shared(&last, keys[2])
        ));
        assert!(!last.cases.iter().any(|(key, _)| *key == keys[0]));
        assert_eq!(shared(&last, added).title(), "Heads up");
    }

    /// A read that publishes nothing leaves the next one sharing what did
    /// not change since the snapshot published last. A store gone missing
    /// and put back is read anew, sharing nothing with the snapshot from
    /// before, and the read after it shares again.
    #[test]
    fn a_read_shares_across_reads_that_publish_nothing_and_none_after_a_missing_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let (db, watch) = watch(&path);
        let keys: Vec<Key> = (0..2)
            .map(|n| question(&db, &format!("body {n}")))
            .collect();
        let shared = |snapshot: &Snapshot, key: Key| {
            let (_, case) = snapshot.cases.iter().find(|(k, _)| *k == key).unwrap();
            Arc::clone(case)
        };
        let first = watch.read().unwrap();
        assert!(Arc::ptr_eq(&watch.read().unwrap(), &first));
        amend(&db, keys[1], "amended");
        let second = watch.read().unwrap();
        assert!(Arc::ptr_eq(
            &shared(&first, keys[0]),
            &shared(&second, keys[0])
        ));
        assert!(!Arc::ptr_eq(
            &shared(&first, keys[1]),
            &shared(&second, keys[1])
        ));

        let aside = dir.path().join("aside.db");
        std::fs::rename(&path, &aside).unwrap();
        assert!(watch.read().unwrap().cases.is_empty());
        std::fs::rename(&aside, &path).unwrap();
        let back = watch.read().unwrap();
        assert_eq!(body(&back), body(&second));
        for key in &keys {
            assert!(!Arc::ptr_eq(&shared(&second, *key), &shared(&back, *key)));
        }

        amend(&db, keys[0], "again");
        let last = watch.read().unwrap();
        assert!(Arc::ptr_eq(
            &shared(&back, keys[1]),
            &shared(&last, keys[1])
        ));
        assert_eq!(shared(&last, keys[0]).body(), "again");
    }

    /// How many of the cases in `after` are the very ones `before` holds.
    fn shared_with(before: &Snapshot, after: &Snapshot) -> usize {
        after
            .cases
            .iter()
            .filter(|(_, case)| before.cases.iter().any(|(_, was)| Arc::ptr_eq(was, case)))
            .count()
    }

    thread_local! {
        /// What the next read of a [`scripted`] reader gives, in place of
        /// the store's answer.
        static SCRIPT: RefCell<Option<Result<Changes<Key>, cases_store_sqlite::Error>>> =
            const { RefCell::new(None) };
    }

    /// A reader of `db` on this thread, whose next poll gives [`SCRIPT`]
    /// when it is set, and the store's answer otherwise.
    fn scripted(db: &Db) -> Reader<'_> {
        SCRIPT.set(None);
        let empty = Snapshot {
            cases: Vec::new(),
            version: MISSING.into(),
        };
        Reader {
            db,
            poller: Poller::with_read(db, |db, since| {
                SCRIPT.take().unwrap_or_else(|| db.rows_since(since))
            }),
            url: "http://127.0.0.1:7337/".into(),
            log: Arc::new(Log::new(false, 0)),
            shared: Arc::new(Shared {
                snapshots: watch::Sender::new(Arc::new(empty)),
                feed: Mutex::new(Feed::new("boot".into())),
            }),
            read: false,
            engine: Engine::default(),
            warned: HashSet::new(),
            refolded: None,
        }
    }

    /// A read the store answers with every case, here because an older copy
    /// of it was put at its path, shares each case that came out equal to
    /// the one the snapshot before held, and holds the one that differs
    /// anew.
    #[test]
    fn a_read_of_every_case_shares_the_cases_that_are_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let copy = dir.path().join("copy.db");
        let (db, watch) = watch(&path);
        let keys: Vec<Key> = (0..3)
            .map(|n| question(&db, &format!("body {n}")))
            .collect();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("VACUUM INTO ?1", [copy.to_str().unwrap()])
            .unwrap();
        amend(&db, keys[1], "amended");
        let before = watch.read().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let from = dir.path().join(format!("cases.db{suffix}"));
            if from.exists() {
                std::fs::rename(&from, dir.path().join(format!("aside.db{suffix}"))).unwrap();
            }
        }
        std::fs::rename(&copy, &path).unwrap();
        let after = watch.read().unwrap();
        assert_ne!(after.version, before.version);
        assert_eq!(after.cases[1].1.body(), "body 1", "the copy was not read");
        assert_eq!(shared_with(&before, &after), 2);
        assert!(!Arc::ptr_eq(&before.cases[1].1, &after.cases[1].1));
    }

    /// A poll that fails on a store that has not moved costs the next
    /// snapshot nothing: the read after it publishes nothing, and the one
    /// after a write still shares every case but the one written to.
    #[test]
    fn a_failed_read_of_an_unmoved_store_leaves_the_next_snapshot_sharing() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let keys: Vec<Key> = (0..3)
            .map(|n| question(&db, &format!("body {n}")))
            .collect();
        let mut reader = scripted(&db);
        let first = reader.read().unwrap();
        SCRIPT.set(Some(Err(cases_store_sqlite::Error::Io(io::Error::other(
            "a passing failure",
        )))));
        assert!(reader.read().is_err());
        assert!(Arc::ptr_eq(&reader.read().unwrap(), &first));
        amend(&db, keys[1], "amended");
        let next = reader.read().unwrap();
        assert_eq!(shared_with(&first, &next), 2);
        assert_eq!(next.cases[1].1.body(), "amended");
    }

    /// The next snapshot holds anew every case folded again since the last
    /// one, by every poll since: here one that published nothing, as a read
    /// may answer, and one that did.
    #[test]
    fn a_snapshot_holds_anew_the_cases_every_poll_since_the_last_folded() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let keys: Vec<Key> = (0..3)
            .map(|n| question(&db, &format!("body {n}")))
            .collect();
        let mut reader = scripted(&db);
        let first = reader.read().unwrap();
        let Changes::All { cases, .. } = db.rows_since(None).unwrap() else {
            panic!("not every case");
        };
        SCRIPT.set(Some(Ok(Changes::Since {
            version: Version::new(first.version.clone()),
            cases: cases
                .into_iter()
                .filter(|(key, _)| *key == keys[0])
                .collect(),
            removed: Vec::new(),
        })));
        assert!(Arc::ptr_eq(&reader.read().unwrap(), &first));
        amend(&db, keys[1], "amended");
        let next = reader.read().unwrap();
        assert!(!Arc::ptr_eq(&first.cases[0].1, &next.cases[0].1));
        assert!(!Arc::ptr_eq(&first.cases[1].1, &next.cases[1].1));
        assert!(Arc::ptr_eq(&first.cases[2].1, &next.cases[2].1));
    }

    /// A copy of the store put at its path whose case went apart and then
    /// took the very same last row, at the very same time, as the case serve
    /// read: the case has the same counts, open time and last event, and
    /// another history. The next read shows the copy's case, not the one
    /// the snapshot before held.
    #[test]
    fn a_copy_whose_case_went_apart_and_ends_alike_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let copy = dir.path().join("copy.db");
        let db = Db::new(&path).with_clock(at("2026-11-01T09:00:00Z"));
        let key = question(&db, "original");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("VACUUM INTO ?1", [copy.to_str().unwrap()])
            .unwrap();
        let note = Append {
            author: "agent".into(),
            event: "note".into(),
            data: r#"{"body":"Same last word."}"#.into(),
        };
        // Apart, then alike, at the same time; and the store serve reads
        // has a case more, so that the copy is behind it and is read whole.
        let other = Db::new(&copy).with_clock(at("2026-11-01T09:05:00Z"));
        amend(&other, key, "branch B");
        let other = other.with_clock(at("2026-11-01T09:10:00Z"));
        other.append(&key, &note, &Writer::new(None), None).unwrap();
        drop(other);
        let db = db.with_clock(at("2026-11-01T09:04:00Z"));
        amend(&db, key, "branch A");
        let db = db.with_clock(at("2026-11-01T09:10:00Z"));
        db.append(&key, &note, &Writer::new(None), None).unwrap();
        fyi(&db);

        let (_db, watch) = watch_db(db);
        let before = watch.read().unwrap();
        assert_eq!(body(&before)[0], (key, 3, "branch A".into()));
        for suffix in ["", "-wal", "-shm"] {
            let from = dir.path().join(format!("cases.db{suffix}"));
            if from.exists() {
                std::fs::rename(&from, dir.path().join(format!("aside.db{suffix}"))).unwrap();
            }
        }
        std::fs::rename(&copy, &path).unwrap();
        let after = watch.read().unwrap();
        assert_ne!(after.version, before.version);
        assert_eq!(body(&after), [(key, 3, "branch B".into())]);
    }

    /// A read that finds the store as it was gives the snapshot it has, and
    /// publishes nothing, so no stream sends a message.
    #[test]
    fn an_unmoved_store_gives_the_same_snapshot_and_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("cases.db"));
        fyi(&db);
        let first = watch.read().unwrap();
        let mut snapshots = watch.subscribe();
        snapshots.borrow_and_update();
        for _ in 0..3 {
            let again = watch.read().unwrap();
            assert!(
                Arc::ptr_eq(&again, &first),
                "a new snapshot of an unmoved store"
            );
        }
        assert!(!snapshots.has_changed().unwrap());
    }

    /// The version is the store's own, as `Store::version` gives it, so it
    /// is the same in every run of serve and every reader of the store
    /// (IN-06).
    #[test]
    fn the_version_is_the_stores() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("cases.db"));
        fyi(&db);
        assert_eq!(
            watch.read().unwrap().version,
            db.version().unwrap().to_string()
        );
    }

    /// A store that goes missing reads as no cases, at a version of its
    /// own, and one put back at the path reads as it was, at its version.
    #[test]
    fn a_missing_store_has_its_own_version_until_it_is_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let (db, watch) = watch(&path);
        assert_eq!(watch.read().unwrap().version, MISSING);
        fyi(&db);
        let there = watch.read().unwrap();
        assert_eq!(there.cases.len(), 1);
        let mut snapshots = watch.subscribe();
        snapshots.borrow_and_update();

        let aside = dir.path().join("aside.db");
        std::fs::rename(&path, &aside).unwrap();
        let gone = watch.read().unwrap();
        assert!(gone.cases.is_empty());
        assert_eq!(gone.version, MISSING);
        assert!(snapshots.has_changed().unwrap());
        snapshots.borrow_and_update();
        assert!(Arc::ptr_eq(&watch.read().unwrap(), &gone));
        assert!(!snapshots.has_changed().unwrap(), "a missing store moved");

        std::fs::rename(&aside, &path).unwrap();
        let back = watch.read().unwrap();
        assert_eq!(back.version, there.version);
        assert_eq!(body(&back), body(&there));
        assert!(snapshots.has_changed().unwrap());
    }

    #[test]
    fn the_version_moves_with_each_event_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("cases.db"));
        let empty = watch.read().unwrap().version.clone();
        let key = fyi(&db);
        let opened = watch.read().unwrap().version.clone();
        assert_ne!(opened, empty);
        assert_eq!(watch.read().unwrap().version, opened, "nothing was written");
        let ack = Append {
            author: "human".into(),
            event: "answer".into(),
            data: r#"{"ack":true}"#.into(),
        };
        db.append(&key, &ack, &Writer::new(None), None).unwrap();
        let answered = watch.read().unwrap().version.clone();
        assert_ne!(answered, opened);

        // Another serve on the store finds the same version.
        let (_, other) = self::watch(&dir.path().join("cases.db"));
        assert_eq!(other.read().unwrap().version, answered);
        // A case taken out of the store moves it on, not back.
        db.archive(&key, None).unwrap();
        let archived = watch.read().unwrap();
        assert!(archived.cases.is_empty());
        assert_ne!(archived.version, answered);
        assert_ne!(archived.version, empty);
    }

    /// Another store put at the path, whose cases have the same keys and
    /// revisions, has another version: keys start again at 1 in a new
    /// store, and a page must not be left showing the old store's cases.
    #[test]
    fn another_store_with_the_same_keys_has_another_version() {
        let versions: Vec<String> = ["2026-09-17T08:00:00Z", "2026-09-17T09:00:00Z"]
            .into_iter()
            .map(|at| {
                let dir = tempfile::tempdir().unwrap();
                let (db, watch) = watch(&dir.path().join("cases.db"));
                let open = Append {
                    author: "agent".into(),
                    event: "open".into(),
                    data: format!(
                        r#"{{"kind":"fyi","urgency":"today","title":"T","opened_at":"{at}"}}"#
                    ),
                };
                let (key, case) = db.create(&open, &Writer::new(None)).unwrap();
                assert_eq!((key.get(), case.revision()), (1, 1));
                watch.read().unwrap().version.clone()
            })
            .collect();
        assert_ne!(versions[0], versions[1]);
    }

    #[test]
    fn a_read_publishes_what_moved_to_its_subscribers() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("cases.db"));
        let mut versions = watch.subscribe();
        watch.read().unwrap();
        versions.borrow_and_update();
        watch.read().unwrap();
        assert!(
            !versions.has_changed().unwrap(),
            "an unmoved store was published"
        );
        fyi(&db);
        let read = watch.read().unwrap();
        assert!(versions.has_changed().unwrap());
        assert_eq!(versions.borrow_and_update().version, read.version);
        assert_eq!(read.cases.len(), 1);
    }

    #[test]
    fn a_missing_store_reads_as_empty_and_its_first_case_notifies() {
        let dir = tempfile::tempdir().unwrap();
        let (db, watch) = watch(&dir.path().join("later").join("cases.db"));
        assert!(watch.read().unwrap().cases.is_empty());
        fyi(&db);
        watch.read().unwrap();
        assert_eq!(watch.notified(), 1);
    }

    /// Once the watch is gone, its reader has let go of the store: with no
    /// connection left open, SQLite folds the write-ahead log back into the
    /// store's file and removes it.
    #[test]
    fn a_watch_that_drops_closes_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let (db, watch) = watch(&path);
        fyi(&db);
        watch.read().unwrap();
        let wal = dir.path().join("cases.db-wal");
        assert!(wal.exists(), "the store keeps a write-ahead log while open");
        drop(watch);
        drop(db);
        assert!(!wal.exists(), "the store was left open");
    }

    #[test]
    fn a_store_put_back_does_not_notify_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let (db, watch) = watch(&path);
        watch.read().unwrap();
        fyi(&db);
        watch.read().unwrap();
        assert_eq!(watch.notified(), 1);
        let aside = dir.path().join("aside.db");
        std::fs::rename(&path, &aside).unwrap();
        watch.read().unwrap();
        std::fs::rename(&aside, &path).unwrap();
        watch.read().unwrap();
        assert_eq!(watch.notified(), 1);
    }
}
