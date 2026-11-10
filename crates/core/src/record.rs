//! Records: what each event says, as it is stored.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::names::EventType;
use crate::time::Time;

/// Who wrote an event: an id, and nothing more.
///
/// The id is 22 characters of the base64url alphabet (`A`–`Z`, `a`–`z`,
/// `0`–`9`, `-` and `_`), 128 random bits. The event's
/// [`Author`](crate::Author) gives the role. The actor is a label, not a
/// permission, and the core never checks it against the author. Names live in
/// the store's table of actors.
///
/// A row whose actor has an id of any other shape, or none, is
/// [`Invalid`](crate::Category::Invalid). The core never decodes an id, and
/// never looks one up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Actor {
    /// The actor's id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
}

/// A row of an approval case: one step for the human to approve, hold or
/// reject.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ApprovalRow {
    /// Names the row in the case: letters, digits, `.`, `-` and `_`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// What the step does.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The script the step runs, as text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub script: String,
    /// Where the script lives.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
    /// A note for the human.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Who carries the step out: empty for the agent, or `human`. On the
    /// human's row, the script says the step, and `approve` means it is done.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
}

/// The human's verdict on one row of an approval case.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RowVerdict {
    /// The row's id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `approve`, `hold` or `reject`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub verdict: String,
    /// A note for the agent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The record of an `open` event: the question a case asks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Open {
    /// The kind, such as `decision`. It is text, so that a kind this version
    /// does not know reads, and is then refused as
    /// [`Unknown`](crate::Category::Unknown).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// The urgency, such as `today`, as text for the same reason.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub urgency: String,
    /// The title.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The body, in markdown.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// A decision's options. A choice numbers them from 1.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The option the agent recommends, numbered from 1 as a choice is: a
    /// decision's only. Left out, or 0, it recommends none. It is shown to
    /// the human, and an answer is never checked against it.
    #[serde(skip_serializing_if = "is_zero")]
    pub recommend: i64,
    /// An approval's rows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<ApprovalRow>,
    /// Whether a briefing's work has landed already, so nothing waits on the
    /// answer. Left out, it is `false`: the agent waits for the answer before
    /// it lands the work. A briefing's only.
    #[serde(skip_serializing_if = "is_false")]
    pub landed: bool,
    /// What a briefing left out on purpose: up to 10 items, each 1 to 300
    /// characters, none blank and none given twice. An answer files them by
    /// number from 1. A briefing's only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
    /// Links for the human.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Labels to find the case by.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// The worker that opened the case. On a message, the worker it is sent
    /// to.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub worker: String,
    /// The worker's brief.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub brief: String,
    /// What the case is part of, such as a release.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub context: String,
    /// Who the case is for, as text: `for` in the record. Nothing checks it
    /// against who answers.
    #[serde(rename = "for", skip_serializing_if = "String::is_empty")]
    pub for_: String,
    /// The case this one is about, such as the message an agent's question
    /// follows, by its id, as text. The core never reads it as an id: ids
    /// belong to the store.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub re: String,
    /// Who opened the case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When the case was opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<Time>,
}

/// The record of an `amend` event: a change to an open case, or to a message
/// that is sent. Options, rows, links and labels are added after the case's;
/// a title, a body, a context or a recommend replaces the case's; the links
/// in `drop_links` are taken off it. A field left out leaves the case's as it
/// was. With `reopen`, the agent's amend also reopens an answered or
/// picked-up case, as a note with `reopen` does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Amend {
    /// A new title.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// A new body.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// Options to add.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The option the agent recommends now: one of the case's options, or of
    /// those this amend adds. Left out, or 0, it leaves the case's as it
    /// was, so an amend cannot take a recommend away.
    #[serde(skip_serializing_if = "is_zero")]
    pub recommend: i64,
    /// Rows to add.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<ApprovalRow>,
    /// A briefing's `landed`, set anew: `false` as well as `true` replaces
    /// the case's, so this field alone is not left out at its zero value.
    /// `None` leaves the case's as it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landed: Option<bool>,
    /// A briefing's new list of what it left out, in place of the case's
    /// whole list. Left out or empty, it leaves the case's as it was. A
    /// list other than the case's changes the question.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
    /// Links to add.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Links to take off the case: each one a link the case has.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drop_links: Vec<String>,
    /// Labels to add.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// A new context.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub context: String,
    /// The amend reopens an answered or picked-up case: the case is open
    /// again, and its answer and pickup are cleared. On a case that is open
    /// or sent it is [`Invalid`](crate::Category::Invalid): there is nothing
    /// to reopen.
    #[serde(skip_serializing_if = "is_false")]
    pub reopen: bool,
    /// Who amended the case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amended_at: Option<Time>,
}

/// The record of an `answer` event.
///
/// Which response fields it may set depends on the case's kind: a decision
/// takes `choice`, or `other` with a note; an approval takes a verdict on
/// every row; a signoff takes `accept`, or `changes` with a note; a stuck case
/// or a question takes `text`; an fyi takes `ack`; a briefing takes `accept`,
/// or `send_back` with a note, and any of its left-out items to `file`. `drop` may stand in for
/// the response on each of these kinds, and each takes a note. A message
/// takes no answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Answer {
    /// The option chosen, numbered from 1.
    #[serde(skip_serializing_if = "is_zero")]
    pub choice: i64,
    /// None of the options: see the note.
    #[serde(skip_serializing_if = "is_false")]
    pub other: bool,
    /// A verdict on each row.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RowVerdict>,
    /// `accept` or `changes`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signoff: String,
    /// A briefing's verdict: `accept`, or `send_back` with a note.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub briefing: String,
    /// The briefing's left-out items to file, by number from 1, as a choice
    /// numbers the options. It goes only with a briefing's verdict.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file: Vec<i64>,
    /// The guidance or the reply.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Dismisses the case in place of a response.
    #[serde(skip_serializing_if = "is_false")]
    pub drop: bool,
    /// Acknowledges an fyi.
    #[serde(skip_serializing_if = "is_false")]
    pub ack: bool,
    /// A note for the agent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The case's `amend_seq` as the answer's writer read it: the question it
    /// answers. Left out, it is 0. It is a count, so a record that gives a
    /// negative one is [`Malformed`](crate::Category::Malformed).
    #[serde(skip_serializing_if = "is_zero")]
    pub amend_seq: i64,
    /// Who answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<Time>,
}

/// The record of a `pickup` event: the agent has read the answer, or has
/// taken the message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Pickup {
    /// Who picked the answer or the message up, as text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
    /// Who picked it up, as an actor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picked_up_at: Option<Time>,
}

/// The record of a `note` event: a follow-up in the case's thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Note {
    /// The note, in markdown.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The note reopens an answered or picked-up case, and says so. The
    /// writer refuses a note on such a case without it, as a
    /// [`Transition`](crate::Category::Transition), so that a follow-up never
    /// reopens a case by accident. A stored note without it still reopens
    /// the case, as every note did before the field. On an open case it is
    /// [`Invalid`](crate::Category::Invalid): there is nothing to reopen.
    #[serde(skip_serializing_if = "is_false")]
    pub reopen: bool,
    /// Who wrote the note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noted_at: Option<Time>,
}

/// The record of a `close` event: what came of the case.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Close {
    /// The outcome, in markdown.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    /// Links to the outcome.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// The close is not the agent's judgement: the human's side closed a
    /// picked-up briefing that no agent closed in the time the human set,
    /// and the outcome says so. Only a briefing closes expired; on any other
    /// kind it is [`Invalid`](crate::Category::Invalid). It is written under
    /// the agent's role, as every close is, so it wakes no wait.
    #[serde(skip_serializing_if = "is_false")]
    pub expired: bool,
    /// Who closed the case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<Time>,
}

/// The record of a `withdraw` event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Withdraw {
    /// Why the case was withdrawn.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Who withdrew it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawn_at: Option<Time>,
}

/// The record of a `park` event: the human puts a stuck case aside.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Park {
    /// Why, or until when.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Who parked the case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parked_at: Option<Time>,
}

/// The record of a `resume` event: the human opens a parked case again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Resume {
    /// Who resumed the case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// When.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<Time>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Any event's record.
///
/// It serializes as the record alone, the JSON object a row stores: each
/// field at its zero value left out, and its time in UTC.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Record {
    /// An `open` event's record.
    Open(Open),
    /// An `amend` event's record.
    Amend(Amend),
    /// An `answer` event's record.
    Answer(Answer),
    /// A `pickup` event's record.
    Pickup(Pickup),
    /// A `note` event's record.
    Note(Note),
    /// A `close` event's record.
    Close(Close),
    /// A `withdraw` event's record.
    Withdraw(Withdraw),
    /// A `park` event's record.
    Park(Park),
    /// A `resume` event's record.
    Resume(Resume),
}

impl Record {
    /// Reads a stored record of `event`, as `spec/vectors/README.md` says
    /// under "Reading a record".
    ///
    /// The text must be one JSON object, with nothing but white space around
    /// it, and no object in it may give a key twice. Each field the event
    /// defines must, where given, have the field's type and not be `null`,
    /// and each time must read as a [`Time`]. Field names are matched
    /// exactly, and a field the event does not define is ignored. A field at
    /// its zero value (`""`, 0, `false` or `[]`) reads as the field left out.
    /// A record that breaks any of this is
    /// [`Malformed`](crate::Category::Malformed).
    ///
    /// # Errors
    ///
    /// Gives [`Malformed`](crate::Category::Malformed) when the text is not
    /// JSON, is not one object, gives a key twice, or a field of `event`
    /// does not read as its type.
    pub fn read(event: EventType, text: &str) -> Result<Record, Error> {
        let Unique(value) = serde_json::from_str(&unsigned_zeros(text))
            .map_err(|e| Error::malformed(format!("the record is not JSON: {e}")))?;
        let record = match event {
            EventType::Open => Field::read(value).map(Record::Open),
            EventType::Amend => Field::read(value).map(Record::Amend),
            EventType::Answer => Field::read(value).map(Record::Answer),
            EventType::Pickup => Field::read(value).map(Record::Pickup),
            EventType::Note => Field::read(value).map(Record::Note),
            EventType::Close => Field::read(value).map(Record::Close),
            EventType::Withdraw => Field::read(value).map(Record::Withdraw),
            EventType::Park => Field::read(value).map(Record::Park),
            EventType::Resume => Field::read(value).map(Record::Resume),
        };
        record.map_err(|e| Error::malformed(format!("the {event} record does not read: {e}")))
    }

    /// The event the record belongs to.
    #[must_use]
    pub fn event(&self) -> EventType {
        match self {
            Record::Open(_) => EventType::Open,
            Record::Amend(_) => EventType::Amend,
            Record::Answer(_) => EventType::Answer,
            Record::Pickup(_) => EventType::Pickup,
            Record::Note(_) => EventType::Note,
            Record::Close(_) => EventType::Close,
            Record::Withdraw(_) => EventType::Withdraw,
            Record::Park(_) => EventType::Park,
            Record::Resume(_) => EventType::Resume,
        }
    }

    /// Who wrote the event, if the record says.
    #[must_use]
    pub fn actor(&self) -> Option<&Actor> {
        match self {
            Record::Open(r) => r.actor.as_ref(),
            Record::Amend(r) => r.actor.as_ref(),
            Record::Answer(r) => r.actor.as_ref(),
            Record::Pickup(r) => r.actor.as_ref(),
            Record::Note(r) => r.actor.as_ref(),
            Record::Close(r) => r.actor.as_ref(),
            Record::Withdraw(r) => r.actor.as_ref(),
            Record::Park(r) => r.actor.as_ref(),
            Record::Resume(r) => r.actor.as_ref(),
        }
    }

    /// The event's time: the record's `opened_at`, `answered_at` and so on.
    #[must_use]
    pub fn at(&self) -> Option<Time> {
        match self {
            Record::Open(r) => r.opened_at,
            Record::Amend(r) => r.amended_at,
            Record::Answer(r) => r.answered_at,
            Record::Pickup(r) => r.picked_up_at,
            Record::Note(r) => r.noted_at,
            Record::Close(r) => r.closed_at,
            Record::Withdraw(r) => r.withdrawn_at,
            Record::Park(r) => r.parked_at,
            Record::Resume(r) => r.resumed_at,
        }
    }

    pub(crate) fn actor_mut(&mut self) -> &mut Option<Actor> {
        match self {
            Record::Open(r) => &mut r.actor,
            Record::Amend(r) => &mut r.actor,
            Record::Answer(r) => &mut r.actor,
            Record::Pickup(r) => &mut r.actor,
            Record::Note(r) => &mut r.actor,
            Record::Close(r) => &mut r.actor,
            Record::Withdraw(r) => &mut r.actor,
            Record::Park(r) => &mut r.actor,
            Record::Resume(r) => &mut r.actor,
        }
    }

    pub(crate) fn at_mut(&mut self) -> &mut Option<Time> {
        match self {
            Record::Open(r) => &mut r.opened_at,
            Record::Amend(r) => &mut r.amended_at,
            Record::Answer(r) => &mut r.answered_at,
            Record::Pickup(r) => &mut r.picked_up_at,
            Record::Note(r) => &mut r.noted_at,
            Record::Close(r) => &mut r.closed_at,
            Record::Withdraw(r) => &mut r.withdrawn_at,
            Record::Park(r) => &mut r.parked_at,
            Record::Resume(r) => &mut r.resumed_at,
        }
    }

    /// The record as a row stores it.
    #[expect(
        clippy::expect_used,
        reason = "a record holds no map whose keys are not strings, and nothing whose serializer can fail"
    )]
    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a record serializes")
    }
}

impl Serialize for Record {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Record::Open(r) => r.serialize(serializer),
            Record::Amend(r) => r.serialize(serializer),
            Record::Answer(r) => r.serialize(serializer),
            Record::Pickup(r) => r.serialize(serializer),
            Record::Note(r) => r.serialize(serializer),
            Record::Close(r) => r.serialize(serializer),
            Record::Withdraw(r) => r.serialize(serializer),
            Record::Park(r) => r.serialize(serializer),
            Record::Resume(r) => r.serialize(serializer),
        }
    }
}

/// A value a record's field can hold, read from JSON by the contract's rules.
/// No field reads from `null`, and a whole number does not read from a number
/// with a fraction part.
///
/// Records are read this way, not with serde's derived `Deserialize`, which
/// also reads a struct from a JSON list, field by field. Each value is taken
/// by value, so a string moves into its field without a copy.
trait Field: Sized {
    /// Reads the value, or says what the JSON should have been.
    fn read(value: Value) -> Result<Self, String>;
}

impl Field for String {
    fn read(value: Value) -> Result<String, String> {
        match value {
            Value::String(s) => Ok(s),
            _ => Err("is not a string".into()),
        }
    }
}

impl Field for bool {
    fn read(value: Value) -> Result<bool, String> {
        value.as_bool().ok_or_else(|| "is not true or false".into())
    }
}

impl Field for i64 {
    fn read(value: Value) -> Result<i64, String> {
        value.as_i64().ok_or_else(|| "is not a whole number".into())
    }
}

/// Reads a count, such as an answer's `amend_seq`: a whole number of 0 or
/// more. A negative number is of the wrong type, so `malformed`.
fn count(value: Value) -> Result<i64, String> {
    match i64::read(value)? {
        n if n < 0 => Err("is not a count: it is below 0".into()),
        n => Ok(n),
    }
}

impl Field for Time {
    fn read(value: Value) -> Result<Time, String> {
        let text = value.as_str().ok_or("is not a time")?;
        text.parse().map_err(|e: Error| format!("{e}"))
    }
}

impl<T: Field> Field for Vec<T> {
    fn read(value: Value) -> Result<Vec<T>, String> {
        let Value::Array(list) = value else {
            return Err("is not a list".into());
        };
        list.into_iter()
            .enumerate()
            .map(|(i, x)| T::read(x).map_err(|e| format!("{i}: {e}")))
            .collect()
    }
}

/// A field that may be left out, but is not `null` when given.
impl<T: Field> Field for Option<T> {
    fn read(value: Value) -> Result<Option<T>, String> {
        T::read(value).map(Some)
    }
}

/// Reads a record from a JSON object: each field by its exact name, and a
/// field the record does not define left alone. A field reads by its type's
/// [`Field`], or by the function given after `as`.
macro_rules! object {
    ($name:ident { $($field:ident = $key:literal $(as $read:path)?,)+ }) => {
        impl Field for $name {
            fn read(value: Value) -> Result<$name, String> {
                let Value::Object(mut object) = value else {
                    return Err("is not a JSON object".into());
                };
                let mut record = $name::default();
                $(
                    if let Some(value) = object.remove($key) {
                        record.$field = read_field!(value $(, $read)?)
                            .map_err(|e| format!("{}: {e}", $key))?;
                    }
                )+
                Ok(record)
            }
        }
    };
}

macro_rules! read_field {
    ($value:expr) => {
        Field::read($value)
    };
    ($value:expr, $read:path) => {
        $read($value)
    };
}

object!(Actor { id = "id", });
object!(ApprovalRow {
    id = "id",
    label = "label",
    script = "script",
    link = "link",
    note = "note",
    by = "by",
});
object!(RowVerdict {
    id = "id",
    verdict = "verdict",
    note = "note",
});
object!(Open {
    kind = "kind",
    urgency = "urgency",
    title = "title",
    body = "body",
    options = "options",
    recommend = "recommend",
    rows = "rows",
    landed = "landed",
    left_out = "left_out",
    links = "links",
    labels = "labels",
    worker = "worker",
    brief = "brief",
    context = "context",
    for_ = "for",
    re = "re",
    actor = "actor",
    opened_at = "opened_at",
});
object!(Amend {
    title = "title",
    body = "body",
    options = "options",
    recommend = "recommend",
    rows = "rows",
    landed = "landed",
    left_out = "left_out",
    links = "links",
    drop_links = "drop_links",
    labels = "labels",
    context = "context",
    reopen = "reopen",
    actor = "actor",
    amended_at = "amended_at",
});
object!(Answer {
    choice = "choice",
    other = "other",
    rows = "rows",
    signoff = "signoff",
    briefing = "briefing",
    file = "file",
    text = "text",
    drop = "drop",
    ack = "ack",
    note = "note",
    amend_seq = "amend_seq" as count,
    actor = "actor",
    answered_at = "answered_at",
});
object!(Pickup {
    by = "by",
    actor = "actor",
    picked_up_at = "picked_up_at",
});
object!(Note {
    body = "body",
    reopen = "reopen",
    actor = "actor",
    noted_at = "noted_at",
});
object!(Close {
    outcome = "outcome",
    links = "links",
    expired = "expired",
    actor = "actor",
    closed_at = "closed_at",
});
object!(Withdraw {
    reason = "reason",
    actor = "actor",
    withdrawn_at = "withdrawn_at",
});
object!(Park {
    note = "note",
    actor = "actor",
    parked_at = "parked_at",
});
object!(Resume {
    actor = "actor",
    resumed_at = "resumed_at",
});

/// The JSON text with a space in place of the sign of each integer `-0`.
///
/// `serde_json` reads the integer `-0` as the float -0.0, which a whole number
/// does not read from, though `-0` has no fraction part. The space keeps the
/// JSON valid, and each byte where it was. A `-0` in a string, and a number
/// such as `-0.5` or `-0e1`, are left as they are, and so is the sign of an
/// exponent, as in `1e-0`.
fn unsigned_zeros(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let (mut in_string, mut escaped) = (false, false);
    let mut signs = Vec::new();
    for (i, &byte) in bytes.iter().enumerate() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'-'
            && !matches!(bytes[..i].last(), Some(b'e' | b'E'))
            && bytes.get(i + 1) == Some(&b'0')
            && !matches!(bytes.get(i + 2), Some(b'0'..=b'9' | b'.' | b'e' | b'E'))
        {
            signs.push(i);
        }
    }
    if signs.is_empty() {
        return Cow::Borrowed(text);
    }
    // Each sign is an ASCII byte, so the text splits on either side of it.
    let mut unsigned = String::with_capacity(text.len());
    let mut from = 0;
    for sign in signs {
        unsigned.push_str(&text[from..sign]);
        unsigned.push(' ');
        from = sign + 1;
    }
    unsigned.push_str(&text[from..]);
    Cow::Owned(unsigned)
}

/// A JSON value in which no object gives a key twice. `serde_json` keeps the
/// last of two equal keys and says nothing, so a record is read through this.
struct Unique(Value);

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Unique, D::Error> {
        deserializer.deserialize_any(UniqueVisitor).map(Unique)
    }
}

struct UniqueVisitor;

impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        Ok(Value::String(v.into()))
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::error::Category;

    fn category(event: EventType, text: &str) -> Option<Category> {
        Record::read(event, text).err().map(|e| e.category())
    }

    #[test]
    fn a_list_where_an_object_goes_is_malformed() {
        // serde's derived Deserialize would read each of these field by field.
        for (event, text) in [
            (
                EventType::Open,
                r#"["fyi", "today", "Nightly build moved"]"#,
            ),
            (
                EventType::Pickup,
                r#"{"actor": ["hV3kQ9mZr2pX7wLc0tBnYA"]}"#,
            ),
            (
                EventType::Open,
                r#"{"rows": [["a", "Install", "npm ci", "https://x"]]}"#,
            ),
            (EventType::Answer, r#"{"rows": [["a", "approve"]]}"#),
        ] {
            assert_eq!(category(event, text), Some(Category::Malformed), "{text}");
        }
    }

    #[test]
    fn null_is_malformed_inside_a_row_and_an_actor() {
        for (event, text) in [
            (EventType::Open, r#"{"rows": [{"id": "a", "label": null}]}"#),
            (
                EventType::Answer,
                r#"{"rows": [{"id": "a", "verdict": null}]}"#,
            ),
            (EventType::Note, r#"{"actor": {"id": null}}"#),
            (EventType::Open, r#"{"options": ["Pin", null]}"#),
        ] {
            assert_eq!(category(event, text), Some(Category::Malformed), "{text}");
        }
    }

    #[test]
    fn an_integer_minus_zero_is_the_whole_number_zero() {
        for text in [
            r#"{"other": true, "note": "x", "choice": -0}"#,
            "{\"choice\":\n-0\n,\"other\":true,\"note\":\"x\"}",
            r#"{"choice": -0, "other": true, "note": "x", "extra": [-0, "-0"]}"#,
        ] {
            let Ok(Record::Answer(answer)) = Record::read(EventType::Answer, text) else {
                panic!("{text} does not read");
            };
            assert_eq!((answer.choice, answer.other), (0, true), "{text}");
        }
        for text in [
            r#"{"choice": -0.0}"#,
            r#"{"choice": -0e0}"#,
            r#"{"choice": -01}"#,
        ] {
            assert_eq!(
                category(EventType::Answer, text),
                Some(Category::Malformed),
                "{text}"
            );
        }
        // In a string, -0 is text, and so is an escaped quote before it.
        let Ok(Record::Note(note)) = Record::read(EventType::Note, r#"{"body": "a \" -0, b\\"}"#)
        else {
            panic!("the note does not read");
        };
        assert_eq!(note.body, r#"a " -0, b\"#);
    }

    #[test]
    fn a_zero_exponent_is_json_inside_a_field_this_version_does_not_know() {
        for text in [
            r#"{"ack": true, "x": 1e-0}"#,
            r#"{"ack": true, "x": 2.5E-0}"#,
            r#"{"ack": true, "x": [1e-0, {"y": 3e-0}]}"#,
            r#"{"ack": true, "x": -0, "y": -1e-0}"#,
        ] {
            let Ok(Record::Answer(answer)) = Record::read(EventType::Answer, text) else {
                panic!("{text} does not read");
            };
            assert!(answer.ack, "{text}");
        }
        // In a field this version knows, the float is still not a whole number.
        assert_eq!(
            category(EventType::Answer, r#"{"choice": 1e-0}"#),
            Some(Category::Malformed)
        );
    }

    #[test]
    fn a_key_given_twice_is_malformed_inside_a_field_this_version_does_not_know() {
        let text = r#"{"ack": true, "extra": [{"n": 1, "n": 2}]}"#;
        assert_eq!(category(EventType::Answer, text), Some(Category::Malformed));
    }

    #[test]
    fn a_whole_number_is_an_integer_of_either_sign() {
        for text in [
            r#"{"choice": 1e0}"#,
            r#"{"choice": 1.5}"#,
            r#"{"choice": "1"}"#,
            r#"{"choice": 9223372036854775808}"#,
            r#"{"choice": true}"#,
        ] {
            assert_eq!(
                category(EventType::Answer, text),
                Some(Category::Malformed),
                "{text}"
            );
        }
        let Ok(Record::Answer(answer)) = Record::read(EventType::Answer, r#"{"choice": -1}"#)
        else {
            panic!("a negative choice does not read");
        };
        assert_eq!(answer.choice, -1);
    }

    /// Every record, with every field it has set, reads back from the text it
    /// serializes to. This holds the reader's field names to the ones serde
    /// writes.
    #[test]
    fn every_record_reads_back_from_its_json() {
        let actor = Some(Actor {
            id: "hV3kQ9mZr2pX7wLc0tBnYA".into(),
        });
        let at = "2026-09-15T11:01:00.25+02:00".parse().ok();
        let texts = || vec![String::from("one"), String::from("two")];
        let row = ApprovalRow {
            id: "a".into(),
            label: "Install deps".into(),
            script: "npm ci".into(),
            link: "https://example.com/a".into(),
            note: "Needs the VPN.".into(),
            by: "human".into(),
        };
        let records = [
            Record::Open(Open {
                kind: "approval".into(),
                urgency: "today".into(),
                title: "Run the release scripts".into(),
                body: "Body.".into(),
                options: texts(),
                recommend: 2,
                rows: vec![row.clone()],
                landed: true,
                left_out: texts(),
                links: texts(),
                labels: texts(),
                worker: "bun-pins".into(),
                brief: "BRIEF.md".into(),
                context: "Release 1.4".into(),
                for_: "Ryan".into(),
                re: "UkLWZ".into(),
                actor: actor.clone(),
                opened_at: at,
            }),
            Record::Amend(Amend {
                title: "Run the release scripts again".into(),
                body: "Body.".into(),
                options: texts(),
                recommend: -3,
                rows: vec![row],
                landed: Some(false),
                left_out: texts(),
                links: texts(),
                drop_links: texts(),
                labels: texts(),
                context: "Release 1.4".into(),
                reopen: true,
                actor: actor.clone(),
                amended_at: at,
            }),
            Record::Answer(Answer {
                choice: -2,
                other: true,
                rows: vec![RowVerdict {
                    id: "a".into(),
                    verdict: "hold".into(),
                    note: "later".into(),
                }],
                signoff: "changes".into(),
                briefing: "send_back".into(),
                file: vec![2, -1],
                text: "Text.".into(),
                drop: true,
                ack: true,
                note: "Note.".into(),
                amend_seq: 3,
                actor: actor.clone(),
                answered_at: at,
            }),
            Record::Pickup(Pickup {
                by: "bun-pins".into(),
                actor: actor.clone(),
                picked_up_at: at,
            }),
            Record::Note(Note {
                body: "Body.".into(),
                reopen: true,
                actor: actor.clone(),
                noted_at: at,
            }),
            Record::Close(Close {
                outcome: "Done.".into(),
                links: texts(),
                expired: true,
                actor: actor.clone(),
                closed_at: at,
            }),
            Record::Withdraw(Withdraw {
                reason: "Found it.".into(),
                actor: actor.clone(),
                withdrawn_at: at,
            }),
            Record::Park(Park {
                note: "after the release".into(),
                actor: actor.clone(),
                parked_at: at,
            }),
            Record::Resume(Resume {
                actor,
                resumed_at: at,
            }),
        ];
        for record in records {
            let text = record.to_json();
            assert!(text.contains("\"2026-09-15T09:01:00.25Z\""), "{text}");
            assert_eq!(Record::read(record.event(), &text), Ok(record), "{text}");
        }
    }
}
