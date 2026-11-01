//! A briefing through the CLI: `open --kind briefing`, `answer --accept` and
//! `--send-back` with `--file`, `amend` of `landed` and the left-out items,
//! `show` and the `wait` line (AG-19, AG-04, AG-07, AG-13, HU-01).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, json};
use serde_json::{Value, json as value};

/// Opens a briefing labelled `bun-pins`, with two left-out items and `extra`.
fn open_briefing(env: &Env, extra: &[&str]) -> String {
    let mut args = vec![
        "--kind",
        "briefing",
        "--urgency",
        "blocking",
        "--title",
        "Pinned the bun version",
        "--body",
        "## What changed\n\nBun is pinned.",
        "--left-out",
        "Bump the lockfile",
        "--left-out",
        "Pin node too",
        "--label",
        "bun-pins",
    ];
    args.extend(extra);
    env.open(&args)
}

#[test]
fn open_writes_a_briefing_s_landed_and_left_out_items() {
    let env = Env::new();
    let gated = open_briefing(&env, &[]);
    let shown = json(&env.ok(&["show", &gated, "--json"]));
    assert_eq!(shown["kind"], "briefing");
    assert_eq!(shown["landed"], false);
    assert_eq!(
        shown["left_out"],
        value!(["Bump the lockfile", "Pin node too"])
    );
    // A gated briefing stores no landed: false is the field left out.
    assert_eq!(
        shown["events"][0]["data"].get("landed"),
        None,
        "{}",
        shown["events"][0]
    );

    let landed = open_briefing(&env, &["--landed"]);
    let shown = json(&env.ok(&["show", &landed, "--json"]));
    assert_eq!(shown["landed"], true);
    assert_eq!(shown["events"][0]["data"]["landed"], true);

    // Another kind has neither field in show --json.
    let fyi = env.open_fyi("Nightly build moved");
    let shown = json(&env.ok(&["show", &fyi, "--json"]));
    assert_eq!(shown.get("landed"), None);
    assert_eq!(shown.get("left_out"), None);

    let text = env.ok(&["show", &gated]);
    for want in [
        "landed:   waiting to land",
        "Left out:\n  1. Bump the lockfile\n  2. Pin node too\n",
    ] {
        assert!(text.contains(want), "show lacks {want:?}:\n{text}");
    }
    assert!(env.ok(&["show", &landed]).contains("landed:   landed\n"));
}

#[test]
fn a_refused_briefing_open_writes_nothing() {
    let env = Env::new();
    let base = ["--urgency", "today", "--title", "T"];
    let refused: &[(&str, &[&str])] = &[
        ("no body", &["--kind", "briefing"]),
        ("a blank body", &["--kind", "briefing", "--body", "  "]),
        (
            "an option",
            &["--kind", "briefing", "--body", "B", "--option", "x"],
        ),
        (
            "a blank item",
            &["--kind", "briefing", "--body", "B", "--left-out", " "],
        ),
        (
            "an item twice",
            &[
                "--kind",
                "briefing",
                "--body",
                "B",
                "--left-out",
                "x",
                "--left-out",
                "x",
            ],
        ),
        ("landed on an fyi", &["--kind", "fyi", "--landed"]),
        (
            "left out on a signoff",
            &["--kind", "signoff", "--left-out", "x"],
        ),
    ];
    for (name, args) in refused {
        let mut all = vec!["open"];
        all.extend(base);
        all.extend(*args);
        let out = env.cases(&all);
        assert_eq!(out.code, 1, "{name}: {out:?}");
        assert_eq!(out.stdout, "", "{name}");
    }
    let long = "a".repeat(301);
    let out = env.cases(&[
        "open",
        "--kind",
        "briefing",
        "--urgency",
        "today",
        "--title",
        "T",
        "--body",
        "B",
        "--left-out",
        &long,
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    let eleven: Vec<String> = (1..=11).map(|n| format!("Item {n}")).collect();
    let mut args = vec![
        "open",
        "--kind",
        "briefing",
        "--urgency",
        "today",
        "--title",
        "T",
        "--body",
        "B",
    ];
    for item in &eleven {
        args.extend(["--left-out", item]);
    }
    assert_eq!(env.cases(&args).code, 1);
    assert_eq!(json(&env.ok(&["list", "--json"]))["list"], value!([]));
}

#[test]
fn a_briefing_sent_back_is_amended_with_reopen_and_judged_again() {
    let env = Env::new();
    let id = open_briefing(&env, &[]);
    env.ok(&[
        "answer",
        &id,
        "--send-back",
        "--note",
        "Show the diff.",
        "--file",
        "1",
    ]);
    env.ok(&["pickup", &id]);

    // Without --reopen, the amend is refused, as a signoff's is.
    let error = env.refused(&id, 3, &["amend", &id, "--body", "Now with the diff."]);
    assert!(error.contains("--reopen"), "{error}");

    let out = env.ok(&[
        "amend",
        &id,
        "--reopen",
        "--body",
        "Now with the diff.",
        "--left-out",
        "Pin node too",
        "--landed",
    ]);
    assert_eq!(out, format!("{id} open\n"));
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "open");
    assert_eq!(shown["landed"], true);
    assert_eq!(shown["left_out"], value!(["Pin node too"]));
    assert_eq!(shown["answer"], Value::Null);
    assert_eq!(shown["amend_seq"], 4);

    // The new list numbers the answer's files.
    env.refused(&id, 1, &["answer", &id, "--accept", "--file", "2"]);
    env.ok(&["answer", &id, "--accept", "--file", "1"]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["answer"]["briefing"], "accept", "{}", shown["answer"]);
    assert_eq!(shown["answer"]["file"], value!([1]));
    assert_eq!(shown["answer"]["amend_seq"], 4);

    let text = env.ok(&["show", &id]);
    for want in [
        "briefing: send back",
        "file 1. Bump the lockfile",
        "reopened the case",
        "replaced the left-out items: Pin node too",
        "       landed\n",
        "briefing: accept",
        "file 1. Pin node too",
    ] {
        assert!(text.contains(want), "show lacks {want:?}:\n{text}");
    }
}

#[test]
fn amend_sets_and_clears_landed() {
    let env = Env::new();
    let id = open_briefing(&env, &["--landed"]);
    // The landed the case has changes nothing.
    env.refused(&id, 1, &["amend", &id, "--landed"]);
    env.ok(&["amend", &id, "--not-landed"]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["landed"], false);
    assert_eq!(shown["events"][1]["data"]["landed"], false);
    // landed does not change the question.
    assert_eq!(shown["amend_seq"], 0);
    let out = env.cases(&["amend", &id, "--landed", "--not-landed"]);
    assert_eq!(out.code, 1, "{out:?}");
    // On another kind, either is refused.
    let fyi = env.open_fyi("Nightly build moved");
    env.refused(&fyi, 1, &["amend", &fyi, "--not-landed"]);
    env.refused(&fyi, 1, &["amend", &fyi, "--left-out", "x"]);
}

/// The session that opened a landed briefing may be gone: a later wait on
/// its label picks the answer up, and its line carries the items the
/// answer's `file` numbers (AG-04, AG-19).
#[test]
fn a_label_listener_picks_up_a_landed_briefing_s_answer_with_its_items() {
    let env = Env::new();
    let id = open_briefing(&env, &["--landed"]);
    env.ok(&["answer", &id, "--accept", "--file", "2"]);
    let out = env.ok(&[
        "wait",
        "--pickup",
        "--label",
        "bun-pins",
        "--by",
        "lead",
        "--since",
        &id,
        "--timeout",
        "20s",
    ]);
    let line: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(line["id"], id.as_str());
    assert_eq!(line["state"], "pickedup");
    assert_eq!(line["kind"], "briefing");
    assert_eq!(line["landed"], true);
    assert_eq!(
        line["left_out"],
        value!(["Bump the lockfile", "Pin node too"])
    );
    assert_eq!(line["answer"]["briefing"], "accept");
    assert_eq!(line["answer"]["file"], value!([2]));
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["pickup"]["by"], "lead");

    // Another kind's line has neither field.
    let fyi = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "F",
        "--label",
        "l2",
    ]);
    env.ok(&["answer", &fyi, "--ack"]);
    let out = env.ok(&["wait", "--label", "l2", "--since", &fyi, "--timeout", "20s"]);
    let line: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(line.get("landed"), None, "{line}");
    assert_eq!(line.get("left_out"), None, "{line}");
}
