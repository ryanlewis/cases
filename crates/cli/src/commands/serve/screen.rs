//! The status screen serve shows on a terminal (SV-02): where the inbox is,
//! the store's cases by state and urgency, what happened since serve
//! started, and the last lines of the log, redrawn in place.

use std::borrow::Borrow;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::sync::{Arc, Weak};
use std::time::Duration;

use cases_core::{Author, Case, EventType, State, Time, Urgency};
use cases_store_sqlite::Key;

use super::watch::Snapshot;
use crate::output;

/// What the screen tells the human about pointing an agent at the store.
const AGENT_HINT: [&str; 2] = [
    "Agents: install the bundled skill with `cases skill install claude` (or codex, pi)",
    "        and they can raise cases with `cases open` and act on answers with `cases wait`.",
];

/// The store's cases, counted for the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Open cases by urgency: blocking, today, whenever.
    pub(crate) open: [usize; 3],
    pub(crate) parked: usize,
    /// Messages the human sent that no agent has picked up yet.
    pub(crate) sent: usize,
    pub(crate) answered: usize,
    pub(crate) picked_up: usize,
    pub(crate) closed: usize,
    /// The cases closed since serve started.
    pub(crate) closed_since: usize,
    /// The human's answers, parks and resumes since serve started, from the
    /// inbox or the CLI.
    pub(crate) answers: usize,
    pub(crate) parks: usize,
    pub(crate) resumes: usize,
    /// The time of the latest event in the store.
    pub(crate) last_event: Option<Time>,
}

impl Stats {
    /// Counts `cases`, with `since` as when serve started.
    pub(crate) fn count<C: Borrow<Case>>(cases: &[(Key, C)], since: Time) -> Stats {
        let mut stats = Stats::default();
        for (_, case) in cases {
            let case = case.borrow();
            match case.state() {
                State::Open => {
                    let at = match case.urgency() {
                        Urgency::Blocking => 0,
                        Urgency::Today => 1,
                        _ => 2,
                    };
                    stats.open[at] += 1;
                }
                State::Parked => stats.parked += 1,
                State::Sent => stats.sent += 1,
                State::Answered => stats.answered += 1,
                State::PickedUp => stats.picked_up += 1,
                State::Closed => stats.closed += 1,
                _ => {}
            }
            stats.last_event = stats.last_event.max(case.updated_at());
            for event in case.events() {
                if event.at().is_none_or(|at| at < since) {
                    continue;
                }
                match (event.author, event.event()) {
                    (_, EventType::Close) => stats.closed_since += 1,
                    (Author::Human, EventType::Answer) => stats.answers += 1,
                    (Author::Human, EventType::Park) => stats.parks += 1,
                    (Author::Human, EventType::Resume) => stats.resumes += 1,
                    _ => {}
                }
            }
        }
        stats
    }
}

/// The counts of the store's latest snapshot, since serve started. A
/// snapshot never changes, and the watch gives the same one until the store
/// does, so each is counted once, not on every frame that shows it.
#[derive(Debug)]
pub(crate) struct Counts {
    since: Time,
    /// The snapshot last counted, and its counts. It is held weakly: its
    /// cases go once the watch lets go of it, but its address is never given
    /// to a new one while this holds it.
    last: Option<(Weak<Snapshot>, Stats)>,
    /// How many snapshots were counted.
    #[cfg(test)]
    counted: usize,
}

impl Counts {
    /// Counts with `since` as when serve started.
    pub(crate) fn new(since: Time) -> Counts {
        Counts {
            since,
            last: None,
            #[cfg(test)]
            counted: 0,
        }
    }

    /// The counts of `snapshot`.
    pub(crate) fn of(&mut self, snapshot: &Arc<Snapshot>) -> Stats {
        if let Some((last, stats)) = &self.last
            && std::ptr::eq(last.as_ptr(), Arc::as_ptr(snapshot))
        {
            return stats.clone();
        }
        let stats = Stats::count(&snapshot.cases, self.since);
        #[cfg(test)]
        {
            self.counted += 1;
        }
        self.last = Some((Arc::downgrade(snapshot), stats.clone()));
        stats
    }
}

/// Everything one frame of the screen shows.
#[derive(Debug)]
pub(crate) struct View<'a> {
    pub(crate) store: &'a str,
    pub(crate) url: &'a str,
    /// The cases, or why the store could not be read.
    pub(crate) stats: Result<Stats, String>,
    pub(crate) uptime: Duration,
    pub(crate) requests: u64,
    /// Notifications queued for the browser since serve started.
    pub(crate) notified: u64,
    pub(crate) log: Vec<String>,
    /// Whether `q` is read from the keyboard.
    pub(crate) keys: bool,
    pub(crate) color: bool,
    pub(crate) now: Time,
}

/// Lays out one frame, a line to an entry.
pub(crate) fn render(view: &View<'_>) -> Vec<String> {
    let style = |code: &str, text: &str| {
        if view.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let (dim, bold, red, cyan, green) = ("2", "1", "1;31", "36", "32");
    let label = |text: &str| style(dim, &format!("{text:<12}"));
    let dot = style(dim, " · ");

    let mut lines = vec![
        style(bold, "cases serve") + &style(dim, &format!("  ·  up {}", uptime(view.uptime))),
        String::new(),
        label("inbox") + &style(cyan, view.url),
        label("store") + view.store,
        String::new(),
    ];
    let stats = match &view.stats {
        Ok(stats) => stats,
        Err(error) => {
            lines.push(style(red, &format!("cannot read the store: {error}")));
            lines.push(String::new());
            &Stats::default()
        }
    };
    let [blocking, today, whenever] = stats.open;
    let open = blocking + today + whenever;
    let mut blocking = format!("{blocking} blocking");
    if stats.open[0] > 0 {
        blocking = style(red, &blocking);
    }
    let mut open_count = format!("{open:3}");
    if open > 0 {
        open_count = style(bold, &open_count);
    }
    lines.extend([
        format!(
            "{}{open_count}   {blocking}{dot}{today} today{dot}{whenever} whenever",
            label("open")
        ),
        format!("{}{:3}", label("parked"), stats.parked),
        format!(
            "{}{:3}   {} sent{dot}{} answered{dot}{} picked up",
            label("with agent"),
            stats.sent + stats.answered + stats.picked_up,
            stats.sent,
            stats.answered,
            stats.picked_up
        ),
        format!(
            "{}{:3}   {} since start",
            label("closed"),
            stats.closed,
            stats.closed_since
        ),
        String::new(),
        format!(
            "{}{} · {} · {} · {} · {}",
            label("since start"),
            output::plural(view.requests, "request"),
            output::plural(stats.answers as u64, "answer"),
            output::plural(stats.parks as u64, "park"),
            output::plural(stats.resumes as u64, "resume"),
            output::plural(view.notified, "notification"),
        ),
        format!("{}{}", label("last event"), ago(stats.last_event, view.now)),
        String::new(),
        style(dim, "log"),
    ]);
    if view.log.is_empty() {
        lines.push(style(dim, "  nothing yet"));
    }
    lines.extend(view.log.iter().map(|line| format!("  {line}")));
    lines.push(String::new());
    lines.extend(AGENT_HINT.map(str::to_owned));
    lines.push(String::new());
    let quit = style(green, "Ctrl-C") + " quit";
    lines.push(if view.keys {
        style(green, "q") + " quit   " + &quit
    } else {
        quit
    });
    lines
}

/// How long serve has been up: minutes, then hours and minutes, then days
/// and hours.
fn uptime(up: Duration) -> String {
    let minutes = up.as_secs() / 60;
    match minutes {
        0..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {:02}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes / 60 % 24),
    }
}

/// How long ago `at` was, for people.
fn ago(at: Option<Time>, now: Time) -> String {
    let Some(at) = at else {
        return "none".into();
    };
    match output::age(Some(at), now).as_str() {
        "now" => "just now".into(),
        age => format!("{age} ago"),
    }
}

/// A terminal the screen is drawn on in place: each frame moves back up
/// over the last one and draws over it.
#[derive(Debug, Default)]
pub(crate) struct Screen {
    /// The lines on the terminal from the last frame.
    drawn: usize,
    /// The last frame, so that one the same is not drawn again.
    frame: String,
}

impl Screen {
    /// The escape codes that hide the cursor and turn off line wrapping,
    /// so a long line is clipped and the count of lines to move up stays
    /// true.
    pub(crate) const START: &str = "\x1b[?25l\x1b[?7l";
    /// The codes that put them back.
    pub(crate) const END: &str = "\x1b[?7h\x1b[?25h";

    /// Draws `lines` over the last frame, unless they are the same. `rows`
    /// is the terminal's height: a frame taller scrolls, and moving back up
    /// by its count of lines would then stack copies, so it is clipped to
    /// leave a row for the cursor.
    pub(crate) fn draw(
        &mut self,
        out: &mut impl Write,
        mut lines: Vec<String>,
        rows: Option<usize>,
    ) -> io::Result<()> {
        if let Some(rows) = rows.filter(|rows| *rows > 1) {
            lines.truncate(rows - 1);
        }
        let frame = lines.join("\n");
        if frame == self.frame {
            return Ok(());
        }
        let mut text = String::new();
        if self.drawn > 0 {
            let _ = write!(text, "\x1b[{}A", self.drawn);
        }
        for line in &lines {
            let _ = write!(text, "\r\x1b[2K{line}\n");
        }
        text.push_str("\x1b[J");
        out.write_all(text.as_bytes())?;
        out.flush()?;
        self.drawn = lines.len();
        self.frame = frame;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Store};
    use cases_store_sqlite::{Db, Writer};

    use super::*;

    fn at(text: &str) -> Time {
        text.parse().unwrap()
    }

    fn view<'a>(stats: Result<Stats, String>, log: Vec<String>, keys: bool) -> View<'a> {
        View {
            store: "/data/cases.db",
            url: "http://127.0.0.1:7337/",
            stats,
            uptime: Duration::from_mins(6),
            requests: 1,
            notified: 1,
            log,
            keys,
            color: false,
            now: at("2026-09-16T12:10:00Z"),
        }
    }

    /// Writes `data` as the event `event` by `author` at `time`, to the case
    /// `key`, or opens a case when `key` is `None`.
    fn write(db: &Db, key: Option<Key>, author: &str, event: &str, data: &str) -> Key {
        let append = Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        };
        match key {
            Some(key) => {
                db.append(&key, &append, &Writer::new(None), None).unwrap();
                key
            }
            None => db.create(&append, &Writer::new(None)).unwrap().0,
        }
    }

    #[test]
    fn the_screen_counts_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let open = |kind: &str, urgency: &str, extra: &str| {
            write(
                &db,
                None,
                "agent",
                "open",
                &format!(
                    r#"{{"kind":"{kind}","urgency":"{urgency}","title":"T","opened_at":"2026-09-16T11:00:00Z"{extra}}}"#
                ),
            )
        };
        open("fyi", "blocking", "");
        open("fyi", "blocking", "");
        open("fyi", "today", "");
        let parked = open("stuck", "whenever", "");
        write(
            &db,
            Some(parked),
            "human",
            "park",
            r#"{"parked_at":"2026-09-16T12:01:00Z"}"#,
        );
        let answered = open("decision", "today", r#","options":["a","b"]"#);
        write(
            &db,
            Some(answered),
            "human",
            "answer",
            r#"{"choice":1,"answered_at":"2026-09-16T11:30:00Z"}"#,
        );
        let closed = open("fyi", "whenever", "");
        write(
            &db,
            Some(closed),
            "human",
            "answer",
            r#"{"ack":true,"answered_at":"2026-09-16T12:01:00Z"}"#,
        );
        write(
            &db,
            Some(closed),
            "agent",
            "pickup",
            r#"{"picked_up_at":"2026-09-16T12:01:00Z"}"#,
        );
        write(
            &db,
            Some(closed),
            "agent",
            "close",
            r#"{"outcome":"done","closed_at":"2026-09-16T12:02:00Z"}"#,
        );
        // A message the human sent waits on an agent, not on the human.
        write(
            &db,
            None,
            "human",
            "open",
            r#"{"kind":"message","urgency":"today","title":"T","worker":"bun-pins","opened_at":"2026-09-16T11:00:00Z"}"#,
        );

        let cases: Vec<(Key, Case)> = db
            .poller()
            .poll()
            .unwrap()
            .iter()
            .map(|(key, case)| (*key, case.clone().unwrap()))
            .collect();
        let stats = Stats::count(&cases, at("2026-09-16T12:00:00Z"));
        assert_eq!(
            stats,
            Stats {
                open: [2, 1, 0],
                parked: 1,
                sent: 1,
                answered: 1,
                picked_up: 0,
                closed: 1,
                closed_since: 1,
                answers: 1,
                parks: 1,
                resumes: 0,
                last_event: Some(at("2026-09-16T12:02:00Z")),
            }
        );

        let frame = render(&view(Ok(stats), vec!["GET / 200 1ms".into()], true)).join("\n");
        for want in [
            "cases serve  ·  up 6m",
            "inbox       http://127.0.0.1:7337/",
            "store       /data/cases.db",
            "open          3   2 blocking · 1 today · 0 whenever",
            "parked        1",
            "with agent    2   1 sent · 1 answered · 0 picked up",
            "closed        1   1 since start",
            "since start 1 request · 1 answer · 1 park · 0 resumes · 1 notification",
            "last event  8m ago",
            "  GET / 200 1ms",
            AGENT_HINT[0],
            "q quit   Ctrl-C quit",
        ] {
            assert!(frame.contains(want), "screen lacks {want:?}:\n{frame}");
        }
        assert!(!frame.contains("\x1b["), "colour without color:\n{frame}");
    }

    /// The cases of `db` as the poller folds them.
    fn cases(db: &Db) -> Vec<(Key, Case)> {
        db.poller()
            .poll()
            .unwrap()
            .iter()
            .map(|(key, case)| (*key, case.clone().unwrap()))
            .collect()
    }

    /// What the first test does not count: an open case of every urgency,
    /// one picked up, and one withdrawn, which no count takes; a resume made
    /// as serve started, which counts; and events no count takes: an
    /// agent's pickup and withdraw, and an answer and a close before serve
    /// started.
    #[test]
    fn the_screen_counts_every_state_and_the_human_s_events_since_start() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        let open = |kind: &str, urgency: &str| {
            let options = if kind == "decision" {
                r#","options":["a","b"]"#
            } else {
                ""
            };
            write(
                &db,
                None,
                "agent",
                "open",
                &format!(
                    r#"{{"kind":"{kind}","urgency":"{urgency}","title":"T","opened_at":"2026-09-16T11:00:00Z"{options}}}"#
                ),
            )
        };
        open("question", "blocking");
        open("question", "today");
        let resumed = open("stuck", "whenever");
        write(
            &db,
            Some(resumed),
            "human",
            "park",
            r#"{"parked_at":"2026-09-16T11:30:00Z"}"#,
        );
        write(
            &db,
            Some(resumed),
            "human",
            "resume",
            r#"{"resumed_at":"2026-09-16T12:00:00Z"}"#,
        );
        let picked = open("decision", "today");
        write(
            &db,
            Some(picked),
            "human",
            "answer",
            r#"{"choice":1,"answered_at":"2026-09-16T12:01:00Z"}"#,
        );
        write(
            &db,
            Some(picked),
            "agent",
            "pickup",
            r#"{"picked_up_at":"2026-09-16T12:04:00Z"}"#,
        );
        let withdrawn = open("decision", "blocking");
        write(
            &db,
            Some(withdrawn),
            "agent",
            "withdraw",
            r#"{"withdrawn_at":"2026-09-16T12:05:00Z"}"#,
        );
        let early = open("fyi", "whenever");
        write(
            &db,
            Some(early),
            "human",
            "answer",
            r#"{"ack":true,"answered_at":"2026-09-16T11:58:00Z"}"#,
        );
        write(
            &db,
            Some(early),
            "agent",
            "pickup",
            r#"{"picked_up_at":"2026-09-16T11:58:30Z"}"#,
        );
        write(
            &db,
            Some(early),
            "agent",
            "close",
            r#"{"outcome":"done","closed_at":"2026-09-16T11:59:00Z"}"#,
        );

        let stats = Stats::count(&cases(&db), at("2026-09-16T12:00:00Z"));
        assert_eq!(
            stats,
            Stats {
                open: [1, 1, 1],
                parked: 0,
                sent: 0,
                answered: 0,
                picked_up: 1,
                closed: 1,
                closed_since: 0,
                answers: 1,
                parks: 0,
                resumes: 1,
                last_event: Some(at("2026-09-16T12:05:00Z")),
            }
        );
        let frame = render(&view(Ok(stats), Vec::new(), false));
        assert_eq!(
            frame,
            [
                "cases serve  ·  up 6m",
                "",
                "inbox       http://127.0.0.1:7337/",
                "store       /data/cases.db",
                "",
                "open          3   1 blocking · 1 today · 1 whenever",
                "parked        0",
                "with agent    1   0 sent · 0 answered · 1 picked up",
                "closed        1   0 since start",
                "",
                "since start 1 request · 1 answer · 0 parks · 1 resume · 1 notification",
                "last event  5m ago",
                "",
                "log",
                "  nothing yet",
                "",
                AGENT_HINT[0],
                AGENT_HINT[1],
                "",
                "Ctrl-C quit",
            ]
        );
    }

    /// A snapshot is counted once, however many frames read it: again
    /// when the store changes, and for a new snapshot that holds the same.
    #[test]
    fn each_snapshot_is_counted_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::new(dir.path().join("cases.db")));
        let fyi =
            r#"{"kind":"fyi","urgency":"today","title":"T","opened_at":"2026-09-16T12:01:00Z"}"#;
        write(&db, None, "agent", "open", fyi);
        let log = Arc::new(super::super::log::Log::new(false, 0));
        let watch = super::super::watch::Watch::new(
            Arc::clone(&db),
            "boot".into(),
            "http://127.0.0.1:7337/".into(),
            log,
        )
        .unwrap();
        let mut counts = Counts::new(at("2026-09-16T12:00:00Z"));
        let one = Stats {
            open: [0, 1, 0],
            last_event: Some(at("2026-09-16T12:01:00Z")),
            ..Stats::default()
        };
        for _ in 0..3 {
            assert_eq!(counts.of(&watch.read().unwrap()), one);
        }
        assert_eq!(counts.counted, 1);
        write(&db, None, "agent", "open", fyi);
        let two = Stats {
            open: [0, 2, 0],
            ..one.clone()
        };
        for _ in 0..3 {
            assert_eq!(counts.of(&watch.read().unwrap()), two);
        }
        assert_eq!(counts.counted, 2);
        let read = watch.read().unwrap();
        let same = Arc::new(Snapshot {
            cases: read.cases.clone(),
            version: read.version.clone(),
        });
        assert_eq!(counts.of(&same), two);
        assert_eq!(counts.counted, 3);
    }

    #[test]
    fn an_empty_store_and_a_failed_read() {
        let empty = render(&view(Ok(Stats::default()), Vec::new(), false)).join("\n");
        for want in [
            "open          0   0 blocking · 0 today · 0 whenever",
            "last event  none",
            "  nothing yet",
        ] {
            assert!(empty.contains(want), "screen lacks {want:?}:\n{empty}");
        }
        assert!(
            !empty.contains("q quit") && empty.ends_with("Ctrl-C quit"),
            "{empty}"
        );
        let mut failed = view(Err("store is locked".into()), Vec::new(), false);
        failed.color = true;
        let failed = render(&failed).join("\n");
        assert!(
            failed.contains("\x1b[1;31mcannot read the store: store is locked\x1b[0m"),
            "{failed:?}"
        );
        let mut blocking = view(
            Ok(Stats {
                open: [2, 0, 0],
                ..Stats::default()
            }),
            Vec::new(),
            false,
        );
        blocking.color = true;
        assert!(
            render(&blocking)
                .join("\n")
                .contains("\x1b[1;31m2 blocking\x1b[0m")
        );
    }

    #[test]
    fn a_frame_draws_over_the_last_and_only_when_it_changed() {
        let mut screen = Screen::default();
        let mut out = Vec::new();
        screen
            .draw(&mut out, vec!["one".into(), "two".into()], None)
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\r\x1b[2Kone\n\r\x1b[2Ktwo\n\x1b[J"
        );
        let mut out = Vec::new();
        screen
            .draw(&mut out, vec!["one".into(), "two".into()], None)
            .unwrap();
        assert!(out.is_empty(), "the same frame was drawn again");
        screen
            .draw(
                &mut out,
                vec!["three".into(), "four".into(), "five".into()],
                Some(3),
            )
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[2A\r\x1b[2Kthree\n\r\x1b[2Kfour\n\x1b[J"
        );
    }

    #[test]
    fn uptime_and_ages_read_at_a_glance() {
        for (secs, want) in [
            (0, "0m"),
            (59 * 60, "59m"),
            (65 * 60, "1h 05m"),
            (25 * 3600, "1d 1h"),
        ] {
            assert_eq!(uptime(Duration::from_secs(secs)), want);
        }
        let now = at("2026-09-16T12:00:00Z");
        assert_eq!(ago(None, now), "none");
        assert_eq!(ago(Some(at("2026-09-16T11:59:30Z")), now), "just now");
        assert_eq!(ago(Some(at("2026-09-16T11:55:00Z")), now), "5m ago");
        assert_eq!(ago(Some(at("2026-09-13T12:00:00Z")), now), "3d ago");
    }
}
