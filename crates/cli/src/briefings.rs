//! Picked-up briefings that nobody closed: once the pickup is older than the
//! config file's `briefing-close-after`, 7 days unless it says otherwise,
//! the briefing is closed as expired, with an outcome that says why (AG-19).
//! serve does it each hour, and `cases sweep --stale-briefings` does it by
//! hand.
//!
//! An agent closes its own briefing once it has acted on the answer. One
//! that never does leaves the briefing in the inbox's briefings tab, where it
//! stays so that the human can take the answer back; after the setting's
//! time nobody will, and it goes. The close is the core's `expired` close:
//! it says the agent did not judge the outcome, and it is written under the
//! agent's role by the unnamed agent, so it wakes no wait, as the human's
//! withdraw would, which tells a waiting agent its answer was taken back.

use std::thread;
use std::time::Duration;

use cases_core::{Author, Case, Close, Kind, Record, State, Time};
use cases_store_sqlite::{Key, Writer};

use crate::commands::sweep::WRITE_PAUSE;
use crate::config::Config;
use crate::duration;
use crate::error::Error;
use crate::output;

/// The config file's key.
pub(crate) const KEY: &str = "briefing-close-after";

/// How long a briefing stays picked up when the config file does not say.
pub(crate) const DEFAULT: &str = "7d";

/// The value of [`KEY`] that turns it off, as `0` does too.
const OFF: &str = "off";

/// How long a briefing stays picked up before it is closed, from the
/// config file's [`KEY`], else [`DEFAULT`]; none when it is `off` or `0`.
///
/// # Errors
///
/// When the file's value is neither a duration nor `off`.
pub(crate) fn close_after(config: &Config) -> Result<Option<Duration>, Error> {
    let text = config.get(KEY).unwrap_or(DEFAULT).trim();
    if text == OFF {
        return Ok(None);
    }
    let after = duration::parse(text)
        .map_err(|error| Error::new(format!("the config file's {KEY}: {error}, or {OFF}")))?;
    Ok((!after.is_zero()).then_some(after))
}

/// The briefings among `cases` that are picked up, and were picked up at
/// least `after` before `now`, with the revision each was read at. A pickup
/// with no time, or one dated after `now`, is not old.
pub(crate) fn stale(cases: &[(Key, Case)], after: Duration, now: Time) -> Vec<(Key, u64)> {
    cases
        .iter()
        .filter(|(_, case)| case.kind() == Kind::Briefing && case.state() == State::PickedUp)
        .filter(|(_, case)| {
            case.pickup()
                .and_then(|pickup| pickup.picked_up_at)
                .and_then(|at| output::elapsed(at, now))
                .is_some_and(|elapsed| elapsed >= after)
        })
        .map(|(key, case)| (*key, case.revision()))
        .collect()
}

/// The outcome each close records, such as "closed after 7 days picked up".
pub(crate) fn reason(after: Duration) -> String {
    format!("closed after {} picked up", span(after))
}

/// `after` for people: in days, hours, minutes or seconds, the largest unit
/// it is a whole number of, else in seconds with a fraction.
fn span(after: Duration) -> String {
    let secs = after.as_secs();
    if after.subsec_nanos() != 0 {
        return format!("{} seconds", after.as_secs_f64());
    }
    for (unit, word) in [(86_400, "day"), (3_600, "hour"), (60, "minute")] {
        if secs > 0 && secs.is_multiple_of(unit) {
            return output::plural(secs / unit, word);
        }
    }
    output::plural(secs, "second")
}

/// The author of the closes: the agent's role, which every close has.
pub(crate) const AUTHOR: Author = Author::Agent;

/// Who writes the closes: the unnamed agent, since no agent did.
pub(crate) fn writer() -> Writer {
    Writer::new(None)
}

/// Closes each briefing in `stale` as expired with `write`, given the case,
/// the close, to write as [`AUTHOR`] by [`writer`], with the outcome for `after`, and the revision it
/// was read at, pausing after each write so that a writer elsewhere gets the
/// store's lock between them. Gives each case after its close, or why it was
/// not closed, such as a write since that moved the revision on.
pub(crate) fn close_each<E>(
    stale: &[(Key, u64)],
    after: Duration,
    mut write: impl FnMut(Key, &Record, u64) -> Result<Case, E>,
) -> Vec<(Key, Result<Case, E>)> {
    let mut close = Close::default();
    close.outcome = reason(after);
    close.expired = true;
    let record = Record::Close(close);
    stale
        .iter()
        .map(|&(key, revision)| {
            let written = write(key, &record, revision);
            thread::sleep(WRITE_PAUSE);
            (key, written)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Store};
    use cases_store_sqlite::Db;

    use super::*;

    fn append(author: &str, event: &str, data: &str) -> Append {
        Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        }
    }

    fn time(text: &str) -> Time {
        text.parse().unwrap()
    }

    const BRIEFING: &str = r#"{"kind": "briefing", "urgency": "today", "title": "T", "body": "B"}"#;

    /// A store with a briefing picked up at `at`, and the case's key.
    fn picked_up(db: &Db, open: &str, at: &str) -> Key {
        let unnamed = Writer::new(None);
        let (key, _) = db.create(&append("agent", "open", open), &unnamed).unwrap();
        let answer = if open == BRIEFING {
            r#"{"briefing": "accept"}"#
        } else {
            r#"{"ack": true}"#
        };
        db.append(&key, &append("human", "answer", answer), &unnamed, None)
            .unwrap();
        let pickup = format!(r#"{{"picked_up_at": "{at}"}}"#);
        db.append(&key, &append("agent", "pickup", &pickup), &unnamed, None)
            .unwrap();
        key
    }

    #[test]
    fn only_a_briefing_picked_up_long_enough_ago_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let unnamed = Writer::new(None);
        let old = picked_up(&db, BRIEFING, "2026-10-01T09:00:00Z");
        let exactly = picked_up(&db, BRIEFING, "2026-10-08T09:00:00Z");
        picked_up(&db, BRIEFING, "2026-10-08T09:00:01Z");
        // In the future, as a clock that is behind could leave it.
        picked_up(&db, BRIEFING, "2026-10-09T09:00:00Z");
        let fyi = r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#;
        picked_up(&db, fyi, "2026-10-01T09:00:00Z");
        let (answered, _) = db
            .create(&append("agent", "open", BRIEFING), &unnamed)
            .unwrap();
        db.append(
            &answered,
            &append(
                "human",
                "answer",
                r#"{"briefing": "accept", "answered_at": "2026-10-01T09:00:00Z"}"#,
            ),
            &unnamed,
            None,
        )
        .unwrap();
        let cases: Vec<_> = db
            .list()
            .unwrap()
            .into_iter()
            .map(|(key, rows)| (key, Case::fold(&rows).unwrap()))
            .collect();
        let now = time("2026-10-15T09:00:00Z");
        assert_eq!(
            stale(&cases, Duration::from_hours(168), now),
            [(old, 3), (exactly, 3)]
        );
        assert_eq!(stale(&cases, Duration::from_hours(24 * 30), now), []);
    }

    #[test]
    fn the_setting_is_a_duration_or_off() {
        let config = |text: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, text).unwrap();
            crate::config::load(Some(path.to_str().unwrap()))
        };
        assert_eq!(
            close_after(&Config::default()).unwrap(),
            Some(Duration::from_hours(168))
        );
        for (text, want) in [
            ("\"36h\"", Some(Duration::from_hours(36))),
            ("\"7d\"", Some(Duration::from_hours(168))),
            ("\"14d\"", Some(Duration::from_hours(336))),
            ("\"off\"", None),
            ("\"0\"", None),
            ("\"0s\"", None),
        ] {
            let config = config(&format!("{KEY} = {text}\n"));
            assert_eq!(close_after(&config).unwrap(), want, "{text}");
        }
        let error = close_after(&config(&format!("{KEY} = \"2w\"\n"))).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the config file's briefing-close-after: \"2w\" is not a duration, such as 30m, \
             72h, 1h30m or 7d, or off"
        );
    }

    #[test]
    fn the_reason_names_the_time_in_its_largest_whole_unit() {
        for (after, want) in [
            (Duration::from_hours(168), "closed after 7 days picked up"),
            (Duration::from_hours(24), "closed after 1 day picked up"),
            (Duration::from_hours(36), "closed after 36 hours picked up"),
            (Duration::from_mins(90), "closed after 90 minutes picked up"),
            (Duration::from_secs(61), "closed after 61 seconds picked up"),
            (
                Duration::from_millis(1500),
                "closed after 1.5 seconds picked up",
            ),
            (
                Duration::from_millis(500),
                "closed after 0.5 seconds picked up",
            ),
        ] {
            assert_eq!(reason(after), want, "{after:?}");
        }
    }

    #[test]
    fn each_stale_briefing_is_closed_expired_by_the_unnamed_agent() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let first = picked_up(&db, BRIEFING, "2026-10-01T09:00:00Z");
        let second = picked_up(&db, BRIEFING, "2026-10-01T09:00:00Z");
        let done = close_each(
            &[(first, 3), (second, 2)],
            Duration::from_hours(168),
            |key, record, revision| {
                let append = Append::new(AUTHOR, record);
                db.append(&key, &append, &writer(), Some(revision))
            },
        );
        let case = done[0].1.as_ref().unwrap();
        assert_eq!(case.state(), State::Closed);
        assert_eq!(case.events().last().unwrap().author, Author::Agent);
        let close = case.close().unwrap();
        assert_eq!(close.outcome, "closed after 7 days picked up");
        assert!(close.expired);
        // Read at a revision it is no longer at, it is left.
        assert!(done[1].1.is_err());
        assert_eq!(
            Case::fold(&db.rows(&second).unwrap().unwrap())
                .unwrap()
                .state(),
            State::PickedUp
        );
    }
}
