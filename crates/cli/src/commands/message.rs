//! `cases message`: the human sends an agent a message, which the first
//! session listening for its worker or one of its labels takes (HU-08).

use std::io::Write;
use std::path::PathBuf;

use cases_core::{Append, Author, Kind, Open, Record, Store, Urgency};

use crate::commands::open::urgencies;
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::output::{self, Written};
use crate::text;

/// The arguments of `cases message`. It needs `--to` or a `--label`, which
/// the core checks, as it does on a replay: a message goes to a worker or a
/// label. It needs `--title` or a body to take one from.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The worker to send it to: the agent session's name. Needs --to or a
    /// --label, or both.
    #[arg(long, value_name = "NAME")]
    to: Option<String>,

    /// A label to send it to: the first session listening on it takes it.
    /// Repeatable.
    #[arg(long = "label", value_name = "TEXT")]
    labels: Vec<String>,

    /// One-line title. Without it, the title is the body's first line,
    /// without its markdown, cut at a word near 80 characters.
    #[arg(long)]
    title: Option<String>,

    /// The body as one line of text.
    #[arg(long, value_name = "TEXT", conflicts_with = "body_file")]
    body: Option<String>,

    /// Markdown body. - reads stdin.
    #[arg(long, value_name = "FILE")]
    body_file: Option<PathBuf>,

    /// How soon it needs doing.
    #[arg(long, value_name = "URGENCY", value_parser = urgencies(), default_value = "today")]
    urgency: Urgency,

    /// A link to send with it. Repeatable.
    #[arg(long = "link", value_name = "URL")]
    links: Vec<String>,

    /// Free-text context for the agent.
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,

    /// The case the message is about: its id, or any part of it that names
    /// one case.
    #[arg(long, value_name = "ID")]
    re: Option<String>,

    /// Send it as the human, written by NAME. A message is the human's
    /// alone, so NAME is required and may not be blank; the config file's
    /// name does not stand in for it.
    #[arg(long = "as", value_name = "NAME", value_parser = ctx::human_name("message"))]
    as_: String,

    /// Print the id, state and revision as JSON.
    #[arg(short, long)]
    json: bool,
}

/// Sends the message and prints its id. Says on stderr when no session is
/// listening for its worker or any of its labels: it waits in the outbox
/// until one is.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let mut open = Open::default();
    open.kind = Kind::Message.as_str().into();
    open.urgency = args.urgency.as_str().into();
    open.body =
        text::read("body", args.body.as_deref(), args.body_file.as_deref())?.unwrap_or_default();
    open.title = match &args.title {
        Some(title) => title.clone(),
        None => title_of(&open.body).ok_or_else(|| {
            Error::new("give --title, or a body whose first line can be the title")
        })?,
    };
    open.worker = args.to.clone().unwrap_or_default();
    open.labels.clone_from(&args.labels);
    open.links.clone_from(&args.links);
    open.context = args.context.clone().unwrap_or_default();
    if let Some(re) = &args.re {
        open.re = ctx.find_case(re)?.to_string();
    }
    let writer = ctx.human(Some(&args.as_));
    let append = Append::new(Author::Human, &Record::Open(open));
    let (key, case) = ctx.store.create(&append, &writer)?;
    if !ctx::message_listened_for(ctx, case.worker(), case.labels()) {
        ctx::warn(format_args!(
            "no session is listening for {} now: the message waits until one is",
            ctx::addressee(case.worker(), case.labels())
        ));
    }
    if args.json {
        output::json(&mut ctx.out, &Written::new(key, &case))
    } else {
        writeln!(ctx.out, "{key}")?;
        Ok(())
    }
}

/// The longest a title taken from a body is, in characters, before the
/// ellipsis a cut adds.
const TITLE_MAX: usize = 80;

/// The title a message sent as a note alone takes: the first line of `body`
/// with a letter or a digit once its markdown is stripped, cut at a word
/// boundary at [`TITLE_MAX`] characters with an ellipsis when cut. `None`
/// when no line has one. The web inbox's composer takes a title the same
/// way (`title()` in compose.ts), and the tests of both use the same
/// examples.
pub(crate) fn title_of(body: &str) -> Option<String> {
    let line = body
        .lines()
        .map(unmarked)
        .find(|line| line.chars().any(char::is_alphanumeric))?;
    if line.chars().count() <= TITLE_MAX {
        return Some(line);
    }
    let head: String = line.chars().take(TITLE_MAX).collect();
    let next = line.chars().nth(TITLE_MAX);
    let cut = if next == Some(' ') {
        head.as_str()
    } else {
        match head.rfind(' ') {
            Some(at) if at > 0 => &head[..at],
            _ => head.as_str(),
        }
    };
    Some(format!("{}…", cut.trim_end()))
}

/// `line` without the markdown a title has no use for: the markers at its
/// start (headings, quotes, list items), then emphasis stars and backticks
/// anywhere, with its runs of spaces made one. A code fence's line is none
/// of it: its info string (`sh`) is no title. A byte order mark counts as a
/// space, as it does in the composer's JavaScript.
fn unmarked(line: &str) -> String {
    let line = line.replace('\u{feff}', " ");
    let mut rest = line.trim();
    loop {
        let before = rest;
        if let Some(after) = rest.strip_prefix('#')
            && let after = after.trim_start_matches('#')
            && (after.is_empty() || after.starts_with(char::is_whitespace))
        {
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix('>') {
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix(['-', '*', '+'])
            && after.starts_with(char::is_whitespace)
        {
            rest = after.trim_start();
        } else {
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if let Some(after) = rest[digits..].strip_prefix(['.', ')'])
                && digits > 0
                && after.starts_with(char::is_whitespace)
            {
                rest = after.trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    if rest.starts_with("```") || rest.starts_with("~~~") {
        return String::new();
    }
    rest.replace(['*', '`'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same examples as compose.test.js's, so the two stay one rule.
    #[test]
    fn a_title_is_the_first_line_of_the_body_without_its_markdown() {
        for (body, want) in [
            (
                "Rebase onto main\n\nThen run the tests.",
                Some("Rebase onto main"),
            ),
            (
                "## Rebase onto *main*\nThen test.",
                Some("Rebase onto main"),
            ),
            ("> - 1. Pin `bun` to **1.2**", Some("Pin bun to 1.2")),
            ("3) ship it", Some("ship it")),
            ("-flag stays", Some("-flag stays")),
            ("#123 is fixed", Some("#123 is fixed")),
            ("```sh\nrm -rf target\n```", Some("rm -rf target")),
            ("> ~~~\nship it", Some("ship it")),
            ("\u{feff}# Rebase", Some("Rebase")),
            ("Rebase", Some("Rebase")),
            ("\n  \n\tRebase  onto   main  \n", Some("Rebase onto main")),
            ("---\n#\n> \nRebase", Some("Rebase")),
            ("---\n# \n**", None),
            ("", None),
            (
                "Rebase the pinning branch onto main, then run the whole test suite and the mutants twice over",
                Some("Rebase the pinning branch onto main, then run the whole test suite and the…"),
            ),
            (&"x".repeat(100), Some(&*format!("{}…", "x".repeat(80)))),
            (&"x".repeat(80), Some(&*"x".repeat(80))),
        ] {
            assert_eq!(title_of(body).as_deref(), want, "{body:?}");
        }
    }
}
