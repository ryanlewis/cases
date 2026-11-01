//! What the commands that write to one case share: the revision guard
//! (AG-12), who writes (AG-01, HU-06), and what a write prints (CL-02).

use std::io::Write as _;

use cases_core::{Append, Author, Case, Category, Kind, Record, State, Store, StoreError};
use cases_store_sqlite::{Key, Writer};

use crate::ctx::Ctx;
use crate::error::{Error, Status};
use crate::output::{self, Written};

/// The flags every write to one case takes.
#[derive(Debug, clap::Args)]
pub(crate) struct WriteArgs {
    /// Refuse the write if the case's revision (from show --json) is no
    /// longer N. Exit 4 when it is not.
    #[arg(long, value_name = "N")]
    pub(crate) revision: Option<u64>,

    /// Print the id, state and new revision as JSON.
    #[arg(short, long)]
    pub(crate) json: bool,
}

/// The writer of the agent's events on the case `key`: the worker the case
/// was opened with, or the unnamed agent when it has none.
///
/// # Errors
///
/// When the store has no such case, or it does not fold.
pub(crate) fn agent(ctx: &Ctx, key: Key) -> Result<Writer, Error> {
    Ok(agent_of(&ctx.case(key)?))
}

/// The writer of the agent's events on `case`, read already: as [`agent`]
/// gives it. On a message that is the session that picked it up, by its
/// pickup's `by`, and before a pickup, or with no `by` or a blank one, the
/// worker it went to (AG-16).
pub(crate) fn agent_of(case: &Case) -> Writer {
    if case.kind() == Kind::Message
        && let Some(pickup) = case.pickup()
        && !pickup.by.trim().is_empty()
    {
        return Writer::new(Some(&pickup.by));
    }
    Writer::new(Some(case.worker()))
}

/// Writes `record` to the case `key` as `author`, by `writer`, at
/// `at_revision` when given, and gives the case after it. A write refused as
/// stale says the state the case is in now, so that the reader can tell what
/// changed (AG-12).
///
/// # Errors
///
/// When the store has no such case, the core refuses the event (exit 3 for
/// a transition, 4 for a stale write), or the store fails.
pub(crate) fn append(
    ctx: &Ctx,
    key: Key,
    author: Author,
    record: &Record,
    writer: &Writer,
    at_revision: Option<u64>,
) -> Result<Case, Error> {
    match ctx
        .store
        .append(&key, &Append::new(author, record), writer, at_revision)
    {
        Ok(case) => Ok(case),
        Err(StoreError::Refused(error)) if error.category() == Category::Stale => {
            let now = ctx
                .case(key)
                .map(|case| format!("; it is now {}", case.state()))
                .unwrap_or_default();
            Err(Error::with_status(Status::Stale, format!("{error}{now}")))
        }
        Err(StoreError::NoCase) => Err(Error::new(format!("no case {key}"))),
        Err(error) => Err(error.into()),
    }
}

/// `error`, the refusal of a write to the case `key` without `--reopen`, with
/// a hint to pass it when the case is answered or picked up: a note or an
/// amend reopens such a case only when it says so.
pub(crate) fn hint_reopen(ctx: &Ctx, key: Key, reopen: bool, error: Error) -> Error {
    if reopen || error.status() != Status::Transition {
        return error;
    }
    match ctx.case(key) {
        Ok(case) if matches!(case.state(), State::Answered | State::PickedUp) => {
            Error::with_status(
                Status::Transition,
                format!(
                    "{error}: pass --reopen to reopen it, which clears the answer and asks \
                     the human again"
                ),
            )
        }
        _ => error,
    }
}

/// Prints what a write did: the id and the state, or with `--json` the id,
/// state and revision.
///
/// # Errors
///
/// When stdout cannot be written.
pub(crate) fn print(ctx: &mut Ctx, key: Key, case: &Case, json: bool) -> Result<(), Error> {
    if json {
        output::json(&mut ctx.out, &Written::new(key, case))
    } else {
        writeln!(ctx.out, "{key} {}", case.state())?;
        Ok(())
    }
}
