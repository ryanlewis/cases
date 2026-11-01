//! Stats: how many cases were opened and answered on each of the last few
//! days, and the median time the human took to answer.
//!
//! An inbox works these out from the cases it already read, so they need no
//! store call of their own, and a forwarded store gives them as a local one
//! does. Days are UTC days.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use crate::case::Case;
use crate::names::{Kind, State};
use crate::record::Record;
use crate::time::Time;

/// Seconds in a day.
const DAY: i64 = 86_400;
/// Nanoseconds in a second.
const NANOS: i128 = 1_000_000_000;

/// What [`stats`] gives: per-day counts, oldest day first and today last,
/// and the median time to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    opened: Vec<u32>,
    answered: Vec<u32>,
    median: Option<Duration>,
}

impl Stats {
    /// How many cases were opened on each day, oldest first.
    #[must_use]
    pub fn opened(&self) -> &[u32] {
        &self.opened
    }

    /// How many answers the human gave on each day, oldest first.
    #[must_use]
    pub fn answered(&self) -> &[u32] {
        &self.answered
    }

    /// The median time to answer, in whole seconds, of the answers given in
    /// the days counted. `None` when there were none with a time to answer.
    #[must_use]
    pub fn median(&self) -> Option<Duration> {
        self.median
    }
}

/// The stats of `cases` over the `days` UTC days up to and including the
/// day of `now`, as `spec/vectors/README.md` says under "Stats".
///
/// - A case counts as opened on the day of its `opened_at`.
/// - Each answer counts on the day of its `answered_at`, so a case answered,
///   reopened and answered again counts twice. A drop is not an answer.
/// - An answer's time to answer runs from when the case was last asked: its
///   `opened_at`, or the time of the note or amend that last reopened it.
///   The median is over the answers counted; with an even number of them it
///   is the mean of the middle two. It is cut to whole seconds.
/// - Messages are left out: the human opens them, and does not answer them.
///
/// An event without a time is not counted. Nor is a time to answer below
/// zero, as a clock behind can give.
#[must_use]
pub fn stats<'a, I>(cases: I, now: Time, days: u16) -> Stats
where
    I: IntoIterator<Item = &'a Case>,
{
    let len = usize::from(days);
    let mut opened = vec![0u32; len];
    let mut answered = vec![0u32; len];
    let mut waits: Vec<i128> = Vec::new();
    let first = day(now) - i64::from(days) + 1;
    let slot = |at: Time| usize::try_from(day(at) - first).ok().filter(|i| *i < len);
    for case in cases {
        if case.kind() == Kind::Message {
            continue;
        }
        if let Some(i) = case.opened_at().and_then(slot) {
            opened[i] = opened[i].saturating_add(1);
        }
        let events = case.events();
        let mut asked = case.opened_at();
        for (n, event) in events.iter().enumerate() {
            let after = events
                .get(n + 1)
                .and_then(|e| e.from)
                .unwrap_or(case.state());
            match &event.record {
                Record::Answer(answer) if !answer.drop => {
                    let Some(at) = answer.answered_at else {
                        continue;
                    };
                    let Some(i) = slot(at) else { continue };
                    answered[i] = answered[i].saturating_add(1);
                    if let Some(asked) = asked {
                        let wait = nanos(at) - nanos(asked);
                        if wait >= 0 {
                            waits.push(wait);
                        }
                    }
                }
                Record::Note(_) | Record::Amend(_)
                    if matches!(event.from, Some(State::Answered | State::PickedUp))
                        && after == State::Open =>
                {
                    asked = event.at();
                }
                _ => {}
            }
        }
    }
    Stats {
        opened,
        answered,
        median: median(&mut waits),
    }
}

/// The UTC day `at` falls on, as days since 1970-01-01.
fn day(at: Time) -> i64 {
    at.unix_secs().div_euclid(DAY)
}

/// `at` in nanoseconds since 1970.
fn nanos(at: Time) -> i128 {
    i128::from(at.unix_secs()) * NANOS + i128::from(at.subsec_nanos())
}

/// The median of `waits`, in nanoseconds, cut to whole seconds.
fn median(waits: &mut [i128]) -> Option<Duration> {
    waits.sort_unstable();
    let mid = waits.len() / 2;
    let median = if waits.len() % 2 == 1 {
        *waits.get(mid)?
    } else {
        i128::midpoint(*waits.get(mid.checked_sub(1)?)?, *waits.get(mid)?)
    };
    u64::try_from(median / NANOS).ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_of_none_is_none() {
        assert_eq!(median(&mut []), None);
    }

    #[test]
    fn the_median_of_an_odd_count_is_the_middle_one() {
        assert_eq!(
            median(&mut [3 * NANOS, NANOS, 2 * NANOS]),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two_cut_to_seconds() {
        assert_eq!(
            median(&mut [4 * NANOS, NANOS, 2 * NANOS, 100 * NANOS]),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            median(&mut [NANOS, 2 * NANOS]),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn a_day_before_1970_is_below_zero() {
        let before = Time::from_unix(-1, 0).unwrap();
        assert_eq!(day(before), -1);
    }
}
