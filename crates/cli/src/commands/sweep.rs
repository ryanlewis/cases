//! `cases sweep`: withdraws the open cases that match a filter, to clear
//! the inbox, and with `--in-flight` the answered and picked-up ones too
//! (ST-14). With `--stale-briefings` it closes, as expired, the briefings
//! picked up longer ago than the config file's `briefing-close-after`, as
//! serve does each hour (AG-19).

use std::io::Write as _;
use std::thread;
use std::time::Duration;

use cases_core::{Author, Case, Record, State, Withdraw};
use cases_store_sqlite::{Key, Writer};

use crate::briefings;
use crate::commands::list::{Filter, older_than};
use crate::commands::write;
use crate::ctx::Ctx;
use crate::duration;
use crate::error::Error;
use crate::output;
use crate::store::Target;

/// How long sweep, prune and `wait --pickup` wait after each write, so that
/// a writer elsewhere, such as an agent's note, gets the store's lock between
/// them. A writer waiting for the lock tries again after growing pauses, and
/// without a gap between writes it can miss every moment the lock is free,
/// and fail (ST-09).
pub(crate) const WRITE_PAUSE: Duration = Duration::from_millis(1);

/// The arguments of `cases sweep`.
#[derive(Debug, clap::Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a flag of its own, as clap takes them"
)]
pub(crate) struct Args {
    /// The reason recorded on each withdraw.
    #[arg(long, value_name = "TEXT", default_value = "swept")]
    reason: String,

    /// Only cases whose last event is older than this, such as 72h. 0 means
    /// any age.
    #[arg(long, value_name = "DURATION", value_parser = duration::parse, default_value = "0")]
    older_than: Duration,

    #[command(flatten)]
    filter: Filter,

    /// Also withdraw the answered and picked-up cases that match: cases an
    /// agent has, such as those a dead session left. An agent still at work
    /// on one has its close refused.
    #[arg(long)]
    in_flight: bool,

    /// Close instead, as expired, the briefings picked up longer ago than
    /// the config file's briefing-close-after (default 7d), which serve does
    /// each hour. The filter narrows them.
    #[arg(long, conflicts_with_all = ["reason", "older_than", "in_flight", "as_"])]
    stale_briefings: bool,

    /// Withdraw the cases. Without it, sweep only prints what it would
    /// withdraw.
    #[arg(short, long)]
    yes: bool,

    /// Record the withdraws as written by NAME. Default: the config file's
    /// name, then the login name.
    #[arg(long = "as", value_name = "NAME")]
    as_: Option<String>,

    /// Accepted for the reflex of typing it: sweep only prints unless given
    /// --yes, so this does nothing.
    #[arg(long, hide = true)]
    #[allow(dead_code, reason = "clap needs the field; sweep never reads it")]
    dry_run: bool,
}

/// Withdraws the open cases that match, as the human, and with `--in-flight`
/// the answered and picked-up ones too. A parked case that matches is listed
/// and left, and so are an answered and a picked-up one without
/// `--in-flight`; a dry run also hints that `--in-flight` would take them.
/// Each withdraw is made at the revision this run read, so a case written to
/// since is refused and left; sweep goes on with the rest, then names the
/// cases it did not withdraw and exits 1.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    if args.stale_briefings {
        return stale_briefings(args, ctx);
    }
    let now = output::now()?;
    let mut sweep = Vec::new();
    let mut left_with_agent = 0u64;
    for (key, case) in ctx.cases()? {
        if !args.filter.matches(&case) || !older_than(&case, args.older_than, now) {
            continue;
        }
        match case.state() {
            State::Open => sweep.push((key, case)),
            State::Answered | State::PickedUp if args.in_flight => sweep.push((key, case)),
            state @ (State::Answered | State::PickedUp | State::Parked) => {
                if matches!(state, State::Answered | State::PickedUp) {
                    left_with_agent += 1;
                }
                writeln!(ctx.out, "{key} {state}{}, left", pickup_by(&case))?;
            }
            _ => {}
        }
    }

    if !args.yes {
        for (key, case) in &sweep {
            // A case an agent has says so, and who picked it up, which may be
            // another session than the worker that opened it: withdrawing it
            // may cut off work.
            match case.state() {
                State::Open => writeln!(ctx.out, "{key} would be withdrawn: {}", case.title())?,
                state => writeln!(
                    ctx.out,
                    "{key} {state}{}, would be withdrawn: {}",
                    pickup_by(case),
                    case.title()
                )?,
            }
        }
        if !sweep.is_empty() {
            eprintln!(
                "{} would be withdrawn. Pass --yes to withdraw them.",
                output::plural(sweep.len() as u64, "case")
            );
        }
        if left_with_agent > 0 {
            let was = if left_with_agent == 1 { "was" } else { "were" };
            eprintln!(
                "{} with an agent {was} left; --in-flight withdraws them too.",
                output::plural(left_with_agent, "case")
            );
        }
        return Ok(());
    }

    let writer = ctx.human(args.as_.as_deref());
    let read: Vec<(Key, u64)> = sweep
        .iter()
        .map(|(key, case)| (*key, case.revision()))
        .collect();
    let written = withdraw_each(ctx, &read, &args.reason, &writer);
    report(ctx, written, "withdrawn")
}

/// Prints the state of each case written, and fails naming the cases that
/// were not `done`, such as "withdrawn".
fn report(
    ctx: &mut Ctx,
    written: Vec<(Key, Result<Case, Error>)>,
    done: &str,
) -> Result<(), Error> {
    let mut failed = Vec::new();
    for (key, written) in written {
        match written {
            Ok(case) => writeln!(ctx.out, "{key} {}", case.state())?,
            Err(error) => failed.push(format!("{key}: {error}")),
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    let was = if failed.len() == 1 { "was" } else { "were" };
    Err(Error::new(format!(
        "{} {was} not {done}:\n  {}",
        output::plural(failed.len() as u64, "case"),
        failed.join("\n  ")
    )))
}

/// `sweep --stale-briefings`: closes as expired, as serve does, the
/// briefings that match and were picked up longer ago than the config
/// file's `briefing-close-after`. Without `--yes` it only prints them. When
/// the setting is off it says so, and does nothing. A forwarded store is
/// refused: the setting is the far side's.
fn stale_briefings(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    // The age is the far side's config file's, which this one cannot read.
    if let Target::Forward { forward } = &ctx.target {
        return Err(Error::new(format!(
            "sweep --stale-briefings runs where the store is: serve on {forward} closes stale \
             briefings each hour, or run cases sweep --stale-briefings there"
        )));
    }
    let Some(after) = briefings::close_after(&ctx.config)? else {
        eprintln!("{} is off: no briefing is closed.", briefings::KEY);
        return Ok(());
    };
    let now = output::now()?;
    let cases: Vec<_> = ctx
        .cases()?
        .into_iter()
        .filter(|(_, case)| args.filter.matches(case))
        .collect();
    let stale = briefings::stale(&cases, after, now);
    if !args.yes {
        for (key, case) in cases
            .iter()
            .filter(|(key, _)| stale.iter().any(|s| s.0 == *key))
        {
            writeln!(
                ctx.out,
                "{key} pickedup{}, would be closed: {}",
                pickup_by(case),
                case.title()
            )?;
        }
        if !stale.is_empty() {
            eprintln!(
                "{} would be closed. Pass --yes to close them.",
                output::plural(stale.len() as u64, "briefing")
            );
        }
        return Ok(());
    }
    let writer = briefings::writer();
    let written = briefings::close_each(&stale, after, |key, record, revision| {
        write::append(ctx, key, briefings::AUTHOR, record, &writer, Some(revision))
    });
    report(ctx, written, "closed")
}

/// Who has `case`, for people: " by NAME", or empty when it has nobody or an
/// unnamed pickup.
fn pickup_by(case: &Case) -> String {
    case.pickup()
        .filter(|pickup| !pickup.by.is_empty())
        .map(|pickup| format!(" by {}", pickup.by))
        .unwrap_or_default()
}

/// Withdraws each case in `read` as the human `writer`, with `reason`, at the
/// revision it was read at, pausing after each write. Gives each case after
/// its withdraw, or why it was not withdrawn.
fn withdraw_each(
    ctx: &Ctx,
    read: &[(Key, u64)],
    reason: &str,
    writer: &Writer,
) -> Vec<(Key, Result<Case, Error>)> {
    read.iter()
        .map(|&(key, revision)| {
            let mut withdraw = Withdraw::default();
            reason.clone_into(&mut withdraw.reason);
            let record = Record::Withdraw(withdraw);
            let written = write::append(ctx, key, Author::Human, &record, writer, Some(revision));
            thread::sleep(WRITE_PAUSE);
            (key, written)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io;

    use cases_core::{Append, Store};
    use cases_store_sqlite::Db;

    use super::*;
    use crate::config::Config;
    use crate::store::{AnyStore, Target};

    fn event(author: &str, event: &str, data: &str) -> Append {
        Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        }
    }

    /// A case written to after sweep read it is left as it is now, and the
    /// error names the state it is in, whatever the write was: an answer, an
    /// amend, or an answer and a note that opens it again. The others are
    /// withdrawn.
    #[test]
    fn a_case_changed_since_it_was_read_is_left() {
        /// Events written after the read, as author, event and record.
        type Writes = &'static [(&'static str, &'static str, &'static str)];
        let changes: [(Writes, State); 3] = [
            (&[("human", "answer", r#"{"ack": true}"#)], State::Answered),
            (
                &[("agent", "amend", r#"{"links": ["https://x"]}"#)],
                State::Open,
            ),
            (
                &[
                    ("human", "answer", r#"{"ack": true}"#),
                    ("agent", "note", r#"{"body": "More?", "reopen": true}"#),
                ],
                State::Open,
            ),
        ];
        for (writes, now) in changes {
            let dir = tempfile::tempdir().unwrap();
            let store = dir.path().join("cases.db");
            let db = Db::new(&store);
            let open = event(
                "agent",
                "open",
                r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
            );
            let unnamed = Writer::new(None);
            let (changed, _) = db.create(&open, &unnamed).unwrap();
            let (fine, _) = db.create(&open, &unnamed).unwrap();
            let ctx = Ctx {
                store: AnyStore::Local(db),
                target: Target::Local { path: store },
                config: Config::default(),
                store_flag: None,
                out: io::stdout().lock(),
            };
            for (author, name, data) in writes {
                ctx.store
                    .append(&changed, &event(author, name, data), &unnamed, None)
                    .unwrap();
            }
            let done = withdraw_each(&ctx, &[(changed, 1), (fine, 1)], "swept", &unnamed);
            let Err(error) = &done[0].1 else {
                panic!("withdrew a changed case")
            };
            assert!(
                error.to_string().ends_with(&format!("; it is now {now}")),
                "{error}"
            );
            assert_eq!(ctx.case(changed).unwrap().state(), now);
            assert_eq!(done[1].1.as_ref().unwrap().state(), State::Withdrawn);
        }
    }
}
