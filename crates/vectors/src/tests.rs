//! The runner's negative tests: a store, or an answer, that is wrong in each
//! way the runner checks, which it must fail with the reason. The vectors
//! themselves, run against a right store, are in the core's and the stores'
//! tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

use std::cell::Cell;

use cases_core::{
    ActorRow, Append, Case, Changes, Key, Listening, Row, Store, StoreError, Time, Version, Wait,
    Writer,
};

use super::*;

#[path = "../../core/tests/memory/mod.rs"]
mod memory;

use memory::{Fault, Memory};

fn at(text: &str) -> Time {
    text.parse().unwrap()
}

fn start() -> Time {
    at(START)
}

/// A step, from the JSON of a vector's.
fn step(json: &str) -> Step {
    serde_json::from_str(json).unwrap()
}

/// The message of a failure.
fn why<T>(result: Result<T, Failed>) -> String {
    match result {
        Ok(_) => panic!("passed, want a failure"),
        Err(failed) => failed.message().unwrap_or("failed").to_owned(),
    }
}

/// A store in memory, as the runner runs it.
struct Right(Memory);

impl Harness for Right {
    type Store = Memory;

    fn store(&self) -> &Memory {
        &self.0
    }

    fn set_clock(&self, now: Time) {
        self.0.now.set(now);
    }

    fn category(&self, error: &Fault) -> Option<&'static str> {
        Some(match error {
            Fault::Missing => "missing",
            Fault::Unnamed | Fault::Doing(_) => "invalid",
            Fault::NotListening => "not-listening",
        })
    }

    const LOCAL: bool = true;
}

/// The same store, as a store that does not keep the local listening
/// record.
struct Plain(Memory);

impl Harness for Plain {
    type Store = Memory;

    fn store(&self) -> &Memory {
        &self.0
    }

    fn set_clock(&self, now: Time) {
        self.0.now.set(now);
    }

    fn category(&self, error: &Fault) -> Option<&'static str> {
        Right(Memory::new(start())).category(error)
    }

    const LOCAL: bool = false;
}

/// The same store, whose errors of its own the harness cannot name.
struct Nameless(Memory);

impl Harness for Nameless {
    type Store = Memory;

    fn store(&self) -> &Memory {
        &self.0
    }

    fn set_clock(&self, now: Time) {
        self.0.now.set(now);
    }

    fn category(&self, _: &Fault) -> Option<&'static str> {
        None
    }

    const LOCAL: bool = false;
}

/// What a store that is wrong gives instead.
#[derive(Clone, Copy)]
enum Skew {
    None,
    /// `rows` knows no case.
    NoRows,
    /// `actors` is empty.
    NoActors,
    /// `actors` gives every actor another name.
    Renamed,
    /// `actors` gives every actor another kind.
    Rekinded,
    /// `rows` gives a case that does not fold.
    Unfolded,
    /// `version` fails.
    NoVersion,
    /// `rows_since` fails.
    NoRowsSince,
}

/// A store in memory that is wrong in one way.
struct Skewed {
    inner: Memory,
    skew: Cell<Skew>,
}

impl Skewed {
    fn new() -> Skewed {
        Skewed {
            inner: Memory::new(start()),
            skew: Cell::new(Skew::None),
        }
    }
}

impl Store for Skewed {
    type Key = Key;
    type Writer = Writer;
    type Error = Fault;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Fault>> {
        self.inner.create(open, writer)
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Fault>> {
        self.inner.append(key, append, writer, at_revision)
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Fault> {
        match self.skew.get() {
            Skew::NoRows => Ok(None),
            Skew::Unfolded => Ok(Some(Vec::new())),
            _ => self.inner.rows(key),
        }
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Fault> {
        self.inner.list()
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Fault>> {
        self.inner.archive(key, at_revision)
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Fault>> {
        self.inner.delete(key, at_revision)
    }

    fn version(&self) -> Result<Version, Fault> {
        match self.skew.get() {
            Skew::NoVersion => Err(Fault::Missing),
            _ => self.inner.version(),
        }
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Fault> {
        match self.skew.get() {
            Skew::NoRowsSince => Err(Fault::Missing),
            _ => self.inner.rows_since(since),
        }
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Fault> {
        let actors = self.inner.actors()?;
        Ok(match self.skew.get() {
            Skew::NoActors => Vec::new(),
            Skew::Renamed => actors
                .into_iter()
                .map(|a| ActorRow::new(a.id, a.kind, Some("other".into()), a.created))
                .collect(),
            Skew::Rekinded => actors
                .into_iter()
                .map(|a| ActorRow::new(a.id, "human".into(), a.name, a.created))
                .collect(),
            _ => actors,
        })
    }

    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Fault> {
        self.inner.listen(wait, worker, labels, kinds, doing)
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Fault> {
        self.inner.set_doing(worker, text)
    }

    fn waits(&self) -> Result<Vec<Wait>, Fault> {
        self.inner.waits()
    }

    fn listeners(&self) -> Result<Vec<Listening>, Fault> {
        self.inner.listeners()
    }
}

impl Harness for Skewed {
    type Store = Skewed;

    fn store(&self) -> &Skewed {
        self
    }

    fn set_clock(&self, now: Time) {
        self.inner.now.set(now);
    }

    fn category(&self, _: &Fault) -> Option<&'static str> {
        None
    }

    const LOCAL: bool = true;
}

const OPEN_A: &str =
    r#"{"open": {"kind": "fyi", "urgency": "today", "title": "A"}, "as": "a", "writer": "lead"}"#;
const OPEN_B: &str =
    r#"{"open": {"kind": "fyi", "urgency": "today", "title": "B"}, "as": "b", "writer": "lead"}"#;

/// Runs the steps in order, as one vector, and gives the first failure.
fn run_steps<H>(harness: &H, steps: &[&str]) -> Result<Names, Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let mut names = Names::default();
    for json in steps {
        run_step(harness, &mut names, &step(json))?;
    }
    Ok(names)
}

fn right() -> Right {
    Right(Memory::new(start()))
}

// outcome

#[test]
fn outcome_compares_what_an_action_gave_with_what_the_step_expects() {
    let plain = step(r#"{"clock": "2026-09-15T09:00:00Z"}"#);
    let refused = step(r#"{"clock": "2026-09-15T09:00:00Z", "expect_error": "invalid"}"#);
    assert!(outcome(&plain, Ok(())).is_ok());
    assert!(outcome(&refused, Err("invalid".into())).is_ok());
    assert_eq!(why(outcome(&refused, Ok(()))), "taken, want invalid");
    assert_eq!(
        why(outcome(&refused, Err("stale".into()))),
        "refused as stale, want invalid"
    );
    assert_eq!(
        why(outcome(&plain, Err("stale".into()))),
        "refused as stale, want it taken"
    );
}

// The actions of run_step.

#[test]
fn a_clock_step_sets_the_clock() {
    let harness = right();
    run_steps(&harness, &[r#"{"clock": "2026-09-15T10:30:00Z"}"#]).unwrap();
    assert_eq!(harness.0.now.get(), at("2026-09-15T10:30:00Z"));
    let message = why(run_steps(&harness, &[r#"{"clock": "later"}"#]));
    assert_eq!(message, r#""later" is not a time"#);
}

#[test]
fn an_open_taken_that_the_step_expects_refused_fails() {
    let harness = right();
    let open = r#"{"open": {"kind": "fyi", "urgency": "today", "title": "A"}, "as": "a", "expect_error": "invalid"}"#;
    assert_eq!(why(run_steps(&harness, &[open])), "taken, want invalid");
}

#[test]
fn an_open_refused_in_another_category_fails() {
    let harness = right();
    let open = |expect: &str| {
        format!(r#"{{"open": {{"kind": "fyi", "urgency": "today", "title": ""}}{expect}}}"#)
    };
    let message = why(run_steps(
        &harness,
        &[&open(r#", "expect_error": "stale""#)],
    ));
    assert_eq!(message, "refused as invalid, want stale");
    let message = why(run_steps(&harness, &[&open(r#", "as": "a""#)]));
    assert_eq!(message, "refused as invalid, want it taken");
    run_steps(&harness, &[&open(r#", "expect_error": "invalid""#)]).unwrap();
}

#[test]
fn an_append_refused_is_named_by_its_category() {
    let harness = right();
    let stale = r#"{"append": "a", "author": "agent", "event": "note", "data": {"body": "More."}, "at_revision": 7, "expect_error": "stale"}"#;
    run_steps(&harness, &[OPEN_A, stale]).unwrap();
    let wrong = stale.replace("stale\"}", "invalid\"}");
    let message = why(run_steps(&harness, &[OPEN_A, &wrong]));
    assert_eq!(message, "refused as stale, want invalid");
}

#[test]
fn an_append_to_a_case_never_named_fails() {
    let harness = right();
    let append =
        r#"{"append": "ghost", "author": "agent", "event": "note", "data": {"body": "More."}}"#;
    assert_eq!(
        why(run_steps(&harness, &[OPEN_A, append])),
        r#"no case named "ghost""#
    );
}

#[test]
fn an_append_to_a_case_taken_out_is_no_case() {
    let harness = right();
    let append = r#"{"append": "a", "author": "agent", "event": "note", "data": {"body": "More."}, "expect_error": "no-case"}"#;
    run_steps(&harness, &[OPEN_A, r#"{"delete": "a"}"#, append]).unwrap();
    run_steps(&right(), &[OPEN_A, r#"{"archive": "a"}"#, append]).unwrap();
}

#[test]
fn an_archive_or_delete_is_expected_to_take_or_refuse() {
    for action in ["archive", "delete"] {
        let harness = right();
        let stale = format!(r#"{{"{action}": "a", "at_revision": 9, "expect_error": "stale"}}"#);
        run_steps(&harness, &[OPEN_A, &stale]).unwrap();
        let taken = format!(r#"{{"{action}": "a", "expect_error": "stale"}}"#);
        assert_eq!(
            why(run_steps(&right(), &[OPEN_A, &taken])),
            "taken, want stale"
        );
    }
}

#[test]
fn a_listen_and_a_set_doing_are_expected_to_take_or_refuse() {
    let harness = right();
    let blank = r#"{"listen": {"wait": "w", "worker": " "}, "expect_error": "invalid"}"#;
    let open_then = |steps: &[&str]| run_steps(&right(), &[&[OPEN_A], steps].concat());
    run_steps(&harness, &[OPEN_A, blank]).unwrap();
    assert_eq!(
        why(open_then(&[
            r#"{"listen": {"wait": "w", "worker": "lead"}, "expect_error": "invalid"}"#
        ])),
        "taken, want invalid"
    );
    assert_eq!(
        why(open_then(&[
            r#"{"set_doing": {"worker": "lead", "text": "x"}, "expect_error": "invalid"}"#
        ])),
        "refused as not-listening, want invalid"
    );
    open_then(&[
        r#"{"set_doing": {"worker": "lead", "text": "x"}, "expect_error": "not-listening"}"#,
        r#"{"listen": {"wait": "w", "worker": "lead"}}"#,
        r#"{"set_doing": {"worker": "lead", "text": "x"}}"#,
    ])
    .unwrap();
}

#[test]
fn an_error_of_the_store_that_the_harness_cannot_name_fails_the_vector() {
    let harness = Nameless(Memory::new(start()));
    // The store is not made yet.
    let message = why(run_steps(
        &harness,
        &[r#"{"listen": {"wait": "w", "worker": "lead"}}"#],
    ));
    assert_eq!(message, "the store failed: no store");
    let message = why(run_steps(
        &harness,
        &[r#"{"set_doing": {"worker": "lead", "text": "x"}}"#],
    ));
    assert_eq!(message, "the store failed: no store");
    let message = why(run_steps(&harness, &[r#"{"actors": []}"#]));
    assert_eq!(message, "the store failed: no store");
    let message = why(run_steps(&harness, &[r#"{"listeners": []}"#]));
    assert_eq!(message, "the store failed: no store");
}

#[test]
fn a_write_whose_error_the_harness_cannot_name_fails_the_vector() {
    // A refusal of the core is named by the core; only the store's own
    // errors go to the harness, and a write of the memory store gives none.
    let harness = Nameless(Memory::new(start()));
    run_steps(&harness, &[OPEN_A]).unwrap();
    assert_eq!(
        store_category(&harness, &Fault::Unnamed)
            .err()
            .and_then(|f| f.message().map(str::to_owned)),
        Some("the store failed: a blank worker".to_owned())
    );
    assert_eq!(
        write_category(&harness, &StoreError::NoCase).unwrap(),
        "no-case"
    );
    assert_eq!(
        why(write_category(
            &harness,
            &StoreError::Store(Fault::NotListening)
        )),
        "the store failed: not listening"
    );
}

#[test]
fn an_open_or_append_stored_with_the_wrong_actor_fails() {
    let append = step(
        r#"{"append": "a", "author": "agent", "event": "note", "data": {"body": "More."}, "writer": "lead"}"#,
    );
    for (skew, want) in [
        (Skew::NoRows, "the case is not stored"),
        (Skew::Unfolded, "the case does not fold: "),
        (Skew::NoActors, "is not in the table"),
        (Skew::Renamed, "want agent Some(\"lead\")"),
        (Skew::Rekinded, "want agent Some(\"lead\")"),
    ] {
        let harness = Skewed::new();
        harness.skew.set(skew);
        let message = why(run_steps(&harness, &[OPEN_A]));
        assert!(message.contains(want), "{message}");
        // An append is checked the same.
        let harness = Skewed::new();
        let mut names = run_steps(&harness, &[OPEN_A]).unwrap();
        run_step(&harness, &mut names, &append).unwrap();
        harness.skew.set(skew);
        let message = why(run_step(&harness, &mut names, &append));
        assert!(message.contains(want), "{message}");
    }
}

#[test]
fn a_version_step_names_the_version_and_checks_it() {
    let harness = right();
    let same = r#"{"version": "v2", "same_as": "v1"}"#;
    let differs = r#"{"version": "v2", "differs_from": ["v1"]}"#;
    run_steps(&harness, &[r#"{"version": "v1"}"#, same]).unwrap();
    run_steps(&harness, &[OPEN_A, r#"{"version": "v1"}"#, OPEN_B, differs]).unwrap();
    // The store has moved.
    assert_eq!(
        why(run_steps(
            &right(),
            &[
                r#"{"version": "v0"}"#,
                OPEN_A,
                r#"{"version": "v3", "same_as": "v0"}"#
            ]
        )),
        "the version moved from v0: 1"
    );
    // The store has not.
    let still = r#"{"version": "v2", "differs_from": ["v1"]}"#;
    assert_eq!(
        why(run_steps(&right(), &[r#"{"version": "v1"}"#, still])),
        "the version is still v1's: 0"
    );
    // A version never named.
    let message = why(run_steps(&right(), &[same]));
    assert_eq!(message, r#"no version named "v1""#);
    let message = why(run_steps(&right(), &[differs]));
    assert_eq!(message, r#"no version named "v1""#);
    // A store that cannot give its version.
    let skewed = Skewed::new();
    skewed.skew.set(Skew::NoVersion);
    let message = why(run_steps(&skewed, &[r#"{"version": "v1"}"#]));
    assert_eq!(message, "the store failed: no store");
}

#[test]
fn a_rows_since_step_fails_for_a_store_that_cannot_answer() {
    let skewed = Skewed::new();
    skewed.skew.set(Skew::NoRowsSince);
    let message = why(run_steps(
        &skewed,
        &[OPEN_A, r#"{"rows_since": null, "expect": {"live": ["a"]}}"#],
    ));
    assert_eq!(message, "the store failed: no store");
    // A version never named.
    let since = r#"{"rows_since": "v1", "expect": {"live": []}}"#;
    assert_eq!(
        why(run_steps(&right(), &[OPEN_A, since])),
        r#"no version named "v1""#
    );
}

#[test]
fn a_rows_since_step_without_expect_fails() {
    // A step as loaded is refused earlier; one built by hand is too.
    let mut names = Names::default();
    let bare = step(r#"{"rows_since": null}"#);
    assert_eq!(
        why(run_step(&right(), &mut names, &bare)),
        "rows_since gives what it expects"
    );
    let mut bare = bare;
    bare.rows_since = None;
    bare.clock = Some(START.into());
    run_step(&right(), &mut names, &bare).unwrap();
}

#[test]
fn an_actors_step_compares_kind_and_name() {
    let harness = right();
    let step =
        |kind: &str, name: &str| format!(r#"{{"actors": [{{"kind": "{kind}", "name": {name}}}]}}"#);
    run_steps(&harness, &[OPEN_A, &step("agent", r#""lead""#)]).unwrap();
    let message = why(run_steps(&harness, &[&step("human", r#""lead""#)]));
    assert_eq!(
        message,
        r#"the actors are [("agent", Some("lead"))], want [("human", Some("lead"))]"#
    );
    let message = why(run_steps(&harness, &[&step("agent", "null")]));
    assert!(message.contains("want [(\"agent\", None)]"), "{message}");
    let message = why(run_steps(&harness, &[r#"{"actors": []}"#]));
    assert!(message.contains("want []"), "{message}");
}

// check_changes

/// A store with the cases `a` and `b` in it, at version 2, and the names.
fn two() -> (Memory, Names, [Key; 2]) {
    let harness = right();
    let names = run_steps(&harness, &[OPEN_A, OPEN_B]).unwrap();
    let keys = [names.case("a").unwrap(), names.case("b").unwrap()];
    (harness.0, names, keys)
}

fn rows(store: &Memory, key: Key) -> Vec<Row> {
    store.rows(&key).unwrap().unwrap()
}

fn want(json: &str) -> ChangesWant {
    serde_json::from_str(json).unwrap()
}

fn v(text: &str) -> Version {
    Version::new(text)
}

fn changes(
    store: &Memory,
    names: &Names,
    since: Option<&str>,
    got: &Changes<Key>,
    want: &ChangesWant,
) -> Result<(), Failed> {
    let since = since.map(v);
    check_changes(store, names, since.as_ref(), got, want)
}

#[test]
fn changes_with_a_case_whose_rows_are_not_the_stores_fail() {
    let (store, names, [a, b]) = two();
    let all = want(r#"{"live": ["a", "b"]}"#);
    let given = Changes::All {
        version: v("2"),
        cases: vec![(a, rows(&store, a)), (b, rows(&store, a))],
    };
    assert_eq!(
        why(changes(&store, &names, None, &given, &all)),
        "b's rows are not the store's"
    );
    // A case the store does not have.
    let ghost = Key::new(9).unwrap();
    let given = Changes::All {
        version: v("2"),
        cases: vec![(ghost, rows(&store, a))],
    };
    assert_eq!(
        why(changes(&store, &names, None, &given, &all)),
        format!("<{ghost}>'s rows are not the store's")
    );
    // The rows of a case with a row missing.
    let given = Changes::All {
        version: v("2"),
        cases: vec![(a, Vec::new())],
    };
    assert!(changes(&store, &names, None, &given, &all).is_err());
}

#[test]
fn changes_whose_version_is_not_the_stores_fail() {
    let (store, names, [a, b]) = two();
    let all = want(r#"{"live": ["a", "b"]}"#);
    let cases = vec![(a, rows(&store, a)), (b, rows(&store, b))];
    let given = Changes::All {
        version: v("1"),
        cases: cases.clone(),
    };
    assert_eq!(
        why(changes(&store, &names, None, &given, &all)),
        "the answer's version is not the store's: 1"
    );
    let given = Changes::All {
        version: v("2"),
        cases,
    };
    changes(&store, &names, None, &given, &all).unwrap();
}

#[test]
fn changes_with_no_version_asked_about_must_be_every_case() {
    let (store, names, _) = two();
    let nothing = want(r#"{"live": ["a", "b"]}"#);
    let unchanged = Changes::Unchanged { version: v("2") };
    assert_eq!(
        why(changes(&store, &names, None, &unchanged, &nothing)),
        "with no version, the answer is every case"
    );
    let since = Changes::Since {
        version: v("2"),
        cases: Vec::new(),
        removed: Vec::new(),
    };
    assert_eq!(
        why(changes(&store, &names, None, &since, &nothing)),
        "with no version, the answer is every case"
    );
}

#[test]
fn unchanged_is_only_for_the_version_asked_about() {
    let (store, names, _) = two();
    let none = want(r#"{"live": ["a", "b"]}"#);
    let unchanged = Changes::Unchanged { version: v("2") };
    changes(&store, &names, Some("2"), &unchanged, &none).unwrap();
    assert_eq!(
        why(changes(&store, &names, Some("1"), &unchanged, &none)),
        "nothing changed, but the version moved to 2"
    );
}

#[test]
fn unchanged_fails_when_the_step_expects_changes_or_removals() {
    let (store, names, _) = two();
    let unchanged = Changes::Unchanged { version: v("2") };
    let changed = want(r#"{"live": ["a", "b"], "changed": ["a"]}"#);
    assert_eq!(
        why(changes(&store, &names, Some("2"), &unchanged, &changed)),
        r#"nothing changed, want ["a"] changed and [] removed"#
    );
    let removed = want(r#"{"live": ["a", "b"], "removed": ["b"]}"#);
    assert_eq!(
        why(changes(&store, &names, Some("2"), &unchanged, &removed)),
        r#"nothing changed, want [] changed and ["b"] removed"#
    );
    // What a step may allow, but not require, does not make it a change.
    let may = want(r#"{"live": ["a", "b"], "removed_may": ["b"]}"#);
    changes(&store, &names, Some("2"), &unchanged, &may).unwrap();
}

#[test]
fn since_gives_the_cases_that_changed_in_the_order_expected() {
    let (store, names, [a, b]) = two();
    let both = want(r#"{"live": ["a", "b"], "changed": ["a", "b"]}"#);
    let given = |cases: Vec<Key>| Changes::Since {
        version: v("2"),
        cases: cases.into_iter().map(|k| (k, rows(&store, k))).collect(),
        removed: Vec::new(),
    };
    changes(&store, &names, Some("0"), &given(vec![a, b]), &both).unwrap();
    assert_eq!(
        why(changes(
            &store,
            &names,
            Some("0"),
            &given(vec![b, a]),
            &both
        )),
        r#"["b", "a"] changed, want ["a", "b"]"#
    );
    assert_eq!(
        why(changes(&store, &names, Some("0"), &given(vec![a]), &both)),
        r#"["a"] changed, want ["a", "b"]"#
    );
    assert_eq!(
        why(changes(&store, &names, Some("0"), &given(vec![]), &both)),
        r#"[] changed, want ["a", "b"]"#
    );
}

#[test]
fn since_never_gives_a_key_twice() {
    let (store, names, [a, b]) = two();
    let w =
        want(r#"{"live": ["a", "b"], "changed": ["a"], "removed": ["b"], "removed_may": ["a"]}"#);
    let given = |removed: Vec<Key>| Changes::Since {
        version: v("2"),
        cases: vec![(a, rows(&store, a))],
        removed,
    };
    changes(&store, &names, Some("0"), &given(vec![b]), &w).unwrap();
    // Removed twice.
    assert_eq!(
        why(changes(&store, &names, Some("0"), &given(vec![b, b]), &w)),
        "a key is given twice"
    );
    // Changed, and removed.
    assert_eq!(
        why(changes(&store, &names, Some("0"), &given(vec![a, b]), &w)),
        "a key is given twice"
    );
}

#[test]
fn since_names_the_removed_cases_it_must_and_may() {
    let (store, names, [a, b]) = two();
    let w = want(r#"{"live": ["a"], "removed": ["b"], "removed_may": ["a"]}"#);
    let c = Key::new(9).unwrap();
    let given = |removed: Vec<Key>| Changes::Since {
        version: v("2"),
        cases: Vec::new(),
        removed,
    };
    let run = |removed: Vec<Key>| changes(&store, &names, Some("0"), &given(removed), &w);
    // Exactly what it must, and what it may as well.
    run(vec![b]).unwrap();
    run(vec![a, b]).unwrap();
    // One that it must is missing.
    assert_eq!(
        why(run(vec![])),
        r#"{} removed, want {"b"}, with ["a"] allowed"#
    );
    assert_eq!(
        why(run(vec![a])),
        r#"{"a"} removed, want {"b"}, with ["a"] allowed"#
    );
    // One that it may not.
    let message = why(run(vec![b, c]));
    assert!(message.contains(&format!("<{c}>")), "{message}");
    assert!(
        message.ends_with(r#" removed, want {"b"}, with ["a"] allowed"#),
        "{message}"
    );
}

#[test]
fn all_gives_every_case_in_the_order_opened() {
    let (store, names, [a, b]) = two();
    let w = want(r#"{"live": ["a", "b"]}"#);
    let given = |cases: Vec<Key>| Changes::All {
        version: v("2"),
        cases: cases.into_iter().map(|k| (k, rows(&store, k))).collect(),
    };
    changes(&store, &names, None, &given(vec![a, b]), &w).unwrap();
    changes(&store, &names, Some("0"), &given(vec![a, b]), &w).unwrap();
    assert_eq!(
        why(changes(&store, &names, None, &given(vec![b, a]), &w)),
        r#"every case is ["b", "a"], want ["a", "b"]"#
    );
    assert_eq!(
        why(changes(&store, &names, None, &given(vec![a]), &w)),
        r#"every case is ["a"], want ["a", "b"]"#
    );
}

#[test]
fn changes_fail_for_a_store_that_cannot_give_its_rows_or_version() {
    let skewed = Skewed::new();
    let names = run_steps(&skewed, &[OPEN_A]).unwrap();
    let a = names.case("a").unwrap();
    let all = want(r#"{"live": ["a"]}"#);
    let given = Changes::All {
        version: v("1"),
        cases: vec![(a, skewed.inner.rows(&a).unwrap().unwrap())],
    };
    check_changes(&skewed, &names, None, &given, &all).unwrap();
    skewed.skew.set(Skew::NoVersion);
    assert_eq!(
        why(check_changes(&skewed, &names, None, &given, &all)),
        "the store failed: no store"
    );
    // A case the store no longer has.
    skewed.skew.set(Skew::NoRows);
    assert_eq!(
        why(check_changes(&skewed, &names, None, &given, &all)),
        "a's rows are not the store's"
    );
}

// check_listeners

fn wait(id: &str, labels: &[&str], kinds: &[&str], since: &str, heard: &str) -> Wait {
    Wait::new(
        id.into(),
        "lead".into(),
        labels.iter().map(|l| (*l).to_owned()).collect(),
        kinds.iter().map(|k| (*k).to_owned()).collect(),
        at(since),
        at(heard),
    )
}

/// The listener `lead`, on `release` since 09:00, heard at 09:01.
fn lead() -> Listening {
    Listening::new(
        "lead".into(),
        vec!["release".into()],
        at("2026-09-15T09:00:00Z"),
        at("2026-09-15T09:01:00Z"),
        None,
        None,
        vec![wait(
            "w1",
            &["release"],
            &[],
            "2026-09-15T09:00:00Z",
            "2026-09-15T09:01:00Z",
        )],
    )
}

fn listeners(json: &str) -> Vec<ListenerWant> {
    serde_json::from_str(json).unwrap()
}

const LEAD: &str = r#"[{"worker": "lead", "labels": ["release"], "doing": null}]"#;

fn listening(got: &[Listening], want: &str) -> Result<(), Failed> {
    let waits: Vec<Wait> = got.iter().flat_map(|l| l.waits.clone()).collect();
    check_listeners(got, &waits, &listeners(want))
}

#[test]
fn listeners_that_are_as_expected_pass() {
    listening(&[lead()], LEAD).unwrap();
    listening(&[], "[]").unwrap();
    let full = r#"[{"worker": "lead", "labels": ["release"], "doing": null,
        "since": "2026-09-15T09:00:00Z", "heard": "2026-09-15T09:01:00Z",
        "waits": [{"wait": "w1", "labels": ["release"], "kinds": []}]}]"#;
    listening(&[lead()], full).unwrap();
}

#[test]
fn a_number_of_listeners_other_than_expected_fails() {
    let message = why(listening(&[lead()], "[]"));
    assert_eq!(
        message,
        r#"listening: ["lead [\"release\"] doing None"], want 0 workers"#
    );
    let two = format!(
        "[{0}, {0}]",
        LEAD.trim_start_matches('[').trim_end_matches(']')
    );
    assert!(why(listening(&[lead()], &two)).ends_with("want 2 workers"));
    assert!(why(listening(&[], LEAD)).ends_with("want 1 workers"));
}

#[test]
fn waits_that_are_not_the_listeners_fail() {
    let got = [lead()];
    let message = why(check_listeners(&got, &[], &listeners(LEAD)));
    assert!(
        message.starts_with("the waits [] are not the listeners'"),
        "{message}"
    );
    // Another wait, or the same waits in another order.
    let other = wait(
        "w2",
        &["release"],
        &[],
        "2026-09-15T09:00:00Z",
        "2026-09-15T09:01:00Z",
    );
    let waits = vec![got[0].waits[0].clone(), other.clone()];
    assert!(check_listeners(&got, &waits, &listeners(LEAD)).is_err());
    let mut two = lead();
    two.waits.push(other);
    let swapped = vec![two.waits[1].clone(), two.waits[0].clone()];
    assert!(check_listeners(&[two.clone()], &swapped, &listeners(LEAD)).is_err());
    check_listeners(&[two.clone()], &two.waits, &listeners(LEAD)).unwrap();
}

#[test]
fn a_listener_that_is_not_whole_fails() {
    let bad = |edit: fn(&mut Listening)| {
        let mut l = lead();
        edit(&mut l);
        let message = why(listening(&[l], LEAD));
        assert!(
            message
                .starts_with("lead: its labels, times and doing line are not those of its waits"),
            "{message}"
        );
    };
    // No waits.
    bad(|l| l.waits.clear());
    // A wait of another worker.
    bad(|l| l.waits[0].worker = "other".into());
    // Labels the waits do not have, or the waits have more of.
    bad(|l| l.labels = vec!["other".into()]);
    bad(|l| l.labels.push("extra".into()));
    bad(|l| l.waits[0].labels.push("extra".into()));
    // A start or a hearing that is not the waits'.
    bad(|l| l.since = at("2026-09-15T08:00:00Z"));
    bad(|l| l.since = at("2026-09-15T09:00:30Z"));
    bad(|l| l.heard = at("2026-09-15T09:00:30Z"));
    bad(|l| l.heard = at("2026-09-15T09:02:00Z"));
    // A doing line without a time, or a time without a line.
    bad(|l| l.doing = Some("x".into()));
    bad(|l| l.doing_at = Some(at("2026-09-15T09:00:00Z")));
}

#[test]
fn the_since_and_heard_of_a_listener_are_those_of_its_first_and_last_wait() {
    let mut l = lead();
    l.waits.push(wait(
        "w2",
        &["release"],
        &[],
        "2026-09-15T09:00:30Z",
        "2026-09-15T09:03:00Z",
    ));
    // Heard at the latest of its waits.
    l.heard = at("2026-09-15T09:03:00Z");
    listening(&[l.clone()], LEAD).unwrap();
    l.heard = at("2026-09-15T09:01:00Z");
    assert!(listening(&[l.clone()], LEAD).is_err());
    // Since at the earliest.
    l.heard = at("2026-09-15T09:03:00Z");
    l.since = at("2026-09-15T09:00:30Z");
    assert!(listening(&[l], LEAD).is_err());
}

#[test]
fn a_listener_that_is_not_the_one_expected_fails() {
    let mismatch = |got: Listening, want: &str| {
        let message = why(listening(&[got], want));
        assert!(message.starts_with("listening: ["), "{message}");
        assert!(message.contains("), want ListenerWant"), "{message}");
    };
    mismatch(
        lead(),
        r#"[{"worker": "other", "labels": ["release"], "doing": null}]"#,
    );
    mismatch(
        lead(),
        r#"[{"worker": "lead", "labels": [], "doing": null}]"#,
    );
    mismatch(
        lead(),
        r#"[{"worker": "lead", "labels": ["release"], "doing": "x"}]"#,
    );
    let mut doing = lead();
    doing.doing = Some("x".into());
    doing.doing_at = Some(at("2026-09-15T09:00:30Z"));
    mismatch(
        doing.clone(),
        r#"[{"worker": "lead", "labels": ["release"], "doing": null}]"#,
    );
    // The times, when given.
    for (field, time) in [
        ("since", "2026-09-15T09:00:01Z"),
        ("heard", "2026-09-15T09:01:01Z"),
    ] {
        mismatch(
            lead(),
            &format!(
                r#"[{{"worker": "lead", "labels": ["release"], "doing": null, "{field}": "{time}"}}]"#
            ),
        );
    }
    let doing_want = |at: &str| {
        format!(
            r#"[{{"worker": "lead", "labels": ["release"], "doing": "x", "doing_at": "{at}"}}]"#
        )
    };
    listening(&[doing.clone()], &doing_want("2026-09-15T09:00:30Z")).unwrap();
    mismatch(doing, &doing_want("2026-09-15T09:00:31Z"));
    // A time the step gives for a doing line there is none of.
    mismatch(
        lead(),
        r#"[{"worker": "lead", "labels": ["release"], "doing": null, "doing_at": "2026-09-15T09:00:30Z"}]"#,
    );
}

#[test]
fn the_waits_of_a_listener_are_compared_when_the_step_gives_them() {
    let wait_want = |waits: &str| {
        format!(r#"[{{"worker": "lead", "labels": ["release"], "doing": null, "waits": {waits}}}]"#)
    };
    let fails = |waits: &str| {
        let message = why(listening(&[lead()], &wait_want(waits)));
        assert!(
            message.contains("waits [(\"w1\", [\"release\"], [])]"),
            "{message}"
        );
    };
    listening(
        &[lead()],
        &wait_want(r#"[{"wait": "w1", "labels": ["release"], "kinds": []}]"#),
    )
    .unwrap();
    // Another id, labels or kinds, or more, or fewer.
    fails(r#"[{"wait": "w2", "labels": ["release"], "kinds": []}]"#);
    fails(r#"[{"wait": "w1", "labels": [], "kinds": []}]"#);
    fails(r#"[{"wait": "w1", "labels": ["release"], "kinds": ["ask"]}]"#);
    fails("[]");
    fails(
        r#"[{"wait": "w1", "labels": ["release"], "kinds": []}, {"wait": "w2", "labels": [], "kinds": []}]"#,
    );
}

#[test]
fn a_time_in_the_step_that_is_not_one_fails_the_vector() {
    let message = why(listening(
        &[lead()],
        r#"[{"worker": "lead", "labels": ["release"], "doing": null, "since": "soon"}]"#,
    ));
    assert_eq!(message, r#""soon" is not a time"#);
    let message = why(listening(
        &[lead()],
        r#"[{"worker": "lead", "labels": ["release"], "doing": null, "heard": "soon"}]"#,
    ));
    assert_eq!(message, r#""soon" is not a time"#);
    let message = why(listening(
        &[lead()],
        r#"[{"worker": "lead", "labels": ["release"], "doing": null, "doing_at": "soon"}]"#,
    ));
    assert_eq!(message, r#""soon" is not a time"#);
}

#[test]
fn a_listeners_step_runs_against_a_store() {
    let harness = right();
    let listen = r#"{"listen": {"wait": "w1", "worker": "lead", "labels": ["release"]}}"#;
    let listeners = |doing: &str| {
        format!(
            r#"{{"listeners": [{{"worker": "lead", "labels": ["release"], "doing": {doing}}}]}}"#
        )
    };
    run_steps(&harness, &[OPEN_A, listen, &listeners("null")]).unwrap();
    let message = why(run_steps(&harness, &[&listeners(r#""x""#)]));
    assert!(message.starts_with("listening: ["), "{message}");
}

// Loading

/// The arguments of a run whose output the test does not want to show.
fn quiet() -> libtest_mimic::Arguments {
    libtest_mimic::Arguments {
        quiet: true,
        ..libtest_mimic::Arguments::default()
    }
}

#[test]
fn a_step_must_be_one_action_with_what_goes_with_it() {
    let refuses = |json: &str, want: &str| {
        let why = step(json).action().err().unwrap();
        assert!(why.contains(want), "{json}: {why}");
    };
    refuses("{}", "0 actions, not one");
    refuses(
        r#"{"clock": "2026-09-15T09:00:00Z", "version": "v"}"#,
        "2 actions, not one",
    );
    refuses(r#"{"clock": "noon"}"#, r#""noon" is not a time"#);
    refuses(
        r#"{"rows_since": 3, "expect": {"live": []}}"#,
        "rows_since is a version's name or null",
    );
    refuses(
        r#"{"listeners": [{"worker": "w", "labels": [], "since": "x"}]}"#,
        "is not a time",
    );
    refuses(
        r#"{"listeners": [{"worker": "w", "labels": [], "heard": "x"}]}"#,
        "is not a time",
    );
    refuses(
        r#"{"listeners": [{"worker": "w", "labels": [], "doing_at": "x"}]}"#,
        "is not a time",
    );
    refuses(
        r#"{"open": {}, "as": "a", "event": "x"}"#,
        "event does not go with this action",
    );
    refuses(
        r#"{"version": "v", "as": "a"}"#,
        "as does not go with this action",
    );
    refuses(
        r#"{"version": "v", "expect_error": "invalid"}"#,
        "expect_error does not go with this action",
    );
    refuses(
        r#"{"open": {}}"#,
        "an open that is taken names its case with as",
    );
    refuses(
        r#"{"append": "a", "event": "x"}"#,
        "an append gives its author and event",
    );
    refuses(
        r#"{"append": "a", "author": "agent"}"#,
        "an append gives its author and event",
    );
    refuses(
        r#"{"rows_since": null}"#,
        "rows_since gives what it expects",
    );
    // Each of the fields an action takes.
    for json in [
        r#"{"open": {}, "as": "a"}"#,
        r#"{"open": {}, "expect_error": "invalid"}"#,
        r#"{"open": {}, "as": "a", "author": "agent", "writer": "w"}"#,
        r#"{"append": "a", "author": "agent", "event": "e", "data": {}, "writer": "w", "at_revision": 1}"#,
        r#"{"archive": "a", "at_revision": 1, "expect_error": "stale"}"#,
        r#"{"delete": "a", "at_revision": 1}"#,
        r#"{"listen": {"wait": "w", "worker": "w"}, "expect_error": "invalid"}"#,
        r#"{"set_doing": {"worker": "w", "text": "t"}, "expect_error": "invalid"}"#,
        r#"{"version": "v", "same_as": "u", "differs_from": ["t"]}"#,
        r#"{"rows_since": "v", "expect": {"live": []}}"#,
    ] {
        step(json).action().unwrap();
    }
    for json in [
        r#"{"archive": "a", "author": "agent"}"#,
        r#"{"archive": "a", "event": "e"}"#,
        r#"{"archive": "a", "data": {}}"#,
        r#"{"archive": "a", "writer": "w"}"#,
        r#"{"clock": "2026-09-15T09:00:00Z", "at_revision": 1}"#,
        r#"{"clock": "2026-09-15T09:00:00Z", "same_as": "v"}"#,
        r#"{"clock": "2026-09-15T09:00:00Z", "differs_from": []}"#,
        r#"{"clock": "2026-09-15T09:00:00Z", "expect": {"live": []}}"#,
    ] {
        assert!(step(json).action().is_err(), "{json}");
    }
}

#[test]
fn the_store_vectors_load_and_are_named_once() {
    let files = load_all().unwrap();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["actors", "changes", "listening", "version"]);
    assert!(files.iter().all(|(_, vectors)| !vectors.is_empty()));
}

#[test]
fn a_trial_is_made_for_each_vector_and_a_local_one_is_ignored_for_a_store_that_is_not() {
    let all = load_all().unwrap();
    let total: u64 = all.iter().map(|(_, vectors)| vectors.len() as u64).sum();
    let local = all
        .iter()
        .flat_map(|(_, vectors)| vectors)
        .filter(|vector| vector.local)
        .count() as u64;
    assert!(local > 0);
    let arguments = quiet();
    let conclusion = libtest_mimic::run(&arguments, trials(|start| Right(Memory::new(start))));
    assert_eq!((conclusion.num_passed, conclusion.num_ignored), (total, 0));
    let conclusion = libtest_mimic::run(&arguments, trials(|start| Plain(Memory::new(start))));
    assert_eq!(conclusion.num_ignored, local);
    assert_eq!(conclusion.num_failed, 0);
    assert_eq!(conclusion.num_passed, total - local);
}

#[test]
fn a_store_that_is_wrong_fails_the_vectors() {
    // A store whose actors are all misnamed fails each vector that writes.
    let failed = load_all()
        .unwrap()
        .iter()
        .flat_map(|(_, vectors)| vectors)
        .filter(|vector| {
            let skewed = Skewed::new();
            skewed.skew.set(Skew::Renamed);
            run(&skewed, vector).is_err()
        })
        .count();
    assert!(failed > 0);
}

#[test]
fn names_say_which_case_a_key_is() {
    let (_, names, [a, _]) = two();
    assert_eq!(names.name_of(a), "a");
    let ghost = Key::new(9).unwrap();
    assert_eq!(names.name_of(ghost), format!("<{ghost}>"));
}
