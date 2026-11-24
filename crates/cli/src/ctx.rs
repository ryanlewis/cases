//! What every command runs with: the store, the config and stdout, and the
//! reads commands share.

use std::env;
use std::fmt::{self, Write as _};
use std::io::{self, Write};

use cases_core::{Case, Kind, Store};
use cases_store_sqlite::{Key, Wait, Writer};
use clap::builder::{StringValueParser, TypedValueParser};

use crate::config::Config;
use crate::error::Error;
use crate::output::{self, Actors};
use crate::store::{AnyStore, Target};

/// The cases that fold, and the keys of those that do not, with why.
pub(crate) type CasesAndUnfolded = (Vec<(Key, Case)>, Vec<(Key, String)>);

/// What a command runs with.
#[derive(Debug)]
pub(crate) struct Ctx {
    /// Where the store is: a local file or a store on a server. Commands
    /// that need the file itself, such as serve and status, use it; the rest
    /// go through `store`.
    pub(crate) target: Target,
    /// The store, local or remote, opened on the first read or write.
    pub(crate) store: AnyStore,
    /// The config file, for the defaults of flags other than `--store`.
    pub(crate) config: Config,
    /// `--store`, as given: for `cases stores` and `cases login`, which
    /// only the `remote` feature has.
    #[cfg_attr(not(feature = "remote"), allow(dead_code))]
    pub(crate) store_flag: Option<String>,
    /// Where results go. Warnings and errors go to stderr.
    pub(crate) out: io::StdoutLock<'static>,
}

impl Ctx {
    /// The case `key`, folded.
    ///
    /// # Errors
    ///
    /// When the store has no such case, the case does not fold, or the store
    /// fails.
    pub(crate) fn case(&self, key: Key) -> Result<Case, Error> {
        case_of(&self.store, key)
    }

    /// Every case in the store, folded, in the order they were opened. A
    /// case that does not fold is warned about and left out, and a store that
    /// is not there yet has no cases.
    ///
    /// # Errors
    ///
    /// When the store fails.
    pub(crate) fn cases(&self) -> Result<Vec<(Key, Case)>, Error> {
        Ok(self.cases_and_unfolded()?.0)
    }

    /// [`Ctx::cases`], and each case that does not fold, with why: it is
    /// warned about too.
    ///
    /// # Errors
    ///
    /// When the store fails.
    pub(crate) fn cases_and_unfolded(&self) -> Result<CasesAndUnfolded, Error> {
        let (mut cases, mut unfolded) = (Vec::new(), Vec::new());
        for (key, rows) in self.rows()? {
            match Case::fold(&rows) {
                Ok(case) => cases.push((key, case)),
                Err(error) => {
                    let why = output::unfolded(&rows, &error);
                    warn(format_args!("{key}: {why}"));
                    unfolded.push((key, why));
                }
            }
        }
        Ok((cases, unfolded))
    }

    /// Every case's rows, or none when the store is not there yet.
    fn rows(&self) -> Result<Vec<(Key, Vec<cases_core::Row>)>, Error> {
        match self.store.list() {
            Ok(cases) => Ok(cases),
            Err(error) if error.is_missing() => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// The store's table of actors, to name who wrote each event.
    ///
    /// # Errors
    ///
    /// When the store fails.
    pub(crate) fn actors(&self) -> Result<Actors, Error> {
        Ok(Actors::of(&self.store)?)
    }

    /// The writer of the human's events (HU-06). On a local store it is
    /// `flag`, the `--as` given, else the config file's `name`, else the
    /// login name in `$USER`, then `$LOGNAME`, else the unnamed human, as
    /// [`human`] gives it. On a remote store it is always the unnamed
    /// human, which is the account's actor, as the hosted inbox writes it:
    /// so one person is one actor in a hosted store. A forwarded store is
    /// named as a local one is, by this machine's flag, config and user.
    pub(crate) fn human(&self, flag: Option<&str>) -> Writer {
        match self.target {
            Target::Local { .. } | Target::Forward { .. } => human(flag, &self.config),
            Target::Remote { .. } | Target::Server { .. } => Writer::new(None),
        }
    }

    /// The case a human names by `text`: its id, or any part of one (HU-07).
    ///
    /// An id, with or without a title slug after it, names that case, and
    /// only that case: an id that is not in the store, such as a pruned
    /// case's, is not matched against the others. Other text names the one
    /// case whose id contains it; none, or several, is an error, and several
    /// are listed. Agent commands take whole ids only, with [`exact`].
    ///
    /// # Errors
    ///
    /// When no case, or more than one, matches, or the store fails.
    pub(crate) fn find_case(&self, text: &str) -> Result<Key, Error> {
        Ok(self.named(text)?.0)
    }

    /// The case a human names by `text`, as [`Ctx::find_case`] finds it, and
    /// the case as read then: one read of the store, not one to find it and
    /// another to fold it.
    ///
    /// # Errors
    ///
    /// As [`Ctx::find_case`], and as [`Ctx::case`] when the case does not
    /// fold.
    pub(crate) fn find(&self, text: &str) -> Result<(Key, Case), Error> {
        match self.named(text)? {
            (key, Some(case)) => Ok((key, case)),
            // A case that does not fold is read again for its error.
            (key, None) => Ok((key, self.case(key)?)),
        }
    }

    /// [`Ctx::find_case`], with the case as it was folded to find it, when
    /// it folds.
    fn named(&self, text: &str) -> Result<(Key, Option<Case>), Error> {
        if let Ok(key) = exact(text) {
            return Ok((key, Some(self.case(key)?)));
        }
        let (part, _slug) = cases_id::split(text);
        let mut matches = Vec::new();
        if !part.is_empty() {
            for (key, rows) in self.rows()? {
                if key.to_string().contains(part) {
                    matches.push((key, Case::fold(&rows).ok()));
                }
            }
        }
        match matches.as_slice() {
            [] => Err(Error::new(format!("no case id contains {text:?}"))),
            [_] => Ok(matches.swap_remove(0)),
            _ => {
                let mut message = format!("{text:?} matches {} cases:", matches.len());
                for (key, case) in &matches {
                    let title = case.as_ref().map_or("", Case::title);
                    let _ = write!(message, "\n  {key}  {title}");
                }
                Err(Error::new(message))
            }
        }
    }
}

/// The writer of the human's events (HU-06): `flag`, the `--as` given, else
/// the config file's `name`, else the login name in `$USER`, then
/// `$LOGNAME`, else the unnamed human. A name that is empty or only white
/// space counts as not given.
fn human(flag: Option<&str>, config: &Config) -> Writer {
    let name = [
        flag.map(str::to_owned),
        config.get("name").map(str::to_owned),
        env::var("USER").ok(),
        env::var("LOGNAME").ok(),
    ]
    .into_iter()
    .flatten()
    .find(|name| !name.trim().is_empty());
    Writer::new(name.as_deref())
}

/// A human's NAME, as `--as` takes it where it makes the event the human's,
/// on `what`, such as a withdraw: it may not be blank, as a blank one, such
/// as an unset variable's, would write the human's event under the config
/// file's name or the login name, which the caller never gave.
pub(crate) fn human_name(what: &'static str) -> impl TypedValueParser<Value = String> {
    StringValueParser::new().try_map(move |name: String| {
        if name.trim().is_empty() {
            Err(format!("the human's {what} needs a name"))
        } else {
            Ok(name)
        }
    })
}

/// Whether a wait listening now would take a message to `worker` and
/// `labels`: one listens for the worker or one of the labels. A store that
/// cannot say has no one listening. A case's answer wakes a listening wait
/// by another rule: see [`answer_wakes`].
pub(crate) fn message_listened_for(ctx: &Ctx, worker: &str, labels: &[String]) -> bool {
    listened_for(&ctx.store, worker, labels)
}

/// [`Ctx::case`], on the store `store`.
///
/// # Errors
///
/// As [`Ctx::case`].
pub(crate) fn case_of(store: &AnyStore, key: Key) -> Result<Case, Error> {
    let rows = match store.rows(&key) {
        Ok(Some(rows)) => rows,
        Ok(None) => return Err(Error::new(format!("no case {key}"))),
        Err(error) if error.is_missing() => return Err(Error::new(format!("no case {key}"))),
        Err(error) => return Err(error.into()),
    };
    Case::fold(&rows)
        .map_err(|error| Error::new(format!("{key}: {}", output::unfolded(&rows, &error))))
}

/// [`message_listened_for`], on the store `store`.
pub(crate) fn listened_for(store: &AnyStore, worker: &str, labels: &[String]) -> bool {
    store
        .waits()
        .unwrap_or_default()
        .iter()
        .any(|wait| wait.worker == worker || wait.labels.iter().any(|label| labels.contains(label)))
}

/// Whether the listening wait `wait` wakes for the answer to a case of
/// `kind`, `worker` and `labels`: `wait` takes a case only when its --worker
/// and, if it has any, one of its --labels and one of its --kinds match,
/// not any one of them, as it takes a message.
fn answer_wakes(wait: &Wait, kind: Kind, worker: &str, labels: &[String]) -> bool {
    crate::commands::list::by_worker_and_label(
        std::slice::from_ref(&wait.worker),
        &wait.labels,
        worker,
        labels,
    ) && (wait.kinds.is_empty() || wait.kinds.iter().any(|k| k == kind.as_str()))
}

/// The listening waits, of `waits`, that share a label with a case of
/// `kind`, `worker` and `labels` though none of them, nor any other, wakes
/// for its answer: an agent that gave its label and another's worker name,
/// which leaves the answer to wake nobody. None when a wait wakes for it, or
/// none shares a label.
fn unwoken<'a>(waits: &'a [Wait], kind: Kind, worker: &str, labels: &[String]) -> Vec<&'a Wait> {
    if waits.iter().any(|w| answer_wakes(w, kind, worker, labels)) {
        return Vec::new();
    }
    waits
        .iter()
        .filter(|w| w.labels.iter().any(|label| labels.contains(label)))
        .collect()
}

/// Warns when sessions listen on one of `case`'s labels but none wakes for
/// its answer, because each waits for its own --worker (see [`unwoken`]). A
/// store that cannot say gives no warning, nor does a case with no labels,
/// nor a message, which has no answer and is taken by its worker or one of
/// its labels (see [`message_listened_for`]). `opening` says whether the
/// case is being opened, when a worker can still be given; after, the only
/// way to hear its answer is a wait on its id.
pub(crate) fn warn_unwoken(ctx: &Ctx, case: &Case, opening: bool) {
    let (worker, labels) = (case.worker(), case.labels());
    if labels.is_empty() || case.kind() == Kind::Message {
        return;
    }
    let listening = ctx.store.waits().unwrap_or_default();
    let waits = unwoken(&listening, case.kind(), worker, labels);
    if waits.is_empty() {
        return;
    }
    // Two waits of one worker on the same labels are said once.
    let mut who: Vec<String> = Vec::new();
    for wait in &waits {
        let shared: Vec<&str> = wait
            .labels
            .iter()
            .filter(|x| labels.contains(x))
            .map(String::as_str)
            .collect();
        let one = format!("label {} ({})", shared.join(", "), wait.worker);
        if !who.contains(&one) {
            who.push(one);
        }
    }
    let who = who.join("; ");
    // A wait for the case's own worker that does not wake is left out by
    // its --kind, and another --worker is no fix for it.
    let others: Vec<&Wait> = waits
        .iter()
        .copied()
        .filter(|w| w.worker != worker)
        .collect();
    let (why, advice) = if let Some(first) = others.first() {
        let not = if worker.is_empty() {
            "this case has none".to_owned()
        } else {
            format!("not {worker}")
        };
        let mut workers: Vec<&str> = others.iter().map(|w| w.worker.as_str()).collect();
        workers.dedup();
        let advice = if opening {
            // --worker names the session that waits on the case, so the
            // listener's name is the fix only when it is the opener's own.
            format!(
                "If {} is your session, open it with --worker {}; if not, wait on it with --id.",
                first.worker, first.worker
            )
        } else {
            "Wait on it with --id.".to_owned()
        };
        (
            format!("wait for --worker {}, {not}", workers.join(", ")),
            advice,
        )
    } else {
        (
            format!("wait for other kinds than {}", case.kind().as_str()),
            "Wait on it with --id.".to_owned(),
        )
    };
    warn(format_args!(
        "the sessions listening on {who} {why}, so this case's answer wakes none of them. {advice}"
    ));
}

/// Who a message to `worker` and `labels` goes to, for people: the worker,
/// then the labels.
pub(crate) fn addressee(worker: &str, labels: &[String]) -> String {
    let mut names: Vec<String> = Vec::new();
    if !worker.is_empty() {
        names.push(worker.to_owned());
    }
    names.extend(labels.iter().map(|label| format!("label {label}")));
    names.join(" or ")
}

/// Prints a warning to stderr. A command goes on after one.
pub(crate) fn warn(message: impl fmt::Display) {
    let _ = writeln!(io::stderr(), "warning: {message}");
}

/// Warns about each row of the case `key` that the fold skipped.
pub(crate) fn warn_problems(key: Key, case: &Case) {
    for problem in case.problems() {
        warn(format_args!("{key}: {}", output::problem(problem)));
    }
}

/// The case `text` names by its whole id, with or without a title slug
/// after it, as agent commands take one.
///
/// # Errors
///
/// When `text` is no case's id.
pub(crate) fn exact(text: &str) -> Result<Key, Error> {
    text.parse()
        .map_err(|_| Error::new(format!("no case {text:?}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use cases_core::Time;
    use cases_store_sqlite::Db;

    use super::*;

    /// The workers of `waits`.
    fn workers(waits: &[&Wait]) -> Vec<String> {
        waits.iter().map(|w| w.worker.clone()).collect()
    }

    /// `labels` as owned strings.
    fn labels(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|l| (*l).to_owned()).collect()
    }

    /// A store in `dir`, made by opening a case, whose clock reads `secs`
    /// seconds past a fixed time.
    fn clocked(dir: &tempfile::TempDir, secs: &Arc<AtomicI64>) -> Db {
        let secs = Arc::clone(secs);
        let start = "2026-09-27T12:00:00Z".parse::<Time>().unwrap().unix_secs();
        let db = Db::new(dir.path().join("cases.db"))
            .with_clock(move || Time::from_unix(start + secs.load(Ordering::SeqCst), 0).unwrap());
        let mut open = cases_core::Open::default();
        open.kind = "fyi".into();
        open.urgency = "today".into();
        open.title = "T".into();
        let append =
            cases_core::Append::new(cases_core::Author::Agent, &cases_core::Record::Open(open));
        db.create(&append, &Writer::new(Some("lead"))).unwrap();
        db
    }

    #[test]
    fn a_wait_on_the_case_s_label_for_another_worker_leaves_its_answer_unwoken() {
        let dir = tempfile::tempdir().unwrap();
        let secs = Arc::new(AtomicI64::new(0));
        let db = clocked(&dir, &secs);
        let homeday = labels(&["homeday"]);
        let decision = Kind::Decision;
        db.listen(
            "w1",
            "homeday",
            &labels(&["homeday", "email-triage"]),
            &[],
            None,
        )
        .unwrap();
        let now = db.waits().unwrap();
        // Its label, another worker's name, or none: the incident.
        assert_eq!(
            workers(&unwoken(&now, decision, "ci-runner", &homeday)),
            ["homeday"]
        );
        assert_eq!(workers(&unwoken(&now, decision, "", &homeday)), ["homeday"]);
        // Its own worker name wakes it.
        assert!(unwoken(&now, decision, "homeday", &homeday).is_empty());
        // No label shared: nobody was waiting for it, so there is nothing to say.
        assert!(unwoken(&now, decision, "ci-runner", &labels(&["deps"])).is_empty());

        // A wait for the worker on another label does not wake for it.
        db.listen("w2", "ci-runner", &labels(&["deps"]), &[], None)
            .unwrap();
        let now = db.waits().unwrap();
        assert_eq!(
            workers(&unwoken(&now, decision, "ci-runner", &homeday)),
            ["homeday"]
        );
        // A wait of the same worker on no label takes its every case, beside
        // one on labels: each wait is its own row, so they no longer read as
        // one wait on the labels, and nothing is said.
        db.listen("w3", "ci-runner", &[], &[], None).unwrap();
        let now = db.waits().unwrap();
        assert!(unwoken(&now, decision, "ci-runner", &homeday).is_empty());

        // A wait takes only its kinds: one for messages alone does not wake
        // for a decision's answer.
        db.listen("w4", "lead", &homeday, &labels(&["message"]), None)
            .unwrap();
        let now = db.waits().unwrap();
        assert_eq!(
            workers(&unwoken(&now, decision, "lead", &homeday)),
            ["homeday", "lead"]
        );
        db.listen(
            "w4",
            "lead",
            &homeday,
            &labels(&["message", "decision"]),
            None,
        )
        .unwrap();
        let now = db.waits().unwrap();
        assert!(unwoken(&now, decision, "lead", &homeday).is_empty());

        // A wait that has lapsed is not listening.
        let lapse = i64::try_from(cases_store_sqlite::LAPSE.as_secs()).unwrap();
        secs.store(lapse + 1, Ordering::SeqCst);
        let now = db.waits().unwrap();
        assert!(unwoken(&now, decision, "ci-runner", &homeday).is_empty());
    }
}
