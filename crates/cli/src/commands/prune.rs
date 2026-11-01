//! `cases prune`: moves closed and withdrawn cases out of the store, into
//! its archive or deleted (ST-13).

use std::io::Write as _;
use std::thread;
use std::time::Duration;

use cases_core::{State, Store};

use crate::commands::list::older_than;
use crate::commands::sweep::WRITE_PAUSE;
use crate::ctx::Ctx;
use crate::duration;
use crate::error::Error;
use crate::output;

/// How old a case's last event must be when neither `--age` nor the config
/// file says: 30 days.
const DEFAULT_AGE: Duration = Duration::from_hours(720);

/// The arguments of `cases prune`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Only cases whose last event is older than this, such as 720h. 0 means
    /// any age. Default: the config file's prune-age, then 720h.
    #[arg(long, value_name = "DURATION", value_parser = duration::parse)]
    age: Option<Duration>,

    /// Only cases in these states: closed, withdrawn or both. Repeat or
    /// comma-separate.
    #[arg(
        long = "state",
        value_name = "STATE",
        value_delimiter = ',',
        default_value = "closed,withdrawn"
    )]
    states: Vec<String>,

    /// Delete the cases instead of moving them to the archive.
    #[arg(long)]
    delete: bool,

    /// Prune the cases. Without it, prune only prints what it would prune.
    #[arg(short, long)]
    yes: bool,
}

/// Archives, or deletes, each matching case in a transaction of its own, at
/// the revision this run read: a case written to since is refused and left,
/// as is one whose key the archive has already, and one that does not fold.
/// Prune goes on with the rest, then names the cases it did not prune and
/// exits 1.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let states = args
        .states
        .iter()
        .map(|state| {
            let state = state.parse::<State>()?;
            if matches!(state, State::Closed | State::Withdrawn) {
                Ok(state)
            } else {
                Err(Error::new(format!(
                    "--state: prune takes closed or withdrawn cases, not {state}"
                )))
            }
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let age = match (args.age, ctx.config.get("prune-age")) {
        (Some(age), _) => age,
        (None, Some(age)) => duration::parse(age)
            .map_err(|error| Error::new(format!("the config file's prune-age: {error}")))?,
        (None, None) => DEFAULT_AGE,
    };

    let now = output::now()?;
    let (cases, unfolded) = ctx.cases_and_unfolded()?;
    let prune: Vec<_> = cases
        .into_iter()
        .filter(|(_, case)| states.contains(&case.state()) && older_than(case, age, now))
        .collect();

    let verb = if args.delete { "deleted" } else { "archived" };
    if !args.yes {
        for (key, case) in &prune {
            writeln!(ctx.out, "{key} {}, would be {verb}", case.state())?;
        }
        if !prune.is_empty() {
            let done = if args.delete {
                "deleted".to_owned()
            } else {
                format!("moved to the archive in {}", ctx.target)
            };
            eprintln!(
                "{} would be {done}. Pass --yes to prune them.",
                output::plural(prune.len() as u64, "case")
            );
        }
        return Ok(());
    }

    let mut failed = Vec::new();
    for (key, case) in &prune {
        let revision = Some(case.revision());
        let removed = if args.delete {
            ctx.store.delete(key, revision)
        } else {
            ctx.store.archive(key, revision)
        };
        thread::sleep(WRITE_PAUSE);
        match removed {
            Ok(()) => writeln!(ctx.out, "{key} {verb}")?,
            Err(error) => failed.push(format!("{key}: {}", Error::from(error))),
        }
    }
    // A case that does not fold may be one to prune; it is left, and named,
    // and prune fails, as for a case it could not move (ST-13).
    for (key, why) in unfolded {
        failed.push(format!("{key}: cannot be loaded, and was left: {why}"));
    }
    if failed.is_empty() {
        return Ok(());
    }
    let was = if failed.len() == 1 { "was" } else { "were" };
    Err(Error::new(format!(
        "{} {was} not pruned:\n  {}",
        output::plural(failed.len() as u64, "case"),
        failed.join("\n  ")
    )))
}
