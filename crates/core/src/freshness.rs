//! Freshness: how a case changed after a form was drawn from it.
//!
//! An inbox draws a form, such as an answer or a withdraw, from a case as
//! it stands, and keeps the case's [`revision`](Case::revision) and
//! [`last_seq`](Case::last_seq). The write carries that revision, and the
//! writer refuses it as stale if the case has moved on (IN-05). The case
//! keeps changing while the form is open, with notes for one, so on each
//! read the inbox asks [`Case::since`] whether the case is still the one
//! the human read, and takes the new revision only when it is.

use crate::case::Case;
use crate::names::{Author, EventType, State};

/// How a case changed after a form was drawn from it: the answer of
/// [`Case::since`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Since {
    /// Nothing, or only notes and amends that kept the case in its state
    /// and left the question as it was. A write may take the new revision.
    Unchanged,
    /// The case moved to other states, each passed once, by rows that all
    /// folded and with the question as it was. A form that fits the new
    /// state may take the new revision; one that does not is closed.
    Moved,
    /// Anything else: a row the fold skipped, a state passed twice, a
    /// question-changing amend, or a revision above the case's. A write
    /// keeps the old revision, and is refused as stale.
    Changed,
}

impl Since {
    /// The answer as the vectors spell it: `unchanged`, `moved` or
    /// `changed`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Since::Unchanged => "unchanged",
            Since::Moved => "moved",
            Since::Changed => "changed",
        }
    }
}

/// The number of states, which bounds the states a walk passes.
const STATES: usize = State::ALL.len();

impl Case {
    /// How the case changed since a form was drawn from it at `revision`,
    /// when its [`last_seq`](Case::last_seq) was `seq` (IN-05, IN-06).
    ///
    /// The events since are the ones with a seq above `seq`. The answer is:
    ///
    /// 1. [`Unchanged`](Since::Unchanged) when `revision` is the case's.
    /// 2. [`Changed`](Since::Changed) when `revision` is above the case's.
    /// 3. `Changed` when the events since are not as many as the rows added
    ///    since `revision`: fewer when the fold skipped a row, more when
    ///    `seq` is below the one `revision` had.
    /// 4. `Changed` when an amend since changed the question, as
    ///    [`amend_seq`](Case::amend_seq) counts one. A message is the
    ///    exception: its amends are the human's own (IN-13).
    /// 5. `Unchanged` when every event since is a note or an amend that
    ///    left the case in its state.
    /// 6. Otherwise the states the case passed: each event's state before
    ///    it, then the state now. `Changed` when an event since has no
    ///    state before it: the open, so the form was drawn before the case
    ///    had a state. [`Moved`](Since::Moved) when no state comes back
    ///    once the case has left it, and `Changed` when one does, as in an
    ///    answer and a reopen, or a park and a resume.
    ///
    /// The state the form was drawn in need not be kept: it is the state
    /// before the first event since.
    #[must_use]
    pub fn since(&self, revision: u64, seq: u64) -> Since {
        if revision == self.revision() {
            return Since::Unchanged;
        }
        let Some(added) = self.revision().checked_sub(revision) else {
            return Since::Changed;
        };
        let events = self.events();
        let since = &events[events.partition_point(|event| event.seq <= seq)..];
        if since.len() as u64 != added {
            return Since::Changed;
        }
        if self.amend_seq() > seq && self.kind().opener() != Author::Human {
            return Since::Changed;
        }
        let state = self.state();
        if since.iter().all(|event| {
            matches!(event.event(), EventType::Note | EventType::Amend) && event.from == Some(state)
        }) {
            return Since::Unchanged;
        }
        // The states the case has left, each once, and the one it is in.
        let mut left = [state; STATES];
        let mut passed = 0;
        let mut at: Option<State> = None;
        for next in since.iter().map(|event| event.from).chain([Some(state)]) {
            // Only the open event has no state before it, and a form is
            // drawn from a case that has one.
            let Some(next) = next else {
                return Since::Changed;
            };
            match at {
                Some(at) if at == next => continue,
                Some(at) => {
                    if left[..passed].contains(&next) {
                        return Since::Changed;
                    }
                    // A state is left once, so `left` holds them all.
                    left[passed] = at;
                    passed += 1;
                }
                None => {}
            }
            at = Some(next);
        }
        Since::Moved
    }

    /// Whether the case is [`Unchanged`](Since::Unchanged) since `revision`
    /// and `seq`, as [`since`](Case::since) says: a write drawn then may
    /// take the case's revision now.
    #[must_use]
    pub fn unchanged_since(&self, revision: u64, seq: u64) -> bool {
        self.since(revision, seq) == Since::Unchanged
    }
}
