//! `cases prune` (ST-13).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
use std::collections::BTreeMap;

use cases_core::State;
use common::{Env, from_now};

/// A store with one case in each state prune looks at, last written 60 days
/// ago, and a case withdrawn an hour ago.
struct Pruned {
    env: Env,
    closed: String,
    withdrawn: String,
    open: String,
    recent: String,
}

fn pruned() -> Pruned {
    let env = Env::new();
    let old = from_now(-60 * 24 * 3600);
    let at = |field: &str| format!(r#"{{"{field}": "{old}""#);
    let closed = env.open_at("Closed", &old, "");
    env.write(
        &closed,
        "human",
        "answer",
        &format!(r#"{}, "ack": true}}"#, at("answered_at")),
    );
    env.write(
        &closed,
        "agent",
        "pickup",
        &format!("{}}}", at("picked_up_at")),
    );
    env.write(
        &closed,
        "agent",
        "close",
        &format!(r#"{}, "outcome": "done"}}"#, at("closed_at")),
    );
    let withdrawn = env.open_at("Withdrawn", &old, "");
    env.write(
        &withdrawn,
        "agent",
        "withdraw",
        &format!("{}}}", at("withdrawn_at")),
    );
    let open = env.open_at("Open", &old, "");
    let recent = env.open_at("Recent", &old, "");
    let hour_ago = format!(r#"{{"withdrawn_at": "{}"}}"#, from_now(-3600));
    env.write(&recent, "agent", "withdraw", &hour_ago);
    Pruned {
        env,
        closed,
        withdrawn,
        open,
        recent,
    }
}

/// Each case in the store, by id, with its revision.
fn revisions(env: &Env) -> BTreeMap<String, u64> {
    let db = env.db();
    cases_core::Store::list(&db)
        .unwrap()
        .into_iter()
        .map(|(key, rows)| (key.to_string(), u64::try_from(rows.len()).unwrap()))
        .collect()
}

#[test]
fn a_dry_run_writes_nothing() {
    let p = pruned();
    let before = revisions(&p.env);
    let out = p.env.cases(&["prune"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "{} closed, would be archived\n{} withdrawn, would be archived\n",
            p.closed, p.withdrawn
        )
    );
    assert_eq!(
        out.stderr,
        format!(
            "2 cases would be moved to the archive in {}. Pass --yes to prune them.\n",
            p.env.store.display()
        )
    );
    // Same message, singular, with one matching case.
    let solo = p.env.cases(&["prune", "--state", "closed"]);
    assert!(
        solo.stderr
            .contains("1 case would be moved to the archive in"),
        "{solo:?}"
    );
    assert_eq!(revisions(&p.env), before);
    assert!(p.env.archived().is_empty());
}

#[test]
fn prune_moves_each_case_whole_into_the_archive() {
    let p = pruned();
    let before = revisions(&p.env);
    assert_eq!(
        p.env.ok(&["prune", "--yes"]),
        format!("{} archived\n{} archived\n", p.closed, p.withdrawn)
    );
    let want: BTreeMap<String, u64> = [
        (p.closed.clone(), before[&p.closed]),
        (p.withdrawn.clone(), before[&p.withdrawn]),
    ]
    .into();
    assert_eq!(p.env.archived(), want);
    for id in [&p.closed, &p.withdrawn] {
        let out = p.env.cases(&["show", id]);
        assert_eq!(
            (out.code, out.stderr),
            (1, format!("Error: no case {id}\n"))
        );
    }
    let list = p.env.ok(&["list", "--all"]);
    assert!(
        !list.contains(&p.closed) && !list.contains(&p.withdrawn),
        "{list}"
    );
    assert!(list.contains(&p.open) && list.contains(&p.recent), "{list}");
    // The archive keeps the id: a new case gets a key of its own.
    let new = p.env.open_fyi("New");
    assert!(![&p.closed, &p.withdrawn, &p.open, &p.recent].contains(&&new));

    // --age 0 takes the recent one too, and --state narrows.
    assert_eq!(p.env.ok(&["prune", "--age", "0", "--state", "closed"]), "");
    assert_eq!(
        p.env
            .ok(&["prune", "--age", "0", "--state", "withdrawn", "--yes"]),
        format!("{} archived\n", p.recent)
    );
}

#[test]
fn a_case_whose_key_the_archive_has_is_left() {
    let p = pruned();
    let rows = cases_core::Store::rows(&p.env.db(), &common::key(&p.closed))
        .unwrap()
        .unwrap();
    p.env.ok(&["prune", "--yes", "--state", "closed"]);
    // Put back in the store by hand.
    let conn = rusqlite::Connection::open(&p.env.store).unwrap();
    let key = i64::try_from(common::key(&p.closed).get()).unwrap();
    conn.execute("INSERT INTO cases (key, opened_at) VALUES (?1, '')", [key])
        .unwrap();
    drop(conn);
    for row in &rows {
        p.env
            .insert(Some(&p.closed), row.seq, &row.author, &row.event, &row.data);
    }

    let out = p.env.cases(&["prune", "--yes"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, format!("{} archived\n", p.withdrawn));
    assert!(out.stderr.contains("1 case was not pruned"), "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "{}: case {} is in the archive already",
            p.closed, p.closed
        )),
        "{out:?}"
    );
    assert_eq!(p.env.case(&p.closed).state(), State::Closed);
    assert_eq!(
        p.env.revision(&p.closed),
        u64::try_from(rows.len()).unwrap()
    );
}

#[test]
fn prune_delete_removes_the_cases() {
    let p = pruned();
    assert!(
        p.env
            .ok(&["prune", "--delete"])
            .contains(&format!("{} closed, would be deleted\n", p.closed))
    );
    assert_eq!(
        p.env.ok(&["prune", "--delete", "--yes"]),
        format!("{} deleted\n{} deleted\n", p.closed, p.withdrawn)
    );
    let left = revisions(&p.env);
    assert!(
        !left.contains_key(&p.closed)
            && !left.contains_key(&p.withdrawn)
            && left.contains_key(&p.open)
    );
    assert!(p.env.archived().is_empty());
}

#[test]
fn prune_leaves_a_case_that_does_not_fold_names_it_and_exits_1() {
    let p = pruned();
    let broken = p.env.insert(None, 1, "agent", "open", "{");
    // A dry run warns about it.
    let out = p.env.cases(&["prune", "--age", "0"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!("warning: {broken}: ")),
        "{out:?}"
    );
    // Prune goes on with the rest, then names it and exits 1 (ST-13).
    let out = p.env.cases(&["prune", "--yes", "--age", "0"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!("warning: {broken}: ")),
        "{out:?}"
    );
    assert!(out.stderr.contains("1 case was not pruned:"), "{out:?}");
    assert!(
        out.stderr
            .contains(&format!("{broken}: cannot be loaded, and was left: ")),
        "{out:?}"
    );
    assert!(!out.stdout.contains(&broken), "{out:?}");
    assert!(
        out.stdout.contains(&format!("{} archived\n", p.closed)),
        "{out:?}"
    );
    assert!(revisions(&p.env).contains_key(&broken));
}

#[test]
fn prune_takes_only_closed_and_withdrawn_cases() {
    let p = pruned();
    for args in [
        &["--state", "open"][..],
        &["--state", "closed,answered"],
        &["--state", "nope"],
        &["--age", "-1h"],
        &["--age", "soon"],
    ] {
        let mut all = vec!["prune", "--yes"];
        all.extend(args);
        let out = p.env.cases(&all);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
    }
    assert!(p.env.archived().is_empty());
}

#[test]
fn prune_on_a_store_that_is_not_there_does_nothing() {
    let env = Env::new();
    assert_eq!(env.ok(&["prune", "--yes"]), "");
    assert!(!env.store.exists());
}

#[test]
fn the_config_file_sets_the_age_below_the_flag() {
    let p = pruned();
    // Without the file, 720h: the case withdrawn an hour ago stays.
    assert!(!p.env.ok(&["prune"]).contains(&p.recent));
    p.env.config("prune-age = \"30m\"\n");
    assert!(p.env.ok(&["prune"]).contains(&p.recent));
    assert!(!p.env.ok(&["prune", "--age", "720h"]).contains(&p.recent));

    // The key is read by prune only.
    p.env.config("prune-age = \"not a duration\"\n");
    p.env.ok(&["list"]);
    let out = p.env.cases(&["prune"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("prune-age"), "{out:?}");
}

#[test]
fn an_age_of_0_takes_a_case_dated_in_the_future() {
    let env = Env::new();
    let id = env.open_at("Ahead", &from_now(0), "");
    env.write(
        &id,
        "agent",
        "withdraw",
        &format!(r#"{{"withdrawn_at": "{}"}}"#, from_now(3600)),
    );
    assert_eq!(
        env.ok(&["prune", "--age", "0"]),
        format!("{id} withdrawn, would be archived\n")
    );
}
