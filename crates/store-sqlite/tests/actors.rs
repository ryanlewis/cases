//! Actors: who writes each event, by id, with the store's table of actors
//! for their kinds and names (ST-12, AG-01, HU-06). The Go store's
//! `actor_test.go`, as `PARITY.md` maps it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use cases_core::{Actor, Case, Category, Store, StoreError};
use cases_store_sqlite::{ActorRow, Db, Writer};
use common::{agent_event, create, fixed, human_event, open_of, scratch, sql, stored};
use serde_json::Value;

fn named(name: &str) -> Writer {
    Writer::new(Some(name))
}

/// The actor of each stored row of the case, from its record.
fn row_actors(db: &Db, key: cases_store_sqlite::Key) -> Vec<Option<String>> {
    stored(db, key)
        .iter()
        .map(|row| {
            let record: Value = serde_json::from_str(&row.data).unwrap();
            record["actor"]["id"].as_str().map(str::to_owned)
        })
        .collect()
}

/// The table's rows as (kind, name).
fn table(db: &Db) -> Vec<(String, Option<String>)> {
    db.actors()
        .unwrap()
        .into_iter()
        .map(|actor| (actor.kind, actor.name))
        .collect()
}

#[test]
fn each_event_carries_its_writers_actor() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-17T10:00:00Z");
    let pins = named("bun-pins");
    let ryan = named("Ryan");
    let open = agent_event(
        "open",
        r#"{"kind": "decision", "urgency": "today", "title": "Pin bun?", "options": ["Pin", "Float"], "for": "Ryan"}"#,
    );
    let (key, _) = db.create(&open, &pins).unwrap();
    db.append(
        &key,
        &human_event("answer", r#"{"choice": 1}"#),
        &ryan,
        None,
    )
    .unwrap();
    let case = db
        .append(&key, &agent_event("pickup", "{}"), &pins, None)
        .unwrap();

    let actors = db.actors().unwrap();
    let [pins_row, ryan_row] = &actors[..] else {
        panic!("the actors are {actors:?}");
    };
    assert_eq!(
        (pins_row.kind.as_str(), pins_row.name.as_deref()),
        ("agent", Some("bun-pins"))
    );
    assert_eq!(
        (ryan_row.kind.as_str(), ryan_row.name.as_deref()),
        ("human", Some("Ryan"))
    );
    assert_eq!(pins_row.created, "2026-09-17T10:00:00Z");
    let id = |row: &ActorRow| {
        let mut actor = Actor::default();
        actor.id.clone_from(&row.id);
        actor
    };
    let events: Vec<Option<&Actor>> = case.events().iter().map(cases_core::Event::actor).collect();
    assert_eq!(
        events,
        [
            Some(&id(pins_row)),
            Some(&id(ryan_row)),
            Some(&id(pins_row))
        ]
    );
    assert_eq!(case.actor(), Some(&id(pins_row)));
    assert_eq!(case.answer().unwrap().actor.as_ref(), Some(&id(ryan_row)));
    assert_eq!(case.for_(), "Ryan");
    assert_eq!(Case::fold(&db.rows(&key).unwrap().unwrap()).unwrap(), case);
}

/// One actor for each kind and name, made by its first write, and one
/// unnamed actor for each kind. The kind is the event's author, so one name
/// that writes as an agent and as a human has an actor for each.
#[test]
fn the_first_write_by_a_name_makes_its_actor() {
    let (_dir, db) = scratch();
    let note = agent_event("note", r#"{"body": "More."}"#);
    let ack = human_event("answer", r#"{"ack": true}"#);
    for (event, writer) in [
        (&note, named("w1")),
        (&note, named("w1")),
        (&note, named("w2")),
        (&note, Writer::new(None)),
        (&note, Writer::new(Some(""))),
        (&ack, named("w1")),
        (&ack, Writer::new(None)),
        (&ack, Writer::new(None)),
    ] {
        // Each case is opened by the unnamed agent.
        let (key, _) = create(&db, "fyi");
        db.append(&key, event, &writer, None).unwrap();
    }
    let name = |name: &str| Some(name.to_owned());
    assert_eq!(
        table(&db),
        [
            ("agent".into(), None),
            ("agent".into(), name("w1")),
            ("agent".into(), name("w2")),
            ("human".into(), name("w1")),
            ("human".into(), None),
        ]
    );
}

/// A write that is refused, or that fails, makes no actor: the actor is made
/// in the write's transaction.
#[test]
fn a_refused_write_makes_no_actor() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let before = table(&db);
    let pickup = agent_event("pickup", "{}");
    match db.append(&key, &pickup, &named("new-worker"), None) {
        Err(StoreError::Refused(error)) => assert_eq!(error.category(), Category::Transition),
        other => panic!("{other:?}"),
    }
    let stale = human_event("answer", r#"{"ack": true}"#);
    assert!(db.append(&key, &stale, &named("Ryan"), Some(0)).is_err());
    assert_eq!(table(&db), before);
}

/// A row of the table can be deleted: the events keep the bare id, and the
/// next write by that name makes a new actor.
#[test]
fn a_deleted_actor_leaves_its_events_a_bare_id() {
    let (_dir, db) = scratch();
    let pins = named("bun-pins");
    let (key, _) = db.create(&open_of("stuck"), &pins).unwrap();
    let old = db.actors().unwrap().remove(0).id;
    sql(&db)
        .execute("DELETE FROM actors WHERE id = ?1", [&old])
        .unwrap();
    assert!(db.actors().unwrap().is_empty());
    let rows = db.rows(&key).unwrap().unwrap();
    let case = Case::fold(&rows).unwrap();
    assert_eq!(
        case.actor().map(|actor| actor.id.as_str()),
        Some(old.as_str())
    );

    let note = agent_event("note", r#"{"body": "More."}"#);
    db.append(&key, &note, &pins, None).unwrap();
    let new = db.actors().unwrap().remove(0).id;
    assert_ne!(new, old);
    assert_eq!(row_actors(&db, key), [Some(old), Some(new)]);
}

/// The writer's actor takes the place of an actor the record gives, once
/// that one has been checked.
#[test]
fn a_records_own_actor_gives_way_to_the_writers() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    let claimed = r#"{"body": "More.", "actor": {"id": "hV3kQ9mZr2pX7wLc0tBnYA"}}"#;
    db.append(
        &key,
        &agent_event("note", claimed),
        &common::unnamed(),
        None,
    )
    .unwrap();
    let ids = row_actors(&db, key);
    assert_eq!(ids[1], ids[0]);
    assert_ne!(ids[1].as_deref(), Some("hV3kQ9mZr2pX7wLc0tBnYA"));
    let bad = r#"{"body": "More.", "actor": {"id": "bun-pins"}}"#;
    match db.append(&key, &agent_event("note", bad), &common::unnamed(), None) {
        Err(StoreError::Refused(error)) => assert_eq!(error.category(), Category::Invalid),
        other => panic!("{other:?}"),
    }
}

/// An actor id is 22 base64url characters, and each actor's is its own.
#[test]
fn actor_ids_have_the_shape_the_core_checks() {
    let (_dir, db) = scratch();
    let (key, _) = create(&db, "fyi");
    for name in ["a", "b", "c"] {
        let note = agent_event("note", r#"{"body": "More."}"#);
        db.append(&key, &note, &named(name), None).unwrap();
    }
    let mut ids: Vec<String> = db.actors().unwrap().into_iter().map(|a| a.id).collect();
    for id in &ids {
        assert_eq!(id.len(), 22, "{id}");
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4);
}
