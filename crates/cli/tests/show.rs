//! `cases show` (AG-13, HU-07, ST-05).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fmt::Write as _;

use common::{Env, json, write_file};

#[test]
fn show_prints_the_case_and_its_thread() {
    let env = Env::new();
    let id = env.open_decision();
    env.write_as(
        &id,
        "human",
        "answer",
        r#"{"choice": 2, "note": "watch the lockfile"}"#,
        Some("Ryan"),
    );

    let out = env.ok(&["show", &id]);
    for want in [
        "Pin bun?\n\n",
        &format!("id:       {id}-pin-bun\n"),
        "state:    answered\n",
        "kind:     decision\n",
        "urgency:  today\n",
        "revision: 2\n",
        "  1. Pin to 1.2.3\n",
        "  other. Other, see note\n",
        "  0001 agent open     ",
        "  0002 human answer   ",
        "       by Ryan\n       chose 2. Float, with renovate\n       note: watch the lockfile\n",
    ] {
        assert!(out.contains(want), "show lacks {want:?}:\n{out}");
    }

    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "answered");
    assert_eq!(shown["answer"]["choice"], 2);
    assert_eq!(shown["revision"], 2);
    let events = shown["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["file"], "0002-human-answer.json");
    assert_eq!(events[1]["seq"], 2);
    assert_eq!(events[1]["author"], "human");
    assert_eq!(events[1]["event"], "answer");
    assert_eq!(events[1]["actor"]["kind"], "human");
    assert_eq!(events[1]["actor"]["name"], "Ryan");
    assert_eq!(events[1]["at"], events[1]["data"]["answered_at"]);
    assert_eq!(shown["updated_at"], events[1]["at"]);
    for key in [
        "id",
        "kind",
        "urgency",
        "title",
        "options",
        "opened_at",
        "url",
    ] {
        assert!(shown.get(key).is_some(), "show --json lacks {key}: {shown}");
    }
}

#[test]
fn show_json_keeps_each_record_as_stored() {
    let env = Env::new();
    let id = env.open_fyi("Unknown fields");
    // A field this version does not define is kept in the row, and shown.
    env.insert(
        Some(&id),
        2,
        "human",
        "answer",
        r#"{"ack": true, "mood": "fine", "answered_at": "2026-09-25T12:00:00Z"}"#,
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["events"][1]["data"]["mood"], "fine");
    assert_eq!(shown["state"], "answered");
}

#[test]
fn show_describes_a_question_reply_as_a_reply() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "question",
        "--urgency",
        "today",
        "--title",
        "Which host?",
    ]);
    env.write(&id, "human", "answer", r#"{"text": "the staging one"}"#);
    let out = env.ok(&["show", &id]);
    for want in [
        "kind:     question",
        "0002 human answer",
        "reply: the staging one",
    ] {
        assert!(out.contains(want), "show lacks {want:?}:\n{out}");
    }
    assert!(!out.contains("guidance:"), "{out}");
}

#[test]
fn show_prints_what_an_amend_replaced() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "Pin?",
        "--option",
        "Pin",
        "--body",
        "Old body.",
        "--context",
        "old context",
    ]);
    env.write(&id, "agent", "amend", r#"{"body": "New body.", "options": ["Float"], "links": ["https://x"], "labels": ["l"], "context": "new context"}"#);
    // The same body again replaces nothing.
    env.write(
        &id,
        "agent",
        "amend",
        r#"{"body": "New body.", "labels": ["m"]}"#,
    );
    let out = env.ok(&["show", &id]);
    let want = "  0002 agent amend    ";
    assert!(out.contains(want), "{out}");
    for want in [
        "       replaced the body\n         previous body:\n         | Old body.\n",
        "       added option: Float\n",
        "       added link: https://x\n",
        "       added label: l\n",
        "       replaced the context: new context\n         previous context:\n         | old context\n",
        "  2. Float\n",
    ] {
        assert!(out.contains(want), "show lacks {want:?}:\n{out}");
    }
    assert_eq!(out.matches("replaced the body").count(), 1, "{out}");
}

#[test]
fn show_refuses_an_id_that_names_no_case() {
    let env = Env::new();
    for id in ["../etc", "nope", "UkLWZ", ""] {
        let out = env.cases(&["show", id]);
        assert_eq!(out.code, 1, "show {id:?}: {out:?}");
        assert_eq!(out.stdout, "");
        assert!(out.stderr.starts_with("Error: "), "{out:?}");
    }
    env.open_fyi("One");
    let out = env.cases(&["show", "gbHJd"]);
    assert_eq!(out.stderr, "Error: no case gbHJd\n");
}

#[test]
fn show_takes_part_of_an_id() {
    let env = Env::new();
    let first = env.open_fyi("First"); // UkLWZ
    let second = env.open_fyi("Second"); // gbHJd
    assert_eq!((first.as_str(), second.as_str()), ("UkLWZ", "gbHJd"));
    for (part, want) in [
        ("UkLWZ", &first),
        ("kLW", &first),
        ("bHJ", &second),
        ("UkLWZ-first", &first),
        ("kLW-anything", &first),
        ("UkLWZ-a-slug-out-of-date", &first),
    ] {
        let out = env.ok(&["show", part]);
        let slug = if want == &first { "first" } else { "second" };
        assert!(
            out.contains(&format!("id:       {want}-{slug}\n")),
            "show {part}:\n{out}"
        );
    }
}

#[test]
fn show_lists_the_cases_a_part_of_an_id_matches() {
    let env = Env::new();
    // Keys 1 to 30: enough for two ids to share a letter.
    let ids: Vec<String> = (1..=30)
        .map(|n| env.open_fyi(&format!("Case {n}")))
        .collect();
    let letter = ('a'..='z')
        .chain('A'..='Z')
        .chain('0'..='9')
        .find(|c| ids.iter().filter(|id| id.contains(*c)).count() > 1)
        .unwrap()
        .to_string();
    let matching: Vec<&String> = ids.iter().filter(|id| id.contains(&letter)).collect();
    let out = env.cases(&["show", &letter]);
    assert_eq!(out.code, 1, "{out:?}");
    let mut want = format!("Error: {letter:?} matches {} cases:\n", matching.len());
    for id in &matching {
        let n = ids.iter().position(|i| i == *id).unwrap() + 1;
        writeln!(want, "  {id}  Case {n}").unwrap();
    }
    assert_eq!(out.stderr, want);

    let out = env.cases(&["show", "mirror"]);
    assert_eq!(out.stderr, "Error: no case id contains \"mirror\"\n");
}

#[test]
fn a_whole_id_that_is_not_in_the_store_matches_no_other_case() {
    let env = Env::new();
    let id = env.open_fyi("Gone");
    let db = env.db();
    cases_core::Store::delete(&db, &common::key(&id), None).unwrap();
    env.open_fyi("Still here");
    let out = env.cases(&["show", &id]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, format!("Error: no case {id}\n"));
}

#[test]
fn show_on_a_store_that_is_not_there() {
    let env = Env::new();
    let out = env.cases(&["show", "kLW"]);
    assert_eq!(out.stderr, "Error: no case id contains \"kLW\"\n");
    assert!(!env.store.exists());
}

#[test]
fn show_json_revision_counts_skipped_rows() {
    let env = Env::new();
    let id = env.open_decision();
    // A malformed event is skipped by the fold but still counts.
    env.insert(Some(&id), 2, "human", "answer", r#"{"choice": 2"#);
    let out = env.cases(&["show", &id, "--json"]);
    assert_eq!(out.code, 0, "{out:?}");
    let shown = json(&out.stdout);
    assert_eq!(shown["revision"], 2);
    assert_eq!(shown["events"].as_array().unwrap().len(), 1);
    let problems = shown["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0]
            .as_str()
            .unwrap()
            .starts_with("0002-human-answer.json: "),
        "{problems:?}"
    );
    assert!(
        out.stderr
            .starts_with(&format!("warning: {id}: 0002-human-answer.json: ")),
        "{out:?}"
    );
    // The next write goes after the skipped row.
    let case = env.write(&id, "human", "answer", r#"{"choice": 1}"#);
    assert_eq!(case.revision(), 3);
}

#[test]
fn show_reports_an_unknown_event_as_version_skew() {
    let env = Env::new();
    let id = env.open_decision();
    env.insert(Some(&id), 2, "agent", "comment", r#"{"body":"x"}"#);
    let out = env.cases(&["show", &id]);
    assert_eq!(out.code, 0, "{out:?}");
    let problem = out
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("  problem: "))
        .unwrap_or_else(|| panic!("no problem line:\n{}", out.stdout));
    assert!(
        problem.starts_with("0002-agent-comment.json: "),
        "{problem}"
    );
    assert!(
        problem.contains("perhaps written by a newer cases"),
        "{problem}"
    );
    assert_eq!(out.stderr, format!("warning: {id}: {problem}\n"));
}

#[test]
fn show_answer_prints_only_the_state_kind_revision_and_answer() {
    let env = Env::new();
    let id = env.open_decision();
    let shown = || {
        let shown = json(&env.ok(&["show", &id, "--answer"]));
        let mut keys: Vec<&String> = shown.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["answer", "kind", "revision", "state"]);
        shown
    };
    assert_eq!(
        shown(),
        serde_json::json!({"state": "open", "kind": "decision", "revision": 1, "answer": null})
    );

    env.write(
        &id,
        "human",
        "answer",
        r#"{"choice": 2, "note": "watch the lockfile"}"#,
    );
    let answered = shown();
    assert_eq!(answered["state"], "answered");
    assert_eq!(answered["revision"], 2);
    assert_eq!(answered["answer"]["choice"], 2);
    assert_eq!(answered["answer"]["note"], "watch the lockfile");

    env.write(&id, "agent", "pickup", "{}");
    env.write(
        &id,
        "agent",
        "note",
        r#"{"body": "Which lockfile?", "reopen": true}"#,
    );
    let reopened = shown();
    assert_eq!(reopened["state"], "open");
    assert_eq!(reopened["revision"], 4);
    assert_eq!(reopened["answer"], serde_json::Value::Null);

    let out = env.cases(&["show", &id, "--answer", "--json"]);
    assert_eq!(out.code, 1, "{out:?}");
}

#[test]
fn show_names_each_actor_from_the_actors_table() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Actors",
        "--worker",
        "bun-pins",
    ]);
    env.write(&id, "human", "answer", r#"{"ack": true}"#);
    // A pickup by the actor it names has no second "by" line.
    env.write_as(
        &id,
        "agent",
        "pickup",
        r#"{"by": "bun-pins"}"#,
        Some("bun-pins"),
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["actor"]["name"], "bun-pins");
    assert_eq!(shown["events"][1]["actor"]["kind"], "human");
    // The unnamed human has no name.
    assert!(shown["events"][1]["actor"].get("name").is_none(), "{shown}");
    let out = env.ok(&["show", &id]);
    assert!(out.contains("  0001 agent open     "), "{out}");
    assert_eq!(out.matches("by bun-pins").count(), 2, "{out}");
    assert!(
        out.contains("0003 agent pickup   ") && out.contains("acknowledged"),
        "{out}"
    );
}

#[test]
fn show_gives_the_url_of_the_running_inbox() {
    let env = Env::new();
    let id = env.open_decision();
    let url = || {
        json(&env.ok(&["show", &id, "--json"]))["url"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(url(), "");
    assert!(!env.ok(&["show", &id]).contains("url:"));

    let listener = common::listen("127.0.0.1:0");
    let addr = listener.local_addr().unwrap();
    let info = serde_json::json!({
        "pid": std::process::id(),
        "url": format!("http://{addr}/"),
        "addr": addr.to_string(),
        "store": env.store,
        "started_at": "2026-09-25T12:00:00Z",
        "version": "0.0.0",
    });
    write_file(&env.instance_file(), &info.to_string());
    let want = format!("http://{addr}/cases/{id}-pin-bun");
    assert_eq!(url(), want);
    assert!(
        env.ok(&["show", &id])
            .contains(&format!("url:      {want}\n"))
    );

    // Nothing accepts on the address once serve stops: the file is stale.
    drop(listener);
    env.stale_instance(std::process::id());
    assert_eq!(url(), "");
}

#[test]
fn show_warns_on_a_damaged_instance_file() {
    let env = Env::new();
    let id = env.open_decision();
    write_file(&env.instance_file(), "{not json");
    let out = env.cases(&["show", &id]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout.contains(&format!("id:       {id}")) && !out.stdout.contains("url:"),
        "{out:?}"
    );
    let want = format!("warning: read {}: ", env.instance_file().display());
    assert!(out.stderr.starts_with(&want), "{out:?}");
}

#[test]
fn show_json_gives_the_amend_seq() {
    let env = Env::new();
    let id = env.open_decision();
    assert_eq!(json(&env.ok(&["show", &id, "--json"]))["amend_seq"], 0);
    // A link leaves the question as it was; an option changes it.
    env.write(&id, "agent", "amend", r#"{"links": ["https://x"]}"#);
    assert_eq!(json(&env.ok(&["show", &id, "--json"]))["amend_seq"], 0);
    env.write(&id, "agent", "amend", r#"{"options": ["Wait"]}"#);
    assert_eq!(json(&env.ok(&["show", &id, "--json"]))["amend_seq"], 3);
}

/// Each event gives the state it moved the case from, left out on the open,
/// and the case gives its last seq, skipped rows included: what a form keeps
/// to say later how the case changed since (IN-05).
#[test]
fn show_json_gives_each_event_s_from_and_the_last_seq() {
    let env = Env::new();
    let id = env.open_decision();
    env.write(&id, "human", "answer", r#"{"choice": 1}"#);
    env.write(
        &id,
        "agent",
        "note",
        r#"{"body": "Again.", "reopen": true}"#,
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    let events = shown["events"].as_array().unwrap();
    assert!(events[0].get("from").is_none(), "{}", events[0]);
    assert_eq!(events[1]["from"], "open");
    assert_eq!(events[2]["from"], "answered");
    assert_eq!(shown["last_seq"], 3);
    // A row the fold skips still counts.
    env.insert(Some(&id), 4, "human", "answer", r#"{"choice": 2"#);
    let shown = json(&env.cases(&["show", &id, "--json"]).stdout);
    assert_eq!(shown["events"].as_array().unwrap().len(), 3);
    assert_eq!(shown["last_seq"], 4);
}

#[test]
fn an_actor_without_a_name_or_a_row_is_shown_by_what_is_known() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Actors",
        "--worker",
        "w1",
    ]);
    env.write(&id, "human", "answer", r#"{"ack": true}"#);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    let human = shown["events"][1]["actor"].clone();
    // An unnamed actor has its id and kind.
    assert_eq!(human.as_object().unwrap().len(), 2, "{human}");
    assert_eq!(human["kind"], "human");

    // An actor whose row is deleted has its id alone.
    let agent = shown["actor"]["id"].as_str().unwrap().to_owned();
    let conn = rusqlite::Connection::open(&env.store).unwrap();
    conn.execute("DELETE FROM actors WHERE id = ?1", [&agent])
        .unwrap();
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["actor"], serde_json::json!({ "id": agent }));
    assert_eq!(
        shown["events"][0]["actor"],
        serde_json::json!({ "id": agent })
    );
    assert!(env.ok(&["show", &id]).contains("by deleted user"));
}

/// A `reject` verdict is stored as it is, and shown to people as "don't
/// run" (HU-01).
#[test]
fn show_says_a_rejected_row_is_not_to_be_run() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "approval",
        "--urgency",
        "today",
        "--title",
        "Scripts",
        "--row",
        r#"{"id":"mig","label":"Migrate","script":"make migrate","link":"https://x"}"#,
    ]);
    env.ok(&["answer", &id, "--row", "mig=reject:not today"]);
    assert_eq!(
        json(&env.ok(&["show", &id, "--json"]))["answer"]["rows"][0]["verdict"],
        "reject"
    );
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("       [mig] don't run: not today\n"),
        "{text}"
    );
}

/// The text `show` prints after the case's fields, up to the thread.
fn after_fields(text: &str) -> &str {
    let (_, fields_on) = text.split_once("\n\n").unwrap();
    let (_, rest) = fields_on.split_once("\n\n").unwrap();
    &rest[..rest.find("Thread:\n").unwrap() + "Thread:\n".len()]
}

#[test]
fn show_prints_the_body_rows_and_links_in_turn() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "approval",
        "--urgency",
        "today",
        "--title",
        "Release",
        "--body",
        "  Run the release steps.\n",
        "--row",
        r#"{"id":"mig","label":"Migrate","script":"make migrate","link":"https://x/mig","note":"back up first","by":"human"}"#,
        "--row",
        r#"{"id":"deps","label":"Install deps","script":"npm ci\nnpm test\n","link":"https://x/deps"}"#,
        "--link",
        "https://example.com/pr/1",
    ]);
    let text = env.ok(&["show", &id]);
    assert_eq!(
        after_fields(&text),
        "Run the release steps.\n\
         \n\
         Rows:\n  \
         [mig] Migrate\n      \
         by: human\n      \
         note: back up first\n      \
         https://x/mig\n      \
         | make migrate\n  \
         [deps] Install deps\n      \
         https://x/deps\n      \
         | npm ci\n      \
         | npm test\n\
         \n\
         Links:\n  \
         https://example.com/pr/1\n\
         \n\
         Thread:\n",
        "{text}"
    );
}

#[test]
fn show_leaves_out_an_empty_body_rows_and_links() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Bare",
        "--worker",
        "w1",
    ]);
    let text = env.ok(&["show", &id]);
    assert_eq!(after_fields(&text), "Thread:\n", "{text}");
    assert!(text.contains("\nworker:   w1\n"), "{text}");
    assert!(!text.contains("to:"), "{text}");
}

#[test]
fn show_names_a_messages_worker_as_its_recipient() {
    let env = Env::new();
    let id = env
        .ok(&["message", "--as", "Ryan", "--title", "Rebase", "--to", "w1"])
        .trim()
        .to_owned();
    let text = env.ok(&["show", &id]);
    assert!(text.contains("\nto:       w1\n"), "{text}");
    assert!(!text.contains("worker:"), "{text}");
}

#[test]
fn a_case_that_does_not_fold_is_refused_alike_by_its_id_or_part_of_it() {
    let env = Env::new();
    let good = env.open_fyi("Good");
    let bad = env.insert(None, 1, "agent", "open", "{}");
    let whole = env.cases(&["show", &bad]);
    assert_eq!(whole.code, 1, "{whole:?}");
    assert!(
        whole.stderr.starts_with(&format!("Error: {bad}: ")),
        "{whole:?}"
    );
    let part = env.cases(&["show", &bad[1..bad.len() - 1]]);
    assert_eq!((part.code, &part.stderr), (1, &whole.stderr), "{part:?}");

    // Listed among the cases a part matches, it has no title. More that do
    // not fold are stored until one shares a letter with the good case.
    let mut bad = vec![bad];
    let letter = loop {
        if let Some(c) = good.chars().find(|c| bad.iter().any(|id| id.contains(*c))) {
            break c.to_string();
        }
        bad.push(env.insert(None, 1, "agent", "open", "{}"));
    };
    let mut want = String::new();
    let mut n = 0;
    for id in std::iter::once(&good).chain(&bad) {
        if id.contains(&letter) {
            n += 1;
            let title = if id == &good { "Good" } else { "" };
            writeln!(want, "  {id}  {title}").unwrap();
        }
    }
    let out = env.cases(&["show", &letter]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!("Error: {letter:?} matches {n} cases:\n{want}")
    );
}
