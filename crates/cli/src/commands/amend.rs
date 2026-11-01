//! `cases amend`: changes an open case before it is answered (AG-07, and a
//! briefing's landed and left-out items, AG-19), or
//! with `--reopen`, an answered or picked-up case, which it reopens; with
//! `--as`, the human's edit of a message no agent has picked up (HU-08).

use std::path::PathBuf;

use cases_core::{Amend, Author, Record};

use crate::commands::open::{recommend, rows, warn_options};
use crate::commands::write::{self, WriteArgs};
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::text;

/// The arguments of `cases amend`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id. With --as, any part of it that names one case.
    id: String,

    /// A title that replaces the case's.
    #[arg(long, value_name = "TEXT")]
    title: Option<String>,

    /// One line of text that replaces the body.
    #[arg(long, value_name = "TEXT", conflicts_with = "body_file")]
    body: Option<String>,

    /// Markdown that replaces the body. - reads stdin.
    #[arg(long, value_name = "FILE")]
    body_file: Option<PathBuf>,

    /// An option to add to a decision case, numbered after the options it
    /// has. Repeat per option.
    #[arg(long = "option", value_name = "TEXT")]
    options: Vec<String>,

    /// The option you recommend now, by its number from 1, in place of the
    /// case's: decision only. It may name an option this amend adds.
    #[arg(
        long,
        value_name = "N",
        value_parser = recommend(),
        allow_negative_numbers = true
    )]
    recommend: Option<i64>,

    /// A row to add to an approval case, as a JSON object with id, label,
    /// script, link, an optional note, and "by":"human" for a step only the
    /// human carries out. The id must not be on the case already. Repeat per
    /// row.
    #[arg(long = "row", value_name = "JSON")]
    rows: Vec<String>,

    /// briefing: the work has landed now.
    #[arg(long, conflicts_with = "not_landed")]
    landed: bool,

    /// briefing: the work has not landed: the briefing waits for the answer.
    #[arg(long)]
    not_landed: bool,

    /// briefing: an item left out on purpose. Repeat per item; the items
    /// given replace the case's whole list.
    #[arg(long = "left-out", value_name = "TEXT")]
    left_out: Vec<String>,

    /// A link to add to the case. Repeatable.
    #[arg(long = "link", value_name = "URL")]
    links: Vec<String>,

    /// A link to take off the case, spelt as the case has it. Repeatable.
    #[arg(long = "drop-link", value_name = "URL")]
    drop_links: Vec<String>,

    /// A label to add to the case. Repeatable.
    #[arg(long = "label", value_name = "TEXT")]
    labels: Vec<String>,

    /// Free-text context that replaces the case's.
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,

    /// Reopen an answered or picked-up case with this amend: its answer is
    /// cleared, and the human answers the case as amended. Without it, an
    /// amend takes only an open case (exit 3 otherwise).
    #[arg(long, conflicts_with = "as_")]
    reopen: bool,

    /// Edit as the human, written by NAME: a message you sent, while no
    /// agent has picked it up. Without it, the agent amends its open case.
    /// NAME may not be blank.
    #[arg(long = "as", value_name = "NAME", value_parser = ctx::human_name("edit"))]
    as_: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Amends the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let (key, author, writer) = if let Some(name) = args.as_.as_deref() {
        let key = ctx.find_case(&args.id)?;
        (key, Author::Human, ctx.human(Some(name)))
    } else {
        let key = ctx::exact(&args.id)?;
        (key, Author::Agent, write::agent(ctx, key)?)
    };
    let mut amend = Amend::default();
    if let Some(title) = &args.title {
        if title.trim().is_empty() {
            return Err(Error::new("the title is blank"));
        }
        amend.title.clone_from(title);
    }
    // The core reads an empty body or context as one left out, which is not
    // what an empty flag or file asks for.
    if let Some(body) = text::read("body", args.body.as_deref(), args.body_file.as_deref())? {
        if body.is_empty() {
            return Err(Error::new("the body is empty"));
        }
        amend.body = body;
    }
    if let Some(context) = &args.context {
        if context.is_empty() {
            return Err(Error::new("the context is empty"));
        }
        amend.context.clone_from(context);
    }
    amend.options.clone_from(&args.options);
    amend.recommend = args.recommend.unwrap_or_default();
    amend.rows = rows(&args.rows)?;
    amend.landed = if args.landed {
        Some(true)
    } else if args.not_landed {
        Some(false)
    } else {
        None
    };
    amend.left_out.clone_from(&args.left_out);
    amend.links.clone_from(&args.links);
    amend.drop_links.clone_from(&args.drop_links);
    amend.labels.clone_from(&args.labels);
    amend.reopen = args.reopen;
    let case = write::append(
        ctx,
        key,
        author,
        &Record::Amend(amend),
        &writer,
        args.write.revision,
    )
    .map_err(|error| {
        if author == Author::Agent {
            write::hint_reopen(ctx, key, args.reopen, error)
        } else {
            error
        }
    })?;
    // The options this amend adds are numbered after the case's others.
    warn_options(&args.options, case.options().len() + 1 - args.options.len());
    if !args.labels.is_empty() {
        ctx::warn_unwoken(ctx, &case, false);
    }
    write::print(ctx, key, &case, args.write.json)
}
