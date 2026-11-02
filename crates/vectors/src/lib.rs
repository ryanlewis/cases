//! Runs the store vectors, `spec/vectors/store/*.json`, against a store.
//!
//! The case vectors (`spec/vectors/*.json`) test the core alone: a fold and
//! a writer need no store. The store vectors test what a store keeps beside
//! the cases, through the core's [`Store`] trait: its version, the changes
//! since one, its table of actors, and the sessions listening. The core
//! does no IO and keeps nothing, so it cannot run them itself. This crate is
//! the runner, and a store runs the vectors by implementing [`Harness`] in
//! its own tests: the core's tests run them against a store in memory, and
//! `cases-store-sqlite`'s against a SQLite file. A store outside this
//! repository, such as a hosted one, can take the crate the same way.
//!
//! The format is in `spec/vectors/README.md`, under "Store vectors".

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};

use cases_core::{Append, Changes, Key, Store, StoreError, Time, Version, Writer};
use libtest_mimic::{Failed, Trial};
use serde::Deserialize;
use serde_json::{Map, Value};

/// The time a vector's store starts at, until a `clock` step moves it.
pub const START: &str = "2026-09-15T09:00:00Z";

/// A store the vectors run against: one empty store for each vector.
pub trait Harness {
    /// The store.
    type Store: Store<Key = Key, Writer = Writer>;

    /// The store, empty when the vector starts: not made yet.
    fn store(&self) -> &Self::Store;

    /// Sets the store's clock to `now`, for every write and read after.
    fn set_clock(&self, now: Time);

    /// The category of an error of the store's own, as the vectors name
    /// it: `missing` for a store not made yet, `invalid` for a blank
    /// worker or a doing line that is not one, and `not-listening` for a
    /// doing line of a worker not listening. `None` for a failure of the
    /// store, which fails the vector.
    fn category(&self, error: &<Self::Store as Store>::Error) -> Option<&'static str>;

    /// Whether the store keeps the local listening record, a heartbeat
    /// that lapses (ST-16): only such a store runs the vectors marked
    /// `local`, and the rest show as ignored.
    const LOCAL: bool;
}

/// A test for each store vector, which runs it against a new harness that
/// `make` gives, with its store's clock at `start`. A vector marked `local`
/// is ignored for a store that is not.
pub fn trials<H, F>(make: F) -> Vec<Trial>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
    F: Fn(Time) -> H + Clone + Send + 'static,
{
    let mut trials = Vec::new();
    match load_all() {
        Ok(files) => {
            for (file, vectors) in files {
                for vector in vectors {
                    let name = format!("store/{file}.json: {}", vector.description);
                    let make = make.clone();
                    let ignored = vector.local && !H::LOCAL;
                    let trial = Trial::test(name, move || {
                        let start: Time = START.parse().map_err(|_| "START is not a time")?;
                        run(&make(start), &vector)
                    })
                    .with_ignored_flag(ignored);
                    trials.push(trial);
                }
            }
        }
        Err(errors) => {
            let errors = errors.join("\n");
            trials.push(Trial::test("the store vectors load", move || {
                Err(errors.into())
            }));
        }
    }
    trials
}

/// The directory of the store vectors.
#[must_use]
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/vectors/store")
}

/// Every file of store vectors, by name without `.json`, in name order,
/// or what is wrong with them.
///
/// # Errors
///
/// Each file that does not read, or holds a vector that is not one, with
/// why.
pub fn load_all() -> Result<Vec<(String, Vec<Vector>)>, Vec<String>> {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let entries = fs::read_dir(dir()).map_err(|e| vec![format!("{}: {e}", dir().display())])?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let read = fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<Vec<Vector>>(&text).map_err(|e| e.to_string()));
        match read {
            Ok(vectors) => {
                let mut seen = BTreeSet::new();
                for vector in &vectors {
                    if !seen.insert(vector.description.clone()) {
                        errors.push(format!("{name}.json: {:?} twice", vector.description));
                    }
                    // A case or a version is named once in a vector.
                    let mut named = BTreeSet::new();
                    for (i, step) in vector.steps.iter().enumerate() {
                        let names = [
                            step.name.as_ref().map(|n| ("case", n)),
                            step.version.as_ref().map(|n| ("version", n)),
                        ];
                        let twice = names.into_iter().flatten().find(|n| !named.insert(*n));
                        let checked = match twice {
                            Some((what, n)) => Err(format!("the {what} {n:?} is named twice")),
                            None => step.action().map(drop),
                        };
                        if let Err(why) = checked {
                            errors.push(format!(
                                "{name}.json: {:?}, step {}: {why}",
                                vector.description,
                                i + 1
                            ));
                        }
                    }
                }
                files.push((name, vectors));
            }
            Err(why) => errors.push(format!("{name}.json: {why}")),
        }
    }
    if errors.is_empty() {
        Ok(files)
    } else {
        Err(errors)
    }
}

/// A store vector: steps run in order against one new store.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vector {
    /// Names the vector, uniquely within its file.
    pub description: String,
    /// Only for a store that keeps the local listening record.
    #[serde(default)]
    pub local: bool,
    /// What to do and what to expect, in order.
    pub steps: Vec<Step>,
}

/// One step: one action, with what it takes and what to expect.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    // The actions: exactly one per step.
    clock: Option<String>,
    open: Option<Map<String, Value>>,
    append: Option<String>,
    archive: Option<String>,
    delete: Option<String>,
    listen: Option<Listen>,
    set_doing: Option<SetDoing>,
    version: Option<String>,
    /// A version's name, or `null` for none: kept apart from the step not
    /// giving it at all.
    #[serde(default, deserialize_with = "present")]
    rows_since: Option<Value>,
    actors: Option<Vec<ActorWant>>,
    listeners: Option<Vec<ListenerWant>>,

    // What an action takes.
    #[serde(rename = "as")]
    name: Option<String>,
    author: Option<String>,
    event: Option<String>,
    data: Option<Map<String, Value>>,
    writer: Option<String>,
    at_revision: Option<u64>,

    // What to expect.
    same_as: Option<String>,
    differs_from: Option<Vec<String>>,
    expect: Option<ChangesWant>,
    expect_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listen {
    wait: String,
    worker: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    kinds: Vec<String>,
    doing: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDoing {
    worker: String,
    text: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActorWant {
    kind: String,
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListenerWant {
    worker: String,
    labels: Vec<String>,
    doing: Option<String>,
    since: Option<String>,
    heard: Option<String>,
    doing_at: Option<String>,
    /// Its waits listening now, by id, when given.
    waits: Option<Vec<WaitWant>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitWant {
    wait: String,
    labels: Vec<String>,
    kinds: Vec<String>,
}

/// The time `text`, or why it is not one.
fn time(text: &str) -> Result<Time, String> {
    text.parse().map_err(|_| format!("{text:?} is not a time"))
}

/// What a `rows_since` step expects.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangesWant {
    /// Every case in the store now, by name, in the order opened.
    live: Vec<String>,
    /// The cases with a row stored since the version.
    #[serde(default)]
    changed: Vec<String>,
    /// The cases archived or deleted since the version, which an answer of
    /// what changed must name.
    #[serde(default)]
    removed: Vec<String>,
    /// Cases opened and removed since the version, which it may name.
    #[serde(default)]
    removed_may: Vec<String>,
}

/// A field that is there, `null` included.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// A step's action, checked to be the one it names.
enum Action<'a> {
    Clock(&'a str),
    Open(&'a Map<String, Value>),
    Append(&'a str),
    Archive(&'a str),
    Delete(&'a str),
    Listen(&'a Listen),
    SetDoing(&'a SetDoing),
    Version(&'a str),
    RowsSince(Option<&'a str>),
    Actors(&'a [ActorWant]),
    Listeners(&'a [ListenerWant]),
}

impl Step {
    /// The step's one action, or why it is not a step.
    fn action(&self) -> Result<Action<'_>, String> {
        let found: Vec<Action> = [
            self.clock.as_deref().map(Action::Clock),
            self.open.as_ref().map(Action::Open),
            self.append.as_deref().map(Action::Append),
            self.archive.as_deref().map(Action::Archive),
            self.delete.as_deref().map(Action::Delete),
            self.listen.as_ref().map(Action::Listen),
            self.set_doing.as_ref().map(Action::SetDoing),
            self.version.as_deref().map(Action::Version),
            self.rows_since()?,
            self.actors.as_deref().map(Action::Actors),
            self.listeners.as_deref().map(Action::Listeners),
        ]
        .into_iter()
        .flatten()
        .collect();
        let [action] = <[Action; 1]>::try_from(found)
            .map_err(|found| format!("{} actions, not one", found.len()))?;
        check_times(&action)?;
        self.check_takes(&action)?;
        self.check_needs(action)
    }

    /// The step's `rows_since` action, if it has one.
    fn rows_since(&self) -> Result<Option<Action<'_>>, String> {
        match &self.rows_since {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(Action::RowsSince(None))),
            Some(Value::String(name)) => Ok(Some(Action::RowsSince(Some(name)))),
            Some(_) => Err("rows_since is a version's name or null".into()),
        }
    }

    /// Checks that the step gives no field `action` does not take.
    fn check_takes(&self, action: &Action) -> Result<(), String> {
        let writes = matches!(
            action,
            Action::Open(_) | Action::Append(_) | Action::Archive(_) | Action::Delete(_)
        );
        let takes = [
            ("as", self.name.is_some(), matches!(action, Action::Open(_))),
            (
                "author",
                self.author.is_some(),
                matches!(action, Action::Open(_) | Action::Append(_)),
            ),
            (
                "event",
                self.event.is_some(),
                matches!(action, Action::Append(_)),
            ),
            (
                "data",
                self.data.is_some(),
                matches!(action, Action::Append(_)),
            ),
            (
                "writer",
                self.writer.is_some(),
                matches!(action, Action::Open(_) | Action::Append(_)),
            ),
            (
                "at_revision",
                self.at_revision.is_some(),
                matches!(
                    action,
                    Action::Append(_) | Action::Archive(_) | Action::Delete(_)
                ),
            ),
            (
                "same_as",
                self.same_as.is_some(),
                matches!(action, Action::Version(_)),
            ),
            (
                "differs_from",
                self.differs_from.is_some(),
                matches!(action, Action::Version(_)),
            ),
            (
                "expect",
                self.expect.is_some(),
                matches!(action, Action::RowsSince(_)),
            ),
            (
                "expect_error",
                self.expect_error.is_some(),
                writes || matches!(action, Action::Listen(_) | Action::SetDoing(_)),
            ),
        ];
        if let Some((key, ..)) = takes.iter().find(|(_, given, allowed)| *given && !allowed) {
            return Err(format!("{key} does not go with this action"));
        }
        Ok(())
    }

    /// `action`, if the step gives every field it needs.
    fn check_needs<'a>(&self, action: Action<'a>) -> Result<Action<'a>, String> {
        match action {
            Action::Open(_) if self.name.is_none() && self.expect_error.is_none() => {
                Err("an open that is taken names its case with as".into())
            }
            Action::Append(_) if self.author.is_none() || self.event.is_none() => {
                Err("an append gives its author and event".into())
            }
            Action::RowsSince(_) if self.expect.is_none() => {
                Err("rows_since gives what it expects".into())
            }
            action => Ok(action),
        }
    }
}

/// Checks that each time an action gives is one.
fn check_times(action: &Action) -> Result<(), String> {
    match action {
        Action::Clock(text) => drop(time(text)?),
        Action::Listeners(want) => {
            for w in *want {
                for text in [&w.since, &w.heard, &w.doing_at].into_iter().flatten() {
                    time(text)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// What one vector's run has named so far.
#[derive(Default)]
struct Names {
    cases: BTreeMap<String, Key>,
    versions: BTreeMap<String, Version>,
}

impl Names {
    fn case(&self, name: &str) -> Result<Key, Failed> {
        self.cases
            .get(name)
            .copied()
            .ok_or_else(|| format!("no case named {name:?}").into())
    }

    fn version(&self, name: &str) -> Result<&Version, Failed> {
        self.versions
            .get(name)
            .ok_or_else(|| format!("no version named {name:?}").into())
    }

    /// The name of the case `key`.
    fn name_of(&self, key: Key) -> String {
        self.cases
            .iter()
            .find(|(_, k)| **k == key)
            .map_or_else(|| format!("<{key}>"), |(name, _)| name.clone())
    }
}

/// Runs the vector against the harness's store.
fn run<H>(harness: &H, vector: &Vector) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let mut names = Names::default();
    for (i, step) in vector.steps.iter().enumerate() {
        run_step(harness, &mut names, step).map_err(|failed| {
            let message = failed.message().unwrap_or("failed");
            Failed::from(format!("step {}: {message}", i + 1))
        })?;
    }
    Ok(())
}

/// The category of a failed write.
fn write_category<H: Harness>(
    harness: &H,
    error: &StoreError<<H::Store as Store>::Error>,
) -> Result<String, Failed>
where
    <H::Store as Store>::Error: Display,
{
    match error {
        StoreError::Refused(error) => Ok(error.category().as_str().to_owned()),
        StoreError::NoCase => Ok("no-case".to_owned()),
        StoreError::Store(error) => store_category(harness, error),
        _ => Err("a store error this runner does not know".into()),
    }
}

/// The category of an error of the store's own, or the failure it is.
fn store_category<H: Harness>(
    harness: &H,
    error: &<H::Store as Store>::Error,
) -> Result<String, Failed>
where
    <H::Store as Store>::Error: Display,
{
    harness
        .category(error)
        .map(str::to_owned)
        .ok_or_else(|| format!("the store failed: {error}").into())
}

/// Compares what an action gave with what the step expects.
fn outcome(step: &Step, got: Result<(), String>) -> Result<(), Failed> {
    match (got, &step.expect_error) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(want)) => Err(format!("taken, want {want}").into()),
        (Err(category), Some(want)) if category == *want => Ok(()),
        (Err(category), want) => Err(format!(
            "refused as {category}, want {}",
            want.as_deref().unwrap_or("it taken")
        )
        .into()),
    }
}

/// Runs one step: its action, checked against what it expects.
fn run_step<H>(harness: &H, names: &mut Names, step: &Step) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let store = harness.store();
    match step.action().map_err(Failed::from)? {
        Action::Clock(text) => {
            harness.set_clock(time(text)?);
            Ok(())
        }
        Action::Open(record) => run_open(harness, names, step, record),
        Action::Append(name) => run_append(harness, names, step, name),
        Action::Archive(name) | Action::Delete(name) => run_remove(harness, names, step, name),
        Action::Listen(listen) => {
            let listened = store.listen(
                &listen.wait,
                &listen.worker,
                &listen.labels,
                &listen.kinds,
                listen.doing.as_deref(),
            );
            stored(harness, step, listened)
        }
        Action::SetDoing(set) => stored(harness, step, store.set_doing(&set.worker, &set.text)),
        Action::Version(name) => run_version(store, names, step, name),
        Action::RowsSince(since) => run_rows_since(store, names, step, since),
        Action::Actors(want) => check_actors(store, want),
        Action::Listeners(want) => run_listeners(store, want),
    }
}

/// Compares what a call other than a write gave, taken or refused with
/// the category of the store's error, with what the step expects.
fn stored<H>(
    harness: &H,
    step: &Step,
    result: Result<(), <H::Store as Store>::Error>,
) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let got = match result {
        Ok(()) => Ok(()),
        Err(error) => Err(store_category(harness, &error)?),
    };
    outcome(step, got)
}

/// Opens the case `record`, naming it as the step says.
fn run_open<H>(
    harness: &H,
    names: &mut Names,
    step: &Step,
    record: &Map<String, Value>,
) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let store = harness.store();
    let writer = || Writer::new(step.writer.as_deref());
    let append = Append {
        author: step.author.clone().unwrap_or_else(|| "agent".into()),
        event: "open".into(),
        data: Value::Object(record.clone()).to_string(),
    };
    let got = match store.create(&append, &writer()) {
        Ok((key, _)) => {
            if let Some(name) = &step.name {
                names.cases.insert(name.clone(), key);
            }
            check_actor(store, key, &append.author, &writer())?;
            Ok(())
        }
        Err(error) => Err(write_category(harness, &error)?),
    };
    outcome(step, got)
}

/// Appends the step's event to the case named `name`.
fn run_append<H>(harness: &H, names: &Names, step: &Step, name: &str) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let store = harness.store();
    let writer = || Writer::new(step.writer.as_deref());
    let key = names.case(name)?;
    let append = Append {
        author: step.author.clone().unwrap_or_default(),
        event: step.event.clone().unwrap_or_default(),
        data: Value::Object(step.data.clone().unwrap_or_default()).to_string(),
    };
    let got = match store.append(&key, &append, &writer(), step.at_revision) {
        Ok(_) => {
            check_actor(store, key, &append.author, &writer())?;
            Ok(())
        }
        Err(error) => Err(write_category(harness, &error)?),
    };
    outcome(step, got)
}

/// Archives or deletes the case named `name`, as the step says.
fn run_remove<H>(harness: &H, names: &Names, step: &Step, name: &str) -> Result<(), Failed>
where
    H: Harness,
    <H::Store as Store>::Error: Display,
{
    let store = harness.store();
    let key = names.case(name)?;
    let removed = if step.archive.is_some() {
        store.archive(&key, step.at_revision)
    } else {
        store.delete(&key, step.at_revision)
    };
    let got = match removed {
        Ok(()) => Ok(()),
        Err(error) => Err(write_category(harness, &error)?),
    };
    outcome(step, got)
}

/// Names the store's version `name`, after checking it against the
/// versions the step says it is the same as or differs from.
fn run_version<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    names: &mut Names,
    step: &Step,
    name: &str,
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let version = store
        .version()
        .map_err(|e| format!("the store failed: {e}"))?;
    if let Some(same) = &step.same_as
        && *names.version(same)? != version
    {
        return Err(format!("the version moved from {same}: {version}").into());
    }
    for other in step.differs_from.iter().flatten() {
        if *names.version(other)? == version {
            return Err(format!("the version is still {other}'s: {version}").into());
        }
    }
    names.versions.insert(name.to_owned(), version);
    Ok(())
}

/// Checks what changed since the version named `since`, or since the
/// start, against what the step expects.
fn run_rows_since<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    names: &Names,
    step: &Step,
    since: Option<&str>,
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let since = since.map(|name| names.version(name)).transpose()?.cloned();
    let want = step.expect.as_ref().ok_or("no expect")?;
    let changes = store
        .rows_since(since.as_ref())
        .map_err(|e| format!("the store failed: {e}"))?;
    check_changes(store, names, since.as_ref(), &changes, want)
}

/// Checks the store's listeners and their waits against `want`.
fn run_listeners<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    want: &[ListenerWant],
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let listeners = store
        .listeners()
        .map_err(|e| format!("the store failed: {e}"))?;
    let waits = store
        .waits()
        .map_err(|e| format!("the store failed: {e}"))?;
    check_listeners(&listeners, &waits, want)
}

/// Checks the store's actors, by kind and name, against `want`.
fn check_actors<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    want: &[ActorWant],
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let actors = store
        .actors()
        .map_err(|e| format!("the store failed: {e}"))?;
    let got: Vec<(String, Option<String>)> = actors.into_iter().map(|a| (a.kind, a.name)).collect();
    let want: Vec<(String, Option<String>)> = want
        .iter()
        .map(|a| (a.kind.clone(), a.name.clone()))
        .collect();
    if got == want {
        Ok(())
    } else {
        Err(format!("the actors are {got:?}, want {want:?}").into())
    }
}

/// Checks that the event just stored in the case `key`, its last, carries
/// the actor the store keeps for `writer` writing as `author` (ST-12).
fn check_actor<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    key: Key,
    author: &str,
    writer: &Writer,
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let failed = |e: S::Error| format!("the store failed: {e}");
    let rows = store
        .rows(&key)
        .map_err(failed)?
        .ok_or("the case is not stored")?;
    let case = cases_core::Case::fold(&rows).map_err(|e| format!("the case does not fold: {e}"))?;
    let actor = case
        .events()
        .last()
        .and_then(cases_core::Event::actor)
        .ok_or("the event has no actor")?;
    let actors = store.actors().map_err(failed)?;
    let row = actors
        .iter()
        .find(|row| row.id == actor.id)
        .ok_or_else(|| format!("the event's actor {:?} is not in the table", actor.id))?;
    if row.kind == author && row.name.as_deref() == writer.name() {
        Ok(())
    } else {
        Err(format!(
            "the event's actor is {} {:?}, want {author} {:?}",
            row.kind,
            row.name,
            writer.name()
        )
        .into())
    }
}

/// Checks an answer of `rows_since` against what the step expects, as the
/// trait allows it: nothing changed, what changed, or every case.
fn check_changes<S: Store<Key = Key, Writer = Writer>>(
    store: &S,
    names: &Names,
    since: Option<&Version>,
    changes: &Changes<Key>,
    want: &ChangesWant,
) -> Result<(), Failed>
where
    S::Error: Display,
{
    let named = |cases: &[(Key, Vec<cases_core::Row>)]| -> Vec<String> {
        cases.iter().map(|(key, _)| names.name_of(*key)).collect()
    };
    // Every case given is as the store gives it now.
    for (key, rows) in changes.cases() {
        let now = store
            .rows(key)
            .map_err(|e| format!("the store failed: {e}"))?;
        if now.as_ref() != Some(rows) {
            return Err(format!("{}'s rows are not the store's", names.name_of(*key)).into());
        }
    }
    let version = store
        .version()
        .map_err(|e| format!("the store failed: {e}"))?;
    if *changes.version() != version {
        return Err(format!(
            "the answer's version is not the store's: {}",
            changes.version()
        )
        .into());
    }
    if since.is_none() && !matches!(changes, Changes::All { .. }) {
        return Err("with no version, the answer is every case".into());
    }
    match changes {
        // Nothing changed only when the store is still at the version asked
        // about.
        Changes::Unchanged { .. } if since != Some(&version) => {
            Err(format!("nothing changed, but the version moved to {version}").into())
        }
        Changes::Unchanged { .. } if !want.changed.is_empty() || !want.removed.is_empty() => {
            Err(format!(
                "nothing changed, want {:?} changed and {:?} removed",
                want.changed, want.removed
            )
            .into())
        }
        Changes::Since { cases, removed, .. } => {
            let got = named(cases);
            if got != want.changed {
                return Err(format!("{got:?} changed, want {:?}", want.changed).into());
            }
            // A store never gives a key twice.
            let keys: BTreeSet<Key> = removed.iter().copied().collect();
            if keys.len() != removed.len() || cases.iter().any(|(key, _)| keys.contains(key)) {
                return Err("a key is given twice".into());
            }
            let removed: BTreeSet<String> = removed.iter().map(|k| names.name_of(*k)).collect();
            let must: BTreeSet<String> = want.removed.iter().cloned().collect();
            let may: BTreeSet<String> = must
                .union(&want.removed_may.iter().cloned().collect())
                .cloned()
                .collect();
            if !must.is_subset(&removed) || !removed.is_subset(&may) {
                return Err(format!(
                    "{removed:?} removed, want {must:?}, with {:?} allowed",
                    want.removed_may
                )
                .into());
            }
            Ok(())
        }
        Changes::All { cases, .. } => {
            let got = named(cases);
            if got == want.live {
                Ok(())
            } else {
                Err(format!("every case is {got:?}, want {:?}", want.live).into())
            }
        }
        Changes::Unchanged { .. } => Ok(()),
    }
}

/// Checks the workers listening now, and that the waits listening now are
/// theirs.
fn check_listeners(
    got: &[cases_core::Listening],
    waits: &[cases_core::Wait],
    want: &[ListenerWant],
) -> Result<(), Failed> {
    let summary =
        |l: &cases_core::Listening| format!("{} {:?} doing {:?}", l.worker, l.labels, l.doing);
    let got_summary: Vec<String> = got.iter().map(summary).collect();
    if got.len() != want.len() {
        return Err(format!("listening: {got_summary:?}, want {} workers", want.len()).into());
    }
    // The waits, by worker and then by id, are the listeners' waits.
    let theirs: Vec<&cases_core::Wait> = got.iter().flat_map(|l| &l.waits).collect();
    if waits.iter().collect::<Vec<_>>() != theirs {
        return Err(format!("the waits {waits:?} are not the listeners' {theirs:?}").into());
    }
    let same = |want: &Option<String>, got: Option<Time>| -> Result<bool, Failed> {
        Ok(match want {
            Some(text) => Some(time(text)?) == got,
            None => true,
        })
    };
    for (l, w) in got.iter().zip(want) {
        let labels: BTreeSet<&String> = l.waits.iter().flat_map(|wait| &wait.labels).collect();
        let whole = !l.waits.is_empty()
            && l.waits.iter().all(|wait| wait.worker == l.worker)
            && l.labels.iter().collect::<BTreeSet<_>>() == labels
            && l.waits.iter().map(|wait| wait.since).min() == Some(l.since)
            && l.waits.iter().map(|wait| wait.heard).max() == Some(l.heard)
            && l.doing.is_some() == l.doing_at.is_some();
        if !whole {
            return Err(format!(
                "{}: its labels, times and doing line are not those of its waits: {l:?}",
                l.worker
            )
            .into());
        }
        let got_waits: Vec<(&str, &[String], &[String])> = l
            .waits
            .iter()
            .map(|wait| (wait.id.as_str(), &wait.labels[..], &wait.kinds[..]))
            .collect();
        let waits_match = w.waits.as_ref().is_none_or(|want| {
            let want: Vec<(&str, &[String], &[String])> = want
                .iter()
                .map(|wait| (wait.wait.as_str(), &wait.labels[..], &wait.kinds[..]))
                .collect();
            want == got_waits
        });
        let mismatch = l.worker != w.worker
            || l.labels != w.labels
            || l.doing != w.doing
            || !same(&w.since, Some(l.since))?
            || !same(&w.heard, Some(l.heard))?
            || !same(&w.doing_at, l.doing_at)?
            || !waits_match;
        if mismatch {
            return Err(format!(
                "listening: {got_summary:?} (since {}, heard {}, doing at {:?}, waits {got_waits:?}), want {w:?}",
                l.since, l.heard, l.doing_at
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
