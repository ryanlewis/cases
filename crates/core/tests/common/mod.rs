//! Reads the core vectors in `spec/vectors/` and checks them against the
//! format in `spec/vectors/README.md`.
//!
//! [`load_all`] gives every file's vectors, or every way the files break the
//! format. The core's runner reads the vectors through it, and so does the
//! store's, `crates/store-sqlite/tests/vectors.rs`, so each only sees vectors
//! of the documented shape. `stats.json` holds vectors of a shape of their
//! own, which [`load_stats`] gives, and [`load_all`] leaves out.

// Each test binary uses part of this module.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// The error categories, as `expect_error` and `problems` name them.
pub(crate) const CATEGORIES: &[&str] = &[
    "unknown",
    "author",
    "malformed",
    "invalid",
    "transition",
    "stale",
    "no-open",
];

/// The states of a case.
pub(crate) const STATES: &[&str] = &[
    "open",
    "answered",
    "pickedup",
    "closed",
    "withdrawn",
    "parked",
    "sent",
];

/// A field of a record, or of a property of the case, by the JSON it holds.
#[derive(Clone, Copy, Debug)]
enum Field {
    /// A string.
    Text,
    /// A list of strings.
    Texts,
    /// A whole number, of either sign.
    Whole,
    /// A list of whole numbers.
    Wholes,
    /// `true` or `false`.
    Flag,
    /// An RFC 3339 time, with an offset or `Z`.
    Time,
    /// A list of approval rows.
    Rows,
    /// A list of verdicts on approval rows.
    Verdicts,
    /// An actor.
    Actor,
}

use Field::{Actor, Flag, Rows, Text, Texts, Time, Verdicts, Whole, Wholes};

const OPEN: &[(&str, Field)] = &[
    ("kind", Text),
    ("urgency", Text),
    ("title", Text),
    ("body", Text),
    ("options", Texts),
    ("recommend", Whole),
    ("rows", Rows),
    ("landed", Flag),
    ("left_out", Texts),
    ("links", Texts),
    ("labels", Texts),
    ("worker", Text),
    ("brief", Text),
    ("context", Text),
    ("for", Text),
    ("re", Text),
    ("actor", Actor),
    ("opened_at", Time),
];
const AMEND: &[(&str, Field)] = &[
    ("title", Text),
    ("body", Text),
    ("options", Texts),
    ("recommend", Whole),
    ("rows", Rows),
    ("landed", Flag),
    ("left_out", Texts),
    ("links", Texts),
    ("drop_links", Texts),
    ("labels", Texts),
    ("context", Text),
    ("reopen", Flag),
    ("actor", Actor),
    ("amended_at", Time),
];
const ANSWER: &[(&str, Field)] = &[
    ("choice", Whole),
    ("other", Flag),
    ("rows", Verdicts),
    ("signoff", Text),
    ("briefing", Text),
    ("file", Wholes),
    ("text", Text),
    ("drop", Flag),
    ("ack", Flag),
    ("note", Text),
    ("amend_seq", Whole),
    ("actor", Actor),
    ("answered_at", Time),
];
const PICKUP: &[(&str, Field)] = &[("by", Text), ("actor", Actor), ("picked_up_at", Time)];
const NOTE: &[(&str, Field)] = &[
    ("body", Text),
    ("reopen", Flag),
    ("actor", Actor),
    ("noted_at", Time),
];
const CLOSE: &[(&str, Field)] = &[
    ("outcome", Text),
    ("links", Texts),
    ("expired", Flag),
    ("actor", Actor),
    ("closed_at", Time),
];
const WITHDRAW: &[(&str, Field)] = &[("reason", Text), ("actor", Actor), ("withdrawn_at", Time)];
const PARK: &[(&str, Field)] = &[("note", Text), ("actor", Actor), ("parked_at", Time)];
const RESUME: &[(&str, Field)] = &[("actor", Actor), ("resumed_at", Time)];

/// The fields of each event's record. An appended record names only these,
/// and none of them is null.
const RECORDS: &[(&str, &[(&str, Field)])] = &[
    ("open", OPEN),
    ("amend", AMEND),
    ("answer", ANSWER),
    ("pickup", PICKUP),
    ("note", NOTE),
    ("close", CLOSE),
    ("withdraw", WITHDRAW),
    ("park", PARK),
    ("resume", RESUME),
];

const ROW: &[(&str, Field)] = &[
    ("id", Text),
    ("label", Text),
    ("note", Text),
    ("script", Text),
    ("link", Text),
    ("by", Text),
];
const VERDICT: &[(&str, Field)] = &[("id", Text), ("verdict", Text), ("note", Text)];
const ACTOR: &[(&str, Field)] = &[("id", Text)];

/// The properties of the case that `expect` may give.
#[derive(Clone, Copy, Debug)]
enum Property {
    State,
    Count,
    /// A time in UTC, or null.
    Time,
    /// A value of this field, with its times in UTC.
    Field(Field),
    /// A record with these fields, or null.
    Record(&'static [(&'static str, Field)]),
    Events,
    Problems,
    /// How the case changed since the form was drawn.
    Since,
}

const PROPERTIES: &[(&str, Property)] = &[
    ("state", Property::State),
    ("revision", Property::Count),
    ("amend_seq", Property::Count),
    ("updated_at", Property::Time),
    ("kind", Property::Field(Text)),
    ("urgency", Property::Field(Text)),
    ("title", Property::Field(Text)),
    ("body", Property::Field(Text)),
    ("options", Property::Field(Texts)),
    ("recommend", Property::Count),
    ("rows", Property::Field(Rows)),
    ("landed", Property::Field(Flag)),
    ("left_out", Property::Field(Texts)),
    ("links", Property::Field(Texts)),
    ("labels", Property::Field(Texts)),
    ("worker", Property::Field(Text)),
    ("brief", Property::Field(Text)),
    ("context", Property::Field(Text)),
    ("for", Property::Field(Text)),
    ("re", Property::Field(Text)),
    ("actor", Property::Record(ACTOR)),
    ("opened_at", Property::Time),
    ("answer", Property::Record(ANSWER)),
    ("pickup", Property::Record(PICKUP)),
    ("park", Property::Record(PARK)),
    ("close", Property::Record(CLOSE)),
    ("events", Property::Events),
    ("problems", Property::Problems),
    ("since", Property::Since),
];

/// The properties of an element of `events`.
const EVENT_PROPERTIES: &[&str] = &[
    "seq", "author", "event", "file", "from", "at", "actor", "data",
];

/// The properties of an element of `problems`.
const PROBLEM_PROPERTIES: &[&str] = &["seq", "file", "category"];

/// One file of vectors.
#[derive(Debug)]
pub(crate) struct VectorFile {
    /// The file's name without `.json`, such as `transitions`.
    pub(crate) name: String,
    pub(crate) vectors: Vec<Vector>,
}

/// One vector: a case's stored rows, perhaps an event to append, and the
/// case or the error that results.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Vector {
    pub(crate) description: String,
    pub(crate) events: Vec<Row>,
    pub(crate) append: Option<Append>,
    /// For a freshness vector: the form was drawn from the case the first
    /// this many rows fold to.
    pub(crate) drawn_after: Option<usize>,
    /// For a freshness vector: the form was drawn at this revision and seq.
    pub(crate) drawn_at: Option<DrawnAt>,
    /// Properties of the case afterwards, as JSON values.
    pub(crate) expect: Option<Map<String, Value>>,
    pub(crate) expect_error: Option<ExpectError>,
}

/// A stored event row.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) seq: u64,
    pub(crate) author: String,
    pub(crate) event: String,
    /// The record as a JSON object, where its bytes do not matter.
    pub(crate) data: Option<Map<String, Value>>,
    /// The record's exact bytes.
    pub(crate) raw: Option<String>,
}

impl Row {
    /// The record as stored: `raw` as given, or `data` written as JSON.
    pub(crate) fn record(&self) -> String {
        match (&self.raw, &self.data) {
            (Some(raw), _) => raw.clone(),
            (None, Some(data)) => Value::Object(data.clone()).to_string(),
            (None, None) => unreachable!("the loader refuses a row with no record"),
        }
    }
}

/// An event to write to the case the rows fold to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Append {
    pub(crate) author: String,
    pub(crate) event: String,
    pub(crate) data: Map<String, Value>,
    /// The writer's clock.
    pub(crate) now: String,
    /// The revision the writer read the case at.
    pub(crate) at_revision: Option<u64>,
}

/// The revision and seq a form was drawn at, as a freshness vector gives
/// them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DrawnAt {
    pub(crate) revision: u64,
    pub(crate) seq: u64,
}

/// How a case changed since a form was drawn, as `expect.since` names it.
pub(crate) const SINCE: &[&str] = &["unchanged", "moved", "changed"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectError {
    pub(crate) category: String,
}

/// The file of stats vectors, without `.json`: its vectors have a shape of
/// their own (the README's "Stats").
pub(crate) const STATS: &str = "stats";

/// One stats vector: the rows of some cases, the clock and the number of
/// days, and the stats they give.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatsVector {
    pub(crate) description: String,
    /// Each case's stored rows. Each case folds.
    pub(crate) cases: Vec<Vec<Row>>,
    pub(crate) now: String,
    pub(crate) days: u16,
    pub(crate) expect: StatsExpect,
}

/// The stats a stats vector expects.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatsExpect {
    pub(crate) opened: Vec<u32>,
    pub(crate) answered: Vec<u32>,
    /// The median time to answer in seconds, or `null` when there is none.
    pub(crate) median: Option<u64>,
}

/// The directory that holds the vectors.
pub(crate) fn vector_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/vectors")
}

/// Reads every `.json` file in [`vector_dir`], in name order.
pub(crate) fn load_all() -> Result<Vec<VectorFile>, Vec<String>> {
    let dir = vector_dir();
    let entries = fs::read_dir(&dir).map_err(|e| vec![format!("{}: {e}", dir.display())])?;
    let mut paths: Vec<PathBuf> = entries
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| vec![format!("{}: {e}", dir.display())])?;
    paths.retain(|p| {
        p.extension().is_some_and(|x| x == "json") && p.file_stem() != Some(STATS.as_ref())
    });
    paths.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
    if paths.is_empty() {
        return Err(vec![format!("{}: no vector files", dir.display())]);
    }
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        match fs::read_to_string(&path) {
            Ok(text) => match parse(&name, &text) {
                Ok(vectors) => files.push(VectorFile { name, vectors }),
                Err(e) => errors.extend(e),
            },
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if errors.is_empty() {
        Ok(files)
    } else {
        Err(errors)
    }
}

/// Reads `stats.json`, and checks its vectors.
pub(crate) fn load_stats() -> Result<Vec<StatsVector>, Vec<String>> {
    let path = vector_dir().join(format!("{STATS}.json"));
    let text = fs::read_to_string(&path).map_err(|e| vec![format!("{STATS}: {e}")])?;
    parse_stats(STATS, &text)
}

/// Reads the text of a file of stats vectors, named `name` in errors, and
/// checks its vectors: each case's rows as a vector's `events` are checked,
/// `now` a time in UTC, one count a day in each list, and a `median` given,
/// as `null` when there is none.
pub(crate) fn parse_stats(name: &str, text: &str) -> Result<Vec<StatsVector>, Vec<String>> {
    let Unique(value) = serde_json::from_str(text).map_err(|e| vec![format!("{name}: {e}")])?;
    let Value::Array(list) = value else {
        return Err(vec![format!("{name}: the file is not one list")]);
    };
    let mut errors = Vec::new();
    let mut vectors = Vec::new();
    for (i, v) in list.into_iter().enumerate() {
        let at = format!("{name}: vector {}", i + 1);
        let before = errors.len();
        if let Value::Object(object) = &v {
            for (key, value) in object {
                if value.is_null() {
                    errors.push(format!("{at}: {key:?} is null"));
                }
            }
            for (c, rows) in object
                .get("cases")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                no_null_rows(rows, &format!("{at}: cases[{c}]"), &mut errors);
            }
            if object
                .get("expect")
                .and_then(Value::as_object)
                .is_some_and(|expect| !expect.contains_key("median"))
            {
                errors.push(format!(
                    "{at}: expect: needs median, null when there is none"
                ));
            }
        }
        if errors.len() > before {
            continue;
        }
        match serde_json::from_value::<StatsVector>(v) {
            Ok(v) => vectors.push(v),
            Err(e) => errors.push(format!("{at}: {e}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if vectors.is_empty() {
        errors.push(format!("{name}: no vectors"));
    }
    let mut seen = BTreeSet::new();
    for (i, v) in vectors.iter().enumerate() {
        let at = format!("{name}: vector {} ({:?})", i + 1, v.description);
        if v.description.is_empty() || !seen.insert(v.description.as_str()) {
            errors.push(format!("{at}: the description is empty or not unique"));
        }
        for (c, rows) in v.cases.iter().enumerate() {
            check_rows(rows, &format!("{at}: cases[{c}]"), &mut errors);
        }
        if !is_utc_time(&v.now) {
            errors.push(format!("{at}: now {:?} is not a time in UTC", v.now));
        }
        let days = usize::from(v.days);
        for (key, counts) in [
            ("opened", &v.expect.opened),
            ("answered", &v.expect.answered),
        ] {
            if counts.len() != days {
                errors.push(format!(
                    "{at}: expect: {key} has {} counts, not one for each of the {days} days",
                    counts.len()
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(vectors)
    } else {
        Err(errors)
    }
}

/// Refuses a null key of a row in `rows`, a JSON list of rows.
fn no_null_rows(rows: &Value, at: &str, errors: &mut Vec<String>) {
    for (i, row) in rows.as_array().into_iter().flatten().enumerate() {
        for (key, value) in row.as_object().into_iter().flatten() {
            if value.is_null() {
                errors.push(format!("{at}[{i}]: {key:?} is null"));
            }
        }
    }
}

/// Reads one file's text, named `name` in errors, and checks its vectors.
pub(crate) fn parse(name: &str, text: &str) -> Result<Vec<Vector>, Vec<String>> {
    // serde_json keeps the last of two equal keys and says nothing, so the
    // text is read once to refuse them, and once more into the types.
    let Unique(value) = serde_json::from_str(text).map_err(|e| vec![format!("{name}: {e}")])?;
    let Value::Array(list) = value else {
        return Err(vec![format!("{name}: the file is not one list")]);
    };
    let mut errors = Vec::new();
    let mut vectors = Vec::new();
    for (i, v) in list.into_iter().enumerate() {
        let at = format!("{name}: vector {}", i + 1);
        let before = errors.len();
        no_null_keys(&v, &at, &mut errors);
        if errors.len() > before {
            continue;
        }
        match serde_json::from_value::<Vector>(v) {
            Ok(v) => vectors.push(v),
            Err(e) => errors.push(format!("{at}: {e}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if vectors.is_empty() {
        errors.push(format!("{name}: no vectors"));
    }
    let mut seen = BTreeSet::new();
    for (i, v) in vectors.iter().enumerate() {
        let at = format!("{name}: vector {} ({:?})", i + 1, v.description);
        if v.description.is_empty() || !seen.insert(v.description.as_str()) {
            errors.push(format!("{at}: the description is empty or not unique"));
        }
        check_vector(v, &at, &mut errors);
    }
    if errors.is_empty() {
        Ok(vectors)
    } else {
        Err(errors)
    }
}

/// Refuses a null key of a vector, of a row or of `append`. Null is left
/// to the records, and to `expect`, where it means none.
fn no_null_keys(v: &Value, at: &str, errors: &mut Vec<String>) {
    let Value::Object(v) = v else { return };
    let mut objects = vec![(at.to_owned(), v)];
    if let Some(Value::Array(rows)) = v.get("events") {
        for (i, row) in rows.iter().enumerate() {
            if let Value::Object(row) = row {
                objects.push((format!("{at}: events[{i}]"), row));
            }
        }
    }
    if let Some(Value::Object(a)) = v.get("append") {
        objects.push((format!("{at}: append"), a));
    }
    for (at, object) in objects {
        for (key, value) in object {
            if value.is_null() {
                errors.push(format!("{at}: {key:?} is null"));
            }
        }
    }
}

/// Checks a case's rows: each `seq` at least 1 and above the one before,
/// and each row with one of `data` and `raw`. Gives the seqs.
fn check_rows(rows: &[Row], at: &str, errors: &mut Vec<String>) -> Vec<u64> {
    let mut seqs = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let at = format!("{at}[{i}]");
        if row.seq < 1 || seqs.last().is_some_and(|&last| row.seq <= last) {
            errors.push(format!("{at}: seq {} is below 1 or does not rise", row.seq));
        }
        seqs.push(row.seq);
        if row.data.is_some() == row.raw.is_some() {
            errors.push(format!("{at}: needs one of data and raw"));
        }
    }
    seqs
}

fn check_vector(v: &Vector, at: &str, errors: &mut Vec<String>) {
    let seqs = check_rows(&v.events, &format!("{at}: events"), errors);
    // The rows the case has afterwards, as (seq, author, event): the stored
    // ones, and the appended one when the append is accepted.
    let mut rows: Vec<(u64, &str, &str)> = v
        .events
        .iter()
        .map(|r| (r.seq, r.author.as_str(), r.event.as_str()))
        .collect();
    let mut appended = None;
    if let Some(a) = &v.append {
        check_append(a, v, &format!("{at}: append"), errors);
        let seq = seqs.last().map_or(1, |last| last + 1);
        appended = Some(seq);
        rows.push((seq, a.author.as_str(), a.event.as_str()));
    }
    match (&v.expect, &v.expect_error) {
        (Some(expect), None) => {
            check_expect(expect, &rows, appended, &format!("{at}: expect"), errors);
        }
        (None, Some(e)) => {
            if !CATEGORIES.contains(&e.category.as_str()) {
                errors.push(format!(
                    "{at}: expect_error: {:?} is not a category",
                    e.category
                ));
            } else if v.append.is_none() && e.category != "no-open" {
                errors.push(format!(
                    "{at}: expect_error: a replay fails only with no-open, not {:?}",
                    e.category
                ));
            }
        }
        _ => errors.push(format!("{at}: needs one of expect and expect_error")),
    }
    check_drawn(v, at, errors);
}

/// Checks a freshness vector: one of `drawn_after` and `drawn_at`, no
/// append, and an `expect` with `since`. `drawn_after` names at least the
/// open row and at most every row. Only such a vector gives `since`.
fn check_drawn(v: &Vector, at: &str, errors: &mut Vec<String>) {
    let drawn = v.drawn_after.is_some() || v.drawn_at.is_some();
    let since = v.expect.as_ref().is_some_and(|e| e.contains_key("since"));
    if !drawn {
        if since {
            errors.push(format!(
                "{at}: expect gives since, with no drawn_after or drawn_at"
            ));
        }
        return;
    }
    if v.drawn_after.is_some() && v.drawn_at.is_some() {
        errors.push(format!(
            "{at}: needs one of drawn_after and drawn_at, not both"
        ));
    }
    if v.append.is_some() {
        errors.push(format!("{at}: a freshness vector has no append"));
    }
    if !since {
        errors.push(format!("{at}: a freshness vector needs expect with since"));
    }
    if let Some(n) = v.drawn_after
        && !(1..=v.events.len()).contains(&n)
    {
        errors.push(format!(
            "{at}: drawn_after {n} is not from 1 to the {} rows",
            v.events.len()
        ));
    }
}

fn check_append(a: &Append, v: &Vector, at: &str, errors: &mut Vec<String>) {
    let Some((_, fields)) = RECORDS.iter().find(|(event, _)| *event == a.event) else {
        errors.push(format!(
            "{at}: {:?} is not an event a writer takes",
            a.event
        ));
        return;
    };
    if a.event == "open" && (!v.events.is_empty() || a.at_revision.is_some()) {
        errors.push(format!(
            "{at}: an open is appended only to no events, and with no at_revision"
        ));
    }
    if !is_utc_time(&a.now) {
        errors.push(format!("{at}: now {:?} is not a time in UTC", a.now));
    }
    check_record(&a.data, fields, is_time, &format!("{at}: data"), errors);
}

/// Checks a record a writer takes: only its event's fields, each of its type.
/// `time` says which strings are times.
fn check_record(
    record: &Map<String, Value>,
    fields: &[(&str, Field)],
    time: fn(&str) -> bool,
    at: &str,
    errors: &mut Vec<String>,
) {
    for (key, value) in record {
        match fields.iter().find(|(name, _)| name == key) {
            None => errors.push(format!("{at}: {key:?} is not a field of the record")),
            Some((_, field)) => {
                if !is_field(value, *field, time, &format!("{at}.{key}"), errors) {
                    errors.push(format!("{at}: {key:?} is not {field:?}"));
                }
            }
        }
    }
}

fn is_field(
    value: &Value,
    field: Field,
    time: fn(&str) -> bool,
    at: &str,
    errors: &mut Vec<String>,
) -> bool {
    match field {
        Text => value.is_string(),
        Texts => value
            .as_array()
            .is_some_and(|l| l.iter().all(Value::is_string)),
        Whole => value.is_i64() || value.is_u64(),
        Wholes => value
            .as_array()
            .is_some_and(|l| l.iter().all(|n| n.is_i64() || n.is_u64())),
        Flag => value.is_boolean(),
        Time => value.as_str().is_some_and(time),
        Rows => is_records(value, ROW, time, at, errors),
        Verdicts => is_records(value, VERDICT, time, at, errors),
        Actor => is_record(value, ACTOR, time, at, errors),
    }
}

/// An object, checked as a record with these fields.
fn is_record(
    value: &Value,
    fields: &[(&str, Field)],
    time: fn(&str) -> bool,
    at: &str,
    errors: &mut Vec<String>,
) -> bool {
    let Value::Object(record) = value else {
        return false;
    };
    check_record(record, fields, time, at, errors);
    true
}

/// A list of objects, each checked as a record with these fields.
fn is_records(
    value: &Value,
    fields: &[(&str, Field)],
    time: fn(&str) -> bool,
    at: &str,
    errors: &mut Vec<String>,
) -> bool {
    let Some(list) = value.as_array() else {
        return false;
    };
    // Every element is checked, so one that is not an object does not hide
    // the errors of those after it.
    list.iter().enumerate().fold(true, |fits, (i, x)| {
        is_record(x, fields, time, &format!("{at}[{i}]"), errors) && fits
    })
}

fn check_expect(
    expect: &Map<String, Value>,
    rows: &[(u64, &str, &str)],
    appended: Option<u64>,
    at: &str,
    errors: &mut Vec<String>,
) {
    if let Some(revision) = expect.get("revision").and_then(Value::as_u64)
        && revision != rows.len() as u64
    {
        errors.push(format!(
            "{at}: revision {revision} is not the number of rows, {}",
            rows.len()
        ));
    }
    if let (Some(Value::Array(events)), Some(Value::Array(problems))) =
        (expect.get("events"), expect.get("problems"))
        && events.len() + problems.len() != rows.len()
    {
        errors.push(format!(
            "{at}: {} events and {} problems are not the {} rows",
            events.len(),
            problems.len(),
            rows.len()
        ));
    }
    for (key, value) in expect {
        let Some((_, property)) = PROPERTIES.iter().find(|(name, _)| name == key) else {
            errors.push(format!("{at}: {key:?} is not a property of the case"));
            continue;
        };
        let at = format!("{at}: {key}");
        let fits = match property {
            Property::State => value.as_str().is_some_and(|s| STATES.contains(&s)),
            Property::Count => value.is_u64(),
            Property::Time => value.is_null() || value.as_str().is_some_and(is_utc_time),
            Property::Field(field) => is_field(value, *field, is_utc_time, &at, errors),
            Property::Record(fields) => {
                value.is_null() || is_record(value, fields, is_utc_time, &at, errors)
            }
            Property::Events => check_elements(value, EVENT_PROPERTIES, rows, &at, errors),
            Property::Since => value.as_str().is_some_and(|s| SINCE.contains(&s)),
            Property::Problems => {
                // An appended row is never skipped: a refused append is an error.
                let stored = match appended {
                    Some(_) => &rows[..rows.len() - 1],
                    None => rows,
                };
                check_elements(value, PROBLEM_PROPERTIES, stored, &at, errors)
            }
        };
        if !fits {
            errors.push(format!("{at}: {value} is not a value of {property:?}"));
        }
    }
}

/// Checks the elements of `events` or `problems`: each names only the
/// properties it has, a `seq` it gives is one of `rows`, in rising order,
/// and its `author`, `event` and `file` are that row's.
fn check_elements(
    value: &Value,
    properties: &[&str],
    rows: &[(u64, &str, &str)],
    at: &str,
    errors: &mut Vec<String>,
) -> bool {
    let Some(list) = value.as_array() else {
        return false;
    };
    let mut last = 0;
    for (i, element) in list.iter().enumerate() {
        let Value::Object(element) = element else {
            return false;
        };
        let at = format!("{at}[{i}]");
        let row = element
            .get("seq")
            .and_then(Value::as_u64)
            .and_then(|seq| rows.iter().find(|(s, _, _)| *s == seq));
        for (key, x) in element {
            if !properties.contains(&key.as_str()) {
                errors.push(format!("{at}: {key:?} is not a property of an element"));
            }
            let fits = match key.as_str() {
                "seq" => x.as_u64().is_some_and(|seq| {
                    let rises = seq > last;
                    last = seq;
                    row.is_some() && rises
                }),
                "category" => x.as_str().is_some_and(|c| CATEGORIES.contains(&c)),
                "from" => x.is_null() || x.as_str().is_some_and(|s| STATES.contains(&s)),
                "at" => x.is_null() || x.as_str().is_some_and(is_utc_time),
                "actor" => x.is_null() || is_record(x, ACTOR, is_utc_time, &at, errors),
                "author" => x.as_str().is_some_and(|a| row.is_none_or(|r| r.1 == a)),
                "event" => x.as_str().is_some_and(|e| row.is_none_or(|r| r.2 == e)),
                "file" => x.as_str().is_some_and(|f| {
                    row.is_none_or(|(seq, author, event)| {
                        f == format!("{seq:04}-{author}-{event}.json")
                    })
                }),
                _ => true,
            };
            if !fits {
                errors.push(format!("{at}: {key} {x} does not fit"));
            }
        }
    }
    true
}

/// A time as the vectors write one: RFC 3339 in UTC, ending in `Z`, with
/// only the fractional digits it needs.
fn is_utc_time(s: &str) -> bool {
    is_time(s)
        && s.strip_suffix('Z').is_some_and(|rest| {
            rest.split_once('.')
                .is_none_or(|(_, fraction)| !fraction.ends_with('0'))
        })
}

/// A time an appended record may give: RFC 3339 with a `T`, and `Z` or an
/// offset. A stored row, not an append, tries the other spellings.
fn is_time(s: &str) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let (rest, offset) = match s.strip_suffix('Z') {
        Some(rest) => (rest, "+00:00"),
        None if s.len() > 6 => s.split_at(s.len() - 6),
        None => return false,
    };
    let offset_fits = offset.len() == 6
        && matches!(&offset[..1], "+" | "-")
        && is_digits(&offset[1..3])
        && &offset[3..4] == ":"
        && is_digits(&offset[4..]);
    let time_fits = match rest.split_once('.') {
        None => is_date_time(rest),
        Some((whole, fraction)) => is_date_time(whole) && is_digits(fraction),
    };
    offset_fits && time_fits
}

/// `YYYY-MM-DDTHH:MM:SS`, digits and separators only.
fn is_date_time(s: &str) -> bool {
    s.len() == 19
        && s.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == 'T',
            13 | 16 => c == ':',
            _ => c.is_ascii_digit(),
        })
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A JSON value in which no object gives a key twice.
struct Unique(Value);

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueVisitor).map(Unique)
    }
}

struct UniqueVisitor;

impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("a number JSON cannot hold"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut list = Vec::new();
        while let Some(Unique(v)) = seq.next_element()? {
            list.push(v);
        }
        Ok(Value::Array(list))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("{key:?} is given twice")));
            }
            let Unique(v) = map.next_value()?;
            object.insert(key, v);
        }
        Ok(Value::Object(object))
    }
}
