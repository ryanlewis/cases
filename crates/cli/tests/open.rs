//! `cases open` (AG-01, AG-02, CL-02, CL-03).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use cases_core::{State, Store};
use common::{Env, json, key, write_file};

#[test]
fn open_keeps_commas_in_options() {
    let env = Env::new();
    let id = env.open_decision();
    let case = env.case(&id);
    assert_eq!(case.options(), ["Pin to 1.2.3", "Float, with renovate"]);
    assert_eq!(case.state(), State::Open);
    assert_eq!(case.revision(), 1);
    assert_eq!(case.title(), "Pin bun?");
}

#[test]
fn open_prints_the_new_case_id_and_writes_one_open_event() {
    let env = Env::new();
    let out = env.cases(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Nightly",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, "UkLWZ\n");
    assert_eq!(out.stderr, "");
    let rows = env.db().rows(&key("UkLWZ")).unwrap().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].seq, rows[0].author.as_str(), rows[0].event.as_str()),
        (1, "agent", "open")
    );
    let data = json(&rows[0].data);
    assert!(data["opened_at"].as_str().unwrap().ends_with('Z'), "{data}");
}

#[test]
fn open_reads_the_body_from_a_file_or_stdin() {
    let env = Env::new();
    let file = env.path("body.md");
    write_file(&file, "# From a file\n");
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "File",
        "--body-file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(env.case(&id).body(), "# From a file\n");

    let args = env.store_args(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Stdin",
        "--body-file",
        "-",
        "--worker",
        "bun-pins",
        "--brief",
        "/briefs/bun.md",
        "--link",
        "https://a,b",
        "--context",
        "ctx",
    ]);
    let out = env.run_with_stdin("from stdin", args);
    assert_eq!(out.code, 0, "{out:?}");
    let case = env.case(out.stdout.trim());
    assert_eq!(case.body(), "from stdin");
    assert_eq!(case.worker(), "bun-pins");
    assert_eq!(case.brief(), "/briefs/bun.md");
    assert_eq!(case.context(), "ctx");
    assert_eq!(case.links(), ["https://a,b"]);
}

#[test]
fn open_takes_an_inline_body() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Inline",
        "--body",
        "Bun is pinned.",
    ]);
    assert_eq!(env.case(&id).body(), "Bun is pinned.");
}

#[test]
fn open_writes_approval_rows() {
    let env = Env::new();
    let id = env.open(&[
        "--kind", "approval", "--urgency", "blocking", "--title", "Scripts",
        "--row", r#"{"id":"deps","label":"Install deps","script":"npm ci --ignore-scripts\nnpm test","link":"https://example.com/deps"}"#,
        "--row", r#"{"id":"mig","label":"Migrate","note":"takes the site down for a minute","script":"make migrate","link":"https://example.com/mig"}"#,
    ]);
    let case = env.case(&id);
    assert_eq!(case.rows().len(), 2);
    assert_eq!(case.rows()[0].script, "npm ci --ignore-scripts\nnpm test");
    assert_eq!(case.rows()[1].id, "mig");

    // A row without a note is shown with no note key.
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert!(shown["rows"][0].get("note").is_none(), "{shown}");
    assert_eq!(shown["rows"][1]["note"], "takes the site down for a minute");
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("  [mig] Migrate\n      note: takes the site down for a minute\n      https://example.com/mig\n"),
        "{text}"
    );
    assert!(text.contains("  [deps] Install deps\n      https://example.com/deps\n      | npm ci --ignore-scripts\n      | npm test\n"), "{text}");
    assert_eq!(text.matches("note:").count(), 1, "{text}");
}

/// A row may say the human carries it out (case i2lDf); a row the agent
/// runs has no `by`.
#[test]
fn open_writes_a_row_the_human_carries_out() {
    let env = Env::new();
    let id = env.open(&[
        "--kind", "approval", "--urgency", "today", "--title", "Release",
        "--row", r#"{"id":"deps","label":"Install deps","script":"npm ci","link":"https://example.com/deps"}"#,
        "--row", r#"{"id":"dns","label":"Point DNS","script":"Point the name at the new host","link":"https://example.com/dns","by":"human"}"#,
    ]);
    let case = env.case(&id);
    assert_eq!(case.rows()[0].by, "");
    assert_eq!(case.rows()[1].by, "human");

    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert!(shown["rows"][0].get("by").is_none(), "{shown}");
    assert_eq!(shown["rows"][1]["by"], "human");
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("  [dns] Point DNS\n      by: human\n      https://example.com/dns\n"),
        "{text}"
    );
    assert!(
        text.contains("  [deps] Install deps\n      https://example.com/deps\n"),
        "{text}"
    );
}

#[test]
fn open_recommends_one_of_a_decisions_options() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "Pin bun?",
        "--option",
        "Pin to 1.2.3",
        "--option",
        "Float, with renovate",
        "--recommend",
        "2",
    ]);
    assert_eq!(env.case(&id).recommend(), 2);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["recommend"], 2);
    assert_eq!(shown["events"][0]["data"]["recommend"], 2);
    assert_eq!(
        json(&env.ok(&["list", "--json"]))["list"][0]["recommend"],
        2
    );
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains(
            "\nOptions:\n  1. Pin to 1.2.3\n  2. Float, with renovate (recommended)\n  other. Other, see note\n"
        ),
        "{text}"
    );
    // The thread starts from the open's recommend, so an amend that gives
    // the same one beside a link adds no recommended line.
    env.ok(&[
        "amend",
        &id,
        "--recommend",
        "2",
        "--link",
        "https://example.com/pr",
    ]);
    let text = env.ok(&["show", &id]);
    assert!(!text.contains("recommended 2."), "{text}");

    // A case that recommends none has no recommend in its JSON, and no mark.
    let none = env.open_decision();
    let shown = json(&env.ok(&["show", &none, "--json"]));
    assert!(shown.get("recommend").is_none(), "{shown}");
    assert!(
        shown["events"][0]["data"].get("recommend").is_none(),
        "{shown}"
    );
    assert!(!env.ok(&["show", &none]).contains("recommended"));
}

#[test]
fn open_writes_labels_and_the_brief() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "whenever",
        "--title",
        "Labelled",
        "--label",
        "feat-labels",
        "--label",
        "round 3, part 2",
        "--worker",
        "w1",
        "--brief",
        "Resume from LEDGER.md",
    ]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(
        shown["labels"],
        serde_json::json!(["feat-labels", "round 3, part 2"])
    );
    assert_eq!(shown["brief"], "Resume from LEDGER.md");
    let text = env.ok(&["show", &id]);
    assert!(
        text.contains("labels:   feat-labels, round 3, part 2\n"),
        "{text}"
    );
}

#[test]
fn open_takes_the_worker_and_one_label_from_the_environment() {
    let mut env = Env::new();
    let open = |env: &Env, extra: &[&str]| {
        let mut args = vec!["--kind", "fyi", "--urgency", "today", "--title", "Env"];
        args.extend(extra);
        env.case(&env.open(&args))
    };
    env.set("CASES_WORKER", "bun-pins");
    // The whole value is one label, commas and all.
    env.set("CASES_LABEL", "feat-labels, round 3");
    let case = open(&env, &[]);
    assert_eq!(case.worker(), "bun-pins");
    assert_eq!(case.labels(), ["feat-labels, round 3"]);

    // A flag replaces the environment's value; it does not add to it.
    let case = open(&env, &["--worker", "other", "--label", "a", "--label", "b"]);
    assert_eq!(case.worker(), "other");
    assert_eq!(case.labels(), ["a", "b"]);

    // Empty, or only white space, each counts as not given, as an empty
    // CASES_STORE does.
    for blank in ["", " "] {
        env.set("CASES_LABEL", blank);
        env.set("CASES_WORKER", blank);
        let case = open(&env, &[]);
        assert_eq!(case.labels(), [] as [&str; 0], "{blank:?}");
        assert_eq!(case.worker(), "", "{blank:?}");
    }
}

#[test]
fn open_records_the_worker_as_the_actor() {
    let env = Env::new();
    let first = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "One",
        "--worker",
        "w1",
    ]);
    let second = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Two",
        "--worker",
        "w1",
    ]);
    let unnamed = env.open_fyi("Three");
    let actor = |id: &str| env.case(id).actor().expect("an actor").id.clone();
    // One actor per worker name, and one unnamed agent.
    assert_eq!(actor(&first), actor(&second));
    assert_ne!(actor(&first), actor(&unnamed));
    let shown = json(&env.ok(&["show", &first, "--json"]));
    assert_eq!(shown["actor"]["kind"], "agent");
    assert_eq!(shown["actor"]["name"], "w1");
    assert_eq!(
        shown["events"][0]["data"]["actor"],
        serde_json::json!({ "id": actor(&first) })
    );
    assert_eq!(shown["worker"], "w1");
}

#[test]
fn open_json_prints_the_id_state_and_revision() {
    let env = Env::new();
    let out = env.ok(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "J",
        "--json",
    ]);
    assert_eq!(
        json(&out),
        serde_json::json!({ "id": "UkLWZ", "state": "open", "revision": 1 })
    );
}

#[test]
fn a_refused_open_writes_nothing_and_exits_1() {
    let base = ["--kind", "fyi", "--urgency", "today", "--title", "x"];
    let decision = [
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "x",
        "--option",
        "a",
    ];
    let row = r#"{"id":"a","label":"l","script":"s","link":"k"}"#;
    let cases: &[(&str, Vec<&str>)] = &[
        (
            "unknown kind",
            vec!["--kind", "poll", "--urgency", "today", "--title", "x"],
        ),
        (
            "a message, which only the human sends",
            vec![
                "--kind",
                "message",
                "--urgency",
                "today",
                "--title",
                "x",
                "--worker",
                "bun-pins",
            ],
        ),
        (
            "unknown urgency",
            vec!["--kind", "fyi", "--urgency", "soon", "--title", "x"],
        ),
        ("no kind", vec!["--urgency", "today", "--title", "x"]),
        ("no title", vec!["--kind", "fyi", "--urgency", "today"]),
        (
            "blank title",
            vec!["--kind", "fyi", "--urgency", "today", "--title", " "],
        ),
        (
            "decision without options",
            vec!["--kind", "decision", "--urgency", "today", "--title", "x"],
        ),
        (
            "approval without rows",
            vec!["--kind", "approval", "--urgency", "today", "--title", "x"],
        ),
        (
            "row that is not JSON",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                "deps=npm ci",
            ],
        ),
        (
            "row with an unknown field",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                r#"{"id":"a","label":"l","script":"s","link":"k","cmd":"x"}"#,
            ],
        ),
        (
            "row by someone other than the human",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                r#"{"id":"a","label":"l","script":"s","link":"k","by":"robot"}"#,
            ],
        ),
        (
            "two rows in one flag",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                r#"{"id":"a","label":"l","script":"s","link":"k"},{"id":"b","label":"l","script":"s","link":"k"}"#,
            ],
        ),
        (
            "two rows copied from an array",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                r#"{"id":"a","label":"l","script":"s","link":"k"}}, {"id":"b","label":"l","script":"s","link":"k"}"#,
            ],
        ),
        (
            "row without a link",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                r#"{"id":"a","label":"l","script":"s"}"#,
            ],
        ),
        (
            "the same row id twice",
            vec![
                "--kind",
                "approval",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                row,
                "--row",
                row,
            ],
        ),
        (
            "options on a question",
            vec![
                "--kind",
                "question",
                "--urgency",
                "today",
                "--title",
                "x",
                "--option",
                "a",
            ],
        ),
        (
            "rows on a question",
            vec![
                "--kind",
                "question",
                "--urgency",
                "today",
                "--title",
                "x",
                "--row",
                row,
            ],
        ),
        (
            "options on an fyi",
            vec![
                "--kind",
                "fyi",
                "--urgency",
                "today",
                "--title",
                "x",
                "--option",
                "a",
            ],
        ),
        (
            "blank label",
            [&base[..], &["--label", "a", "--label", " "]].concat(),
        ),
        (
            "the same label twice",
            [&base[..], &["--label", "a", "--label", "a"]].concat(),
        ),
        (
            "body and body file",
            [&base[..], &["--body", "x", "--body-file", "-"]].concat(),
        ),
        (
            "body that names a file",
            [&base[..], &["--body", "body.md"]].concat(),
        ),
        (
            "body file that is not there",
            [&base[..], &["--body-file", "missing.md"]].concat(),
        ),
        // AG-02: open is checked as strictly as amend.
        (
            "the same option twice",
            [&decision[..], &["--option", "a"]].concat(),
        ),
        ("blank link", [&base[..], &["--link", " "]].concat()),
        (
            "the same link twice",
            [&base[..], &["--link", "https://a", "--link", "https://a"]].concat(),
        ),
        ("blank worker", [&base[..], &["--worker", " "]].concat()),
        ("blank brief", [&base[..], &["--brief", " "]].concat()),
        ("blank context", [&base[..], &["--context", "\t"]].concat()),
        ("blank for", [&base[..], &["--for", " "]].concat()),
        // A decision's recommend names one of its options, and no other kind
        // takes one.
        (
            "a recommend on an fyi",
            [&base[..], &["--recommend", "1"]].concat(),
        ),
        (
            "a recommend past the options",
            [&decision[..], &["--recommend", "2"]].concat(),
        ),
        // The core reads 0 as no recommend, so the flag refuses it.
        (
            "a recommend of 0",
            [&decision[..], &["--recommend", "0"]].concat(),
        ),
        (
            "a negative recommend",
            [&decision[..], &["--recommend", "-1"]].concat(),
        ),
    ];
    for (name, args) in cases {
        let env = Env::new();
        write_file(&env.path("body.md"), "# Body\n");
        let mut all = vec!["open"];
        all.extend(args);
        let out = env.cases(&all);
        assert_eq!(out.code, 1, "{name}: {out:?}");
        assert_eq!(out.stdout, "", "{name}");
        assert!(!out.stderr.is_empty(), "{name}");
        assert!(!env.store.exists(), "{name}: a refused open made the store");
    }
}

#[test]
fn an_inline_body_that_names_a_file_says_to_use_the_file_flag() {
    let env = Env::new();
    write_file(&env.path("body.md"), "# Body\n");
    let out = env.cases(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "x",
        "--body",
        "body.md",
    ]);
    assert_eq!(
        out.stderr,
        "Error: --body \"body.md\" names a file; pass it with --body-file\n"
    );
}

#[test]
fn a_whitespace_body_is_accepted() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "x",
        "--body",
        "  ",
    ]);
    assert_eq!(env.case(&id).body(), "  ");
}

#[test]
fn open_stores_who_the_case_is_for() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "For",
        "--for",
        "Ryan",
    ]);
    assert_eq!(env.case(&id).for_(), "Ryan");
    assert_eq!(json(&env.ok(&["show", &id, "--json"]))["for"], "Ryan");
    assert!(env.ok(&["show", &id]).contains("for:      Ryan\n"));
}

/// An option numbered as the body's list is, or marked "(recommended)", is
/// opened as given, with a warning on stderr that names --recommend. Other
/// options open without one.
#[test]
fn open_warns_of_options_numbered_or_marked_as_the_pick() {
    let env = Env::new();
    let decision = |options: &[&str]| {
        let mut args = vec![
            "open",
            "--kind",
            "decision",
            "--urgency",
            "today",
            "--title",
            "Plate or slab?",
        ];
        for option in options {
            args.extend(["--option", option]);
        }
        env.cases(&args)
    };
    let out = decision(&["3 Split plate (recommended)", "Split slab", "2) Glass"]);
    assert_eq!(out.code, 0, "{out:?}");
    let id = out.stdout.trim();
    assert_eq!(
        env.case(id).options(),
        ["3 Split plate (recommended)", "Split slab", "2) Glass"]
    );
    let lines: Vec<&str> = out.stderr.lines().collect();
    assert_eq!(lines.len(), 2, "{}", out.stderr);
    assert!(
        lines[0].starts_with(
            "warning: option 1 (\"3 Split plate (recommended)\") starts with a number."
        ),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("warning: option 3 (\"2) Glass\") starts with a number."),
        "{}",
        lines[1]
    );
    assert!(
        lines.iter().all(|l| l.contains("--recommend N")),
        "{}",
        out.stderr
    );
    let marked = decision(&["Split plate (recommended)", "Split slab"]);
    assert_eq!(marked.code, 0, "{marked:?}");
    assert!(
        marked.stderr.starts_with(
            "warning: option 1 (\"Split plate (recommended)\") says \"(recommended)\"."
        ),
        "{}",
        marked.stderr
    );
    let plain = decision(&["Split plate", "Pin to 1.2.3"]);
    assert_eq!(plain.code, 0, "{plain:?}");
    assert_eq!(plain.stderr, "");
}

/// An `open --wait` hears its case's answer itself, so it is not warned
/// that the sessions listening on the label will not, nor told to open the
/// case again or wait on its id.
#[test]
fn open_wait_is_not_warned_of_listeners_its_answer_does_not_wake() {
    let env = Env::new();
    let open = |wait: bool| {
        let mut args = vec![
            "open",
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            "Runner",
            "--worker",
            "ci-runner",
            "--label",
            "homeday",
        ];
        if wait {
            args.extend(["--wait", "--timeout", "100ms"]);
        }
        env.cases(&args)
    };
    assert_eq!(open(false).code, 0);
    env.db()
        .listen("homeday", "homeday", &["homeday".to_owned()], &[], None)
        .unwrap();

    assert!(open(false).stderr.contains("wakes none of them"));
    let out = open(true);
    assert_eq!(out.code, 2, "{out:?}");
    assert!(!out.stderr.contains("wakes none of them"), "{out:?}");
}

#[test]
fn open_warns_when_its_answer_wakes_none_of_the_waits_on_its_label() {
    let env = Env::new();
    let fyi = |worker: &str, label: &str| {
        env.cases(&[
            "open",
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            "Runner",
            "--worker",
            worker,
            "--label",
            label,
        ])
    };
    // The store is made by the first case: a listener never makes one.
    let first = fyi("homeday", "homeday");
    assert_eq!((first.code, first.stderr.as_str()), (0, ""), "{first:?}");
    env.db()
        .listen(
            "homeday",
            "homeday",
            &["homeday".to_owned(), "email-triage".to_owned()],
            &[],
            None,
        )
        .unwrap();

    // The lead's label and its worker's name: the answer wakes nobody.
    let out = fyi("ci-runner", "homeday");
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        "warning: the sessions listening on label homeday (homeday) wait for --worker homeday, not ci-runner, so this case's answer wakes none of them. If homeday is your session, open it with --worker homeday; if not, wait on it with --id.\n"
    );
    let id = out.stdout.trim();
    assert_eq!(env.case(id).worker(), "ci-runner", "the case is written");

    // Its own worker name, or a label no one waits on, says nothing.
    for out in [fyi("homeday", "homeday"), fyi("ci-runner", "deps")] {
        assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    }

    // A worker's wait on no label beside its wait on another label takes
    // its every case: each wait is recorded on its own, so nothing is said.
    let db = env.db();
    db.listen("w1", "runner-2", &["ops".to_owned()], &[], None)
        .unwrap();
    db.listen("w2", "runner-2", &[], &[], None).unwrap();
    let out = fyi("runner-2", "homeday");
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");

    // The case's own worker waits on its label for messages alone: its
    // --kind leaves the answer out, and another --worker is no fix.
    db.listen(
        "w3",
        "runner-3",
        &["triage".to_owned()],
        &["message".to_owned()],
        None,
    )
    .unwrap();
    let out = fyi("runner-3", "triage");
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        "warning: the sessions listening on label triage (runner-3) wait for other kinds than fyi, so this case's answer wakes none of them. Wait on it with --id.\n"
    );

    // A label added by amend is checked the same way.
    let id = fyi("ci-runner", "deps").stdout.trim().to_owned();
    let out = env.cases(&["amend", &id, "--label", "email-triage"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        "warning: the sessions listening on label email-triage (homeday) wait for --worker homeday, not ci-runner, so this case's answer wakes none of them. Wait on it with --id.\n"
    );

    // A message the human labels is taken by its label: it has no answer.
    let message = env.ok(&[
        "message",
        "--as",
        "Ryan",
        "--title",
        "T",
        "--to",
        "ci-runner",
    ]);
    let out = env.cases(&[
        "amend",
        message.trim(),
        "--as",
        "Ryan",
        "--label",
        "homeday",
    ]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
}

/// `cases open` with `args` and `--wait`, started with its stdout and
/// stderr piped; gives it once the first line of stdout, the id, is out.
fn opening_and_waiting(env: &Env, args: &[&str]) -> (std::process::Child, String) {
    use std::io::{BufRead, BufReader};
    let mut all = vec!["open"];
    all.extend(args);
    all.push("--wait");
    let mut child = common::spawn(
        env.command(env.store_args(&all))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    );
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut id = String::new();
    stdout.read_line(&mut id).unwrap();
    // Put back what is left for the end, in order.
    child.stdout = Some(stdout.into_inner());
    (child, id.trim().to_owned())
}

/// The lines a finished `child` printed after its id, with its stderr and
/// exit code.
fn rest(child: std::process::Child) -> (Vec<serde_json::Value>, String, i32) {
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    (
        stdout.lines().map(json).collect(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap(),
    )
}

/// `line` with its case's id, and the times that differ between two cases
/// answered one after the other, left out.
fn same_but_when(line: &serde_json::Value) -> serde_json::Value {
    let mut line = line.clone();
    let fields = line.as_object_mut().unwrap();
    fields.remove("id");
    fields.remove("next_since");
    fields["answer"]
        .as_object_mut()
        .unwrap()
        .remove("answered_at");
    line
}

const DECISION: &[&str] = &[
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
    "--worker",
    "bun-pins",
];

/// `open --wait` prints the id first, before the case is answered, and
/// then, once it is, the line `cases wait --id ID --since ID --pickup --by
/// WORKER` prints after a plain open: the case picked up by its worker.
/// It ends as that wait does, exit 0, with the same stderr.
#[test]
fn open_wait_prints_the_id_then_what_the_chained_wait_prints() {
    let env = Env::new();
    let (waiting, id) = opening_and_waiting(&env, DECISION);
    // The id is out while the case is still open, before any answer.
    assert_eq!(env.case(&id).state(), State::Open);
    env.ok(&["answer", &id, "--option", "1", "--as", "Ryan"]);
    let (lines, stderr, code) = rest(waiting);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["id"], id.as_str());
    assert_eq!(lines[0]["state"], "pickedup");
    let case = env.case(&id);
    assert_eq!(case.pickup().unwrap().by, "bun-pins");

    // The two commands, chained as the skill teaches them.
    let mut open = vec!["open"];
    open.extend(DECISION);
    let chained = env.ok(&open).trim().to_owned();
    let mut wait = env.command(env.store_args(&[
        "wait", "--id", &chained, "--since", &chained, "--pickup", "--by", "bun-pins",
    ]));
    let waiting = common::spawn(
        wait.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    );
    env.ok(&["answer", &chained, "--option", "1", "--as", "Ryan"]);
    let (theirs, their_stderr, their_code) = rest(waiting);
    assert_eq!((code, stderr.as_str()), (their_code, their_stderr.as_str()));
    assert_eq!(
        lines.iter().map(same_but_when).collect::<Vec<_>>(),
        theirs.iter().map(same_but_when).collect::<Vec<_>>()
    );
    assert_eq!(
        lines[0]["next_since"],
        env.case(&id).events()[1].at().unwrap().to_string().as_str()
    );
}

/// With no worker, `open --wait` picks the answer up by no name, as `cases
/// wait --pickup` given no `--by` does.
#[test]
fn open_wait_with_no_worker_picks_up_by_no_name() {
    let env = Env::new();
    let args = ["--kind", "fyi", "--urgency", "today", "--title", "Seen?"];
    let (waiting, id) = opening_and_waiting(&env, &args);
    env.ok(&["answer", &id, "--ack"]);
    let (lines, stderr, code) = rest(waiting);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines[0]["state"], "pickedup");
    assert_eq!(env.case(&id).pickup().unwrap().by, "");
}

/// A `--wait` that times out has printed the id already, and ends as `cases
/// wait` does at its timeout: one line on stderr, exit 2. The case stays
/// open.
#[test]
fn open_wait_that_times_out_leaves_the_id_on_stdout_and_exits_2() {
    let env = Env::new();
    let mut args = vec!["open"];
    args.extend(DECISION);
    args.extend(["--wait", "--timeout", "100ms"]);
    let out = env.cases(&args);
    assert_eq!(out.code, 2, "{out:?}");
    assert_eq!(out.stdout, "UkLWZ\n");
    assert_eq!(out.stderr, "no case needed the agent within 100ms\n");
    assert_eq!(env.case("UkLWZ").state(), State::Open);
}

/// `--timeout` is the wait's, so an open without `--wait` refuses it: exit
/// 1, and nothing written. Without `--wait`, open is as it was.
#[test]
fn open_refuses_a_timeout_without_wait_and_writes_nothing() {
    let env = Env::new();
    let mut args = vec!["open"];
    args.extend(DECISION);
    args.extend(["--timeout", "1s"]);
    let out = env.cases(&args);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--wait"), "{out:?}");
    assert!(!env.store.exists());
}

/// `--json` prints a record over several lines, and the wait's lines are
/// one each, so `--wait` refuses it: exit 1, and nothing written.
#[test]
fn open_refuses_json_with_wait_and_writes_nothing() {
    let env = Env::new();
    let mut args = vec!["open"];
    args.extend(DECISION);
    args.extend(["--json", "--wait"]);
    let out = env.cases(&args);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--json"), "{out:?}");
    assert!(!env.store.exists());
}

/// A config file whose mood the wait would refuse is refused before the
/// case is opened: exit 1, and no case left with no wait on it.
#[test]
fn open_wait_refuses_a_bad_mood_before_it_opens_the_case() {
    let env = Env::new();
    env.config("mood = \"sleepy\"\n");
    let mut args = vec!["open"];
    args.extend(DECISION);
    args.extend(["--wait", "--timeout", "100ms"]);
    let out = env.cases(&args);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("\"sleepy\""), "{out:?}");
    assert!(!env.store.exists());
}
