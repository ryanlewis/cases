//! `cases answer`: answers an open case, in the shape its kind asks for
//! (HU-01), or drops it (HU-02).

use std::path::PathBuf;

use cases_core::{Answer, Author, Kind, Record, RowVerdict};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::Ctx;
use crate::error::Error;
use crate::text;

/// The arguments of `cases answer`. One response at most; the core checks
/// that it fits the case's kind.
#[derive(Debug, clap::Args)]
#[group(id = "response", multiple = false, args = [
    "option", "other", "rows", "accept", "changes", "send_back", "text", "text_file", "drop",
    "ack",
])]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a response flag, and clap takes one at most"
)]
pub(crate) struct Args {
    /// Case id, or any part of it that names one case.
    id: String,

    /// decision: choose option N, from 1.
    #[arg(long, value_name = "N")]
    option: Option<i64>,

    /// decision: other, see note (needs --note).
    #[arg(long)]
    other: bool,

    /// approval: a verdict per row, as id=approve, id=hold or id=reject
    /// (approve: run it; hold: not now, ask again later; reject: never run
    /// it), optionally with a note, as id=verdict:note. Repeat for every
    /// row.
    #[arg(long = "row", value_name = "ID=VERDICT")]
    rows: Vec<String>,

    /// signoff or briefing: accept.
    #[arg(long)]
    accept: bool,

    /// signoff: ask for changes (needs --note).
    #[arg(long)]
    changes: bool,

    /// briefing: send it back (needs --note, the reason).
    #[arg(long)]
    send_back: bool,

    /// briefing: file left-out item N, from 1, with the verdict. Repeat per
    /// item.
    #[arg(long = "file", value_name = "N", allow_negative_numbers = true)]
    file: Vec<i64>,

    /// stuck: guidance for the agent. question: the reply.
    #[arg(long, value_name = "TEXT")]
    text: Option<String>,

    /// --text read from a file. - reads stdin.
    #[arg(long, value_name = "FILE")]
    text_file: Option<PathBuf>,

    /// Any kind: drop the case, so that the agent stops the work.
    #[arg(long)]
    drop: bool,

    /// fyi: acknowledge.
    #[arg(long)]
    ack: bool,

    /// A note to the agent. Allowed with every response.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,

    /// Record the answer as written by NAME. Default: the config file's
    /// name, then the login name.
    #[arg(long = "as", value_name = "NAME")]
    as_: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Answers the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let key = ctx.find_case(&args.id)?;
    // The case as read now: its kind says which verdict --accept is, and
    // its amend seq goes with the answer.
    let case = ctx.case(key)?;
    let mut answer = Answer::default();
    // The core reads empty text as no response, which is not what an empty
    // file is.
    if let Some(text) = text::read("text", args.text.as_deref(), args.text_file.as_deref())? {
        if args.text_file.is_some() && text.is_empty() {
            return Err(Error::new("the answer text is empty"));
        }
        answer.text = text;
    }
    answer.choice = args.option.unwrap_or_default();
    answer.other = args.other;
    answer.rows = args
        .rows
        .iter()
        .map(|row| verdict(row))
        .collect::<Result<_, _>>()?;
    if args.accept && case.kind() == Kind::Briefing {
        answer.briefing = "accept".into();
    } else if args.accept {
        answer.signoff = "accept".into();
    } else if args.changes {
        answer.signoff = "changes".into();
    } else if args.send_back {
        answer.briefing = "send_back".into();
    }
    answer.file.clone_from(&args.file);
    answer.drop = args.drop;
    answer.ack = args.ack;
    answer.note = args.note.clone().unwrap_or_default();
    // The answer carries the amend it was written against, so that one
    // written before the agent changed the question is refused as stale
    // (ST-04). It is the case as read now, so an amend that lands between
    // this read and the write is caught; with --revision, the revision
    // guard catches one before it.
    answer.amend_seq = i64::try_from(case.amend_seq()).unwrap_or(i64::MAX);
    let writer = ctx.human(args.as_.as_deref());
    let record = Record::Answer(answer);
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

/// Reads a row's verdict: `id=verdict`, or `id=verdict:note`.
fn verdict(text: &str) -> Result<RowVerdict, Error> {
    let refused = || {
        Error::new(format!(
            "--row {text:?}: want id=approve, id=hold or id=reject"
        ))
    };
    let (id, rest) = text.split_once('=').ok_or_else(refused)?;
    let (verdict, note) = rest.split_once(':').unwrap_or((rest, ""));
    if id.is_empty() || verdict.is_empty() {
        return Err(refused());
    }
    let mut row = RowVerdict::default();
    row.id = id.into();
    row.verdict = verdict.into();
    row.note = note.into();
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_reads_with_or_without_a_note() {
        let row = verdict("b=hold:needs a dry run: first").unwrap();
        assert_eq!((row.id.as_str(), row.verdict.as_str()), ("b", "hold"));
        assert_eq!(row.note, "needs a dry run: first");
        assert_eq!(verdict("a=approve").unwrap().note, "");
        for bad in ["approve", "=approve", "a=", "a=:note"] {
            assert!(verdict(bad).is_err(), "{bad}");
        }
    }
}
