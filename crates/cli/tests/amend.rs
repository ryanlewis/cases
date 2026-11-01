//! `cases amend` (AG-07, AG-12, CL-02, CL-03).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, json, write_file};

#[test]
fn amend_adds_to_the_case_and_replaces_its_body_and_context() {
    let env = Env::new();
    let id = env.open_approval();
    let args = env.store_args(&[
        "amend", &id, "--body-file", "-",
        "--row", r#"{"id":"mig","label":"Migrate","script":"make migrate","link":"https://example.com/mig"}"#,
        "--link", "https://example.com/a,b", "--context", "Release 1.4, then 1.4.1",
    ]);
    let out = env.run_with_stdin("Two scripts now.\n", args);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{id} open\n"));

    // show --json has the amended fields on the case, and the open event as
    // it was written beside the amend.
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "open");
    assert_eq!(shown["body"], "Two scripts now.\n");
    assert_eq!(shown["context"], "Release 1.4, then 1.4.1");
    assert_eq!(shown["rows"][1]["id"], "mig");
    assert_eq!(
        shown["links"],
        serde_json::json!(["https://example.com/a,b"])
    );
    let events = shown["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["file"], "0002-agent-amend.json");
    assert!(events[0]["data"].get("body").is_none());
    assert_eq!(events[0]["data"]["context"], "Release 1.4");
    assert_eq!(events[0]["data"]["rows"].as_array().unwrap().len(), 1);
    // The amend's record holds only what changed. Its body is the case's
    // body, so it is given once, there, and the event says so.
    let amend = events[1]["data"].as_object().unwrap();
    let mut fields: Vec<&String> = amend.keys().collect();
    fields.sort();
    assert_eq!(fields, ["actor", "amended_at", "context", "links", "rows"]);
    assert_eq!(events[1]["body_in_case"], true);
    assert!(events[0].get("body_in_case").is_none());

    let text = env.ok(&["show", &id]);
    for want in [
        "Two scripts now.",
        "[mig] Migrate",
        "0002 agent amend",
        "replaced the body",
        "added row [mig] Migrate",
        "added link: https://example.com/a,b",
        "replaced the context: Release 1.4, then 1.4.1",
    ] {
        assert!(text.contains(want), "show lacks {want:?}:\n{text}");
    }

    // The answer needs a verdict on the added row too, and an answered case
    // takes no amend.
    let out = env.cases(&["answer", &id, "--row", "deps=approve"]);
    assert_eq!(out.code, 1, "{out:?}");
    env.ok(&["answer", &id, "--row", "deps=approve", "--row", "mig=hold"]);
    env.refused(
        &id,
        3,
        &["amend", &id, "--link", "https://example.com/late"],
    );
}

/// An amend may add a row the human carries out, and is refused one whose
/// `by` is not `human` (case i2lDf).
#[test]
fn amend_adds_a_row_the_human_carries_out() {
    let env = Env::new();
    let id = env.open_approval();
    let robot = env.cases(&[
        "amend", &id,
        "--row", r#"{"id":"dns","label":"Point DNS","script":"Point the name","link":"https://example.com/dns","by":"robot"}"#,
    ]);
    assert_eq!(robot.code, 1, "{robot:?}");
    env.ok(&[
        "amend", &id,
        "--row", r#"{"id":"dns","label":"Point DNS","script":"Point the name","link":"https://example.com/dns","by":"human"}"#,
    ]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["rows"][1]["id"], "dns");
    assert_eq!(shown["rows"][1]["by"], "human");
}

#[test]
fn amend_takes_an_inline_body() {
    let env = Env::new();
    let id = env.open_approval();
    assert_eq!(
        env.ok(&["amend", &id, "--body", "One script."]),
        format!("{id} open\n")
    );
    assert_eq!(env.case(&id).body(), "One script.");
}

#[test]
fn amend_adds_options_that_an_answer_can_choose() {
    let env = Env::new();
    let id = env.open_decision();
    env.ok(&["amend", &id, "--option", "Vendor it, for now"]);
    assert_eq!(
        env.case(&id).options(),
        ["Pin to 1.2.3", "Float, with renovate", "Vendor it, for now"]
    );
    assert_eq!(
        env.ok(&["answer", &id, "--option", "3"]),
        format!("{id} answered\n")
    );
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("added option: Vendor it, for now")
            && text.contains("chose 3. Vendor it, for now"),
        "{text}"
    );
}

#[test]
fn amend_changes_the_recommend_and_leaves_the_amend_seq() {
    let env = Env::new();
    let id = env.open_decision();
    // A recommend may name an option the same amend adds.
    env.ok(&["amend", &id, "--option", "Vendor it", "--recommend", "3"]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["recommend"], 3);
    assert_eq!(shown["amend_seq"], 2);
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("       added option: Vendor it\n       recommended 3. Vendor it\n"),
        "{text}"
    );
    assert!(text.contains("  3. Vendor it (recommended)\n"), "{text}");

    // A new recommend is advice about the question, not the question: it
    // leaves the amend seq where the added option put it (ST-04).
    env.ok(&["amend", &id, "--recommend", "1"]);
    let case = env.case(&id);
    assert_eq!((case.recommend(), case.amend_seq()), (1, 2));
    assert!(
        env.ok(&["show", &id])
            .contains("recommended 1. Pin to 1.2.3")
    );

    // The recommend the case has changes nothing, and one past the options
    // names none.
    env.refused(&id, 1, &["amend", &id, "--recommend", "1"]);
    env.refused(&id, 1, &["amend", &id, "--recommend", "4"]);
    // The flag refuses a number below 1 itself, a negative one after a space
    // too.
    for n in ["0", "-1"] {
        let err = env.refused(&id, 1, &["amend", &id, "--recommend", n]);
        assert!(err.contains("options are numbered from 1"), "{err}");
    }

    // An amend that leaves the recommend out, or gives the one the case has
    // beside a label, adds no recommended line to the thread.
    env.ok(&["amend", &id, "--link", "https://example.com/log"]);
    env.ok(&["amend", &id, "--recommend", "1", "--label", "later"]);
    let text = env.ok(&["show", &id]);
    assert_eq!(text.matches("recommended ").count(), 2, "{text}");
}

#[test]
fn amend_adds_labels() {
    let env = Env::new();
    let id = env.open_approval();
    env.ok(&["amend", &id, "--label", "round 3", "--label", "a,b"]);
    assert_eq!(env.case(&id).labels(), ["release", "round 3", "a,b"]);
    assert!(env.ok(&["show", &id]).contains("added label: round 3"));
}

#[test]
fn amend_at_a_revision_the_case_has_left_is_refused_as_stale() {
    let env = Env::new();
    let id = env.open_approval();
    env.ok(&["amend", &id, "--link", "https://example.com/a"]);
    env.refused_as_stale(
        &id,
        1,
        2,
        &["amend", &id, "--body", "Replaced.", "--revision", "1"],
    );
    let out = json(&env.ok(&[
        "amend",
        &id,
        "--body",
        "Replaced.",
        "--revision",
        "2",
        "--json",
    ]));
    assert_eq!(
        out,
        serde_json::json!({ "id": id, "state": "open", "revision": 3 })
    );
}

#[test]
fn amend_records_the_worker_as_the_actor() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "T",
        "--worker",
        "w1",
    ]);
    let case = env.write(&id, "agent", "note", r#"{"body": "x"}"#);
    let _ = case;
    env.ok(&["amend", &id, "--context", "c"]);
    let case = env.case(&id);
    let actors: Vec<_> = case
        .events()
        .iter()
        .map(|e| e.actor().unwrap().id.clone())
        .collect();
    // The amend is the open's actor, the worker's; the note written with no
    // name is the unnamed agent's.
    assert_eq!(actors[2], actors[0]);
    assert_ne!(actors[1], actors[0]);
}

#[test]
fn a_refused_amend_writes_nothing() {
    let missing = "missing.md";
    let cases: &[(&str, &str, &[&str])] = &[
        ("nothing to amend", "", &[]),
        ("blank body", " \n", &["--body-file", "-"]),
        // The core would take an empty body as no change and add the link.
        (
            "empty body file with a link",
            "",
            &["--body-file", "-", "--link", "https://example.com/log"],
        ),
        (
            "empty inline body with a link",
            "",
            &["--body", "", "--link", "https://example.com/log"],
        ),
        ("blank inline body", "", &["--body", " "]),
        ("inline body that names a file", "", &["--body", "body.md"]),
        (
            "body and body file",
            "x",
            &["--body", "x", "--body-file", "-"],
        ),
        ("missing body file", "", &["--body-file", missing]),
        (
            "body file with no name",
            "",
            &["--body-file", "", "--link", "https://example.com/log"],
        ),
        ("options on an approval", "", &["--option", "Skip it"]),
        ("row that is not JSON", "", &["--row", "mig=make migrate"]),
        (
            "row id already on the case",
            "",
            &[
                "--row",
                r#"{"id":"deps","label":"Again","script":"npm ci","link":"https://example.com/deps"}"#,
            ],
        ),
        (
            "row with a stray bracket",
            "",
            &[
                "--row",
                r#"{"id":"mig","label":"Migrate","script":"make migrate","link":"https://example.com/mig"}] {"id":"x"}"#,
            ],
        ),
        // The core would take an empty context as no change and add the link.
        (
            "empty context with a link",
            "",
            &["--context", "", "--link", "https://example.com/log"],
        ),
        ("blank link", "", &["--link", " "]),
        (
            "the context the case has",
            "",
            &["--context", "Release 1.4"],
        ),
        (
            "the same link twice",
            "",
            &[
                "--link",
                "https://example.com/log",
                "--link",
                "https://example.com/log",
            ],
        ),
        ("blank label", "", &["--label", ""]),
        ("blank title", "", &["--title", " "]),
        ("the title the case has", "", &["--title", "Scripts"]),
        (
            "a dropped link the case does not have",
            "",
            &["--drop-link", "https://example.com/deps"],
        ),
        ("reopen an open case", "", &["--reopen", "--context", "x"]),
        ("label already on the case", "", &["--label", "release"]),
        ("a recommend on an approval", "", &["--recommend", "1"]),
        // The core reads 0 as no recommend, so the flag refuses it.
        (
            "a recommend of 0 with a link",
            "",
            &["--recommend", "0", "--link", "https://example.com/log"],
        ),
    ];
    for (name, stdin, args) in cases {
        let env = Env::new();
        write_file(&env.path("body.md"), "# Body\n");
        let id = env.open_approval();
        let mut all = vec!["amend", id.as_str()];
        all.extend(*args);
        let stderr = env.refused_with_stdin(&id, 1, stdin, &all);
        assert!(!stderr.is_empty(), "{name}");
    }
    let env = Env::new();
    env.open_approval();
    for id in ["../etc", "nope", "gbHJd", "kLW"] {
        let out = env.cases(&["amend", id, "--context", "x"]);
        assert_eq!(out.code, 1, "amend {id}: {out:?}");
    }
}

/// After a signoff answered `changes`, the agent makes them, then one amend
/// with `--reopen` points the case at the new work: a new title and body, the
/// old link dropped and the new one added (cases `6M6gx`, `AV8s1`).
#[test]
fn amend_reopens_a_picked_up_signoff_with_a_new_title_and_links() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "signoff",
        "--urgency",
        "today",
        "--title",
        "Release notes, draft one",
        "--body",
        "Draft one.",
        "--link",
        "https://example.com/notes/1",
    ]);
    env.write(
        &id,
        "human",
        "answer",
        r#"{"signoff": "changes", "note": "Say what broke."}"#,
    );
    env.ok(&["pickup", &id]);

    // Without --reopen, the amend is refused, and says how.
    let error = env.refused(
        &id,
        3,
        &["amend", &id, "--title", "Release notes, draft two"],
    );
    assert!(error.contains("--reopen"), "{error}");
    // A link the case does not have is refused, with --reopen too.
    env.refused(
        &id,
        1,
        &[
            "amend",
            &id,
            "--reopen",
            "--drop-link",
            "https://example.com/notes/9",
        ],
    );
    // The human's edit does not reopen a case.
    let out = env.cases(&["amend", &id, "--reopen", "--as", "Ryan", "--body", "x"]);
    assert_eq!(out.code, 1, "{out:?}");

    let out = env.ok(&[
        "amend",
        &id,
        "--reopen",
        "--title",
        "Release notes, draft two",
        "--body",
        "Draft two.",
        "--drop-link",
        "https://example.com/notes/1",
        "--link",
        "https://example.com/notes/2",
    ]);
    assert_eq!(out, format!("{id} open\n"));
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "open");
    assert_eq!(shown["title"], "Release notes, draft two");
    assert_eq!(shown["body"], "Draft two.");
    assert_eq!(
        shown["links"],
        serde_json::json!(["https://example.com/notes/2"])
    );
    assert_eq!(shown["answer"], serde_json::Value::Null);
    assert_eq!(shown["amend_seq"], 4);
    let amend = &shown["events"][3]["data"];
    assert_eq!(amend["reopen"], true);
    assert_eq!(
        amend["drop_links"],
        serde_json::json!(["https://example.com/notes/1"])
    );
    let text = env.ok(&["show", &id]);
    for want in [
        "reopened the case",
        "replaced the title: Release notes, draft two",
        "dropped link: https://example.com/notes/1",
        "added link: https://example.com/notes/2",
        "signoff: changes",
    ] {
        assert!(text.contains(want), "show lacks {want:?}:\n{text}");
    }
    // The human answers the case as amended.
    env.write(
        &id,
        "human",
        "answer",
        r#"{"signoff": "accept", "amend_seq": 4}"#,
    );
    assert_eq!(env.case(&id).state(), cases_core::State::Answered);
}

/// An option an amend adds is warned of as open warns of it, by the number
/// it takes after the case's options.
#[test]
fn amend_warns_of_an_added_option_numbered_as_a_list_item() {
    let env = Env::new();
    let id = env.open_decision();
    let out = env.cases(&[
        "amend",
        &id,
        "--option",
        "Wait a week",
        "--option",
        "4. Drop it",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(env.case(&id).options().len(), 4);
    assert!(
        out.stderr
            .starts_with("warning: option 4 (\"4. Drop it\") starts with a number."),
        "{}",
        out.stderr
    );
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
    let quiet = env.cases(&["amend", &id, "--option", "Ask again"]);
    assert_eq!(quiet.stderr, "");
}
