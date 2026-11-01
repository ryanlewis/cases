//! Messages from the human: `cases message`, the human's edit and withdraw
//! of one, what the agent's commands do on one, `cases open --re`, and
//! `cases doing` (HU-08, AG-16, AG-17). How `cases wait` takes a message is
//! in `wait.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, json};
use serde_json::Value;

/// Sends a message titled "Rebase onto main" as Ryan, with `args`, and gives
/// its id.
fn message(env: &Env, args: &[&str]) -> String {
    let mut all = vec!["message", "--as", "Ryan", "--title", "Rebase onto main"];
    all.extend(args);
    env.ok(&all).trim().to_owned()
}

/// The case `id` as `show --json` gives it.
fn shown(env: &Env, id: &str) -> Value {
    json(&env.ok(&["show", "--json", id]))
}

#[test]
fn a_message_goes_to_a_worker_or_a_label_as_the_human() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins", "--body", "Then run the tests."]);
    let case = shown(&env, &id);
    for (field, want) in [
        ("kind", "message"),
        ("state", "sent"),
        ("urgency", "today"),
        ("worker", "bun-pins"),
        ("title", "Rebase onto main"),
        ("body", "Then run the tests."),
    ] {
        assert_eq!(case[field], want, "{field}: {case}");
    }
    assert!(case.get("labels").is_none(), "{case}");
    let open = &case["events"][0];
    assert_eq!(
        (&open["author"], &open["event"]),
        (&"human".into(), &"open".into())
    );
    assert_eq!(
        (&open["actor"]["kind"], &open["actor"]["name"]),
        (&"human".into(), &"Ryan".into())
    );

    let id = message(
        &env,
        &[
            "--label",
            "release",
            "--label",
            "docs",
            "--urgency",
            "blocking",
        ],
    );
    let case = shown(&env, &id);
    assert_eq!(case["labels"], serde_json::json!(["release", "docs"]));
    assert!(case.get("worker").is_none(), "{case}");
    assert_eq!(case["urgency"], "blocking");

    // The body from a file, with links and context.
    let file = env.path("body.md");
    std::fs::write(&file, "Rebase, then **test**.\n").unwrap();
    let id = message(
        &env,
        &[
            "--to",
            "bun-pins",
            "--body-file",
            file.to_str().unwrap(),
            "--link",
            "https://example.com/pr/1",
            "--context",
            "main moved",
        ],
    );
    let case = shown(&env, &id);
    assert_eq!(case["body"], "Rebase, then **test**.\n");
    assert_eq!(
        case["links"],
        serde_json::json!(["https://example.com/pr/1"])
    );
    assert_eq!(case["context"], "main moved");

    // A message is never on the human's own list: that is open and parked.
    assert_eq!(env.ok(&["list", "--count"]).trim(), "0");
    assert_eq!(env.ok(&["list", "--state", "sent", "--count"]).trim(), "3");
    let json_out = env.ok(&[
        "message", "--as", "Ryan", "--title", "T", "--to", "w", "--json",
    ]);
    let written = json(&json_out);
    assert_eq!(
        (&written["state"], &written["revision"]),
        (&"sent".into(), &1.into())
    );
}

#[test]
fn a_message_with_no_title_takes_its_first_line_as_the_title() {
    let env = Env::new();
    let note = "## Rebase onto *main*\n\nThen run the tests.";
    let id = env.ok(&[
        "message", "--as", "Ryan", "--to", "bun-pins", "--body", note,
    ]);
    let case = shown(&env, id.trim());
    assert_eq!(case["title"], "Rebase onto main", "{case}");
    assert_eq!(case["body"], note, "{case}");
}

#[test]
fn a_message_needs_who_it_goes_to_and_the_human_s_name() {
    let env = Env::new();
    for (args, want) in [
        (
            vec!["--as", "Ryan", "--title", "T"],
            "a message goes to a worker or a label",
        ),
        (vec!["--to", "w", "--title", "T"], "--as"),
        (
            vec!["--as", "", "--to", "w", "--title", "T"],
            "the human's message needs a name",
        ),
        (
            vec!["--as", "  ", "--to", "w", "--title", "T"],
            "the human's message needs a name",
        ),
        (
            vec!["--as", "Ryan", "--to", "w", "--title", " "],
            "the title is blank",
        ),
        (
            vec!["--as", "Ryan", "--to", " ", "--title", "T"],
            "a message goes to a worker or a label",
        ),
        (
            vec!["--as", "Ryan", "--to", "w", "--title", "T", "--re", "zzzzz"],
            "no case",
        ),
        (
            vec!["--as", "Ryan", "--to", "w"],
            "give --title, or a body whose first line can be the title",
        ),
        (
            vec!["--as", "Ryan", "--to", "w", "--body=> **"],
            "give --title, or a body whose first line can be the title",
        ),
    ] {
        let mut all = vec!["message"];
        all.extend(&args);
        let out = env.cases(&all);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert!(out.stderr.contains(want), "{args:?}: {}", out.stderr);
    }
    assert!(!env.store.exists(), "a refused message made the store");
}

#[test]
fn a_message_says_when_no_session_is_listening_for_it() {
    let env = Env::new();
    let out = env.cases(&[
        "message", "--as", "Ryan", "--title", "T", "--to", "bun-pins", "--label", "release",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        "warning: no session is listening for bun-pins or label release now: the message waits until one is\n"
    );
    // A session listening on one of its labels would take it.
    env.db()
        .listen("lead", "lead", &["release".to_owned()], &[], None)
        .unwrap();
    let out = env.cases(&[
        "message", "--as", "Ryan", "--title", "T", "--label", "release",
    ]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    env.db()
        .listen("bun-pins", "bun-pins", &[], &[], None)
        .unwrap();
    let out = env.cases(&[
        "message", "--as", "Ryan", "--title", "T", "--to", "bun-pins",
    ]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
}

#[test]
fn a_message_is_about_a_case_named_by_part_of_its_id() {
    let env = Env::new();
    let about = env.open_fyi("Nightly build moved");
    let id = message(&env, &["--to", "w", "--re", &about[1..]]);
    assert_eq!(shown(&env, &id)["re"], about.as_str());
    let text = env.ok(&["show", &id]);
    assert!(text.contains(&format!("re:       {about}\n")), "{text}");
    assert!(text.contains("to:       w\n"), "{text}");
    assert!(text.contains("       sent to w\n"), "{text}");
}

#[test]
fn the_human_edits_a_message_until_it_is_picked_up() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins", "--body", "Rebase."]);
    let out = env.cases(&[
        "amend",
        &id[..3],
        "--as",
        "Ryan",
        "--body",
        "Rebase, then test.",
        "--json",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out.stdout)["state"], "sent");
    let case = shown(&env, &id);
    assert_eq!(case["body"], "Rebase, then test.");
    assert_eq!(case["events"][1]["author"], "human");
    // The agent's amend of a message is refused for its author, and the
    // human's of the agent's case too: exit 1, nothing written.
    let refused = env.cases(&["amend", &id, "--body", "Mine now."]);
    assert_eq!(refused.code, 1, "{refused:?}");
    let theirs = env.open_fyi("Nightly build moved");
    let refused = env.cases(&["amend", &theirs, "--as", "Ryan", "--context", "x"]);
    assert_eq!(refused.code, 1, "{refused:?}");
    let blank = env.cases(&["amend", &id, "--as", " ", "--body", "x"]);
    assert!(
        blank.stderr.contains("the human's edit needs a name"),
        "{blank:?}"
    );
    // Once an agent picks it up, it stays as it was: exit 3.
    env.ok(&["pickup", &id, "--by", "bun-pins"]);
    let late = env.cases(&["amend", &id, "--as", "Ryan", "--body", "Too late."]);
    assert_eq!(late.code, 3, "{late:?}");
    assert_eq!(env.revision(&id), 3);
}

#[test]
fn the_human_takes_back_a_message_no_agent_has_picked_up() {
    let env = Env::new();
    let id = message(&env, &["--to", "bun-pins"]);
    // Not the agent's to withdraw: it is never open.
    assert_eq!(env.cases(&["withdraw", &id]).code, 3);
    let out = env.ok(&["withdraw", &id, "--as", "Ryan", "--reason", "done by hand"]);
    assert_eq!(out.trim(), format!("{id} withdrawn"));
}

/// The agent's events on a message are the session's that picked it up, and
/// it takes no note: a question goes in a case of its own, about it.
#[test]
fn an_agent_takes_a_message_and_closes_it_as_itself() {
    let env = Env::new();
    let id = message(&env, &["--label", "release"]);
    env.ok(&["pickup", &id, "--by", "lead-2"]);
    let noted = env.cases(&["note", &id, "--body", "Which branch?"]);
    assert_eq!(noted.code, 3, "{noted:?}");
    let question = env.open(&[
        "--kind",
        "question",
        "--urgency",
        "today",
        "--title",
        "Which branch?",
        "--worker",
        "lead-2",
        "--re",
        &id,
    ]);
    assert_eq!(shown(&env, &question)["re"], id.as_str());
    env.ok(&[
        "close",
        &id,
        "--revision",
        "2",
        "--outcome",
        "Rebased in abc123.",
    ]);
    let case = shown(&env, &id);
    assert_eq!(case["state"], "closed");
    for event in [&case["events"][1], &case["events"][2]] {
        assert_eq!(event["actor"]["name"], "lead-2", "{event}");
        assert_eq!(event["actor"]["kind"], "agent", "{event}");
    }
}

#[test]
fn open_re_names_a_case_by_its_whole_id() {
    let env = Env::new();
    let about = env.open_fyi("Nightly build moved");
    let base = [
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "T",
        "--re",
    ];
    for re in ["zzzzz", &about[1..]] {
        let mut all = vec!["open"];
        all.extend(base);
        all.push(re);
        let out = env.cases(&all);
        assert_eq!(out.code, 1, "{re}: {out:?}");
    }
    let mut all = vec!["open"];
    all.extend(base);
    let slugged = format!("{about}-nightly-build-moved");
    all.push(&slugged);
    let id = env.ok(&all).trim().to_owned();
    assert_eq!(shown(&env, &id)["re"], about.as_str());
}

#[test]
fn doing_sets_the_line_of_a_listening_worker() {
    let mut env = Env::new();
    // No store: no one listens.
    let out = env.cases(&["doing", "rebasing", "--worker", "bun-pins"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("\"bun-pins\" is not listening"),
        "{out:?}"
    );
    env.open_fyi("A case, so that the store is there");
    let out = env.cases(&["doing", "rebasing", "--worker", "bun-pins"]);
    assert_eq!(out.code, 1, "{out:?}");

    env.db()
        .listen("bun-pins", "bun-pins", &[], &[], None)
        .unwrap();
    let doing = |env: &Env| env.db().listeners().unwrap()[0].doing.clone();
    assert_eq!(
        env.ok(&["doing", "  rebasing onto main ", "--worker", "bun-pins"]),
        ""
    );
    assert_eq!(doing(&env).as_deref(), Some("rebasing onto main"));
    env.set("CASES_WORKER", "bun-pins");
    env.ok(&["doing", "running the tests"]);
    assert_eq!(doing(&env).as_deref(), Some("running the tests"));
    for text in ["a\nb", &"x".repeat(81)] {
        let out = env.cases(&["doing", text]);
        assert_eq!(out.code, 1, "{text:?}: {out:?}");
    }
    assert_eq!(doing(&env).as_deref(), Some("running the tests"));
    env.ok(&["doing", ""]);
    assert_eq!(doing(&env), None);
    env.unset("CASES_WORKER");
    let out = env.cases(&["doing", "x"]);
    assert!(out.stderr.contains("doing needs --worker"), "{out:?}");
}
