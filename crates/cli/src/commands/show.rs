//! `cases show`: one case and its thread (AG-13).

use std::io::Write;

use cases_core::{Case, Kind};
use cases_store_sqlite::Key;

use crate::commands::inbox::hosted_case_url;
use crate::ctx::{self, Ctx};
use crate::describe::{self, Previous};
use crate::error::Error;
use crate::instance;
use crate::output::{self, Actors, AnswerJson, CaseJson};
use crate::store::AnyStore;

/// The arguments of `cases show`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id, or any part of it that names one case.
    id: String,

    /// Print JSON, including every event as written.
    #[arg(short, long, conflicts_with = "answer")]
    json: bool,

    /// Print only the state, kind, revision and current answer, as JSON.
    #[arg(long)]
    answer: bool,
}

/// Prints the case.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let (key, case) = ctx.find(&args.id)?;
    ctx::warn_problems(key, &case);
    if args.answer {
        return output::json(&mut ctx.out, &AnswerJson::new(&case));
    }
    let url = inbox_url(ctx, key, &case);
    let actors = ctx.actors()?;
    if args.json {
        let shown = CaseJson::new(key, &case, &actors)?.with_url(url);
        return output::json(&mut ctx.out, &shown);
    }
    print(&mut ctx.out, key, &case, &actors, &url)
}

/// The case's page in the inbox running for the store, at its link (an
/// allowed host's, when serve has one), or empty when none is. An instance
/// file that cannot be read is a warning, not a reason to fail. On a remote
/// store it is the case's page in the hosted inbox, with
/// no request; on a forwarded one, at the link the far side's serve gives,
/// asked for once, and empty when it gives none (ST-11, AG-13).
fn inbox_url(ctx: &Ctx, key: Key, case: &Case) -> String {
    if let Some(url) = hosted_case_url(ctx, key, case) {
        return url;
    }
    if let AnyStore::Forward(forwarded) = &ctx.store {
        return match forwarded.inbox_link() {
            Ok(Some(link)) => instance::case_url(&link, &output::titled(key, case.title())),
            Ok(None) => String::new(),
            Err(error) => {
                ctx::warn(error);
                String::new()
            }
        };
    }
    let Some(path) = ctx.target.path() else {
        return String::new();
    };
    match instance::running(path) {
        Ok(Some(info)) => instance::case_url(info.link(), &output::titled(key, case.title())),
        Ok(None) => String::new(),
        Err(error) => {
            ctx::warn(error);
            String::new()
        }
    }
}

/// Prints the case for people: its fields, body, options, rows and links,
/// then its thread and any rows the fold skipped.
fn print(
    out: &mut impl Write,
    key: Key,
    case: &Case,
    actors: &Actors,
    url: &str,
) -> Result<(), Error> {
    writeln!(out, "{}\n", case.title())?;
    fields(out, key, case, url)?;
    let body = case.body().trim();
    if !body.is_empty() {
        writeln!(out, "\n{body}")?;
    }
    options(out, case)?;
    left_out(out, case)?;
    rows(out, case)?;
    links(out, case)?;
    thread(out, case, actors)
}

/// Prints each field the case has, one to a line.
fn fields(out: &mut impl Write, key: Key, case: &Case, url: &str) -> Result<(), Error> {
    let revision = case.revision().to_string();
    let labels = case.labels().join(", ");
    let opened = output::stamp(case.opened_at());
    for (name, value) in [
        ("id", output::titled(key, case.title()).as_str()),
        ("url", url),
        ("state", case.state().as_str()),
        ("kind", case.kind().as_str()),
        ("urgency", case.urgency().as_str()),
        ("opened", &opened),
        ("revision", &revision),
        ("labels", &labels),
        ("for", case.for_()),
        // A message's worker is the one it is sent to.
        (
            if case.kind() == Kind::Message {
                "to"
            } else {
                "worker"
            },
            case.worker(),
        ),
        ("re", case.re()),
        (
            "landed",
            match (case.kind(), case.landed()) {
                (Kind::Briefing, true) => "landed",
                (Kind::Briefing, false) => "waiting to land",
                _ => "",
            },
        ),
        ("brief", case.brief()),
        ("context", case.context()),
    ] {
        if !value.is_empty() {
            writeln!(out, "{:<9} {value}", format!("{name}:"))?;
        }
    }
    Ok(())
}

/// Prints a briefing's left-out items, numbered as an answer files them, if
/// it has any.
fn left_out(out: &mut impl Write, case: &Case) -> Result<(), Error> {
    if case.left_out().is_empty() {
        return Ok(());
    }
    writeln!(out, "\nLeft out:")?;
    for (n, item) in (1..).zip(case.left_out()) {
        writeln!(out, "  {n}. {item}")?;
    }
    Ok(())
}

/// Prints the case's options, numbered, if it has any.
fn options(out: &mut impl Write, case: &Case) -> Result<(), Error> {
    if case.options().is_empty() {
        return Ok(());
    }
    writeln!(out, "\nOptions:")?;
    for (n, option) in (1..).zip(case.options()) {
        let mark = if n == case.recommend() {
            " (recommended)"
        } else {
            ""
        };
        writeln!(out, "  {n}. {option}{mark}")?;
    }
    writeln!(out, "  other. Other, see note")?;
    Ok(())
}

/// Prints the case's rows, each with its script, if it has any.
fn rows(out: &mut impl Write, case: &Case) -> Result<(), Error> {
    if case.rows().is_empty() {
        return Ok(());
    }
    writeln!(out, "\nRows:")?;
    for row in case.rows() {
        writeln!(out, "  [{}] {}", row.id, row.label)?;
        if !row.by.is_empty() {
            writeln!(out, "      by: {}", row.by)?;
        }
        if !row.note.is_empty() {
            writeln!(out, "      note: {}", row.note)?;
        }
        writeln!(out, "      {}", row.link)?;
        quote(out, "      ", row.script.trim_end_matches('\n'))?;
    }
    Ok(())
}

/// Prints the case's links, if it has any.
fn links(out: &mut impl Write, case: &Case) -> Result<(), Error> {
    if case.links().is_empty() {
        return Ok(());
    }
    writeln!(out, "\nLinks:")?;
    for link in case.links() {
        writeln!(out, "  {link}")?;
    }
    Ok(())
}

/// Prints the case's thread, with what each amend replaced, then the rows
/// the fold skipped.
fn thread(out: &mut impl Write, case: &Case, actors: &Actors) -> Result<(), Error> {
    writeln!(out, "\nThread:")?;
    for (event, lines) in describe::thread(case, actors) {
        writeln!(
            out,
            "  {:04} {:<5} {:<8} {}",
            event.seq,
            event.author.as_str(),
            event.event().as_str(),
            output::stamp(event.at())
        )?;
        for line in lines {
            writeln!(out, "       {}", line.text)?;
            let (field, text) = match &line.previous {
                Some(Previous::Body(text)) => ("body", text),
                Some(Previous::Context(text)) => ("context", text),
                None => continue,
            };
            writeln!(out, "         previous {field}:")?;
            quote(out, "         ", text.trim())?;
        }
    }
    for problem in case.problems() {
        writeln!(out, "  problem: {}", output::problem(problem))?;
    }
    Ok(())
}

/// Prints each line of `text` after `indent` and `| `.
fn quote(out: &mut impl Write, indent: &str, text: &str) -> Result<(), Error> {
    for line in text.split('\n') {
        writeln!(out, "{indent}| {line}")?;
    }
    Ok(())
}
