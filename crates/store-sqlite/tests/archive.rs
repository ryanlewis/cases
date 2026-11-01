//! Archiving and deleting cases. The Go store's `archive_test.go`, as
//! `PARITY.md` maps it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use cases_core::{Case, Category, State, Store, StoreError};
use cases_store_sqlite::{Db, Error, Key};
use common::{
    agent_event, archived_rows, create, fixed, insert_row, scratch, sql, stored, unnamed,
};

/// Opens a case and withdraws it: a case at revision 2.
fn withdrawn(db: &Db) -> Key {
    let (key, _) = create(db, "fyi");
    let withdraw = agent_event("withdraw", r#"{"reason": "not needed"}"#);
    db.append(&key, &withdraw, &unnamed(), None).unwrap();
    key
}

#[test]
fn archive_moves_the_case_out_of_sight() {
    let (dir, _) = scratch();
    let db = fixed(&dir.path().join("cases.db"), "2026-09-17T08:00:00Z");
    let (kept, _) = create(&db, "stuck");
    let case = withdrawn(&db);
    let before = stored(&db, case);

    db.archive(&case, Some(2)).unwrap();
    assert_eq!(db.rows(&case).unwrap(), None);
    let listed: Vec<Key> = db.list().unwrap().into_iter().map(|(key, _)| key).collect();
    assert_eq!(listed, [kept]);
    assert!(stored(&db, case).is_empty(), "rows left behind");
    assert_eq!(
        archived_rows(&db, case),
        before,
        "the archive keeps the rows as written"
    );
    let archived_at: String = sql(&db)
        .query_row(
            "SELECT archived_at FROM archive_cases WHERE key = ?1",
            [common::key_sql(case)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived_at, "2026-09-17T08:00:00Z");

    // The archived case had the highest key, and a new case still gets a
    // key of its own: no key, and so no id, is given twice.
    let (again, _) = create(&db, "fyi");
    assert_eq!(again.get(), case.get() + 1);
}

/// A case whose key the archive holds already, as after a case is put back
/// by hand, is refused and left where it is.
#[test]
fn archive_refuses_a_key_the_archive_has() {
    let (_dir, db) = scratch();
    let case = withdrawn(&db);
    db.archive(&case, None).unwrap();
    let open = archived_rows(&db, case)[0].data.clone();
    insert_row(&db, common::key_sql(case), 1, "agent", "open", &open);

    match db.archive(&case, None) {
        Err(StoreError::Store(error @ Error::Archived { key })) => {
            assert_eq!(key, case);
            assert!(error.to_string().contains(&case.to_string()), "{error}");
        }
        other => panic!("archive: {other:?}"),
    }
    let rows = db.rows(&case).unwrap().unwrap();
    assert_eq!(Case::fold(&rows).unwrap().state(), State::Open);
    assert_eq!(archived_rows(&db, case).len(), 2, "the archive changed");
}

#[test]
fn delete_removes_the_case() {
    let (_dir, db) = scratch();
    let case = withdrawn(&db);
    db.delete(&case, Some(2)).unwrap();
    assert_eq!(db.rows(&case).unwrap(), None);
    assert!(stored(&db, case).is_empty());
    assert!(
        archived_rows(&db, case).is_empty(),
        "the delete archived the case"
    );
}

#[test]
fn archive_and_delete_refuse() {
    for archive in [true, false] {
        let remove = |db: &Db, key: Key, at: Option<u64>| {
            if archive {
                db.archive(&key, at)
            } else {
                db.delete(&key, at)
            }
        };
        let (_dir, db) = scratch();
        let missing = Key::new(900).unwrap();
        assert!(
            matches!(
                remove(&db, missing, None),
                Err(StoreError::Store(Error::Missing { .. }))
            ),
            "no store"
        );
        assert!(!db.path().exists(), "made the store");
        let case = withdrawn(&db);
        assert!(matches!(
            remove(&db, missing, None),
            Err(StoreError::NoCase)
        ));
        // Changed since it was read at revision 1.
        match remove(&db, case, Some(1)) {
            Err(StoreError::Refused(error)) => assert_eq!(error.category(), Category::Stale),
            other => panic!("stale: {other:?}"),
        }
        let rows = db.rows(&case).unwrap().unwrap();
        assert_eq!(Case::fold(&rows).unwrap().revision(), 2);
        assert!(archived_rows(&db, case).is_empty());
    }
}

/// The store-wide change only goes up, even when the cases that hold the
/// highest ones are archived or deleted (ST-01).
#[test]
fn the_change_only_goes_up() {
    let (_dir, db) = scratch();
    let (older, _) = create(&db, "stuck");
    let highest = |db: &Db| -> i64 {
        sql(db)
            .query_row("SELECT coalesce(max(change), 0) FROM events", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    let mut last = highest(&db);
    for archive in [true, false] {
        let newest = withdrawn(&db);
        let before = highest(&db);
        assert!(before > last);
        if archive {
            db.archive(&newest, None).unwrap();
        } else {
            db.delete(&newest, None).unwrap();
        }
        assert!(highest(&db) < before);
        let note = agent_event("note", r#"{"body": "More."}"#);
        db.append(&older, &note, &unnamed(), None).unwrap();
        last = highest(&db);
        assert!(last > before, "a change was given twice");
    }
}

/// A key is never given again, not even the highest one, once its case is
/// deleted or archived (ST-10, ST-13).
#[test]
fn the_highest_key_is_not_given_again_once_its_case_goes() {
    for archive in [true, false] {
        let (_dir, db) = scratch();
        create(&db, "fyi");
        let last = withdrawn(&db);
        if archive {
            db.archive(&last, Some(2)).unwrap();
        } else {
            db.delete(&last, Some(2)).unwrap();
        }
        let (next, _) = create(&db, "fyi");
        assert!(next > last, "archive {archive}: {next:?} after {last:?}");
    }
}
