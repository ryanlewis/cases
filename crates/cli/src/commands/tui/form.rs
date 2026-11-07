//! The form for one write to a case (TU-05): an answer in the shape its
//! kind asks for, a park, or a withdraw. It holds what is typed and chosen,
//! and the revision, last seq and amend seq the case had when it was drawn,
//! and turns into the record to send.

use cases_core::{Answer, Case, Kind, Park, Record, RowVerdict, Since, State, Withdraw};
use cases_store_sqlite::Key;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::Hit;
use super::field::Field;

/// What the form writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Answer,
    Park,
    Withdraw,
}

impl Act {
    /// The event the act writes, for the line that says it was recorded.
    pub(crate) fn event(self) -> &'static str {
        match self {
            Act::Answer => "answer",
            Act::Park => "park",
            Act::Withdraw => "withdraw",
        }
    }

    /// Whether the case can take the act now. A message is not answered.
    pub(crate) fn fits(self, case: &Case) -> bool {
        match self {
            Act::Answer => case.state() == State::Open && case.kind() != Kind::Message,
            Act::Park => case.state() == State::Open && case.kind() == Kind::Stuck,
            // A sent message can be taken back too, before or after an agent
            // takes it, until it is closed (HU-08).
            Act::Withdraw => matches!(
                case.state(),
                State::Answered | State::PickedUp | State::Sent
            ),
        }
    }
}

/// Which field has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Text,
    Note,
    /// The note on an approval row, by its place in the case.
    Row(usize),
}

/// An approval row's verdict as the form holds it (IN-03).
#[derive(Debug)]
pub(crate) struct RowPick {
    pub(crate) id: String,
    /// `approve`, `hold` or `reject`, once chosen.
    pub(crate) verdict: Option<&'static str>,
    pub(crate) note: Field,
    /// Whether the human carries the step out (`by: human`), so approve
    /// says it is done (case i2lDf).
    pub(crate) human: bool,
}

/// The word for a row's verdict: on a step the human carries out, approve
/// is "done" and reject is "won't do" (case i2lDf). It is still sent as
/// the verdict.
pub(crate) fn verdict_word(verdict: &str, human: bool) -> &str {
    match (verdict, human) {
        ("approve", true) => "done",
        ("reject", true) => "won't do",
        _ => verdict,
    }
}

/// A question the form asks before it goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    /// Send a drop.
    Drop,
    /// Send the withdraw.
    Withdraw,
    /// Throw away what was typed.
    Discard,
}

/// A part of the answer's panel the keys can be on (TU-05, TU-12): tab
/// and shift-tab move between them, in the order [`Form::parts`] gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spot {
    /// A decision's options, a signoff's or a briefing's two answers.
    Choices,
    /// An approval's rows, with their verdicts.
    Rows,
    /// A briefing's left-out items, to file.
    Items,
    /// A field to type in.
    Field(Which),
    /// One of the panel's buttons.
    Button(Press),
}

/// A button of the answer's panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Press {
    /// Send the answer: acknowledge, for an fyi.
    Send,
    /// Park a stuck case instead, in the park's form.
    Park,
    /// Drop the case, after asking: the agent stops the work.
    Drop,
    /// Throw away what was chosen or typed, after asking: the case stays
    /// as it is, and its answer starts again.
    Discard,
}

impl Press {
    /// The buttons of the answer's panel of a case of `kind`, as the web
    /// inbox has them: send, then park on a stuck case, and drop; and
    /// discard, once something was chosen or typed.
    pub(crate) fn of(kind: Kind, touched: bool) -> Vec<Press> {
        let mut presses = vec![Press::Send];
        if kind == Kind::Stuck {
            presses.push(Press::Park);
        }
        presses.push(Press::Drop);
        if touched {
            presses.push(Press::Discard);
        }
        presses
    }
}

/// What a key did to the form.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// Nothing for the app to do.
    Stay,
    /// Send this record.
    Send(Box<Record>),
    /// Close the form, sending nothing.
    Cancel,
    /// Open the field's text in the editor.
    Edit(Which),
    /// Nothing was sent: this is missing.
    Missing(&'static str),
    /// Leave the answer's panel, keeping what is in it as a draft.
    Leave,
    /// Park the case instead, in the park's form.
    Park,
}

/// A form for one case.
#[derive(Debug)]
pub(crate) struct Form {
    pub(crate) key: Key,
    pub(crate) kind: Kind,
    pub(crate) act: Act,
    /// The case's revision when the form was drawn, or after a note.
    pub(crate) revision: u64,
    /// The case's amend seq at that revision.
    pub(crate) amend_seq: u64,
    /// The case's last seq at that revision: the rows above it are the
    /// ones since.
    seq: u64,
    options: i64,
    pub(crate) choice: Option<i64>,
    pub(crate) other: bool,
    /// `accept` or `changes`, for a signoff.
    pub(crate) signoff: Option<&'static str>,
    /// `accept` or `send_back`, for a briefing.
    pub(crate) briefing: Option<&'static str>,
    /// A briefing's left-out items as the form was drawn, and whether each
    /// is ticked to file.
    pub(crate) left_out: Vec<String>,
    pub(crate) file: Vec<bool>,
    pub(crate) text: Field,
    pub(crate) note: Field,
    /// An approval's rows, in the case's order, and the one the keys act on.
    pub(crate) rows: Vec<RowPick>,
    pub(crate) row: usize,
    pub(crate) editing: Option<Which>,
    pub(crate) confirm: Option<Confirm>,
    /// The part of the answer's panel the keys are on, when no field has
    /// them: see [`Form::spot`].
    spot: Option<Spot>,
    /// The left-out item the keys are on, among a briefing's items.
    pub(crate) item: usize,
    /// Whether the next frame brings the part of the answer's panel the
    /// keys are on into sight: after a key, which may have moved or grown
    /// it.
    pub(crate) reveal: bool,
}

impl Form {
    pub(crate) fn new(key: Key, case: &Case, act: Act) -> Form {
        let kind = case.kind();
        // Text is the response of a question or a stuck case: typing starts
        // there. A park's note and a withdraw's reason are the only field.
        let editing = match (act, kind) {
            (Act::Answer, Kind::Question | Kind::Stuck) => Some(Which::Text),
            (Act::Park | Act::Withdraw, _) => Some(Which::Note),
            _ => None,
        };
        Form {
            key,
            kind,
            act,
            revision: case.revision(),
            amend_seq: case.amend_seq(),
            seq: case.last_seq(),
            options: i64::try_from(case.options().len()).unwrap_or(i64::MAX),
            choice: None,
            other: false,
            signoff: None,
            briefing: None,
            left_out: case.left_out().to_vec(),
            file: vec![false; case.left_out().len()],
            text: Field::default(),
            note: Field::default(),
            rows: case
                .rows()
                .iter()
                .map(|row| RowPick {
                    id: row.id.clone(),
                    verdict: None,
                    note: Field::default(),
                    human: row.by == "human",
                })
                .collect(),
            row: 0,
            editing,
            confirm: None,
            spot: None,
            item: 0,
            reveal: false,
        }
    }

    /// How many options a decision has.
    pub(crate) fn options(&self) -> i64 {
        self.options
    }

    /// The part of the answer's panel the keys are on: the field being
    /// typed in, a row's note being on the rows; or else the part tab last
    /// moved to or a click last landed on; or else the first.
    pub(crate) fn spot(&self) -> Spot {
        match self.being_edited() {
            Some(Which::Row(_)) => Spot::Rows,
            Some(which) => Spot::Field(which),
            // A part that has gone, as discard once nothing is chosen or
            // typed, leaves the keys on the first.
            None => {
                let parts = self.parts();
                self.spot
                    .filter(|spot| parts.contains(spot))
                    .or_else(|| parts.first().copied())
                    .unwrap_or(Spot::Button(Press::Send))
            }
        }
    }

    /// The parts of the answer's panel, in the order tab moves through
    /// them: what its kind asks for, the note, then its buttons.
    pub(crate) fn parts(&self) -> Vec<Spot> {
        let mut parts = Vec::new();
        match self.kind {
            Kind::Decision | Kind::Signoff => parts.push(Spot::Choices),
            Kind::Briefing => {
                parts.push(Spot::Choices);
                if !self.file.is_empty() {
                    parts.push(Spot::Items);
                }
            }
            Kind::Approval if !self.rows.is_empty() => parts.push(Spot::Rows),
            Kind::Question | Kind::Stuck => parts.push(Spot::Field(Which::Text)),
            _ => {}
        }
        parts.push(Spot::Field(Which::Note));
        parts.extend(self.presses().into_iter().map(Spot::Button));
        parts
    }

    /// The buttons of the answer's panel, as [`Press::of`] gives them.
    pub(crate) fn presses(&self) -> Vec<Press> {
        Press::of(self.kind, self.touched())
    }

    /// Moves the keys `by` parts along the panel: into a field to type in
    /// it, or onto anything else. Gives whether there was a part to move
    /// to: past the last, or before the first, the keys leave the panel,
    /// out of any field, for the list (TU-12).
    pub(crate) fn tab(&mut self, by: isize) -> bool {
        let parts = self.parts();
        let next = match parts.iter().position(|part| *part == self.spot()) {
            Some(at) => at.checked_add_signed(by),
            None if by > 0 => Some(0),
            None => parts.len().checked_sub(1),
        };
        if let Some(part) = next.and_then(|at| parts.get(at)).copied() {
            self.go(part);
            true
        } else {
            self.spot = Some(self.spot());
            self.editing = None;
            false
        }
    }

    /// Puts the keys on `part`: in its field, when it is one.
    pub(crate) fn go(&mut self, part: Spot) {
        self.spot = Some(part);
        self.editing = match part {
            Spot::Field(which) => Some(which),
            _ => None,
        };
    }

    /// Answers by a number (TU-12): a decision's option, other being the
    /// number after the last, with the keys put in the note to say it; a
    /// signoff's or a briefing's 1 to accept, 2 for changes or to send it
    /// back; or an approval's row, with the keys on its verdicts. Gives
    /// whether the number names one.
    pub(crate) fn pick(&mut self, n: u32) -> bool {
        let wide = i64::from(n);
        let at = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
        let marks = match self.kind {
            Kind::Signoff => ["accept", "changes"],
            _ => ["accept", "send_back"],
        };
        let mark = usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|at| marks.get(at).copied());
        match self.kind {
            Kind::Decision if (1..=self.options).contains(&wide) => {
                self.choice = Some(wide);
                self.other = false;
                self.go(Spot::Choices);
            }
            Kind::Decision if wide == self.options + 1 => {
                self.choice = None;
                self.other = true;
                self.go(Spot::Field(Which::Note));
            }
            Kind::Signoff if mark.is_some() => {
                self.signoff = mark;
                self.go(Spot::Choices);
            }
            Kind::Briefing if mark.is_some() => {
                self.briefing = mark;
                self.go(Spot::Choices);
            }
            Kind::Approval if (1..=self.rows.len()).contains(&at(n)) => {
                self.row = at(n) - 1;
                self.go(Spot::Rows);
            }
            _ => return false,
        }
        true
    }

    /// Presses one of the panel's buttons.
    pub(crate) fn press(&mut self, press: Press) -> Outcome {
        self.spot = Some(Spot::Button(press));
        self.editing = None;
        match press {
            Press::Send => self.send(),
            Press::Park => Outcome::Park,
            Press::Drop => {
                self.confirm = Some(Confirm::Drop);
                Outcome::Stay
            }
            Press::Discard => {
                self.confirm = Some(Confirm::Discard);
                Outcome::Stay
            }
        }
    }

    /// Whether anything was chosen or typed, which leaving the panel keeps
    /// as a draft.
    pub(crate) fn touched(&self) -> bool {
        self.typed()
            || self.choice.is_some()
            || self.other
            || self.signoff.is_some()
            || self.briefing.is_some()
            || self.file.iter().any(|tick| *tick)
            || self.rows.iter().any(|row| row.verdict.is_some())
    }

    /// Whether anything was typed that a cancel would lose.
    pub(crate) fn typed(&self) -> bool {
        !self.text.is_blank()
            || !self.note.is_blank()
            || self.rows.iter().any(|row| !row.note.is_blank())
    }

    /// Takes the case as amended, after a send was refused as stale
    /// (IN-05): its revision, amend seq and options, and its rows. A row
    /// keeps its verdict and note by its id, a row the case no longer has
    /// goes, and a row the amend added has neither. The keys stay on the
    /// same row where it is still there.
    pub(crate) fn amended(&mut self, case: &Case) {
        self.revision = case.revision();
        self.amend_seq = case.amend_seq();
        self.seq = case.last_seq();
        self.take_options(case);
        let id = |at: usize, rows: &[RowPick]| rows.get(at).map(|row| row.id.clone());
        let row = id(self.row, &self.rows);
        let editing = match self.editing {
            Some(Which::Row(at)) => Some(id(at, &self.rows)),
            _ => None,
        };
        let mut old = std::mem::take(&mut self.rows);
        self.rows = case
            .rows()
            .iter()
            .map(|row| {
                let mut pick = match old.iter().position(|pick| pick.id == row.id) {
                    Some(at) => old.swap_remove(at),
                    None => RowPick {
                        id: row.id.clone(),
                        verdict: None,
                        note: Field::default(),
                        human: false,
                    },
                };
                pick.human = row.by == "human";
                pick
            })
            .collect();
        let at =
            |id: Option<String>| id.and_then(|id| self.rows.iter().position(|row| row.id == id));
        self.row = at(row).unwrap_or(self.row.min(self.rows.len().saturating_sub(1)));
        if let Some(id) = editing {
            self.editing = at(id).map(Which::Row);
        }
    }

    /// Follows the case as a new read shows it (IN-05, IN-06), by the
    /// core's freshness rule ([`Case::since`]). The form takes the case's
    /// new revision when the case is unchanged since it was drawn, as after
    /// a note, an amend that left the question as it was, or the human's
    /// own edit of a sent message (IN-13); or when the case moved to
    /// another state the form still fits without passing any state twice,
    /// as a withdraw does when an agent picks the case up. Otherwise the
    /// case changed: answered elsewhere and reopened, parked and resumed,
    /// asked another question, or with a row the fold skipped. The form
    /// keeps its revision, so that its send is refused as stale. The
    /// options an amend added can be chosen either way.
    pub(crate) fn follow(&mut self, case: &Case) {
        let take = match case.since(self.revision, self.seq) {
            Since::Unchanged => true,
            Since::Moved => self.act.fits(case),
            _ => false,
        };
        if take {
            self.revision = case.revision();
            self.amend_seq = case.amend_seq();
            self.seq = case.last_seq();
        }
        self.take_options(case);
    }

    /// Takes the case's options, so that one an amend added can be chosen
    /// as soon as it shows, and a briefing's new left-out list. An item is
    /// filed by its number, so a new list takes no ticks from the old one.
    pub(crate) fn take_options(&mut self, case: &Case) {
        self.options = i64::try_from(case.options().len()).unwrap_or(i64::MAX);
        if self.left_out != case.left_out() {
            self.left_out = case.left_out().to_vec();
            self.file = vec![false; self.left_out.len()];
        }
    }

    /// The field `which` types into: a row past the end is the last one,
    /// as the row the keys act on is kept to the rows, and with no rows it
    /// is the case's note.
    fn resolve(&self, which: Which) -> Which {
        match which {
            Which::Row(at) if !self.rows.is_empty() => Which::Row(at.min(self.rows.len() - 1)),
            Which::Row(_) => Which::Note,
            other => other,
        }
    }

    pub(crate) fn field(&mut self, which: Which) -> &mut Field {
        match self.resolve(which) {
            Which::Text => &mut self.text,
            Which::Row(at) => &mut self.rows[at].note,
            Which::Note => &mut self.note,
        }
    }

    /// The field the keys type into, if any: the one the view marks.
    pub(crate) fn being_edited(&self) -> Option<Which> {
        self.editing.map(|which| self.resolve(which))
    }

    /// Takes a key.
    pub(crate) fn key(&mut self, key: KeyEvent) -> Outcome {
        self.reveal = self.act == Act::Answer;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(confirm) = self.confirm.take() {
            return self.confirmed(confirm, key);
        }
        if ctrl && key.code == KeyCode::Char('s') {
            return self.send();
        }
        if ctrl && key.code == KeyCode::Char('e') {
            return Outcome::Edit(self.edited());
        }
        if let Some(which) = self.editing {
            let line = key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
            match key.code {
                // Enter sends, and shift- or alt-enter is a new line. In
                // the answer's panel tab moves on to the next part, or out
                // of the panel past its ends, and esc leaves the field for
                // the panel.
                KeyCode::Enter if !line => return self.send(),
                KeyCode::Tab if self.act == Act::Answer && !self.tab(1) => {
                    return Outcome::Leave;
                }
                KeyCode::BackTab if self.act == Act::Answer && !self.tab(-1) => {
                    return Outcome::Leave;
                }
                KeyCode::Tab | KeyCode::BackTab if self.act == Act::Answer => {}
                KeyCode::Esc if self.act == Act::Answer => {
                    self.spot = Some(self.spot());
                    self.editing = None;
                }
                KeyCode::Esc | KeyCode::Tab => self.editing = None,
                _ => {
                    self.field(which).key(key);
                }
            }
            return Outcome::Stay;
        }
        if self.act == Act::Answer {
            return self.panel_key(key.code);
        }
        self.command(key.code)
    }

    /// A key in the answer's panel with no field typing: tab and shift-tab
    /// move between its parts, and out past its ends, esc leaves it, a
    /// number picks, a part's own keys move on it or press it, and the
    /// form's other keys are as they were (TU-05).
    fn panel_key(&mut self, code: KeyCode) -> Outcome {
        match (code, self.spot()) {
            (KeyCode::Tab, _) if !self.tab(1) => return Outcome::Leave,
            (KeyCode::BackTab, _) if !self.tab(-1) => return Outcome::Leave,
            (KeyCode::Tab | KeyCode::BackTab, _) => {}
            (KeyCode::Esc, _) => return Outcome::Leave,
            // A number picks what it names, but on a briefing's items,
            // where it ticks the item it names to file.
            (KeyCode::Char(c @ '0'..='9'), spot)
                if !(self.kind == Kind::Briefing && spot == Spot::Items) =>
            {
                self.pick(c.to_digit(10).unwrap_or(0));
            }
            (KeyCode::Enter | KeyCode::Char(' '), Spot::Button(press)) => return self.press(press),
            (KeyCode::Char('j') | KeyCode::Down, Spot::Choices) => self.move_choice(1),
            (KeyCode::Char('k') | KeyCode::Up, Spot::Choices) => self.move_choice(-1),
            (KeyCode::Char('j') | KeyCode::Down, Spot::Items) => {
                self.item = (self.item + 1).min(self.file.len().saturating_sub(1));
            }
            (KeyCode::Char('k') | KeyCode::Up, Spot::Items) => {
                self.item = self.item.saturating_sub(1);
            }
            (KeyCode::Char(' '), Spot::Items) => {
                if let Some(tick) = self.file.get_mut(self.item) {
                    *tick = !*tick;
                }
            }
            _ => return self.command(code),
        }
        Outcome::Stay
    }

    /// Moves the choice `by` one along the panel's choices: a decision's
    /// options then other; a signoff's or a briefing's two answers.
    fn move_choice(&mut self, by: i64) {
        match self.kind {
            Kind::Decision => {
                let at = match (self.choice, self.other) {
                    (_, true) => self.options + 1,
                    (Some(n), _) => n,
                    (None, false) => 0,
                };
                let next = (at + by).clamp(1, self.options + 1);
                if next > self.options {
                    self.choice = None;
                    self.other = true;
                } else {
                    self.choice = Some(next);
                    self.other = false;
                }
            }
            Kind::Signoff => {
                self.signoff = Some(if by > 0 { "changes" } else { "accept" });
            }
            Kind::Briefing => {
                self.briefing = Some(if by > 0 { "send_back" } else { "accept" });
            }
            _ => {}
        }
    }

    /// A click on a part of the form or its case (TU-12): as the key that
    /// does the same, a choice by its number, any one of them, an item to
    /// file, a row to act on, or a field to type in. A click while the form
    /// asks a question takes it back, as any other key.
    pub(crate) fn click(&mut self, hit: Hit) {
        if self.confirm.take().is_some()
            || (self.act != Act::Answer && hit != Hit::Field(Which::Note))
        {
            return;
        }
        match hit {
            Hit::Choice(n) if self.kind == Kind::Decision && (1..=self.options).contains(&n) => {
                self.choice = Some(n);
                self.other = false;
                self.go(Spot::Choices);
            }
            // Other, after the options: its note is where it is said.
            Hit::Choice(n) if self.kind == Kind::Decision && n == self.options + 1 => {
                self.choice = None;
                self.other = true;
                self.go(Spot::Field(Which::Note));
            }
            Hit::Mark(mark) if self.kind == Kind::Signoff => {
                self.signoff = Some(mark);
                self.go(Spot::Choices);
            }
            Hit::Mark(mark) if self.kind == Kind::Briefing => {
                self.briefing = Some(mark);
                self.go(Spot::Choices);
            }
            Hit::LeftOut(at) if self.kind == Kind::Briefing => {
                if let Some(tick) = self.file.get_mut(at) {
                    *tick = !*tick;
                }
                self.item = at;
                self.go(Spot::Items);
            }
            Hit::ApprovalRow(at) if at < self.rows.len() => {
                self.row = at;
                self.go(Spot::Rows);
            }
            Hit::Verdict(at, verdict) if at < self.rows.len() => {
                self.row = at;
                if let Some(row) = self.rows.get_mut(at) {
                    row.verdict = Some(verdict);
                }
                self.go(Spot::Rows);
            }
            Hit::Field(Which::Text) if matches!(self.kind, Kind::Question | Kind::Stuck) => {
                self.editing = Some(Which::Text);
            }
            Hit::Field(Which::Note) => self.editing = Some(Which::Note),
            Hit::Field(Which::Row(at)) if at < self.rows.len() => {
                self.row = at;
                self.editing = Some(Which::Row(at));
            }
            _ => {}
        }
    }

    /// The answer to a question the form asked: y or Y goes on, any other
    /// key takes the question back.
    fn confirmed(&mut self, confirm: Confirm, key: KeyEvent) -> Outcome {
        if !matches!(key.code, KeyCode::Char('y' | 'Y')) {
            return Outcome::Stay;
        }
        match confirm {
            Confirm::Drop => Outcome::Send(Box::new(self.answer(true))),
            Confirm::Withdraw => Outcome::Send(Box::new(self.record())),
            Confirm::Discard => Outcome::Cancel,
        }
    }

    /// The field the editor opens: the one being typed in, or else the
    /// form's first field.
    fn edited(&self) -> Which {
        self.editing.unwrap_or(match (self.act, self.kind) {
            (Act::Answer, Kind::Question | Kind::Stuck) => Which::Text,
            _ => Which::Note,
        })
    }

    /// A key when no field has the keys: the form's own, then its kind's.
    fn command(&mut self, code: KeyCode) -> Outcome {
        match code {
            KeyCode::Esc if self.typed() => self.confirm = Some(Confirm::Discard),
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return self.send(),
            KeyCode::Char('n') => self.editing = Some(Which::Note),
            KeyCode::Char('t' | 'i')
                if self.act == Act::Answer && matches!(self.kind, Kind::Question | Kind::Stuck) =>
            {
                self.editing = Some(Which::Text);
            }
            KeyCode::Char('d') if self.act == Act::Answer => self.confirm = Some(Confirm::Drop),
            _ => self.kind_key(code),
        }
        Outcome::Stay
    }

    /// A key that is not the form's own, for the keys of its kind. Only an
    /// answer has them: a park or a withdraw sends the note alone.
    fn kind_key(&mut self, code: KeyCode) {
        if self.act != Act::Answer {
            return;
        }
        match self.kind {
            Kind::Decision => self.decision_key(code),
            Kind::Approval => self.approval_key(code),
            Kind::Signoff => self.signoff_key(code),
            Kind::Briefing => self.briefing_key(code),
            _ => {}
        }
    }

    /// A decision's own key: o for other. Its options are picked by their
    /// numbers, as [`Form::pick`] has them.
    fn decision_key(&mut self, code: KeyCode) {
        if code == KeyCode::Char('o') {
            self.choice = None;
            self.other = true;
            self.editing = Some(Which::Note);
        }
    }

    /// An approval's keys: move between rows, give the row a verdict, or
    /// write its note.
    fn approval_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.row = (self.row + 1).min(self.rows.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.row = self.row.saturating_sub(1);
            }
            KeyCode::Char('y') => self.give_verdict("approve"),
            KeyCode::Char('h') => self.give_verdict("hold"),
            KeyCode::Char('x') => self.give_verdict("reject"),
            KeyCode::Char('r') if !self.rows.is_empty() => {
                self.editing = Some(Which::Row(self.row));
            }
            _ => {}
        }
    }

    /// Gives the row its verdict, and moves on to the next row without
    /// one, if there is one.
    fn give_verdict(&mut self, verdict: &'static str) {
        if let Some(row) = self.rows.get_mut(self.row) {
            row.verdict = Some(verdict);
        }
        if let Some(next) = (self.row + 1..self.rows.len())
            .chain(0..self.row)
            .find(|at| self.rows[*at].verdict.is_none())
        {
            self.row = next;
        }
    }

    /// A signoff's keys: y to accept, or c to ask for changes.
    fn signoff_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => self.signoff = Some("accept"),
            KeyCode::Char('c') => {
                self.signoff = Some("changes");
                self.editing = Some(Which::Note);
            }
            _ => {}
        }
    }

    /// A briefing's keys: y to accept, s to send it back with the reason in
    /// the note, and, on its items, a number to tick or untick the
    /// left-out item it names to file, 0 for the tenth.
    fn briefing_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => self.briefing = Some("accept"),
            KeyCode::Char('s') => {
                self.briefing = Some("send_back");
                self.editing = Some(Which::Note);
            }
            KeyCode::Char(c @ '0'..='9') => {
                let n = match c.to_digit(10) {
                    Some(0) => 10,
                    Some(n) => n,
                    None => return,
                };
                if let Some(tick) = usize::try_from(n - 1)
                    .ok()
                    .and_then(|at| self.file.get_mut(at))
                {
                    *tick = !*tick;
                }
            }
            _ => {}
        }
    }

    /// Sends the form, or says what it is missing. A withdraw asks first.
    fn send(&mut self) -> Outcome {
        if let Some(missing) = self.missing() {
            return Outcome::Missing(missing);
        }
        if self.act == Act::Withdraw {
            self.editing = None;
            self.confirm = Some(Confirm::Withdraw);
            return Outcome::Stay;
        }
        Outcome::Send(Box::new(self.record()))
    }

    /// What the answer still needs, by its kind (IN-03).
    pub(crate) fn missing(&self) -> Option<&'static str> {
        if self.act != Act::Answer {
            return None;
        }
        match self.kind {
            Kind::Decision if self.other && self.note.is_blank() => Some("other needs a note"),
            Kind::Decision if !self.other && self.choice.is_none() => {
                Some("choose an option first, or o for other")
            }
            Kind::Signoff if self.signoff.is_none() => Some("y to accept, or c to ask for changes"),
            Kind::Signoff if self.signoff == Some("changes") && self.note.is_blank() => {
                Some("asking for changes needs a note")
            }
            Kind::Briefing if self.briefing.is_none() => Some("y to accept, or s to send it back"),
            Kind::Briefing if self.briefing == Some("send_back") && self.note.is_blank() => {
                Some("sending back needs a reason: write it in the note")
            }
            Kind::Approval if self.rows.is_empty() => {
                Some("the case has no rows to answer: d to drop it")
            }
            Kind::Approval if self.rows.iter().any(|row| row.verdict.is_none()) => {
                Some("a verdict for every row: y approve, h hold, x reject")
            }
            Kind::Question if self.text.is_blank() => Some("write the reply first"),
            Kind::Stuck if self.text.is_blank() => Some("write the guidance first"),
            _ => None,
        }
    }

    /// The record the form sends.
    pub(crate) fn record(&self) -> Record {
        match self.act {
            Act::Answer => self.answer(false),
            Act::Park => {
                let mut park = Park::default();
                self.note.text().trim().clone_into(&mut park.note);
                Record::Park(park)
            }
            Act::Withdraw => {
                let mut withdraw = Withdraw::default();
                self.note.text().trim().clone_into(&mut withdraw.reason);
                Record::Withdraw(withdraw)
            }
        }
    }

    /// The answer, or a drop of the case with the note. Text and note are
    /// trimmed at their ends (IN-03), and the answer carries the amend seq
    /// the form was drawn at (ST-04).
    fn answer(&self, drop: bool) -> Record {
        let mut answer = Answer::default();
        answer.amend_seq = i64::try_from(self.amend_seq).unwrap_or(i64::MAX);
        self.note.text().trim().clone_into(&mut answer.note);
        if drop {
            answer.drop = true;
            return Record::Answer(answer);
        }
        match self.kind {
            Kind::Decision => {
                answer.choice = self.choice.unwrap_or_default();
                answer.other = self.other;
            }
            Kind::Signoff => self
                .signoff
                .unwrap_or_default()
                .clone_into(&mut answer.signoff),
            Kind::Briefing => {
                self.briefing
                    .unwrap_or_default()
                    .clone_into(&mut answer.briefing);
                answer.file = (1i64..)
                    .zip(&self.file)
                    .filter(|(_, tick)| **tick)
                    .map(|(n, _)| n)
                    .collect();
            }
            Kind::Question | Kind::Stuck => {
                self.text.text().trim().clone_into(&mut answer.text);
            }
            Kind::Fyi => answer.ack = true,
            Kind::Approval => {
                answer.rows = self
                    .rows
                    .iter()
                    .map(|row| {
                        let mut verdict = RowVerdict::default();
                        verdict.id.clone_from(&row.id);
                        row.verdict
                            .unwrap_or_default()
                            .clone_into(&mut verdict.verdict);
                        row.note.text().trim().clone_into(&mut verdict.note);
                        verdict
                    })
                    .collect();
            }
            _ => {}
        }
        Record::Answer(answer)
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, ApprovalRow, Author, Open, Time};

    use super::*;

    fn case(kind: &str, with: impl FnOnce(&mut Open)) -> Case {
        let mut open = Open::default();
        kind.clone_into(&mut open.kind);
        "today".clone_into(&mut open.urgency);
        "Pin it?".clone_into(&mut open.title);
        "worker".clone_into(&mut open.worker);
        with(&mut open);
        let append = Append::new(Author::Agent, &Record::Open(open));
        Case::open(&append, Time::from_unix(1, 0).unwrap(), None)
            .unwrap()
            .0
    }

    fn form(kind: &str, act: Act) -> Form {
        let case = case(kind, |open| match kind {
            "decision" => open.options = vec!["Pin it".into(), "Float it".into()],
            "approval" => {
                open.rows = ["a", "b", "c"]
                    .iter()
                    .map(|id| {
                        let mut row = ApprovalRow::default();
                        (*id).clone_into(&mut row.id);
                        "step".clone_into(&mut row.label);
                        "true".clone_into(&mut row.script);
                        "run.sh".clone_into(&mut row.link);
                        row
                    })
                    .collect();
            }
            _ => {}
        });
        Form::new(Key::new(1).unwrap(), &case, act)
    }

    fn press(form: &mut Form, code: KeyCode) -> Outcome {
        form.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(form: &mut Form, c: char) -> Outcome {
        form.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn typing(form: &mut Form, text: &str) {
        for c in text.chars() {
            assert_eq!(press(form, KeyCode::Char(c)), Outcome::Stay);
        }
    }

    fn answer(with: impl FnOnce(&mut Answer)) -> Outcome {
        let mut answer = Answer::default();
        with(&mut answer);
        Outcome::Send(Box::new(Record::Answer(answer)))
    }

    /// Tab and shift-tab go round the panel's parts, by its kind: into a
    /// field to type in it, onto a button to press it with enter.
    #[test]
    fn tab_goes_round_the_panels_parts() {
        let mut decision = form("decision", Act::Answer);
        assert_eq!(
            decision.parts(),
            [
                Spot::Choices,
                Spot::Field(Which::Note),
                Spot::Button(Press::Send),
                Spot::Button(Press::Drop)
            ]
        );
        assert_eq!(decision.spot(), Spot::Choices);
        // On the choices, j and k move the choice, other after the options.
        press(&mut decision, KeyCode::Char('j'));
        assert_eq!(decision.choice, Some(1));
        press(&mut decision, KeyCode::Down);
        press(&mut decision, KeyCode::Down);
        assert!(decision.other && decision.choice.is_none());
        press(&mut decision, KeyCode::Up);
        assert_eq!((decision.choice, decision.other), (Some(2), false));
        press(&mut decision, KeyCode::Tab);
        assert_eq!(decision.editing, Some(Which::Note));
        typing(&mut decision, "x");
        press(&mut decision, KeyCode::Tab);
        assert_eq!(
            (decision.spot(), decision.editing),
            (Spot::Button(Press::Send), None)
        );
        press(&mut decision, KeyCode::Tab);
        assert_eq!(press(&mut decision, KeyCode::Enter), Outcome::Stay);
        assert_eq!(decision.confirm, Some(Confirm::Drop));
        decision.confirm = None;
        // With something chosen or typed, discard is the last button.
        press(&mut decision, KeyCode::Tab);
        assert_eq!(decision.spot(), Spot::Button(Press::Discard));
        assert_eq!(press(&mut decision, KeyCode::Enter), Outcome::Stay);
        assert_eq!(decision.confirm, Some(Confirm::Discard));
        assert_eq!(press(&mut decision, KeyCode::Char('y')), Outcome::Cancel);
        // Past the last part, the keys leave the panel, for the list.
        assert_eq!(press(&mut decision, KeyCode::Tab), Outcome::Leave);
        assert_eq!(decision.spot(), Spot::Button(Press::Discard));
        press(&mut decision, KeyCode::BackTab);
        press(&mut decision, KeyCode::BackTab);
        assert_eq!(
            press(&mut decision, KeyCode::Char(' ')),
            answer(|a| {
                a.choice = 2;
                "x".clone_into(&mut a.note);
            })
        );
        // Before the first part, the keys leave it too, out of a field.
        let mut decision = form("decision", Act::Answer);
        assert_eq!(press(&mut decision, KeyCode::BackTab), Outcome::Leave);
        let mut question = form("question", Act::Answer);
        assert_eq!(question.editing, Some(Which::Text));
        assert_eq!(press(&mut question, KeyCode::BackTab), Outcome::Leave);
        assert_eq!(question.editing, None);
        // A stuck case parks from its panel; an fyi has only its note.
        let stuck = form("stuck", Act::Answer);
        assert_eq!(stuck.presses(), [Press::Send, Press::Park, Press::Drop]);
        assert_eq!(stuck.parts()[0], Spot::Field(Which::Text));
        let mut fyi = form("fyi", Act::Answer);
        assert_eq!(fyi.parts()[0], Spot::Field(Which::Note));
        assert_eq!(fyi.press(Press::Park), Outcome::Park);
        let approval = form("approval", Act::Answer);
        assert_eq!(approval.parts()[0], Spot::Rows);
    }

    #[test]
    fn a_reply_is_typed_in_place_until_esc_or_tab_moves_on() {
        let missing = [
            ("question", "write the reply first"),
            ("stuck", "write the guidance first"),
        ];
        for (kind, missing) in missing {
            let mut form = form(kind, Act::Answer);
            assert_eq!(form.editing, Some(Which::Text));
            press(&mut form, KeyCode::Char(' '));
            assert_eq!(ctrl(&mut form, 's'), Outcome::Missing(missing));
            // While typing, the form's keys are the field's.
            typing(&mut form, "nd ");
            assert_eq!(form.text.text(), " nd ");
            assert_eq!(form.confirm, None);
            assert_eq!(ctrl(&mut form, 'e'), Outcome::Edit(Which::Text));
            // Esc leaves the field; tab goes on to the next part, the
            // note, and shift-tab back.
            assert_eq!(press(&mut form, KeyCode::Esc), Outcome::Stay);
            assert_eq!(form.editing, None);
            assert_eq!(form.spot(), Spot::Field(Which::Text));
            // Out of the field, Ctrl-E still opens the reply.
            assert_eq!(ctrl(&mut form, 'e'), Outcome::Edit(Which::Text));
            press(&mut form, KeyCode::Char('t'));
            assert_eq!(form.editing, Some(Which::Text), "{kind}");
            press(&mut form, KeyCode::Tab);
            assert_eq!(form.editing, Some(Which::Note));
            press(&mut form, KeyCode::BackTab);
            assert_eq!(form.editing, Some(Which::Text));
            press(&mut form, KeyCode::Esc);
            press(&mut form, KeyCode::Char('i'));
            assert_eq!(form.editing, Some(Which::Text));
            press(&mut form, KeyCode::Esc);
            press(&mut form, KeyCode::Char('n'));
            assert_eq!(form.editing, Some(Which::Note));
            assert_eq!(ctrl(&mut form, 'e'), Outcome::Edit(Which::Note));
            typing(&mut form, "a note");
            assert_eq!(form.note.text(), "a note");
            assert_eq!(
                ctrl(&mut form, 's'),
                answer(|a| {
                    "nd".clone_into(&mut a.text);
                    "a note".clone_into(&mut a.note);
                })
            );
        }
    }

    #[test]
    fn keys_that_are_not_the_forms_do_nothing() {
        // (kind, act, key): none of them changes the form.
        let cases: &[(&str, Act, char)] = &[
            ("decision", Act::Answer, 't'),
            ("decision", Act::Answer, 'i'),
            ("decision", Act::Answer, 'y'),
            ("decision", Act::Answer, '0'),
            ("decision", Act::Answer, '4'),
            ("fyi", Act::Answer, '1'),
            ("fyi", Act::Answer, 'o'),
            ("fyi", Act::Answer, 'y'),
            ("fyi", Act::Answer, 'c'),
            ("fyi", Act::Answer, 'r'),
            ("fyi", Act::Answer, 'j'),
            ("signoff", Act::Answer, 'x'),
            ("stuck", Act::Park, 'd'),
            ("stuck", Act::Park, 't'),
        ];
        for &(kind, act, c) in cases {
            let mut form = form(kind, act);
            form.editing = None;
            // Only a decision takes a number, whatever the count of options.
            form.options = 2;
            assert_eq!(press(&mut form, KeyCode::Char(c)), Outcome::Stay);
            assert_eq!(
                (form.editing, form.confirm, form.choice, form.other),
                (None, None, None, false),
                "{c} on a {kind} {act:?}"
            );
            assert_eq!((form.signoff, form.row), (None, 0), "{c} on a {kind}");
            assert!(form.rows.iter().all(|row| row.verdict.is_none()));
        }
    }

    #[test]
    fn a_decision_takes_an_option_in_range_or_other_with_a_note() {
        let mut form = form("decision", Act::Answer);
        assert_eq!(form.editing, None);
        assert_eq!(ctrl(&mut form, 'e'), Outcome::Edit(Which::Note));
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("choose an option first, or o for other")
        );
        press(&mut form, KeyCode::Char('4'));
        assert_eq!((form.choice, form.other), (None, false));
        // Other is the number after the last option: its note is where it
        // is said.
        press(&mut form, KeyCode::Char('3'));
        assert_eq!((form.choice, form.other), (None, true));
        assert_eq!(form.editing, Some(Which::Note));
        press(&mut form, KeyCode::Esc);
        press(&mut form, KeyCode::Char('2'));
        assert_eq!((form.choice, form.other), (Some(2), false));
        press(&mut form, KeyCode::Char('o'));
        assert_eq!((form.choice, form.other), (None, true));
        assert_eq!(form.editing, Some(Which::Note));
        press(&mut form, KeyCode::Esc);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("other needs a note")
        );
        press(&mut form, KeyCode::Char('n'));
        typing(&mut form, " Sink it ");
        // Enter in the note sends; alt- or shift-enter is a new line.
        for modifiers in [KeyModifiers::ALT, KeyModifiers::SHIFT] {
            assert_eq!(
                form.key(KeyEvent::new(KeyCode::Enter, modifiers)),
                Outcome::Stay
            );
            assert_eq!(form.note.text(), " Sink it \n");
            press(&mut form, KeyCode::Backspace);
        }
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            answer(|a| {
                a.other = true;
                "Sink it".clone_into(&mut a.note);
            })
        );
        press(&mut form, KeyCode::Tab);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            answer(|a| {
                a.other = true;
                "Sink it".clone_into(&mut a.note);
            })
        );
        press(&mut form, KeyCode::Char('1'));
        assert_eq!((form.choice, form.other), (Some(1), false));
        assert_eq!(
            ctrl(&mut form, 's'),
            answer(|a| {
                a.choice = 1;
                "Sink it".clone_into(&mut a.note);
            })
        );
    }

    #[test]
    fn an_approval_moves_between_rows_and_goes_on_to_the_next_without_a_verdict() {
        let mut form = form("approval", Act::Answer);
        // Esc leaves the answer's panel, keeping what is in it.
        assert_eq!(press(&mut form, KeyCode::Esc), Outcome::Leave);
        let verdicts = |form: &Form| form.rows.iter().map(|r| r.verdict).collect::<Vec<_>>();
        for code in [KeyCode::Char('j'), KeyCode::Down, KeyCode::Char('j')] {
            press(&mut form, code);
        }
        assert_eq!(form.row, 2);
        press(&mut form, KeyCode::Char('k'));
        assert_eq!(form.row, 1);
        for code in [KeyCode::Up, KeyCode::Char('k')] {
            press(&mut form, code);
        }
        assert_eq!(form.row, 0);
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Down);
        // From the last row, the next without a verdict is the first.
        press(&mut form, KeyCode::Char('x'));
        assert_eq!(verdicts(&form), [None, None, Some("reject")]);
        assert_eq!(form.row, 0);
        press(&mut form, KeyCode::Char('h'));
        assert_eq!(form.row, 1);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("a verdict for every row: y approve, h hold, x reject")
        );
        press(&mut form, KeyCode::Char('r'));
        assert_eq!(form.editing, Some(Which::Row(1)));
        assert_eq!(ctrl(&mut form, 'e'), Outcome::Edit(Which::Row(1)));
        typing(&mut form, "later");
        press(&mut form, KeyCode::Esc);
        press(&mut form, KeyCode::Char('y'));
        assert_eq!(
            verdicts(&form),
            [Some("hold"), Some("approve"), Some("reject")]
        );
        // Every row has one: the keys stay on the row.
        assert_eq!(form.row, 1);
        press(&mut form, KeyCode::Char('x'));
        assert_eq!(form.row, 1);
        let Outcome::Send(record) = press(&mut form, KeyCode::Enter) else {
            panic!("an approval with every verdict sends");
        };
        let Record::Answer(answer) = *record else {
            panic!("an answer");
        };
        let rows: Vec<_> = answer
            .rows
            .iter()
            .map(|r| (r.id.as_str(), r.verdict.as_str(), r.note.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("a", "hold", ""),
                ("b", "reject", "later"),
                ("c", "reject", "")
            ]
        );

        // A note on a row alone is typed: Esc leaves the field, then the
        // panel, which keeps it.
        let mut form = self::form("approval", Act::Answer);
        press(&mut form, KeyCode::Char('r'));
        typing(&mut form, "why");
        press(&mut form, KeyCode::Esc);
        assert_eq!(press(&mut form, KeyCode::Esc), Outcome::Leave);
        assert!(form.touched());
        // A row past the end types into the last row's note, not the
        // case's.
        form.editing = Some(Which::Row(3));
        typing(&mut form, "x");
        assert_eq!((form.note.text(), form.rows[2].note.text()), ("", "x"));

        // With no rows, the keys have nothing to act on.
        let mut form = self::form("approval", Act::Answer);
        form.rows.clear();
        for c in ['j', 'k', 'y', 'r'] {
            assert_eq!(press(&mut form, KeyCode::Char(c)), Outcome::Stay);
            assert_eq!((form.row, form.editing), (0, None));
        }
    }

    #[test]
    fn a_signoff_accepts_or_asks_for_changes_with_a_note() {
        let mut form = form("signoff", Act::Answer);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("y to accept, or c to ask for changes")
        );
        press(&mut form, KeyCode::Char('c'));
        assert_eq!(form.signoff, Some("changes"));
        assert_eq!(form.editing, Some(Which::Note));
        press(&mut form, KeyCode::Esc);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("asking for changes needs a note")
        );
        press(&mut form, KeyCode::Char('y'));
        assert_eq!(form.signoff, Some("accept"));
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            answer(|a| "accept".clone_into(&mut a.signoff))
        );
    }

    #[test]
    fn a_briefing_files_its_tenth_item_by_zero() {
        let case = case("briefing", |open| {
            "The work.".clone_into(&mut open.body);
            open.left_out = (1..=10).map(|n| format!("item {n}")).collect();
        });
        let mut form = Form::new(Key::new(1).unwrap(), &case, Act::Answer);
        assert_eq!(form.editing, None);
        // On the answers, a number answers; on the items, it ticks one.
        press(&mut form, KeyCode::Char('2'));
        assert_eq!(form.briefing, Some("send_back"));
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.spot(), Spot::Items);
        press(&mut form, KeyCode::Char('0'));
        press(&mut form, KeyCode::Char('9'));
        press(&mut form, KeyCode::Char('y'));
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            answer(|a| {
                "accept".clone_into(&mut a.briefing);
                a.file = vec![9, 10];
            })
        );
        // A drop files nothing, whatever is ticked.
        press(&mut form, KeyCode::Char('d'));
        assert_eq!(
            press(&mut form, KeyCode::Char('y')),
            answer(|a| a.drop = true)
        );
    }

    #[test]
    fn a_drop_a_withdraw_and_a_discard_ask_first() {
        // Only y or Y goes on; any other key takes the question away.
        let mut form = form("fyi", Act::Answer);
        assert_eq!(press(&mut form, KeyCode::Char('d')), Outcome::Stay);
        assert_eq!(form.confirm, Some(Confirm::Drop));
        assert_eq!(press(&mut form, KeyCode::Char('n')), Outcome::Stay);
        assert_eq!((form.confirm, form.editing), (None, None));
        press(&mut form, KeyCode::Char('d'));
        assert_eq!(ctrl(&mut form, 's'), Outcome::Stay);
        assert_eq!(form.confirm, None);
        press(&mut form, KeyCode::Char('n'));
        typing(&mut form, "no longer needed");
        press(&mut form, KeyCode::Esc);
        press(&mut form, KeyCode::Char('d'));
        assert_eq!(
            press(&mut form, KeyCode::Char('Y')),
            answer(|a| {
                a.drop = true;
                "no longer needed".clone_into(&mut a.note);
            })
        );
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            answer(|a| {
                a.ack = true;
                "no longer needed".clone_into(&mut a.note);
            })
        );

        let mut form = self::form("decision", Act::Withdraw);
        assert_eq!(form.editing, Some(Which::Note));
        typing(&mut form, "wrong case ");
        assert_eq!(ctrl(&mut form, 's'), Outcome::Stay);
        assert_eq!(
            (form.confirm, form.editing),
            (Some(Confirm::Withdraw), None)
        );
        let mut withdraw = Withdraw::default();
        "wrong case".clone_into(&mut withdraw.reason);
        assert_eq!(
            press(&mut form, KeyCode::Char('y')),
            Outcome::Send(Box::new(Record::Withdraw(withdraw)))
        );

        let mut form = self::form("stuck", Act::Park);
        typing(&mut form, "after lunch");
        press(&mut form, KeyCode::Esc);
        let mut park = Park::default();
        "after lunch".clone_into(&mut park.note);
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Send(Box::new(Record::Park(park)))
        );
        assert_eq!(press(&mut form, KeyCode::Esc), Outcome::Stay);
        assert_eq!(form.confirm, Some(Confirm::Discard));
        assert_eq!(press(&mut form, KeyCode::Char('y')), Outcome::Cancel);

        // Nothing typed: Esc closes the form without asking.
        let mut form = self::form("stuck", Act::Park);
        press(&mut form, KeyCode::Esc);
        assert_eq!(press(&mut form, KeyCode::Esc), Outcome::Cancel);
        assert_eq!(form.confirm, None);
    }

    #[test]
    fn a_park_or_a_withdraw_takes_no_keys_of_the_case_kind() {
        for act in [Act::Park, Act::Withdraw] {
            for kind in ["decision", "approval", "signoff"] {
                let mut form = form(kind, act);
                press(&mut form, KeyCode::Tab);
                for c in ['1', 'o', 'j', 'y', 'h', 'x', 'r', 'c'] {
                    assert_eq!(press(&mut form, KeyCode::Char(c)), Outcome::Stay);
                }
                assert_eq!(
                    (
                        form.choice,
                        form.other,
                        form.signoff,
                        form.row,
                        form.editing
                    ),
                    (None, false, None, 0, None),
                    "{kind} {act:?}"
                );
                assert!(form.rows.iter().all(|row| row.verdict.is_none()));
            }
        }
    }

    #[test]
    fn a_row_note_with_no_rows_is_the_case_note() {
        let mut form = form("approval", Act::Answer);
        form.rows.clear();
        form.field(Which::Row(0)).set("x".into());
        assert_eq!(form.note.text(), "x");
    }

    #[test]
    fn an_approval_with_no_rows_is_not_sent() {
        // Core refuses an approval answer with no rows: it has no response.
        let mut form = form("approval", Act::Answer);
        form.rows.clear();
        assert_eq!(
            press(&mut form, KeyCode::Enter),
            Outcome::Missing("the case has no rows to answer: d to drop it")
        );
        press(&mut form, KeyCode::Char('d'));
        assert_eq!(
            press(&mut form, KeyCode::Char('y')),
            answer(|a| a.drop = true)
        );
    }

    #[test]
    fn the_field_being_edited_is_the_one_typing_goes_to() {
        let mut form = form("approval", Act::Answer);
        assert_eq!(form.being_edited(), None);
        form.editing = Some(Which::Row(1));
        assert_eq!(form.being_edited(), Some(Which::Row(1)));
        // A row past the end types into the last row.
        form.editing = Some(Which::Row(3));
        assert_eq!(form.being_edited(), Some(Which::Row(2)));
        // With no rows, into the case's note.
        form.rows.clear();
        assert_eq!(form.being_edited(), Some(Which::Note));
        form.editing = Some(Which::Text);
        assert_eq!(form.being_edited(), Some(Which::Text));
    }
}
