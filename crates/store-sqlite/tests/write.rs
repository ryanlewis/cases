//! Writes: opening cases, appending events, and writers taking turns.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::process::{Command, Stdio};
use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Case, Category, Row, State, Store, StoreError, Time, Writer};
use cases_store_sqlite::{Db, Error, Key};
use common::{
    agent_event, answer_of, create, fixed, human_event, insert_row, names, open_of, scratch, sql,
    stored, unnamed,
};
use rusqlite::TransactionBehavior;
use serde_json::{Value, json};

/// The case the store has for `key`, folded.
fn get(db: &Db, key: Key) -> Case {
    let rows = db.rows(&key).unwrap().expect("the store has the case");
    Case::fold(&rows).unwrap()
}

/// The category a refused write gives.
fn refused<T: std::fmt::Debug>(outcome: Result<T, StoreError<Error>>) -> Category {
    match outcome {
        Err(StoreError::Refused(error)) => error.category(),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn create_writes_the_open_event() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-15T09:12:03Z");
    let open = agent_event(
        "open",
        r#"{"kind": "fyi", "urgency": "whenever", "title": "Pin bun or float?"}"#,
    );
    let (key, case) = db.create(&open, &unnamed()).unwrap();
    assert_eq!(key.to_string(), "UkLWZ");
    assert_eq!(case.revision(), 1);
    assert_eq!(case.events()[0].file(), "0001-agent-open.json");
    assert_eq!(names(&stored(&db, key)), ["0001-agent-open.json"]);
    let loaded = get(&db, key);
    assert_eq!(loaded, case);
    assert_eq!(
        (loaded.state(), loaded.title()),
        (State::Open, "Pin bun or float?")
    );
    assert_eq!(loaded.opened_at(), Some(common::at("2026-09-15T09:12:03Z")));
    let opened_at: String = sql(&db)
        .query_row(
            "SELECT opened_at FROM cases WHERE key = ?1",
            [common::key_sql(key)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(opened_at, "2026-09-15T09:12:03Z");

    // The same title in the same second is another case, with the next key
    // and its own id: ids never clash (ST-10).
    let (again, _) = db.create(&open, &unnamed()).unwrap();
    assert_eq!(again.get(), 2);
    assert_eq!(again.to_string(), "gbHJd");
}

#[test]
fn create_writes_labels() {
    let (_dir, db) = scratch();
    let open = agent_event(
        "open",
        r#"{"kind": "fyi", "urgency": "today", "title": "T", "labels": ["feat-labels", "round 3"]}"#,
    );
    let (key, _) = db.create(&open, &unnamed()).unwrap();
    let data: Value = serde_json::from_str(&stored(&db, key)[0].data).unwrap();
    assert_eq!(data["labels"], json!(["feat-labels", "round 3"]));
    assert_eq!(get(&db, key).labels(), ["feat-labels", "round 3"]);
}

/// An open stored without labels, such as one a hand edit wrote, folds with
/// none.
#[test]
fn a_stored_open_without_labels_folds() {
    let (_dir, db) = scratch();
    create(&db, "fyi");
    insert_row(
        &db,
        900,
        1,
        "agent",
        "open",
        r#"{"kind":"fyi","urgency":"whenever","title":"Old case","worker":"w1","brief":"BRIEF.md","opened_at":"2026-09-01T10:00:00Z"}"#,
    );
    let case = get(&db, Key::new(900).unwrap());
    assert_eq!(case.state(), State::Open);
    assert!(case.labels().is_empty());
    assert_eq!(case.worker(), "w1");
    assert!(case.problems().is_empty());
}

#[test]
fn a_refused_open_does_not_make_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new").join("cases.db");
    let db = Db::new(&path);
    let no_options = agent_event(
        "open",
        r#"{"kind": "decision", "urgency": "today", "title": "No options"}"#,
    );
    assert_eq!(
        refused(db.create(&no_options, &unnamed())),
        Category::Invalid
    );
    let by_the_human = human_event(
        "open",
        r#"{"kind": "fyi", "urgency": "today", "title": "T"}"#,
    );
    assert_eq!(
        refused(db.create(&by_the_human, &unnamed())),
        Category::Author
    );
    assert!(
        !path.parent().unwrap().exists(),
        "a refused open made the store"
    );
}

#[test]
fn appends_number_events_in_order() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "decision");
    for event in [
        human_event("answer", r#"{"choice": 1}"#),
        agent_event("pickup", r#"{"by": "mgr"}"#),
        agent_event("note", r#"{"body": "and?", "reopen": true}"#),
        human_event("answer", r#"{"other": true, "note": "neither"}"#),
        agent_event("pickup", "{}"),
        agent_event(
            "close",
            r#"{"outcome": "went with neither", "links": ["https://x"]}"#,
        ),
    ] {
        db.append(&key, &event, &unnamed(), None).unwrap();
    }
    assert_eq!(
        names(&stored(&db, key)),
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-agent-pickup.json",
            "0004-agent-note.json",
            "0005-human-answer.json",
            "0006-agent-pickup.json",
            "0007-agent-close.json",
        ]
    );
    let loaded = get(&db, key);
    assert_eq!(loaded.state(), State::Closed);
    assert!(loaded.answer().unwrap().other);
    assert_eq!(loaded.close().unwrap().outcome, "went with neither");
}

#[test]
fn a_refused_event_is_not_written() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "decision");
    for (name, event, category) in [
        (
            "pickup an open case",
            agent_event("pickup", "{}"),
            Category::Transition,
        ),
        (
            "close an open case",
            agent_event("close", r#"{"outcome": "x"}"#),
            Category::Transition,
        ),
        (
            "park a decision",
            human_event("park", "{}"),
            Category::Transition,
        ),
        (
            "resume an open case",
            human_event("resume", "{}"),
            Category::Transition,
        ),
        (
            "answer with a bad choice",
            human_event("answer", r#"{"choice": 9}"#),
            Category::Invalid,
        ),
        (
            "resume by an unknown author",
            common::append("robot", "resume", "{}"),
            Category::Author,
        ),
        (
            "amend with nothing",
            agent_event("amend", "{}"),
            Category::Invalid,
        ),
        (
            "amend a decision with rows",
            agent_event(
                "amend",
                r#"{"rows": [{"id": "a", "label": "Install deps", "script": "npm ci", "link": "https://example.com/a"}]}"#,
            ),
            Category::Invalid,
        ),
    ] {
        assert_eq!(
            refused(db.append(&key, &event, &unnamed(), None)),
            category,
            "{name}"
        );
    }
    assert_eq!(names(&stored(&db, key)), ["0001-agent-open.json"]);
}

#[test]
fn an_amend_adds_to_the_case() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-16T11:00:00Z");
    let (key, _) = create(&db, "decision");
    let opened = stored(&db, key)[0].data.clone();
    let amend = agent_event(
        "amend",
        r#"{"options": ["Vendor it"], "links": ["https://example.com/log"]}"#,
    );
    let amended = db.append(&key, &amend, &unnamed(), None).unwrap();
    assert_eq!(amended.state(), State::Open);
    assert_eq!(amended.options(), ["Pin", "Float", "Vendor it"]);
    let rows = stored(&db, key);
    assert_eq!(
        names(&rows),
        ["0001-agent-open.json", "0002-agent-amend.json"]
    );
    assert_eq!(rows[0].data, opened, "the open row changed");
    // Only the fields the amend sets are written, with its time and the
    // writer's actor.
    let record: Value = serde_json::from_str(&rows[1].data).unwrap();
    let fields: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(fields, ["actor", "amended_at", "links", "options"]);
    assert_eq!(record["amended_at"], "2026-09-16T11:00:00Z");
    let loaded = get(&db, key);
    assert_eq!((loaded.options().len(), loaded.links().len()), (3, 1));
    assert!(loaded.problems().is_empty());
}

#[test]
fn appending_to_a_missing_case() {
    let (_dir, db) = scratch();
    let missing = Key::new(900).unwrap();
    let pickup = agent_event("pickup", "{}");
    match db.append(&missing, &pickup, &unnamed(), None) {
        Err(StoreError::Store(error @ Error::Missing { .. })) => {
            assert!(error.to_string().starts_with("no store at "), "{error}");
        }
        other => panic!("no store: {other:?}"),
    }
    create(&db, "fyi");
    match db.append(&missing, &pickup, &unnamed(), None) {
        Err(StoreError::NoCase) => {}
        other => panic!("no case: {other:?}"),
    }
}

/// A case whose last row has the highest seq SQLite holds has no seq left
/// for another: the append is refused as damage, and stores nothing.
#[test]
fn a_case_with_no_seq_left_for_another_row_is_damaged() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    sql(&db)
        .execute(
            "UPDATE events SET seq = 9223372036854775807 WHERE case_key = ?1",
            [common::key_sql(key)],
        )
        .unwrap();
    let before = stored(&db, key);
    let note = agent_event("note", r#"{"body": "More."}"#);
    let outcome = db.append(&key, &note, &unnamed(), None);
    assert!(
        matches!(outcome, Err(StoreError::Store(Error::Damaged { .. }))),
        "{outcome:?}"
    );
    assert_eq!(stored(&db, key), before);
}

/// The insert's encoding and the read's decoding agree: `data` goes in as
/// the bytes of the record, in a BLOB, `author` and `event` as text, and each
/// comes back as it was held, with non-ASCII and escaped characters in the
/// record.
#[test]
fn a_row_reads_back_as_it_was_written() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let note = agent_event("note", r#"{"body": "café 🦀 \"quoted\" \\ ﷽"}"#);
    let rows = db.append_rows(&key, &note, &unnamed(), None).unwrap();
    assert_eq!(rows, db.rows(&key).unwrap().unwrap());
    let written = rows.last().unwrap();
    assert!(
        written.data.contains(r#""body":"café 🦀 \"quoted\" \\ ﷽""#),
        "{}",
        written.data
    );
    let stored: (String, String, String, Vec<u8>) = sql(&db)
        .query_row(
            "SELECT typeof(data), author, event, data FROM events
             WHERE case_key = ?1 AND seq = 2",
            [common::key_sql(key)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "blob".to_owned(),
            written.author.clone(),
            written.event.clone(),
            written.data.clone().into_bytes(),
        )
    );
}

/// Checks that the case's rows are numbered 1 to `n` with no gap and none
/// twice, and that they all fold.
fn check_sequence(db: &Db, key: Key, n: usize) {
    let rows = stored(db, key);
    let seqs: Vec<i64> = rows.iter().map(|row| row.seq).collect();
    let want: Vec<i64> = (1..).take(n).collect();
    assert_eq!(seqs, want);
    let case = get(db, key);
    assert_eq!(case.events().len(), n);
    assert_eq!(case.revision(), n as u64);
    assert!(case.problems().is_empty(), "{:?}", case.problems());
}

#[test]
fn concurrent_appends_take_distinct_seqs() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let n = 20;
    let start = Barrier::new(n);
    thread::scope(|scope| {
        for _ in 0..n {
            scope.spawn(|| {
                start.wait();
                let note = agent_event("note", r#"{"body": "more"}"#);
                db.append(&key, &note, &unnamed(), None).unwrap();
            });
        }
    });
    check_sequence(&db, key, n + 1);
}

/// Two writers with their own connections to one file, as two commands on
/// one store are, take turns: each reads the case the other left, so the
/// rows are numbered 1 to n with no gap and none twice, and no write fails.
#[test]
fn two_writers_on_one_file_take_turns() {
    let (_dir, first) = scratch();
    let second = Db::new(first.path());
    let (key, _) = create(&first, "stuck");
    let n = 25;
    let start = Barrier::new(2);
    thread::scope(|scope| {
        for (i, db) in [&first, &second].into_iter().enumerate() {
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for j in 0..n {
                    let note = format!(r#"{{"body": "writer {i}, note {j}"}}"#);
                    db.append(&key, &agent_event("note", &note), &unnamed(), None)
                        .unwrap();
                }
            });
        }
    });
    check_sequence(&first, key, 2 * n + 1);
}

/// Hands `writer_process` its store, case, count and start file.
const WRITER_ENV: &str = "CASES_STORE_TEST_WRITER";

/// The same race between two processes, which is what the file's locks are
/// for: each process has its own connection and its own view of the locks.
#[test]
fn two_processes_on_one_file_take_turns() {
    let (dir, db) = scratch();
    let (key, _) = create(&db, "stuck");
    let n = 20;
    let start = dir.path().join("start");
    let spec = format!("{}\n{key}\n{n}\n{}", db.path().display(), start.display());
    let children: Vec<_> = (0..2)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args(["writer_process", "--exact", "--nocapture"])
                .env(WRITER_ENV, &spec)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    // Both processes wait for this file, so their writes overlap.
    std::fs::write(&start, "").unwrap();
    for child in children {
        let out = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "the writer failed or did not run:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    check_sequence(&db, key, 2 * n + 1);
}

/// One writer for `two_processes_on_one_file_take_turns`, run in a process
/// of its own. Run by any other means, it does nothing.
#[test]
fn writer_process() {
    let Ok(spec) = std::env::var(WRITER_ENV) else {
        return;
    };
    let parts: Vec<&str> = spec.split('\n').collect();
    let [path, key, n, start] = parts[..] else {
        panic!("the spec is {spec:?}");
    };
    let key: Key = key.parse().unwrap();
    let n: usize = n.parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::path::Path::new(start).exists() {
        assert!(Instant::now() < deadline, "never told to start");
        thread::sleep(Duration::from_millis(1));
    }
    let db = Db::new(path);
    for i in 0..n {
        let note = format!(r#"{{"body": "process {}, note {i}"}}"#, std::process::id());
        db.append(&key, &agent_event("note", &note), &unnamed(), None)
            .unwrap();
    }
}

#[test]
fn a_write_at_a_revision_is_refused_once_the_case_moves_on() {
    let (_dir, db) = scratch();
    let (key, case) = create(&db, "stuck");
    let read = case.revision();
    // Answered and reopened by a note since it was read: the case is open
    // again, as it was then.
    db.append(&key, &answer_of("stuck"), &unnamed(), None)
        .unwrap();
    db.append(
        &key,
        &agent_event("note", r#"{"body": "Which mirror?", "reopen": true}"#),
        &unnamed(),
        None,
    )
    .unwrap();
    let drop = human_event("answer", r#"{"drop": true}"#);
    let park = human_event("park", "{}");
    for (name, event, at) in [
        ("answer at the revision read", &drop, read),
        ("park at the revision read", &park, read),
        ("park ahead of the case", &park, 4),
        ("answer at revision 0", &drop, 0),
    ] {
        let outcome = db.append(&key, event, &unnamed(), Some(at));
        assert_eq!(refused(outcome), Category::Stale, "{name}");
    }
    assert_eq!(stored(&db, key).len(), 3);

    // At the case's revision the write goes through and raises it.
    let parked = db.append(&key, &park, &unnamed(), Some(3)).unwrap();
    assert_eq!(parked.revision(), 4);
    // Without a revision a write is unconditional.
    let resume = human_event("resume", "{}");
    db.append(&key, &resume, &unnamed(), None).unwrap();
    // Resumed since revision 4, the case is open: a resume at 4 is refused as
    // stale rather than as a resume of an open case.
    assert_eq!(
        refused(db.append(&key, &resume, &unnamed(), Some(4))),
        Category::Stale
    );
    db.append(&key, &park, &unnamed(), Some(5)).unwrap();
    db.append(&key, &resume, &unnamed(), Some(6)).unwrap();
    let answered = db.append(&key, &drop, &unnamed(), Some(7)).unwrap();
    assert_eq!(
        (answered.state(), answered.revision()),
        (State::Answered, 8)
    );
    assert_eq!(stored(&db, key).len(), 8);
}

/// The agent's writes, and the human's withdraw, take the same revision.
/// Each is refused at a revision the case has moved on from, and goes
/// through at its current one.
#[test]
fn every_writer_writes_at_a_revision() {
    let pickup = || agent_event("pickup", "{}");
    for (name, setup, write) in [
        (
            "amend",
            vec![],
            agent_event("amend", r#"{"links": ["https://example.com/log"]}"#),
        ),
        ("pickup", vec![answer_of("stuck")], pickup()),
        (
            "note",
            vec![answer_of("stuck")],
            agent_event("note", r#"{"body": "Which mirror?", "reopen": true}"#),
        ),
        (
            "close",
            vec![answer_of("stuck"), pickup()],
            agent_event("close", r#"{"outcome": "Done."}"#),
        ),
        ("withdraw", vec![], agent_event("withdraw", "{}")),
        (
            "the human's withdraw",
            vec![],
            human_event("withdraw", r#"{"reason": "swept"}"#),
        ),
    ] {
        let (_dir, db) = scratch();
        let (key, _) = create(&db, "stuck");
        for event in setup {
            db.append(&key, &event, &unnamed(), None).unwrap();
        }
        let revision = stored(&db, key).len() as u64;
        assert_eq!(
            refused(db.append(&key, &write, &unnamed(), Some(revision - 1))),
            Category::Stale,
            "{name}"
        );
        assert_eq!(stored(&db, key).len() as u64, revision, "{name}");
        let done = db.append(&key, &write, &unnamed(), Some(revision)).unwrap();
        assert_eq!(done.revision(), revision + 1, "{name}");
    }
}

/// The revision is checked inside the write's transaction. Another writer
/// holds the lock and adds a note while the answer waits for it; the answer
/// must then see the note and be refused, not pass a check made before it
/// had the lock.
#[test]
fn the_revision_is_checked_inside_the_write_transaction() {
    let (_dir, db) = scratch();
    let (key, case) = create(&db, "fyi");
    let mut other = sql(&db);
    let tx = other
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "INSERT INTO events (case_key, seq, author, event, at, data)
         VALUES (?1, 2, 'agent', 'note', '', CAST('{\"body\":\"One more thing.\"}' AS BLOB))",
        [common::key_sql(key)],
    )
    .unwrap();
    thread::scope(|scope| {
        let (done, outcome) = mpsc::channel();
        let db = &db;
        scope.spawn(move || {
            let answer = human_event("answer", r#"{"ack": true}"#);
            done.send(db.append(&key, &answer, &unnamed(), Some(case.revision())))
                .unwrap();
        });
        // The pause does not decide the result; it gives an answer that did
        // not wait for the lock time to go through, so the test would catch
        // one.
        thread::sleep(Duration::from_millis(50));
        assert!(
            outcome.try_recv().is_err(),
            "the answer did not wait for the other writer"
        );
        tx.commit().unwrap();
        assert_eq!(refused(outcome.recv().unwrap()), Category::Stale);
    });
    assert_eq!(
        names(&stored(&db, key)),
        ["0001-agent-open.json", "0002-agent-note.json"]
    );
}

/// An event is stamped once its write holds the store's lock, so an event
/// stored after another never records an earlier time, however long it
/// waited.
#[test]
fn events_are_stamped_once_the_write_holds_the_lock() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let mut other = sql(&db);
    let tx = other
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let answered = thread::scope(|scope| {
        let db = &db;
        let answer = scope.spawn(move || {
            let answer = human_event("answer", r#"{"ack": true}"#);
            db.append(&key, &answer, &unnamed(), None)
        });
        thread::sleep(Duration::from_millis(100));
        let released = now();
        tx.rollback().unwrap();
        let case = answer.join().unwrap().unwrap();
        (case, released)
    });
    let (case, released) = answered;
    let at = case.answer().unwrap().answered_at.unwrap();
    assert!(
        at >= released,
        "answered at {at}, before the lock was released at {released}"
    );
}

/// The system's time now.
fn now() -> Time {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    Time::from_unix(
        i64::try_from(since.as_secs()).unwrap(),
        since.subsec_nanos(),
    )
    .unwrap()
}

#[test]
fn a_written_record_has_its_events_fields_and_time() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-15T10:00:00Z");
    let (key, _) = create(&db, "decision");
    let answer = human_event("answer", r#"{"choice": 2, "note": "go"}"#);
    db.append(&key, &answer, &unnamed(), None).unwrap();
    let rows = stored(&db, key);
    assert_eq!(
        (rows[1].author.as_str(), rows[1].event.as_str()),
        ("human", "answer")
    );
    let record: Value = serde_json::from_str(&rows[1].data).unwrap();
    let actor = &db.actors().unwrap()[1];
    assert_eq!(actor.kind, "human");
    assert_eq!(
        record,
        json!({"choice": 2, "note": "go", "actor": {"id": actor.id}, "answered_at": "2026-09-15T10:00:00Z"})
    );
    assert_eq!(rows[1].at, "2026-09-15T10:00:00Z");
}

/// The `at` column holds the event's time in UTC, whatever offset its
/// record was given with (ST-01); the record keeps it as written, in UTC.
#[test]
fn the_at_column_is_the_record_s_time_in_utc() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let answer = human_event(
        "answer",
        r#"{"ack": true, "answered_at": "2026-09-15T10:01:00+01:00"}"#,
    );
    db.append(&key, &answer, &unnamed(), None).unwrap();
    let rows = stored(&db, key);
    assert_eq!(rows[1].at, "2026-09-15T09:01:00Z");
    let data: Value = serde_json::from_str(&rows[1].data).unwrap();
    assert_eq!(data["answered_at"], "2026-09-15T09:01:00Z");
}

/// `create_rows` gives the open's rows as it stored them, and they fold to
/// the case `create` would give.
#[test]
fn create_rows_gives_the_rows_the_open_stored() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-15T09:12:03Z");
    let open = agent_event(
        "open",
        r#"{"kind": "fyi", "urgency": "whenever", "title": "Pin bun or float?"}"#,
    );
    let (key, rows) = db.create_rows(&open, &unnamed()).unwrap();
    assert_eq!(rows, db.rows(&key).unwrap().unwrap());
    assert_eq!(rows.len(), 1);
    assert_eq!(Case::fold(&rows).unwrap(), get(&db, key));
    // A refused open stores nothing, and makes no store.
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-15T09:12:03Z");
    let bad = agent_event("open", r#"{"kind": "fyi"}"#);
    assert!(matches!(
        db.create_rows(&bad, &unnamed()),
        Err(StoreError::Refused(_))
    ));
    assert!(!dir.path().join("cases.db").exists());
}

/// `append_rows` reads the case's rows in the append's own transaction: a
/// writer waiting on the lock while the append runs writes after it, and
/// its row is not among those the append gives.
#[test]
fn append_rows_gives_the_rows_of_its_own_write_and_no_later_one() {
    let (dir, _) = scratch();
    let path = dir.path().join("cases.db");
    let (key, _) = create(&fixed(&path, "2026-09-15T09:12:03Z"), "fyi");
    let other = std::sync::Mutex::new(None);
    let later = path.clone();
    let db = Db::new(&path).with_clock(move || {
        // On the append's first read of the clock, under the store's lock, a
        // second writer starts, and waits for that lock.
        let mut other = other.lock().unwrap();
        if other.is_none() {
            let later = later.clone();
            *other = Some(thread::spawn(move || {
                fixed(&later, "2026-09-15T09:14:00Z")
                    .append(
                        &key,
                        &agent_event("note", r#"{"body": "later"}"#),
                        &unnamed(),
                        None,
                    )
                    .unwrap()
            }));
        }
        common::at("2026-09-15T09:13:00Z")
    });
    let note = agent_event("note", r#"{"body": "mine"}"#);
    let rows = db.append_rows(&key, &note, &unnamed(), Some(1)).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows[1].data.starts_with(r#"{"body":"mine","#),
        "{}",
        rows[1].data
    );
    // The second writer stored its row after.
    let deadline = Instant::now() + Duration::from_secs(10);
    while db.rows(&key).unwrap().unwrap().len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let now = db.rows(&key).unwrap().unwrap();
    assert_eq!(now.len(), 3);
    assert_eq!(now[..2], rows[..]);
    // A refused append stores nothing, and a case not there is none.
    assert_eq!(
        refused(db.append_rows(&key, &note, &unnamed(), Some(1))),
        Category::Stale
    );
    let gone = Key::new(99).unwrap();
    assert!(matches!(
        db.append_rows(&gone, &note, &unnamed(), None),
        Err(StoreError::NoCase)
    ));
}

/// What `create_rows` and `append_rows` give is, field for field, what a
/// read of the case gives after the write: a first row, later rows with the
/// time and the actor the write stamps, rows past a gap in the seqs, a row
/// another writer stored that does not read, and one whose data is not
/// UTF-8 and reads with U+FFFD.
#[test]
fn the_rows_a_write_gives_are_those_a_read_gives_after_it() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-15T09:12:03Z");
    let given = |rows: Vec<Row>, key: Key| {
        assert_eq!(rows, db.rows(&key).unwrap().unwrap());
    };
    for kind in ["fyi", "decision", "approval"] {
        let (key, rows) = db.create_rows(&open_of(kind), &unnamed()).unwrap();
        given(rows, key);
        let note = agent_event("note", r#"{"body": "More."}"#);
        given(db.append_rows(&key, &note, &unnamed(), None).unwrap(), key);
        let named = Writer::new(Some("w1"));
        given(db.append_rows(&key, &note, &named, None).unwrap(), key);
        given(
            db.append_rows(&key, &answer_of(kind), &named, None)
                .unwrap(),
            key,
        );
    }

    // Rows another writer stored: one past a gap in the seqs, one with an
    // event this cases does not know, and one whose data is not UTF-8.
    let (key, _) = create(&db, "fyi");
    insert_row(&db, common::key_sql(key), 5, "agent", "poke", "{}");
    insert_row(
        &db,
        common::key_sql(key),
        7,
        "agent",
        "note",
        r#"{"body": "x"}"#,
    );
    sql(&db)
        .execute(
            "UPDATE events SET data = x'7b22626f6479223a22ff227d' WHERE case_key = ?1 AND seq = 7",
            [common::key_sql(key)],
        )
        .unwrap();
    let note = agent_event("note", r#"{"body": "After."}"#);
    let rows = db.append_rows(&key, &note, &unnamed(), None).unwrap();
    let seqs: Vec<u64> = rows.iter().map(|row| row.seq).collect();
    assert_eq!(seqs, [1, 5, 7, 8]);
    assert_eq!(rows[2].data, "{\"body\":\"\u{fffd}\"}");
    given(rows, key);
}
