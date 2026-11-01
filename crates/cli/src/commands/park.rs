//! `cases park`: puts an open stuck case aside until someone resumes it
//! (HU-03).

use cases_core::{Author, Park, Record};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::Ctx;
use crate::error::Error;

/// The arguments of `cases park`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id, or any part of it that names one case.
    id: String,

    /// A note to the agent, such as when to ask again.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,

    /// Record the park as written by NAME. Default: the config file's name,
    /// then the login name.
    #[arg(long = "as", value_name = "NAME")]
    as_: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Parks the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx.find_case(&args.id)?;
    let mut park = Park::default();
    park.note = args.note.clone().unwrap_or_default();
    let writer = ctx.human(args.as_.as_deref());
    let record = Record::Park(park);
    let case = write::append(
        ctx,
        key,
        Author::Human,
        &record,
        &writer,
        args.write.revision,
    )?;
    write::print(ctx, key, &case, args.write.json)
}
