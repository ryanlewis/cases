//! `cases answer` (HU-01, HU-02, HU-06, HU-07, AG-12, ST-04).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
use cases_core::State;
use common::{Env, json, write_file};

/// The arguments of `open` for a briefing with two left-out items.
const BRIEFING: &[&str] = &[
    "--kind",
    "briefing",
    "--body",
    "Pinned bun.",
    "--left-out",
    "Bump the lockfile",
    "--left-out",
    "Pin node too",
];

/// Opens a case with `open` after the urgency and title, and answers it with
/// `answer`. Gives the id and the answer's output.
fn open_and_answer(env: &Env, open: &[&str], answer: &[&str]) -> (String, common::Output) {
    let mut args = vec!["--urgency", "today", "--title", "T"];
    args.extend(open);
    let id = env.open(&args);
    let mut all = vec!["answer", id.as_str()];
    all.extend(answer);
    let out = env.cases(&all);
    (id, out)
}

#[test]
fn answer_takes_the_response_each_kind_asks_for() {
    let rows = [
        "--row",
        r#"{"id":"a","label":"A","script":"echo a","link":"https://x/a"}"#,
        "--row",
        r#"{"id":"b","label":"B","script":"echo b","link":"https://x/b"}"#,
    ];
    let approval: Vec<&str> = ["--kind", "approval"].into_iter().chain(rows).collect();
    let cases: &[(&str, &[&str], &[&str], serde_json::Value)] = &[
        (
            "decision option",
            &["--kind", "decision", "--option", "x", "--option", "y"],
            &["--option", "2"],
            serde_json::json!({"choice": 2}),
        ),
        (
            "decision other",
            &["--kind", "decision", "--option", "x"],
            &["--other", "--note", "neither"],
            serde_json::json!({"other": true, "note": "neither"}),
        ),
        (
            "approval rows with notes",
            &approval,
            &[
                "--row",
                "a=approve",
                "--row",
                "b=reject:needs a dry run: first",
            ],
            serde_json::json!({"rows": [{"id": "a", "verdict": "approve"}, {"id": "b", "verdict": "reject", "note": "needs a dry run: first"}]}),
        ),
        (
            "signoff accept",
            &["--kind", "signoff"],
            &["--accept"],
            serde_json::json!({"signoff": "accept"}),
        ),
        (
            "signoff changes",
            &["--kind", "signoff"],
            &["--changes", "--note", "rename it"],
            serde_json::json!({"signoff": "changes", "note": "rename it"}),
        ),
        (
            "briefing accept",
            BRIEFING,
            &["--accept"],
            serde_json::json!({"briefing": "accept"}),
        ),
        (
            "briefing accept that files items",
            BRIEFING,
            &["--accept", "--file", "2", "--file", "1"],
            serde_json::json!({"briefing": "accept", "file": [2, 1]}),
        ),
        (
            "briefing send back",
            BRIEFING,
            &["--send-back", "--note", "show the diff", "--file", "1"],
            serde_json::json!({"briefing": "send_back", "note": "show the diff", "file": [1]}),
        ),
        (
            "briefing drop",
            BRIEFING,
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
        (
            "stuck text",
            &["--kind", "stuck"],
            &["--text", "use the mirror"],
            serde_json::json!({"text": "use the mirror"}),
        ),
        (
            "question text",
            &["--kind", "question"],
            &[
                "--text",
                "the staging one",
                "--note",
                "ask again if it moves",
            ],
            serde_json::json!({"text": "the staging one", "note": "ask again if it moves"}),
        ),
        (
            "fyi ack",
            &["--kind", "fyi"],
            &["--ack", "--note", "thanks"],
            serde_json::json!({"ack": true, "note": "thanks"}),
        ),
        // Any kind can be dropped.
        (
            "stuck drop",
            &["--kind", "stuck"],
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
        (
            "question drop",
            &["--kind", "question"],
            &["--drop", "--note", "not needed"],
            serde_json::json!({"drop": true, "note": "not needed"}),
        ),
        (
            "decision drop",
            &["--kind", "decision", "--option", "x"],
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
        (
            "approval drop",
            &approval,
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
        (
            "signoff drop",
            &["--kind", "signoff"],
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
        (
            "fyi drop",
            &["--kind", "fyi"],
            &["--drop"],
            serde_json::json!({"drop": true}),
        ),
    ];
    for (name, open, answer, want) in cases {
        let env = Env::new();
        let (id, out) = open_and_answer(&env, open, answer);
        assert_eq!(out.code, 0, "{name}: {out:?}");
        assert_eq!(out.stdout, format!("{id} answered\n"), "{name}");
        let shown = json(&env.ok(&["show", &id, "--json"]));
        assert_eq!(shown["state"], "answered", "{name}");
        for (field, value) in want.as_object().unwrap() {
            assert_eq!(&shown["answer"][field], value, "{name}: {field}");
        }
        let data = &shown["events"][1]["data"];
        assert!(data["answered_at"].is_string(), "{name}: {data}");
        assert_eq!(shown["events"][1]["file"], "0002-human-answer.json");
    }
}

#[test]
fn an_answer_that_does_not_fit_the_kind_is_refused() {
    let rows = [
        "--row",
        r#"{"id":"a","label":"A","script":"echo a","link":"https://x/a"}"#,
        "--row",
        r#"{"id":"b","label":"B","script":"echo b","link":"https://x/b"}"#,
    ];
    let approval: Vec<&str> = ["--kind", "approval"].into_iter().chain(rows).collect();
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "decision other needs a note",
            &["--kind", "decision", "--option", "x"],
            &["--other"],
        ),
        (
            "decision out of range",
            &["--kind", "decision", "--option", "x"],
            &["--option", "5"],
        ),
        (
            "decision option 0",
            &["--kind", "decision", "--option", "x"],
            &["--option", "0"],
        ),
        (
            "two responses",
            &["--kind", "decision", "--option", "x"],
            &["--option", "1", "--ack"],
        ),
        (
            "drop with a response",
            &["--kind", "fyi"],
            &["--drop", "--ack"],
        ),
        ("approval missing row", &approval, &["--row", "a=approve"]),
        ("approval malformed row", &approval, &["--row", "approve"]),
        (
            "approval unknown verdict",
            &approval,
            &["--row", "a=approve", "--row", "b=maybe"],
        ),
        (
            "signoff changes needs a note",
            &["--kind", "signoff"],
            &["--changes"],
        ),
        (
            "question blank text",
            &["--kind", "question"],
            &["--text", "  "],
        ),
        (
            "question no response",
            &["--kind", "question"],
            &["--note", "hm"],
        ),
        ("question ack", &["--kind", "question"], &["--ack"]),
        ("fyi signoff", &["--kind", "fyi"], &["--accept"]),
        (
            "fyi send back",
            &["--kind", "fyi"],
            &["--send-back", "--note", "x"],
        ),
        (
            "signoff file",
            &["--kind", "signoff"],
            &["--accept", "--file", "1"],
        ),
        (
            "signoff send back",
            &["--kind", "signoff"],
            &["--send-back", "--note", "x"],
        ),
        (
            "briefing send back needs a note",
            BRIEFING,
            &["--send-back"],
        ),
        ("briefing changes", BRIEFING, &["--changes", "--note", "x"]),
        ("briefing file alone", BRIEFING, &["--file", "1"]),
        (
            "briefing file with drop",
            BRIEFING,
            &["--drop", "--file", "1"],
        ),
        (
            "briefing file past the items",
            BRIEFING,
            &["--accept", "--file", "3"],
        ),
        ("briefing file 0", BRIEFING, &["--accept", "--file", "0"]),
        (
            "briefing file twice",
            BRIEFING,
            &["--accept", "--file", "1", "--file", "1"],
        ),
        ("fyi no response", &["--kind", "fyi"], &["--note", "hm"]),
        ("park flag is gone", &["--kind", "stuck"], &["--park"]),
    ];
    for (name, open, answer) in cases {
        let env = Env::new();
        let (id, out) = open_and_answer(&env, open, answer);
        assert_eq!(out.code, 1, "{name}: {out:?}");
        eprintln!(
            "refused {name}: {}",
            out.stderr.lines().next().unwrap_or_default()
        );
        assert_eq!(env.case(&id).state(), State::Open, "{name}");
        assert_eq!(env.revision(&id), 1, "{name}");
    }
}

#[test]
fn answer_reads_its_text_from_a_file_or_stdin() {
    let env = Env::new();
    write_file(
        &env.path("reply.md"),
        "# The staging one\n\nIt has the fixtures.\n",
    );
    write_file(&env.path("empty.md"), "");
    let id = env.open(&["--kind", "question", "--urgency", "today", "--title", "T"]);
    for (stdin, args) in [
        ("", &["--text-file", "empty.md"][..]),
        ("", &["--text-file", "-"]),
        ("", &["--text-file", "missing.md"]),
        ("", &["--text", "x", "--text-file", "reply.md"]),
        ("", &["--text", "reply.md"]),
    ] {
        let mut all = vec!["answer", id.as_str()];
        all.extend(args);
        env.refused_with_stdin(&id, 1, stdin, &all);
    }
    env.ok(&["answer", &id, "--text-file", "reply.md"]);
    assert_eq!(
        env.case(&id).answer().unwrap().text,
        "# The staging one\n\nIt has the fixtures.\n"
    );

    let other = env.open(&["--kind", "stuck", "--urgency", "today", "--title", "S"]);
    let out = env.run_with_stdin(
        "use the mirror\n",
        env.store_args(&["answer", &other, "--text-file", "-"]),
    );
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(env.case(&other).answer().unwrap().text, "use the mirror\n");
}

#[test]
fn answer_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = env.open_decision();
    env.write(&id, "agent", "amend", r#"{"options": ["Vendor it"]}"#);
    env.refused_as_stale(
        &id,
        1,
        2,
        &["answer", &id, "--option", "3", "--revision", "1"],
    );
    env.refused_as_stale(
        &id,
        0,
        2,
        &["answer", &id, "--option", "3", "--revision", "0"],
    );
    let out = json(&env.ok(&["answer", &id, "--option", "3", "--revision", "2", "--json"]));
    assert_eq!(
        out,
        serde_json::json!({ "id": id, "state": "answered", "revision": 3 })
    );
}

#[test]
fn an_answer_carries_the_amend_it_saw() {
    let env = Env::new();
    let id = env.open_decision();
    env.write(&id, "agent", "amend", r#"{"options": ["Vendor it"]}"#);
    env.write(&id, "agent", "amend", r#"{"links": ["https://x"]}"#);
    env.ok(&["answer", &id, "--option", "3"]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["answer"]["amend_seq"], 2);
    assert_eq!(shown["amend_seq"], 2);
}

#[test]
fn answer_on_a_case_that_is_not_open_exits_3() {
    let env = Env::new();
    let id = env.open_decision();
    env.ok(&["withdraw", &id]);
    env.refused(&id, 3, &["answer", &id, "--option", "1"]);
}

#[test]
fn answer_takes_part_of_an_id_and_agent_commands_do_not() {
    let env = Env::new();
    let id = env.open_decision();
    assert_eq!(id, "UkLWZ");
    assert_eq!(
        env.ok(&["answer", "kLW", "--option", "1"]),
        format!("{id} answered\n")
    );
    let out = env.cases(&["pickup", "kLW"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(env.revision(&id), 2);
}

#[test]
fn the_human_is_named_by_as_then_the_config_file_then_the_login_name() {
    let mut env = Env::new();
    let name = |env: &Env, id: &str| {
        let shown = json(&env.ok(&["show", id, "--json"]));
        shown["events"][1]["actor"]["name"]
            .as_str()
            .map(str::to_owned)
    };
    let fyi = |env: &Env| env.open(&["--kind", "fyi", "--urgency", "today", "--title", "T"]);

    // No name at all: the unnamed human, with the kind but no name.
    let id = fyi(&env);
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(name(&env, &id), None);
    assert_eq!(
        json(&env.ok(&["show", &id, "--json"]))["events"][1]["actor"]["kind"],
        "human"
    );

    env.set("LOGNAME", "login");
    let id = fyi(&env);
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(name(&env, &id).as_deref(), Some("login"));
    env.set("USER", "user");
    let id = fyi(&env);
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(name(&env, &id).as_deref(), Some("user"));

    env.config("name = \"Ryan\"\n");
    let id = fyi(&env);
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(name(&env, &id).as_deref(), Some("Ryan"));
    assert!(env.ok(&["show", &id]).contains("by Ryan"));

    let id = fyi(&env);
    env.ok(&["answer", &id, "--ack", "--as", "Sam"]);
    assert_eq!(name(&env, &id).as_deref(), Some("Sam"));

    // One actor per name.
    let again = fyi(&env);
    env.ok(&["answer", &again, "--ack", "--as", "Sam"]);
    let actor =
        |id: &str| json(&env.ok(&["show", id, "--json"]))["events"][1]["actor"]["id"].clone();
    assert_eq!(actor(&id), actor(&again));
}

/// `for` names who a case is addressed to, and checks no one: another
/// human answers it as any human does (ST-12).
#[test]
fn a_case_for_someone_else_is_answered_by_whoever_answers() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "For Sam",
        "--for",
        "Sam",
    ]);
    assert_eq!(
        env.ok(&["answer", &id, "--ack", "--as", "Ryan"]),
        format!("{id} answered\n")
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["for"], "Sam");
    assert_eq!(shown["events"][1]["actor"]["name"], "Ryan");
}
