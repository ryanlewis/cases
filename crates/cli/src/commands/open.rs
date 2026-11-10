//! `cases open`: opens a case (AG-01, AG-02).

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use cases_core::{Append, ApprovalRow, Author, Kind, Open, Record, Store, Urgency};
use cases_store_sqlite::Writer;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use serde::Deserialize;

use crate::commands::wait;
use crate::ctx::{self, Ctx};
use crate::duration;
use crate::error::Error;
use crate::output::{self, Written};
use crate::text;

/// The arguments of `cases open`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// What the case asks for.
    #[arg(long, value_name = "KIND", value_parser = opened_kinds())]
    kind: Kind,

    /// How soon it needs an answer.
    #[arg(long, value_name = "URGENCY", value_parser = urgencies())]
    urgency: Urgency,

    /// One-line title.
    #[arg(long)]
    title: String,

    /// The body as one line of text.
    #[arg(long, value_name = "TEXT", conflicts_with = "body_file")]
    body: Option<String>,

    /// Markdown body. - reads stdin.
    #[arg(long, value_name = "FILE")]
    body_file: Option<PathBuf>,

    /// An option for a decision case. Repeat per option; "Other, see note"
    /// is always offered.
    #[arg(long = "option", value_name = "TEXT")]
    options: Vec<String>,

    /// The option you recommend, by its number from 1: decision only. The
    /// human sees it marked, and may choose any option.
    #[arg(
        long,
        value_name = "N",
        value_parser = recommend(),
        allow_negative_numbers = true
    )]
    recommend: Option<i64>,

    /// A row for an approval case, as a JSON object with id, label, script,
    /// link, an optional note, and "by":"human" for a step only the human
    /// carries out, e.g. '{"id":"deps","label":"Install deps","script":"npm
    /// ci","link":"https://…"}'. Repeat per row.
    #[arg(long = "row", value_name = "JSON")]
    rows: Vec<String>,

    /// briefing: the work has landed already, so nothing waits on the
    /// answer. Left out, the briefing is a gate: wait for the answer before
    /// landing the work.
    #[arg(long)]
    landed: bool,

    /// briefing: something the work left out on purpose, which the human
    /// may file. Repeat per item, up to 10, each at most 300 characters.
    #[arg(long = "left-out", value_name = "TEXT")]
    left_out: Vec<String>,

    /// A link to show with the case. Repeatable.
    #[arg(long = "link", value_name = "URL")]
    links: Vec<String>,

    #[arg(
        long = "label",
        value_name = "TEXT",
        help = "A label to group the case by, such as the work it belongs to. Repeatable; \
                replaces the one label $CASES_LABEL gives."
    )]
    labels: Vec<String>,

    #[arg(
        long,
        value_name = "NAME",
        help = "The agent session waiting on this case. Default: $CASES_WORKER."
    )]
    worker: Option<String>,

    /// Where to restart from if the case is parked: a brief, a ledger or a
    /// note, as a path or a short line.
    #[arg(long, value_name = "TEXT")]
    brief: Option<String>,

    /// Free-text context for the human, shown with the case.
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,

    /// Who the case is addressed to, such as the human expected to answer
    /// it.
    #[arg(long = "for", value_name = "NAME")]
    for_: Option<String>,

    /// The case this one is about, by its whole id, such as the message
    /// from the human that you have a question about.
    #[arg(long, value_name = "ID")]
    re: Option<String>,

    /// Print the id, state and revision as JSON.
    #[arg(short, long)]
    json: bool,

    /// Then wait on the case, as `cases wait --id ID --since ID --pickup
    /// --by WORKER` does: the id is printed first, and the case's line once
    /// it is your turn, picked up when answered. Exit 2 at --timeout. Not
    /// with --json.
    #[arg(long, conflicts_with = "json")]
    wait: bool,

    /// With --wait: give up after this long, such as 30s or 10m, after the
    /// id: one line on stderr, exit 2. 0 waits for ever.
    #[arg(long, value_name = "DURATION", value_parser = duration::parse, requires = "wait")]
    timeout: Option<Duration>,
}

/// The value of the environment variable `var`, unless it is unset, empty
/// or only white space, which count as not given, as for `CASES_STORE`
/// (AG-01, case 2026-09-25T20-55-32Z-empty-cases-label-not-given-or-refused).
pub(crate) fn given(var: &str) -> Option<String> {
    env::var(var).ok().filter(|value| !value.trim().is_empty())
}

/// The kinds, for clap to offer and check.
pub(crate) fn kinds() -> impl TypedValueParser<Value = Kind> {
    kinds_where(|_| true)
}

/// The kinds the agent opens: every kind but a message, which only the
/// human sends.
fn opened_kinds() -> impl TypedValueParser<Value = Kind> {
    kinds_where(|kind| kind.opener() == Author::Agent)
}

/// The kinds `keep` keeps, for clap to offer and check.
fn kinds_where(keep: fn(Kind) -> bool) -> impl TypedValueParser<Value = Kind> {
    let kinds = Kind::ALL.iter().filter(move |kind| keep(**kind));
    PossibleValuesParser::new(kinds.map(|kind| kind.as_str())).try_map(|kind| kind.parse::<Kind>())
}

/// The urgencies, for clap to offer and check.
pub(crate) fn urgencies() -> impl TypedValueParser<Value = Urgency> {
    PossibleValuesParser::new(Urgency::ALL.iter().map(|urgency| urgency.as_str()))
        .try_map(|urgency| urgency.parse::<Urgency>())
}

/// An option's number, from 1, for `--recommend` on open and amend. The core
/// reads a recommend of 0 as one left out, so clap refuses it here rather
/// than let `--recommend 0` recommend nothing.
pub(crate) fn recommend() -> impl TypedValueParser<Value = i64> {
    clap::value_parser!(i64).try_map(|n: i64| {
        if n >= 1 {
            Ok(n)
        } else {
            Err("options are numbered from 1")
        }
    })
}

/// Opens the case and prints its id.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let mut open = Open::default();
    open.kind = args.kind.as_str().into();
    open.urgency = args.urgency.as_str().into();
    open.title.clone_from(&args.title);
    open.body =
        text::read("body", args.body.as_deref(), args.body_file.as_deref())?.unwrap_or_default();
    open.options.clone_from(&args.options);
    open.recommend = args.recommend.unwrap_or_default();
    open.rows = rows(&args.rows)?;
    open.landed = args.landed;
    open.left_out.clone_from(&args.left_out);
    open.links.clone_from(&args.links);
    // The environment's label is one label, commas and all, and a label
    // given here replaces it.
    open.labels = if args.labels.is_empty() {
        given("CASES_LABEL").into_iter().collect()
    } else {
        args.labels.clone()
    };
    open.worker = args
        .worker
        .clone()
        .or_else(|| given("CASES_WORKER"))
        .unwrap_or_default();
    open.brief = args.brief.clone().unwrap_or_default();
    open.context = args.context.clone().unwrap_or_default();
    open.for_ = args.for_.clone().unwrap_or_default();
    // An id that names no case in the store is refused, so that the link
    // does not dangle; the case is kept by its bare id.
    if let Some(re) = &args.re {
        let key = ctx::exact(re)?;
        ctx.case(key)?;
        open.re = key.to_string();
    }

    // A wait that would fail on the config file fails before the case is
    // opened, so that there is no case left with no wait on it.
    if args.wait {
        wait::chosen(None, ctx)?;
    }

    let writer = Writer::new(Some(&open.worker));
    let append = Append::new(Author::Agent, &Record::Open(open));
    let (key, case) = ctx.store.create(&append, &writer)?;
    warn_options(&args.options, 1);
    // A command that goes on to wait on the case hears its answer itself.
    if !args.wait {
        ctx::warn_unwoken(ctx, &case, true);
    }
    if args.json {
        output::json(&mut ctx.out, &Written::new(key, &case))?;
    } else {
        writeln!(ctx.out, "{key}")?;
    }
    if !args.wait {
        return Ok(());
    }
    // The id is out before the wait begins, so that one killed while it
    // waits still leaves it.
    ctx.out.flush()?;
    let by = Some(case.worker().to_owned()).filter(|worker| !worker.is_empty());
    let timeout = args.timeout.unwrap_or_default();
    wait::run(&wait::Args::opened(key.to_string(), by, timeout), ctx)
}

/// Warns on stderr, without refusing, of each option in `options`, the
/// first numbered `first`, whose text looks like a list of the body's: a
/// number and a separator at its start ("3 ", "3. ", "3)"), or
/// "(recommended)" in it. The inbox numbers the options itself, in the
/// order given, and its number keys choose by that number, so an option
/// that carries another number, or was moved to the top as the pick, reads
/// wrong there; `--recommend` marks the pick.
pub(crate) fn warn_options(options: &[String], first: usize) {
    for (i, text) in options.iter().enumerate() {
        if let Some(problem) = option_problem(text) {
            eprintln!(
                "warning: option {} ({text:?}) {problem}. The inbox numbers the options itself, in the order given: keep the order the body gives them in, and use --recommend N for the one you recommend.",
                first + i
            );
        }
    }
}

/// What is wrong with an option's `text`, if it looks like a list's item:
/// see `warn_options`.
fn option_problem(text: &str) -> Option<&'static str> {
    let t = text.trim_start();
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    // "3.1.4" or "2.5x" is a number of the option's own, not a list's.
    let rest = &t[digits..];
    let listed = rest.starts_with([' ', ')'])
        || (rest.starts_with('.') && rest[1..].chars().next().is_none_or(char::is_whitespace));
    if digits > 0 && listed {
        return Some("starts with a number");
    }
    if text.to_lowercase().contains("(recommended)") {
        return Some("says \"(recommended)\"");
    }
    None
}

/// A row as `--row` takes it: a field the row does not have is refused, so
/// that a misspelt field is not dropped.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RowArg {
    id: String,
    label: String,
    script: String,
    link: String,
    note: String,
    by: String,
}

/// Reads the `--row` values, one JSON object each, for open and amend.
pub(crate) fn rows(values: &[String]) -> Result<Vec<ApprovalRow>, Error> {
    values
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let n = i + 1;
            let mut objects = serde_json::Deserializer::from_str(text).into_iter::<RowArg>();
            let arg = match objects.next() {
                Some(Ok(arg)) => arg,
                Some(Err(error)) => return Err(Error::new(format!("--row {n}: {error}"))),
                None => return Err(Error::new(format!("--row {n}: no row object"))),
            };
            if !text[objects.byte_offset()..].trim().is_empty() {
                return Err(Error::new(format!(
                    "--row {n}: unexpected data after the row object; pass one --row per row"
                )));
            }
            let mut row = ApprovalRow::default();
            row.id = arg.id;
            row.label = arg.label;
            row.script = arg.script;
            row.link = arg.link;
            row.note = arg.note;
            row.by = arg.by;
            Ok(row)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An option numbered as a list's item, or marked as the pick, is named;
    /// a number of the option's own is not.
    #[test]
    fn an_option_that_looks_like_a_list_item_is_named() {
        for text in [
            "3 Split plate",
            "3. Split plate",
            "3) Split plate",
            "  12 x",
            "1.",
            "Split plate (recommended)",
            "Split slab (Recommended)",
        ] {
            assert!(option_problem(text).is_some(), "{text}");
        }
        for text in [
            "Split plate",
            "Pin to 1.2.3",
            "3.1.4 then 3.2",
            "2x the budget",
            "1password",
            "recommended by the vendor",
            "",
        ] {
            assert_eq!(option_problem(text), None, "{text}");
        }
    }

    fn error(value: &str) -> String {
        rows(&["{}".into(), value.into()]).unwrap_err().to_string()
    }

    #[test]
    fn a_row_reads_from_json() {
        let rows = rows(&[
            r#"{"id":"mig","label":"Migrate","note":"a minute down","script":"make migrate","link":"https://x"}"#.into(),
        ])
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "mig");
        assert_eq!(rows[0].note, "a minute down");
        assert_eq!(rows[0].link, "https://x");
    }

    #[test]
    fn a_row_that_is_not_one_object_is_refused() {
        assert!(error("deps=npm ci").starts_with("--row 2: "));
        assert!(error(r#"{"id":"a","cmd":"x"}"#).contains("unknown field `cmd`"));
        assert!(error("").starts_with("--row 2: no row object"));
        for two in [
            r#"{"id":"a"},{"id":"b"}"#,
            r#"{"id":"a"}}, {"id":"b"}"#,
            r#"{"id":"a"}]"#,
        ] {
            assert_eq!(
                error(two),
                "--row 2: unexpected data after the row object; pass one --row per row"
            );
        }
        assert!(rows(&[" {} \n".into()]).is_ok());
    }
}
