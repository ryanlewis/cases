//! `cases sweep`, and the human's withdraw of one case, `cases withdraw
//! --as` (ST-14).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
use cases_core::State;
use common::{Env, from_now, json};

#[test]
fn a_dry_run_writes_nothing() {
    let env = Env::new();
    let id = env.open_decision();
    let out = env.cases(&["sweep"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{id} would be withdrawn: Pin bun?\n"));
    assert_eq!(
        out.stderr,
        "1 case would be withdrawn. Pass --yes to withdraw them.\n"
    );
    assert_eq!(env.revision(&id), 1);
}

/// A dry run's hint about `--in-flight` is singular when only one case with
/// an agent was left.
#[test]
fn a_dry_run_hints_in_flight_singular_for_one_case() {
    let env = Env::new();
    let answered = env.open_decision();
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let out = env.cases(&["sweep"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        "1 case with an agent was left; --in-flight withdraws them too.\n"
    );
}

#[test]
fn sweep_withdraws_open_cases_as_the_human_and_leaves_the_rest() {
    let env = Env::new();
    let open = env.open_decision();
    let answered = env.open_decision();
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let parked = env.open(&["--kind", "stuck", "--urgency", "today", "--title", "Stuck"]);
    env.write(&parked, "human", "park", "{}");
    let closed = env.open_decision();
    env.write(&closed, "human", "answer", r#"{"choice": 1}"#);
    env.write(&closed, "agent", "pickup", "{}");
    env.write(&closed, "agent", "close", r#"{"outcome": "done"}"#);
    // A message waits on an agent: the human takes it back with withdraw.
    let sent = env
        .ok(&[
            "message", "--as", "Ryan", "--to", "bun-pins", "--title", "Rebase",
        ])
        .trim()
        .to_owned();

    let out = env.ok(&[
        "sweep",
        "--yes",
        "--reason",
        "clearing the inbox",
        "--as",
        "Ryan",
    ]);
    for want in [
        format!("{open} withdrawn\n"),
        format!("{answered} answered, left\n"),
        format!("{parked} parked, left\n"),
    ] {
        assert!(out.contains(&want), "sweep --yes lacks {want:?}:\n{out}");
    }
    assert!(!out.contains(&closed) && !out.contains(&sent), "{out}");
    let shown = json(&env.ok(&["show", &open, "--json"]));
    assert_eq!(shown["state"], "withdrawn");
    let withdraw = &shown["events"][1];
    assert_eq!(withdraw["file"], "0002-human-withdraw.json");
    assert_eq!(withdraw["data"]["reason"], "clearing the inbox");
    assert_eq!(withdraw["actor"]["kind"], "human");
    assert_eq!(withdraw["actor"]["name"], "Ryan");
    for (id, state) in [
        (&answered, State::Answered),
        (&parked, State::Parked),
        (&closed, State::Closed),
        (&sent, State::Sent),
    ] {
        assert_eq!(env.case(id).state(), state, "{id}");
    }

    // The default reason, and a second sweep has nothing to withdraw.
    let other = env.open_decision();
    env.ok(&["sweep", "-y"]);
    assert_eq!(json(&env.case(&other).events()[1].data)["reason"], "swept");
    assert!(!env.ok(&["sweep", "--yes"]).contains("withdrawn"));
}

#[test]
fn sweep_filters_by_label_worker_kind_and_the_age_of_the_last_event() {
    let env = Env::new();
    let old = env.open_at(
        "Old",
        &from_now(-48 * 3600),
        r#", "labels": ["round-1"], "worker": "w1""#,
    );
    let recent = env.open_at(
        "Recent",
        &from_now(-3600),
        r#", "labels": ["round-1"], "worker": "w1""#,
    );
    let other = env.open_at(
        "Other",
        &from_now(-48 * 3600),
        r#", "labels": ["round-2"], "worker": "w2""#,
    );
    // Opened long ago, amended an hour ago: its last event is recent.
    let amended = env.open_at(
        "Amended",
        &from_now(-48 * 3600),
        r#", "labels": ["round-3"]"#,
    );
    let data = format!(
        r#"{{"links": ["https://x"], "amended_at": "{}"}}"#,
        from_now(-3600)
    );
    env.write(&amended, "agent", "amend", &data);
    let decision = env.open_decision();

    let all = [&old, &recent, &other, &amended, &decision];
    let cases: &[(&[&str], Vec<&String>)] = &[
        (&[], all.to_vec()),
        (&["--label", "round-1"], vec![&old, &recent]),
        (&["--worker", "w2"], vec![&other]),
        (&["--kind", "decision"], vec![&decision]),
        (&["--older-than", "24h"], vec![&old, &other]),
        (&["--older-than", "24h", "--label", "round-1"], vec![&old]),
        (&["--label", "none"], vec![]),
    ];
    for (args, want) in cases {
        let mut all_args = vec!["sweep"];
        all_args.extend(*args);
        let out = env.ok(&all_args);
        for id in all {
            assert_eq!(
                out.contains(id.as_str()),
                want.contains(&id),
                "sweep {args:?}:\n{out}"
            );
        }
    }
    for args in [&["--kind", "decison"][..], &["--older-than", "soon"]] {
        let mut all_args = vec!["sweep"];
        all_args.extend(args);
        assert_eq!(env.cases(&all_args).code, 1, "{args:?}");
    }

    env.ok(&[
        "sweep",
        "--older-than",
        "24h",
        "--label",
        "round-1",
        "--yes",
    ]);
    assert_eq!(env.case(&old).state(), State::Withdrawn);
    assert_eq!(env.case(&recent).state(), State::Open);
    assert_eq!(env.case(&other).state(), State::Open);
}

#[test]
fn sweep_on_a_store_that_is_not_there_does_nothing() {
    let env = Env::new();
    assert_eq!(env.ok(&["sweep", "--yes"]), "");
    assert!(!env.store.exists());
}

/// A withdraw that cannot be made, here because another writer holds the
/// store's lock for longer than a write waits, is left; sweep names it and
/// exits 1, as any failure but a transition or a stale write does (ST-14,
/// CL-01).
#[test]
fn sweep_names_what_it_could_not_withdraw_and_exits_1() {
    let env = Env::new();
    let id = env.open_decision();
    let conn = rusqlite::Connection::open(&env.store).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let out = env.cases(&["sweep", "--yes"]);
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr
            .starts_with("Error: 1 case was not withdrawn:\n  "),
        "{out:?}"
    );
    assert!(out.stderr.contains(&format!("{id}: store ")), "{out:?}");
    assert!(out.stderr.contains("is locked"), "{out:?}");
    assert_eq!(env.case(&id).state(), State::Open);
}

/// A decision case opened by the worker `worker`, and gives its id.
fn open_by(env: &Env, worker: &str) -> String {
    env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "today",
        "--title",
        "Pin bun?",
        "--option",
        "Pin",
        "--worker",
        worker,
    ])
}

/// With --in-flight, sweep also withdraws the answered and picked-up cases
/// that match, as the human, and they keep their answer and pickup. A dry
/// run names their state, and who picked one up, since an agent may still be
/// at work on one, and it may be another session than the worker that
/// opened it. A parked case is still left, and without the flag only open
/// cases go (ST-14, case `CmGSr`).
#[test]
fn sweep_in_flight_withdraws_the_cases_agents_have_too() {
    let env = Env::new();
    let open = open_by(&env, "gone");
    let answered = open_by(&env, "gone");
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let picked = open_by(&env, "gone");
    env.write(&picked, "human", "answer", r#"{"choice": 1}"#);
    env.write(&picked, "agent", "pickup", r#"{"by": "gone"}"#);
    // The dead session's case that a live one picked up, as a lead picks up
    // its workers' cases.
    let taken = open_by(&env, "gone");
    env.write(&taken, "human", "answer", r#"{"choice": 1}"#);
    env.write(&taken, "agent", "pickup", r#"{"by": "alive"}"#);
    let parked = env.open(&[
        "--kind",
        "stuck",
        "--urgency",
        "today",
        "--title",
        "Stuck",
        "--worker",
        "gone",
    ]);
    env.write(&parked, "human", "park", "{}");
    // Another session's case, picked up, which the filter leaves alone.
    let alive = open_by(&env, "alive");
    env.write(&alive, "human", "answer", r#"{"choice": 1}"#);
    env.write(&alive, "agent", "pickup", r#"{"by": "alive"}"#);

    let out = env.cases(&["sweep", "--worker", "gone"]);
    assert_eq!(
        out.stdout,
        format!(
            "{answered} answered, left\n{picked} pickedup by gone, left\n\
             {taken} pickedup by alive, left\n{parked} parked, left\n\
             {open} would be withdrawn: Pin bun?\n"
        )
    );
    assert_eq!(
        out.stderr,
        "1 case would be withdrawn. Pass --yes to withdraw them.\n\
         3 cases with an agent were left; --in-flight withdraws them too.\n"
    );
    let out = env.cases(&["sweep", "--worker", "gone", "--in-flight"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "{parked} parked, left\n{open} would be withdrawn: Pin bun?\n\
             {answered} answered, would be withdrawn: Pin bun?\n\
             {picked} pickedup by gone, would be withdrawn: Pin bun?\n\
             {taken} pickedup by alive, would be withdrawn: Pin bun?\n"
        )
    );
    assert_eq!(
        out.stderr,
        "4 cases would be withdrawn. Pass --yes to withdraw them.\n"
    );
    for (id, revision) in [
        (&open, 1),
        (&answered, 2),
        (&picked, 3),
        (&taken, 3),
        (&parked, 2),
    ] {
        assert_eq!(env.revision(id), revision, "a dry run wrote to {id}");
    }

    let out = env.ok(&[
        "sweep",
        "--worker",
        "gone",
        "--in-flight",
        "--yes",
        "--reason",
        "its session died",
        "--as",
        "Ryan",
    ]);
    assert_eq!(
        out,
        format!(
            "{parked} parked, left\n{open} withdrawn\n{answered} withdrawn\n{picked} withdrawn\n\
             {taken} withdrawn\n"
        )
    );
    let shown = json(&env.ok(&["show", &picked, "--json"]));
    assert_eq!(shown["state"], "withdrawn");
    assert_eq!(shown["answer"]["choice"], 1);
    assert_eq!(shown["pickup"]["by"], "gone");
    let withdraw = &shown["events"][3];
    assert_eq!(withdraw["file"], "0004-human-withdraw.json");
    assert_eq!(withdraw["data"]["reason"], "its session died");
    assert_eq!(withdraw["actor"]["kind"], "human");
    assert_eq!(withdraw["actor"]["name"], "Ryan");
    for (id, state) in [
        (&open, State::Withdrawn),
        (&answered, State::Withdrawn),
        (&taken, State::Withdrawn),
        (&parked, State::Parked),
        (&alive, State::PickedUp),
    ] {
        assert_eq!(env.case(id).state(), state, "{id}");
    }
}

/// `withdraw --as` is the human's withdraw of one case: an open one, or one
/// an agent has, answered or picked up, such as one whose session died. It
/// is written as the human, with a reason if given and the revision guard,
/// and the case keeps its answer and pickup (ST-14, cases `Q8PA4` and `YjXET`).
#[test]
fn withdraw_as_the_human_ends_a_case_an_agent_has() {
    let env = Env::new();
    let open = env.open_decision();
    let answered = env.open_decision();
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let picked = env.open_decision();
    env.write(&picked, "human", "answer", r#"{"choice": 2}"#);
    env.write(&picked, "agent", "pickup", r#"{"by": "bun-pins"}"#);

    assert_eq!(
        env.ok(&["withdraw", &open, "--as", "Ryan"]),
        format!("{open} withdrawn\n")
    );
    // The agent's withdraw of an open case prints the same: the event says
    // whose it is.
    let shown = json(&env.ok(&["show", &open, "--json"]));
    let withdraw = &shown["events"][1];
    assert_eq!(withdraw["file"], "0002-human-withdraw.json");
    assert_eq!(withdraw["actor"]["kind"], "human");
    assert_eq!(withdraw["actor"]["name"], "Ryan");
    assert_eq!(
        env.ok(&["withdraw", &answered, "--as", "Ryan"]),
        format!("{answered} withdrawn\n")
    );
    env.refused_as_stale(
        &picked,
        2,
        3,
        &["withdraw", &picked, "--as", "Ryan", "--revision", "2"],
    );
    let out = json(&env.ok(&[
        "withdraw",
        &picked,
        "--as",
        "Ryan",
        "--reason",
        "its session died",
        "--revision",
        "3",
        "--json",
    ]));
    assert_eq!(
        out,
        serde_json::json!({ "id": picked, "state": "withdrawn", "revision": 4 })
    );

    let shown = json(&env.ok(&["show", &picked, "--json"]));
    assert_eq!(shown["answer"]["choice"], 2);
    assert_eq!(shown["pickup"]["by"], "bun-pins");
    let withdraw = &shown["events"][3];
    assert_eq!(withdraw["file"], "0004-human-withdraw.json");
    assert_eq!(withdraw["data"]["reason"], "its session died");
    assert_eq!(withdraw["actor"]["kind"], "human");
    assert_eq!(withdraw["actor"]["name"], "Ryan");
    let answer = json(&env.ok(&["show", &answered, "--answer"]));
    assert_eq!(answer["state"], "withdrawn");
    assert_eq!(answer["answer"]["choice"], 1);
    let text = env.ok(&["show", &picked]);
    assert!(
        text.contains("0004 human withdraw") && text.contains("reason: its session died\n"),
        "{text}"
    );
    // Prune takes it as it takes any withdrawn case (ST-13).
    let out = env.ok(&["prune", "--age", "0", "--state", "withdrawn"]);
    assert!(
        out.contains(&format!("{picked} withdrawn, would be archived\n")),
        "{out}"
    );
}

/// The human's withdraw is refused, and writes nothing, on a case that is
/// closed, withdrawn or parked. Without --as, the withdraw is the agent's,
/// which only an open case takes, and a blank --as is refused (ST-14, AG-11).
#[test]
fn withdraw_as_the_human_leaves_a_case_that_is_done_or_parked() {
    let env = Env::new();
    let closed = env.open_decision();
    env.write(&closed, "human", "answer", r#"{"choice": 1}"#);
    env.write(&closed, "agent", "pickup", "{}");
    env.write(&closed, "agent", "close", r#"{"outcome": "Pinned."}"#);
    let withdrawn = env.open_decision();
    env.write(&withdrawn, "agent", "withdraw", "{}");
    let parked = env.open_stuck();
    env.write(&parked, "human", "park", "{}");
    for id in [&closed, &withdrawn, &parked] {
        env.refused(id, 3, &["withdraw", id, "--as", "Ryan"]);
    }

    let answered = env.open_decision();
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let picked = env.open_decision();
    env.write(&picked, "human", "answer", r#"{"choice": 1}"#);
    env.write(&picked, "agent", "pickup", "{}");
    for id in [&answered, &picked] {
        env.refused(id, 3, &["withdraw", id]);
    }
    // A blank name would make it the human's withdraw, written under the
    // config file's name or the login name, which the caller never gave.
    for name in ["", "  "] {
        env.refused(&picked, 1, &["withdraw", &picked, "--as", name]);
    }
}

/// The human's withdraw takes part of an id, as the human's other writes do;
/// the agent's takes only a whole one (HU-07).
#[test]
fn withdraw_as_the_human_takes_part_of_an_id() {
    let env = Env::new();
    let id = env.open_decision();
    assert!(id.contains("kLW") && id != "kLW", "{id}");
    assert_eq!(env.cases(&["withdraw", "kLW"]).code, 1);
    assert_eq!(
        env.ok(&["withdraw", "kLW", "--as", "Ryan"]),
        format!("{id} withdrawn\n")
    );
}

/// Opens a briefing titled `title` for the worker `bun-pins`, accepts it,
/// and with `picked_up` picks it up that many seconds ago. Gives its id.
fn briefing(env: &Env, title: &str, picked_up: Option<i64>) -> String {
    let id = env.open(&[
        "--kind",
        "briefing",
        "--urgency",
        "today",
        "--title",
        title,
        "--body",
        "Done.",
        "--worker",
        "bun-pins",
    ]);
    env.write(&id, "human", "answer", r#"{"briefing": "accept"}"#);
    if let Some(ago) = picked_up {
        let at = format!(r#"{{"picked_up_at": "{}"}}"#, from_now(-ago));
        env.write(&id, "agent", "pickup", &at);
    }
    id
}

const DAY: i64 = 24 * 3600;

/// `--stale-briefings` closes as expired the briefings picked up 7 days ago
/// or more, by the unnamed agent, with the outcome saying why, and leaves a
/// younger one, an answered one not picked up, and a case of another kind
/// (AG-19).
#[test]
fn stale_briefings_are_closed_expired_after_7_days_picked_up() {
    let env = Env::new();
    let old = briefing(&env, "Old", Some(8 * DAY));
    let young = briefing(&env, "Young", Some(6 * DAY));
    let answered = briefing(&env, "Answered", None);
    let other = env.open_fyi("Heads up");
    env.write(&other, "human", "answer", r#"{"ack": true}"#);
    let at = format!(r#"{{"picked_up_at": "{}"}}"#, from_now(-30 * DAY));
    env.write(&other, "agent", "pickup", &at);

    let out = env.cases(&["sweep", "--stale-briefings"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!("{old} pickedup, would be closed: Old\n")
    );
    assert_eq!(
        out.stderr,
        "1 briefing would be closed. Pass --yes to close them.\n"
    );
    assert_eq!(env.case(&old).state(), State::PickedUp);

    let out = env.ok(&["sweep", "--stale-briefings", "--yes"]);
    assert_eq!(out, format!("{old} closed\n"));
    let shown = json(&env.ok(&["show", &old, "--json"]));
    assert_eq!(shown["state"], "closed");
    assert_eq!(shown["close"]["outcome"], "closed after 7 days picked up");
    assert_eq!(shown["close"]["expired"], true);
    let close = &shown["events"][3];
    assert_eq!(close["author"], "agent");
    assert_eq!(close["event"], "close");
    assert_eq!(close["data"]["expired"], true);
    // The thread shows the outcome, as any close's.
    let text = env.ok(&["show", &old]);
    assert!(text.contains("closed after 7 days picked up"), "{text}");
    assert_eq!(env.case(&young).state(), State::PickedUp);
    assert_eq!(env.case(&answered).state(), State::Answered);
    assert_eq!(env.case(&other).state(), State::PickedUp);
}

/// A pickup wait of the briefing's worker, waiting while it is closed as
/// expired, does not print it: nothing the human did is news to the agent.
#[test]
fn an_expired_close_wakes_no_wait() {
    let env = Env::new();
    let old = briefing(&env, "Old", Some(8 * DAY));
    let mut wait = env.command(env.store_args(&[
        "wait",
        "--pickup",
        "--worker",
        "bun-pins",
        "--timeout",
        "3s",
    ]));
    let waiting = common::spawn(
        wait.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    );
    std::thread::sleep(std::time::Duration::from_millis(800));
    env.ok(&["sweep", "--stale-briefings", "--yes"]);
    let out = waiting.wait_with_output().unwrap();
    assert_eq!(env.case(&old).state(), State::Closed);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains(&old), "{stdout}");
}

/// The config file's briefing-close-after sets the time, in days or hours,
/// and off, or 0, turns it off; a value that is neither is refused, naming
/// the key.
#[test]
fn the_config_file_sets_when_briefings_are_closed_or_turns_it_off() {
    let env = Env::new();
    let two = briefing(&env, "Two days", Some(2 * DAY));
    let fifteen = briefing(&env, "Fifteen days", Some(15 * DAY));
    assert_eq!(
        env.ok(&["sweep", "--stale-briefings"]),
        format!("{fifteen} pickedup, would be closed: Fifteen days\n")
    );

    for off in ["off", "0"] {
        env.config(&format!("briefing-close-after = \"{off}\"\n"));
        let out = env.cases(&["sweep", "--stale-briefings", "--yes"]);
        assert_eq!(out.code, 0, "{out:?}");
        assert_eq!(out.stdout, "");
        assert_eq!(
            out.stderr,
            "briefing-close-after is off: no briefing is closed.\n"
        );
    }

    env.config("briefing-close-after = \"2w\"\n");
    let out = env.cases(&["sweep", "--stale-briefings"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("briefing-close-after"), "{out:?}");

    env.config("briefing-close-after = \"14d\"\n");
    assert_eq!(
        env.ok(&["sweep", "--stale-briefings", "--yes"]),
        format!("{fifteen} closed\n")
    );
    let shown = json(&env.ok(&["show", &fifteen, "--json"]));
    assert_eq!(shown["close"]["outcome"], "closed after 14 days picked up");

    env.config("briefing-close-after = \"36h\"\n");
    assert_eq!(
        env.ok(&["sweep", "--stale-briefings", "--yes"]),
        format!("{two} closed\n")
    );
    let shown = json(&env.ok(&["show", &two, "--json"]));
    assert_eq!(shown["close"]["outcome"], "closed after 36 hours picked up");
}

/// `--stale-briefings` sets its own outcome, age and writer, and takes no
/// in-flight.
#[test]
fn stale_briefings_takes_no_reason_age_writer_or_in_flight() {
    let env = Env::new();
    for flag in [
        &["--reason", "x"][..],
        &["--older-than", "1h"],
        &["--in-flight"],
        &["--as", "Ryan"],
    ] {
        let mut args = vec!["sweep", "--stale-briefings"];
        args.extend(flag);
        let out = env.cases(&args);
        assert_eq!(out.code, 1, "{flag:?}: {out:?}");
        assert!(
            out.stderr.contains("cannot be used with"),
            "{flag:?}: {out:?}"
        );
    }
}
