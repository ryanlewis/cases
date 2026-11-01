//! `cases list`: the cases in the store, by state and filter (AG-14).

use std::io::Write;
use std::time::Duration;

use cases_core::{Case, Kind, State, Time, Urgency};
use cases_store_sqlite::Key;
use serde::Serialize;

use crate::commands::open::{kinds, urgencies};
use crate::ctx::{self, Ctx};
use crate::duration;
use crate::error::Error;
use crate::output::{self, CaseJson};

/// The arguments of `cases list`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Only cases in these states (open, answered, pickedup, closed,
    /// withdrawn, parked, sent). Repeat or comma-separate. Without it, open
    /// and parked.
    #[arg(long = "state", value_name = "STATE", value_delimiter = ',')]
    states: Vec<String>,

    /// Cases in every state. --state wins over it.
    #[arg(long)]
    all: bool,

    /// Only cases with this urgency. Repeat for any of them.
    #[arg(long = "urgency", value_name = "URGENCY", value_parser = urgencies())]
    urgencies: Vec<Urgency>,

    /// Only cases whose last event is older than this, such as 30m or 72h, so
    /// a resumed case counts from its resume. 0 means any age.
    #[arg(long, value_name = "DURATION", value_parser = duration::parse, default_value = "0")]
    older_than: Duration,

    #[command(flatten)]
    filter: Filter,

    /// Print only the number of matching cases.
    #[arg(long)]
    count: bool,

    /// Print JSON.
    #[arg(short, long)]
    json: bool,
}

/// Picks cases by kind, label and worker, for list, wait and sweep. Within a
/// filter any value matches; every filter given must match.
#[derive(Debug, clap::Args)]
pub(crate) struct Filter {
    /// Only cases of this kind. Repeat for any of them.
    #[arg(long = "kind", value_name = "KIND", value_parser = kinds())]
    pub(crate) kinds: Vec<Kind>,

    /// Only cases with this label. Repeat for cases with any of them.
    #[arg(long = "label", value_name = "TEXT")]
    pub(crate) labels: Vec<String>,

    /// Only cases from this worker. Repeat for cases from any of them.
    #[arg(long = "worker", value_name = "NAME")]
    pub(crate) workers: Vec<String>,
}

impl Filter {
    /// Whether `case` has one of the kinds, one of the labels and one of the
    /// workers asked for. A filter left empty matches every case.
    pub(crate) fn matches(&self, case: &Case) -> bool {
        (self.kinds.is_empty() || self.kinds.contains(&case.kind()))
            && by_worker_and_label(&self.workers, &self.labels, case.worker(), case.labels())
    }
}

/// Whether a case of `worker` and `labels` has one of `workers` and one of
/// `wanted` labels, as a filter takes it: an empty list matches every case.
/// A listening `wait` wakes for a case's answer by this rule.
pub(crate) fn by_worker_and_label(
    workers: &[String],
    wanted: &[String],
    worker: &str,
    labels: &[String],
) -> bool {
    (wanted.is_empty() || labels.iter().any(|l| wanted.contains(l)))
        && (workers.is_empty() || workers.iter().any(|w| w == worker))
}

/// Whether the last event of `case` is at least `age` before `now`, for
/// `--older-than` on list and sweep and `--age` on prune: an age of 0 takes
/// every case, one dated in the future too.
pub(crate) fn older_than(case: &Case, age: Duration, now: Time) -> bool {
    age.is_zero()
        || case
            .updated_at()
            .is_none_or(|at| output::elapsed(at, now).is_some_and(|elapsed| elapsed >= age))
}

/// The states listed when none is asked for: the cases waiting on the human,
/// and those put aside (AG-14, IN-01).
pub(crate) const WAITING: [State; 2] = [State::Open, State::Parked];

/// The cases `keep` takes, in inbox order: most urgent first, then in the
/// order they were opened. `cases` are in the order they were opened, as the
/// store gives them, and a stable sort keeps it within each urgency.
pub(crate) fn inbox_order(
    cases: &[(Key, Case)],
    keep: impl Fn(&Case) -> bool,
) -> Vec<&(Key, Case)> {
    let mut shown: Vec<_> = cases.iter().filter(|(_, case)| keep(case)).collect();
    shown.sort_by_key(|(_, case)| case.urgency());
    shown
}

/// Lists the cases: most urgent first, then in the order they were opened.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let mut states = args
        .states
        .iter()
        .map(|state| state.parse::<State>())
        .collect::<Result<Vec<_>, _>>()?;
    if states.is_empty() && !args.all {
        states = WAITING.to_vec();
    }
    let now = output::now()?;
    let cases = ctx.cases()?;
    for (key, case) in &cases {
        ctx::warn_problems(*key, case);
    }
    let shown = inbox_order(&cases, |case| {
        (states.is_empty() || states.contains(&case.state()))
            && (args.urgencies.is_empty() || args.urgencies.contains(&case.urgency()))
            && older_than(case, args.older_than, now)
            && args.filter.matches(case)
    });

    if args.count {
        writeln!(ctx.out, "{}", shown.len())?;
        return Ok(());
    }
    if args.json {
        let actors = ctx.actors()?;
        let list = shown
            .iter()
            .map(|(key, case)| CaseJson::new(*key, case, &actors))
            .collect::<Result<Vec<_>, _>>()?;
        return output::json(&mut ctx.out, &Listed { list });
    }
    if shown.is_empty() {
        writeln!(ctx.out, "No cases.")?;
        return Ok(());
    }
    // Labels come before the title, so that a long title does not push them
    // off the line.
    let mut rows = vec![
        ["ID", "STATE", "URGENCY", "KIND", "AGE", "LABELS", "TITLE"]
            .map(String::from)
            .to_vec(),
    ];
    for (key, case) in shown {
        rows.push(vec![
            output::titled(*key, case.title()),
            case.state().as_str().into(),
            case.urgency().as_str().into(),
            case.kind().as_str().into(),
            output::age(case.opened_at(), now),
            case.labels().join(","),
            case.title().into(),
        ]);
    }
    output::table(&mut ctx.out, &rows)
}

/// What `list --json` prints, and `GET /v1/cases` gives.
#[derive(Debug, Serialize)]
pub(crate) struct Listed<'a> {
    pub(crate) list: Vec<CaseJson<'a>>,
}
