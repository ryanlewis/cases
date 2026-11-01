//! `cases close`: records the outcome of a picked-up case (AG-10).

use std::path::PathBuf;

use cases_core::{Author, Close, Record};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::text;

/// The arguments of `cases close`.
#[derive(Debug, clap::Args)]
#[group(id = "close_outcome", required = true, args = ["outcome", "outcome_file"])]
pub(crate) struct Args {
    /// Case id.
    id: String,

    /// The outcome as one line of text.
    #[arg(long, value_name = "TEXT", conflicts_with = "outcome_file")]
    outcome: Option<String>,

    /// Markdown outcome. - reads stdin.
    #[arg(long, value_name = "FILE")]
    outcome_file: Option<PathBuf>,

    /// A link to evidence of the outcome. Repeatable.
    #[arg(long = "link", value_name = "URL")]
    links: Vec<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Closes the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx::exact(&args.id)?;
    let mut close = Close::default();
    close.outcome = text::read(
        "outcome",
        args.outcome.as_deref(),
        args.outcome_file.as_deref(),
    )?
    .unwrap_or_default();
    if close.outcome.trim().is_empty() {
        return Err(Error::new("the outcome is empty"));
    }
    close.links.clone_from(&args.links);
    let writer = write::agent(ctx, key)?;
    let record = Record::Close(close);
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
