//! Cases: the fold of a case's rows, and the writer.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Category, Error};
use crate::names::{Author, EventType, Kind, State, Urgency};
use crate::record::{Actor, Amend, Answer, ApprovalRow, Close, Note, Open, Park, Pickup, Record};
use crate::rules;
use crate::time::Time;

/// One stored event of a case: a row, as the store keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The row's number in its case: from 1, and rising, perhaps with gaps.
    pub seq: u64,
    /// Who wrote the event, as stored: `agent` or `human`.
    pub author: String,
    /// The event, as stored, such as `answer`.
    pub event: String,
    /// The record: JSON text, kept exactly as written.
    pub data: String,
}

/// An event for the writer: its author, its event and its record, as a
/// writer is handed them.
///
/// [`Append::new`] makes one from a typed record. Its fields are text so that
/// an append from outside, such as a form sent to the web inbox, is checked as
/// a stored row is: an unknown event, an author who may not write it, or a
/// record that does not read is refused with the category a replay would give
/// the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Append {
    /// Who writes the event: `agent` or `human`.
    pub author: String,
    /// The event, such as `answer`.
    pub event: String,
    /// The record, as JSON text. The writer stores the record as it reads
    /// it, so a field the event does not define is left out of the row.
    pub data: String,
}

impl Append {
    /// The append of `record`, written by `author`.
    #[must_use]
    pub fn new(author: Author, record: &Record) -> Append {
        Append {
            author: author.as_str().into(),
            event: record.event().as_str().into(),
            data: record.to_json(),
        }
    }
}

/// An event that folded: a row the case took.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Event {
    /// The row's seq.
    pub seq: u64,
    /// Who wrote the event.
    pub author: Author,
    /// The case's state before the event: `None` for the open event.
    pub from: Option<State>,
    /// The record, as the event reads it.
    pub record: Record,
    /// The record as stored: JSON text, with any fields this version does
    /// not know.
    pub data: String,
}

impl Event {
    /// What the event does.
    #[must_use]
    pub fn event(&self) -> EventType {
        self.record.event()
    }

    /// The event's time, when its record has one.
    #[must_use]
    pub fn at(&self) -> Option<Time> {
        self.record.at()
    }

    /// Who wrote the event, when its record says.
    #[must_use]
    pub fn actor(&self) -> Option<&Actor> {
        self.record.actor()
    }

    /// The name the event had as a file: its seq as at least four digits,
    /// its author and its event, such as `0004-agent-pickup.json`. Readers
    /// name events by it.
    #[must_use]
    pub fn file(&self) -> String {
        file(self.seq, self.author.as_str(), self.event().as_str())
    }
}

/// A row the fold skipped, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Problem {
    /// The row's seq.
    pub seq: u64,
    /// The row's author, as stored.
    pub author: String,
    /// The row's event, as stored.
    pub event: String,
    /// Why the case could not take the row.
    pub error: Error,
}

impl Problem {
    /// The name the row had as a file, as [`Event::file`] gives it.
    #[must_use]
    pub fn file(&self) -> String {
        file(self.seq, &self.author, &self.event)
    }
}

fn file(seq: u64, author: &str, event: &str) -> String {
    format!("{seq:04}-{author}-{event}.json")
}

/// A case: its stored rows, folded.
///
/// [`Case::fold`] makes one from a case's rows, and [`Case::open`] from a new
/// open event. [`Case::append`] checks a new event against the case and gives
/// the row to store. The properties are the ones `spec/vectors/README.md`
/// lists under "The case".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    state: State,
    revision: u64,
    /// The highest seq among the rows, skipped ones included.
    last_seq: u64,
    amend_seq: u64,
    updated_at: Option<Time>,
    kind: Kind,
    urgency: Urgency,
    /// The open event's record, with the body, context, options, recommend,
    /// rows, links and labels as the amends since left them.
    question: Open,
    answer: Option<Answer>,
    pickup: Option<Pickup>,
    park: Option<Park>,
    close: Option<Close>,
    events: Vec<Event>,
    problems: Vec<Problem>,
}

impl Case {
    /// Folds a case's stored rows, in the order the store keeps them.
    ///
    /// A row the case cannot take is skipped: it is listed in
    /// [`problems`](Case::problems) with its error, it still counts in the
    /// [`revision`](Case::revision), and the fold goes on with the next row.
    /// The fold fails, as [`NoOpen`](Category::NoOpen), only when no row is a
    /// valid open event.
    ///
    /// # Errors
    ///
    /// Gives [`Category::NoOpen`] when no row is a valid open event, so no
    /// case can be built.
    pub fn fold(rows: &[Row]) -> Result<Case, Error> {
        let mut case: Option<Case> = None;
        let mut problems = Vec::new();
        for row in rows {
            let taken = read(&row.author, &row.event, &row.data).and_then(|(author, record)| {
                let data = row.data.clone();
                if let Some(case) = case.as_mut() {
                    return case.take(row.seq, author, record, data, false);
                }
                case = Some(Case::start(row.seq, author, record, data, false)?);
                Ok(())
            });
            if let Err(error) = taken {
                problems.push(Problem {
                    seq: row.seq,
                    author: row.author.clone(),
                    event: row.event.clone(),
                    error,
                });
            }
        }
        let Some(mut case) = case else {
            // Every row was skipped, and each one says why.
            let reasons: Vec<String> = problems
                .iter()
                .map(|problem| format!("{}: {}", problem.file(), problem.error))
                .collect();
            let message = if reasons.is_empty() {
                String::from("the case has no rows")
            } else {
                format!("no row is a valid open event: {}", reasons.join("; "))
            };
            return Err(Error::new(Category::NoOpen, message));
        };
        case.problems = problems;
        case.revision = rows.len() as u64;
        case.last_seq = rows.iter().map(|row| row.seq).max().unwrap_or_default();
        Ok(case)
    }

    /// Opens a case: the writer's part of an `open`.
    ///
    /// `append` is checked as a stored open is, in the same order, and must
    /// also keep the rules only a write applies: no option or link given
    /// twice, no link of only white space, and no worker, brief or context of
    /// only white space (AG-02). An append of any other event is refused as a
    /// [`Transition`](Category::Transition), since a case with no open event
    /// takes only an open. The record gets the time `now` when it has none.
    /// The human opens a message, which starts [`Sent`](State::Sent), and
    /// the agent every other kind, which starts [`Open`](State::Open).
    ///
    /// `actor`, when given, is who writes the event, such as the actor a
    /// store keeps for its writer. The record gets it in place of any actor
    /// of its own, once that one has been checked: an append refused without
    /// `actor` is refused with it too. `actor` itself is checked as a
    /// record's actor is.
    ///
    /// Gives the case, and its first row, numbered 1, for the store to keep.
    ///
    /// # Errors
    ///
    /// Gives [`Category::Author`] when the author does not exist, or does not
    /// open the kind; [`Category::Unknown`] when the event, kind or urgency
    /// is not one this version knows; [`Category::Malformed`] when the record
    /// does not read as an open; [`Category::Invalid`] when it reads but
    /// breaks a rule of open, or its actor or `actor` has an id of the wrong
    /// shape; and [`Category::Transition`] when `append` is any other event.
    pub fn open(append: &Append, now: Time, actor: Option<&Actor>) -> Result<(Case, Row), Error> {
        let (author, record) = stamp(append, now, actor)?;
        let row = new_row(1, author, &record);
        let mut case = Case::start(1, author, record, row.data.clone(), true)?;
        case.revision = 1;
        case.last_seq = 1;
        Ok((case, row))
    }

    /// Appends an event to the case: the writer's part of a write.
    ///
    /// With `at_revision`, the revision the writer read the case at, the
    /// append is refused as [`Stale`](Category::Stale) unless the case still
    /// has that revision. That is checked first. Then the event is checked as
    /// a stored row is, in the same order, against the case as it stands; an
    /// open is a [`Transition`](Category::Transition), since a case has one.
    /// The record gets the time `now` when it has none, and keeps a time it
    /// has. With `actor`, the record gets that actor in place of its own, as
    /// [`Case::open`] says. The new row is numbered one above the highest seq
    /// among the case's rows, skipped ones included.
    ///
    /// When the event is accepted, the case changes as if the row were folded,
    /// and the row is returned for the store to keep. When it is refused, the
    /// case is left as it was.
    ///
    /// # Errors
    ///
    /// Gives [`Category::Stale`] when `at_revision` does not match the
    /// case's revision, or when an answer's `amend_seq` is below the case's;
    /// [`Category::Author`] when the author does not exist, or amends a case
    /// of a kind the other side opens;
    /// [`Category::Unknown`] when the event, kind or urgency is not one this
    /// version knows; [`Category::Malformed`] when the record does not read
    /// as its event's; [`Category::Invalid`] when it reads but breaks a rule
    /// of its event, or its actor or `actor` has an id of the wrong shape;
    /// and [`Category::Transition`] when the case's state or kind does not
    /// allow the event, including a second open and a note on a message, or
    /// the state does not allow it from its author, as for the agent's
    /// withdraw of an answered case.
    pub fn append(
        &mut self,
        append: &Append,
        now: Time,
        actor: Option<&Actor>,
        at_revision: Option<u64>,
    ) -> Result<Row, Error> {
        if let Some(read) = at_revision
            && read != self.revision
        {
            return Err(Error::stale(read, self.revision));
        }
        let (author, record) = stamp(append, now, actor)?;
        let seq = self
            .last_seq
            .checked_add(1)
            .ok_or_else(|| Error::transition("the case has no seq left for another row"))?;
        let row = new_row(seq, author, &record);
        self.take(seq, author, record, row.data.clone(), true)?;
        self.revision += 1;
        self.last_seq = seq;
        Ok(row)
    }

    /// Starts a case from its open event, as the checks after the record's
    /// actor give it: a case with no open takes only an open, then the open's
    /// own rules. A message starts sent, and every other kind open.
    fn start(
        seq: u64,
        author: Author,
        record: Record,
        data: String,
        write: bool,
    ) -> Result<Case, Error> {
        let Record::Open(open) = &record else {
            return Err(Error::transition(format!(
                "a case with no open event does not take {}",
                record.event()
            )));
        };
        let (kind, urgency) = rules::check_open(open, author, write)?;
        Ok(Case {
            // A case waits first on the side that did not open it: open is
            // waiting on the human, and sent waiting on an agent.
            state: match kind.opener() {
                Author::Human => State::Sent,
                Author::Agent => State::Open,
            },
            revision: 0,
            last_seq: 0,
            amend_seq: 0,
            updated_at: open.opened_at,
            kind,
            urgency,
            question: open.clone(),
            answer: None,
            pickup: None,
            park: None,
            close: None,
            events: vec![Event {
                seq,
                author,
                from: None,
                record,
                data,
            }],
            problems: Vec::new(),
        })
    }

    /// Takes an event into the case, as the checks after the record's actor
    /// give it: an amend's author, by the case's kind, and an answer's
    /// `amend_seq`, then the state, then the record's rules. Every check
    /// comes before any change, so a refused event leaves the case as it
    /// was. `write` adds the rule only a write applies: a note reopens an
    /// answered or picked-up case only when it says `reopen`.
    fn take(
        &mut self,
        seq: u64,
        author: Author,
        record: Record,
        data: String,
        write: bool,
    ) -> Result<(), Error> {
        let from = self.state;
        let event = record.event();
        match &record {
            Record::Open(_) => Err(Error::transition("the case has an open event already")),
            Record::Amend(amend) => self.take_amend(seq, author, amend),
            Record::Answer(answer) => self.take_answer(answer),
            Record::Pickup(pickup) => self.take_pickup(pickup),
            Record::Note(note) => self.take_note(note, write),
            Record::Close(close) => self.take_close(close),
            Record::Withdraw(_) => self.take_withdraw(author, event),
            Record::Park(park) => self.take_park(park),
            Record::Resume(_) => self.take_resume(),
        }?;
        if record.at() > self.updated_at {
            self.updated_at = record.at();
        }
        self.events.push(Event {
            seq,
            author,
            from: Some(from),
            record,
            data,
        });
        Ok(())
    }

    /// Takes an amend. The side that opened the case amends it: the human a
    /// message while it is sent, the agent any other case while it is open.
    /// The agent's amend that says `reopen` also takes an answered or
    /// picked-up case, and reopens it.
    fn take_amend(&mut self, seq: u64, author: Author, amend: &Amend) -> Result<(), Error> {
        let from = self.state;
        let event = EventType::Amend;
        rules::check_opener(self.kind, author, event)?;
        let reopens = matches!(from, State::Answered | State::PickedUp);
        self.allow(
            event,
            matches!(from, State::Open | State::Sent)
                || amend.reopen && reopens && self.kind.opener() == Author::Agent,
        )?;
        if amend.reopen && !reopens {
            return Err(Error::invalid(format!(
                "a case that is {from} is not reopened"
            )));
        }
        rules::check_amend(amend, self)?;
        if rules::changes_question(amend, self) {
            self.amend_seq = seq;
        }
        self.amend_question(amend);
        if amend.reopen {
            self.reopen();
        }
        Ok(())
    }

    /// Changes the question as a checked amend says: its lists grow, its
    /// dropped links go, and each field the amend sets replaces the old one.
    fn amend_question(&mut self, amend: &Amend) {
        let question = &mut self.question;
        question.options.extend_from_slice(&amend.options);
        question.rows.extend_from_slice(&amend.rows);
        question
            .links
            .retain(|link| !amend.drop_links.contains(link));
        question.links.extend_from_slice(&amend.links);
        question.labels.extend_from_slice(&amend.labels);
        if !amend.title.is_empty() {
            question.title.clone_from(&amend.title);
        }
        if !amend.body.is_empty() {
            question.body.clone_from(&amend.body);
        }
        if !amend.context.is_empty() {
            question.context.clone_from(&amend.context);
        }
        if amend.recommend != 0 {
            question.recommend = amend.recommend;
        }
        if let Some(landed) = amend.landed {
            question.landed = landed;
        }
        if !amend.left_out.is_empty() {
            question.left_out.clone_from(&amend.left_out);
        }
    }

    /// Takes an answer: one that saw the last amend that changed the
    /// question, to an open case, by the answer's rules.
    fn take_answer(&mut self, answer: &Answer) -> Result<(), Error> {
        if i128::from(answer.amend_seq) < i128::from(self.amend_seq) {
            return Err(Error::new(
                Category::Stale,
                format!(
                    "the answer saw amend {}, not amend {}, which changed the question",
                    answer.amend_seq, self.amend_seq
                ),
            ));
        }
        self.allow(EventType::Answer, self.state == State::Open)?;
        rules::check_answer(answer, self)?;
        self.answer = Some(answer.clone());
        self.state = State::Answered;
        Ok(())
    }

    /// Takes a pickup: the agent picks up an answer, or takes a message.
    fn take_pickup(&mut self, pickup: &Pickup) -> Result<(), Error> {
        self.allow(
            EventType::Pickup,
            matches!(self.state, State::Answered | State::Sent),
        )?;
        self.pickup = Some(pickup.clone());
        self.state = State::PickedUp;
        Ok(())
    }

    /// Takes a note. A note asks the human for a new answer, and a case the
    /// human opened, such as a message, never waits on the human.
    fn take_note(&mut self, note: &Note, write: bool) -> Result<(), Error> {
        let from = self.state;
        self.allow(
            EventType::Note,
            matches!(from, State::Open | State::Answered | State::PickedUp),
        )?;
        if self.kind.opener() == Author::Human {
            return Err(Error::transition(format!("a {} takes no note", self.kind)));
        }
        let reopens = from != State::Open;
        // The writer refuses a note that would reopen the case without
        // saying so. A stored one still reopens it: notes did before
        // `reopen`, and replay keeps them.
        if write && reopens && !note.reopen {
            return Err(Error::transition(format!(
                "a note on a case that is {from} would reopen it, and does not say reopen"
            )));
        }
        if rules::blank(&note.body) {
            return Err(Error::invalid("the note is blank"));
        }
        if note.reopen && !reopens {
            return Err(Error::invalid(format!(
                "a case that is {from} is not reopened"
            )));
        }
        if reopens {
            self.reopen();
        }
        Ok(())
    }

    /// Takes a close, with an outcome, of a picked-up case; an expired one
    /// only of a briefing.
    fn take_close(&mut self, close: &Close) -> Result<(), Error> {
        self.allow(EventType::Close, self.state == State::PickedUp)?;
        if rules::blank(&close.outcome) {
            return Err(Error::invalid("the outcome is blank"));
        }
        if close.expired && self.kind != Kind::Briefing {
            return Err(Error::invalid(format!(
                "only a briefing closes expired, and this is a {}",
                self.kind
            )));
        }
        self.close = Some(close.clone());
        self.state = State::Closed;
        Ok(())
    }

    /// Takes a withdraw. The agent withdraws only an open case. The human
    /// may also take back a message no agent has picked up, and end a case
    /// an agent has, answered or picked up, such as one whose session died;
    /// the answer and the pickup stay. The refusal names the author, since
    /// the rule depends on it.
    fn take_withdraw(&mut self, author: Author, event: EventType) -> Result<(), Error> {
        let from = self.state;
        if !(from == State::Open
            || author == Author::Human
                && matches!(from, State::Sent | State::Answered | State::PickedUp))
        {
            return Err(Error::transition(format!(
                "a case that is {from} does not take the {author}'s {event}"
            )));
        }
        self.state = State::Withdrawn;
        Ok(())
    }

    /// Takes a park: only a stuck case, and only while it is open.
    fn take_park(&mut self, park: &Park) -> Result<(), Error> {
        self.allow(EventType::Park, self.state == State::Open)?;
        if self.kind != Kind::Stuck {
            return Err(Error::transition(format!(
                "only a stuck case is parked, and this is a {}",
                self.kind
            )));
        }
        self.park = Some(park.clone());
        self.state = State::Parked;
        Ok(())
    }

    /// Takes a resume of a parked case, which opens it again.
    fn take_resume(&mut self) -> Result<(), Error> {
        self.allow(EventType::Resume, self.state == State::Parked)?;
        self.park = None;
        self.state = State::Open;
        Ok(())
    }

    /// Opens the case again, for a new answer: a note or an amend that
    /// reopens it.
    fn reopen(&mut self) {
        self.answer = None;
        self.pickup = None;
        self.state = State::Open;
    }

    /// Refuses `event` as a transition unless the state allows it.
    fn allow(&self, event: EventType, allowed: bool) -> Result<(), Error> {
        if allowed {
            Ok(())
        } else {
            Err(Error::transition(format!(
                "a case that is {} does not take {event}",
                self.state
            )))
        }
    }

    /// Where the case is.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// The number of stored rows, skipped ones included. A writer passes it
    /// back as `at_revision`.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The highest seq among the case's rows, skipped ones included. A
    /// form keeps it, with the [`revision`](Case::revision), to ask
    /// [`since`](Case::since) later how the case has changed.
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// The seq of the last amend that changed the question: one that added
    /// options or rows, or set a body or a briefing's left-out list other
    /// than the case's. 0 when there is
    /// none. An answer carries the `amend_seq` its writer read (ST-04).
    #[must_use]
    pub fn amend_seq(&self) -> u64 {
        self.amend_seq
    }

    /// The latest time of the events that folded, if any of them has one.
    #[must_use]
    pub fn updated_at(&self) -> Option<Time> {
        self.updated_at
    }

    /// The kind, from the open event.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The urgency, from the open event.
    #[must_use]
    pub fn urgency(&self) -> Urgency {
        self.urgency
    }

    /// The title, as the open event or the last amend that set one left it.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.question.title
    }

    /// The body, as the open event or the last amend that set one left it.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.question.body
    }

    /// The options: the open event's, then each amend's, in order.
    #[must_use]
    pub fn options(&self) -> &[String] {
        &self.question.options
    }

    /// The option the agent recommends, numbered from 1 as a choice is: the
    /// open event's, or the last amend's that set one. 0 when it recommends
    /// none. It is shown to the human, and an answer is never checked
    /// against it; a new one does not move the
    /// [`amend_seq`](Case::amend_seq).
    #[must_use]
    pub fn recommend(&self) -> i64 {
        self.question.recommend
    }

    /// The rows: the open event's, then each amend's, in order.
    #[must_use]
    pub fn rows(&self) -> &[ApprovalRow] {
        &self.question.rows
    }

    /// Whether a briefing's work has landed: the open event's, as the last
    /// amend that set it left it. `false` on every other kind.
    #[must_use]
    pub fn landed(&self) -> bool {
        self.question.landed
    }

    /// What a briefing left out on purpose: the open event's list, or the
    /// last amend's that set one. An answer files its items by number from 1.
    #[must_use]
    pub fn left_out(&self) -> &[String] {
        &self.question.left_out
    }

    /// The links: the open event's, then each amend's, in order.
    #[must_use]
    pub fn links(&self) -> &[String] {
        &self.question.links
    }

    /// The labels: the open event's, then each amend's, in order.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.question.labels
    }

    /// The worker, from the open event.
    #[must_use]
    pub fn worker(&self) -> &str {
        &self.question.worker
    }

    /// The brief, from the open event.
    #[must_use]
    pub fn brief(&self) -> &str {
        &self.question.brief
    }

    /// The context, as the open event or the last amend that set one left
    /// it.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.question.context
    }

    /// Who the case is for, from the open event's `for`.
    #[must_use]
    pub fn for_(&self) -> &str {
        &self.question.for_
    }

    /// The case this one is about, by its id, from the open event's `re`.
    #[must_use]
    pub fn re(&self) -> &str {
        &self.question.re
    }

    /// Who opened the case, when the open event says.
    #[must_use]
    pub fn actor(&self) -> Option<&Actor> {
        self.question.actor.as_ref()
    }

    /// When the case was opened, when the open event says.
    #[must_use]
    pub fn opened_at(&self) -> Option<Time> {
        self.question.opened_at
    }

    /// The answer the case stands on. A note or an amend that reopens the
    /// case clears it.
    #[must_use]
    pub fn answer(&self) -> Option<&Answer> {
        self.answer.as_ref()
    }

    /// The pickup of the answer the case stands on, or of the message. A note
    /// or an amend that reopens the case clears it.
    #[must_use]
    pub fn pickup(&self) -> Option<&Pickup> {
        self.pickup.as_ref()
    }

    /// The park, while the case is parked.
    #[must_use]
    pub fn park(&self) -> Option<&Park> {
        self.park.as_ref()
    }

    /// The close, once the case is closed.
    #[must_use]
    pub fn close(&self) -> Option<&Close> {
        self.close.as_ref()
    }

    /// The events that folded, in order.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The rows the fold skipped, in order.
    #[must_use]
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }
}

/// The checks that come before the case: the event is one this version
/// knows, the author may write it, the record reads, and its actor, if it has
/// one, has a valid id.
fn read(author: &str, event: &str, data: &str) -> Result<(Author, Record), Error> {
    let event: EventType = event.parse()?;
    let author = author
        .parse::<Author>()
        .ok()
        .filter(|author| event.written_by(*author))
        .ok_or_else(|| {
            Error::new(
                Category::Author,
                format!("{event} is not written by {author:?}"),
            )
        })?;
    let record = Record::read(event, data)?;
    rules::check_actor(record.actor())?;
    Ok((author, record))
}

/// Reads an append as a stored row is read, gives its record `actor`, when
/// there is one, in place of its own, and gives it the time `now` when it
/// has none.
fn stamp(append: &Append, now: Time, actor: Option<&Actor>) -> Result<(Author, Record), Error> {
    let (author, mut record) = read(&append.author, &append.event, &append.data)?;
    if let Some(actor) = actor {
        rules::check_actor(Some(actor))?;
        *record.actor_mut() = Some(actor.clone());
    }
    record.at_mut().get_or_insert(now);
    Ok((author, record))
}

/// The row that stores `record`: its time in UTC, and each field at its zero
/// value left out.
fn new_row(seq: u64, author: Author, record: &Record) -> Row {
    Row {
        seq,
        author: author.as_str().into(),
        event: record.event().as_str().into(),
        data: record.to_json(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Answer;

    fn now() -> Time {
        "2026-09-15T09:01:00Z".parse().unwrap()
    }

    fn append(author: &str, event: &str, data: &str) -> Append {
        Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        }
    }

    fn opened() -> Case {
        let open = r#"{"kind": "fyi", "urgency": "today", "title": "Nightly build moved"}"#;
        Case::open(&append("agent", "open", open), now(), None)
            .unwrap()
            .0
    }

    #[test]
    fn only_an_open_opens_a_case() {
        let answer = append("human", "answer", r#"{"ack": true}"#);
        let error = Case::open(&answer, now(), None).unwrap_err();
        assert_eq!(error.category(), Category::Transition);
    }

    #[test]
    fn a_case_takes_no_second_open() {
        let mut case = opened();
        let open = r#"{"kind": "fyi", "urgency": "today", "title": "Again"}"#;
        let error = case.append(&append("agent", "open", open), now(), None, None);
        assert_eq!(error.unwrap_err().category(), Category::Transition);
        assert_eq!(case, opened());
    }

    #[test]
    fn a_row_stores_the_record_as_read() {
        // A field the event does not define is left out, and so is a field
        // at its zero value. The time is stamped, in UTC.
        let mut case = opened();
        let answer = r#"{"ack": true, "note": "", "colour": "teal"}"#;
        let row = case
            .append(&append("human", "answer", answer), now(), None, Some(1))
            .unwrap();
        assert_eq!(row.seq, 2);
        assert_eq!(
            row.data,
            r#"{"ack":true,"answered_at":"2026-09-15T09:01:00Z"}"#
        );
    }

    #[test]
    fn a_typed_record_appends_as_its_json_would() {
        let answer = Answer {
            ack: true,
            ..Answer::default()
        };
        let typed = Append::new(Author::Human, &Record::Answer(answer));
        assert_eq!(typed, append("human", "answer", r#"{"ack":true}"#));
        let mut case = opened();
        case.append(&typed, now(), None, None).unwrap();
        assert_eq!(case.state(), State::Answered);
    }

    #[test]
    fn an_actor_id_needs_both_its_length_and_its_alphabet() {
        for id in [
            "hV3kQ9mZr2pX7wLc0tBnY",
            "hV3kQ9mZr2pX7wLc0tBnYAA",
            "hV3kQ9mZr2pX7wLc0tBn+A",
        ] {
            let open = format!(
                r#"{{"kind": "fyi", "urgency": "today", "title": "T", "actor": {{"id": "{id}"}}}}"#
            );
            let error = Case::open(&append("agent", "open", &open), now(), None).unwrap_err();
            assert_eq!(error.category(), Category::Invalid, "{id}");
        }
        let open = r#"{"kind": "fyi", "urgency": "today", "title": "T", "actor": {"id": "hV3kQ9mZr2pX7wLc0tBn-_"}}"#;
        assert!(Case::open(&append("agent", "open", open), now(), None).is_ok());
    }

    fn actor(id: &str) -> Actor {
        Actor { id: id.into() }
    }

    /// A store's writer: the actor the record gets.
    const WRITER: &str = "Q2xbTPWyJh8kVs3dNr6aLg";

    #[test]
    fn the_writers_actor_goes_into_the_record_in_place_of_its_own() {
        let writer = actor(WRITER);
        for open in [
            r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
            r#"{"kind": "fyi", "urgency": "today", "title": "T", "actor": {"id": "hV3kQ9mZr2pX7wLc0tBnYA"}}"#,
        ] {
            let (case, row) =
                Case::open(&append("agent", "open", open), now(), Some(&writer)).unwrap();
            assert_eq!(case.actor(), Some(&writer), "{open}");
            assert_eq!(
                row.data,
                r#"{"kind":"fyi","urgency":"today","title":"T","actor":{"id":"Q2xbTPWyJh8kVs3dNr6aLg"},"opened_at":"2026-09-15T09:01:00Z"}"#
            );
        }
        let mut case = opened();
        let row = case
            .append(
                &append("human", "answer", r#"{"ack": true}"#),
                now(),
                Some(&writer),
                Some(1),
            )
            .unwrap();
        assert_eq!(
            row.data,
            r#"{"ack":true,"actor":{"id":"Q2xbTPWyJh8kVs3dNr6aLg"},"answered_at":"2026-09-15T09:01:00Z"}"#
        );
        assert_eq!(case.answer().and_then(|a| a.actor.as_ref()), Some(&writer));
        assert_eq!(Case::fold(&[opened_row(), row]).unwrap(), case);
    }

    fn opened_row() -> Row {
        let open = r#"{"kind": "fyi", "urgency": "today", "title": "Nightly build moved"}"#;
        Case::open(&append("agent", "open", open), now(), None)
            .unwrap()
            .1
    }

    #[test]
    fn a_records_own_actor_is_checked_before_the_writers_takes_its_place() {
        let writer = actor(WRITER);
        let open =
            r#"{"kind": "fyi", "urgency": "today", "title": "T", "actor": {"id": "bun-pins"}}"#;
        let error = Case::open(&append("agent", "open", open), now(), Some(&writer)).unwrap_err();
        assert_eq!(error.category(), Category::Invalid);
        let mut case = opened();
        let answer = r#"{"ack": true, "actor": {}}"#;
        let error = case.append(
            &append("human", "answer", answer),
            now(),
            Some(&writer),
            None,
        );
        assert_eq!(error.unwrap_err().category(), Category::Invalid);
        assert_eq!(case, opened());
    }

    #[test]
    fn a_writers_actor_is_checked_as_a_records_is() {
        for id in ["bun-pins", "hV3kQ9mZr2pX7wLc0tBn+A", ""] {
            let writer = actor(id);
            let open = r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#;
            let error =
                Case::open(&append("agent", "open", open), now(), Some(&writer)).unwrap_err();
            assert_eq!(error.category(), Category::Invalid, "{id:?}");
            let mut case = opened();
            let note = append("agent", "note", r#"{"body": "More."}"#);
            let error = case.append(&note, now(), Some(&writer), None).unwrap_err();
            assert_eq!(error.category(), Category::Invalid, "{id:?}");
            assert_eq!(case, opened());
        }
        // The author and the event are checked before the writer's actor.
        let writer = actor("bun-pins");
        let error = case_error(&append("robot", "note", r#"{"body": "More."}"#), &writer);
        assert_eq!(error, Category::Author);
        let error = case_error(&append("agent", "poke", "{}"), &writer);
        assert_eq!(error, Category::Unknown);
    }

    fn case_error(append: &Append, writer: &Actor) -> Category {
        opened()
            .append(append, now(), Some(writer), None)
            .unwrap_err()
            .category()
    }

    #[test]
    fn a_stale_append_says_which_revisions() {
        let mut case = opened();
        let error = case
            .append(
                &append("human", "answer", r#"{"ack": true}"#),
                now(),
                None,
                Some(0),
            )
            .unwrap_err();
        assert_eq!(error, Error::stale(0, 1));
    }

    #[test]
    fn an_approval_of_one_row_takes_a_verdict_on_it() {
        let open = r#"{"kind": "approval", "urgency": "today", "title": "T", "rows": [{"id": "a", "label": "Install", "script": "npm ci", "link": "https://example.com/a"}]}"#;
        let mut case = Case::open(&append("agent", "open", open), now(), None)
            .unwrap()
            .0;
        let answer = r#"{"rows": [{"id": "a", "verdict": "approve"}]}"#;
        case.append(&append("human", "answer", answer), now(), None, Some(1))
            .unwrap();
        assert_eq!(case.state(), State::Answered);
    }
}
