//! `cases resume`: opens a parked case again (HU-04). Only the human
//! resumes (HU-05).

use cases_core::{Author, Record, Resume};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::Ctx;
use crate::error::Error;

/// The arguments of `cases resume`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id, or any part of it that names one case.
    id: String,

    /// Record the resume as written by NAME. Default: the config file's
    /// name, then the login name.
    #[arg(long = "as", value_name = "NAME")]
    as_: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Resumes the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx.find_case(&args.id)?;
    let writer = ctx.human(args.as_.as_deref());
    let record = Record::Resume(Resume::default());
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
