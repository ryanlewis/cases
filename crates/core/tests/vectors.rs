//! Runs every vector in `spec/vectors/` against `cases-core`, as
//! `spec/vectors/README.md` says under "Running a vector". Each vector is a
//! test of its own, named by its file and its description, so a failure names
//! the vector.
//!
//! One more test checks that `transitions.json` still tries every event, by
//! each author that may write it, from every state of a stuck case and of a
//! message, and an open and an amend of each kind by the side that does not
//! open it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::collections::BTreeSet;

use cases_core::{Append, Author, Case, Error, Event, EventType, Kind, Row, Since, State, Time};
use libtest_mimic::{Arguments, Failed, Trial};
use serde::Serialize;
use serde_json::{Map, Value, json};

fn main() {
    let args = Arguments::from_args();
    let mut trials = vec![Trial::test(
        "transitions.json tries every event by each author from every state of a stuck case and of a message, and each kind's open and amend by the other side",
        coverage,
    )];
    match common::load_all() {
        Ok(files) => {
            for file in files {
                for vector in file.vectors {
                    let name = format!("{}.json: {}", file.name, vector.description);
                    trials.push(Trial::test(name, move || run(&vector)));
                }
            }
        }
        Err(errors) => {
            let errors = errors.join("\n");
            trials.push(Trial::test("the vectors load", move || Err(errors.into())));
        }
    }
    match common::load_stats() {
        Ok(vectors) => {
            for vector in vectors {
                let name = format!("{}.json: {}", common::STATS, vector.description);
                trials.push(Trial::test(name, move || run_stats(&vector)));
            }
        }
        Err(errors) => {
            let errors = errors.join("\n");
            trials.push(Trial::test("the stats vectors load", move || {
                Err(errors.into())
            }));
        }
    }
    libtest_mimic::run(&args, trials).exit();
}

/// Runs one stats vector, as the README says under "Stats": folds each
/// case's rows, and compares the stats of the cases at `now` over `days`
/// with `expect`.
fn run_stats(vector: &common::StatsVector) -> Result<(), Failed> {
    let now: Time = vector
        .now
        .parse()
        .map_err(|e| format!("now does not read: {e}"))?;
    let cases = vector
        .cases
        .iter()
        .enumerate()
        .map(|(i, rows)| {
            let rows: Vec<Row> = rows.iter().map(row).collect();
            Case::fold(&rows).map_err(|e| format!("cases[{i}] does not fold: {e}"))
        })
        .collect::<Result<Vec<Case>, String>>()?;
    let stats = cases_core::stats(&cases, now, vector.days);
    let median = stats.median().map(|median| median.as_secs());
    let mut wrong = Vec::new();
    if stats.opened() != vector.expect.opened.as_slice() {
        wrong.push(format!(
            "opened is {:?}, want {:?}",
            stats.opened(),
            vector.expect.opened
        ));
    }
    if stats.answered() != vector.expect.answered.as_slice() {
        wrong.push(format!(
            "answered is {:?}, want {:?}",
            stats.answered(),
            vector.expect.answered
        ));
    }
    if median != vector.expect.median {
        wrong.push(format!(
            "median is {median:?}, want {:?}",
            vector.expect.median
        ));
    }
    if stats
        .median()
        .is_some_and(|median| median.subsec_nanos() != 0)
    {
        wrong.push(format!(
            "median {:?} is not in whole seconds",
            stats.median()
        ));
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("\n").into())
    }
}

/// The stored row a vector's row gives.
fn row(row: &common::Row) -> Row {
    Row {
        seq: row.seq,
        author: row.author.clone(),
        event: row.event.clone(),
        data: row.record(),
    }
}

/// Runs one vector: a replay folds the rows, and an append writes to the case
/// they fold to. Then compares the case with `expect`, or the error's
/// category with `expect_error`.
fn run(vector: &common::Vector) -> Result<(), Failed> {
    let rows: Vec<Row> = vector.events.iter().map(row).collect();
    let outcome = match &vector.append {
        None => Case::fold(&rows),
        Some(append) => write(&rows, append)?,
    };
    match (outcome, &vector.expect, &vector.expect_error) {
        (Ok(case), Some(expect), None) => {
            let since = drawn(vector, &rows)?.map(|(revision, seq)| since(&case, revision, seq));
            check(&case, since.transpose()?, expect)
        }
        (Ok(_), None, Some(want)) => Err(format!("accepted, want {}", want.category).into()),
        (Err(error), None, Some(want)) if error.category().as_str() == want.category => Ok(()),
        (Err(error), None, Some(want)) => Err(format!(
            "refused as {}, want {}: {error}",
            error.category(),
            want.category
        )
        .into()),
        (Err(error), Some(_), None) => {
            Err(format!("refused as {}, want a case: {error}", error.category()).into())
        }
        _ => unreachable!("the loader gives every vector one of expect and expect_error"),
    }
}

/// Does what the writer does, in the README's steps for a vector with
/// `append`: gives the case the append leaves, or the error it is refused
/// with. The outer error is the runner's own failure.
fn write(rows: &[Row], append: &common::Append) -> Result<Result<Case, Error>, Failed> {
    let now: Time = append
        .now
        .parse()
        .map_err(|e| format!("now does not read: {e}"))?;
    let new = Append {
        author: append.author.clone(),
        event: append.event.clone(),
        data: Value::Object(append.data.clone()).to_string(),
    };
    // 1. An open starts from an empty case. Any other event goes to the case
    // the rows fold to, and a case that does not fold refuses it as no-open.
    // 2 to 5. The writer checks at_revision, stamps the time, numbers the row
    // and checks the event.
    let (case, new_row) = if append.event == "open" {
        match Case::open(&new, now, None) {
            Ok(opened) => opened,
            Err(error) => return Ok(Err(error)),
        }
    } else {
        let mut case = match Case::fold(rows) {
            Ok(case) => case,
            Err(error) => return Ok(Err(error)),
        };
        let before = case.clone();
        match case.append(&new, now, None, append.at_revision) {
            Ok(new_row) => (case, new_row),
            Err(error) if case == before => return Ok(Err(error)),
            Err(error) => {
                return Err(format!(
                    "the append, refused as {}, changed the case\nfrom {before:#?}\nto {case:#?}",
                    error.category()
                )
                .into());
            }
        }
    };
    // 6. The rows with the new one fold to the same case.
    let mut all = rows.to_vec();
    all.push(new_row);
    match Case::fold(&all) {
        Ok(folded) if folded == case => Ok(Ok(case)),
        Ok(folded) => Err(format!(
            "the rows with the new one fold to\n{folded:#?}\nnot the case the append left\n{case:#?}"
        )
        .into()),
        Err(error) => Err(format!("the rows with the new one do not fold: {error}").into()),
    }
}

/// The revision and seq a freshness vector's form was drawn at, as the
/// README says under "Freshness": `drawn_at` as given, or those of the case
/// the first `drawn_after` rows fold to. `None` for any other vector.
fn drawn(vector: &common::Vector, rows: &[Row]) -> Result<Option<(u64, u64)>, Failed> {
    if let Some(at) = &vector.drawn_at {
        return Ok(Some((at.revision, at.seq)));
    }
    let Some(n) = vector.drawn_after else {
        return Ok(None);
    };
    let case = Case::fold(&rows[..n]).map_err(|e| format!("the drawn case does not fold: {e}"))?;
    Ok(Some((case.revision(), case.last_seq())))
}

/// How the case changed since `revision` and `seq`, once `unchanged_since`
/// is checked to agree with `since`.
fn since(case: &Case, revision: u64, seq: u64) -> Result<Since, Failed> {
    let since = case.since(revision, seq);
    if case.unchanged_since(revision, seq) != (since == Since::Unchanged) {
        return Err(format!("unchanged_since disagrees with since, {since:?}").into());
    }
    Ok(since)
}

/// Compares the case with `expect`, as the README says under "The case":
/// each property given is compared as a whole, except `events` and
/// `problems`, whose elements are compared on the properties they give.
/// `since`, for a freshness vector, is how the case changed since its form
/// was drawn.
fn check(case: &Case, since: Option<Since>, expect: &Map<String, Value>) -> Result<(), Failed> {
    let mut have = properties(case);
    if let Some(since) = since {
        have.insert("since".into(), since.as_str().into());
    }
    let mut wrong = Vec::new();
    for (key, want) in expect {
        match (key.as_str(), have.get(key)) {
            (_, None) => wrong.push(format!("{key} is not a property of the case")),
            ("events" | "problems", Some(have)) => elements(key, have, want, &mut wrong),
            (_, Some(have)) if have != want => wrong.push(format!("{key} is {have}, want {want}")),
            _ => {}
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("\n").into())
    }
}

/// Compares the elements of `events` or `problems`. The lists must be as
/// long, and each element must have the properties the vector gives it,
/// where `null` means the property is absent.
fn elements(key: &str, have: &Value, want: &Value, wrong: &mut Vec<String>) {
    let (Some(have), Some(want)) = (have.as_array(), want.as_array()) else {
        wrong.push(format!("{key} is not a list"));
        return;
    };
    if have.len() != want.len() {
        wrong.push(format!(
            "{key} has {} elements, want {}: {}",
            have.len(),
            want.len(),
            Value::from(have.clone())
        ));
        return;
    }
    for (i, (have, want)) in have.iter().zip(want).enumerate() {
        for (property, want) in want.as_object().into_iter().flatten() {
            match (have.get(property), want) {
                (None, Value::Null) => {}
                (Some(have), want) if have == want => {}
                (have, want) => wrong.push(format!(
                    "{key}[{i}].{property} is {}, want {}",
                    have.map_or("absent".to_owned(), Value::to_string),
                    if want.is_null() {
                        "absent".to_owned()
                    } else {
                        want.to_string()
                    }
                )),
            }
        }
    }
}

/// The case's properties, as the README's table names them.
fn properties(case: &Case) -> Map<String, Value> {
    let properties = json!({
        "state": case.state().as_str(),
        "revision": case.revision(),
        "amend_seq": case.amend_seq(),
        "updated_at": time(case.updated_at()),
        "kind": case.kind().as_str(),
        "urgency": case.urgency().as_str(),
        "title": case.title(),
        "body": case.body(),
        "options": case.options(),
        "recommend": case.recommend(),
        "rows": value(case.rows()),
        "landed": case.landed(),
        "left_out": case.left_out(),
        "links": case.links(),
        "labels": case.labels(),
        "worker": case.worker(),
        "brief": case.brief(),
        "context": case.context(),
        "for": case.for_(),
        "re": case.re(),
        "actor": value(case.actor()),
        "opened_at": time(case.opened_at()),
        "answer": value(case.answer()),
        "pickup": value(case.pickup()),
        "park": value(case.park()),
        "close": value(case.close()),
        "events": case.events().iter().map(event).collect::<Vec<_>>(),
        "problems": case.problems().iter().map(|problem| json!({
            "seq": problem.seq,
            "file": problem.file(),
            "category": problem.error.category().as_str(),
        })).collect::<Vec<_>>(),
    });
    let Value::Object(properties) = properties else {
        unreachable!("json! makes an object of an object")
    };
    properties
}

/// An element of `events`, with each property it has.
fn event(event: &Event) -> Value {
    let mut properties = Map::new();
    properties.insert("seq".into(), event.seq.into());
    properties.insert("author".into(), event.author.as_str().into());
    properties.insert("event".into(), event.event().as_str().into());
    properties.insert("file".into(), event.file().into());
    if let Some(from) = event.from {
        properties.insert("from".into(), from.as_str().into());
    }
    if let Some(at) = event.at() {
        properties.insert("at".into(), time(Some(at)));
    }
    if let Some(actor) = event.actor() {
        properties.insert("actor".into(), value(actor));
    }
    let data = serde_json::from_str(&event.data).expect("an event that folded has JSON data");
    properties.insert("data".into(), data);
    Value::Object(properties)
}

fn time(time: Option<Time>) -> Value {
    time.map_or(Value::Null, |time| time.to_string().into())
}

/// A record, or a list or an option of them, as JSON: the record as it
/// serializes, or null.
fn value(record: impl Serialize) -> Value {
    serde_json::to_value(record).expect("a record serializes")
}

/// The authors of each event, as the README's table of error categories
/// gives them. Either writes `open` and `amend`, which belong to the side
/// that opens the case's kind.
const WRITTEN_BY: &[(&str, &str)] = &[
    ("open", "agent"),
    ("open", "human"),
    ("amend", "agent"),
    ("amend", "human"),
    ("pickup", "agent"),
    ("note", "agent"),
    ("close", "agent"),
    ("withdraw", "agent"),
    ("answer", "human"),
    ("park", "human"),
    ("resume", "human"),
    ("withdraw", "human"),
];

/// The kinds the matrix tries, each with the states only it can be in: only
/// a stuck case is open, answered or parked, and only a message is sent. A
/// kind is tried from every state but the other kind's own, so a state
/// added later is tried for both until it is listed here.
const ONLY: &[(Kind, &[State])] = &[
    (Kind::Stuck, &[State::Open, State::Answered, State::Parked]),
    (Kind::Message, &[State::Sent]),
];

/// The states the matrix tries a case of `kind` from: every state but those
/// only the other kind can be in.
fn states(kind: Kind) -> impl Iterator<Item = State> {
    State::ALL.iter().copied().filter(move |state| {
        ONLY.iter()
            .all(|(other, only)| *other == kind || !only.contains(state))
    })
}

/// Checks that `transitions.json` tries every event, by each author that may
/// write it, from every state of a stuck case and of a message: 6 and 4
/// states, and 12 pairs of an event and an author. A vector tries its append,
/// or for a replay its last row, from the state the rows before it fold to.
/// A vector that tries one of them from a state the matrix leaves out for
/// its kind fails the check too. It also checks that an open and an amend of
/// each kind by the side that does not open it are tried, and refused for
/// their author: who opens a kind is checked once the kind is known, so one
/// kind's vector does not cover another's.
fn coverage() -> Result<(), Failed> {
    for event in EventType::ALL {
        for author in Author::ALL {
            let listed = WRITTEN_BY.contains(&(event.as_str(), author.as_str()));
            if event.written_by(*author) != listed {
                return Err(format!(
                    "the core says {author} may write {event}: {}, and the README: {listed}",
                    event.written_by(*author)
                )
                .into());
            }
        }
    }
    let files = common::load_all().map_err(|errors| errors.join("\n"))?;
    let transitions = files
        .iter()
        .find(|file| file.name == "transitions")
        .ok_or("there is no transitions.json")?;
    let mut tried = BTreeSet::new();
    let mut refused_for_author = BTreeSet::new();
    for vector in &transitions.vectors {
        let mut rows: Vec<Row> = vector.events.iter().map(row).collect();
        let (event, author) = match &vector.append {
            Some(append) => (append.event.clone(), append.author.clone()),
            None => match rows.pop() {
                Some(last) => (last.event, last.author),
                None => continue,
            },
        };
        let folded = Case::fold(&rows);
        if let Some(append) = &vector.append
            && vector
                .expect_error
                .as_ref()
                .is_some_and(|error| error.category == "author")
        {
            let kind = match append.event.as_str() {
                "open" => append
                    .data
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(|kind| kind.parse::<Kind>().ok()),
                "amend" => folded.as_ref().ok().map(Case::kind),
                _ => None,
            };
            if let Some(kind) = kind {
                refused_for_author.insert((kind, event.clone(), author.clone()));
            }
        }
        if let Ok(case) = folded {
            tried.insert((case.kind(), case.state(), event, author));
        }
    }
    if let Some((kind, state, ..)) = tried.iter().find(|(kind, state, ..)| {
        ONLY.iter().any(|(tries, _)| tries == kind) && !states(*kind).any(|s| s == *state)
    }) {
        return Err(
            format!("a vector tries a {kind} that is {state}, and the matrix does not").into(),
        );
    }
    let mut missing: Vec<String> = ONLY
        .iter()
        .flat_map(|(kind, _)| states(*kind).map(move |state| (*kind, state)))
        .flat_map(|(kind, state)| WRITTEN_BY.iter().map(move |pair| (kind, state, *pair)))
        .filter(|(kind, state, (event, author))| {
            !tried.contains(&(*kind, *state, (*event).to_owned(), (*author).to_owned()))
        })
        .map(|(kind, state, (event, author))| {
            format!("a {kind} that is {state}: {event} by the {author}")
        })
        .collect();
    for kind in Kind::ALL {
        for other in Author::ALL
            .iter()
            .filter(|author| **author != kind.opener())
        {
            for event in ["open", "amend"] {
                let pair = (*kind, event.to_owned(), other.as_str().to_owned());
                if !refused_for_author.contains(&pair) {
                    missing.push(format!(
                        "a {kind}: {event} by the {other}, refused for its author"
                    ));
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("no vector tries\n{}", missing.join("\n")).into())
    }
}
