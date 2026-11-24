//! What can be done on the screen, as data (TU-12): each action's name as
//! the palette shows it, why one would, its key, when it fits, and the key
//! that does it. The palette, the menu a right-click opens on a row and
//! the case's bar all read this table, so that they and the keys agree.

use ratatui::crossterm::event::KeyCode;

use super::app::{App, Filter, Verb, edits, one_case_takes, resumes};
use super::form::Act;

/// An action of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Answer,
    Park,
    Resume,
    Withdraw,
    Edit,
    Message,
    Filter(Filter),
    NextFilter,
    PreviousFilter,
    Fold,
    Stats,
    First,
    Last,
    Help,
    Quit,
}

impl Action {
    /// Every action, in the order the palette lists them when nothing is
    /// typed: the selected case's first, then moving about, then the rest.
    pub(crate) const ALL: [Action; 19] = [
        Action::Answer,
        Action::Park,
        Action::Resume,
        Action::Withdraw,
        Action::Edit,
        Action::Message,
        Action::Filter(Filter::Inbox),
        Action::Filter(Filter::Flight),
        Action::Filter(Filter::Done),
        Action::Filter(Filter::All),
        Action::Filter(Filter::Briefings),
        Action::NextFilter,
        Action::PreviousFilter,
        Action::Fold,
        Action::First,
        Action::Last,
        Action::Stats,
        Action::Help,
        Action::Quit,
    ];

    /// The action's name, as the palette and the menu show it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Action::Answer => "Answer",
            Action::Park => "Park",
            Action::Resume => "Resume",
            Action::Withdraw => "Withdraw",
            Action::Edit => "Edit message",
            Action::Message => "New message",
            Action::Filter(Filter::Inbox) => "Show inbox",
            Action::Filter(Filter::Flight) => "Show in flight",
            Action::Filter(Filter::Done) => "Show done",
            Action::Filter(Filter::All) => "Show all",
            Action::Filter(Filter::Briefings) => "Show briefings",
            Action::NextFilter => "Next filter",
            Action::PreviousFilter => "Previous filter",
            Action::Fold => "Fold answered briefings",
            Action::First => "First case",
            Action::Last => "Last case",
            Action::Stats => "Stats",
            Action::Help => "Keys",
            Action::Quit => "Quit",
        }
    }

    /// Why one would, in a few words.
    pub(crate) fn why(self) -> &'static str {
        match self {
            Action::Answer => "write the answer at the foot of the case",
            Action::Park => "set a stuck case aside, with why",
            Action::Resume => "take a parked case up again",
            Action::Withdraw => "take back an answer, or a message",
            Action::Edit => "change a message no agent has picked up",
            Action::Message => "write to an agent",
            Action::Filter(_) | Action::NextFilter | Action::PreviousFilter => {
                "what the list shows"
            }
            Action::Fold => "show or hide the answered briefings",
            Action::First => "select the first case in the list",
            Action::Last => "select the last case in the list",
            Action::Stats => "show or hide the last 14 days",
            Action::Help => "every key, in a box",
            Action::Quit => "leave the inbox",
        }
    }

    /// The key that does it, as the screen shows it.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Action::Answer => "a",
            Action::Park => "p",
            Action::Resume => "r",
            Action::Withdraw => "w",
            Action::Edit => "e",
            Action::Message => "m",
            // A filter by name has no key of its own: [ and ] step to it.
            Action::Filter(_) => "",
            Action::NextFilter => "]",
            Action::PreviousFilter => "[",
            Action::Fold => "f",
            Action::First => "g",
            Action::Last => "G",
            Action::Stats => "s",
            Action::Help => "?",
            Action::Quit => "q",
        }
    }

    /// The key pressed to do it: its first character, if it has one.
    pub(crate) fn code(self) -> KeyCode {
        self.key()
            .chars()
            .next()
            .map_or(KeyCode::Null, KeyCode::Char)
    }

    /// The act on the case the bar offers for it, if it is one.
    pub(crate) fn verb(self) -> Option<Verb> {
        match self {
            Action::Answer => Some(Verb::Answer),
            Action::Park => Some(Verb::Park),
            Action::Resume => Some(Verb::Resume),
            Action::Withdraw => Some(Verb::Withdraw),
            Action::Edit => Some(Verb::Edit),
            _ => None,
        }
    }

    /// Whether it fits the screen as it is: the selected case, the filter
    /// and the screen, the inbox or one case's (TU-11), whose keys it
    /// takes. While a park's or a withdraw's form or a message has the
    /// keys, only help and quit, as the rest would leave it.
    pub(crate) fn fits(self, app: &App) -> bool {
        if matches!(self, Action::Help | Action::Quit) {
            return true;
        }
        if app.modal() {
            return false;
        }
        // A filter by name has no key, which one case's screen does not
        // take either.
        if app.only.is_some() && !one_case_takes(self.code(), false) {
            return false;
        }
        let case = app.selected_case().map(|(_, case)| case);
        let on = |fits: fn(&cases_core::Case) -> bool| case.is_some_and(fits);
        match self {
            Action::Answer => on(|case| Act::Answer.fits(case)),
            Action::Park => on(|case| Act::Park.fits(case)),
            Action::Resume => on(resumes),
            Action::Withdraw => on(|case| Act::Withdraw.fits(case)),
            Action::Edit => on(edits),
            Action::Filter(filter) => filter != app.filter,
            Action::Fold => app.filter == Filter::Briefings,
            Action::First | Action::Last => !app.shown(app.filter).is_empty(),
            Action::Stats => app.strip.room,
            Action::NextFilter
            | Action::PreviousFilter
            | Action::Message
            | Action::Help
            | Action::Quit => true,
        }
    }

    /// The actions that fit, in the table's order.
    pub(crate) fn fitting(app: &App) -> Vec<Action> {
        Action::ALL
            .into_iter()
            .filter(|action| action.fits(app))
            .collect()
    }

    /// The actions that fit the selected case and act on it, as the menu a
    /// right-click opens on its row lists them: the case's bar's.
    pub(crate) fn on_case(app: &App) -> Vec<Action> {
        Action::ALL
            .into_iter()
            .filter(|action| action.verb().is_some() && action.fits(app))
            .collect()
    }
}

/// How well `query` matches `name`, lower is better: its characters in
/// order, in any case, each where it is first found from the last. A
/// match from the start, and one with its characters together, comes
/// first. None when they are not all there in order.
pub(crate) fn fuzzy(query: &str, name: &str) -> Option<usize> {
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let mut at = 0;
    let mut first = None;
    let mut gaps = 0;
    for want in query.chars().flat_map(char::to_lowercase) {
        if want == ' ' && query.trim().is_empty() {
            continue;
        }
        let found = name[at..].iter().position(|c| *c == want)?;
        if first.is_some() {
            gaps += found;
        }
        first.get_or_insert(at + found);
        at += found + 1;
    }
    Some(first.unwrap_or(0) * 4 + gaps)
}

/// The actions that fit, matching `query`, best first; in the table's
/// order when nothing is typed, or for an equal match.
pub(crate) fn ranked(app: &App, query: &str) -> Vec<Action> {
    let mut found: Vec<(usize, usize, Action)> = Action::fitting(app)
        .into_iter()
        .enumerate()
        .filter_map(|(order, action)| {
            fuzzy(query.trim(), action.name()).map(|score| (score, order, action))
        })
        .collect();
    found.sort_by_key(|(score, order, _)| (*score, *order));
    found.into_iter().map(|(_, _, action)| action).collect()
}
