//! A decision case end to end, through the binary and the inbox's API, as
//! the agent and the human use it: open, a wait that picks up, an answer
//! over HTTP with the page's CSRF token, close, and show.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::io::Read as _;
use std::process::Stdio;

use common::{Env, json};

#[test]
fn a_decision_case_from_open_to_close_through_the_inbox() {
    let env = Env::new();
    // The agent opens the case, and waits for its answer in the background.
    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "blocking",
        "--worker",
        "bun-pins",
        "--title",
        "Pin bun or float?",
        "--option",
        "Pin to 1.2.3",
        "--option",
        "Float and fix the lockfile",
    ]);
    let mut wait = common::spawn(
        env.command(env.store_args(&[
            "wait",
            "--id",
            &id,
            "--since",
            &id,
            "--pickup",
            "--by",
            "bun-pins",
            "--timeout",
            "30s",
        ]))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()),
    );

    // The human answers from the inbox.
    let serve = env.serve(&["--as", "Ryan"]);
    let answered = serve.post(
        &format!("/v1/cases/{id}/answer"),
        r#"{"revision": 1, "answer": {"choice": 1, "note": "Revisit after 1.3", "amend_seq": 0}}"#,
    );
    assert_eq!(answered.status, 200, "{answered:?}");
    assert_eq!(answered.json()["state"], "answered");

    // The wait wakes with the answer, already picked up.
    let status = wait.wait().unwrap();
    let mut stdout = String::new();
    wait.stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(status.success(), "{status}: {stdout}");
    let line = json(stdout.trim());
    assert_eq!(line["id"], id.as_str());
    assert_eq!(line["state"], "pickedup");
    assert_eq!(line["answer"]["choice"], 1);
    assert_eq!(line["answer"]["note"], "Revisit after 1.3");
    let revision = line["revision"].as_u64().unwrap().to_string();
    assert_eq!(revision, "3");

    // The agent closes it at the revision the wait gave.
    let closed = json(&env.ok(&[
        "close",
        &id,
        "--outcome",
        "Pinned in #12.",
        "--revision",
        &revision,
        "--json",
    ]));
    assert_eq!(closed["state"], "closed");

    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "closed");
    assert_eq!(shown["close"]["outcome"], "Pinned in #12.");
    assert_eq!(
        shown["url"],
        format!("{}cases/{id}-pin-bun-or-float", serve.url)
    );
    let files: Vec<&str> = shown["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["file"].as_str().unwrap())
        .collect();
    assert_eq!(
        files,
        [
            "0001-agent-open.json",
            "0002-human-answer.json",
            "0003-agent-pickup.json",
            "0004-agent-close.json",
        ]
    );
    let names: Vec<&str> = shown["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["actor"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bun-pins", "Ryan", "bun-pins", "bun-pins"]);
    assert_eq!(serve.stop().code, 0);
}
