//! Reading cases, and the poller. The Go store's `list_test.go`, as
//! `PARITY.md` maps it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::collections::BTreeMap;

use cases_core::{Case, Category, State, Store};
use cases_store_sqlite::{Db, Key, Poller};
use common::{
    agent_event, create, human_event, insert_row, put_back, scratch, stored, throw_away, unnamed,
};

/// The case the store has for `key`, folded, or the fold's error.
fn get(db: &Db, key: Key) -> Result<Case, cases_core::Error> {
    Case::fold(&db.rows(&key).unwrap().expect("the store has the case"))
}

#[test]
fn a_case_with_damaged_rows_still_folds() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let key_sql = common::key_sql(key);
    insert_row(&db, key_sql, 2, "human", "answer", r#"{"ack": tru"#);
    insert_row(&db, key_sql, 3, "bob", "note", r#"{"body":"hi"}"#);

    let case = get(&db, key).unwrap();
    assert_eq!(
        (case.state(), case.revision(), case.events().len()),
        (State::Open, 3, 1)
    );
    let problems: Vec<(String, Category)> = case
        .problems()
        .iter()
        .map(|problem| (problem.file(), problem.error.category()))
        .collect();
    assert_eq!(
        problems,
        [
            ("0002-human-answer.json".into(), Category::Malformed),
            ("0003-bob-note.json".into(), Category::Author),
        ]
    );

    // The next write goes after the damaged rows, not on top of them.
    let ack = human_event("answer", r#"{"ack": true}"#);
    let after = db.append(&key, &ack, &unnamed(), None).unwrap();
    assert_eq!(
        after.events().last().unwrap().file(),
        "0004-human-answer.json"
    );
    assert_eq!(after.revision(), 4);
}

#[test]
fn fields_this_version_does_not_know_are_kept() {
    let (_dir, db) = scratch();
    create(&db, "fyi");
    let data = r#"{"kind":"fyi","urgency":"today","title":"x","opened_at":"2026-09-15T09:00:00Z","colour":"teal","extra":{"n":1}}"#;
    insert_row(&db, 900, 1, "agent", "open", data);
    let case = get(&db, Key::new(900).unwrap()).unwrap();
    assert!(case.problems().is_empty());
    assert_eq!(case.events()[0].data, data);
}

#[test]
fn a_case_without_an_open_does_not_fold() {
    let (_dir, db) = scratch();
    create(&db, "fyi");
    common::sql(&db)
        .execute("INSERT INTO cases (key, opened_at) VALUES (900, '')", [])
        .unwrap();
    let empty = Key::new(900).unwrap();
    assert_eq!(db.rows(&empty).unwrap(), Some(Vec::new()));
    assert_eq!(get(&db, empty).unwrap_err().category(), Category::NoOpen);
    insert_row(&db, 901, 1, "agent", "open", "not json");
    let broken = get(&db, Key::new(901).unwrap()).unwrap_err();
    assert_eq!(broken.category(), Category::NoOpen);
}

#[test]
fn a_case_that_does_not_fold_is_listed_with_the_rest() {
    let (_dir, db) = scratch();
    let (good, _) = create(&db, "decision");
    insert_row(&db, 900, 1, "agent", "open", r#"{"kind":"#);
    let listed = db.list().unwrap();
    let folded: Vec<(u64, Option<Category>)> = listed
        .iter()
        .map(|(key, rows)| (key.get(), Case::fold(rows).err().map(|e| e.category())))
        .collect();
    assert_eq!(folded, [(good.get(), None), (900, Some(Category::NoOpen))]);
    let mut poller = db.poller();
    let polled: Vec<(u64, Option<Category>)> = poller
        .poll()
        .unwrap()
        .iter()
        .map(|(key, case)| {
            (
                key.get(),
                case.as_ref().err().map(cases_core::Error::category),
            )
        })
        .collect();
    assert_eq!(polled, folded);
}

/// Cases list in the order they were opened, which is their keys' order and
/// not their ids' (ST-10, AG-14).
#[test]
fn cases_list_in_the_order_they_were_opened() {
    let (_dir, db) = scratch();
    for title in ["B case", "A case", "C case"] {
        let open = format!(r#"{{"kind": "fyi", "urgency": "today", "title": "{title}"}}"#);
        db.create(&agent_event("open", &open), &unnamed()).unwrap();
    }
    let listed = db.list().unwrap();
    let ids: Vec<String> = listed.iter().map(|(key, _)| key.to_string()).collect();
    assert_eq!(ids, ["UkLWZ", "gbHJd", "EfhxL"]);
    let titles: Vec<String> = listed
        .iter()
        .map(|(_, rows)| Case::fold(rows).unwrap().title().to_owned())
        .collect();
    assert_eq!(titles, ["B case", "A case", "C case"]);
}

/// Polls, and gives each case's state by key.
fn states(poller: &mut Poller<'_>) -> BTreeMap<Key, State> {
    poller
        .poll()
        .unwrap()
        .iter()
        .map(|(key, case)| (*key, case.as_ref().expect("the case folds").state()))
        .collect()
}

#[test]
fn the_poller_sees_new_events_and_cases() {
    let (_dir, db) = scratch();
    let (first, _) = create(&db, "fyi");
    let mut poller = db.poller();
    assert_eq!(states(&mut poller), BTreeMap::from([(first, State::Open)]));

    let ack = human_event("answer", r#"{"ack": true}"#);
    db.append(&first, &ack, &unnamed(), None).unwrap();
    let (second, _) = create(&Db::new(db.path()), "stuck");
    assert_eq!(
        states(&mut poller),
        BTreeMap::from([(first, State::Answered), (second, State::Open)])
    );
    // With nothing written, the cases are as they were.
    assert_eq!(states(&mut poller).len(), 2);
    db.append(&second, &human_event("park", "{}"), &unnamed(), None)
        .unwrap();
    assert_eq!(states(&mut poller)[&second], State::Parked);
    db.delete(&second, None).unwrap();
    assert_eq!(
        states(&mut poller),
        BTreeMap::from([(first, State::Answered)])
    );
}

/// Taking out the case that holds the highest change and putting another in
/// changes the keys, which the poller's first check reads. The poller then
/// folds again only the cases with a row above the highest change it had, so
/// a row written to a case that stays must still get a change above it: a
/// change is never used twice, even once the case that held it is gone. The
/// write comes before the new case, which would otherwise take that change.
#[test]
fn the_poller_sees_a_case_replaced_by_another() {
    let (_dir, db) = scratch();
    let (older, _) = create(&db, "stuck");
    let (mut newest, _) = create(&db, "question");
    let mut poller = db.poller();
    assert_eq!(states(&mut poller).len(), 2);
    for (archive, event, state) in [
        (false, human_event("park", "{}"), State::Parked),
        (true, human_event("resume", "{}"), State::Open),
    ] {
        if archive {
            db.archive(&newest, None).unwrap();
        } else {
            db.delete(&newest, None).unwrap();
        }
        db.append(&older, &event, &unnamed(), None).unwrap();
        let open = agent_event(
            "open",
            &format!(r#"{{"kind": "stuck", "urgency": "today", "title": "Replacement {newest}"}}"#),
        );
        let (replaced, _) = db.create(&open, &unnamed()).unwrap();
        assert_eq!(
            states(&mut poller),
            BTreeMap::from([(older, state), (replaced, State::Open)])
        );
        newest = replaced;
    }
}

/// A case put back from the archive by hand keeps the changes its rows had,
/// which are below the highest the poller has seen. The poller still finds
/// it, as a case it has not seen.
#[test]
fn the_poller_sees_a_case_put_back_from_the_archive() {
    let (_dir, db) = scratch();
    let (case, _) = create(&db, "fyi");
    create(&db, "stuck");
    db.archive(&case, None).unwrap();
    let mut poller = db.poller();
    assert_eq!(states(&mut poller).len(), 1);
    put_back(&db, case);
    let after = states(&mut poller);
    assert_eq!((after.len(), after[&case]), (2, State::Open));
}

/// Between two polls, one case is archived and another is put back from the
/// archive. The number of cases is as it was, and so is the highest change:
/// the case that holds it stays, and the case put back keeps its rows'
/// changes. The poller must still see the swap.
#[test]
fn the_poller_sees_a_case_archived_and_another_put_back() {
    let (_dir, db) = scratch();
    let (restored, _) = create(&db, "fyi");
    let (holder, _) = create(&db, "stuck");
    let (removed, _) = create(&db, "question");
    db.append(
        &holder,
        &agent_event("note", r#"{"body": "Still stuck."}"#),
        &unnamed(),
        None,
    )
    .unwrap();
    db.archive(&restored, None).unwrap();
    let mut poller = db.poller();
    assert_eq!(
        states(&mut poller),
        BTreeMap::from([(holder, State::Open), (removed, State::Open)])
    );
    db.archive(&removed, None).unwrap();
    put_back(&db, restored);
    assert_eq!(
        states(&mut poller),
        BTreeMap::from([(restored, State::Open), (holder, State::Open)])
    );
}

/// A store thrown away and started again holds a case with the same key and
/// the same change as the case the poller has: the poller must fold the new
/// file, whether or not a poll saw the store missing in between.
#[test]
fn the_poller_starts_again_on_a_new_file() {
    let (_dir, db) = scratch();
    let same_title = |kind: &str| {
        agent_event(
            "open",
            &format!(
                r#"{{"kind": "{kind}", "urgency": "today", "title": "Same key", "opened_at": "2026-09-15T09:00:00Z"}}"#
            ),
        )
    };
    let (key, _) = db.create(&same_title("fyi"), &unnamed()).unwrap();
    let mut poller = db.poller();
    poller.poll().unwrap();
    let start_again = |kind: &str| {
        let again = Db::new(db.path());
        again.create(&same_title(kind), &unnamed()).unwrap()
    };
    let kind_of = |poller: &mut Poller<'_>| {
        let cases = poller.poll().unwrap();
        assert_eq!(cases.keys().copied().collect::<Vec<_>>(), [key]);
        cases[&key].as_ref().unwrap().kind().as_str().to_owned()
    };

    throw_away(db.path());
    assert!(poller.poll().is_err());
    start_again("stuck");
    assert_eq!(
        kind_of(&mut poller),
        "stuck",
        "after a poll saw the store missing"
    );

    throw_away(db.path());
    start_again("question");
    assert_eq!(kind_of(&mut poller), "question", "with no poll in between");
    assert_eq!(stored(&db, key).len(), 1);
}
