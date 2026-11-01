//! What commands print: JSON (CL-02, AG-13), tables, times and ages.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::time::Duration;

use cases_core::{
    Actor, Answer, ApprovalRow, Case, Category, Close, Event, EventType, Kind, Park, Pickup,
    Problem, Record, Row, State, Store as _, Time, Urgency,
};
use cases_store_sqlite::{ActorRow, Db, Key};

use crate::store::{AnyStore, Failure};
use serde::Serialize;
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde_json::value::RawValue;

use crate::error::Error;

/// Prints `value` as indented JSON, then a line break.
pub(crate) fn json(out: &mut impl Write, value: &impl Serialize) -> Result<(), Error> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}

/// What every write prints with `--json`: the case, its state and its new
/// revision, which the next write takes as `--revision` (CL-02).
#[derive(Debug, Serialize)]
pub(crate) struct Written {
    id: String,
    state: &'static str,
    revision: u64,
}

impl Written {
    /// The case `key` as `case` stands after a write.
    pub(crate) fn new(key: Key, case: &Case) -> Written {
        Written {
            id: key.to_string(),
            state: case.state().as_str(),
            revision: case.revision(),
        }
    }
}

/// The store's table of actors, by id, to name the actor an event holds only
/// the id of (AG-13).
#[derive(Debug, Default)]
pub(crate) struct Actors(HashMap<String, ActorRow>);

impl Actors {
    /// The actors in `rows`.
    pub(crate) fn new(rows: Vec<ActorRow>) -> Actors {
        Actors(rows.into_iter().map(|row| (row.id.clone(), row)).collect())
    }

    /// The store's table of actors. A store not made yet has none.
    ///
    /// # Errors
    ///
    /// When the store fails.
    pub(crate) fn of(store: &AnyStore) -> Result<Actors, Failure> {
        match store.actors() {
            Ok(rows) => Ok(Actors::new(rows)),
            Err(error) if error.is_missing() => Ok(Actors::default()),
            Err(error) => Err(error),
        }
    }

    /// The table of actors of the local store `db`, as [`Actors::of`] gives
    /// it.
    ///
    /// # Errors
    ///
    /// When the store fails.
    pub(crate) fn read(db: &Db) -> Result<Actors, cases_store_sqlite::Error> {
        match db.actors() {
            Ok(rows) => Ok(Actors::new(rows)),
            Err(cases_store_sqlite::Error::Missing { .. }) => Ok(Actors::default()),
            Err(error) => Err(error),
        }
    }

    /// `actor` as JSON shows it: its id, with the kind and name the table
    /// gives it. An actor the table does not have, one whose row was deleted,
    /// has its id alone.
    fn json<'a>(&'a self, actor: &'a Actor) -> ActorJson<'a> {
        let row = self.0.get(&actor.id);
        ActorJson {
            id: &actor.id,
            kind: row.map(|row| row.kind.as_str()),
            name: row.and_then(|row| row.name.as_deref()),
        }
    }

    /// Who `actor` is, for people: the name the table gives, `None` for an
    /// unnamed actor, and `deleted user` for one the table does not have.
    pub(crate) fn name<'a>(&'a self, actor: &Actor) -> Option<&'a str> {
        match self.0.get(&actor.id) {
            Some(row) => row.name.as_deref(),
            None => Some("deleted user"),
        }
    }
}

/// An actor in JSON output.
#[derive(Debug, Serialize)]
struct ActorJson<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// A case as `show --json` and `list --json` print it: the case's fields at
/// the top level, then its events as stored, the rows the fold skipped, and
/// its revision and amend seq. Empty optional fields are left out.
#[derive(Debug, Serialize)]
pub(crate) struct CaseJson<'a> {
    id: String,
    state: &'static str,
    kind: &'static str,
    urgency: &'static str,
    title: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    body: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    options: &'a [String],
    /// The option the agent recommends, numbered from 1, when it recommends
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    recommend: Option<i64>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    rows: &'a [ApprovalRow],
    /// A briefing's only: whether its work has landed.
    #[serde(skip_serializing_if = "Option::is_none")]
    landed: Option<bool>,
    /// A briefing's left-out items, numbered from 1 by an answer's `file`.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    left_out: &'a [String],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    links: &'a [String],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    labels: &'a [String],
    #[serde(skip_serializing_if = "str::is_empty")]
    worker: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    brief: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    context: &'a str,
    #[serde(rename = "for", skip_serializing_if = "str::is_empty")]
    for_: &'a str,
    /// The case this one is about, by its id.
    #[serde(skip_serializing_if = "str::is_empty")]
    re: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<ActorJson<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opened_at: Option<Time>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<Time>,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<&'a Answer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pickup: Option<&'a Pickup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    park: Option<&'a Park>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close: Option<&'a Close>,
    events: Vec<EventJson<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    problems: Vec<String>,
    revision: u64,
    /// The seq of the last amend that changed the question, 0 when none: an
    /// answer carries it as the amend it saw (ST-04).
    amend_seq: u64,
    /// The highest seq among the case's rows, skipped ones included: with
    /// the revision, what a form drawn from the case keeps, to say later how
    /// the case changed since (IN-05).
    last_seq: u64,
    /// The case's page in the running inbox, or empty: `show` gives it, and
    /// `list` leaves it out.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// An event in JSON output: its record as stored, unknown fields included,
/// with its actor named. An open or amend whose `body` is the case's `body`
/// leaves it out, and says so with `body_in_case`, so a long body is given
/// once, not twice.
#[derive(Debug, Serialize)]
struct EventJson<'a> {
    seq: u64,
    author: &'static str,
    event: &'static str,
    /// The case's state before the event; left out on the open, which has
    /// none.
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<&'static str>,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<Time>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<ActorJson<'a>>,
    data: Cow<'a, RawValue>,
    /// The record's `body`, left out of `data`, is the case's `body`.
    #[serde(skip_serializing_if = "is_false")]
    body_in_case: bool,
}

// serde's skip_serializing_if takes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl<'a> CaseJson<'a> {
    /// The case `key`, with its actors named from `actors`.
    ///
    /// # Errors
    ///
    /// When an event's record is not JSON, which a case that folded never
    /// has.
    pub(crate) fn new(key: Key, case: &'a Case, actors: &'a Actors) -> Result<CaseJson<'a>, Error> {
        let events = case
            .events()
            .iter()
            .map(|event| event_json(event, case.body(), actors))
            .collect::<Result<_, _>>()?;
        Ok(CaseJson {
            id: key.to_string(),
            state: case.state().as_str(),
            kind: case.kind().as_str(),
            urgency: case.urgency().as_str(),
            title: case.title(),
            body: case.body(),
            options: case.options(),
            recommend: (case.recommend() != 0).then_some(case.recommend()),
            rows: case.rows(),
            landed: (case.kind() == Kind::Briefing).then(|| case.landed()),
            left_out: case.left_out(),
            links: case.links(),
            labels: case.labels(),
            worker: case.worker(),
            brief: case.brief(),
            context: case.context(),
            for_: case.for_(),
            re: case.re(),
            actor: case.actor().map(|actor| actors.json(actor)),
            opened_at: case.opened_at(),
            updated_at: case.updated_at(),
            answer: case.answer(),
            pickup: case.pickup(),
            park: case.park(),
            close: case.close(),
            events,
            problems: case.problems().iter().map(problem).collect(),
            revision: case.revision(),
            amend_seq: case.amend_seq(),
            last_seq: case.last_seq(),
            url: None,
        })
    }

    /// The case with its page in the running inbox: `url`, or empty when no
    /// inbox runs.
    pub(crate) fn with_url(self, url: String) -> CaseJson<'a> {
        CaseJson {
            url: Some(url),
            ..self
        }
    }
}

fn event_json<'a>(
    event: &'a Event,
    body: &str,
    actors: &'a Actors,
) -> Result<EventJson<'a>, Error> {
    let data: &RawValue = serde_json::from_str(&event.data)?;
    // The record as read says whether its body is the case's, so a record
    // whose body is not, or that has none, is not taken apart to see.
    let set = match &event.record {
        Record::Open(open) => open.body.as_str(),
        Record::Amend(amend) => amend.body.as_str(),
        _ => "",
    };
    let without = if !body.is_empty() && set == body {
        without_body(data, body)?
    } else {
        None
    };
    Ok(EventJson {
        seq: event.seq,
        author: event.author.as_str(),
        event: event.event().as_str(),
        from: event.from.map(State::as_str),
        file: event.file(),
        at: event.at(),
        actor: event.actor().map(|actor| actors.json(actor)),
        body_in_case: without.is_some(),
        data: without.map_or(Cow::Borrowed(data), Cow::Owned),
    })
}

/// The record `data` without its `body`, when its one `body` is `body`,
/// compared as text, escapes decoded: its other fields as written, in their
/// order, each value as stored. None, so that the record is given as it is,
/// when it is not an object, or has no `body`, another one, or more than one.
fn without_body(data: &RawValue, body: &str) -> Result<Option<Box<RawValue>>, Error> {
    let Ok(Fields(fields)) = serde_json::from_str::<Fields>(data.get()) else {
        return Ok(None);
    };
    let mut bodies = fields.iter().filter(|(key, _)| key == "body");
    let same = match (bodies.next(), bodies.next()) {
        (Some((_, value)), None) => {
            serde_json::from_str::<String>(value.get()).is_ok_and(|text| text == body)
        }
        _ => false,
    };
    if !same {
        return Ok(None);
    }
    let kept = Fields(
        fields
            .into_iter()
            .filter(|(key, _)| key != "body")
            .collect(),
    );
    Ok(Some(serde_json::value::to_raw_value(&kept)?))
}

/// A JSON object's fields in the order written, each value as stored.
struct Fields(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Fields, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Fields;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Fields, A::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

impl Serialize for Fields {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// What `show --answer` prints: enough to act on the answer without the rest
/// of the case. `answer` is `null` when the case has none.
#[derive(Debug, Serialize)]
pub(crate) struct AnswerJson<'a> {
    state: &'static str,
    kind: &'static str,
    revision: u64,
    answer: Option<&'a Answer>,
}

impl<'a> AnswerJson<'a> {
    pub(crate) fn new(case: &'a Case) -> AnswerJson<'a> {
        AnswerJson {
            state: case.state().as_str(),
            kind: case.kind().as_str(),
            revision: case.revision(),
            answer: case.answer(),
        }
    }
}

/// What a warning about something this version does not know adds (ST-05).
const NEWER: &str = ": perhaps written by a newer cases, or not by cases at all; if newer, update \
                     cases on this machine";

/// A row the fold skipped, for people: its file name and why. A row this
/// version does not know is said to be perhaps from a newer cases (ST-05).
pub(crate) fn problem(problem: &Problem) -> String {
    let hint = if problem.error.category() == Category::Unknown {
        NEWER
    } else {
        ""
    };
    format!("{}: {}{hint}", problem.file(), problem.error)
}

/// Why a case's `rows` do not fold, for people: `error`, and, when a row
/// has an event, or an open a kind or urgency, this version does not know,
/// that the case is perhaps from a newer cases (ST-05).
pub(crate) fn unfolded(rows: &[Row], error: &cases_core::Error) -> String {
    let newer = rows.iter().any(|row| {
        let Ok(event) = row.event.parse::<EventType>() else {
            return true;
        };
        let Ok(Record::Open(open)) = Record::read(event, &row.data) else {
            return false;
        };
        !open.kind.is_empty() && open.kind.parse::<Kind>().is_err()
            || !open.urgency.is_empty() && open.urgency.parse::<Urgency>().is_err()
    });
    format!("{error}{}", if newer { NEWER } else { "" })
}

/// `rows` as columns, each as wide as its widest cell and two spaces apart.
/// The last column is not padded.
pub(crate) fn table(out: &mut impl Write, rows: &[Vec<String>]) -> Result<(), Error> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let width = cell.chars().count();
            match widths.get_mut(i) {
                Some(max) => *max = (*max).max(width),
                None => widths.push(width),
            }
        }
    }
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if i + 1 < row.len() {
                let pad = widths[i] - cell.chars().count() + 2;
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// The time now, from the system's clock.
///
/// # Errors
///
/// When the clock is outside the years 1970 to 9999.
pub(crate) fn now() -> Result<Time, Error> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok();
    since
        .and_then(|since| {
            Time::from_unix(i64::try_from(since.as_secs()).ok()?, since.subsec_nanos())
        })
        .ok_or_else(|| Error::new("the clock is outside the years 1970 to 9999"))
}

/// How long after `from` `to` is, or `None` when it is before.
pub(crate) fn elapsed(from: Time, to: Time) -> Option<Duration> {
    let nanos = |t: Time| i128::from(t.unix_secs()) * 1_000_000_000 + i128::from(t.subsec_nanos());
    let diff = u128::try_from(nanos(to) - nanos(from)).ok()?;
    let secs = u64::try_from(diff / 1_000_000_000).ok()?;
    let subsec = u32::try_from(diff % 1_000_000_000).ok()?;
    Some(Duration::new(secs, subsec))
}

/// How old something from `at` is, for a table: `now`, then minutes, hours
/// up to two days, then days. `-` when there is no time.
pub(crate) fn age(at: Option<Time>, now: Time) -> String {
    let Some(at) = at else {
        return "-".into();
    };
    let secs = elapsed(at, now).unwrap_or_default().as_secs();
    match secs {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..172_800 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// The lowercase ASCII words of `text`, joined by `-`: `case` when it has
/// none, and cut at a word to 60 characters.
pub(crate) fn slug(text: &str) -> String {
    const MAX: usize = 60;
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug = slug.trim_end_matches('-').to_owned();
    if slug.len() > MAX {
        slug.truncate(MAX);
        if let Some(dash) = slug.rfind('-').filter(|dash| *dash > 0) {
            slug.truncate(dash);
        }
    }
    if slug.is_empty() {
        slug.push_str("case");
    }
    slug
}

/// A case's id with its title's slug after it, `UkLWZ-pin-bun-or-float`,
/// as people see it: in the list table, show's header and the inbox's links
/// (ST-10, case 2026-09-25T20-55-32Z-title-slug-after-ids-show-it-in-v1-lists-and).
/// Every lookup ignores the slug. JSON ids stay bare.
pub(crate) fn titled(key: Key, title: &str) -> String {
    format!("{key}-{}", slug(title))
}

/// `n` and `word`, in the plural unless `n` is 1.
pub(crate) fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// A time for people: RFC 3339 in UTC, to the second. Empty when there is
/// none.
pub(crate) fn stamp(at: Option<Time>) -> String {
    at.and_then(|at| Time::from_unix(at.unix_secs(), 0))
        .map(|at| at.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_without_its_body_keeps_its_other_fields_as_written() {
        let data: &RawValue =
            serde_json::from_str(r#"{"z": 1, "body": "B\n", "a": [1,  2], "x": {"body": "B\n"}}"#)
                .unwrap();
        let without = without_body(data, "B\n").unwrap().unwrap();
        assert_eq!(without.get(), r#"{"z":1,"a":[1,  2],"x":{"body": "B\n"}}"#);
        // Another body, or none, is left as it is.
        assert!(without_body(data, "C").unwrap().is_none());
        let none: &RawValue = serde_json::from_str(r#"{"title": "T"}"#).unwrap();
        assert!(without_body(none, "B").unwrap().is_none());
    }

    #[test]
    fn a_body_is_compared_as_text_whatever_its_escapes() {
        let data: &RawValue =
            serde_json::from_str(r#"{"body": "caf\u00e9 \/x \"q\"", "title": "T"}"#).unwrap();
        let without = without_body(data, "café /x \"q\"").unwrap().unwrap();
        assert_eq!(without.get(), r#"{"title":"T"}"#);
        // A body that differs only in its case is another body.
        assert!(without_body(data, "CAFÉ /x \"q\"").unwrap().is_none());
    }

    #[test]
    fn a_record_that_is_not_one_object_with_one_body_is_left_as_it_is() {
        for text in [
            r#"["B"]"#,
            r#""B""#,
            "1",
            "null",
            r#"{"body": 1}"#,
            r#"{"body": null}"#,
            r#"{"body": "B", "body": "B"}"#,
            r#"{"body": "B", "body": "C"}"#,
            r#"{"body": "C", "body": "B"}"#,
        ] {
            let data: &RawValue = serde_json::from_str(text).unwrap();
            assert!(without_body(data, "B").unwrap().is_none(), "{text}");
        }
    }

    fn at(text: &str) -> Time {
        text.parse().unwrap()
    }

    #[test]
    fn a_table_lines_up_its_columns() {
        let rows = [
            vec!["ID".into(), "LABELS".into(), "TITLE".into()],
            vec!["UkLWZ".into(), String::new(), "Plain".into()],
            vec!["gMvFo".into(), "round-1,docs".into(), "Labelled".into()],
        ];
        let mut out = Vec::new();
        table(&mut out, &rows).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "ID     LABELS        TITLE\n\
             UkLWZ                Plain\n\
             gMvFo  round-1,docs  Labelled\n"
        );
    }

    #[test]
    fn an_age_is_in_the_largest_unit_that_fits() {
        let now = at("2026-09-25T12:00:00Z");
        for (then, want) in [
            ("2026-09-25T11:59:30Z", "now"),
            ("2026-09-25T12:00:30Z", "now"),
            ("2026-09-25T11:59:00Z", "1m"),
            ("2026-09-25T11:00:01Z", "59m"),
            ("2026-09-25T11:00:00Z", "1h"),
            ("2026-09-23T12:00:01Z", "47h"),
            ("2026-09-23T12:00:00Z", "2d"),
        ] {
            assert_eq!(age(Some(at(then)), now), want, "{then}");
        }
        assert_eq!(age(None, now), "-");
    }

    #[test]
    fn elapsed_is_none_backwards() {
        let (a, b) = (at("2026-09-25T12:00:00Z"), at("2026-09-25T12:00:01.5Z"));
        assert_eq!(elapsed(a, b), Some(Duration::from_millis(1_500)));
        assert_eq!(elapsed(b, a), None);
        assert_eq!(elapsed(a, a), Some(Duration::ZERO));
    }

    #[test]
    fn slug_keeps_the_words() {
        assert_eq!(slug("cases.db"), "cases-db");
        assert_eq!(slug("Work Store!.db"), "work-store-db");
        assert_eq!(slug("..."), "case");
        assert_eq!(slug(&"word ".repeat(20)).len(), 59);
    }

    #[test]
    fn a_titled_id_is_the_id_and_the_title_s_slug() {
        let key = Key::new(1).unwrap();
        assert_eq!(titled(key, "Pin bun or float?"), "UkLWZ-pin-bun-or-float");
        assert_eq!(titled(key, "Ünïcode ✓"), "UkLWZ-n-code");
        assert_eq!(titled(key, "!!!"), "UkLWZ-case");
        assert_eq!(titled(key, "Pin bun or float?").parse::<Key>(), Ok(key));
    }

    #[test]
    fn plural_is_singular_only_at_one() {
        assert_eq!(plural(0, "case"), "0 cases");
        assert_eq!(plural(1, "case"), "1 case");
        assert_eq!(plural(2, "case"), "2 cases");
    }

    #[test]
    fn a_stamp_is_to_the_second() {
        assert_eq!(
            stamp(Some(at("2026-09-25T12:00:00.123+01:00"))),
            "2026-09-25T11:00:00Z"
        );
        assert_eq!(stamp(None), "");
    }
}

#[cfg(test)]
mod freshness;
