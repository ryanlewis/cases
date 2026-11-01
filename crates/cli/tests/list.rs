//! `cases list` (AG-14, ST-05, ST-08).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use cases_core::{Append, Store};
use cases_store_sqlite::Writer;
use common::{Env, json};

/// The ids `list --json` prints with `args`, in its order.
fn listed(env: &Env, args: &[&str]) -> Vec<String> {
    let mut all = vec!["list", "--json"];
    all.extend(args);
    json(&env.ok(&all))["list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["id"].as_str().unwrap().to_owned())
        .collect()
}

/// `ids`, sorted, to compare what a filter picks whatever the order.
fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn strings(ids: &[&String]) -> Vec<String> {
    sorted(ids.iter().map(|id| (*id).clone()).collect())
}

#[test]
fn list_sorts_by_urgency_then_open_order() {
    let env = Env::new();
    let later = env.open(&["--kind", "fyi", "--urgency", "whenever", "--title", "Later"]);
    let first_today = env.open_fyi("Today 1");
    let urgent = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "blocking",
        "--title",
        "Urgent",
    ]);
    let second_today = env.open_fyi("Today 2");
    assert_eq!(
        listed(&env, &[]),
        [
            urgent.clone(),
            first_today.clone(),
            second_today.clone(),
            later.clone()
        ]
    );
    let out = env.ok(&["list"]);
    let at = |id: &str| out.find(id).unwrap();
    assert!(
        at(&urgent) < at(&first_today)
            && at(&first_today) < at(&second_today)
            && at(&second_today) < at(&later),
        "{out}"
    );
}

#[test]
fn list_filters_by_state_and_prints_json() {
    let env = Env::new();
    env.open_fyi("Open");
    let answered = env.open_fyi("Seen");
    env.write(&answered, "human", "answer", r#"{"ack": true}"#);

    let shown = json(&env.ok(&["list", "--state", "answered", "--json"]));
    let list = shown["list"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], answered.as_str());
    assert_eq!(list[0]["state"], "answered");
    // Each item is the case as show --json prints it, with its revision and
    // without a url.
    assert_eq!(list[0]["revision"], 2);
    assert_eq!(list[0]["events"].as_array().unwrap().len(), 2);
    assert!(list[0].get("url").is_none(), "{shown}");
    assert_eq!(list[0]["amend_seq"], 0);

    assert_eq!(
        env.ok(&["list", "--state", "closed,withdrawn", "--json"]),
        "{\n  \"list\": []\n}\n"
    );
    let out = env.cases(&["list", "--state", "done"]);
    assert_eq!(out.code, 1, "{out:?}");
}

#[test]
fn list_shows_labels_before_the_title() {
    let env = Env::new();
    let labelled = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "blocking",
        "--title",
        "Labelled",
        "--label",
        "round-1",
        "--label",
        "docs",
    ]);
    let plain = env.open_fyi("Plain");
    let out = env.ok(&["list"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    let labels_at = lines[0].find("LABELS").unwrap();
    let title_at = lines[0].find("TITLE").unwrap();
    for (line, id, labels, title) in [
        (lines[1], &labelled, "round-1,docs", "Labelled"),
        (lines[2], &plain, "", "Plain"),
    ] {
        // The id with its title's slug after it (ST-10).
        let slug = title.to_lowercase();
        assert!(line.starts_with(&format!("{id}-{slug} ")), "{line}");
        assert_eq!(line[labels_at..title_at].trim(), labels, "{line}");
        assert_eq!(&line[title_at..], title, "{line}");
    }
    assert_eq!(
        lines[0].split_whitespace().collect::<Vec<_>>(),
        ["ID", "STATE", "URGENCY", "KIND", "AGE", "LABELS", "TITLE"],
        "{out}"
    );
    assert!(lines[1].contains(" now "), "{out}");
}

#[test]
fn list_filters_by_label_and_worker() {
    let env = Env::new();
    let open = |title: &str, extra: &[&str]| {
        let mut args = vec!["--kind", "fyi", "--urgency", "today", "--title", title];
        args.extend(extra);
        env.open(&args)
    };
    let a = open("A", &["--label", "round-1", "--worker", "w1"]);
    let b = open(
        "B",
        &["--label", "round-1", "--label", "docs", "--worker", "w2"],
    );
    let c = open("C", &["--label", "round-2", "--worker", "w1"]);
    let d = open("D", &[]);
    let cases: &[(&[&str], Vec<&String>)] = &[
        (&[], vec![&a, &b, &c, &d]),
        (&["--label", "round-1"], vec![&a, &b]),
        (&["--label", "docs", "--label", "round-2"], vec![&b, &c]),
        (&["--worker", "w1"], vec![&a, &c]),
        (&["--worker", "w1", "--worker", "w2"], vec![&a, &b, &c]),
        (&["--label", "round-1", "--worker", "w1"], vec![&a]),
        (&["--label", "docs", "--worker", "w1"], vec![]),
        (&["--label", "nope"], vec![]),
    ];
    for (args, want) in cases {
        assert_eq!(sorted(listed(&env, args)), strings(want), "list {args:?}");
    }
}

#[test]
fn list_warns_about_a_case_that_does_not_fold_and_lists_the_rest() {
    let env = Env::new();
    let good = env.open_fyi("Good");
    let broken = env.insert(None, 1, "agent", "open", "{");
    let out = env.cases(&["list"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains(&good), "{out:?}");
    assert!(!out.stdout.contains(&broken), "{out:?}");
    assert!(
        out.stderr.starts_with(&format!("warning: {broken}: ")),
        "{out:?}"
    );
}

#[test]
fn list_defaults_to_open_and_parked() {
    let env = Env::new();
    let open = |title: &str| env.open(&["--kind", "stuck", "--urgency", "today", "--title", title]);
    let opened = open("Open");
    let parked = open("Parked");
    env.write(&parked, "human", "park", "{}");
    let answered = open("Answered");
    env.write(&answered, "human", "answer", r#"{"text": "Try again"}"#);
    let withdrawn = open("Withdrawn");
    env.write(&withdrawn, "agent", "withdraw", "{}");

    let cases: &[(&[&str], Vec<&String>)] = &[
        (&[], vec![&opened, &parked]),
        (&["--all"], vec![&opened, &parked, &answered, &withdrawn]),
        (&["--state", "withdrawn"], vec![&withdrawn]),
        (&["--state", "answered", "--all"], vec![&answered]),
        (
            &["--state", "answered", "--state", "parked"],
            vec![&parked, &answered],
        ),
        (&["--state", "closed"], vec![]),
    ];
    for (args, want) in cases {
        assert_eq!(sorted(listed(&env, args)), strings(want), "list {args:?}");
    }
    let out = env.ok(&["list"]);
    assert!(
        out.contains(&opened) && out.contains(&parked) && !out.contains(&answered),
        "{out}"
    );
}

#[test]
fn list_filters_by_kind_and_urgency() {
    let env = Env::new();
    let open = |kind: &str, urgency: &str| {
        let title = format!("{kind} {urgency}");
        let mut args = vec!["--kind", kind, "--urgency", urgency, "--title", &title];
        if kind == "decision" {
            args.extend(["--option", "Yes", "--option", "No"]);
        }
        env.open(&args)
    };
    let fyi_today = open("fyi", "today");
    let fyi_blocking = open("fyi", "blocking");
    let decision = open("decision", "blocking");
    let question = open("question", "whenever");
    let cases: &[(&[&str], Vec<&String>)] = &[
        (&["--kind", "fyi"], vec![&fyi_today, &fyi_blocking]),
        (
            &["--kind", "decision", "--kind", "question"],
            vec![&decision, &question],
        ),
        (&["--kind", "stuck"], vec![]),
        (&["--urgency", "blocking"], vec![&fyi_blocking, &decision]),
        (
            &["--urgency", "today", "--urgency", "whenever"],
            vec![&fyi_today, &question],
        ),
        (
            &["--kind", "fyi", "--urgency", "blocking"],
            vec![&fyi_blocking],
        ),
    ];
    for (args, want) in cases {
        assert_eq!(sorted(listed(&env, args)), strings(want), "list {args:?}");
    }
    // A typo is refused rather than matching nothing.
    for args in [
        ["--kind", "decison"],
        ["--kind", "fyi,decision"],
        ["--urgency", "urgent"],
    ] {
        let mut all = vec!["list"];
        all.extend(args);
        let out = env.cases(&all);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert!(out.stderr.contains("invalid value"), "{args:?}: {out:?}");
    }
}

#[test]
fn list_count_prints_only_the_number() {
    let env = Env::new();
    for args in [&["list", "--count"][..], &["list", "--count", "--json"]] {
        let out = env.cases(args);
        assert_eq!(
            (out.code, out.stdout.as_str()),
            (0, "0\n"),
            "{args:?} on a missing store"
        );
    }
    env.open(&["--kind", "fyi", "--urgency", "blocking", "--title", "A"]);
    env.open(&["--kind", "fyi", "--urgency", "blocking", "--title", "B"]);
    let answered = env.open_fyi("C");
    env.write(&answered, "human", "answer", r#"{"ack": true}"#);
    for (args, want) in [
        (&[][..], "2\n"),
        (&["--all"], "3\n"),
        (&["--urgency", "blocking"], "2\n"),
        (&["--urgency", "whenever"], "0\n"),
        (&["--json"], "2\n"),
    ] {
        let mut all = vec!["list", "--count"];
        all.extend(args);
        assert_eq!(env.ok(&all), want, "{args:?}");
    }
}

#[test]
fn list_on_a_store_that_is_not_there_is_empty() {
    let env = Env::new();
    assert_eq!(env.ok(&["list"]), "No cases.\n");
    assert_eq!(env.ok(&["list", "--json"]), "{\n  \"list\": []\n}\n");
    assert!(!env.store.exists(), "list made the store");
}

#[test]
fn list_older_than_reads_the_last_event() {
    let env = Env::new();
    let db = env.db();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let ago = |secs: u64| {
        cases_core::Time::from_unix(i64::try_from(now - secs).unwrap(), 0)
            .unwrap()
            .to_string()
    };
    let event = |author: &str, event: &str, data: String| Append {
        author: author.into(),
        event: event.into(),
        data,
    };
    let open = |title: &str, secs: u64| {
        let data = format!(
            r#"{{"kind": "stuck", "urgency": "today", "title": "{title}", "opened_at": "{}"}}"#,
            ago(secs)
        );
        db.create(&event("agent", "open", data), &Writer::new(None))
            .unwrap()
            .0
    };
    let park = |key, secs| {
        let data = format!(r#"{{"parked_at": "{}"}}"#, ago(secs));
        db.append(
            &key,
            &event("human", "park", data),
            &Writer::new(None),
            None,
        )
        .unwrap();
    };
    let old = open("Old", 48 * 3600);
    let recent = open("Recent", 60);
    // Opened and parked long ago, resumed a minute ago: it has not waited
    // 30 minutes.
    let resumed = open("Resumed", 48 * 3600);
    park(resumed, 47 * 3600);
    let data = format!(r#"{{"resumed_at": "{}"}}"#, ago(60));
    db.append(
        &resumed,
        &event("human", "resume", data),
        &Writer::new(None),
        None,
    )
    .unwrap();
    // Parked long ago and left parked: it has.
    let parked = open("Parked", 48 * 3600);
    park(parked, 47 * 3600);
    // Opened in the future, by a clock that is ahead: any age takes it.
    let ahead = open("Ahead", 0);
    let ahead_data = format!(
        r#"{{"body": "x", "amended_at": "{}"}}"#,
        cases_core::Time::from_unix(i64::try_from(now + 3600).unwrap(), 0).unwrap()
    );
    db.append(
        &ahead,
        &event("agent", "amend", ahead_data),
        &Writer::new(None),
        None,
    )
    .unwrap();

    let [old, recent, resumed, parked, ahead] =
        [old, recent, resumed, parked, ahead].map(|key| key.to_string());
    let cases: &[(&[&str], Vec<&String>)] = &[
        (&[], vec![&old, &recent, &resumed, &parked, &ahead]),
        (
            &["--older-than", "0"],
            vec![&old, &recent, &resumed, &parked, &ahead],
        ),
        (&["--older-than", "30m"], vec![&old, &parked]),
        (&["--older-than", "30m", "--state", "open"], vec![&old]),
        (&["--older-than", "72h"], vec![]),
    ];
    for (args, want) in cases {
        assert_eq!(sorted(listed(&env, args)), strings(want), "list {args:?}");
    }
    let out = env.cases(&["list", "--older-than", "soon"]);
    assert_eq!(out.code, 1, "{out:?}");
}

/// A case whose open has a kind, urgency or event this version does not
/// know does not fold, and is reported as perhaps from a newer cases
/// (ST-05). One that is only damaged is not.
#[test]
fn a_case_from_a_newer_cases_is_reported_as_such() {
    let env = Env::new();
    env.open_fyi("Good");
    let hint = "perhaps written by a newer cases";
    for (data, event, newer) in [
        (
            r#"{"kind": "poll", "urgency": "today", "title": "T"}"#,
            "open",
            true,
        ),
        (
            r#"{"kind": "fyi", "urgency": "soon", "title": "T"}"#,
            "open",
            true,
        ),
        (r#"{"body": "x"}"#, "comment", true),
        (r#"{"kind": "fyi", "urgency": "today"}"#, "open", false),
        ("{", "open", false),
    ] {
        let id = env.insert(None, 1, "agent", event, data);
        let out = env.cases(&["list"]);
        let warning = out
            .stderr
            .lines()
            .find(|line| line.starts_with(&format!("warning: {id}: ")))
            .unwrap_or_else(|| panic!("no warning about {id}: {out:?}"));
        assert_eq!(warning.contains(hint), newer, "{data}: {warning}");
        let shown = env.cases(&["show", &id]);
        assert_eq!(shown.code, 1, "{shown:?}");
        assert_eq!(shown.stderr.contains(hint), newer, "{data}: {shown:?}");
    }
}
