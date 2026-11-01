//! `cases pickup`: records that the answer has been read (AG-08).

use cases_core::{Author, Kind, Pickup, Record};
use cases_store_sqlite::Writer;

use crate::commands::write::{self, WriteArgs};
use crate::ctx::{self, Ctx};
use crate::error::Error;

/// The arguments of `cases pickup`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id.
    id: String,

    /// Who picked it up, such as the agent session's name.
    #[arg(long, value_name = "NAME")]
    by: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Picks up the answer.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx::exact(&args.id)?;
    let mut pickup = Pickup::default();
    pickup.by = args.by.clone().unwrap_or_default();
    // A message is picked up by the session that takes it, by its --by. A
    // blank --by, as an unset variable gives, is none, as for wait.
    let case = ctx.case(key)?;
    let writer = if case.kind() == Kind::Message && !pickup.by.trim().is_empty() {
        Writer::new(Some(&pickup.by))
    } else {
        write::agent_of(&case)
    };
    let record = Record::Pickup(pickup);
    let case = write::append(
        ctx,
        key,
        Author::Agent,
        &record,
        &writer,
        args.write.revision,
    )?;
    write::print(ctx, key, &case, args.write.json)
}
