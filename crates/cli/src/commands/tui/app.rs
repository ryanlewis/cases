//! What the screen shows, and what each key does to it. Nothing here reads
//! the store or draws: the loop gives it the cases, and the view draws it.

use std::cmp::Reverse;
use std::time::{Duration, Instant};

use cases_core::{Case, Kind, Listening, Record, Resume, State, Stats, Time, Urgency};
use cases_store_sqlite::Key;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use super::compose::{self, Compose, Part};
use super::form::{Act, Form, Outcome, Press, Which};
use super::reader::Wrote;
use crate::commands::list;
use crate::output::Actors;

/// What [`App::latest_first`] keeps: the cases `filter` takes that are in
/// one of `states`.
fn filter_of(filter: Filter, states: &[State]) -> impl Fn(&Case) -> bool + '_ {
    move |case| filter.takes(case) && states.contains(&case.state())
}

/// How long a toast stays.
const TOAST_FOR: Duration = Duration::from_secs(4);

/// The most toasts shown at once: a new one pushes out the oldest.
const TOASTS: usize = 3;

/// The list's filters, as the web inbox has them (IN-01, IN-07), and the
/// briefings' tab beside them (TU-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filter {
    Inbox,
    Flight,
    Done,
    All,
    Briefings,
}

impl Filter {
    pub(crate) const ALL: [Filter; 5] = [
        Filter::Inbox,
        Filter::Flight,
        Filter::Done,
        Filter::All,
        Filter::Briefings,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Filter::Inbox => "Inbox",
            Filter::Flight => "In flight",
            Filter::Done => "Done",
            Filter::All => "All",
            Filter::Briefings => "Briefings",
        }
    }

    /// Whether the filter lists `case`. A briefing is in none of the
    /// filters until it is closed or withdrawn, and the briefings' tab has
    /// it until then: open, answered or picked up (IN-14, TU-10).
    fn takes(self, case: &Case) -> bool {
        let briefing = case.kind() == Kind::Briefing;
        match self {
            Filter::Inbox => !briefing && list::WAITING.contains(&case.state()),
            Filter::Flight => {
                !briefing
                    && matches!(
                        case.state(),
                        State::Sent | State::Answered | State::PickedUp
                    )
            }
            Filter::Done => matches!(case.state(), State::Closed | State::Withdrawn),
            Filter::All => true,
            Filter::Briefings => {
                briefing
                    && matches!(
                        case.state(),
                        State::Open | State::Answered | State::PickedUp
                    )
            }
        }
    }

    fn step(self, by: isize) -> Filter {
        let at = Filter::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let len = Filter::ALL.len().cast_signed();
        Filter::ALL[usize::try_from((at.cast_signed() + by).rem_euclid(len)).unwrap_or(0)]
    }
}

/// Cases as a list shows them, in its order.
pub(crate) type Listed<'a> = Vec<&'a (Key, Case)>;

/// Whether a group of the list is folded away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Folded,
    Shown,
}

/// What a click on a part of the screen does (TU-12). The view says where
/// each part was drawn, in [`App::hits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    /// The list: the wheel moves the selection.
    List,
    /// The case pane: the wheel scrolls it.
    Case,
    /// A filter's tab.
    Tab(Filter),
    /// A case in the list.
    Row(Key),
    /// The briefings' answered group's heading: folds it.
    Fold,
    /// A key shown in the footer: the click presses it.
    Key(KeyEvent),
    /// A decision's option, by its number.
    Choice(i64),
    /// A briefing's left-out item, by its place.
    LeftOut(usize),
    /// An approval row in the form, by its place.
    ApprovalRow(usize),
    /// A field of the form.
    Field(Which),
    /// A signoff's or a briefing's answer, in the answer's panel.
    Mark(&'static str),
    /// An approval row's verdict, by the row's place, in the answer's panel.
    Verdict(usize, &'static str),
    /// A line of an approval row's verdicts, by the row's place, and
    /// whether the human carries the step out: the view splits it into a
    /// [`Hit::Verdict`] for each, so it is never where a click lands.
    Verdicts(usize, bool),
    /// One of the answer's panel's buttons.
    Press(Press),
    /// One of the case's bar's buttons.
    Verb(Verb),
    /// A dialog, behind its buttons: a click on it does nothing.
    Dialog,
    /// A toast, by its id: a click puts it away.
    Toast(u64),
}

/// A short-lived notice in a corner of the screen: what a write did, or
/// why a key did nothing.
#[derive(Debug, Clone)]
pub(crate) struct Toast {
    /// Which toast it is, while it is up: a click names it by this.
    pub(crate) id: u64,
    pub(crate) text: String,
    /// Whether it says something was done, rather than not.
    pub(crate) done: bool,
    at: Instant,
}

/// Whether one case's screen (TU-11) takes a key that reaches
/// [`App::pane_key`]: its answer, a park, help, and the keys that scroll
/// it by the page. j, k, the arrows, q and a form's keys are taken before
/// that. The list's, the filters' and messages' keys do nothing there,
/// and nor does a withdraw or a resume, as the screen ends once the case
/// leaves open. The help it shows is [`super::view::ONE_KEYS`].
fn one_case_takes(code: KeyCode, ctrl: bool) -> bool {
    match code {
        KeyCode::Char('d' | 'u') => ctrl,
        KeyCode::Char('a' | 'p' | '?' | ' ' | 'J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
            true
        }
        _ => false,
    }
}

/// Whether r resumes `case`: only a parked one.
fn resumes(case: &Case) -> bool {
    case.state() == State::Parked
}

/// Whether e edits `case`: only a message no agent has picked up (HU-08).
fn edits(case: &Case) -> bool {
    case.kind() == Kind::Message && case.state() == State::Sent
}

/// An act on the selected case that the case's bar offers as a button,
/// each pressing the key that does it (TU-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verb {
    Answer,
    Park,
    Resume,
    Withdraw,
    Edit,
}

impl Verb {
    pub(crate) const ALL: [Verb; 5] = [
        Verb::Answer,
        Verb::Park,
        Verb::Resume,
        Verb::Withdraw,
        Verb::Edit,
    ];

    /// The key that does it, which its button underlines.
    pub(crate) fn key(self) -> char {
        match self {
            Verb::Answer => 'a',
            Verb::Park => 'p',
            Verb::Resume => 'r',
            Verb::Withdraw => 'w',
            Verb::Edit => 'e',
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Verb::Answer => "Answer",
            Verb::Park => "Park",
            Verb::Resume => "Resume",
            Verb::Withdraw => "Withdraw",
            Verb::Edit => "Edit",
        }
    }
}

/// How far a turn of the wheel scrolls the case.
const WHEEL: u16 = 3;

/// Which pane has the keys. Side by side both show; on a narrow terminal
/// only the one with the keys does (TU-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    List,
    Case,
}

/// The screen's state.
#[derive(Debug)]
pub(crate) struct App {
    /// The store, for the header.
    pub(crate) store: String,
    /// Every case that folds, in open order.
    pub(crate) cases: Vec<(Key, Case)>,
    /// How many cases did not fold.
    pub(crate) unfolded: usize,
    pub(crate) actors: Actors,
    pub(crate) now: Time,
    /// Why the last read of the store failed, while it does.
    pub(crate) problem: Option<String>,
    pub(crate) filter: Filter,
    /// The selected case, kept by key across reads.
    pub(crate) selected: Option<Key>,
    /// Where the selected case stood in the list, for when it leaves it.
    selected_at: usize,
    pub(crate) focus: Focus,
    /// How far the case pane is scrolled; the view keeps it in range.
    pub(crate) scroll: u16,
    /// The list's scroll, kept between frames.
    pub(crate) list_offset: usize,
    /// The toasts shown, oldest first.
    pub(crate) toasts: Vec<Toast>,
    /// The id the next toast takes.
    next_toast: u64,
    pub(crate) help: bool,
    pub(crate) quit: bool,
    /// The form that has the keys, for the selected case (TU-05): an
    /// answer's, in the panel at the foot of the case, or a park's or a
    /// withdraw's.
    pub(crate) form: Option<Form>,
    /// What was chosen or typed in the answer's panels of other cases, or
    /// of this one with the keys out of it: kept with each case.
    pub(crate) drafts: Vec<Form>,
    /// A write for the loop to send.
    pub(crate) pending: Option<Send>,
    /// The write being sent, until the read after it lands (TU-04).
    pub(crate) sending: Option<Sending>,
    /// A field for the loop to open in the editor (TU-06).
    pub(crate) edit: Option<Which>,
    /// The message being written (TU-07).
    pub(crate) compose: Option<Compose>,
    /// A new message for the loop to send.
    pub(crate) pending_message: Option<Record>,
    /// The message's field for the loop to open in the editor.
    pub(crate) edit_message: Option<Part>,
    /// The sessions listening now (ST-16), for the message picker and inbox
    /// zero.
    pub(crate) listeners: Vec<Listening>,
    /// Quitting would lose what was typed: asking first (TU-08).
    pub(crate) confirm_quit: bool,
    /// Whether the briefings' tab folds its answered briefings away
    /// (IN-14).
    pub(crate) answered: Group,
    /// The one case shown, when `cases tui ID` opened it (TU-11): no list,
    /// no filters and no message, and the loop ends once it leaves open.
    pub(crate) only: Option<One>,
    /// Where the last frame drew each part a click acts on, latest on top.
    pub(crate) hits: Vec<(Rect, Hit)>,
    /// Where the pointer was last seen, when the terminal reports it.
    pub(crate) pointer: Option<Position>,
    /// What a click under the pointer would act on, which the view lights.
    pub(crate) hover: Option<Hit>,
    /// The stats strip (TU-13).
    pub(crate) strip: Strip,
}

/// The stats strip's state (TU-13).
#[derive(Debug)]
pub(crate) struct Strip {
    /// The stats of the last read, none before the first.
    pub(crate) stats: Option<Stats>,
    /// Whether it is shown: from the config file's `tui-stats`, then `s`.
    pub(crate) shown: bool,
    /// Whether the last frame had the rows for it: `s` does nothing
    /// without them.
    pub(crate) room: bool,
    /// Whether it is drawn as a kitty image: `auto`, on a terminal that
    /// draws them.
    pub(crate) image: bool,
    /// Where the last frame left the cells for the image, when it is one
    /// and is shown: none while help is up, which it would cover.
    pub(crate) area: Option<Rect>,
}

/// The one case `cases tui ID` shows (TU-11).
#[derive(Debug, Clone, Copy)]
pub(crate) struct One {
    key: Key,
    /// It was not in the last read that worked, with every case folded:
    /// it is gone from the store.
    gone: bool,
}

impl App {
    pub(crate) fn new(store: String) -> App {
        App {
            store,
            cases: Vec::new(),
            unfolded: 0,
            actors: Actors::default(),
            now: Time::from_unix(0, 0).unwrap_or_else(|| unreachable!("the epoch is a time")),
            problem: None,
            filter: Filter::Inbox,
            selected: None,
            selected_at: 0,
            focus: Focus::List,
            scroll: 0,
            list_offset: 0,
            toasts: Vec::new(),
            next_toast: 0,
            help: false,
            quit: false,
            form: None,
            drafts: Vec::new(),
            pending: None,
            sending: None,
            edit: None,
            compose: None,
            pending_message: None,
            edit_message: None,
            listeners: Vec::new(),
            confirm_quit: false,
            answered: Group::Folded,
            only: None,
            hits: Vec::new(),
            pointer: None,
            hover: None,
            strip: Strip {
                stats: None,
                shown: true,
                room: true,
                image: false,
                area: None,
            },
        }
    }

    /// The screen for the one case `key` (TU-11), with the case pane
    /// showing it.
    pub(crate) fn one(store: String, key: Key) -> App {
        App {
            selected: Some(key),
            focus: Focus::Case,
            only: Some(One { key, gone: false }),
            ..App::new(store)
        }
    }

    /// Whether the one case shown has left open, or the store: answered,
    /// parked or withdrawn, here or anywhere else (TU-11). Not before a
    /// read of the store has worked.
    pub(crate) fn left(&self) -> bool {
        self.only.is_some_and(|one| {
            one.gone
                || self
                    .cases
                    .iter()
                    .any(|(k, case)| *k == one.key && case.state() != State::Open)
        })
    }

    /// Takes a new read of the store.
    pub(crate) fn update(
        &mut self,
        cases: Vec<(Key, Case)>,
        unfolded: usize,
        actors: Actors,
        now: Time,
    ) {
        self.cases = cases;
        self.unfolded = unfolded;
        // A case that did not fold may be the one case: not gone.
        if let Some(one) = &mut self.only {
            one.gone = unfolded == 0 && !self.cases.iter().any(|(k, _)| *k == one.key);
        }
        self.actors = actors;
        self.now = now;
        self.problem = None;
        self.keep_selection();
        self.follow_form();
        self.follow_compose();
    }

    /// Closes the edit of a message an agent has picked up since, or that
    /// was withdrawn: only a sent message is edited (HU-08). Otherwise the
    /// edit follows the message as [`Compose::follow`] says.
    fn follow_compose(&mut self) {
        let Some((key, _)) = self.compose.as_ref().and_then(|c| c.editing) else {
            return;
        };
        let case = self
            .cases
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, case)| case);
        let state = case.map(Case::state);
        if let (Some(case), Some(compose)) = (case, &mut self.compose)
            && state == Some(State::Sent)
        {
            compose.follow(case);
        } else {
            self.compose = None;
            let now = state.map_or("gone".to_string(), |s| format!("now {s}"));
            self.flash(&format!("{key} is {now}: the edit was not sent"));
        }
    }

    /// Keeps the form in step with its case (IN-06): closed when the case
    /// has moved to a state the form cannot write to, and otherwise
    /// following it as [`Form::follow`] says.
    fn follow_form(&mut self) {
        // The write being sent moves its case on: how it went is said once
        // it lands, by `sent`, not as a case that moved with nothing sent.
        let sending = match &self.sending {
            Some(Sending::Event(send)) => Some(send.key),
            _ => None,
        };
        // A draft follows its case quietly, and goes once its case can no
        // longer be answered, saying so as the form does.
        let cases = &self.cases;
        let mut gone = Vec::new();
        self.drafts.retain_mut(
            |draft| match cases.iter().find(|(key, _)| *key == draft.key) {
                Some((_, case)) if draft.act.fits(case) => {
                    draft.follow(case);
                    true
                }
                _ if Some(draft.key) == sending => true,
                Some((_, case)) => {
                    gone.push(format!(
                        "{} is now {}: nothing was sent",
                        draft.key,
                        case.state()
                    ));
                    false
                }
                None => {
                    gone.push(format!("{} is gone: nothing was sent", draft.key));
                    false
                }
            },
        );
        for line in gone {
            self.flash(&line);
        }
        let Some(form) = &self.form else {
            return;
        };
        let case = self
            .cases
            .iter()
            .find(|(key, _)| *key == form.key)
            .map(|(_, case)| case);
        match case {
            Some(case) if form.act.fits(case) => {
                if let Some(form) = &mut self.form {
                    form.follow(case);
                }
            }
            _ if Some(form.key) == sending => {}
            Some(case) => {
                let line = format!("{} is now {}: nothing was sent", form.key, case.state());
                self.form = None;
                self.flash(&line);
            }
            None => {
                let line = format!("{} is gone: nothing was sent", form.key);
                self.form = None;
                self.flash(&line);
            }
        }
    }

    /// Records a read that failed: what was read before stays on screen.
    pub(crate) fn unreadable(&mut self, why: &str, now: Time) {
        self.problem = Some(why.into());
        self.now = now;
    }

    /// The cases under `filter`, in its order (IN-01, IN-07): the inbox by
    /// urgency then open order; in flight the messages sent, newest first,
    /// then the cases with an agent, latest first; done newest first by its
    /// last event; all of them, in that order.
    ///
    /// The briefings' tab lists the open briefings, then the answered ones
    /// unless they are folded away (IN-14).
    pub(crate) fn shown(&self, filter: Filter) -> Vec<&(Key, Case)> {
        if let Some(one) = self.only {
            return self.cases.iter().filter(|(k, _)| *k == one.key).collect();
        }
        let flight = || {
            let mut cases = self.latest_first(filter_of(Filter::Flight, &[State::Sent]));
            cases.extend(self.latest_first(filter_of(
                Filter::Flight,
                &[State::Answered, State::PickedUp],
            )));
            cases
        };
        let done =
            || self.latest_first(filter_of(Filter::Done, &[State::Closed, State::Withdrawn]));
        match filter {
            Filter::Inbox => self.inbox(),
            Filter::Flight => flight(),
            Filter::Done => done(),
            Filter::All => {
                let mut cases = self.inbox();
                cases.extend(flight());
                cases.extend(done());
                cases
            }
            Filter::Briefings => {
                let (mut open, answered) = self.briefings();
                if self.answered == Group::Shown {
                    open.extend(answered);
                }
                open
            }
        }
    }

    fn inbox(&self) -> Vec<&(Key, Case)> {
        list::inbox_order(&self.cases, |case| Filter::Inbox.takes(case))
    }

    /// The cases `keep` takes, latest first.
    fn latest_first(&self, keep: impl Fn(&Case) -> bool) -> Vec<&(Key, Case)> {
        let mut cases: Vec<_> = self.cases.iter().filter(|(_, case)| keep(case)).collect();
        cases.sort_by_key(|(key, case)| Reverse((case.updated_at(), *key)));
        cases
    }

    /// The open briefings, gated ones before landed ones, then by urgency
    /// and open order; and the answered ones, picked up or not, so that an
    /// answer an agent has picked up can still be withdrawn until the
    /// briefing closes, newest answer first, as the web inbox orders them
    /// (IN-14): by the answer, so that a pickup does not move one.
    pub(crate) fn briefings(&self) -> (Listed<'_>, Listed<'_>) {
        let mut open = list::inbox_order(&self.cases, |case| {
            case.kind() == Kind::Briefing && case.state() == State::Open
        });
        // Stable, so urgency and open order hold within each.
        open.sort_by_key(|(_, case)| case.landed());
        let mut answered: Listed<'_> = self
            .cases
            .iter()
            .filter(|(_, case)| {
                case.kind() == Kind::Briefing
                    && matches!(case.state(), State::Answered | State::PickedUp)
            })
            .collect();
        answered.sort_by_key(|(key, case)| {
            Reverse((case.answer().map(|answer| answer.answered_at), *key))
        });
        (open, answered)
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        let key = self.selected?;
        self.shown(self.filter).iter().position(|(k, _)| *k == key)
    }

    pub(crate) fn selected_case(&self) -> Option<&(Key, Case)> {
        let key = self.selected?;
        self.cases.iter().find(|(k, _)| *k == key)
    }

    /// Keeps the selected case while the filter shows it; when it has left,
    /// selects the case that took its place (TU-04).
    fn keep_selection(&mut self) {
        match self.selected_index() {
            Some(at) => self.selected_at = at,
            None => self.select(self.selected_at),
        }
    }

    /// Selects the case at `index` in the list, or the last one.
    fn select(&mut self, index: usize) {
        let shown = self.shown(self.filter);
        let at = index.min(shown.len().saturating_sub(1));
        let key = shown.get(at).map(|(key, _)| *key);
        if self.selected != key {
            self.scroll = 0;
        }
        self.selected = key;
        self.selected_at = at;
    }

    fn move_by(&mut self, by: isize) {
        let at = self.selected_index().unwrap_or(0);
        self.select(at.saturating_add_signed(by));
    }

    fn set_filter(&mut self, filter: Filter) {
        if filter != self.filter {
            self.filter = filter;
            self.list_offset = 0;
            self.select(0);
        }
    }

    /// How many cases wait on the human, and how many of those block: the
    /// open ones. Parked cases do not count (IN-01, TU-09), nor do
    /// briefings, which have their own count (TU-10).
    pub(crate) fn waiting(&self) -> (usize, usize) {
        let open = self
            .cases
            .iter()
            .filter(|(_, case)| case.state() == State::Open && case.kind() != Kind::Briefing);
        open.fold((0, 0), |(waiting, blocking), (_, case)| {
            (
                waiting + 1,
                blocking + usize::from(case.urgency() == Urgency::Blocking),
            )
        })
    }

    /// Says why something was not done, or what to do, in a toast.
    pub(crate) fn flash(&mut self, text: &str) {
        self.toast(text, false);
    }

    /// Says that something was done, in a toast.
    pub(crate) fn done(&mut self, text: &str) {
        self.toast(text, true);
    }

    fn toast(&mut self, text: &str, done: bool) {
        // The same again is the newest toast up for longer, not another.
        if let Some(last) = self
            .toasts
            .last_mut()
            .filter(|last| last.text == text && last.done == done)
        {
            last.at = Instant::now();
            return;
        }
        if self.toasts.len() >= TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            id: self.next_toast,
            text: text.into(),
            done,
            at: Instant::now(),
        });
        self.next_toast += 1;
    }

    /// The newest toast's text.
    pub(crate) fn said(&self) -> Option<&str> {
        self.toasts.last().map(|toast| toast.text.as_str())
    }

    /// The newest text of a toast that says something was done.
    pub(crate) fn said_done(&self) -> Option<&str> {
        self.toasts
            .iter()
            .rev()
            .find(|toast| toast.done)
            .map(|toast| toast.text.as_str())
    }

    /// Puts away the toasts that have been up long enough.
    ///
    /// Gives whether any went, so that the screen is drawn without them.
    pub(crate) fn expire_toasts(&mut self, now: Instant) -> bool {
        let up = self.toasts.len();
        self.toasts
            .retain(|toast| now.duration_since(toast.at) <= TOAST_FOR);
        self.toasts.len() != up
    }

    /// What a click at `at` acts on, from where the last frame drew it.
    pub(crate) fn hit_at(&self, at: Position) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, hit)| *hit)
    }

    /// Whether a click on `hit` would act, as [`App::click`] has it, so
    /// that the pointer over it lights it: not a pane, which only the wheel
    /// acts on, nor a dialog's face; nothing under help, which a click only
    /// closes; only a dialog's buttons while it asks; and not the list or
    /// the tabs while a form or a message is open.
    pub(crate) fn acts_on(&self, hit: Hit) -> bool {
        if self.help {
            return false;
        }
        if self.asking() {
            return matches!(hit, Hit::Key(_));
        }
        let modal = self.modal();
        let typing = self.form.as_ref().is_some_and(|f| f.editing.is_some());
        match hit {
            Hit::List | Hit::Case | Hit::Dialog | Hit::Verdicts(..) => false,
            Hit::Tab(_) => !modal && !typing && self.only.is_none(),
            Hit::Row(_) | Hit::Fold => !modal && !typing,
            Hit::Choice(_)
            | Hit::LeftOut(_)
            | Hit::ApprovalRow(_)
            | Hit::Field(_)
            | Hit::Mark(_)
            | Hit::Verdict(..)
            | Hit::Press(_)
            | Hit::Verb(_) => !modal,
            Hit::Key(_) | Hit::Toast(_) => true,
        }
    }

    /// Whether a form that is not the answer's panel, or a message, has
    /// the keys: a park's or a withdraw's, which the case's bar and its
    /// panel wait for, as the list and the filters do.
    pub(crate) fn modal(&self) -> bool {
        self.compose.is_some() || self.form.as_ref().is_some_and(|f| f.act != Act::Answer)
    }

    /// The answer's panel of the case `key` as it stands: the form with
    /// the keys, or the case's draft.
    pub(crate) fn answer_of(&self, key: Key) -> Option<&Form> {
        self.form
            .iter()
            .chain(&self.drafts)
            .find(|form| form.key == key && form.act == Act::Answer)
    }

    /// Takes the keys out of the answer's panel, keeping what was chosen or
    /// typed in it as its case's draft.
    fn leave_panel(&mut self) {
        if self.form.as_ref().is_some_and(|f| f.act == Act::Answer)
            && let Some(form) = self.form.take()
            && form.touched()
        {
            self.drafts.retain(|draft| draft.key != form.key);
            self.drafts.push(form);
        }
    }

    /// What a click under the pointer would act on, from where the last
    /// frame drew each part: what the view lights.
    pub(crate) fn hovered(&self) -> Option<Hit> {
        self.pointer
            .and_then(|at| self.hit_at(at))
            .filter(|hit| self.acts_on(*hit))
    }

    /// The acts the case's bar offers on the selected case, as buttons
    /// (TU-12): those its keys would do, so that the bar and the keys
    /// agree. None while a form or a message is open, which takes the keys;
    /// on one case's screen (TU-11), only those it takes.
    pub(crate) fn verbs(&self) -> Vec<Verb> {
        if self.modal() {
            return Vec::new();
        }
        let Some((_, case)) = self.selected_case() else {
            return Vec::new();
        };
        let fits = |verb: Verb| match verb {
            Verb::Answer => Act::Answer.fits(case),
            Verb::Park => Act::Park.fits(case),
            Verb::Resume => resumes(case),
            Verb::Withdraw => Act::Withdraw.fits(case),
            Verb::Edit => edits(case),
        };
        Verb::ALL
            .into_iter()
            .filter(|verb| fits(*verb))
            .filter(|verb| self.only.is_none() || one_case_takes(KeyCode::Char(verb.key()), false))
            .collect()
    }

    /// Whether a question is up, in a dialog: quitting, or the form's or
    /// the message's.
    pub(crate) fn asking(&self) -> bool {
        self.confirm_quit
            || self.form.as_ref().is_some_and(|f| f.confirm.is_some())
            || self.compose.as_ref().is_some_and(|c| c.confirm)
    }

    /// Does what `key` asks (TU-08).
    pub(crate) fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.confirm_quit {
            self.confirm_quit = false;
            self.quit = matches!(key.code, KeyCode::Char('y' | 'Y'));
            return;
        }
        if self.help && !(ctrl && key.code == KeyCode::Char('c')) {
            self.help = false;
            return;
        }
        if self.quits(key.code, ctrl) {
            self.ask_quit();
            return;
        }
        if self.form.is_some() || self.compose.is_some() {
            self.open_key(key);
            return;
        }
        match self.focus {
            Focus::List => self.list_key(key.code, ctrl),
            Focus::Case => self.case_key(key.code, ctrl),
        }
    }

    /// Does what the mouse asks (TU-12): a click acts on what it lands on,
    /// the wheel scrolls what it is over, and what the pointer is over is
    /// lit. While help is up, a click is any other key: it closes help.
    /// While a question is up, a click on one of its buttons presses its
    /// key, on the dialog does nothing, and anywhere else answers no.
    ///
    /// Gives whether the screen is to be drawn again: not for a move of
    /// the pointer that leaves it over the same thing.
    pub(crate) fn mouse(&mut self, event: MouseEvent) -> bool {
        let at = Position::new(event.column, event.row);
        let hit = self.hit_at(at);
        self.pointer = Some(at);
        let was = self.hover;
        self.hover = self.hovered();
        // A toast is not scrolled: the wheel over one scrolls what is
        // under it.
        let under = || {
            self.hits
                .iter()
                .rev()
                .filter(|(_, hit)| !matches!(hit, Hit::Toast(_)))
                .find(|(rect, _)| rect.contains(at))
                .map(|(_, hit)| *hit)
        };
        match event.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => return self.hover != was,
            MouseEventKind::Down(MouseButton::Left) => self.click(hit),
            MouseEventKind::ScrollDown => self.wheel(under(), 1),
            MouseEventKind::ScrollUp => self.wheel(under(), -1),
            _ => {}
        }
        // What the click or the wheel did may have put something over the
        // pointer, as help, which nothing under it is lit through.
        self.hover = self.hovered();
        true
    }

    fn wheel(&mut self, hit: Option<Hit>, by: isize) {
        if self.help || self.confirm_quit {
            return;
        }
        match hit {
            Some(Hit::List | Hit::Row(_) | Hit::Fold | Hit::Tab(_))
                if self.form.is_none() && self.compose.is_none() =>
            {
                self.move_by(by);
            }
            Some(
                Hit::List
                | Hit::Row(_)
                | Hit::Fold
                | Hit::Tab(_)
                | Hit::Key(_)
                | Hit::Dialog
                | Hit::Toast(_),
            )
            | None => {}
            Some(_) if by > 0 => self.scroll = self.scroll.saturating_add(WHEEL),
            Some(_) => self.scroll = self.scroll.saturating_sub(WHEEL),
        }
    }

    fn click(&mut self, hit: Option<Hit>) {
        if self.help || self.confirm_quit {
            if let Some(Hit::Key(key)) = hit {
                self.key(key);
            } else if !(self.confirm_quit && hit == Some(Hit::Dialog)) {
                self.help = false;
                self.confirm_quit = false;
            }
            return;
        }
        // A question the form or the message asks: a click on one of the
        // dialog's buttons presses its key, and one off the dialog is any
        // other key, which answers no.
        if self.form.as_ref().is_some_and(|f| f.confirm.is_some())
            || self.compose.as_ref().is_some_and(|c| c.confirm)
        {
            if let Some(Hit::Key(key)) = hit {
                self.key(key);
            } else if hit != Some(Hit::Dialog) {
                if let Some(form) = &mut self.form {
                    form.confirm = None;
                }
                if let Some(compose) = &mut self.compose {
                    compose.confirm = false;
                }
            }
            return;
        }
        let Some(hit) = hit else {
            return;
        };
        let open = self.modal() || self.form.as_ref().is_some_and(|f| f.editing.is_some());
        match hit {
            Hit::Key(key) => self.key(key),
            Hit::Toast(id) => {
                self.toasts.retain(|toast| toast.id != id);
            }
            // As the keys: the list and the filters wait while a form or a
            // message is open. A dialog's face does nothing.
            // The list and the filters wait while a park, a withdraw or a
            // message is open, or a field of the answer is typed in; with
            // the keys only in the answer's panel, they take them out of it.
            Hit::Tab(filter) if !open && self.only.is_none() => {
                self.leave_panel();
                self.set_filter(filter);
            }
            Hit::Fold if !open => {
                self.leave_panel();
                self.pane_key(KeyCode::Char('f'), false);
            }
            Hit::Row(key) if !open => {
                if self.selected == Some(key) {
                    self.focus = Focus::Case;
                } else if let Some(at) = self.shown(self.filter).iter().position(|(k, _)| *k == key)
                {
                    self.leave_panel();
                    self.select(at);
                }
            }
            Hit::Tab(_)
            | Hit::Fold
            | Hit::Row(_)
            | Hit::List
            | Hit::Case
            | Hit::Dialog
            | Hit::Verdicts(..) => {}
            _ if self.modal() => {}
            // The bar's buttons do what their keys do outside the panel.
            Hit::Verb(verb) => {
                self.leave_panel();
                self.pane_key(KeyCode::Char(verb.key()), false);
            }
            Hit::Press(press) => {
                if let Some(form) = self.enter_panel() {
                    let outcome = form.press(press);
                    self.form_outcome(outcome);
                }
            }
            Hit::Choice(_)
            | Hit::LeftOut(_)
            | Hit::ApprovalRow(_)
            | Hit::Field(_)
            | Hit::Mark(_)
            | Hit::Verdict(..) => {
                // A click in the answer's panel puts the keys there, and
                // does what it lands on.
                if let Some(form) = self.enter_panel() {
                    form.click(hit);
                }
            }
        }
    }

    /// Puts the keys in the selected case's answer's panel, with its draft,
    /// and gives its form; none when the case cannot be answered.
    fn enter_panel(&mut self) -> Option<&mut Form> {
        let here = self
            .form
            .as_ref()
            .is_some_and(|f| f.act == Act::Answer && Some(f.key) == self.selected);
        if !here {
            self.start(Act::Answer);
        }
        self.form.as_mut().filter(|form| form.act == Act::Answer)
    }

    /// Whether the key quits: ctrl+c always, and q unless a field of the
    /// form or a part of the message has the keys.
    fn quits(&self, code: KeyCode, ctrl: bool) -> bool {
        (ctrl && code == KeyCode::Char('c'))
            || (self.form.as_ref().is_none_or(|f| f.editing.is_none())
                && self.compose.as_ref().is_none_or(|c| c.part.is_none())
                && code == KeyCode::Char('q'))
    }

    /// Quits, or asks first when a form or a message has something typed
    /// that is not on its way in the write being sent.
    fn ask_quit(&mut self) {
        let sent = |key: Option<Key>| match &self.sending {
            Some(Sending::Event(send)) => key == Some(send.key),
            Some(Sending::Message(_)) => key.is_none(),
            None => false,
        };
        if self
            .form
            .as_ref()
            .is_some_and(|form| form.typed() && !sent(Some(form.key)))
            || self.compose.as_ref().is_some_and(|compose| {
                compose.typed() && !sent(compose.editing.map(|(key, _)| key))
            })
            || self
                .drafts
                .iter()
                .any(|draft| draft.typed() && !sent(Some(draft.key)))
        {
            self.confirm_quit = true;
        } else {
            self.quit = true;
        }
    }

    /// A key while a form or a message is open: the page keys scroll the
    /// case, and the form or the message takes the rest.
    fn open_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            _ => {
                if let Some(form) = &mut self.form {
                    let outcome = form.key(key);
                    self.form_outcome(outcome);
                } else {
                    self.compose_key(key);
                }
            }
        }
    }

    /// A key when the list has the keys: esc quits, and j and k move the
    /// selection.
    fn list_key(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            _ => self.pane_key(code, ctrl),
        }
    }

    /// A key when the case has the keys: esc, left or h go back to the
    /// list, when there is one, and j and k scroll the case by a line.
    fn case_key(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                if self.only.is_none() {
                    self.focus = Focus::List;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            _ => self.pane_key(code, ctrl),
        }
    }

    /// A key either pane takes: an act on the selected case, help or a
    /// filter, then the keys that move about. q never gets here: `quits`
    /// takes it while no form or message is open. On one case (TU-11)
    /// only the keys [`one_case_takes`] names do anything.
    fn pane_key(&mut self, code: KeyCode, ctrl: bool) {
        if self.only.is_some() {
            // p parks only a stuck case: on another, as the footer says,
            // it does nothing.
            let parks = self
                .selected_case()
                .is_some_and(|(_, case)| Act::Park.fits(case));
            if !one_case_takes(code, ctrl) || (code == KeyCode::Char('p') && !parks) {
                return;
            }
        }
        match code {
            KeyCode::Char('a') => {
                // The panel is at the foot of the case: scrolled to.
                self.start(Act::Answer);
                if self.form.is_some() {
                    self.scroll = u16::MAX;
                }
            }
            KeyCode::Char('p') => self.start(Act::Park),
            KeyCode::Char('w') => self.start(Act::Withdraw),
            KeyCode::Char('r') => self.resume(),
            KeyCode::Char('m') => {
                self.compose = Some(Compose::new());
                self.focus = Focus::Case;
            }
            KeyCode::Char('e') => self.edit_sent(),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('s') if !ctrl && self.strip.room => self.strip.shown = !self.strip.shown,
            KeyCode::Tab => self.set_filter(self.filter.step(1)),
            KeyCode::BackTab => self.set_filter(self.filter.step(-1)),
            KeyCode::Char('f') if self.filter == Filter::Briefings => {
                self.answered = match self.answered {
                    Group::Folded => Group::Shown,
                    Group::Shown => Group::Folded,
                };
                self.keep_selection();
            }
            KeyCode::Char(c @ '1'..='5') => {
                let at = usize::from(u8::try_from(c).unwrap_or(b'1') - b'1');
                self.set_filter(Filter::ALL[at]);
            }
            _ => self.move_key(code, ctrl),
        }
    }

    /// The keys that move about: open the selected case, scroll it by the
    /// page, or select the first or last case.
    fn move_key(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') if self.selected.is_some() => {
                self.focus = Focus::Case;
            }
            KeyCode::Char('d') if ctrl => self.scroll = self.scroll.saturating_add(10),
            KeyCode::Char('u') if ctrl => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Char('J' | ' ') | KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(10);
            }
            KeyCode::Char('K') | KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(usize::MAX),
            _ => {}
        }
    }

    /// Opens the form for `act` on the selected case, or says why it
    /// cannot.
    fn start(&mut self, act: Act) {
        let Some((key, case)) = self.selected_case() else {
            return;
        };
        let key = *key;
        if act.fits(case) {
            let fresh = Form::new(key, case, act);
            self.leave_panel();
            let draft = self
                .drafts
                .iter()
                .position(|draft| draft.key == key && act == Act::Answer)
                .map(|at| self.drafts.remove(at));
            self.form = Some(draft.unwrap_or(fresh));
            if self.focus == Focus::List {
                self.focus = Focus::Case;
            }
            return;
        }
        let why = match act {
            Act::Answer if case.kind() == Kind::Message => {
                format!("{key} is a message: an agent picks it up")
            }
            Act::Answer => format!("{key} is {}: nothing to answer", case.state()),
            Act::Park if case.kind() != Kind::Stuck => {
                format!("{key}: only a stuck case parks")
            }
            Act::Park => format!("{key} is {}: only an open case parks", case.state()),
            Act::Withdraw => format!(
                "{key} is {}: withdraw takes back an answered or picked-up case; d in the answer drops an open one",
                case.state()
            ),
        };
        self.flash(&why);
    }

    /// Resumes the selected case, if it is parked.
    fn resume(&mut self) {
        let Some((key, case)) = self.selected_case() else {
            return;
        };
        let key = *key;
        if !resumes(case) {
            let line = format!("{key} is {}: only a parked case resumes", case.state());
            self.flash(&line);
            return;
        }
        let revision = case.revision();
        self.send(key, Record::Resume(Resume::default()), revision, "resume");
    }

    fn form_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Stay => {}
            Outcome::Cancel => self.form = None,
            Outcome::Leave => self.leave_panel(),
            Outcome::Park => {
                self.leave_panel();
                self.start(Act::Park);
            }
            Outcome::Edit(which) => self.edit = Some(which),
            Outcome::Missing(what) => self.flash(&format!("nothing sent: {what}")),
            Outcome::Send(record) => {
                if let Some(form) = &self.form {
                    let (key, revision, what) = (form.key, form.revision, form.act.event());
                    self.send(key, *record, revision, what);
                }
            }
        }
    }

    /// Opens the selected sent message to edit its body (HU-08).
    fn edit_sent(&mut self) {
        let Some((key, case)) = self.selected_case() else {
            return;
        };
        let key = *key;
        if edits(case) {
            self.compose = Some(Compose::edit(key, case));
            self.focus = Focus::Case;
        } else {
            let line = format!("{key}: only a message no agent has picked up can be edited");
            self.flash(&line);
        }
    }

    fn compose_key(&mut self, key: KeyEvent) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        // The picker: a number names a session listening now.
        if compose.part.is_none()
            && !compose.confirm
            && compose.editing.is_none()
            && let KeyCode::Char(c @ '1'..='9') = key.code
        {
            let at = usize::try_from(u32::from(c) - u32::from('1')).unwrap_or(usize::MAX);
            if let Some(listening) = self.listeners.get(at) {
                compose.pick(&listening.worker.clone());
            }
            return;
        }
        match compose.key(key) {
            compose::Outcome::Stay => {}
            compose::Outcome::Cancel => self.compose = None,
            compose::Outcome::Edit(part) => self.edit_message = Some(part),
            compose::Outcome::Missing(what) => self.flash(&format!("nothing sent: {what}")),
            compose::Outcome::Send(_) if self.sending.is_some() => self.still_sending(),
            compose::Outcome::Send(record) => match compose.editing {
                Some((key, revision)) => self.send(key, *record, revision, "edit"),
                None => self.pending_message = Some(*record),
            },
        }
    }

    /// Takes how a new message went: its key, and whether a session listens
    /// for it now; or why it was refused.
    pub(crate) fn sent_message(&mut self, result: Result<(Key, bool), String>) {
        match result {
            Ok((key, listened)) => {
                self.compose = None;
                let line = if listened {
                    format!("sent message {key}")
                } else {
                    format!(
                        "sent message {key}; no session is listening for it now, so it waits until one is"
                    )
                };
                self.done(&line);
            }
            Err(why) => self.flash(&format!("nothing sent: {why}")),
        }
    }

    /// Says that a write cannot be sent while one is: the one sent may be
    /// refused, or change the case.
    fn still_sending(&mut self) {
        if let Some(sending) = &self.sending {
            let line = format!("{}: wait for it, then send", sending.line());
            self.flash(&line);
        }
    }

    /// The line the footer shows while a write is being sent.
    pub(crate) fn sending_line(&self) -> Option<String> {
        self.sending.as_ref().map(Sending::line)
    }

    /// What the human typed into the write being sent, part by part: none
    /// when none is being sent.
    pub(crate) fn sending_typed(&self) -> Vec<String> {
        match &self.sending {
            Some(Sending::Event(send)) => typed_in(&send.record),
            Some(Sending::Message(record)) => typed_in(record),
            None => Vec::new(),
        }
    }

    /// Takes how the write being sent went, once the read after it has
    /// landed.
    pub(crate) fn wrote(&mut self, wrote: Wrote) {
        match (self.sending.take(), wrote) {
            (Some(Sending::Event(send)), Wrote::Append(result)) => self.sent(&send, result),
            (Some(Sending::Message(_)), Wrote::Create(result)) => self.sent_message(result),
            // Not asked for here: nothing to say.
            _ => {}
        }
        // With the write said, a form or a draft its read left open on a
        // case it no longer fits, as when the write was refused because
        // the case moved on meanwhile, goes, saying nothing was sent.
        self.follow_form();
    }

    /// Queues a write for the loop, with the list as it stands, so that the
    /// next case can be selected after it (IN-03); not while one is being
    /// sent.
    fn send(&mut self, key: Key, record: Record, revision: u64, what: &'static str) {
        if self.sending.is_some() {
            self.still_sending();
            return;
        }
        let order: Vec<Key> = self.shown(self.filter).iter().map(|(k, _)| *k).collect();
        let at = order.iter().position(|k| *k == key).unwrap_or(0);
        self.pending = Some(Send {
            key,
            record,
            revision,
            what,
            order,
            at,
        });
    }

    /// Takes how a write went, after the store was read again.
    pub(crate) fn sent(&mut self, send: &Send, result: Result<(), Refusal>) {
        match result {
            Ok(()) => {
                if self.form.as_ref().is_some_and(|f| f.key == send.key) {
                    self.form = None;
                }
                self.drafts.retain(|draft| draft.key != send.key);
                if self
                    .compose
                    .as_ref()
                    .is_some_and(|c| c.editing.is_some_and(|(k, _)| k == send.key))
                {
                    self.compose = None;
                }
                let shown: Vec<Key> = self.shown(self.filter).iter().map(|(k, _)| *k).collect();
                let next = send.order[send.at + 1..]
                    .iter()
                    .chain(send.order[..send.at].iter().rev())
                    .find(|k| shown.contains(k));
                match next.and_then(|k| shown.iter().position(|s| s == k)) {
                    // A form opened on another case while this was sent
                    // keeps its case selected: it is drawn only there.
                    Some(at) if self.form.is_none() => self.select(at),
                    _ => self.keep_selection(),
                }
                if self.form.is_none() && self.selected != Some(send.key) {
                    self.focus = Focus::List;
                }
                self.done(&format!("recorded {} on {}", send.what, send.key));
            }
            Err(Refusal::Stale) => {
                // The case as it is now is on screen. A form keeps what was
                // typed and takes the case as amended, so it can be read and
                // sent again (IN-05); only the one that sent it: the other,
                // opened on the case meanwhile, has not been shown the case
                // as it is now. The edit of a message, which sends an amend
                // as a form never does, instead starts again from the
                // message as it is now, as the web inbox does, so that a
                // second send does not write over the edit made elsewhere.
                let now = self
                    .cases
                    .iter()
                    .find(|(k, _)| *k == send.key)
                    .map(|(_, case)| case);
                let edit = matches!(send.record, Record::Amend(_));
                if let (Some(form), Some(case)) = (&mut self.form, now)
                    && !edit
                    && form.key == send.key
                {
                    form.amended(case);
                }
                // Or its draft, when the keys left the answer while it was
                // being sent.
                if let Some(case) = now
                    && !edit
                    && let Some(draft) = self.drafts.iter_mut().find(|d| d.key == send.key)
                {
                    draft.amended(case);
                }
                let mut replaced = false;
                if let (Some(compose), Some(case)) = (&mut self.compose, now)
                    && edit
                    && compose.editing.is_some_and(|(key, _)| key == send.key)
                {
                    *compose = Compose::edit(send.key, case);
                    replaced = true;
                }
                if replaced {
                    self.flash(&format!(
                        "nothing sent: {} changed since you read it; your edit was replaced by the message as it is now",
                        send.key
                    ));
                } else {
                    self.flash(&format!(
                        "nothing sent: {} changed since you read it; read it again, then send",
                        send.key
                    ));
                }
            }
            Err(Refusal::Other(why)) => self.flash(&format!("nothing sent: {why}")),
        }
    }
}

/// A write for the loop to send.
#[derive(Debug)]
pub(crate) struct Send {
    pub(crate) key: Key,
    pub(crate) record: Record,
    /// The revision the case had when the write was asked for.
    pub(crate) revision: u64,
    /// The event, for the line that says it was recorded.
    pub(crate) what: &'static str,
    /// The list's keys when it was asked for, and where the case stood.
    order: Vec<Key>,
    at: usize,
}

/// A write being sent (TU-04).
#[derive(Debug)]
pub(crate) enum Sending {
    /// The human's event on a case.
    Event(Box<Send>),
    /// A new message.
    Message(Box<Record>),
}

impl Sending {
    fn line(&self) -> String {
        match self {
            Sending::Event(send) => format!("⋯ sending {} on {}", send.what, send.key),
            Sending::Message(_) => "⋯ sending the message".into(),
        }
    }
}

/// The text the human typed into `record`, part by part, leaving out the
/// empty parts.
fn typed_in(record: &Record) -> Vec<String> {
    let parts: Vec<&str> = match record {
        Record::Answer(answer) => std::iter::once(answer.text.as_str())
            .chain(answer.rows.iter().map(|row| row.note.as_str()))
            .chain(std::iter::once(answer.note.as_str()))
            .collect(),
        Record::Park(park) => vec![&park.note],
        Record::Withdraw(withdraw) => vec![&withdraw.reason],
        Record::Amend(amend) => vec![&amend.body],
        Record::Open(open) => vec![&open.title, &open.body],
        _ => Vec::new(),
    };
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// Why a write was refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The case changed since the form was drawn.
    Stale,
    /// Anything else: the reason.
    Other(String),
}
