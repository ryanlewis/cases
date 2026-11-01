//! The agent's writes after the answer: `cases pickup`, `cases note`, `cases close` and `cases withdraw` (AG-08 to AG-12, CL-03).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
use cases_core::State;
use common::{Env, json, write_file};

/// A decision case, answered.
fn answered(env: &Env) -> String {
    let id = env.open_decision();
    env.write(&id, "human", "answer", r#"{"choice": 1}"#);
    id
}

#[test]
fn pickup_records_who_read_the_answer() {
    let env = Env::new();
    let id = env.open_decision();
    let stderr = env.refused(&id, 3, &["pickup", &id]);
    assert!(stderr.starts_with("Error: "), "{stderr}");
    env.write(&id, "human", "answer", r#"{"choice": 1}"#);
    assert_eq!(
        env.ok(&["pickup", &id, "--by", "agent-inbox-bd"]),
        format!("{id} pickedup\n")
    );
    let case = env.case(&id);
    assert_eq!(case.pickup().unwrap().by, "agent-inbox-bd");
    assert!(case.pickup().unwrap().picked_up_at.is_some());
}

#[test]
fn pickup_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = answered(&env);
    env.refused_as_stale(&id, 1, 2, &["pickup", &id, "--revision", "1"]);
    let out = json(&env.ok(&["pickup", &id, "--revision", "2", "--json"]));
    assert_eq!(
        out,
        serde_json::json!({ "id": id, "state": "pickedup", "revision": 3 })
    );
}

#[test]
fn the_agents_writes_carry_the_workers_actor() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "T",
        "--worker",
        "bun-pins",
    ]);
    env.write(&id, "human", "answer", r#"{"ack": true}"#);
    env.ok(&["pickup", &id]);
    env.ok(&["close", &id, "--outcome", "Done."]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    for seq in [0, 2, 3] {
        assert_eq!(
            shown["events"][seq]["actor"]["name"], "bun-pins",
            "event {seq}"
        );
    }
    // A case opened with no worker: the unnamed agent.
    let other = env.open_fyi("No worker");
    env.ok(&["note", &other, "--body", "more"]);
    let shown = json(&env.ok(&["show", &other, "--json"]));
    assert_eq!(shown["events"][1]["actor"]["kind"], "agent");
    assert!(shown["events"][1]["actor"].get("name").is_none());
}

#[test]
fn a_note_reopens_an_answered_case() {
    let env = Env::new();
    let id = answered(&env);
    env.ok(&["pickup", &id]);
    let out = env.run_with_stdin(
        "Pin to which patch?",
        env.store_args(&["note", &id, "--reopen", "--body-file", "-"]),
    );
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{id} open\n"));
    let case = env.case(&id);
    assert!(case.answer().is_none());
    assert_eq!(case.events().len(), 4);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["events"][3]["data"]["reopen"], true);
    assert!(env.ok(&["show", &id]).contains("reopened the case"));
    env.write(&id, "human", "answer", r#"{"choice": 2}"#);

    env.refused_with_stdin(
        &id,
        1,
        "   \n",
        &["note", &id, "--reopen", "--body-file", "-"],
    );
    let out = env.cases(&["note", &id]);
    assert_eq!(out.code, 1, "{out:?}");
}

/// A note on an answered or picked-up case says it reopens the case, so a
/// follow-up never throws an answer away by accident (cases `6M6gx`, `AV8s1`).
#[test]
fn a_note_without_reopen_leaves_an_answered_case_alone() {
    let env = Env::new();
    let id = answered(&env);
    for pick_up in [true, false] {
        let error = env.refused(&id, 3, &["note", &id, "--body", "Pin to which patch?"]);
        assert!(error.contains("--reopen"), "{error}");
        assert!(env.case(&id).answer().is_some());
        if pick_up {
            env.ok(&["pickup", &id]);
        }
    }
}

#[test]
fn a_note_with_reopen_on_an_open_case_is_refused() {
    let env = Env::new();
    let id = env.open_decision();
    let error = env.refused(&id, 1, &["note", &id, "--reopen", "--body", "More."]);
    assert!(!error.contains("pass --reopen"), "{error}");
    env.ok(&["note", &id, "--body", "More."]);
}

#[test]
fn a_note_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = answered(&env);
    env.refused_as_stale(
        &id,
        1,
        2,
        &[
            "note",
            &id,
            "--reopen",
            "--body",
            "Pin to which patch?",
            "--revision",
            "1",
        ],
    );
    assert!(
        env.case(&id).answer().is_some(),
        "the refused note cleared the answer"
    );
    assert_eq!(
        env.ok(&[
            "note",
            &id,
            "--reopen",
            "--body",
            "Pin to which patch?",
            "--revision",
            "2"
        ]),
        format!("{id} open\n")
    );
}

#[test]
fn a_note_takes_an_inline_body() {
    let env = Env::new();
    let id = env.open_decision();
    write_file(&env.path("note.md"), "From a file.");
    env.refused(
        &id,
        1,
        &["note", &id, "--body", "x", "--body-file", "note.md"],
    );
    let stderr = env.refused(&id, 1, &["note", &id, "--body", "note.md"]);
    assert!(
        stderr.contains("names a file; pass it with --body-file"),
        "{stderr}"
    );
    env.refused(&id, 1, &["note", &id, "--body", " "]);
    assert_eq!(
        env.ok(&["note", &id, "--body", "Also pin bunx."]),
        format!("{id} open\n")
    );
    let case = env.case(&id);
    assert!(case.events()[1].data.contains("Also pin bunx."));
}

#[test]
fn close_records_the_outcome_of_a_picked_up_case() {
    let env = Env::new();
    let id = answered(&env);
    write_file(&env.path("outcome.md"), "Pinned in #12.");
    env.refused(&id, 3, &["close", &id, "--outcome-file", "outcome.md"]);
    env.ok(&["pickup", &id]);
    let out = env.ok(&[
        "close",
        &id,
        "--outcome-file",
        "outcome.md",
        "--link",
        "https://github.com/x/y/pull/12",
    ]);
    assert_eq!(out, format!("{id} closed\n"));
    let case = env.case(&id);
    assert_eq!(case.close().unwrap().outcome, "Pinned in #12.");
    assert_eq!(
        case.close().unwrap().links,
        ["https://github.com/x/y/pull/12"]
    );
    for args in [&["--outcome-file", "missing.md"][..], &[]] {
        let mut all = vec!["close", id.as_str()];
        all.extend(args);
        env.refused(&id, 1, &all);
    }
}

#[test]
fn close_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = answered(&env);
    env.ok(&["pickup", &id]);
    env.refused_as_stale(
        &id,
        2,
        3,
        &["close", &id, "--outcome", "Pinned.", "--revision", "2"],
    );
    assert_eq!(
        env.ok(&["close", &id, "--outcome", "Pinned.", "--revision", "3"]),
        format!("{id} closed\n")
    );
}

#[test]
fn close_takes_an_inline_outcome() {
    let env = Env::new();
    let id = answered(&env);
    env.ok(&["pickup", &id]);
    write_file(&env.path("outcome.md"), "Pinned in #12.");
    env.refused(
        &id,
        1,
        &[
            "close",
            &id,
            "--outcome",
            "Acknowledged",
            "--outcome-file",
            "outcome.md",
        ],
    );
    let stderr = env.refused(&id, 1, &["close", &id, "--outcome", "outcome.md"]);
    assert!(
        stderr.contains("names a file; pass it with --outcome-file"),
        "{stderr}"
    );
    env.refused(&id, 1, &["close", &id, "--outcome", ""]);
    let out = json(&env.ok(&["close", &id, "--outcome", "Acknowledged", "--json"]));
    assert_eq!(
        out,
        serde_json::json!({ "id": id, "state": "closed", "revision": 4 })
    );
    assert_eq!(env.case(&id).close().unwrap().outcome, "Acknowledged");
}

#[test]
fn withdraw_takes_back_an_open_case() {
    let env = Env::new();
    let id = env.open_decision();
    assert_eq!(env.ok(&["withdraw", &id]), format!("{id} withdrawn\n"));
    env.refused(&id, 3, &["answer", &id, "--option", "1"]);
    env.refused(&id, 3, &["withdraw", &id]);
    // An answered case is not withdrawn.
    let answered = answered(&env);
    env.refused(&answered, 3, &["withdraw", &answered]);
}

/// A case the human withdrew once the agent had picked it up, such as one
/// whose session the human took for dead, is finished for the agent: its
/// close and its note exit 3, and 4 with the revision it read before, whose
/// error says the case is now withdrawn. The case keeps the answer (ST-14,
/// AG-10, AG-12).
#[test]
fn a_case_the_human_withdrew_takes_no_close_or_note_from_the_agent() {
    let env = Env::new();
    let id = answered(&env);
    let picked = json(&env.ok(&["pickup", &id, "--json"]));
    assert_eq!(picked["revision"], 3);
    env.ok(&[
        "withdraw",
        &id,
        "--as",
        "Ryan",
        "--reason",
        "its session died",
    ]);

    env.refused(&id, 3, &["close", &id, "--outcome", "Pinned."]);
    env.refused(&id, 3, &["note", &id, "--body", "Still there?"]);
    let close = ["close", &id, "--revision", "3", "--outcome", "Pinned."];
    let error = env.refused_as_stale(&id, 3, 4, &close);
    assert!(error.ends_with("; it is now withdrawn\n"), "{error}");
    let note = ["note", &id, "--revision", "3", "--body", "Still there?"];
    env.refused_as_stale(&id, 3, 4, &note);
    let answer = json(&env.ok(&["show", &id, "--answer"]));
    assert_eq!(answer["state"], "withdrawn");
    assert_eq!(answer["revision"], 4);
    assert_eq!(answer["answer"]["choice"], 1);
}

#[test]
fn withdraw_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = env.open_decision();
    env.write(
        &id,
        "agent",
        "amend",
        r#"{"links": ["https://example.com/log"]}"#,
    );
    env.refused_as_stale(&id, 1, 2, &["withdraw", &id, "--revision", "1"]);
    assert_eq!(
        env.ok(&["withdraw", &id, "--revision", "2"]),
        format!("{id} withdrawn\n")
    );
}

#[test]
fn withdraw_records_its_reason() {
    let env = Env::new();
    let id = env.open_decision();
    env.ok(&["withdraw", &id]);
    let data = json(&env.case(&id).events()[1].data);
    let mut fields: Vec<&String> = data.as_object().unwrap().keys().collect();
    fields.sort();
    assert_eq!(fields, ["actor", "withdrawn_at"]);
    assert!(!env.ok(&["show", &id]).contains("reason:"));

    let id = env.open_decision();
    env.ok(&["withdraw", &id, "--reason", "found it in the lockfile"]);
    let case = env.case(&id);
    assert_eq!(case.state(), State::Withdrawn);
    assert_eq!(
        json(&case.events()[1].data)["reason"],
        "found it in the lockfile"
    );
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("0002 agent withdraw")
            && text.contains("       reason: found it in the lockfile\n"),
        "{text}"
    );
}

#[test]
fn a_close_shows_its_outcome_links_and_time() {
    let env = Env::new();
    let id = answered(&env);
    env.ok(&["pickup", &id]);
    env.ok(&[
        "close",
        &id,
        "--outcome",
        "Pinned.",
        "--link",
        "https://x/1",
        "--link",
        "https://x/2",
    ]);
    let close = &json(&env.ok(&["show", &id, "--json"]))["close"];
    assert_eq!(close["outcome"], "Pinned.");
    assert_eq!(
        close["links"],
        serde_json::json!(["https://x/1", "https://x/2"])
    );
    assert!(close["closed_at"].is_string(), "{close}");
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("       Pinned.\n       https://x/1\n       https://x/2\n"),
        "{text}"
    );
}

/// A dropped case is picked up and closed as any answer is (HU-02).
#[test]
fn a_dropped_case_is_picked_up_and_closed() {
    let env = Env::new();
    let id = env.open_decision();
    env.ok(&["answer", &id, "--drop", "--note", "Not needed."]);
    assert_eq!(env.ok(&["pickup", &id]), format!("{id} pickedup\n"));
    assert_eq!(
        env.ok(&["close", &id, "--outcome", "Stopped."]),
        format!("{id} closed\n")
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["answer"]["drop"], true);
    assert_eq!(shown["state"], "closed");
}

/// The agent's writes take whole ids only (HU-07).
#[test]
fn the_agents_writes_take_no_part_of_an_id() {
    let env = Env::new();
    let id = answered(&env);
    env.ok(&["pickup", &id]);
    assert!(id.contains("kLW") && id != "kLW", "{id}");
    for args in [
        &["note", "kLW", "--body", "x"][..],
        &["close", "kLW", "--outcome", "x"],
        &["withdraw", "kLW"],
        &["amend", "kLW", "--context", "x"],
        &["pickup", "kLW"],
    ] {
        let out = env.cases(args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
    }
    assert_eq!(env.revision(&id), 3);
}

/// Every write prints its id, state and new revision with `--json` (CL-02).
#[test]
fn note_and_withdraw_print_json() {
    let env = Env::new();
    let id = env.open_decision();
    let noted = json(&env.ok(&["note", &id, "--body", "More.", "--json"]));
    assert_eq!(
        noted,
        serde_json::json!({ "id": id, "state": "open", "revision": 2 })
    );
    let withdrawn = json(&env.ok(&["withdraw", &id, "--json"]));
    assert_eq!(
        withdrawn,
        serde_json::json!({ "id": id, "state": "withdrawn", "revision": 3 })
    );
}
