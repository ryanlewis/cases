//! `cases wait` (AG-03, AG-04, AG-05, AG-06, AG-18, CL-01).
//!
//! Most tests write an event while a wait runs. Only an event written after
//! the wait's first read of the store is new to it, so those tests start the
//! wait with [`Waiting::after_first_read`], which returns once that read is
//! done.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cases_core::Event;
use cases_store_sqlite::Key;
use common::{Env, Output, from_now, insert_row, json, key};
use rusqlite::Connection;
use serde_json::Value;

/// How long a test gives a wait to do what the test waits for, and the
/// `--timeout` of every wait here that does not give its own.
const PATIENCE: Duration = Duration::from_secs(10);

/// A `cases wait` running in the background.
struct Waiting {
    child: Child,
    stdout: JoinHandle<String>,
    stderr: Receiver<String>,
    /// The lines of stderr read so far.
    read: Vec<String>,
}

impl Waiting {
    /// Starts `cases --store <store> wait` with `args`.
    fn start(env: &Env, args: &[&str]) -> Waiting {
        let mut child = common::spawn(
            wait_command(env, args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let mut stdout = child.stdout.take().expect("a stdout");
        let stdout = thread::spawn(move || {
            let mut text = String::new();
            stdout.read_to_string(&mut text).expect("UTF-8 on stdout");
            text
        });
        let stderr = BufReader::new(child.stderr.take().expect("a stderr"));
        let (send, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in stderr.lines() {
                if send.send(line.expect("UTF-8 on stderr")).is_err() {
                    break;
                }
            }
        });
        Waiting {
            child,
            stdout,
            stderr: lines,
            read: Vec::new(),
        }
    }

    /// Starts a wait, and returns once it has read the store for the first
    /// time. The store must hold `marker`, a case that does not fold, from
    /// [`marker`]: every wait warns about it once, on its first read, so the
    /// warning says that read is done. An event written after it is new to
    /// the wait.
    fn after_first_read(env: &Env, marker: &str, args: &[&str]) -> Waiting {
        let mut waiting = Waiting::start(env, args);
        waiting.until(&format!("warning: {marker}: "));
        waiting
    }

    /// Reads stderr until a line starts with `prefix`.
    fn until(&mut self, prefix: &str) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.stderr.recv_timeout(left) {
                Ok(line) => {
                    let found = line.starts_with(prefix);
                    self.read.push(line);
                    if found {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    panic!("no line starting {prefix:?} on stderr: {:?}", self.read)
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!(
                        "wait ended with no line starting {prefix:?}: {:?}",
                        self.read
                    )
                }
            }
        }
    }

    /// Returns once it has read the store again after what the test wrote:
    /// a new marker, stored now, is warned about on that read. A test that
    /// checks a wait does not wake on its writes calls this first, so that
    /// a wait that ends before it reads them fails the test.
    fn reads_again(&mut self, env: &Env) {
        // Its one row is at seq 2, so that its warning is not the one about a
        // marker from the first read, which a store made again since can give
        // the same key: wait warns about each warning's text once.
        let marker = env.insert(None, 2, "agent", "open", "{}");
        self.until(&format!("warning: {marker}: "));
    }

    /// Checks that it has not woken on what the test wrote, and ends it. It
    /// must read the store after the writes, as [`Waiting::reads_again`]
    /// shows, and then once more: a wait that wakes prints its cases and
    /// ends on the read that woke it, so it gives no warning on the next.
    /// The wait needs no short `--timeout` of its own, only the default of
    /// [`PATIENCE`], which covers the test's writes and both reads with room
    /// to spare on a slow machine. Gives its stderr.
    fn stays_asleep(mut self, env: &Env) -> String {
        self.reads_again(env);
        self.reads_again(env);
        assert!(self.running(), "the wait ended: {:?}", self.read);
        self.child.kill().expect("the wait is stopped");
        self.child.wait().expect("the wait ends");
        let stdout = self.stdout.join().expect("stdout is read");
        assert_eq!(stdout, "", "the wait woke: {:?}", self.read);
        self.read.extend(self.stderr.iter());
        self.read.iter().flat_map(|line| [line, "\n"]).collect()
    }

    /// Whether it is still waiting.
    fn running(&mut self) -> bool {
        self.child.try_wait().expect("the wait's status").is_none()
    }

    /// Waits for it to end, and gives what it did.
    fn finish(mut self) -> Output {
        let status = self.child.wait().expect("the wait ends");
        let stdout = self.stdout.join().expect("stdout is read");
        self.read.extend(self.stderr.iter());
        Output {
            code: status.code().expect("an exit code"),
            stdout,
            stderr: self.read.iter().flat_map(|line| [line, "\n"]).collect(),
        }
    }
}

/// Stores a case that does not fold, and gives its id. Each wait warns about
/// it once, on its first read of the store.
fn marker(env: &Env) -> String {
    env.insert(None, 1, "agent", "open", "{}")
}

/// `stderr` without the warning about `marker`.
fn without(stderr: &str, marker: &str) -> String {
    let warning = format!("warning: {marker}: ");
    stderr
        .lines()
        .filter(|line| !line.starts_with(&warning))
        .flat_map(|line| [line, "\n"])
        .collect()
}

/// Runs `cases <command> <id>` with `args`, which must succeed.
fn run(env: &Env, command: &str, id: &str, args: &[&str]) {
    let mut all = vec![command, id];
    all.extend(args);
    env.ok(&all);
}

/// `cases --store <store> wait` with `args`, and a `--timeout` of
/// [`PATIENCE`] before them: a `--timeout` in `args` comes later, and wins.
fn wait_command(env: &Env, args: &[&str]) -> Command {
    let timeout = format!("{}s", PATIENCE.as_secs());
    let mut all = vec!["wait", "--timeout", &timeout];
    all.extend(args);
    env.command(env.store_args(&all))
}

/// Runs `cases wait` with `args` to its end.
fn wait(env: &Env, args: &[&str]) -> Output {
    common::output(
        wait_command(env, args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .into()
}

/// The cases a wait that woke printed, one JSON object to a line.
fn woke(out: &Output) -> Vec<Value> {
    assert_eq!(out.code, 0, "{out:?}");
    out.stdout.lines().map(json).collect()
}

/// Checks that a wait gave up: exit 2, and nothing on stdout.
fn timed_out(out: &Output) {
    assert_eq!(out.code, 2, "{out:?}");
    assert_eq!(out.stdout, "", "{out:?}");
}

/// The ids of `lines`, in order.
fn ids(lines: &[Value]) -> Vec<&str> {
    lines
        .iter()
        .map(|line| line["id"].as_str().unwrap())
        .collect()
}

/// The file names of the case `id`'s events.
fn files(env: &Env, id: &str) -> Vec<String> {
    env.case(id).events().iter().map(Event::file).collect()
}

/// The time of the case `id`'s last event.
fn last_event_at(env: &Env, id: &str) -> String {
    env.case(id).updated_at().unwrap().to_string()
}

/// Takes the store's write lock, as a write does, and holds it until the
/// connection commits or rolls back: other writers wait for it, and readers
/// do not.
fn lock(env: &Env) -> Connection {
    let conn = Connection::open(&env.store).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn
}

/// Throws the store away, with the files SQLite keeps beside it, as a person
/// who starts again does.
fn remove_store(env: &Env) {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = env.store.clone().into_os_string();
        file.push(suffix);
        let file = PathBuf::from(file);
        if let Err(error) = fs::remove_file(&file)
            && error.kind() != ErrorKind::NotFound
        {
            panic!("{}: {error}", file.display());
        }
    }
}

/// Puts a file that is not a store at `env`'s store path, so that the next
/// read fails, and not in a way a wait rides out. The file is renamed over
/// the store, so no read finds the path empty, which is a store not there
/// yet.
fn spoil_store(env: &Env) {
    let spoilt = env.store.with_extension("spoilt");
    fs::write(&spoilt, "not a database, but a file of text").unwrap();
    fs::rename(&spoilt, &env.store).unwrap();
    for suffix in ["-wal", "-shm"] {
        let mut file = env.store.clone().into_os_string();
        file.push(suffix);
        let file = PathBuf::from(file);
        if let Err(error) = fs::remove_file(&file)
            && error.kind() != ErrorKind::NotFound
        {
            panic!("{}: {error}", file.display());
        }
    }
}

#[test]
fn wait_returns_an_answer_that_lands() {
    let env = Env::new();
    let id = env.open_decision();
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &["--full"]);
    run(&env, "answer", &id, &["--option", "2"]);

    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(line["kind"], "decision");
    assert_eq!(line["state"], "answered");
    assert_eq!(line["answer"]["choice"], 2);
    assert_eq!(line["revision"], 2);
    assert_eq!(line["fresh"], true);
    assert_eq!(line["next_since"], line["updated_at"]);
}

#[test]
fn wait_times_out_with_exit_2_and_nothing_on_stdout() {
    let env = Env::new();
    let id = env.open_decision();
    let started = Instant::now();
    let out = wait(&env, &["--id", &id, "--timeout", "100ms"]);
    timed_out(&out);
    // One line, and not an error's: a timeout is not a fault (CL-01).
    assert_eq!(out.stderr, "no case needed the agent within 100ms\n");
    // It gives up at the timeout, not at the next read of the store, a
    // second after the last.
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(900),
        "the timeout took {took:?}"
    );
}

#[test]
fn wait_ignores_events_from_before_it_started() {
    let env = Env::new();
    let old = env.open_decision();
    run(&env, "answer", &old, &["--option", "1"]);
    timed_out(&wait(&env, &["--timeout", "100ms"]));

    // Once something new lands, every case that is the agent's turn is
    // printed, the old answer's too.
    let fresh = env.open_decision();
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "answer", &fresh, &["--option", "2"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [old.as_str(), fresh.as_str()]);
    assert_eq!(lines[0]["fresh"], false);
    assert_eq!(lines[1]["fresh"], true);
}

#[test]
fn wait_since_counts_events_already_in_the_store() {
    let env = Env::new();
    let id = env.open_decision();
    let before = from_now(-60);
    run(&env, "answer", &id, &["--option", "1"]);

    let lines = woke(&wait(&env, &["--since", &before]));
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["fresh"], true);
    timed_out(&wait(
        &env,
        &["--since", &from_now(60), "--timeout", "100ms"],
    ));
    // An empty --since, as `"$NEXT"` gives when the variable is not set, is
    // refused too, not taken as no --since.
    for bad in ["yesterday", "2026-09-25 12:00:00Z", ""] {
        let out = wait(&env, &["--since", bad]);
        assert_eq!(out.code, 1, "{bad}: {out:?}");
        assert_eq!(out.stdout, "", "{bad}");
    }
}

#[test]
fn wait_wakes_on_a_park_and_a_resume_and_not_on_the_agent_s_own_events() {
    let env = Env::new();
    let id = env.open_stuck();
    let marker = marker(&env);

    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "park", &id, &["--note", "Back on Monday."]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["state"], "parked");

    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "resume", &id, &[]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["state"], "open");

    // The agent's own events never wake it. The agent does not resume a
    // case (HU-05), so its note and withdraw stand for its events here.
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "note", &id, &["--body", "Trying the mirror."]);
    run(&env, "withdraw", &id, &["--reason", "The mirror works."]);
    waiting.stays_asleep(&env);
}

#[test]
fn wait_on_cases_by_their_whole_id() {
    let env = Env::new();
    let a = env.open_decision();
    let b = env.open_decision();
    let marker = marker(&env);

    // A title slug after the id is ignored (ST-10).
    let slugged = format!("{b}-pin-bun");
    let waiting = Waiting::after_first_read(&env, &marker, &["--id", &slugged]);
    run(&env, "answer", &a, &["--option", "1"]);
    run(&env, "answer", &b, &["--option", "2"]);
    assert_eq!(ids(&woke(&waiting.finish())), [b.as_str()]);

    // An agent names a case by its whole id (HU-07): a part of one, or what
    // is no id, is refused.
    for bad in ["../x", &b[..3]] {
        let out = wait(&env, &["--id", bad]);
        assert_eq!(out.code, 1, "{bad}: {out:?}");
        assert_eq!(out.stdout, "", "{bad}");
    }
}

#[test]
fn wait_by_label_and_worker() {
    let env = Env::new();
    let open = |title: &str, label: &str, worker: &str| {
        env.open(&[
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            title,
            "--label",
            label,
            "--worker",
            worker,
        ])
    };
    let mine = open("Mine", "round-1", "w1");
    let theirs = open("Theirs", "round-2", "w2");
    let other_worker = open("Other worker", "round-1", "w2");
    let marker = marker(&env);

    // Answers to cases that do not match do not wake it.
    let args = ["--label", "round-1", "--worker", "w1"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &theirs, &["--ack"]);
    run(&env, "answer", &other_worker, &["--ack"]);
    run(&env, "answer", &mine, &["--ack"]);
    assert_eq!(ids(&woke(&waiting.finish())), [mine.as_str()]);

    // A filter no case matches keeps waiting.
    let args = ["--label", "nope"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    let fresh = open("Fresh", "round-1", "w1");
    run(&env, "answer", &fresh, &["--ack"]);
    waiting.stays_asleep(&env);
}

#[test]
fn wait_by_id_and_label_waits_on_cases_that_pass_both() {
    let env = Env::new();
    let labelled = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "A",
        "--label",
        "round-1",
    ]);
    let unlabelled = env.open_fyi("B");
    let other = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "C",
        "--label",
        "round-1",
    ]);
    let marker = marker(&env);

    let args = ["--id", &labelled, "--id", &unlabelled, "--label", "round-1"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    for id in [&unlabelled, &other, &labelled] {
        run(&env, "answer", id, &["--ack"]);
    }
    assert_eq!(ids(&woke(&waiting.finish())), [labelled.as_str()]);
}

#[test]
fn wait_by_kind() {
    let env = Env::new();
    let notice = env.open_fyi("Notice");
    let decision = env.open_decision();
    let marker = marker(&env);

    let args = ["--kind", "decision"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &notice, &["--ack"]);
    run(&env, "answer", &decision, &["--option", "1"]);
    assert_eq!(ids(&woke(&waiting.finish())), [decision.as_str()]);

    // A misspelt kind is refused, rather than matching nothing.
    let out = wait(&env, &["--kind", "decison"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("decison"), "{out:?}");
}

#[test]
fn wait_waits_for_a_store_that_is_not_there_yet() {
    let mut env = Env::new();
    env.store = env.path("not-yet").join("cases.db");
    // It waits for the store, and does not make it.
    timed_out(&wait(&env, &["--timeout", "300ms"]));
    assert!(!env.path("not-yet").exists());
    // Nor does a pickup wait make the directory of its locks.
    timed_out(&wait(
        &env,
        &["--pickup", "--label", "x", "--timeout", "300ms"],
    ));
    assert!(!env.path("not-yet").exists());

    // A wait on a case by its id waits for the store too: a store's first
    // case has key 1.
    let first = Key::new(1).unwrap().to_string();
    let mut waiting = Waiting::start(&env, &[]);
    let mut by_id = Waiting::start(&env, &["--id", &first]);
    thread::sleep(Duration::from_millis(300));
    assert!(waiting.running(), "the wait did not wait for the store");
    assert!(by_id.running(), "the wait by id did not wait for the store");
    let id = env.open_decision();
    assert_eq!(id, first);
    let marker = marker(&env);
    waiting.until(&format!("warning: {marker}: "));
    by_id.until(&format!("warning: {marker}: "));
    run(&env, "answer", &id, &["--option", "1"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["fresh"], true);
    assert_eq!(ids(&woke(&by_id.finish())), [id.as_str()]);
}

/// A store that is there but cannot be read is not waited for, as one not
/// there yet is, nor ridden out: the wait fails at once, with exit 1.
#[test]
fn wait_on_a_store_that_cannot_be_read_fails_at_once() {
    let env = Env::new();
    fs::write(&env.store, "not a database, but a file of text").unwrap();
    let out = wait(&env, &[]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "", "{out:?}");
    assert!(!out.stderr.contains("goes on"), "{out:?}");
}

#[test]
fn answers_with_no_time_are_new_when_they_land_and_print_in_open_order() {
    let env = Env::new();
    let first = env.open_fyi("First");
    let second = env.open_fyi("Second");
    let third = env.open_fyi("Third");
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    // Answers written by hand or by another tool may have no answered_at.
    // They still count, because they are new to the store. Written in one
    // transaction, one read finds them all.
    let conn = lock(&env);
    for id in [&third, &first, &second] {
        insert_row(&conn, key(id), 2, "human", "answer", r#"{"ack":true}"#);
    }
    conn.execute_batch("COMMIT").unwrap();

    let lines = woke(&waiting.finish());
    // With no times to order them by, the cases are in the order they were
    // opened, which is not the order of their ids (ST-10).
    assert_eq!(
        ids(&lines),
        [first.as_str(), second.as_str(), third.as_str()]
    );
    let mut by_id = ids(&lines);
    by_id.sort_unstable();
    assert_ne!(by_id, ids(&lines));
    for line in &lines {
        assert_eq!(line["answer"]["ack"], true, "{line}");
        assert_eq!(line["fresh"], true, "{line}");
        // No event it printed has a time, and there is no --since.
        assert!(line.get("next_since").is_none(), "{line}");
    }
}

#[test]
fn wait_warns_once_about_a_case_or_row_that_does_not_fold() {
    let env = Env::new();
    let id = env.open_decision();
    let marker = marker(&env);
    let mut waiting = Waiting::after_first_read(&env, &marker, &[]);
    // The answer is skipped when the case folds, so the case is never the
    // agent's turn: the warning is the only sign that anything arrived.
    env.insert(Some(&id), 2, "human", "answer", r#"{"choice": "#);
    let row = format!("warning: {id}: 0002-human-answer.json: ");
    waiting.until(&row);

    // It reads both again, twice, and warns about neither.
    let stderr = waiting.stays_asleep(&env);
    let case = format!("warning: {marker}: ");
    assert_eq!(stderr.matches(&row).count(), 1, "{stderr}");
    assert_eq!(stderr.matches(&case).count(), 1, "{stderr}");
    // The first read's marker, the row, and the marker of each later read.
    assert_eq!(stderr.matches("warning: ").count(), 4, "{stderr}");
}

#[test]
fn wait_for_human_wakes_when_the_agent_hands_a_case_over() {
    let env = Env::new();
    let answered = env.open_decision();
    run(&env, "answer", &answered, &["--option", "1"]);
    let unanswered = env.open_decision();
    let marker = marker(&env);

    // A case that is the human's turn when wait starts does not wake it.
    timed_out(&wait(&env, &["--for", "human", "--timeout", "100ms"]));

    // A new case does, and every case that is the human's turn is printed;
    // the answered one is not.
    let waiting = Waiting::after_first_read(&env, &marker, &["--for", "human"]);
    let opened = env.open_decision();
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [unanswered.as_str(), opened.as_str()]);
    assert_eq!(lines[0]["fresh"], false);
    assert_eq!(lines[1]["fresh"], true);

    // A note after an answer hands the case back to the human.
    let args = ["--for", "human", "--id", &answered];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "pickup", &answered, &[]);
    run(
        &env,
        "note",
        &answered,
        &["--reopen", "--body", "Which version?"],
    );
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [answered.as_str()]);
    assert_eq!(lines[0]["state"], "open");
}

#[test]
fn wait_for_human_passes_over_amends_and_the_human_s_own_events() {
    let env = Env::new();
    let id = env.open_stuck();
    let marker = marker(&env);
    let args = ["--for", "human"];

    // An amend changes a case that is the human's turn already; it does not
    // announce the case again.
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(
        &env,
        "amend",
        &id,
        &["--context", "Seen on the mirror too."],
    );
    waiting.stays_asleep(&env);

    // A human event never wakes it, and a parked case is no one's turn.
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "park", &id, &[]);
    waiting.stays_asleep(&env);

    // A human resume leaves the case the agent's turn, until the agent
    // amends it. The agent does not resume a case (HU-05).
    run(&env, "resume", &id, &[]);
    let waiting = Waiting::after_first_read(&env, &marker, &["--for", "human"]);
    run(&env, "amend", &id, &["--context", "Tried the mirror."]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["state"], "open");

    let out = wait(&env, &["--for", "nobody"]);
    assert_eq!(out.code, 1, "{out:?}");
    let out = wait(&env, &["--for", "human", "--id", &id, "--timeout", "50ms"]);
    timed_out(&out);
    assert!(
        out.stderr
            .ends_with("no case needed the human within 50ms\n"),
        "{out:?}"
    );
}

#[test]
fn wait_since_a_case_id_counts_from_its_open() {
    let env = Env::new();
    // The answer lands between the open and the wait: with the case's id as
    // --since, wait still reports it.
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    let lines = woke(&wait(&env, &["--id", &id, "--since", &id]));
    assert_eq!(ids(&lines), [id.as_str()]);

    // A case opened after the answer counts from later.
    let later = env.open_decision();
    timed_out(&wait(
        &env,
        &["--id", &id, "--since", &later, "--timeout", "100ms"],
    ));

    let unknown = Key::new(99).unwrap().to_string();
    for bad in ["2026-09-17T00-00-00Z-no-such-case", "../x", &unknown] {
        let out = wait(&env, &["--since", bad]);
        assert_eq!(out.code, 1, "{bad}: {out:?}");
        assert!(
            out.stderr.starts_with(&format!("Error: --since {bad:?}")),
            "{bad}: {out:?}"
        );
    }
}

#[test]
fn wait_prints_the_next_since() {
    let env = Env::new();
    let id = env.open_stuck();
    let other = env.open_decision();
    run(&env, "answer", &other, &["--option", "1"]);
    let marker = marker(&env);

    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "park", &id, &[]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [other.as_str(), id.as_str()]);
    // Every line gives the same next_since: the time of the latest event that
    // made a printed case the agent's turn, the park.
    let next = lines[1]["next_since"].as_str().unwrap().to_owned();
    assert_eq!(lines[0]["next_since"], next.as_str());
    assert_eq!(next, last_event_at(&env, &id));

    // Passed back, it does not wake on the park or the older answer, but a
    // resume that lands before the next wait starts does wake it.
    timed_out(&wait(&env, &["--since", &next, "--timeout", "100ms"]));
    run(&env, "resume", &id, &[]);
    let lines = woke(&wait(&env, &["--since", &next]));
    assert_eq!(ids(&lines), [other.as_str(), id.as_str()]);
    assert_eq!(lines[1]["state"], "open");

    // A --since later than every event printed is kept.
    let future = from_now(3600);
    let args = ["--since", &future, "--id", &other];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "pickup", &other, &[]);
    run(
        &env,
        "note",
        &other,
        &["--reopen", "--body", "Which version?"],
    );
    // The note hands the case to the human; a new answer hands it back.
    run(&env, "answer", &other, &["--option", "2"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [other.as_str()]);
    assert_eq!(lines[0]["next_since"], future.as_str());
}

#[test]
fn wait_marks_which_cases_are_fresh() {
    let env = Env::new();
    let old = env.open_decision();
    run(&env, "answer", &old, &["--option", "1"]);
    let late = env.open_decision();
    run(&env, "answer", &late, &["--option", "1"]);
    let idle = env.open_decision();
    let marker = marker(&env);
    let fresh = |lines: &[Value]| -> Vec<bool> {
        lines
            .iter()
            .map(|line| line["fresh"].as_bool().unwrap())
            .collect()
    };

    // Without --since, only the event that landed while waiting is fresh.
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "answer", &idle, &["--option", "2"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [old.as_str(), late.as_str(), idle.as_str()]);
    assert_eq!(fresh(&lines), [false, false, true]);

    // With --since, an event in the store already and later than it is fresh
    // too.
    let since = last_event_at(&env, &old);
    let lines = woke(&wait(&env, &["--since", &since]));
    assert_eq!(ids(&lines), [old.as_str(), late.as_str(), idle.as_str()]);
    assert_eq!(fresh(&lines), [false, true, true]);
}

#[test]
fn a_human_withdraw_wakes_wait_once() {
    let env = Env::new();
    let swept = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Swept",
        "--label",
        "stale",
    ]);
    let other = env.open_fyi("Other");
    let marker = marker(&env);

    // A human withdraw, as sweep writes, wakes it, and the case is printed
    // as withdrawn (AG-04, ST-14).
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    env.ok(&["sweep", "--label", "stale", "--yes"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [swept.as_str()]);
    assert_eq!(lines[0]["state"], "withdrawn");
    assert_eq!(lines[0]["fresh"], true);

    // Unlike a parked case, it is not printed again when something else
    // wakes a wait: with the next_since, or with none.
    let next = lines[0]["next_since"].as_str().unwrap().to_owned();
    run(&env, "answer", &other, &["--ack"]);
    let lines = woke(&wait(&env, &["--since", &next]));
    assert_eq!(ids(&lines), [other.as_str()]);
    let third = env.open_fyi("Third");
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "answer", &third, &["--ack"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [other.as_str(), third.as_str()]);

    // The agent's own withdraw does not wake it.
    let fourth = env.open_fyi("Fourth");
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "withdraw", &fourth, &[]);
    waiting.stays_asleep(&env);
}

/// A message the human takes back while it is sent is not the agent's turn:
/// no agent had it, so its withdraw wakes no wait, not even one on the
/// worker it was sent to.
#[test]
fn a_human_withdraw_of_a_sent_message_wakes_no_wait() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins"]);
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &["--worker", "bun-pins"]);
    run(&env, "withdraw", &id, &["--as", "Ryan"]);
    assert_eq!(env.case(&id).state().as_str(), "withdrawn");
    waiting.stays_asleep(&env);
}

/// A human withdraw of a case an agent has, answered or picked up, such as
/// one whose session the human took for dead, wakes the agent's wait as the
/// withdraw of an open case does: the case is printed once, withdrawn, with
/// its answer, and `--pickup` picks nothing up (AG-04, AG-05, ST-14).
#[test]
fn a_human_withdraw_of_a_case_an_agent_has_wakes_wait_once() {
    let env = Env::new();
    let answered = env.open_decision();
    let picked = env.open_decision();
    let marker = marker(&env);

    // An answer the human withdraws before any wait reads it is not picked
    // up.
    run(&env, "answer", &answered, &["--option", "2"]);
    run(&env, "withdraw", &answered, &["--as", "Ryan"]);
    let args = [
        "--full", "--id", &answered, "--since", &answered, "--pickup", "--by", "me",
    ];
    let out = wait(&env, &args);
    let lines = woke(&out);
    assert_eq!(ids(&lines), [answered.as_str()]);
    assert_eq!(lines[0]["state"], "withdrawn");
    assert_eq!(lines[0]["answer"]["choice"], 2);
    assert!(lines[0].get("pickup").is_none());
    assert_eq!(without(&out.stderr, &marker), "");
    assert_eq!(
        files(&env, &answered),
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-human-withdraw.json"
        ]
    );

    // The recipe's wait picks the answer up, and the agent waits on.
    run(&env, "answer", &picked, &["--option", "1"]);
    let args = [
        "--full", "--id", &picked, "--since", &picked, "--pickup", "--by", "me",
    ];
    let lines = woke(&wait(&env, &args));
    assert_eq!(lines[0]["state"], "pickedup");
    let next = lines[0]["next_since"].as_str().unwrap().to_owned();
    let args = [
        "--full", "--id", &picked, "--since", &next, "--pickup", "--by", "me",
    ];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "withdraw", &picked, &["--as", "Ryan"]);
    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(ids(&lines), [picked.as_str()]);
    assert_eq!(lines[0]["state"], "withdrawn");
    assert_eq!(lines[0]["fresh"], true);
    assert_eq!(lines[0]["revision"], 4);
    assert_eq!(lines[0]["answer"]["choice"], 1);
    assert_eq!(lines[0]["pickup"]["by"], "me");
    assert_eq!(without(&out.stderr, &marker), "");
    // It is not printed again.
    let next = lines[0]["next_since"].as_str().unwrap().to_owned();
    let args = [
        "--full",
        "--id",
        &picked,
        "--since",
        &next,
        "--timeout",
        "100ms",
    ];
    timed_out(&wait(&env, &args));
}

#[test]
fn wait_pickup_picks_up_the_answer() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--worker",
        "bun-pins",
        "--title",
        "Pin bun?",
        "--option",
        "Pin to 1.2.3",
        "--option",
        "Float",
        "--recommend",
        "2",
    ]);
    // --since makes the answer wake it whether it lands before the wait's
    // first read or after.
    let args = [
        "--full", "--id", &id, "--since", &id, "--pickup", "--by", "bun-pins",
    ];
    let waiting = Waiting::start(&env, &args);
    // The human may choose an option the agent does not recommend.
    run(&env, "answer", &id, &["--option", "1", "--as", "Ryan"]);

    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(line["state"], "pickedup");
    assert_eq!(line["recommend"], 2);
    assert_eq!(line["answer"]["choice"], 1);
    assert_eq!(line["pickup"]["by"], "bun-pins");
    assert_eq!(line["revision"], 3);
    assert_eq!(line["fresh"], true);
    assert_eq!(out.stderr, "");

    // The pickup is the agent's, with the worker's actor (AG-03).
    assert_eq!(
        files(&env, &id),
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-agent-pickup.json"
        ]
    );
    let mut shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["events"][2]["actor"]["name"], "bun-pins");
    assert_eq!(shown["events"][2]["actor"], shown["actor"]);

    // The line is the case after the pickup, as show --json prints it
    // without its url, and its revision is the one close takes.
    let mut line = line.clone();
    for field in ["fresh", "next_since"] {
        line.as_object_mut().unwrap().remove(field);
    }
    shown.as_object_mut().unwrap().remove("url");
    assert_eq!(line, shown);
    let closed = env.ok(&["close", &id, "--outcome", "Pinned.", "--revision", "3"]);
    assert_eq!(closed, format!("{id} closed\n"));
}

#[test]
fn the_recipe_catches_an_answer_that_lands_before_the_wait_starts() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Notice",
        "--worker",
        "w1",
    ]);
    run(&env, "answer", &id, &["--ack", "--note", "Seen."]);

    // AG-03's recipe: open, then wait --id ID --since ID --pickup --by NAME.
    let args = ["--id", &id, "--since", &id, "--pickup", "--by", "w1"];
    let lines = woke(&wait(&env, &args));
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["state"], "pickedup");
    assert_eq!(lines[0]["answer"]["note"], "Seen.");
    assert_eq!(lines[0]["revision"], 3);

    // A later wait never prints the case again.
    let next = lines[0]["next_since"].as_str().unwrap();
    timed_out(&wait(
        &env,
        &["--id", &id, "--since", next, "--timeout", "100ms"],
    ));
    timed_out(&wait(
        &env,
        &["--id", &id, "--since", &id, "--timeout", "100ms"],
    ));
}

#[test]
fn wait_pickup_leaves_parked_resumed_and_withdrawn_cases() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "stuck",
        "--urgency",
        "blocking",
        "--title",
        "Blocked",
        "--label",
        "stale",
    ]);

    // Each wait passes --since, so the event wakes it whether it lands before
    // the first read or after. An empty stderr shows that wait did not try to
    // pick up a case with no answer. The human withdraws as sweep does.
    let mut since = id.clone();
    for (write, state) in [
        (&["park", &id][..], "parked"),
        (&["resume", &id], "open"),
        (&["sweep", "--label", "stale", "--yes"], "withdrawn"),
    ] {
        let command = write[0];
        let args = ["--id", &id, "--since", &since, "--pickup"];
        let waiting = Waiting::start(&env, &args);
        env.ok(write);
        let out = waiting.finish();
        let lines = woke(&out);
        assert_eq!(ids(&lines), [id.as_str()], "{command}");
        assert_eq!(lines[0]["state"], state, "{command}");
        assert_eq!(lines[0]["revision"], env.revision(&id), "{command}");
        assert!(lines[0].get("pickup").is_none(), "{command}");
        assert_eq!(out.stderr, "", "{command}");
        since = lines[0]["next_since"].as_str().unwrap().to_owned();
    }
    assert_eq!(
        files(&env, &id),
        [
            "0001-agent-open.json",
            "0002-human-park.json",
            "0003-human-resume.json",
            "0004-human-withdraw.json"
        ]
    );
}

#[test]
fn wait_pickup_leaves_a_case_that_changed_since_it_was_read() {
    let note = (
        3,
        "agent",
        "note",
        r#"{"body": "1.2.3 has a CVE. Pin to which patch?"}"#,
    );
    for (name, rows, last, now) in [
        ("noted", vec![note], "0003-agent-note.json", "open"),
        (
            "noted and answered again",
            vec![note, (4, "human", "answer", r#"{"choice": 2}"#)],
            "0004-human-answer.json",
            "answered",
        ),
        (
            "picked up by another wait",
            vec![(3, "agent", "pickup", r#"{"by": "other"}"#)],
            "0003-agent-pickup.json",
            "pickedup",
        ),
        (
            "withdrawn by the human",
            vec![(3, "human", "withdraw", r#"{"reason": "gone"}"#)],
            "0003-human-withdraw.json",
            "withdrawn",
        ),
    ] {
        let env = Env::new();
        let changed = env.open_decision();
        let fine = env.open_decision();
        run(&env, "answer", &changed, &["--option", "1"]);
        run(&env, "answer", &fine, &["--option", "2"]);
        let marker = marker(&env);

        // The store's lock is held until the change is written, after the
        // wait has read the cases, so that its pickups wait for the change.
        let lock = lock(&env);
        let args = [
            "--full", "--id", &changed, "--id", &fine, "--since", &changed, "--pickup", "--by",
            "me",
        ];
        let waiting = Waiting::after_first_read(&env, &marker, &args);
        for &(seq, author, event, data) in &rows {
            insert_row(&lock, key(&changed), seq, author, event, data);
        }
        lock.execute_batch("COMMIT").unwrap();

        let out = waiting.finish();
        let lines = woke(&out);
        assert_eq!(ids(&lines), [changed.as_str(), fine.as_str()], "{name}");
        // The case that changed is printed as read: answered, at revision 2.
        let read = &lines[0];
        assert_eq!(read["state"], "answered", "{name}");
        assert_eq!(read["revision"], 2, "{name}");
        assert_eq!(read["answer"]["choice"], 1, "{name}");
        assert!(read.get("pickup").is_none(), "{name}");
        // The other is picked up as usual.
        assert_eq!(lines[1]["state"], "pickedup", "{name}");
        assert_eq!(lines[1]["revision"], 3, "{name}");
        assert_eq!(lines[1]["pickup"]["by"], "me", "{name}");
        // One line on stderr says why, and the state the case is in now.
        let warning = without(&out.stderr, &marker);
        let revision = 2 + rows.len();
        assert!(
            warning.starts_with(&format!("warning: {changed}: not picked up: ")),
            "{name}: {warning}"
        );
        assert!(
            warning.ends_with(&format!(
                "read at revision 2, now at {revision}; it is now {now}\n"
            )),
            "{name}: {warning}"
        );
        assert_eq!(warning.lines().count(), 1, "{name}: {warning}");

        let events = files(&env, &changed);
        assert_eq!(events.len(), revision, "{name}");
        assert_eq!(events.last().unwrap(), last, "{name}");
        let case = env.case(&fine);
        assert_eq!(
            (case.state().as_str(), case.revision()),
            ("pickedup", 3),
            "{name}"
        );
    }
}

#[test]
fn two_pickup_waits_on_one_case_pick_it_up_once() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "Ship it?",
        "--option",
        "yes",
        "--option",
        "no",
        "--label",
        "deploy",
    ]);
    run(&env, "answer", &id, &["--option", "1"]);
    let marker = marker(&env);

    // The store's lock is held until both waits have read the answer, so
    // that both try to pick it up at the revision they read. Both start
    // before either read is waited for, so that the first wait's pickup
    // waits for the lock about as long as the second wait takes to read,
    // not as long as it takes to start, well inside the store's 5 s.
    // Two pickup waits on one id refuse each other (AG-05), so one names the
    // case by its id and the other by its label.
    let lock = lock(&env);
    let mut waits: Vec<(&str, Waiting)> = [("a", "--id", id.as_str()), ("b", "--label", "deploy")]
        .into_iter()
        .map(|(by, flag, value)| {
            let args = [
                "--full", flag, value, "--since", &id, "--pickup", "--by", by,
            ];
            (by, Waiting::start(&env, &args))
        })
        .collect();
    for (_, waiting) in &mut waits {
        waiting.until(&format!("warning: {marker}: "));
    }
    lock.execute_batch("ROLLBACK").unwrap();

    let mut picked = Vec::new();
    for (by, waiting) in waits {
        let out = waiting.finish();
        let lines = woke(&out);
        assert_eq!(ids(&lines), [id.as_str()], "{by}");
        let line = &lines[0];
        let warning = without(&out.stderr, &marker);
        if line["state"] == "pickedup" {
            assert_eq!(line["revision"], 3, "{by}");
            assert_eq!(line["pickup"]["by"], by, "{by}");
            assert_eq!(warning, "", "{by}");
            picked.push(by);
        } else {
            assert_eq!(line["state"], "answered", "{by}");
            assert_eq!(line["revision"], 2, "{by}");
            assert!(line.get("pickup").is_none(), "{by}");
            assert!(
                warning.ends_with("read at revision 2, now at 3; it is now pickedup\n"),
                "{by}: {warning}"
            );
        }
    }
    assert_eq!(picked.len(), 1, "picked up by {picked:?}");
    assert_eq!(
        files(&env, &id),
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-agent-pickup.json"
        ]
    );
    assert_eq!(env.case(&id).pickup().unwrap().by, picked[0]);
}

#[test]
fn wait_pickup_by_label() {
    let env = Env::new();
    let open = |kind: &str, title: &str, label: &str| {
        env.open(&[
            "--kind",
            kind,
            "--urgency",
            "today",
            "--title",
            title,
            "--label",
            label,
        ])
    };
    let early = open("fyi", "Early", "round-1");
    let parked = open("stuck", "Blocked", "round-1");
    let late = open("fyi", "Late", "round-1");
    let theirs = open("fyi", "Theirs", "round-2");
    run(&env, "answer", &early, &["--ack"]);
    run(&env, "park", &parked, &[]);

    // A --since at the park, so that the answer and the park are in the store
    // already, and the answer to late is new, whenever the first read runs.
    let since = last_event_at(&env, &parked);
    let args = [
        "--full", "--label", "round-1", "--since", &since, "--pickup", "--by", "w1",
    ];
    let waiting = Waiting::start(&env, &args);
    run(&env, "answer", &theirs, &["--ack"]);
    run(&env, "answer", &late, &["--ack"]);

    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(
        ids(&lines),
        [early.as_str(), parked.as_str(), late.as_str()]
    );
    // Nothing on stderr: the parked case is not tried.
    assert_eq!(out.stderr, "");
    // A parked case is printed on each wake, and is not fresh (AG-06).
    for (line, (state, fresh)) in
        lines
            .iter()
            .zip([("pickedup", false), ("parked", false), ("pickedup", true)])
    {
        assert_eq!(line["state"], state, "{line}");
        assert_eq!(line["fresh"], fresh, "{line}");
        assert_eq!(line.get("pickup").is_some(), state == "pickedup", "{line}");
    }
    for (id, state) in [
        (&early, "pickedup"),
        (&parked, "parked"),
        (&late, "pickedup"),
        (&theirs, "answered"),
    ] {
        assert_eq!(env.case(id).state().as_str(), state, "{id}");
    }
}

#[test]
fn wait_pickup_refuses_what_could_pick_up_other_agents_cases() {
    let env = Env::new();
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    let id = id.as_str();
    for (args, want) in [
        (&["--id", id, "--by", "me"][..], "--by needs --pickup"),
        // Even an empty one, as "$NAME" gives when the variable is not set.
        (&["--id", id, "--by", ""], "--by needs --pickup"),
        (
            &["--pickup", "--for", "human", "--id", id],
            "--pickup is only for --for agent",
        ),
        (&["--pickup"], "--pickup needs --id, --label or --worker"),
        (
            &["--pickup", "--kind", "decision"],
            "--pickup needs --id, --label or --worker",
        ),
        // A blank worker, as "$CASES_WORKER" gives when the variable is not
        // set, would match every case opened without one, this one too.
        (
            &["--pickup", "--worker", ""],
            "--pickup refuses a blank --worker",
        ),
        (
            &["--pickup", "--id", id, "--worker", " "],
            "--pickup refuses a blank --worker",
        ),
        // No case has a blank label, so it names none of the agent's own.
        (
            &["--pickup", "--label", ""],
            "--pickup refuses a blank --label",
        ),
        (
            &["--pickup", "--worker", "me", "--label", " "],
            "--pickup refuses a blank --label",
        ),
    ] {
        let mut all = vec!["--since", id];
        all.extend(args);
        let out = wait(&env, &all);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert_eq!(out.stdout, "", "{args:?}");
        assert!(out.stderr.contains(want), "{args:?}: {out:?}");
    }
    assert_eq!(env.case(id).state().as_str(), "answered");

    // A worker names the agent's own cases, and --by can be left out, as for
    // pickup.
    let mine = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Mine",
        "--worker",
        "w1",
    ]);
    run(&env, "answer", &mine, &["--ack"]);
    let args = ["--since", id, "--worker", "w1", "--pickup"];
    let lines = woke(&wait(&env, &args));
    assert_eq!(ids(&lines), [mine.as_str()]);
    assert_eq!(lines[0]["state"], "pickedup");
    assert!(lines[0]["pickup"].get("by").is_none(), "{}", lines[0]);
    assert_eq!(env.case(id).state().as_str(), "answered");
}

/// Checks that `out` is the refusal of a pickup wait that overlaps the one
/// `holder` runs on `what`: exit 1, nothing on stdout, and the holder named.
fn refused(out: &Output, what: &str, holder: &Waiting) {
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "", "{out:?}");
    let want = format!(
        "Error: another wait is already picking up {what} (pid {}, since ",
        holder.child.id()
    );
    assert!(out.stderr.starts_with(&want), "{want}: {out:?}");
    assert!(
        out.stderr
            .ends_with("; read its output instead of starting a second\n"),
        "{out:?}"
    );
}

/// A pickup wait on a worker, label or id another pickup wait is on refuses
/// to start, naming the wait that is (AG-05): the first would take what the
/// second waits for. A wait with a worker locks its workers, not its labels;
/// one without locks its labels; both lock their ids.
#[test]
fn a_second_pickup_wait_on_the_same_worker_label_or_id_is_refused() {
    let env = Env::new();
    let id = env.open_decision();
    let marker = marker(&env);
    for (first, second, what) in [
        (
            &["--label", "deploy"][..],
            &["--label", "other", "--label", "deploy"][..],
            "label \"deploy\"",
        ),
        (
            &["--worker", "w1", "--label", "a"],
            &["--worker", "w1", "--label", "b"],
            "worker \"w1\"",
        ),
        (
            &["--id", id.as_str()],
            &["--label", "other", "--id", id.as_str()],
            &format!("id {id:?}"),
        ),
    ] {
        let mut args = vec!["--pickup"];
        args.extend(first);
        let mut holder = Waiting::after_first_read(&env, &marker, &args);
        let mut args = vec!["--pickup"];
        args.extend(second);
        refused(&wait(&env, &args), what, &holder);
        assert!(holder.running(), "{first:?}");
        holder.child.kill().unwrap();
        holder.child.wait().unwrap();
    }
    assert_eq!(env.case(&id).state().as_str(), "open");
}

/// A pickup wait on the store by another path to it is refused as one by
/// the same path is (AG-05): through a symlink to the store file, through a
/// symlink to a directory and back up past it, and relative to where it
/// runs.
#[cfg(unix)]
#[test]
fn a_second_pickup_wait_on_the_same_store_by_another_path_is_refused() {
    let mut env = Env::new();
    env.open_decision();
    let marker = marker(&env);
    let real = env.store.clone();
    let link = env.path("link.db");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    // `deep/../../cases.db` is the store, as `deep` is `a/b`; as text alone
    // it would be the `cases.db` above the test's directory.
    fs::create_dir_all(env.path("a").join("b")).unwrap();
    std::os::unix::fs::symlink(env.path("a").join("b"), env.path("deep")).unwrap();
    let up = env.path("deep").join("..").join("..").join("cases.db");
    let relative = PathBuf::from("cases.db");
    for (first, second) in [
        (&real, &link),
        (&link, &real),
        (&real, &up),
        (&up, &relative),
    ] {
        env.store = first.clone();
        let mut holder = Waiting::after_first_read(&env, &marker, &["--pickup", "--worker", "w"]);
        env.store = second.clone();
        refused(
            &wait(&env, &["--pickup", "--worker", "w"]),
            "worker \"w\"",
            &holder,
        );
        assert!(holder.running(), "{first:?}");
        holder.child.kill().unwrap();
        holder.child.wait().unwrap();
    }
}

/// Pickup waits on different workers, labels and ids run side by side, and
/// so do two waits of different workers that share a label: the first to
/// take a message to it has it (AG-16).
#[test]
fn pickup_waits_on_different_scopes_run_side_by_side() {
    let env = Env::new();
    let id = env.open_decision();
    let other = env.open_decision();
    let marker = marker(&env);
    for (first, second) in [
        (&["--label", "a"][..], &["--label", "b"][..]),
        (&["--worker", "w1"], &["--worker", "w2"]),
        (
            &["--worker", "w1", "--label", "shared"],
            &["--worker", "w2", "--label", "shared"],
        ),
        (&["--id", id.as_str()], &["--id", other.as_str()]),
    ] {
        let mut args = vec!["--pickup"];
        args.extend(first);
        let mut holder = Waiting::after_first_read(&env, &marker, &args);
        let mut args = vec!["--pickup", "--timeout", "100ms"];
        args.extend(second);
        timed_out(&wait(&env, &args));
        assert!(holder.running(), "{first:?}");
        holder.child.kill().unwrap();
        holder.child.wait().unwrap();
    }
}

/// A pickup wait that names a worker, label or id twice takes its lock once,
/// rather than refusing itself.
#[test]
fn a_pickup_wait_that_names_a_scope_twice_runs() {
    let env = Env::new();
    let id = env.open_decision();
    for args in [
        &["--worker", "w", "--worker", "w"][..],
        &["--label", "deploy", "--label", "deploy"],
        &["--id", id.as_str(), "--id", id.as_str()],
    ] {
        let mut args = args.to_vec();
        args.extend(["--pickup", "--timeout", "100ms"]);
        timed_out(&wait(&env, &args));
    }
}

/// A wait without --pickup takes no lock, and runs beside a pickup wait on
/// the same label.
#[test]
fn a_wait_without_pickup_runs_beside_a_pickup_wait() {
    let env = Env::new();
    env.open_fyi("A case, so that the store is there");
    let marker = marker(&env);
    let mut holder = Waiting::after_first_read(&env, &marker, &["--pickup", "--label", "deploy"]);
    timed_out(&wait(&env, &["--label", "deploy", "--timeout", "100ms"]));
    assert!(holder.running());
    holder.child.kill().unwrap();
    holder.child.wait().unwrap();
}

/// A pickup wait's locks go when it ends, however it ends, so the next wait
/// on the same label starts: one chained straight after a wait that picked up
/// and exited, and one after a wait that was killed.
#[test]
fn a_pickup_wait_after_the_last_one_ended_runs() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Shipped",
        "--label",
        "deploy",
    ]);
    run(&env, "answer", &id, &["--ack"]);
    let lines = woke(&wait(
        &env,
        &["--pickup", "--label", "deploy", "--since", &id],
    ));
    assert_eq!(ids(&lines), [id.as_str()]);
    timed_out(&wait(
        &env,
        &["--pickup", "--label", "deploy", "--timeout", "100ms"],
    ));

    let marker = marker(&env);
    let mut killed = Waiting::after_first_read(&env, &marker, &["--pickup", "--label", "deploy"]);
    killed.child.kill().unwrap();
    killed.child.wait().unwrap();
    timed_out(&wait(
        &env,
        &["--pickup", "--label", "deploy", "--timeout", "100ms"],
    ));
}

#[test]
fn a_pickup_wait_that_times_out_writes_nothing() {
    let env = Env::new();
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    // The answer was in the store before the wait started, so it is not new,
    // and nothing wakes the wait.
    let args = ["--id", &id, "--pickup", "--by", "me", "--timeout", "300ms"];
    timed_out(&wait(&env, &args));
    let case = env.case(&id);
    assert_eq!((case.state().as_str(), case.revision()), ("answered", 2));
}

#[test]
fn a_pickup_wait_that_cannot_print_leaves_the_cases_after_answered() {
    let env = Env::new();
    let open = |title: &str| {
        env.open(&[
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            title,
            "--worker",
            "w1",
        ])
    };
    let first = open("First");
    let second = open("Second");
    for id in [&first, &second] {
        run(&env, "answer", id, &["--ack"]);
    }

    // Nothing reads the wait's stdout, so its first line fails to print.
    let (reader, writer) = common::pipe();
    drop(reader);
    let args = ["--worker", "w1", "--since", &first, "--pickup"];
    let out: Output = common::output(
        wait_command(&env, &args)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(Stdio::piped()),
    )
    .into();
    assert_eq!(out.code, 1, "{out:?}");
    // Each case is picked up just before it is printed, so the one after the
    // line that failed is left for the next wait, and stderr names the one
    // picked up whose line did not print.
    let state = |id: &str| env.case(id).state().as_str();
    assert_eq!(
        [state(first.as_str()), state(second.as_str())],
        ["pickedup", "answered"]
    );
    let lost = format!("warning: {first}: picked up, but not printed: ");
    assert!(out.stderr.starts_with(&lost), "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
}

#[test]
fn a_pickup_that_makes_its_actor_names_it() {
    let env = Env::new();
    env.open_fyi("Makes the store");
    // A case opened by hand, whose worker has no actor in the store yet.
    let open = r#"{"kind": "fyi", "urgency": "today", "title": "By hand", "worker": "w9"}"#;
    let id = env.insert(None, 1, "agent", "open", open);
    let marker = marker(&env);
    let args = ["--full", "--id", &id, "--pickup", "--by", "me"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &id, &["--ack"]);

    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    // The line names the actor the pickup made, as show --json does.
    let mut line = lines[0].clone();
    assert_eq!(line["state"], "pickedup");
    assert_eq!(line["events"][2]["actor"]["kind"], "agent");
    assert_eq!(line["events"][2]["actor"]["name"], "w9");
    for field in ["fresh", "next_since"] {
        line.as_object_mut().unwrap().remove(field);
    }
    let mut shown = json(&env.ok(&["show", &id, "--json"]));
    shown.as_object_mut().unwrap().remove("url");
    assert_eq!(line, shown);
}

#[test]
fn each_case_is_one_line_whatever_its_records_hold() {
    let env = Env::new();
    // NEL and the line and paragraph separators, which some readers end a
    // line at...
    let title = "Line\u{2028}and paragraph\u{2029}separators, and NEL\u{85}";
    let id = env.open_fyi(title);
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    // ...and a record stored with line breaks, by hand or by another tool.
    let answer = "{\"ack\": true,\n  \"note\": \"Seen.\"\r\n}";
    env.insert(Some(&id), 2, "human", "answer", answer);

    let out = waiting.finish();
    let breaks = ['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'];
    assert_eq!(out.stdout.matches(breaks).count(), 1, "{out:?}");
    let lines = woke(&out);
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["title"], title);
    assert_eq!(lines[0]["answer"]["note"], "Seen.");
}

#[test]
fn every_event_of_a_store_made_again_while_wait_runs_is_new() {
    let env = Env::new();
    let old = env.open_decision();
    run(&env, "answer", &old, &["--option", "1"]);
    let marker = marker(&env);
    let for_agent = Waiting::after_first_read(&env, &marker, &[]);
    let for_human = Waiting::after_first_read(&env, &marker, &["--for", "human"]);

    // The store is thrown away and made again (ST-08). Its first case has
    // the old first case's key, so its events have the old events' case and
    // seq.
    remove_store(&env);
    let new = env.open_decision();
    assert_eq!(new, old, "the new store's first case has the old one's id");
    let lines = woke(&for_human.finish());
    assert_eq!(ids(&lines), [new.as_str()]);
    assert_eq!(lines[0]["fresh"], true);

    run(&env, "answer", &new, &["--option", "2"]);
    let lines = woke(&for_agent.finish());
    assert_eq!(ids(&lines), [new.as_str()]);
    assert_eq!(lines[0]["answer"]["choice"], 2);
    assert_eq!(lines[0]["fresh"], true);
}

#[test]
fn an_id_names_the_case_it_named_when_the_wait_started() {
    let env = Env::new();
    let mine = env.open_decision();
    let marker = marker(&env);
    let args = ["--id", &mine, "--since", &mine, "--pickup", "--by", "me"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);

    // The store is thrown away and made again, and another agent's case
    // takes the key, and so the id, of the case the wait is on.
    remove_store(&env);
    let theirs = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Theirs",
        "--worker",
        "them",
    ]);
    assert_eq!(
        theirs, mine,
        "the new store's first case has the old one's id"
    );
    run(&env, "answer", &theirs, &["--ack"]);

    // Its answer is not this wait's to pick up (AG-05).
    waiting.stays_asleep(&env);
    assert_eq!(env.case(&theirs).state().as_str(), "answered");
}

#[test]
fn a_decision_goes_from_open_to_closed() {
    let mut env = Env::new();
    env.store = env.path("cases").join("cases.db");
    let body = env.path("body.md");
    common::write_file(&body, "Bun 1.2.4 breaks the lockfile.\n");

    // The agent opens the case and starts waiting before anyone answers.
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "blocking",
        "--worker",
        "bun-pins",
        "--brief",
        "/briefs/bun-pins.md",
        "--title",
        "Pin bun or float?",
        "--body-file",
        body.to_str().unwrap(),
        "--option",
        "Pin to 1.2.3",
        "--option",
        "Float and fix the lockfile",
        "--link",
        "https://github.com/oven-sh/bun/issues/1",
    ]);
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &["--full"]);

    // The human answers from the other side of the store.
    run(
        &env,
        "answer",
        &id,
        &["--option", "1", "--note", "Revisit after 1.3"],
    );
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(line["state"], "answered");
    assert_eq!(line["answer"]["choice"], 1);
    assert_eq!(line["answer"]["note"], "Revisit after 1.3");
    assert_eq!(line["brief"], "/briefs/bun-pins.md");

    run(&env, "pickup", &id, &["--by", "bun-pins"]);
    let close = env.store_args(&["close", &id, "--outcome-file", "-"]);
    let out = env.run_with_stdin("Pinned in #12.\n", close);
    assert_eq!(out.code, 0, "{out:?}");

    // A closed case is no longer waited on, even from before it was opened.
    let args = [
        "--full",
        "--since",
        "2000-01-01T00:00:00Z",
        "--timeout",
        "50ms",
    ];
    timed_out(&wait(&env, &args));
    assert_eq!(
        files(&env, &id),
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-agent-pickup.json",
            "0004-agent-close.json"
        ]
    );
    assert!(env.ok(&["list", "--state", "closed"]).contains(&id));
}

/// A case from a newer cases is warned about as such, as list does (ST-05).
#[test]
fn a_case_from_a_newer_cases_is_warned_about_as_such() {
    let env = Env::new();
    env.open_decision();
    let newer = env.insert(
        None,
        1,
        "agent",
        "open",
        r#"{"kind": "poll", "urgency": "today", "title": "T"}"#,
    );
    let out = wait(&env, &["--timeout", "50ms"]);
    timed_out(&out);
    let warning = format!("warning: {newer}: ");
    let line = out
        .stderr
        .lines()
        .find(|line| line.starts_with(&warning))
        .unwrap();
    assert!(line.contains("perhaps written by a newer cases"), "{line}");
}

/// Without `--timeout`, a wait waits on (AG-04): it is still waiting after
/// its reads over a few seconds.
#[test]
fn without_a_timeout_a_wait_waits_on() {
    let env = Env::new();
    env.open_decision();
    let mut child = common::spawn(
        env.command(env.store_args(&["wait"]))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    thread::sleep(Duration::from_secs(3));
    let status = child.try_wait().expect("the wait's status");
    child.kill().expect("the wait is stopped");
    child.wait().expect("the wait ends");
    assert_eq!(status, None, "the wait ended by itself");
}

/// Sends a message as Ryan with `args`, and gives its id.
fn message(env: &Env, args: &[&str]) -> String {
    let mut all = vec!["message", "--as", "Ryan", "--title", "Rebase onto main"];
    all.extend(args);
    env.ok(&all).trim().to_owned()
}

/// The args of a wait that listens as `worker`, with `more`.
fn listening<'a>(worker: &'a str, more: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["--pickup", "--worker", worker, "--by", worker];
    all.extend(more);
    all
}

/// A listening wait takes a message sent to its worker while it waits: it
/// picks it up as its session and prints it, fresh (AG-16).
#[test]
fn a_listening_wait_takes_a_message_sent_to_its_worker() {
    let env = Env::new();
    env.open_fyi("A case, so that the store is there");
    let marker = marker(&env);
    let waiting = Waiting::after_first_read(&env, &marker, &listening("bun-pins", &["--full"]));
    let id = message(&env, &["--to", "bun-pins", "--body", "Then run the tests."]);
    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(
        (&line["kind"], &line["state"]),
        (&"message".into(), &"pickedup".into())
    );
    assert_eq!(
        (&line["fresh"], &line["revision"]),
        (&true.into(), &2.into())
    );
    assert_eq!(line["body"], "Then run the tests.");
    assert_eq!(line["pickup"]["by"], "bun-pins");
    assert_eq!(line["events"][1]["actor"]["name"], "bun-pins");
    assert_eq!(without(&out.stderr, &marker), "");
    assert_eq!(
        files(&env, &id),
        ["0001-human-open.json", "0002-agent-pickup.json"]
    );
}

/// A listening wait takes each message before it prints any, so one whose
/// first line fails to print names every message it took on stderr: no later
/// wait takes them again (AG-16).
#[test]
fn a_listening_wait_that_cannot_print_names_each_message_it_took() {
    let env = Env::new();
    let first = message(&env, &["--to", "w1", "--body", "First."]);
    let second = message(&env, &["--to", "w1", "--body", "Second."]);

    // Nothing reads the wait's stdout, so its first line fails to print.
    let (reader, writer) = common::pipe();
    drop(reader);
    let out: Output = common::output(
        wait_command(&env, &listening("w1", &[]))
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(Stdio::piped()),
    )
    .into();
    assert_eq!(out.code, 1, "{out:?}");
    for id in [&first, &second] {
        assert_eq!(env.case(id).state().as_str(), "pickedup", "{id}");
        let lost = format!("warning: {id}: picked up, but not printed: ");
        assert!(out.stderr.contains(&lost), "{id}: {out:?}");
    }
    assert_eq!(out.stderr.lines().count(), 2, "{out:?}");
}

/// Waits until the case `id` is picked up, as a wait that takes it does.
fn until_picked_up(env: &Env, id: &str) {
    let deadline = Instant::now() + PATIENCE;
    while env.case(id).state().as_str() != "pickedup" {
        assert!(Instant::now() < deadline, "{id} was not picked up");
        thread::sleep(Duration::from_millis(50));
    }
}

/// A steady wait that took a message and then fails to read the store, as
/// it collects, still prints the message it took, and says the failure on
/// stderr: no later wait takes it again (AG-16).
#[test]
fn a_steady_wait_that_fails_after_taking_a_message_prints_it() {
    let env = Env::new();
    // A case it waits on still open, so that it lingers after the message.
    env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Left open",
        "--worker",
        "w9",
    ]);
    let marker = marker(&env);
    let args = listening("w9", &["--mood", "steady", "--timeout", "30s"]);
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    let sent = message(&env, &["--to", "w9", "--body", "Then run the tests."]);
    until_picked_up(&env, &sent);

    spoil_store(&env);
    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(ids(&lines), [sent.as_str()], "{out:?}");
    assert_eq!(lines[0]["state"], "pickedup", "{out:?}");
    assert_eq!(
        without(&out.stderr, &marker),
        "warning: file is not a database\n",
        "{out:?}"
    );
}

/// A steady wait that collected an answer, took a message and then fails
/// to read the store prints both: printing the message alone would move
/// its `next_since` past the answer, which a later wait would then miss.
#[test]
fn a_steady_wait_that_fails_after_taking_a_message_prints_what_it_collected() {
    let env = Env::new();
    let open = |title: &str| {
        env.open(&[
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            title,
            "--worker",
            "w9",
        ])
    };
    let answered = open("Answered");
    let _left = open("Left open");
    let marker = marker(&env);
    let args = listening("w9", &["--mood", "steady", "--timeout", "30s"]);
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &answered, &["--ack"]);
    thread::sleep(Duration::from_millis(1500));
    let sent = message(&env, &["--to", "w9", "--body", "Then run the tests."]);
    until_picked_up(&env, &sent);

    spoil_store(&env);
    let out = waiting.finish();
    let lines = woke(&out);
    assert_eq!(ids(&lines), [answered.as_str(), sent.as_str()], "{out:?}");
    // The store cannot take the answer's pickup: it is printed as read.
    assert_eq!(lines[0]["state"], "answered", "{out:?}");
    assert_eq!(lines[1]["state"], "pickedup", "{out:?}");
    assert!(
        without(&out.stderr, &marker).starts_with("warning: file is not a database\n"),
        "{out:?}"
    );
}

/// A message waits until a wait takes it: one sent before the wait began is
/// taken at once, whatever `--since` says, and one to a label goes to a wait
/// on that label.
#[test]
fn a_message_sent_before_the_wait_is_taken_at_once() {
    let env = Env::new();
    let id = message(&env, &["--label", "release"]);
    let later = from_now(3600);
    let out = wait(
        &env,
        &listening(
            "lead-2",
            &["--full", "--label", "release", "--since", &later],
        ),
    );
    let lines = woke(&out);
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["pickup"]["by"], "lead-2");
    // A later wait does not print it again.
    timed_out(&wait(
        &env,
        &listening(
            "lead-2",
            &["--full", "--label", "release", "--timeout", "2s"],
        ),
    ));
}

/// A listening wait picks a message up as its first worker when it has no
/// `--by`, or a blank one.
#[test]
fn a_listening_wait_without_a_name_takes_a_message_as_its_worker() {
    let env = Env::new();
    for by in [None, Some(""), Some("  ")] {
        let id = message(&env, &["--to", "bun-pins"]);
        let mut args = vec![
            "--full", "--pickup", "--worker", "bun-pins", "--worker", "lead-2",
        ];
        args.extend(by.map(|by| ["--by", by]).into_iter().flatten());
        let lines = woke(&wait(&env, &args));
        assert_eq!(ids(&lines), [id.as_str()], "{by:?}");
        assert_eq!(lines[0]["pickup"]["by"], "bun-pins", "{by:?}");
        assert_eq!(lines[0]["events"][1]["actor"]["name"], "bun-pins", "{by:?}");
    }
}

/// Only a wait that listens takes a message, and only one sent to one of its
/// workers or labels: the message stays sent for every other wait.
#[test]
fn only_a_wait_that_listens_takes_a_message_sent_to_it() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins", "--label", "release"]);
    let fyi = env.open_fyi("A case of the worker's own");
    for args in [
        vec!["--worker", "bun-pins", "--label", "release"],
        vec!["--pickup", "--label", "release", "--by", "bun-pins"],
        vec!["--pickup", "--id", &fyi, "--worker", "bun-pins"],
        vec!["--pickup", "--worker", "bun-pins", "--kind", "fyi"],
        vec!["--pickup", "--worker", "lead-2", "--label", "docs"],
        vec!["--for", "human", "--worker", "bun-pins"],
    ] {
        let mut all = args.clone();
        all.extend(["--timeout", "2s"]);
        let out = wait(&env, &all);
        timed_out(&out);
        assert_eq!(env.case(&id).state().as_str(), "sent", "{args:?}");
    }
    let out = wait(
        &env,
        &listening("bun-pins", &["--kind", "message", "--kind", "fyi"]),
    );
    assert_eq!(ids(&woke(&out)), [id.as_str()]);
}

/// Two waits that listen on one label, each holding the store's lock back
/// until both have read the message: one takes it, and the other prints
/// nothing and goes on waiting.
#[test]
fn two_listening_waits_take_a_message_once() {
    let env = Env::new();
    let id = message(&env, &["--label", "release"]);
    let marker = marker(&env);
    let lock = lock(&env);
    let mut waits: Vec<(&str, Waiting)> = ["w1", "w2"]
        .into_iter()
        .map(|by| {
            (
                by,
                Waiting::start(&env, &listening(by, &["--full", "--label", "release"])),
            )
        })
        .collect();
    for (_, waiting) in &mut waits {
        waiting.until(&format!("warning: {marker}: "));
    }
    lock.execute_batch("ROLLBACK").unwrap();
    // The one that took it ends; the other is still waiting.
    let deadline = Instant::now() + PATIENCE;
    let taker = loop {
        if let Some(at) = waits.iter_mut().position(|(_, waiting)| !waiting.running()) {
            break at;
        }
        assert!(Instant::now() < deadline, "neither wait took the message");
        thread::sleep(Duration::from_millis(50));
    };
    let (by, waiting) = waits.remove(taker);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(lines[0]["pickup"]["by"], by);
    let (_, other) = waits.remove(0);
    let stderr = other.stays_asleep(&env);
    assert!(!stderr.contains(&id), "{stderr}");
    assert_eq!(env.case(&id).pickup().unwrap().by, by);
    assert_eq!(
        files(&env, &id),
        ["0001-human-open.json", "0002-agent-pickup.json"]
    );
}

/// Stores a human event on the case `id` through `conn`, whose transaction
/// holds the store's lock, and commits it.
fn human_event(conn: &Connection, id: &str, seq: u64, event: &str, data: &str) {
    insert_row(conn, key(id), seq, "human", event, data);
    conn.execute_batch("COMMIT").unwrap();
}

/// An edit that lands between a wait's read of a message and its pickup
/// refuses the pickup, which was of the message as it was: the wait takes it
/// again as edited, on its next read.
#[test]
fn a_message_edited_before_the_pickup_is_taken_as_edited() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins", "--body", "Rebase."]);
    let marker = marker(&env);
    let lock = lock(&env);
    let mut waiting = Waiting::start(&env, &listening("bun-pins", &[]));
    waiting.until(&format!("warning: {marker}: "));
    human_event(&lock, &id, 2, "amend", r#"{"body": "Rebase, then test."}"#);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(
        (&lines[0]["body"], &lines[0]["revision"]),
        (&"Rebase, then test.".into(), &3.into())
    );
    assert_eq!(
        files(&env, &id),
        [
            "0001-human-open.json",
            "0002-human-amend.json",
            "0003-agent-pickup.json"
        ]
    );
}

/// A withdraw that lands between a wait's read of a message and its pickup
/// refuses the pickup: no one has the message, so no wait prints it.
#[test]
fn a_message_withdrawn_before_the_pickup_is_not_taken() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins"]);
    let marker = marker(&env);
    let lock = lock(&env);
    let mut waiting = Waiting::start(&env, &listening("bun-pins", &[]));
    waiting.until(&format!("warning: {marker}: "));
    human_event(&lock, &id, 2, "withdraw", r#"{"reason": "done by hand"}"#);
    let stderr = waiting.stays_asleep(&env);
    assert!(!stderr.contains(&id), "{stderr}");
    assert_eq!(
        files(&env, &id),
        ["0001-human-open.json", "0002-human-withdraw.json"]
    );
}

/// The human's withdraw of a message a session took wakes that session's
/// wait once, as withdrawn; another listening on the same label never sees
/// it (AG-04).
#[test]
fn a_withdraw_of_a_message_wakes_the_wait_that_took_it_once() {
    let env = Env::new();
    let id = message(&env, &["--label", "release"]);
    env.ok(&["pickup", &id, "--by", "w1"]);
    let marker = marker(&env);
    let mut other =
        Waiting::after_first_read(&env, &marker, &listening("w2", &["--label", "release"]));
    let taker = Waiting::after_first_read(&env, &marker, &listening("w1", &["--label", "release"]));
    env.ok(&["withdraw", &id, "--as", "Ryan"]);
    let lines = woke(&taker.finish());
    assert_eq!(ids(&lines), [id.as_str()]);
    assert_eq!(
        (&lines[0]["state"], &lines[0]["fresh"]),
        (&"withdrawn".into(), &true.into())
    );
    other.reads_again(&env);
    let stderr = other.stays_asleep(&env);
    assert!(!stderr.contains(&id), "{stderr}");
    // Once seen, it is not the turn again.
    let since = last_event_at(&env, &id);
    timed_out(&wait(
        &env,
        &listening(
            "w1",
            &["--label", "release", "--since", &since, "--timeout", "2s"],
        ),
    ));
}

/// A message picked up by hand with no `--by`, or a blank one, is the
/// worker's it went to, as its actor is: the human's withdraw of it wakes
/// that worker's listening wait once (AG-16).
#[test]
fn a_withdraw_of_a_message_picked_up_by_no_name_wakes_its_workers_wait() {
    let env = Env::new();
    for by in [None, Some("  ")] {
        let id = message(&env, &["--to", "bun-pins"]);
        let mut pickup = vec!["pickup", id.as_str()];
        pickup.extend(by.map(|by| ["--by", by]).into_iter().flatten());
        env.ok(&pickup);
        let actor = &json(&env.ok(&["show", "--json", &id]))["events"][1]["actor"];
        assert_eq!(actor["name"], "bun-pins", "{by:?}");
        let marker = marker(&env);
        let waiting = Waiting::after_first_read(&env, &marker, &listening("bun-pins", &[]));
        env.ok(&["withdraw", &id, "--as", "Ryan"]);
        let lines = woke(&waiting.finish());
        assert_eq!(ids(&lines), [id.as_str()], "{by:?}");
        assert_eq!(
            (&lines[0]["state"], &lines[0]["fresh"]),
            (&"withdrawn".into(), &true.into()),
            "{by:?}"
        );
    }
}

/// The workers listening now, as the store keeps them.
fn listeners(env: &Env) -> Vec<cases_store_sqlite::Listening> {
    env.db().listeners().unwrap()
}

/// Waits until the store says `worker` is listening.
fn until_listening(env: &Env, worker: &str) -> cases_store_sqlite::Listening {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(found) = listeners(env).into_iter().find(|l| l.worker == worker) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{worker} is not recorded as listening"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Two listening waits are recorded each on its own, by an id each makes,
/// with its own labels and kinds (case sgD8j). They are of two workers: a
/// second pickup wait of one worker is refused (AG-05).
#[test]
fn each_listening_wait_records_its_own_labels_and_kinds() {
    let env = Env::new();
    env.open_fyi("A case, so that the store is there");
    let mut labelled = Waiting::start(&env, &listening("bun-pins", &["--label", "release"]));
    let mut kinds = Waiting::start(
        &env,
        &listening("bun-tests", &["--kind", "message", "--kind", "decision"]),
    );
    let deadline = Instant::now() + PATIENCE;
    let waits = loop {
        let waits = env.db().waits().unwrap();
        if waits.len() == 2 {
            break waits;
        }
        assert!(
            Instant::now() < deadline,
            "the waits are not recorded: {waits:?}"
        );
        thread::sleep(Duration::from_millis(50));
    };
    assert_ne!(waits[0].id, waits[1].id, "{waits:?}");
    let mut recorded: Vec<(Vec<String>, Vec<String>)> =
        waits.into_iter().map(|w| (w.labels, w.kinds)).collect();
    recorded.sort();
    let names = |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_owned()).collect() };
    assert_eq!(
        recorded,
        [
            (names(&[]), names(&["decision", "message"])),
            (names(&["release"]), names(&[])),
        ]
    );
    for waiting in [&mut labelled, &mut kinds] {
        waiting.child.kill().unwrap();
        waiting.child.wait().unwrap();
    }
}

/// A listening wait records its worker, labels and doing line while it
/// runs; `cases doing` changes the line. Killed, it leaves its record, which
/// lapses 45 s after its last heartbeat and is then deleted by the next
/// listening write (ST-16).
#[test]
fn a_listening_wait_keeps_a_record_that_lapses_when_it_dies() {
    let env = Env::new();
    env.open_fyi("A case, so that the store is there");
    let mut waiting = Waiting::start(
        &env,
        &listening(
            "bun-pins",
            &["--label", "release", "--doing", "rebasing onto main"],
        ),
    );
    let found = until_listening(&env, "bun-pins");
    assert_eq!(found.labels, ["release"]);
    assert_eq!(found.doing.as_deref(), Some("rebasing onto main"));
    env.ok(&["doing", "running the tests", "--worker", "bun-pins"]);
    assert_eq!(
        until_listening(&env, "bun-pins").doing.as_deref(),
        Some("running the tests")
    );

    waiting.child.kill().unwrap();
    waiting.child.wait().unwrap();
    assert_eq!(
        listeners(&env).len(),
        1,
        "a killed wait's record lapses; it is not gone at once"
    );
    // As if 50 s had passed since its last heartbeat.
    let conn = Connection::open(&env.store).unwrap();
    conn.execute("UPDATE waits SET heard = ?1", [from_now(-50)])
        .unwrap();
    assert!(
        listeners(&env).is_empty(),
        "a lapsed record is still listed"
    );
    let doing = env.cases(&["doing", "x", "--worker", "bun-pins"]);
    assert_eq!(doing.code, 1, "{doing:?}");
    env.db().listen("lead-2", "lead-2", &[], &[], None).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM waits", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the lapsed record was not deleted");
}

#[test]
fn doing_needs_a_wait_that_listens() {
    let env = Env::new();
    for args in [
        vec!["--worker", "bun-pins", "--doing", "x"],
        vec![
            "--pickup", "--id", "UkLWZ", "--worker", "bun-pins", "--doing", "x",
        ],
        vec![
            "--pickup", "--worker", "bun-pins", "--kind", "fyi", "--doing", "x",
        ],
        vec!["--pickup", "--worker", "bun-pins", "--doing", "a\nb"],
    ] {
        let out = wait(&env, &args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
    }
}

/// The keys of `line`, sorted.
fn keys(line: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = line
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// A wait line is short by default: which case, where it stands, and the
/// answer, so a long body is not carried in the agent's context. `--full`
/// gives the whole case, with its body once (AG-04, case sHP8j).
#[test]
fn a_wait_line_is_short_unless_full() {
    let env = Env::new();
    let body = "A long body. ".repeat(5000);
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "Pin bun?",
        "--option",
        "Pin",
        "--option",
        "Float",
        "--body",
        &body,
    ]);
    run(
        &env,
        "answer",
        &id,
        &["--option", "2", "--note", "Float it."],
    );
    let since = ["--id", id.as_str(), "--since", id.as_str()];

    let full = wait(&env, &[&since[..], &["--full"]].concat());
    let text = full.stdout.trim();
    assert_eq!(
        text.matches("A long body.").count(),
        5000,
        "the body is given once"
    );
    let line = json(text);
    assert_eq!(line["body"], body.as_str());
    assert_eq!(line["events"][0]["body_in_case"], true);
    assert!(line["events"][0]["data"].get("body").is_none());
    assert_eq!(line["events"][0]["data"]["title"], "Pin bun?");

    let short = wait(&env, &since);
    let line = &woke(&short)[0];
    assert!(short.stdout.len() < 400, "{}", short.stdout);
    assert_eq!(
        keys(line),
        [
            "amend_seq",
            "answer",
            "fresh",
            "id",
            "kind",
            "next_since",
            "revision",
            "state",
            "title"
        ]
    );
    assert_eq!(line["id"], id.as_str());
    assert_eq!(line["state"], "answered");
    assert_eq!(line["kind"], "decision");
    assert_eq!(line["title"], "Pin bun?");
    assert_eq!(line["answer"]["choice"], 2);
    assert_eq!(line["answer"]["note"], "Float it.");
    assert_eq!(line["revision"], 2);
    assert_eq!(line["amend_seq"], 0);

    // Picked up, the line says so, with the revision close takes.
    let lines = woke(&wait(
        &env,
        &[&since[..], &["--pickup", "--by", "me"]].concat(),
    ));
    assert_eq!(lines[0]["state"], "pickedup");
    assert_eq!(lines[0]["revision"], 3);
    assert_eq!(keys(&lines[0]), keys(line));
}

/// A parked case's short line gives the park, whose note says why or until
/// when.
#[test]
fn a_short_line_gives_the_park() {
    let env = Env::new();
    let id = env.open_stuck();
    run(&env, "park", &id, &["--note", "Back on Monday."]);
    let lines = woke(&wait(&env, &["--id", &id, "--since", &id]));
    assert_eq!(
        keys(&lines[0]),
        [
            "amend_seq",
            "fresh",
            "id",
            "kind",
            "next_since",
            "park",
            "revision",
            "state",
            "title"
        ]
    );
    assert_eq!(lines[0]["state"], "parked");
    assert_eq!(lines[0]["park"]["note"], "Back on Monday.");
}

/// A message is the human's instruction, so its short line keeps all the
/// human wrote in it: its urgency, title, body, context, links and the case
/// it is about.
#[test]
fn a_short_line_gives_a_message_s_body_and_re() {
    let env = Env::new();
    let about = env.open_fyi("A case, so that the store is there");
    let id = message(
        &env,
        &[
            "--to",
            "bun-pins",
            "--body",
            "Then run the tests.",
            "--re",
            &about,
            "--context",
            "After the rebase.",
            "--link",
            "https://example.com/pr/9",
            "--urgency",
            "blocking",
        ],
    );
    let lines = woke(&wait(&env, &listening("bun-pins", &[])));
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(
        keys(line),
        [
            "amend_seq",
            "body",
            "context",
            "fresh",
            "id",
            "kind",
            "links",
            "next_since",
            "re",
            "revision",
            "state",
            "title",
            "urgency"
        ]
    );
    assert_eq!(line["kind"], "message");
    assert_eq!(line["state"], "pickedup");
    assert_eq!(line["urgency"], "blocking");
    assert_eq!(line["title"], "Rebase onto main");
    assert_eq!(line["body"], "Then run the tests.");
    assert_eq!(line["context"], "After the rebase.");
    assert_eq!(
        line["links"],
        serde_json::json!(["https://example.com/pr/9"])
    );
    assert_eq!(line["re"], about.as_str());
}

/// A withdrawn case's short line gives the withdraw, whose reason may say
/// what to do instead.
#[test]
fn a_short_line_gives_the_withdraw() {
    let env = Env::new();
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    run(
        &env,
        "withdraw",
        &id,
        &[
            "--as",
            "Ryan",
            "--reason",
            "Superseded: do case:abcde instead.",
        ],
    );
    let lines = woke(&wait(&env, &["--id", &id, "--since", &id]));
    assert_eq!(
        keys(&lines[0]),
        [
            "amend_seq",
            "answer",
            "fresh",
            "id",
            "kind",
            "next_since",
            "revision",
            "state",
            "title",
            "withdraw"
        ]
    );
    assert_eq!(lines[0]["state"], "withdrawn");
    assert_eq!(
        lines[0]["withdraw"]["reason"],
        "Superseded: do case:abcde instead."
    );
}

/// Opens an fyi case with `label` and `urgency`, and gives its id.
fn open_labelled(env: &Env, title: &str, label: &str, urgency: &str) -> String {
    env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        urgency,
        "--title",
        title,
        "--label",
        label,
    ])
}

#[test]
fn a_steady_wait_prints_answers_given_together_once() {
    let env = Env::new();
    let first = open_labelled(&env, "First", "burst", "today");
    let second = open_labelled(&env, "Second", "burst", "today");
    let _left = open_labelled(&env, "Left open", "burst", "today");
    let marker = marker(&env);

    // A case is still open, so it collects until its timeout, and then
    // prints both answers and exits 0: it has something to report (AG-18).
    let args = [
        "--label",
        "burst",
        "--mood",
        "steady",
        "--pickup",
        "--by",
        "w1",
        "--timeout",
        "4s",
    ];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &first, &["--ack"]);
    thread::sleep(Duration::from_millis(1500));
    run(&env, "answer", &second, &["--ack"]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [first.as_str(), second.as_str()]);
    for line in &lines {
        assert_eq!(line["state"], "pickedup", "{line}");
        assert_eq!(line["fresh"], true, "{line}");
    }

    // The next_since covers both: a wait with it does not print them again.
    let next = lines[1]["next_since"].as_str().unwrap();
    timed_out(&wait(
        &env,
        &["--label", "burst", "--since", next, "--timeout", "1s"],
    ));
}

/// A steady wait with a case it waits on still open prints once its quiet
/// time passes with no event new to it, well before its timeout (AG-18).
#[test]
fn a_steady_wait_prints_at_the_end_of_its_quiet_time() {
    let env = Env::new();
    let answered = open_labelled(&env, "Answered", "quiet", "today");
    let _left = open_labelled(&env, "Left open", "quiet", "today");
    let marker = marker(&env);
    let args = ["--label", "quiet", "--mood", "steady", "--timeout", "40s"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    let start = Instant::now();
    run(&env, "answer", &answered, &["--ack"]);
    let lines = woke(&waiting.finish());
    let took = start.elapsed();
    assert_eq!(ids(&lines), [answered.as_str()]);
    // Its quiet time is 10 s, from the read that found the answer.
    assert!(
        took >= Duration::from_secs(9) && took < Duration::from_secs(16),
        "{took:?}"
    );
}

#[test]
fn a_steady_wait_prints_at_once_when_none_it_waits_on_is_open() {
    let env = Env::new();
    let first = open_labelled(&env, "First", "all-back", "today");
    let second = open_labelled(&env, "Second", "all-back", "today");
    let marker = marker(&env);

    // Both answers are in: nothing more can come, so it does not sit out its
    // quiet time.
    let args = [
        "--label",
        "all-back",
        "--mood",
        "patient",
        "--timeout",
        "60s",
    ];
    let start = Instant::now();
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &first, &["--ack"]);
    run(&env, "answer", &second, &["--ack"]);
    let lines = woke(&waiting.finish());
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(ids(&lines).len(), 2, "{lines:?}");

    // A wait on one case never lingers.
    let one = env.open_decision();
    let args = ["--id", &one, "--mood", "patient", "--timeout", "60s"];
    let start = Instant::now();
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &one, &["--option", "1"]);
    assert_eq!(ids(&woke(&waiting.finish())), [one.as_str()]);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_blocking_answer_or_a_withdraw_ends_a_steady_wait() {
    let env = Env::new();
    let blocking = open_labelled(&env, "Blocking", "urgent", "blocking");
    let _left = open_labelled(&env, "Left open", "urgent", "today");
    let swept = open_labelled(&env, "Swept", "urgent", "today");
    let marker = marker(&env);

    let args = ["--label", "urgent", "--mood", "patient", "--timeout", "60s"];
    let start = Instant::now();
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &blocking, &["--ack"]);
    assert_eq!(ids(&woke(&waiting.finish())), [blocking.as_str()]);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );

    let start = Instant::now();
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    env.ok(&["withdraw", &swept, "--as", "Ryan", "--reason", "not needed"]);
    let lines = woke(&waiting.finish());
    assert!(ids(&lines).contains(&swept.as_str()), "{lines:?}");
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn the_config_file_gives_the_mood_and_a_bad_one_is_refused() {
    let env = Env::new();
    let first = open_labelled(&env, "First", "cfg", "today");
    let _left = open_labelled(&env, "Left open", "cfg", "today");
    let marker = marker(&env);

    // Steady from the config: it collects until its timeout, and exits 0.
    env.config("mood = \"steady\"\n");
    let args = ["--label", "cfg", "--timeout", "3s"];
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &first, &["--ack"]);
    assert_eq!(ids(&woke(&waiting.finish())), [first.as_str()]);

    env.config("mood = \"sleepy\"\n");
    let out = wait(&env, &["--label", "cfg", "--timeout", "1s"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("mood"), "{out:?}");

    // The flag wins over the file.
    let out = wait(
        &env,
        &["--label", "cfg", "--mood", "eager", "--timeout", "1s"],
    );
    assert_eq!(out.code, 2, "{out:?}");
    let out = wait(&env, &["--mood", "sleepy"]);
    assert_eq!(out.code, 1, "{out:?}");
}

#[test]
fn a_message_taken_while_a_steady_wait_collects_is_in_the_same_print() {
    let env = Env::new();
    let open = |title: &str| {
        env.open(&[
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            title,
            "--worker",
            "w9",
        ])
    };
    let answered = open("Answered");
    let _left = open("Left open");
    let marker = marker(&env);

    let args = listening("w9", &["--mood", "steady", "--timeout", "4s"]);
    let waiting = Waiting::after_first_read(&env, &marker, &args);
    run(&env, "answer", &answered, &["--ack"]);
    thread::sleep(Duration::from_millis(1500));
    let sent = message(&env, &["--to", "w9", "--body", "Then run the tests."]);
    let lines = woke(&waiting.finish());
    assert_eq!(ids(&lines), [answered.as_str(), sent.as_str()]);
    for line in &lines {
        assert_eq!(line["state"], "pickedup", "{line}");
    }
}

// Pins of how a wait prints what it found (`print` in `commands/wait.rs`):
// each line exactly, its order, and how a wait ends when a write or a read
// fails part-way.

/// The time of the event `seq` of the case `id`, as a line gives it.
fn event_at(env: &Env, id: &str, seq: usize) -> String {
    env.case(id).events()[seq - 1].at().unwrap().to_string()
}

/// The id of the actor of the event `seq` of the case `id`.
fn actor_of(env: &Env, id: &str, seq: usize) -> String {
    env.case(id).events()[seq - 1].actor().unwrap().id.clone()
}

/// The short line of a decision answered with option `choice` at event 2.
fn answered_line(env: &Env, id: &str, state: &str, revision: u64, choice: u64) -> String {
    format!(
        r#"{{"id":"{id}","state":"{state}","kind":"decision","title":"Pin bun?","answer":{{"choice":{choice},"actor":{{"id":"{}"}},"answered_at":"{}"}},"revision":{revision},"amend_seq":0"#,
        actor_of(env, id, 2),
        event_at(env, id, 2),
    )
}

/// The end of every line of a wait: `fresh`, then `next_since`.
fn tail(fresh: bool, next_since: &str) -> String {
    format!(r#","fresh":{fresh},"next_since":"{next_since}"}}"#)
}

/// A wait without `--pickup` prints each case it found as one short line,
/// in the order the events that made them the agent's turn came, not the
/// order the cases were opened: a park, an answer, a resume, then the
/// answer that woke it. Only the one that landed while it waited is fresh,
/// and every line gives the time of that last event as `next_since`. A
/// withdraw from before the wait started is not printed, even with other
/// cases.
#[test]
fn a_wait_prints_one_exact_short_line_for_each_case_in_the_order_of_its_turn() {
    let env = Env::new();
    let answered = env.open_decision();
    let parked = env.open_stuck();
    let resumed = env.open_stuck();
    let withdrawn = env.open_decision();
    let woke_it = env.open_fyi("Seen?");
    run(&env, "park", &parked, &["--note", "Monday."]);
    run(
        &env,
        "answer",
        &answered,
        &["--option", "1", "--as", "Ryan"],
    );
    run(&env, "park", &resumed, &[]);
    run(&env, "resume", &resumed, &[]);
    run(&env, "answer", &withdrawn, &["--option", "2"]);
    run(
        &env,
        "withdraw",
        &withdrawn,
        &["--as", "Ryan", "--reason", "Gone."],
    );
    let marker = marker(&env);

    let waiting = Waiting::after_first_read(&env, &marker, &[]);
    run(&env, "answer", &woke_it, &["--ack"]);
    let out = waiting.finish();

    let next = event_at(&env, &woke_it, 2);
    let expected = [
        format!(
            r#"{{"id":"{parked}","state":"parked","kind":"stuck","title":"Blocked on mirror","park":{{"note":"Monday.","actor":{{"id":"{}"}},"parked_at":"{}"}},"revision":2,"amend_seq":0{}"#,
            actor_of(&env, &parked, 2),
            event_at(&env, &parked, 2),
            tail(false, &next),
        ),
        answered_line(&env, &answered, "answered", 2, 1) + &tail(false, &next),
        format!(
            r#"{{"id":"{resumed}","state":"open","kind":"stuck","title":"Blocked on mirror","revision":3,"amend_seq":0{}"#,
            tail(false, &next),
        ),
        format!(
            r#"{{"id":"{woke_it}","state":"answered","kind":"fyi","title":"Seen?","answer":{{"ack":true,"actor":{{"id":"{}"}},"answered_at":"{next}"}},"revision":2,"amend_seq":0{}"#,
            actor_of(&env, &woke_it, 2),
            tail(true, &next),
        ),
    ];
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.lines().collect::<Vec<_>>(), expected);
    assert_eq!(without(&out.stderr, &marker), "");
}

/// A `--pickup` wait picks up each answered case it prints, and only those:
/// the line of one it picked up is the case after the pickup, `pickedup` at
/// the next revision, with nothing more in it. A parked or withdrawn case
/// is printed as it is and left alone. With `--since` an event already in
/// the store is fresh, and `next_since` is the latest event printed, here
/// the withdraw, not the pickups the wait wrote.
#[test]
fn a_pickup_wait_prints_one_exact_short_line_for_each_case_and_picks_up_the_answers() {
    let env = Env::new();
    let first = env.open_decision();
    let parked = env.open_stuck();
    let withdrawn = env.open_decision();
    let second = env.open_decision();
    run(&env, "answer", &first, &["--option", "1", "--as", "Ryan"]);
    run(&env, "park", &parked, &["--note", "Monday."]);
    run(&env, "answer", &second, &["--option", "2"]);
    run(&env, "answer", &withdrawn, &["--option", "2"]);
    run(
        &env,
        "withdraw",
        &withdrawn,
        &["--as", "Ryan", "--reason", "Gone."],
    );

    let ids = [&first, &parked, &withdrawn, &second];
    let mut args = vec!["--since", first.as_str(), "--pickup", "--by", "me"];
    for id in ids {
        args.extend(["--id", id.as_str()]);
    }
    let out = wait(&env, &args);

    let next = event_at(&env, &withdrawn, 3);
    let expected = [
        answered_line(&env, &first, "pickedup", 3, 1) + &tail(true, &next),
        format!(
            r#"{{"id":"{parked}","state":"parked","kind":"stuck","title":"Blocked on mirror","park":{{"note":"Monday.","actor":{{"id":"{}"}},"parked_at":"{}"}},"revision":2,"amend_seq":0{}"#,
            actor_of(&env, &parked, 2),
            event_at(&env, &parked, 2),
            tail(true, &next),
        ),
        answered_line(&env, &second, "pickedup", 3, 2) + &tail(true, &next),
        answered_line(&env, &withdrawn, "withdrawn", 3, 2).replace(
            r#","revision":3"#,
            &format!(
                r#","withdraw":{{"reason":"Gone.","actor":{{"id":"{}"}},"withdrawn_at":"{next}"}},"revision":3"#,
                actor_of(&env, &withdrawn, 3),
            ),
        ) + &tail(true, &next),
    ];
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.lines().collect::<Vec<_>>(), expected);
    assert_eq!(out.stderr, "");
    for id in [&first, &second] {
        assert_eq!(files(&env, id).last().unwrap(), "0003-agent-pickup.json");
    }
    assert_eq!(env.revision(&parked), 2);
    assert_eq!(env.revision(&withdrawn), 3);
}

/// `--full` gives each case as `show --json` does, with `fresh` and
/// `next_since` after it, and names each event's actor from the store's
/// table: by kind and name, by kind alone for an unnamed actor, and by id
/// alone for one the table does not have, such as a deleted one. The short
/// line names no actor but by id.
#[test]
fn a_full_line_names_actors_by_name_and_an_unknown_one_by_id() {
    let env = Env::new();
    let named = env.open_decision();
    let deleted = env.open_decision();
    run(&env, "answer", &named, &["--option", "1", "--as", "Ryan"]);
    run(&env, "answer", &deleted, &["--option", "2", "--as", "Gone"]);
    let conn = Connection::open(&env.store).unwrap();
    conn.execute("DELETE FROM actors WHERE name = 'Gone'", [])
        .unwrap();

    let args = [
        "--full", "--id", &named, "--id", &deleted, "--since", &named,
    ];
    let out = wait(&env, &args);
    let lines = woke(&out);
    assert_eq!(ids(&lines), [named.as_str(), deleted.as_str()]);
    assert_eq!(out.stderr, "");
    let agent = actor_of(&env, &named, 1);
    assert_eq!(
        lines[0]["events"][0]["actor"],
        serde_json::json!({"id": agent, "kind": "agent"})
    );
    assert_eq!(
        lines[0]["events"][1]["actor"],
        serde_json::json!({"id": actor_of(&env, &named, 2), "kind": "human", "name": "Ryan"})
    );
    assert_eq!(
        lines[1]["events"][1]["actor"],
        serde_json::json!({"id": actor_of(&env, &deleted, 2)})
    );
    // What --full adds to the short line: everything show --json gives
    // but its url, then fresh and next_since last.
    let next = event_at(&env, &deleted, 2);
    for text in out.stdout.lines() {
        assert!(text.ends_with(&tail(true, &next)), "{text}");
    }
    let mut line = lines[1].clone();
    let fields = line.as_object_mut().unwrap();
    fields.remove("fresh");
    fields.remove("next_since");
    let mut shown = json(&env.ok(&["show", &deleted, "--json"]));
    shown.as_object_mut().unwrap().remove("url");
    assert_eq!(line, shown);
}

/// A pickup the store refuses leaves the case as it was read: the line is
/// the answered case, at the revision read, and one warning says why. The
/// wait ends as it would have, with exit 0.
#[test]
fn a_pickup_the_store_refuses_prints_the_case_as_read_and_says_why() {
    let env = Env::new();
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    let conn = Connection::open(&env.store).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON events
         BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;",
    )
    .unwrap();

    let out = wait(
        &env,
        &["--id", &id, "--since", &id, "--pickup", "--by", "me"],
    );
    assert_eq!(out.code, 0, "{out:?}");
    let next = event_at(&env, &id, 2);
    assert_eq!(
        out.stdout,
        answered_line(&env, &id, "answered", 2, 1) + &tail(true, &next) + "\n"
    );
    assert_eq!(
        out.stderr,
        format!("warning: {id}: not picked up: refused by the test\n")
    );
    assert_eq!(files(&env, &id).len(), 2);
}

/// Swaps the store's table of actors for a view of it, which a write still
/// finds and adds actors through, but which reading the whole table, by
/// rowid, fails on: as a store that cannot give the actors when a wait
/// prints.
fn spoil_actors(env: &Env) {
    let conn = Connection::open(&env.store).unwrap();
    conn.execute_batch(
        "ALTER TABLE actors RENAME TO actors_kept;
         CREATE VIEW actors AS SELECT id, kind, name, created FROM actors_kept;
         CREATE TRIGGER actors_insert INSTEAD OF INSERT ON actors BEGIN
             INSERT INTO actors_kept VALUES (NEW.id, NEW.kind, NEW.name, NEW.created);
         END;",
    )
    .unwrap();
}

/// A `--full` wait reads the actors before it picks anything up, so one
/// whose read fails ends there: exit 1, nothing on stdout, the error on
/// stderr, and nothing picked up.
///
/// Open follow-up from #82: a steady `--full` wait whose store goes out of
/// reach when it gives what it collected ends this way too, with nothing
/// printed. This pins it as it is today.
#[test]
fn a_full_wait_whose_actors_cannot_be_read_prints_nothing_and_picks_up_nothing() {
    let env = Env::new();
    let id = env.open_decision();
    run(&env, "answer", &id, &["--option", "1"]);
    spoil_actors(&env);

    let args = [
        "--full", "--id", &id, "--since", &id, "--pickup", "--by", "me",
    ];
    let out = wait(&env, &args);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.starts_with("Error: no such column: rowid"),
        "{}",
        out.stderr
    );
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
    let case = env.case(&id);
    assert_eq!((case.state().as_str(), case.revision()), ("answered", 2));

    // The short line needs no actors, so the same wait without --full
    // prints and picks up.
    let out = wait(&env, &args[1..]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        answered_line(&env, &id, "pickedup", 3, 1) + &tail(true, &event_at(&env, &id, 2)) + "\n"
    );
}

/// A message a listening `--full` wait took is picked up already, so a
/// read of the actors that fails does not lose it: it is printed with each
/// actor named by id alone, after one warning, and the wait ends with exit
/// 0. Its `next_since` is the message's open, not the pickup the wait wrote.
#[test]
fn a_full_wait_that_took_a_message_prints_it_by_actor_id_when_the_actors_cannot_be_read() {
    let env = Env::new();
    let id = message(&env, &["--to", "w"]);
    spoil_actors(&env);

    let out = wait(&env, &["--pickup", "--worker", "w", "--full"]);
    assert_eq!(out.code, 0, "{out:?}");
    let warning =
        "warning: the actors cannot be read, so they are named by id: no such column: rowid";
    assert!(out.stderr.starts_with(warning), "{}", out.stderr);
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
    let lines = woke(&out);
    assert_eq!(ids(&lines), [id.as_str()]);
    let line = &lines[0];
    assert_eq!(line["state"], "pickedup");
    assert_eq!(
        line["actor"],
        serde_json::json!({"id": actor_of(&env, &id, 1)})
    );
    for (i, event) in line["events"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            event["actor"],
            serde_json::json!({"id": actor_of(&env, &id, i + 1)}),
            "{event}"
        );
    }
    assert_eq!(line["fresh"], true);
    assert_eq!(line["next_since"], event_at(&env, &id, 1).as_str());
    assert_eq!(env.case(&id).state().as_str(), "pickedup");
}

/// How many times a `--full` pickup wait that picks up three cases reads
/// the store's actors: once before any pickup, then again after each, so
/// that an actor a pickup made is named. It is counted on a forwarded
/// store, whose every call is a request on the pipe. The count is pinned
/// so that reading them less (finding B2) changes it on purpose.
#[cfg(unix)]
#[test]
fn a_full_pickup_wait_of_three_cases_reads_the_actors_four_times() {
    let mut env = Env::new();
    let cases = [
        env.open_decision(),
        env.open_decision(),
        env.open_decision(),
    ];
    for id in &cases {
        run(&env, "answer", id, &["--option", "1"]);
    }
    let log = env.path("requests.log");
    // The far side writes its profile, under `cargo llvm-cov`, in the
    // test's own directory: the near side kills it 200 ms after closing
    // its pipe, which can cut a profile short, and one cut short in the
    // run's own directory fails the whole report.
    let forward = format!(
        "exec:tee -a '{}' | LLVM_PROFILE_FILE='{}' CASES_STORE='{}' exec '{}' dial-stdio",
        log.display(),
        env.path("far-%p.profraw").display(),
        env.store.display(),
        env!("CARGO_BIN_EXE_cases"),
    );
    env.set("PATH", "/usr/bin:/bin");
    env.set("CASES_FORWARD", &forward);

    let mut args = vec![
        "wait",
        "--timeout",
        "10s",
        "--full",
        "--since",
        &cases[0],
        "--pickup",
        "--by",
        "me",
    ];
    for id in &cases {
        args.extend(["--id", id.as_str()]);
    }
    let out: Output = common::output(
        env.command(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .into();
    let lines = woke(&out);
    assert_eq!(
        ids(&lines),
        cases.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(
        lines.iter().all(|line| line["state"] == "pickedup"),
        "{out:?}"
    );
    let requests = fs::read_to_string(&log).unwrap();
    // A request's body ends with no line break, so the next request's
    // line can follow it on the same line of the log.
    let actors = requests
        .matches("GET /v1/stores/st_dialstdio0000000/store/actors HTTP/1.1")
        .count();
    assert_eq!(actors, 4, "{requests}");
}
