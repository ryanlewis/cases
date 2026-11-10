//! The poller over a store kept in memory: it folds again only the cases
//! that changed, drops those taken out, and reads every case after a failed
//! poll.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod memory;

use std::cell::{Cell, RefCell};

use cases_core::{Append, Cases, Changes, Key, Poller, Refolded, Store, Time, Version, Writer};

use memory::{Fault, Memory};

thread_local! {
    /// How many cases the reads of a [`watched`] poller have given, which
    /// is how many it has folded.
    static FOLDS: Cell<usize> = const { Cell::new(0) };
    /// The version each read was asked about.
    static ASKED: RefCell<Vec<Option<Version>>> = const { RefCell::new(Vec::new()) };
    /// Whether the next read fails.
    static FAIL: Cell<bool> = const { Cell::new(false) };
    /// Whether the reads give every case, as a store that cannot tell what
    /// changed does.
    static ALL: Cell<bool> = const { Cell::new(false) };
}

/// A poller over `store` that reads with `rows_since`, counts the cases it
/// folds, and records the version each read asks about.
fn watched(store: &Memory) -> Poller<'_, Memory> {
    FOLDS.set(0);
    ASKED.with_borrow_mut(Vec::clear);
    FAIL.set(false);
    ALL.set(false);
    Poller::with_read(store, |store, since| {
        ASKED.with_borrow_mut(|asked| asked.push(since.cloned()));
        if FAIL.replace(false) {
            return Err(Fault::Missing);
        }
        let changes = store.rows_since(if ALL.get() { None } else { since })?;
        let n = match &changes {
            Changes::Unchanged { .. } => 0,
            Changes::Since { cases, .. } | Changes::All { cases, .. } => cases.len(),
        };
        FOLDS.set(FOLDS.get() + n);
        Ok(changes)
    })
}

fn folds() -> usize {
    FOLDS.get()
}

fn start() -> Time {
    "2026-09-16T12:00:00Z".parse().unwrap()
}

/// Opens an fyi case.
fn create(store: &Memory) -> Key {
    let open = Append {
        author: "agent".into(),
        event: "open".into(),
        data: r#"{"kind":"fyi","urgency":"today","title":"T"}"#.into(),
    };
    store.create(&open, &Writer::new(None)).unwrap().0
}

/// Writes a note on the case.
fn note(store: &Memory, key: Key) {
    let note = Append {
        author: "agent".into(),
        event: "note".into(),
        data: r#"{"body":"More."}"#.into(),
    };
    store.append(&key, &note, &Writer::new(None), None).unwrap();
}

fn keys(cases: &Cases) -> Vec<Key> {
    cases.keys().copied().collect()
}

/// A poller made with `new` reads with `rows_since`.
#[test]
fn a_new_poller_reads_with_rows_since() {
    let store = Memory::new(start());
    let first = create(&store);
    let mut poller = Poller::new(&store);
    assert_eq!(keys(poller.poll().unwrap()), vec![first]);
    let second = create(&store);
    note(&store, first);
    let cases = poller.poll().unwrap();
    assert_eq!(keys(cases), vec![first, second]);
    assert_eq!(cases[&first].as_ref().unwrap().revision(), 2);
}

/// With nothing written, a poll folds nothing; a write to one case, or a
/// new case, is folded alone; a case taken out is dropped unfolded.
#[test]
fn a_poll_folds_again_only_the_cases_that_changed() {
    let store = Memory::new(start());
    let first = create(&store);
    let second = create(&store);
    let mut poller = watched(&store);
    assert_eq!(poller.poll().unwrap().len(), 2);
    assert_eq!(folds(), 2);
    poller.poll().unwrap();
    assert_eq!(folds(), 2, "an unchanged store was folded again");
    note(&store, second);
    let cases = poller.poll().unwrap();
    assert_eq!(cases[&second].as_ref().unwrap().revision(), 2);
    assert_eq!(cases[&first].as_ref().unwrap().revision(), 1);
    assert_eq!(folds(), 3);
    let third = create(&store);
    assert_eq!(keys(poller.poll().unwrap()), vec![first, second, third]);
    assert_eq!(folds(), 4);
    store.delete(&first, None).unwrap();
    assert_eq!(keys(poller.poll().unwrap()), vec![second, third]);
    store.archive(&third, None).unwrap();
    assert_eq!(keys(poller.poll().unwrap()), vec![second]);
    assert_eq!(folds(), 4);
}

/// Each poll asks about the version the last one read at, which the poller
/// gives.
#[test]
fn a_poll_asks_what_changed_since_the_last_read() {
    let store = Memory::new(start());
    create(&store);
    let mut poller = watched(&store);
    assert_eq!(poller.version(), None);
    assert!(poller.cases().is_empty());
    let polled = poller.poll().unwrap().clone();
    let then = store.version().unwrap();
    assert_eq!(poller.version(), Some(&then));
    assert_eq!(keys(poller.cases()), keys(&polled));
    assert_eq!(poller.cases().len(), 1);
    create(&store);
    poller.poll().unwrap();
    let now = store.version().unwrap();
    assert_eq!(poller.version(), Some(&now));
    poller.poll().unwrap();
    let asked = ASKED.with_borrow(Clone::clone);
    assert_eq!(asked, vec![None, Some(then), Some(now)]);
}

/// When the store gives every case, the poller drops the cases it had that
/// are not among them.
#[test]
fn a_poll_given_every_case_drops_the_ones_gone() {
    let store = Memory::new(start());
    let first = create(&store);
    let second = create(&store);
    let mut poller = watched(&store);
    poller.poll().unwrap();
    store.delete(&first, None).unwrap();
    ALL.set(true);
    assert_eq!(keys(poller.poll().unwrap()), vec![second]);
    assert_eq!(folds(), 3);
}

/// A poll that fails gives the store's error and forgets the version it
/// read at, and the next poll reads every case again.
#[test]
fn a_poll_after_a_failed_one_reads_every_case() {
    let store = Memory::new(start());
    let first = create(&store);
    let second = create(&store);
    let mut poller = watched(&store);
    poller.poll().unwrap();
    store.delete(&first, None).unwrap();
    FAIL.set(true);
    assert!(matches!(poller.poll(), Err(Fault::Missing)));
    assert_eq!(poller.version(), None, "a failed poll kept its version");
    assert!(poller.cases().is_empty(), "a failed poll kept its cases");
    assert_eq!(keys(poller.poll().unwrap()), vec![second]);
    let asked = ASKED.with_borrow(Clone::clone);
    assert_eq!(asked.len(), 3);
    assert_eq!(
        asked[2], None,
        "the poll after a failed one asked since a version"
    );
    assert_eq!(folds(), 3);
}

/// What the cases folded again and dropped were, poll by poll, as the poll
/// before left them.
fn only(folded: &[Key], removed: &[Key]) -> Refolded {
    Refolded::Some {
        folded: folded.to_vec(),
        removed: removed.to_vec(),
    }
}

/// Each poll says what it folded again and what it dropped: every case on
/// the first poll, on one given every case even when nothing changed, and
/// on one after a poll that failed; otherwise the cases written to and
/// opened, and the cases it had that were taken out. A case opened and
/// taken out between two polls was never had, so it is neither.
#[test]
fn a_poll_says_which_cases_it_folded_again_and_dropped() {
    let store = Memory::new(start());
    let first = create(&store);
    let second = create(&store);
    let mut poller = watched(&store);
    assert_eq!(poller.refolded(), &Refolded::All, "before any poll");
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &Refolded::All);
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &only(&[], &[]));

    note(&store, second);
    let third = create(&store);
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &only(&[second, third], &[]));
    assert_eq!(folds(), 4);

    store.delete(&first, None).unwrap();
    store.archive(&third, None).unwrap();
    let passing = create(&store);
    store.delete(&passing, None).unwrap();
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &only(&[], &[first, third]));

    // Given every case, though nothing changed: every case is folded
    // again, and so said.
    ALL.set(true);
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &Refolded::All);
    ALL.set(false);
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &only(&[], &[]));

    FAIL.set(true);
    assert!(poller.poll().is_err());
    assert_eq!(poller.refolded(), &Refolded::All, "after a failed poll");
    note(&store, second);
    assert_eq!(keys(poller.poll().unwrap()), vec![second]);
    assert_eq!(poller.refolded(), &Refolded::All, "the poll after it");
    note(&store, second);
    poller.poll().unwrap();
    assert_eq!(poller.refolded(), &only(&[second], &[]));
}

thread_local! {
    /// The answer the next read of a [`scripted`] poller gives, in place
    /// of the store's.
    static SCRIPT: RefCell<Option<Changes<Key>>> = const { RefCell::new(None) };
}

/// A poller over `store` whose next read gives [`SCRIPT`] when it is set,
/// as a read given to `with_read` may answer, and the store's otherwise.
fn scripted(store: &Memory) -> Poller<'_, Memory> {
    SCRIPT.set(None);
    Poller::with_read(store, |store, since| match SCRIPT.take() {
        Some(changes) => Ok(changes),
        None => store.rows_since(since),
    })
}

/// The report holds for a read that answers outside what a store does:
/// what changed, when asked with no version, is still every case folded
/// again; and a key both taken out and given again is folded again, in
/// one list only.
#[test]
fn a_poll_says_every_case_after_no_version_and_a_key_in_one_list() {
    let store = Memory::new(start());
    let first = create(&store);
    let second = create(&store);
    let Changes::All { version, cases } = store.rows_since(None).unwrap() else {
        panic!("not every case");
    };
    let mut poller = scripted(&store);
    SCRIPT.set(Some(Changes::Since {
        version: version.clone(),
        cases: cases.clone(),
        removed: Vec::new(),
    }));
    assert_eq!(keys(poller.poll().unwrap()), vec![first, second]);
    assert_eq!(poller.refolded(), &Refolded::All, "asked with no version");

    SCRIPT.set(Some(Changes::Since {
        version: Version::new("later"),
        cases: vec![cases[0].clone()],
        removed: vec![first, second],
    }));
    assert_eq!(keys(poller.poll().unwrap()), vec![first]);
    assert_eq!(poller.refolded(), &only(&[first], &[second]));
}
