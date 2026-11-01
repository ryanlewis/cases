//! `cases park` and `cases resume` (HU-03, HU-04, HU-05, AG-12).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
use cases_core::State;
use common::{Env, json};

#[test]
fn park_and_resume_a_stuck_case() {
    let env = Env::new();
    let id = env.open_stuck();
    env.refused(&id, 3, &["resume", &id]);
    assert_eq!(
        env.ok(&["park", &id, "--note", "after release", "--as", "Ryan"]),
        format!("{id} parked\n")
    );
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "parked");
    assert_eq!(shown["park"]["note"], "after release");
    assert!(shown["park"]["parked_at"].is_string());
    assert_eq!(shown["events"][1]["file"], "0002-human-park.json");
    assert_eq!(shown["events"][1]["actor"]["name"], "Ryan");
    // A parked case is listed by default.
    assert!(env.ok(&["list"]).contains(&id));
    env.refused(&id, 3, &["park", &id]);

    assert_eq!(env.ok(&["resume", &id]), format!("{id} open\n"));
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["state"], "open");
    assert!(shown.get("park").is_none(), "{shown}");
    assert_eq!(shown["events"][2]["file"], "0003-human-resume.json");
    assert!(shown["events"][2]["data"]["resumed_at"].is_string());
}

#[test]
fn only_a_stuck_case_parks() {
    let env = Env::new();
    for open in [
        &["--kind", "question"][..],
        &["--kind", "decision", "--option", "x"],
        &["--kind", "fyi"],
    ] {
        let mut args = vec!["--urgency", "today", "--title", "T"];
        args.extend(open);
        let id = env.open(&args);
        env.refused(&id, 3, &["park", &id]);
    }
}

#[test]
fn resume_takes_no_agent_flag() {
    let env = Env::new();
    let id = env.open_stuck();
    env.ok(&["park", &id]);
    let out = env.cases(&["resume", &id, "--agent"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(env.case(&id).state(), State::Parked);
}

#[test]
fn park_and_resume_at_a_revision_the_case_has_left_are_refused_as_stale() {
    let env = Env::new();
    let id = env.open_stuck();
    env.write(&id, "agent", "amend", r#"{"context": "CI is red"}"#);
    env.refused_as_stale(&id, 1, 2, &["park", &id, "--revision", "1"]);
    let out = json(&env.ok(&[
        "park",
        &id,
        "--note",
        "after release",
        "--revision",
        "2",
        "--json",
    ]));
    assert_eq!(
        out,
        serde_json::json!({ "id": id, "state": "parked", "revision": 3 })
    );
    // A resume and a second park elsewhere leave the case parked, as it was
    // read, but at a later revision.
    env.ok(&["resume", &id]);
    env.ok(&["park", &id]);
    env.refused_as_stale(&id, 3, 5, &["resume", &id, "--revision", "3"]);
    assert_eq!(
        env.ok(&["resume", &id, "--revision", "5"]),
        format!("{id} open\n")
    );
    assert_eq!(env.revision(&id), 6);
}

#[test]
fn park_and_resume_take_part_of_an_id() {
    let env = Env::new();
    let id = env.open_stuck();
    assert_eq!(env.ok(&["park", "kLW"]), format!("{id} parked\n"));
    assert_eq!(env.ok(&["resume", "kLW"]), format!("{id} open\n"));
}

#[test]
fn park_and_resume_record_the_human_who_wrote_them() {
    let env = Env::new();
    let id = env.open_stuck();
    env.ok(&["park", &id, "--as", "Ryan"]);
    env.ok(&["resume", &id, "--as", "Sam"]);
    let shown = json(&env.ok(&["show", &id, "--json"]));
    assert_eq!(shown["events"][1]["actor"]["name"], "Ryan");
    assert_eq!(shown["events"][2]["actor"]["name"], "Sam");
    assert_eq!(shown["events"][2]["actor"]["kind"], "human");
}

#[test]
fn resume_prints_json() {
    let env = Env::new();
    let id = env.open_stuck();
    env.ok(&["park", &id]);
    let resumed = json(&env.ok(&["resume", &id, "--json"]));
    assert_eq!(
        resumed,
        serde_json::json!({ "id": id, "state": "open", "revision": 3 })
    );
}
