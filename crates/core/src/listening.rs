//! Listening: the agent sessions whose `cases wait` takes messages, and
//! what each says it is doing (ST-16, AG-17). They are not events: a store
//! keeps them beside its cases.
//!
//! A local store keeps a row for each wait that listens, which the wait
//! refreshes every [`HEARTBEAT`] and which lapses [`LAPSE`] after its last
//! refresh. A hosted store counts a worker as listening while one of its
//! waits holds a poll open, and writes nothing per poll.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;

use crate::error::Error;
use crate::time::Time;

/// How often a listening wait on a local store refreshes its row.
pub const HEARTBEAT: Duration = Duration::from_secs(15);

/// How long a local store's row counts as listening after its last refresh:
/// three heartbeats, 45 s. A wait that stops cleanly leaves its row to
/// lapse, as one that crashed does, so a session that starts its next wait
/// within this time stays listed, with when it began listening and what it
/// is doing.
pub const LAPSE: Duration = Duration::from_secs(3 * HEARTBEAT.as_secs());

/// The most characters a doing line holds (AG-17).
pub const DOING_MAX: usize = 80;

/// `text` as a doing line (AG-17): trimmed, or `None` when that leaves
/// nothing, which clears the line. Every store checks a doing line by this,
/// in [`Store::listen`](crate::Store::listen) and
/// [`Store::set_doing`](crate::Store::set_doing).
///
/// # Errors
///
/// An [`Error`] of category [`Invalid`](crate::Category::Invalid) when the
/// line has a line break or another control character, or a character that
/// sets the direction of the text after it, which a page showing the line
/// would carry on past its end (U+2028, U+2029, U+202A to U+202E, U+2066 to
/// U+2069), or is longer than [`DOING_MAX`] characters once trimmed.
pub fn doing_line(text: &str) -> Result<Option<String>, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().any(|c| {
        c.is_control()
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err(Error::invalid(
            "a doing line is one line, with no control characters",
        ));
    }
    let length = text.chars().count();
    if length > DOING_MAX {
        return Err(Error::invalid(format!(
            "a doing line holds at most {DOING_MAX} characters, and this one has {length}"
        )));
    }
    Ok(Some(text.to_string()))
}

/// One wait listening for messages, as it recorded itself: which cases and
/// messages it takes follows from its worker, labels and kinds, as its
/// filter has them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Wait {
    /// The id the wait made when it started, or `listeners` for a row of an
    /// older cases, which records no id.
    pub id: String,
    /// The worker it waits for: one of its `--worker`s.
    pub worker: String,
    /// Its `--label`s, by name, without a blank one.
    pub labels: Vec<String>,
    /// Its `--kind`s, by name: none for every kind.
    pub kinds: Vec<String>,
    /// When its worker began listening: when this wait began, or, when it
    /// began while another wait of the worker listened, that one's time.
    pub since: Time,
    /// When it last refreshed its row, or, on a hosted store, when the
    /// store last heard from it.
    pub heard: Time,
}

impl Wait {
    /// The wait `id` of `worker`, on `labels` and `kinds`, listening since
    /// `since` and last heard at `heard`.
    #[must_use]
    pub fn new(
        id: String,
        worker: String,
        labels: Vec<String>,
        kinds: Vec<String>,
        since: Time,
        heard: Time,
    ) -> Wait {
        Wait {
            id,
            worker,
            labels,
            kinds,
            since,
            heard,
        }
    }
}

/// A worker listening for messages: what its waits listening now have in
/// common, as the inbox shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listening {
    /// The session's name: its waits' `--worker`.
    pub worker: String,
    /// The labels its waits listen on, by name.
    pub labels: Vec<String>,
    /// When it began listening: when the first of its waits began, or, when
    /// a wait began while another listened, that one's time.
    pub since: Time,
    /// When a wait of its was last heard.
    pub heard: Time,
    /// What it says it is doing, one line, if anything.
    pub doing: Option<String>,
    /// When it set that line. Setting the same line again keeps its time.
    pub doing_at: Option<Time>,
    /// Its waits listening now, by id, each with the labels and kinds it
    /// waits on.
    pub waits: Vec<Wait>,
}

impl Listening {
    /// The worker `worker`, listening on `labels` since `since`, last heard
    /// at `heard`, doing `doing` since `doing_at`, by its `waits`.
    #[must_use]
    pub fn new(
        worker: String,
        labels: Vec<String>,
        since: Time,
        heard: Time,
        doing: Option<String>,
        doing_at: Option<Time>,
        waits: Vec<Wait>,
    ) -> Listening {
        Listening {
            worker,
            labels,
            since,
            heard,
            doing,
            doing_at,
            waits,
        }
    }

    /// Whether the worker still counts as listening at `now`, on a local
    /// store: a wait of its was heard no more than [`LAPSE`] before. One
    /// heard after `now`, by a clock ahead of this one, counts.
    #[must_use]
    pub fn live(&self, now: Time) -> bool {
        heard_in_time(self.heard, now)
    }
}

/// Whether something heard at `heard` still counts at `now`: no more than
/// [`LAPSE`] before it, or after it, by a clock ahead of this one. A row
/// whose clock was far ahead counts until it lapses by this one; bounding
/// that would drop every live row on a clock stepped back.
#[must_use]
pub fn heard_in_time(heard: Time, now: Time) -> bool {
    before(heard, now).is_none_or(|ago| ago <= LAPSE)
}

/// How long before `now` the time `at` is, or `None` when it is after it.
fn before(at: Time, now: Time) -> Option<Duration> {
    let nanos = |t: Time| i128::from(t.unix_secs()) * 1_000_000_000 + i128::from(t.subsec_nanos());
    let diff = u128::try_from(nanos(now) - nanos(at)).ok()?;
    let secs = u64::try_from(diff / 1_000_000_000).ok()?;
    let subsec = u32::try_from(diff % 1_000_000_000).ok()?;
    Some(Duration::new(secs, subsec))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: i64 = 1_000_000_000;

    /// The time `nanos` after a fixed start.
    fn at(nanos: i64) -> Time {
        let start = 1_790_000_000;
        let secs = start + nanos.div_euclid(SECOND);
        let nanos = u32::try_from(nanos.rem_euclid(SECOND)).unwrap();
        Time::from_unix(secs, nanos).unwrap()
    }

    #[test]
    fn a_row_lapses_three_heartbeats_after_it_was_heard() {
        assert_eq!(HEARTBEAT, Duration::from_secs(15));
        assert_eq!(LAPSE, Duration::from_secs(45));
        let wait = Wait::new(
            "w1".into(),
            "bun-pins".into(),
            Vec::new(),
            Vec::new(),
            at(0),
            at(60 * SECOND),
        );
        let listening = Listening::new(
            "bun-pins".into(),
            Vec::new(),
            at(0),
            at(60 * SECOND),
            None,
            None,
            alloc::vec![wait.clone()],
        );
        assert_eq!(listening.waits, [wait]);
        // Heard later than now, by a clock ahead: listening.
        assert!(listening.live(at(0)));
        assert!(listening.live(at(60 * SECOND)));
        assert!(listening.live(at(105 * SECOND)));
        assert!(!listening.live(at(105 * SECOND + 1)));
    }

    #[test]
    fn a_doing_line_is_one_line_of_at_most_80_characters() {
        use crate::Category;

        assert_eq!(doing_line("  rebasing  ").unwrap(), Some("rebasing".into()));
        for blank in ["", "   ", "\u{a0}"] {
            assert_eq!(doing_line(blank).unwrap(), None, "{blank:?}");
        }
        let long = "é".repeat(DOING_MAX);
        assert_eq!(doing_line(&long).unwrap(), Some(long.clone()));
        let longer = format!("{long}é");
        let error = doing_line(&longer).unwrap_err();
        assert_eq!(error.category(), Category::Invalid);
        assert_eq!(
            error.to_string(),
            "a doing line holds at most 80 characters, and this one has 81"
        );
        for text in [
            "a\nb",
            "a\rb",
            "a\tb",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
            "a\u{202a}b",
            "a\u{202e}b",
            "a\u{2066}b",
            "a\u{2069}b",
        ] {
            let error = doing_line(text).unwrap_err();
            assert_eq!(error.category(), Category::Invalid, "{text:?}");
        }
        // Just outside the ranges: taken.
        for text in ["a\u{2027}b", "a\u{202f}b", "a\u{2065}b", "a\u{206a}b"] {
            assert!(doing_line(text).is_ok(), "{text:?}");
        }
    }

    #[test]
    fn before_is_how_long_before_now_a_time_is() {
        assert_eq!(before(at(0), at(0)), Some(Duration::ZERO));
        assert_eq!(
            before(at(SECOND), at(3 * SECOND + 5)),
            Some(Duration::new(2, 5))
        );
        assert_eq!(before(at(1), at(0)), None);
    }
}
