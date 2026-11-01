//! `cases note`: adds a follow-up to the thread of an open case, or with
//! `--reopen`, reopens an answered or picked-up case with it (AG-09).

use std::path::PathBuf;

use cases_core::{Author, Note, Record};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::text;

/// The arguments of `cases note`.
#[derive(Debug, clap::Args)]
#[group(id = "note_body", required = true, args = ["body", "body_file"])]
pub(crate) struct Args {
    /// Case id.
    id: String,

    /// The follow-up as one line of text.
    #[arg(long, value_name = "TEXT", conflicts_with = "body_file")]
    body: Option<String>,

    /// Markdown follow-up. - reads stdin.
    #[arg(long, value_name = "FILE")]
    body_file: Option<PathBuf>,

    /// Reopen an answered or picked-up case: its answer is cleared, and the
    /// human answers again. Without it, a note on such a case is refused
    /// (exit 3); with it, a note on an open case is refused.
    #[arg(long)]
    reopen: bool,

    #[command(flatten)]
    write: WriteArgs,
}

/// Adds the note.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx::exact(&args.id)?;
    let mut note = Note::default();
    note.body =
        text::read("body", args.body.as_deref(), args.body_file.as_deref())?.unwrap_or_default();
    if note.body.trim().is_empty() {
        return Err(Error::new("the note is empty"));
    }
    note.reopen = args.reopen;
    let writer = write::agent(ctx, key)?;
    let record = Record::Note(note);
    let case = write::append(
        ctx,
        key,
        Author::Agent,
        &record,
        &writer,
        args.write.revision,
    )
    .map_err(|error| write::hint_reopen(ctx, key, args.reopen, error))?;
    write::print(ctx, key, &case, args.write.json)
}
