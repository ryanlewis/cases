//! `cases wait`: blocks until a case is the agent's turn, or the human's,
//! and prints the cases waiting (AG-03 to AG-06). A wait that listens takes
//! the messages sent to its worker or labels, and keeps a listening record
//! while it runs (AG-16, ST-16). A steady or patient wait keeps collecting
//! after the first event, so a burst of answers wakes the agent once (AG-18).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use cases_core::Store as _;
use cases_core::{
    Answer, Author, Case, Event, EventType, Kind, Park, Pickup, Record, State, Time, Urgency,
    Version, Withdraw,
};
use cases_store_sqlite::{Cases, HEARTBEAT, Key, Writer};
use serde::Serialize;

use crate::commands::doing;
use crate::commands::list::Filter;
use crate::commands::sweep::WRITE_PAUSE;
use crate::commands::write;
use crate::ctx::{self, Ctx};
use crate::duration;
use crate::error::{Error, Status};
use crate::forward::FarLocks;
use crate::output::{self, Actors, CaseJson};
use crate::store::{AnyStore, Failure, HOLD_MAX, Poller};

pub(crate) mod lock;

/// How long after a listening write that failed a wait tries it again: a
/// heartbeat, so that a fault that lasts costs the store no more writes
/// than a wait that works, rather than one at every read, while a worker
/// it could not record is tried again well inside the time a record lasts.
const RETRY: Duration = HEARTBEAT;

/// How long wait sleeps between reads of a local store. A remote store is
/// followed by the held poll instead.
const POLL: Duration = Duration::from_secs(1);

/// The shortest hold of a held poll: a shorter one asks for the version at
/// once, so the time left is slept first.
const HOLD_MIN: Duration = Duration::from_secs(1);

/// The first and the longest pause before a wait tries a server it could
/// not reach again; the pause doubles between them.
const OFFLINE_PAUSE: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(15));

/// The arguments of `cases wait`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Whose turn to wait for.
    #[arg(long = "for", value_name = "SIDE", value_enum, default_value_t = Side::Agent)]
    for_: Side,

    /// Also count events written after this time, even those in the store
    /// when wait started: an RFC 3339 time, or a case id for the time that
    /// case was opened. Without it, only events that land while waiting
    /// count.
    #[arg(long, value_name = "TIME|ID")]
    since: Option<String>,

    /// Give up after this long, such as 30s or 10m: nothing on stdout, one
    /// line on stderr, exit 2. 0 waits for ever.
    #[arg(long, value_name = "DURATION", value_parser = duration::parse, default_value = "0")]
    timeout: Duration,

    /// Only this case, by its whole id. Repeat for any of them.
    #[arg(long = "id", value_name = "ID")]
    ids: Vec<String>,

    #[command(flatten)]
    filter: Filter,

    /// Pick up each answered case before printing it, as pickup does. A case
    /// that changed after wait read it is not picked up, and is printed as
    /// read, with a warning. Needs --id, --label or --worker, and refuses a
    /// blank --worker. With --worker, no --id, and no --kind that leaves
    /// out message, wait listens: it takes each message the human sends to
    /// one of its workers or labels.
    #[arg(long)]
    pickup: bool,

    /// With --pickup: who picked the cases up, such as the agent session's
    /// name.
    #[arg(long, value_name = "NAME")]
    by: Option<String>,

    /// Print each case whole, as show --json gives it without its url,
    /// rather than the short line: which case, where it stands, and its
    /// answer, park or withdraw, with all a message says.
    #[arg(long)]
    full: bool,

    /// With a wait that listens: what you are working on, one line of at
    /// most 80 characters, shown beside your worker in the human's inbox
    /// while you listen. cases doing changes it.
    #[arg(long, value_name = "TEXT", value_parser = doing::line())]
    doing: Option<String>,

    /// How long to keep collecting after the first event, so that answers
    /// given together are printed together. It stops early when no case it
    /// waits on is still open, on the answer to a blocking case, on a
    /// withdraw, and at --timeout, which then prints what it has and exits
    /// 0. Default: the config file's mood, then eager.
    #[arg(long, value_name = "MOOD", value_enum)]
    mood: Option<Mood>,
}

/// How long a wait keeps collecting after the first event that would wake it
/// (AG-18). The numbers may change; the order will not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Mood {
    /// Print as soon as an event wakes it.
    Eager,
    /// Print once nothing new has come for 10s, or 1m after the first event.
    Steady,
    /// Print once nothing new has come for 30s, or 5m after the first event.
    Patient,
}

impl Mood {
    /// How long to wait for the next event before printing, and how long
    /// after the first event to print at the latest; none when eager.
    fn linger(self) -> Option<(Duration, Duration)> {
        match self {
            Mood::Eager => None,
            Mood::Steady => Some((Duration::from_secs(10), Duration::from_secs(60))),
            Mood::Patient => Some((Duration::from_secs(30), Duration::from_secs(300))),
        }
    }
}

/// The mood the config file gives as `text`.
///
/// # Errors
///
/// When `text` is not a mood.
pub(crate) fn mood(text: &str) -> Result<Mood, Error> {
    <Mood as clap::ValueEnum>::from_str(text, false).map_err(|_| {
        Error::new(format!(
            "the config file's mood: {text:?} is not eager, steady or patient"
        ))
    })
}

/// A steady or patient wait that has seen the first event that would wake
/// it, and is collecting the rest (AG-18).
#[derive(Debug)]
struct Collecting {
    /// When it saw the first event.
    first: Instant,
    /// When it last saw an event it had not seen.
    last: Instant,
    /// The events it has seen, by case and seq.
    seen: HashSet<(CaseId, u64)>,
}

impl Collecting {
    fn new(now: Instant) -> Collecting {
        Collecting {
            first: now,
            last: now,
            seen: HashSet::new(),
        }
    }

    /// Notes the events in `events` seen `now`: one not seen before starts
    /// the quiet time again.
    fn saw(&mut self, events: impl IntoIterator<Item = (CaseId, u64)>, now: Instant) {
        let before = self.seen.len();
        self.seen.extend(events);
        if self.seen.len() > before {
            self.last = now;
        }
    }

    /// When to print, given the quiet time and the cap: `quiet` after the
    /// last new event, and never later than `cap` after the first.
    fn until(&self, (quiet, cap): (Duration, Duration)) -> Instant {
        (self.last + quiet).min(self.first + cap)
    }
}

/// Whose turn a case is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Side {
    /// A human answered, parked, resumed or withdrew the case.
    Agent,
    /// The agent opened the case, or noted or amended it after the human.
    Human,
}

impl Side {
    /// The side as `--for` spells it.
    fn as_str(self) -> &'static str {
        match self {
            Side::Agent => "agent",
            Side::Human => "human",
        }
    }

    /// The event that makes `case` this side's turn, if one does.
    fn turn(self, case: &Case) -> Option<&Event> {
        match self {
            Side::Agent => agent_turn(case),
            Side::Human => human_turn(case),
        }
    }
}

/// The event that makes `case` the agent's turn, if one does: its last
/// event, when that is a human answer, park, resume or withdraw. Once the
/// agent picks up, notes, amends, closes or withdraws, it is not, and nor is
/// a message the human takes back while it is sent, which no agent had. When
/// a message is a listening wait's turn is [`Listening`]'s to say.
fn agent_turn(case: &Case) -> Option<&Event> {
    let last = case.events().last()?;
    (last.author == Author::Human
        && matches!(
            last.event(),
            EventType::Answer | EventType::Park | EventType::Resume | EventType::Withdraw
        )
        && last.from != Some(State::Sent))
    .then_some(last)
}

/// The event that makes `case` the human's turn, if one does: the case is
/// open and its last event is the agent's. An amend changes a case that was
/// waiting already, so the event is the agent's event before any amends at
/// the end; an amend after a human event, such as a resume, is the event
/// itself.
fn human_turn(case: &Case) -> Option<&Event> {
    let (mut event, before) = case.events().split_last()?;
    if case.state() != State::Open || event.author != Author::Agent {
        return None;
    }
    for previous in before.iter().rev() {
        if event.event() != EventType::Amend || previous.author != Author::Agent {
            break;
        }
        event = previous;
    }
    Some(event)
}

/// A case whose turn it is, as wait found it.
#[derive(Debug)]
struct Turn {
    key: Key,
    case: Case,
    /// The time of the event that made it the turn, if its record has one.
    at: Option<Time>,
    /// Whether that event is new to this wait: written since its first read
    /// of the store, or after `--since`.
    fresh: bool,
    /// Whether wait took the case, a message, before printing anything: its
    /// pickup is written already.
    taken: bool,
}

/// A case as wait prints it: short, or with `--full` as `show --json`
/// prints it without its url; then whether the event that made it the turn
/// is new to this wait, and the `--since` for the next wait. That is the
/// latest time among the events that made the printed cases the turn, or the
/// `--since` given if it is later, so the next wait does not wake again on
/// what this one printed.
#[derive(Debug, Serialize)]
struct Line<'a> {
    #[serde(flatten)]
    case: Shown<'a>,
    fresh: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_since: Option<Time>,
}

/// The case in a line.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Shown<'a> {
    Short(Box<Short<'a>>),
    Full(Box<CaseJson<'a>>),
}

/// What an agent acts on when a case is its turn (AG-04): which case, where
/// it stands, and the human's answer, park or withdraw. A message is the
/// human's instruction, so its line also gives all the human wrote in it.
/// The rest is `show`'s, or `--full`'s.
#[derive(Debug, Serialize)]
struct Short<'a> {
    id: String,
    state: &'static str,
    kind: &'static str,
    /// A message's only.
    #[serde(skip_serializing_if = "Option::is_none")]
    urgency: Option<&'static str>,
    title: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    body: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    context: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    links: &'a [String],
    #[serde(skip_serializing_if = "str::is_empty")]
    re: &'a str,
    /// A briefing's only: whether its work has landed.
    #[serde(skip_serializing_if = "Option::is_none")]
    landed: Option<bool>,
    /// A briefing's left-out items, which its answer's `file` numbers from 1.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    left_out: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<&'a Answer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    park: Option<&'a Park>,
    /// The withdraw's record, whose reason may say what to do instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    withdraw: Option<&'a Withdraw>,
    revision: u64,
    amend_seq: u64,
}

impl<'a> Short<'a> {
    fn new(key: Key, case: &'a Case) -> Short<'a> {
        let message = case.kind() == Kind::Message;
        let mine = |text: &'a str| if message { text } else { "" };
        Short {
            id: key.to_string(),
            state: case.state().as_str(),
            kind: case.kind().as_str(),
            urgency: message.then(|| case.urgency().as_str()),
            title: case.title(),
            body: mine(case.body()),
            context: mine(case.context()),
            links: if message { case.links() } else { &[] },
            re: mine(case.re()),
            landed: (case.kind() == Kind::Briefing).then(|| case.landed()),
            left_out: case.left_out(),
            answer: case.answer(),
            park: case.park(),
            withdraw: withdrawn(case),
            revision: case.revision(),
            amend_seq: case.amend_seq(),
        }
    }
}

/// The record of the withdraw that ended `case`, when it is withdrawn.
fn withdrawn(case: &Case) -> Option<&Withdraw> {
    if case.state() != State::Withdrawn {
        return None;
    }
    case.events()
        .iter()
        .rev()
        .find_map(|event| match &event.record {
            Record::Withdraw(withdraw) => Some(withdraw),
            _ => None,
        })
}

/// Waits until a case is the turn of the side asked for because of an event
/// new to this wait, then prints every case that is that side's turn and
/// matches the filters, one JSON object to a line.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    check_pickup(args)?;
    let ids = args
        .ids
        .iter()
        .map(|id| ctx::exact(id))
        .collect::<Result<Vec<Key>, Error>>()?;
    let since = match &args.since {
        Some(text) => since(ctx, text)?,
        None => None,
    };
    let mood = match (args.mood, ctx.config.get("mood")) {
        (Some(mood), _) => mood,
        (None, Some(text)) => mood(text)?,
        (None, None) => Mood::Eager,
    };
    // Held until wait has printed what it picked up. A store not there yet
    // gets no directory of locks either, as a wait makes nothing (ST-08), so
    // a pickup wait started before the store's first open takes none.
    let _held = match ctx.target.path() {
        Some(store) if args.pickup && store.exists() => {
            Some(lock::take(store, &scopes(args, &ids))?)
        }
        // A remote store's waits are on other computers too: no lock here
        // could tell of them.
        _ => None,
    };
    // A forwarded store's are taken on its own machine, by its dial-stdio,
    // and held there while this wait's connection for them is open and it
    // renews their lease. A wait whose lease may have run out stops before
    // the far side lets them go, rather than wait on unlocked. One that
    // ends, however it ends, lets them go there at once: the far side keeps
    // them past a connection that ends without that, until the lease runs
    // out.
    let held_there = match &ctx.store {
        AnyStore::Forward(forwarded) if args.pickup => Some(forwarded.hold(&scopes(args, &ids))?),
        _ => None,
    };
    let far = held_there.as_ref();
    let turns = wait(args, ctx, &ids, since, mood, far)?;
    print(args, ctx, since, turns, far)
}

/// What a pickup wait locks: each worker it was given or, with none, each
/// label, and each id. A label is shared by the workers that listen on it,
/// and the first session to take a message to it has it (AG-16), so a wait
/// with a worker leaves its labels unlocked.
fn scopes(args: &Args, ids: &[Key]) -> Vec<(&'static str, String)> {
    let named: Vec<(&'static str, String)> = if args.filter.workers.is_empty() {
        let labels = args.filter.labels.iter();
        labels.map(|label| ("label", label.clone())).collect()
    } else {
        let workers = args.filter.workers.iter();
        workers.map(|worker| ("worker", worker.clone())).collect()
    };
    let ids = ids.iter().map(|id| ("id", id.to_string()));
    named.into_iter().chain(ids).collect()
}

/// Refuses --by without --pickup, --doing where wait does not listen, and
/// --pickup where it has nothing to pick up or could pick up other agents'
/// cases (AG-05). A blank --worker, as `"$CASES_WORKER"` gives when the
/// variable is not set, matches every case opened without a worker, so it
/// does not name the agent's own cases.
fn check_pickup(args: &Args) -> Result<(), Error> {
    let refusal = if args.by.is_some() && !args.pickup {
        "--by needs --pickup"
    } else if args.doing.is_some() && !listens(args) {
        "--doing needs a wait that listens for messages: --pickup and --worker, with no --id, \
         and no --kind that leaves out message"
    } else if !args.pickup {
        return Ok(());
    } else if args.for_ == Side::Human {
        "--pickup is only for --for agent: a case waiting on the human has no answer to pick up"
    } else if args.filter.workers.iter().any(|w| w.trim().is_empty()) {
        "--pickup refuses a blank --worker: it would pick up every case opened without a \
         worker, other agents' included"
    } else if args.filter.labels.iter().any(|l| l.trim().is_empty()) {
        "--pickup refuses a blank --label: no case or message has one, so it names none of \
         your own"
    } else if args.ids.is_empty() && args.filter.labels.is_empty() && args.filter.workers.is_empty()
    {
        "--pickup needs --id, --label or --worker, so that it picks up only your own cases"
    } else {
        return Ok(());
    };
    Err(Error::new(refusal))
}

/// The time `--since` gives: an RFC 3339 time, or a case id for the time
/// that case was opened, `None` when its open has none.
fn since(ctx: &Ctx, text: &str) -> Result<Option<Time>, Error> {
    if let Ok(at) = text.parse::<Time>() {
        return Ok(Some(at));
    }
    let key = ctx::exact(text).map_err(|_| {
        Error::new(format!(
            "--since {text:?} is neither an RFC 3339 time nor a case id"
        ))
    })?;
    let case = ctx
        .case(key)
        .map_err(|error| Error::new(format!("--since {text:?}: {error}")))?;
    Ok(case.opened_at())
}

/// A case as wait tells cases apart: its key, and when it was opened. A store
/// thrown away and made again at the path while wait runs gives its cases the
/// keys the old store's had, from 1, so a key alone could name another case.
type CaseId = (Key, Option<Time>);

/// What wait's first read of the store found.
#[derive(Debug, Default)]
struct First {
    /// Its events, by case and seq: none of them is new by appearing.
    events: HashSet<(CaseId, u64)>,
    /// The case each `--id` names, by its key: the one wait first found with
    /// that key, and not a case a store made again since gives the same key,
    /// which is another agent's (AG-05).
    named: HashMap<Key, CaseId>,
}

impl First {
    /// What `cases` holds, and the cases in it that `ids` names.
    fn read(cases: &Cases, ids: &[Key]) -> First {
        let mut first = First::default();
        for (key, case) in cases {
            let Ok(case) = case else { continue };
            let id = (*key, case.opened_at());
            first
                .events
                .extend(case.events().iter().map(|event| (id, event.seq)));
            if ids.contains(key) {
                first.named.insert(*key, id);
            }
        }
        first
    }
}

/// Reads the store, every [`POLL`] for a local one and as its held poll
/// says it changed for a remote one, until a case that matches is the turn of
/// the side asked for because of an event new to this wait, and gives every
/// case that matches and is that side's turn then. The events in the store
/// at the first read are not new, unless they are after `since`; every event
/// of a store that is not there yet at the first read, or is made again
/// after it, is. A wait that listens also records itself as listening, and
/// wakes on each message it takes.
///
/// A steady or patient `mood` reads on after that first new event, and
/// gives the cases once the first of these comes: its quiet time with no
/// event new to it, its cap after the first, no case it waits on still open,
/// the answer to a blocking case or a withdraw among the new events, or the
/// timeout (AG-18).
///
/// An event is judged new by its appearing, not by its time alone, since a
/// record can give a time well before it was stored.
///
/// On a remote store, no request or retry goes past the timeout. After a
/// first read, a server that cannot be reached, or answers 429, 502, 503 or
/// 504, is waited out until the timeout, or the end of a lingering wait's
/// collecting, and said once on stderr; a first read that fails fails the
/// wait at once, so a wrong store is not waited on.
///
/// A pickup wait on a forwarded store checks its locks there, `far`, before
/// each read and each message it takes, and holds no poll and pauses no
/// longer than they are held for. Once they may have been let go it takes
/// nothing more: it gives the messages it took, or fails with none.
fn wait(
    args: &Args,
    ctx: &Ctx,
    ids: &[Key],
    since: Option<Time>,
    mood: Mood,
    far: Option<&FarLocks>,
) -> Result<Vec<Turn>, Error> {
    let db = &ctx.store;
    let deadline = deadline(args.timeout);
    db.until(deadline);
    let asked = Asked {
        args,
        ctx,
        ids,
        since,
        mood,
        far,
        deadline,
    };
    let mut offline = Offline::default();
    let mut poller = db.poller();
    let none = Cases::new();
    let mut first = None;
    let mut warned = HashSet::new();
    let mut unfolded = HashMap::new();
    let mut listening = Listening::of(args);
    // The messages this wait took: a later read finds them picked up, so
    // they are kept here until the wait gives its cases.
    let mut taken: Vec<Turn> = Vec::new();
    let mut collecting: Option<Collecting> = None;
    // What a lingering wait collected at its last read, beside the
    // messages it took: what it has, should its timeout pass while the
    // store cannot be read.
    let mut collected: Vec<Turn> = Vec::new();
    loop {
        if let Some(error) = lapsed(far) {
            return stop(taken, error);
        }
        let read = asked.read(
            &mut poller,
            &none,
            first.is_some(),
            collecting.as_ref(),
            &mut offline,
        );
        let cases = match read {
            Ok(Some(cases)) => cases,
            Ok(None) => continue,
            Err(error) => return failed(error, collecting.is_some(), collected, taken),
        };
        warn_once(db, cases, &mut warned, &mut unfolded);
        if let Some(listening) = &mut listening {
            listening.beat(db, &mut warned);
        }
        let first = first.get_or_insert_with(|| First::read(cases, ids));
        let found = match asked.find(listening.as_ref(), first, cases, &mut taken, &mut warned) {
            Ok(found) => found,
            Err(error) => return stop(taken, error),
        };
        let woke = !taken.is_empty() || found.woke();
        if woke && asked.done(&mut collecting, &found, &taken) {
            return Ok(found.give(taken));
        }
        // What it was collecting has gone, such as a case another wait
        // picked up: it waits as it did before the first event.
        if !woke {
            collecting = None;
        }
        collected = if collecting.is_some() {
            found.give(Vec::new())
        } else {
            Vec::new()
        };
        if let Err(error) = asked.rest(&poller, collecting.as_ref(), &mut offline) {
            return failed(error, collecting.is_some(), collected, taken);
        }
    }
}

/// When a wait with `timeout` gives up, if it does: a timeout of 0, or one
/// too long to add to the clock, waits for ever.
fn deadline(timeout: Duration) -> Option<Instant> {
    (!timeout.is_zero())
        .then(|| Instant::now().checked_add(timeout))
        .flatten()
}

/// What a wait was asked: its flags, the cases its `--id`s name, the time
/// `--since` gives, its mood, a pickup wait's locks on the far side of a
/// forwarded store, `far`, and when it gives up, if it does.
struct Asked<'a> {
    args: &'a Args,
    ctx: &'a Ctx,
    ids: &'a [Key],
    since: Option<Time>,
    mood: Mood,
    far: Option<&'a FarLocks>,
    deadline: Option<Instant>,
}

/// What a read of the store gives a wait, beside the messages it took.
struct Found<'c> {
    /// Each case that matches and is the side's turn, with the event that
    /// makes it so, and whether that event is new to this wait.
    turns: Vec<(Key, &'c Case, &'c Event, bool)>,
    /// The messages it took that the human has withdrawn since, while the
    /// withdraw is new (AG-04).
    withdrawn: Vec<Turn>,
    /// Whether a case this wait waits on is still open, so that another
    /// answer can come.
    open: bool,
}

impl Asked<'_> {
    /// The store as `poller`'s next read finds it: `none`, no cases, when
    /// it is not there yet (ST-08), or nothing, for the wait to read again,
    /// when a read after the first, as `read_before` says this is, fails in
    /// a way [`Offline`] rides out. A wait `collecting` rides it out no
    /// longer than its linger, so that it gives what it has then (AG-18).
    ///
    /// # Errors
    ///
    /// When the read fails for good, or the timeout, or the linger of a
    /// wait collecting, passes while a failing store is ridden out.
    fn read<'p>(
        &self,
        poller: &'p mut Poller,
        none: &'p Cases,
        read_before: bool,
        collecting: Option<&Collecting>,
        offline: &mut Offline,
    ) -> Result<Option<&'p Cases>, Error> {
        match poller.poll() {
            Ok(cases) => {
                offline.reached();
                Ok(Some(cases))
            }
            // The store is not there yet: wait for it (ST-08).
            Err(error) if error.is_missing() => Ok(Some(none)),
            Err(error) if read_before && error.is_transient() => {
                let until = until(self.deadline, self.mood, collecting);
                offline.pause(&error, until, self.args, self.far)?;
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// What `cases`, the store as a read found it, holds for this wait. A
    /// message it takes, it adds to `taken`, owing it to the far side's
    /// locks while it writes the pickup, when it holds them.
    ///
    /// # Errors
    ///
    /// When a pickup wait's locks on the far side may have been let go.
    fn find<'c>(
        &self,
        listening: Option<&Listening>,
        first: &mut First,
        cases: &'c Cases,
        taken: &mut Vec<Turn>,
        warned: &mut HashSet<String>,
    ) -> Result<Found<'c>, Failure> {
        let mut found = Found {
            turns: Vec::new(),
            withdrawn: Vec::new(),
            open: false,
        };
        for (key, case) in cases {
            let Ok(case) = case else { continue };
            // A message is this wait's only when it listens, and then by
            // whom it is sent to or who took it, not by the filters.
            if case.kind() == Kind::Message {
                if let Some(listening) = listening {
                    let writing = self.far.map(FarLocks::write).transpose()?;
                    if let Some(turn) =
                        listening.message(self.ctx, first, self.since, *key, case, warned)
                    {
                        if turn.taken {
                            if let Some(writing) = &writing {
                                writing.owe(turn.key);
                            }
                            taken.push(turn);
                        } else {
                            found.withdrawn.push(turn);
                        }
                    }
                }
                continue;
            }
            if !self.matches(first, *key, case) {
                continue;
            }
            found.open |= case.state() == State::Open;
            if let Some((event, fresh)) = self.turn_of(first, *key, case) {
                found.turns.push((*key, case, event, fresh));
            }
        }
        Ok(found)
    }

    /// Whether `case`, by its `key`, is one this wait waits on: one its
    /// `--id`s name, if it has any, that its filters match. An id names the
    /// case found with its key at the first read or, when there was none
    /// then, such as in a store not there yet, the first case found with it
    /// since.
    fn matches(&self, first: &mut First, key: Key, case: &Case) -> bool {
        let id = (key, case.opened_at());
        let named = self.ids.contains(&key) && *first.named.entry(key).or_insert(id) == id;
        (self.ids.is_empty() || named) && self.args.filter.matches(case)
    }

    /// The event that makes `case`, by its `key`, the side's turn, and
    /// whether it is new to this wait: not in its first read, or after
    /// `--since`. A case the human withdrew is the agent's turn once: while
    /// the withdraw is new (AG-04).
    fn turn_of<'c>(&self, first: &First, key: Key, case: &'c Case) -> Option<(&'c Event, bool)> {
        let event = self.args.for_.turn(case)?;
        let id = (key, case.opened_at());
        let fresh = !first.events.contains(&(id, event.seq))
            || self
                .since
                .is_some_and(|since| event.at().is_some_and(|at| at > since));
        (event.event() != EventType::Withdraw || fresh).then_some((event, fresh))
    }

    /// Whether a wait that woke on what it `found` and `taken` gives its
    /// cases now: an eager one at once; one that lingers, which notes in
    /// `collecting` what it saw, once its linger ends, nothing it waits on
    /// is open, something urgent came, or the timeout passed (AG-18).
    fn done(&self, collecting: &mut Option<Collecting>, found: &Found, taken: &[Turn]) -> bool {
        let Some(linger) = self.mood.linger() else {
            return true;
        };
        let now = Instant::now();
        let collecting = collecting.get_or_insert_with(|| Collecting::new(now));
        let seen = |turn: &Turn| {
            let seq = turn.case.events().last().map_or(0, |event| event.seq);
            ((turn.key, turn.case.opened_at()), seq)
        };
        collecting.saw(
            found
                .turns
                .iter()
                .filter(|&&(.., fresh)| fresh)
                .map(|&(key, case, event, _)| ((key, case.opened_at()), event.seq))
                .chain(taken.iter().chain(&found.withdrawn).map(seen)),
            now,
        );
        found.urgent()
            || !found.open
            || collecting.until(linger) <= now
            || self.deadline.is_some_and(|deadline| deadline <= now)
    }

    /// Waits until the next read: a remote or forwarded store's held poll
    /// says it changed, or a local store's [`POLL`] passes, no longer than a
    /// pickup wait's locks on the far side are held for. A wait `collecting`
    /// stops at the end of its linger, or the timeout, whichever is first;
    /// one that is not stops only at the timeout.
    ///
    /// # Errors
    ///
    /// When the timeout passes with nothing collected, or a held poll fails
    /// for good.
    fn rest(
        &self,
        poller: &Poller,
        collecting: Option<&Collecting>,
        offline: &mut Offline,
    ) -> Result<(), Error> {
        let db = &self.ctx.store;
        let until = until(self.deadline, self.mood, collecting);
        if let (true, Some(version)) = (db.follows(), poller.version()) {
            return match follow(db, &version, until, self.args, offline, self.far) {
                // Its linger has ended: the next read gives the cases.
                Err(error) if collecting.is_some() && error.status() == Status::Timeout => Ok(()),
                result => result,
            };
        }
        let mut pause = POLL;
        if let Some(held) = held_for(self.far) {
            pause = pause.min(held);
        }
        if let Some(left) = left(until) {
            if left.is_zero() && collecting.is_none() {
                return Err(timed_out(self.args));
            }
            pause = pause.min(left);
        }
        thread::sleep(pause);
        Ok(())
    }
}

impl Found<'_> {
    /// Whether it wakes the wait: a message withdrawn, or a turn new to it.
    fn woke(&self) -> bool {
        !self.withdrawn.is_empty() || self.turns.iter().any(|&(.., fresh)| fresh)
    }

    /// Whether it ends a lingering wait at once: the agent cannot go on
    /// without a blocking case's answer, and the sooner it hears of a
    /// withdraw, the less work it throws away.
    fn urgent(&self) -> bool {
        !self.withdrawn.is_empty()
            || self.turns.iter().any(|&(_, case, event, fresh)| {
                fresh
                    && (event.event() == EventType::Withdraw
                        || event.event() == EventType::Answer
                            && case.urgency() == Urgency::Blocking)
            })
    }

    /// The cases it gives, with the messages the wait `taken`. A message
    /// taken and then withdrawn while collecting is given withdrawn.
    fn give(self, taken: Vec<Turn>) -> Vec<Turn> {
        let keys: HashSet<Key> = self.withdrawn.iter().map(|turn| turn.key).collect();
        self.turns
            .into_iter()
            .map(|(key, case, event, fresh)| Turn {
                key,
                case: case.clone(),
                at: event.at(),
                fresh,
                taken: false,
            })
            .chain(self.withdrawn)
            .chain(taken.into_iter().filter(|turn| !keys.contains(&turn.key)))
            .collect()
    }
}

/// When a wait in `mood` stops waiting on the store: once it is
/// `collecting`, at the end of its linger or the `deadline`, whichever is
/// first; before that, at the `deadline`, if there is one.
fn until(
    deadline: Option<Instant>,
    mood: Mood,
    collecting: Option<&Collecting>,
) -> Option<Instant> {
    match (collecting, mood.linger()) {
        (Some(collecting), Some(linger)) => {
            let until = collecting.until(linger);
            Some(deadline.map_or(until, |deadline| deadline.min(until)))
        }
        _ => deadline,
    }
}

/// The time left before `deadline`, if there is one.
fn left(deadline: Option<Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()))
}

/// Why a pickup wait's locks on the far side, `far`, may have been let go,
/// once they may have.
fn lapsed(far: Option<&FarLocks>) -> Option<Failure> {
    far.and_then(|far| far.check().err())
}

/// How long a pickup wait's locks on the far side, `far`, are held for
/// still, if they are on a lease: none once they may have been let go,
/// which the wait's next turn finds.
fn held_for(far: Option<&FarLocks>) -> Option<Duration> {
    far.and_then(|far| far.left().unwrap_or(Some(Duration::ZERO)))
}

/// The end of a pickup wait whose locks on the far side may have been let
/// go, `error`: the messages it `taken` already, which were taken while it
/// held them and no later wait takes again (AG-16), or, with none, `error`.
fn stop(taken: Vec<Turn>, error: Failure) -> Result<Vec<Turn>, Error> {
    if taken.is_empty() {
        return Err(error.into());
    }
    ctx::warn(format_args!("{error}"));
    Ok(taken)
}

/// The end of a wait whose read of the store, or held poll, failed with
/// `error`. A timeout that passed while it was `collecting` gives what it
/// `collected` at its last read, with the messages it `taken` (AG-18). Any
/// other failure after it took messages, which no later wait takes again,
/// gives them, with what it collected so that its `next_since` passes no
/// event it has not printed, and says the failure on stderr (AG-16), as
/// [`stop`] does; with none taken, it is `error`.
fn failed(
    error: Error,
    collecting: bool,
    collected: Vec<Turn>,
    taken: Vec<Turn>,
) -> Result<Vec<Turn>, Error> {
    if !(collecting && error.status() == Status::Timeout) {
        if taken.is_empty() {
            return Err(error);
        }
        ctx::warn(format_args!("{error}"));
    }
    // What it collected holds no message it took: a withdraw of one is
    // urgent, and ended the wait at that read.
    Ok(collected.into_iter().chain(taken).collect())
}

/// The error of a wait whose timeout passed with nothing to report.
fn timed_out(args: &Args) -> Error {
    Error::with_status(
        Status::Timeout,
        format!(
            "no case needed the {} within {:?}",
            args.for_.as_str(),
            args.timeout
        ),
    )
}

/// Holds polls on a remote or forwarded store, each up to [`HOLD_MAX`] and never past the
/// deadline, until its version is not `version`. A hold too short to make
/// is slept, and the version asked for at once at its end. A failure the
/// next try may not have is waited out, as [`Offline`] says. No hold goes
/// past the time a pickup wait's locks on the far side, `far`, are held
/// for.
///
/// # Errors
///
/// When the deadline passes first, or a poll fails for good. Once the
/// locks may have been let go, it ends at once, as if the version had
/// changed, so that the wait's next turn finds it.
fn follow(
    db: &AnyStore,
    version: &Version,
    deadline: Option<Instant>,
    args: &Args,
    offline: &mut Offline,
    far: Option<&FarLocks>,
) -> Result<(), Error> {
    loop {
        if lapsed(far).is_some() {
            return Ok(());
        }
        let held = held_for(far);
        let left = left(deadline);
        if left.is_some_and(|left| left.is_zero()) {
            return Err(timed_out(args));
        }
        let hold = left.map_or(HOLD_MAX, |left| left.min(HOLD_MAX));
        let hold = held.map_or(hold, |held| hold.min(held));
        if hold < HOLD_MIN {
            thread::sleep(hold);
        }
        match db.changes(version, hold) {
            Ok(now) => {
                offline.reached();
                if now != *version {
                    return Ok(());
                }
            }
            Err(error) if error.is_transient() => offline.pause(&error, deadline, args, far)?,
            Err(error) => return Err(error.into()),
        }
    }
}

/// How a wait rides out a server it read once and cannot reach now: it
/// says so once, on stderr, and tries again after a pause that grows from
/// the first of [`OFFLINE_PAUSE`] to the second, or the server's
/// `Retry-After` when longer, but never past the deadline.
#[derive(Debug, Default)]
struct Offline {
    said: bool,
    pause: Duration,
}

impl Offline {
    /// The server answered: the next pause is the first again.
    fn reached(&mut self) {
        self.pause = Duration::ZERO;
    }

    /// Says `error` once in the wait and pauses before the next try, for
    /// no longer than a pickup wait's locks on the far side, `far`, are
    /// held for.
    ///
    /// # Errors
    ///
    /// When the deadline has passed: the wait times out, and the failure
    /// is not said. Once the locks may have been let go, it does not pause,
    /// so that the wait's next turn finds it.
    fn pause(
        &mut self,
        error: &Failure,
        deadline: Option<Instant>,
        args: &Args,
        far: Option<&FarLocks>,
    ) -> Result<(), Error> {
        if lapsed(far).is_some() {
            return Ok(());
        }
        let held = held_for(far);
        let left = left(deadline);
        if left.is_some_and(|left| left.is_zero()) {
            return Err(timed_out(args));
        }
        if !self.said {
            self.said = true;
            let until = if deadline.is_some() {
                " until the timeout"
            } else {
                ""
            };
            ctx::warn(format_args!(
                "{error}; the wait goes on, trying again{until}"
            ));
        }
        let (first, most) = OFFLINE_PAUSE;
        self.pause = (self.pause * 2).clamp(first, most);
        let mut pause = self.pause;
        if let Some(after) = error.retry_after() {
            pause = pause.max(after);
        }
        if let Some(left) = left {
            pause = pause.min(left);
        }
        if let Some(held) = held {
            pause = pause.min(held);
        }
        thread::sleep(pause);
        Ok(())
    }
}

/// Whether `error`, from a worker's listening write, is about that worker or
/// the doing line given with it, which the store refused: the wait's other
/// workers may still be recorded. Any other failure is the store's own, and
/// would meet them too.
fn about_the_worker(error: &Failure) -> bool {
    use cases_store_remote::Error as RemoteError;
    use cases_store_sqlite::Error as DbError;
    let remote =
        |error: &RemoteError| matches!(error, RemoteError::Invalid(_) | RemoteError::NotListening);
    match error {
        Failure::Local(error) => matches!(
            error,
            DbError::Unnamed | DbError::Doing(_) | DbError::NotListening { .. }
        ),
        Failure::Forward { error, .. } => remote(error),
        #[cfg(feature = "remote")]
        Failure::Remote { error, .. } => remote(error),
        Failure::Named(_) => false,
    }
}

/// Whether a wait with `args` listens for messages: it picks up and names a
/// worker, and neither `--id` nor a `--kind` that leaves out messages scopes
/// it to other cases.
fn listens(args: &Args) -> bool {
    args.pickup
        && !args.filter.workers.is_empty()
        && args.ids.is_empty()
        && (args.filter.kinds.is_empty() || args.filter.kinds.contains(&Kind::Message))
}

/// A wait that listens for messages (AG-16): the id it records itself by,
/// made at its first record, the workers it records as listening, the
/// labels and kinds it waits on, the name it picks messages up as, the doing
/// line it gives on each worker's first record, the workers whose record has
/// had it, and when its next heartbeat is due.
#[derive(Debug)]
struct Listening<'a> {
    id: Option<String>,
    workers: &'a [String],
    labels: &'a [String],
    kinds: Vec<String>,
    by: &'a str,
    doing: Option<&'a str>,
    given: HashSet<&'a str>,
    due: Option<Instant>,
}

impl<'a> Listening<'a> {
    /// How a wait with `args` listens, if it does. It picks messages up as
    /// its `--by`, or its first `--worker` when `--by` is not given or is
    /// blank, as `--by "$VAR"` is when the variable is not set.
    fn of(args: &'a Args) -> Option<Listening<'a>> {
        let worker = args.filter.workers.first()?;
        listens(args).then(|| Listening {
            id: None,
            workers: &args.filter.workers,
            labels: &args.filter.labels,
            kinds: args
                .filter
                .kinds
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect(),
            by: args
                .by
                .as_deref()
                .filter(|by| !by.trim().is_empty())
                .unwrap_or(worker),
            doing: args.doing.as_deref(),
            given: HashSet::new(),
            due: None,
        })
    }

    /// Records the wait's workers as listening, when a heartbeat is due: at
    /// the first read of a store that is there, and every [`HEARTBEAT`]
    /// after (ST-16). A store not there yet is tried again at the next read.
    /// A worker whose write fails is warned about once, the workers after it
    /// are recorded still when the failure is about that worker alone, and
    /// the next beat is a [`RETRY`] later. The record
    /// lapses when the wait stops refreshing it, whether it stops cleanly or
    /// not, so nothing is written when it ends.
    fn beat(&mut self, db: &AnyStore, warned: &mut HashSet<String>) {
        if self.due.is_some_and(|due| Instant::now() < due) {
            return;
        }
        let id = match &self.id {
            Some(id) => id.clone(),
            None => match cases_store_sqlite::new_wait() {
                Ok(id) => self.id.insert(id).clone(),
                Err(error) => {
                    let message = format!("this wait is not recorded as listening: {error}");
                    if warned.insert(message.clone()) {
                        ctx::warn(message);
                    }
                    return;
                }
            },
        };
        let mut failed = false;
        for worker in self.workers {
            // The doing line goes on each worker's first record alone, so
            // that a later `cases doing` is not put back, even by a beat
            // that tries again after another worker's write failed.
            let doing = self.doing.filter(|_| !self.given.contains(worker.as_str()));
            match db.listen(&id, worker, self.labels, &self.kinds, doing) {
                Ok(()) => {
                    self.given.insert(worker);
                }
                Err(error) if error.is_missing() => return,
                Err(error) => {
                    let message = format!("{worker} is not recorded as listening: {error}");
                    if warned.insert(message.clone()) {
                        ctx::warn(message);
                    }
                    failed = true;
                    // A failure of the store would meet each worker after
                    // it, a timeout each on a store that does not answer.
                    if !about_the_worker(&error) {
                        break;
                    }
                }
            }
        }
        let next = if failed { RETRY } else { HEARTBEAT };
        self.due = Some(Instant::now() + next);
    }

    /// The turn `case`, a message, gives this wait, if any: a sent message
    /// for it, which it takes, or one it took that the human has withdrawn
    /// since, while the withdraw is new (AG-04). A sent message is new to
    /// every wait whatever `--since` says, as it waits until one takes it.
    /// A take that fails is warned about once in the wait, however many
    /// reads try it again, in `warned`.
    fn message(
        &self,
        ctx: &Ctx,
        first: &First,
        since: Option<Time>,
        key: Key,
        case: &Case,
        warned: &mut HashSet<String>,
    ) -> Option<Turn> {
        match case.state() {
            State::Sent if self.is_for(case) => self.take(ctx, key, case, warned),
            State::Withdrawn if self.took(case) => {
                let event = agent_turn(case)?;
                let fresh = !first.events.contains(&((key, case.opened_at()), event.seq))
                    || since.is_some_and(|since| event.at().is_some_and(|at| at > since));
                fresh.then(|| Turn {
                    key,
                    case: case.clone(),
                    at: event.at(),
                    fresh,
                    taken: false,
                })
            }
            _ => None,
        }
    }

    /// Whether the message `case` is sent to this wait: to one of its
    /// workers, or with one of its labels.
    fn is_for(&self, case: &Case) -> bool {
        self.workers.iter().any(|worker| worker == case.worker())
            || case
                .labels()
                .iter()
                .any(|label| self.labels.contains(label))
    }

    /// Whether this wait's session took the message `case`: its pickup is
    /// by this wait's name or one of its workers. A pickup with no `by`, as
    /// `cases pickup` without `--by` writes, is the worker's the message went
    /// to, as its actor is (AG-16).
    fn took(&self, case: &Case) -> bool {
        case.pickup().is_some_and(|pickup| {
            let by = if pickup.by.trim().is_empty() {
                case.worker()
            } else {
                pickup.by.as_str()
            };
            by == self.by || self.workers.iter().any(|worker| worker == by)
        })
    }

    /// Takes the sent message `case`: picks it up at the revision wait read
    /// it at, as this wait's name, and gives the turn of the message as it
    /// is after, or `None` when another session took it first or the human
    /// edited or withdrew it since the read. Only a failure of another sort
    /// is said, on stderr, and once in the wait, in `warned`: a store that
    /// cannot be written would otherwise say it at every read. An edited
    /// message is taken at its new revision on the next read.
    fn take(&self, ctx: &Ctx, key: Key, case: &Case, warned: &mut HashSet<String>) -> Option<Turn> {
        let at = case.events().last().and_then(Event::at);
        let mut pickup = Pickup::default();
        self.by.clone_into(&mut pickup.by);
        let writer = Writer::new(Some(self.by));
        let record = Record::Pickup(pickup);
        let revision = Some(case.revision());
        let taken = write::append(ctx, key, Author::Agent, &record, &writer, revision);
        thread::sleep(WRITE_PAUSE);
        match taken {
            Ok(after) => Some(Turn {
                key,
                case: after,
                at,
                fresh: true,
                taken: true,
            }),
            Err(error) => {
                let message = format!("{key}: not taken: {error}");
                if !matches!(error.status(), Status::Stale | Status::Transition)
                    && warned.insert(message.clone())
                {
                    ctx::warn(message);
                }
                None
            }
        }
    }
}

/// Warns about each case that does not fold and each row a case skipped,
/// once in the wait, however many reads find it (AG-04, ST-05). A case that
/// does not fold has its rows read again, to say whether it is perhaps from
/// a newer cases, and only again when why it does not fold changes:
/// `unfolded` keeps, by key, the error last read again for and what it said.
fn warn_once(
    db: &AnyStore,
    cases: &Cases,
    warned: &mut HashSet<String>,
    unfolded: &mut HashMap<Key, (String, String)>,
) {
    for (key, case) in cases {
        let messages = match case {
            Ok(case) => case
                .problems()
                .iter()
                .map(|problem| format!("{key}: {}", output::problem(problem)))
                .collect(),
            Err(error) => {
                let text = error.to_string();
                let why = match unfolded.get(key) {
                    Some((seen, why)) if *seen == text => why.clone(),
                    _ => {
                        let why = match cases_core::Store::rows(db, key) {
                            Ok(Some(rows)) => output::unfolded(&rows, error),
                            _ => text.clone(),
                        };
                        unfolded.insert(*key, (text, why.clone()));
                        why
                    }
                };
                vec![format!("{key}: {why}")]
            }
        };
        for message in messages {
            if !warned.contains(&message) {
                ctx::warn(&message);
                warned.insert(message);
            }
        }
    }
}

/// Prints the cases in the order of the events that made them the turn.
/// Cases whose events have the same time, or none, are in the order they
/// were opened. With --pickup, each answered case is picked up just before
/// it is printed, so that a wait stopped part-way, or whose stdout has gone,
/// leaves the cases after the one it was on answered for the next wait. A
/// case picked up whose line then fails to print is named on stderr. The
/// messages a listening wait took were picked up before any line, so when
/// one line fails, each of them not printed yet is named too: no later wait
/// takes them again (AG-16). A pickup wait on a forwarded store checks its
/// locks there, `far`, before each pickup, and picks up nothing once they
/// may have been let go.
fn print(
    args: &Args,
    ctx: &mut Ctx,
    since: Option<Time>,
    mut turns: Vec<Turn>,
    far: Option<&FarLocks>,
) -> Result<(), Error> {
    turns.sort_by_key(|turn| turn.at);
    let next_since = turns.iter().filter_map(|turn| turn.at).chain(since).max();
    // Only a whole case names its actors, so a short line needs none. Read
    // before any pickup, so that a store that cannot give them fails wait
    // before it writes. A message wait took is picked up already, so it is
    // printed with its actors named by id alone rather than lost.
    let read = if args.full {
        ctx.actors()
    } else {
        Ok(Actors::default())
    };
    let mut actors = match read {
        Ok(actors) => actors,
        Err(error) if turns.iter().any(|turn| turn.taken) => {
            ctx::warn(format_args!(
                "the actors cannot be read, so they are named by id: {error}"
            ));
            Actors::default()
        }
        Err(error) => return Err(error),
    };
    // The messages wait took whose lines are not printed yet.
    let mut unprinted: Vec<Key> = turns
        .iter()
        .filter(|turn| turn.taken)
        .map(|turn| turn.key)
        .collect();
    let by = args.by.as_deref().unwrap_or_default();
    for mut turn in turns {
        // A pickup wait whose locks on the far side may have been let go
        // picks up nothing more: the case is printed as read, as one whose
        // pickup failed is.
        let picked = args.pickup
            && turn.case.state() == State::Answered
            && match far.map(FarLocks::write).transpose() {
                Err(error) => {
                    ctx::warn(format_args!("{}: not picked up: {error}", turn.key));
                    false
                }
                Ok(writing) => {
                    let picked = pick_up(ctx, by, &mut turn);
                    if picked && let Some(writing) = writing {
                        writing.owe(turn.key);
                    }
                    picked
                }
            };
        // A pickup makes its actor when the table does not have it, such as
        // for a case opened by hand. A read that fails keeps the table read
        // before, so that the case is still printed.
        if args.full
            && picked
            && let Ok(now) = ctx.actors()
        {
            actors = now;
        }
        let printed = match line(&turn, &actors, next_since, args.full) {
            Ok(text) => {
                writeln!(ctx.out, "{text}").map_err(|error| (error.to_string(), Error::from(error)))
            }
            Err(error) => Err((error.to_string(), error)),
        };
        if let Err((why, error)) = printed {
            let lost = picked.then_some(turn.key).into_iter().chain(unprinted);
            for key in lost {
                ctx::warn(format_args!("{key}: picked up, but not printed: {why}"));
            }
            return Err(error);
        }
        unprinted.retain(|key| *key != turn.key);
        // Only once the line is out: a signal while it is printed names
        // the case too, rather than wait on a stdout that may be full.
        if let Some(mut owed) = far.map(FarLocks::owed) {
            owed.retain(|key| *key != turn.key);
        }
    }
    Ok(())
}

/// The line `turn` prints as: its case short, or when `full` as `show
/// --json` gives it, with its actors named from `actors`, then `fresh` and
/// `next_since`.
fn line(
    turn: &Turn,
    actors: &Actors,
    next_since: Option<Time>,
    full: bool,
) -> Result<String, Error> {
    let case = if full {
        Shown::Full(Box::new(CaseJson::new(turn.key, &turn.case, actors)?))
    } else {
        Shown::Short(Box::new(Short::new(turn.key, &turn.case)))
    };
    let line = Line {
        case,
        fresh: turn.fresh,
        next_since,
    };
    // A record stored by hand or by another tool can hold line breaks,
    // which its JSON as stored keeps. Outside a string they are only white
    // space, so they become spaces. NEL and the line and paragraph
    // separators, which some readers also end a line at, can only be in a
    // string, where they are escaped. So each case is one line.
    Ok(serde_json::to_string(&line)?
        .replace(['\n', '\r'], " ")
        .replace('\u{85}', "\\u0085")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

/// Picks up the answered case of `turn`, as the case's worker, at the
/// revision wait read it at, so that wait never picks up an answer other
/// than the one it read (AG-05, AG-12), and says whether it did. A case not
/// picked up, because another event landed first or for any other reason, is
/// left as read, and why goes to stderr, with the state a case that changed
/// is in now.
fn pick_up(ctx: &Ctx, by: &str, turn: &mut Turn) -> bool {
    let mut pickup = Pickup::default();
    by.clone_into(&mut pickup.by);
    let writer = write::agent_of(&turn.case);
    let record = Record::Pickup(pickup);
    let revision = Some(turn.case.revision());
    let picked = match write::append(ctx, turn.key, Author::Agent, &record, &writer, revision) {
        Ok(case) => {
            turn.case = case;
            true
        }
        Err(error) => {
            ctx::warn(format_args!("{}: not picked up: {error}", turn.key));
            false
        }
    };
    thread::sleep(WRITE_PAUSE);
    picked
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use cases_store_sqlite::Db;

    use super::*;

    #[test]
    fn a_new_event_starts_the_quiet_time_again_up_to_the_cap() {
        let linger = Mood::Steady.linger().unwrap();
        let (quiet, cap) = linger;
        let start = Instant::now();
        let mut collecting = Collecting::new(start);
        let one = ((Key::new(1).unwrap(), None), 1);
        let two = ((Key::new(2).unwrap(), None), 1);
        collecting.saw([one], start);
        assert_eq!(collecting.until(linger), start + quiet);

        // An event seen before does not start it again; a new one does.
        let later = start + quiet / 2;
        collecting.saw([one], later);
        assert_eq!(collecting.until(linger), start + quiet);
        collecting.saw([one, two], later);
        assert_eq!(collecting.until(linger), later + quiet);

        // However many come, it prints by the cap.
        let late = start + cap.saturating_sub(Duration::from_secs(1));
        collecting.saw([((Key::new(3).unwrap(), None), 1)], late);
        assert_eq!(collecting.until(linger), start + cap);
    }

    #[test]
    fn a_wait_collecting_stops_at_its_linger_or_its_deadline_whichever_is_first() {
        let start = Instant::now();
        let mut collecting = Collecting::new(start);
        collecting.saw([((Key::new(1).unwrap(), None), 1)], start);
        let end = collecting.until(Mood::Steady.linger().unwrap());
        let (soon, late) = (
            start + Duration::from_secs(1),
            start + Duration::from_secs(3600),
        );
        // Before it collects, only the deadline stops it.
        assert_eq!(until(None, Mood::Steady, None), None);
        assert_eq!(until(Some(late), Mood::Steady, None), Some(late));
        // Once it does, its linger stops it too.
        assert_eq!(until(None, Mood::Steady, Some(&collecting)), Some(end));
        assert_eq!(
            until(Some(late), Mood::Steady, Some(&collecting)),
            Some(end)
        );
        assert_eq!(
            until(Some(soon), Mood::Steady, Some(&collecting)),
            Some(soon)
        );
        // An eager wait never lingers.
        assert_eq!(
            until(Some(late), Mood::Eager, Some(&collecting)),
            Some(late)
        );
    }

    #[test]
    fn eager_never_lingers_and_patient_lingers_longest() {
        assert_eq!(Mood::Eager.linger(), None);
        let (steady, patient) = (Mood::Steady.linger(), Mood::Patient.linger());
        assert!(steady.unwrap().0 < patient.unwrap().0);
        assert!(steady.unwrap().1 < patient.unwrap().1);
        assert_eq!(mood("steady").unwrap(), Mood::Steady);
        assert!(mood("Steady").is_err());
        assert!(mood("sleepy").is_err());
    }

    /// A local store at `path` whose clock reads `clock`, in seconds since
    /// the epoch, so that a test sees which beats wrote.
    fn clocked(path: &std::path::Path, clock: &Arc<AtomicI64>) -> AnyStore {
        let clock = Arc::clone(clock);
        let db = Db::new(path)
            .with_clock(move || Time::from_unix(clock.load(Ordering::SeqCst), 0).unwrap());
        AnyStore::Local(db)
    }

    /// Makes the store `db` by opening a case in it.
    fn make(db: &AnyStore) {
        let open = cases_core::Append {
            author: "agent".into(),
            event: "open".into(),
            data: r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#.into(),
        };
        db.create(&open, &Writer::new(None)).unwrap();
    }

    fn listening<'a>(
        workers: &'a [String],
        labels: &'a [String],
        doing: Option<&'a str>,
    ) -> Listening<'a> {
        Listening {
            id: None,
            workers,
            labels,
            kinds: vec!["message".into()],
            by: &workers[0],
            doing,
            given: HashSet::new(),
            due: None,
        }
    }

    /// The time `secs` seconds after the epoch.
    fn at(secs: i64) -> Time {
        Time::from_unix(secs, 0).unwrap()
    }

    /// (worker, wait id, heard) of each wait recorded in `db`.
    fn heard(db: &AnyStore) -> Vec<(String, String, Time)> {
        let mut waits: Vec<_> = db
            .waits()
            .unwrap()
            .into_iter()
            .map(|wait| (wait.worker, wait.id, wait.heard))
            .collect();
        waits.sort();
        waits
    }

    /// The doing line of `worker` in `db`.
    fn doing_of(db: &AnyStore, worker: &str) -> Option<String> {
        let AnyStore::Local(local) = db else {
            unreachable!()
        };
        let listeners = local.listeners().unwrap();
        let listening = listeners.iter().find(|l| l.worker == worker).unwrap();
        listening.doing.clone()
    }

    /// The first beat records each worker, with its labels, kinds and doing
    /// line, under one wait id. Until the heartbeat is due no beat writes;
    /// once it is, the beat refreshes each record under the same id, and
    /// does not put back a doing line cleared since (ST-16, AG-17).
    #[test]
    fn a_listening_wait_records_its_workers_again_only_once_a_heartbeat_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(100));
        let db = clocked(&dir.path().join("cases.db"), &clock);
        make(&db);
        let workers = ["ada".to_owned(), "bo".to_owned()];
        let labels = ["ops".to_owned()];
        let mut listening = listening(&workers, &labels, Some("reading"));
        let mut warned = HashSet::new();

        listening.beat(&db, &mut warned);
        let id = listening.id.clone().expect("the first beat makes an id");
        let ada = ("ada".to_owned(), id.clone());
        let bo = ("bo".to_owned(), id.clone());
        assert_eq!(
            heard(&db),
            [
                (ada.0.clone(), ada.1.clone(), at(100)),
                (bo.0.clone(), bo.1.clone(), at(100))
            ]
        );
        let waits = db.waits().unwrap();
        assert!(waits.iter().all(|wait| wait.labels == labels));
        assert!(waits.iter().all(|wait| wait.kinds == ["message"]));
        assert_eq!(doing_of(&db, "ada").as_deref(), Some("reading"));
        assert_eq!(doing_of(&db, "bo").as_deref(), Some("reading"));
        let due = listening.due.expect("a beat that wrote sets the next");
        assert!(due > Instant::now() + HEARTBEAT / 2, "{due:?}");
        assert!(warned.is_empty(), "{warned:?}");

        // `cases doing` clears the line; a beat before the heartbeat is due
        // writes nothing.
        let AnyStore::Local(local) = &db else {
            unreachable!()
        };
        local.set_doing("ada", "").unwrap();
        clock.store(105, Ordering::SeqCst);
        listening.beat(&db, &mut warned);
        assert_eq!(listening.due, Some(due));
        assert!(heard(&db).iter().all(|(.., heard)| *heard == at(100)));

        // Once it is due, the beat refreshes both, and the line stays clear.
        listening.due = Some(Instant::now());
        clock.store(110, Ordering::SeqCst);
        listening.beat(&db, &mut warned);
        assert_eq!(heard(&db), [(ada.0, ada.1, at(110)), (bo.0, bo.1, at(110))]);
        assert_eq!(listening.id.as_deref(), Some(id.as_str()));
        assert_eq!(doing_of(&db, "ada"), None);
        assert!(listening.due.unwrap() > Instant::now() + HEARTBEAT / 2);
        assert!(warned.is_empty(), "{warned:?}");
    }

    /// A beat on a store not there yet makes nothing and warns of nothing,
    /// and the next beat tries again: once the store is there, it records
    /// the wait under the id the first beat made (ST-08).
    #[test]
    fn a_wait_on_a_store_not_there_yet_records_itself_once_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        let clock = Arc::new(AtomicI64::new(100));
        let db = clocked(&path, &clock);
        let workers = ["ada".to_owned()];
        let mut listening = listening(&workers, &[], Some("reading"));
        let mut warned = HashSet::new();

        listening.beat(&db, &mut warned);
        assert!(!path.exists());
        assert!(warned.is_empty(), "{warned:?}");
        assert_eq!(listening.due, None);
        let id = listening
            .id
            .clone()
            .expect("the id is made before the write");

        make(&db);
        listening.beat(&db, &mut warned);
        assert_eq!(heard(&db), [("ada".to_owned(), id, at(100))]);
        assert_eq!(doing_of(&db, "ada").as_deref(), Some("reading"));
        assert!(listening.due.is_some());
        assert!(warned.is_empty(), "{warned:?}");
    }

    /// A write that fails for the store, not for one worker, such as a file
    /// that is not a store, ends the beat at the first worker: one write and
    /// one warning, however many workers the wait has, rather than one each.
    /// The beat tries them all again a [`RETRY`] later.
    #[test]
    fn a_store_that_fails_is_written_once_a_beat_whatever_the_workers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases.db");
        std::fs::write(&path, "not a database, but a file of text").unwrap();
        let clock = Arc::new(AtomicI64::new(100));
        let db = clocked(&path, &clock);
        let workers = ["ada".to_owned(), "bo".to_owned(), "cy".to_owned()];
        let mut listening = listening(&workers, &[], Some("reading"));
        let mut warned = HashSet::new();

        listening.beat(&db, &mut warned);
        assert_eq!(warned.len(), 1, "{warned:?}");
        let warning = warned.iter().next().unwrap().clone();
        assert!(
            warning.starts_with("ada is not recorded as listening: "),
            "{warning}"
        );
        let due = listening.due.expect("a beat that failed sets the next");
        assert!(due > Instant::now() + RETRY / 2, "{due:?}");

        listening.due = Some(Instant::now());
        listening.beat(&db, &mut warned);
        assert_eq!(warned, HashSet::from([warning]));
    }

    /// A worker whose write fails is warned about once, however many beats
    /// meet it, and the workers after it are still recorded. The beat that
    /// met it tries again a [`RETRY`] later, not at the next read; a doing
    /// line that went on a worker's record, and was cleared since, is not
    /// put back by it (AG-17). A blank worker stands in for a failing
    /// write: `--pickup` refuses one, and the store refuses to record it.
    #[test]
    fn a_worker_whose_write_fails_is_warned_about_once_and_the_rest_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(100));
        let db = clocked(&dir.path().join("cases.db"), &clock);
        make(&db);
        let workers = ["ada".to_owned(), " ".to_owned(), "bo".to_owned()];
        let mut listening = listening(&workers, &[], Some("reading"));
        let mut warned = HashSet::new();

        listening.beat(&db, &mut warned);
        assert_eq!(warned.len(), 1, "{warned:?}");
        let warning = warned.iter().next().unwrap().clone();
        assert!(
            warning.starts_with("  is not recorded as listening: "),
            "{warning}"
        );
        let id = listening.id.clone().unwrap();
        let recorded = |secs| {
            [
                ("ada".to_owned(), id.clone(), at(secs)),
                ("bo".to_owned(), id.clone(), at(secs)),
            ]
        };
        assert_eq!(heard(&db), recorded(100));
        assert_eq!(doing_of(&db, "ada").as_deref(), Some("reading"));
        assert_eq!(doing_of(&db, "bo").as_deref(), Some("reading"));
        let due = listening.due.expect("a beat that failed sets the next");
        assert!(due > Instant::now() + RETRY / 2, "{due:?}");

        // Until the retry is due, no beat writes.
        clock.store(105, Ordering::SeqCst);
        listening.beat(&db, &mut warned);
        assert_eq!(listening.due, Some(due));
        assert_eq!(heard(&db), recorded(100));

        // Once it is, the beat writes each worker again, warns no more, and
        // leaves the lines `cases doing` cleared as they are.
        let AnyStore::Local(local) = &db else {
            unreachable!()
        };
        local.set_doing("ada", "").unwrap();
        local.set_doing("bo", "").unwrap();
        listening.due = Some(Instant::now());
        clock.store(110, Ordering::SeqCst);
        listening.beat(&db, &mut warned);
        assert_eq!(warned, HashSet::from([warning]));
        assert_eq!(heard(&db), recorded(110));
        assert_eq!(doing_of(&db, "ada"), None);
        assert_eq!(doing_of(&db, "bo"), None);
        assert!(listening.due.unwrap() > Instant::now() + RETRY / 2);
    }
}
