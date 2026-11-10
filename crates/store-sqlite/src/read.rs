//! Reading cases' rows.

use std::collections::BTreeSet;
use std::path::Path;

use cases_core::{Changes, Key, Row, Version};
use rusqlite::{OptionalExtension, Transaction, params_from_iter};

use crate::error::Error;
use crate::key::{RowId, from_row};

/// The rows of the cases with the keys `keys`, or of every case when `keys`
/// is `None`, in key order: the order the cases were opened. Each case's
/// rows are read with it in one statement, in seq order, so a case is never
/// read part-way through a write. A case with no rows is given with none.
pub(crate) fn load_cases(
    tx: &Transaction<'_>,
    path: &Path,
    keys: Option<&[Key]>,
) -> Result<Vec<(Key, Vec<Row>)>, Error> {
    let only = match keys {
        Some(keys) => format!("WHERE c.key IN ({})", vec!["?"; keys.len()].join(", ")),
        None => String::new(),
    };
    let sql_keys = keys.unwrap_or_default().iter().map(|key| key.sql());
    load(tx, path, &only, params_from_iter(sql_keys))
}

/// The rows of the cases with a row stored after the change `after`, as
/// [`load_cases`] gives them, in one statement however many there are.
fn load_changed(
    tx: &Transaction<'_>,
    path: &Path,
    after: i64,
) -> Result<Vec<(Key, Vec<Row>)>, Error> {
    load(
        tx,
        path,
        "WHERE c.key IN (SELECT case_key FROM events WHERE change > ?1)",
        [after],
    )
}

/// The rows of the cases `only`, a `WHERE` clause on `cases c`, picks.
fn load(
    tx: &Transaction<'_>,
    path: &Path,
    only: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<(Key, Vec<Row>)>, Error> {
    let mut statement = tx.prepare(&format!(
        "SELECT c.key, e.seq, e.author, e.event, e.data
         FROM cases c LEFT JOIN events e ON e.case_key = c.key
         {only} ORDER BY c.key, e.seq"
    ))?;
    let mut found = statement.query(params)?;
    let mut cases: Vec<(Key, Vec<Row>)> = Vec::new();
    while let Some(found) = found.next()? {
        let key = from_row(found.get(0)?, path)?;
        let row = read_row(found, path)?;
        match cases.last_mut() {
            Some((last, rows)) if *last == key => rows.extend(row),
            _ => cases.push((key, row.into_iter().collect())),
        }
    }
    Ok(cases)
}

/// Where the store is, as its version says it:
/// `<store>.<change>.<key>.<count>.<tag>`.
#[derive(Debug, PartialEq, Eq)]
struct At {
    /// The store's id, from `meta`.
    store: String,
    /// The highest change, of an event or a removal: one counter, which
    /// only goes up and which a write that fails rolls back.
    change: i64,
    /// The highest key given to a case.
    key: i64,
    /// The number of cases in the store.
    count: i64,
    /// The time of the row that holds `change`: an event's time, or a
    /// removal's. It tells this store from a copy put back from before
    /// `change` and written past it again, which gives the same change to
    /// another row.
    tag: String,
}

impl At {
    fn read(tx: &Transaction<'_>) -> Result<At, Error> {
        // A store not upgraded yet, as one that could not be written when
        // it was opened, has no id and no logs: no id, no tag.
        let store = crate::schema::upgraded(tx)?;
        // The highest change and key, each 0 before its first, and the
        // number of cases, in one statement.
        let (change, key, count) = tx.query_row(
            "SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'events'), 0),
                    coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'cases'), 0),
                    (SELECT count(*) FROM cases)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let tag = if store.is_some() {
            tag(tx, change)?
        } else {
            String::new()
        };
        Ok(At {
            store: store.unwrap_or_default(),
            change,
            key,
            count,
            tag,
        })
    }

    fn version(&self) -> Version {
        let At {
            store,
            change,
            key,
            count,
            tag,
        } = self;
        Version::new(format!("{store}.{change}.{key}.{count}.{tag}"))
    }

    /// The place `version` names, when it is one this store's versions
    /// have the form of.
    fn parse(version: &Version) -> Option<At> {
        let mut parts = version.as_str().splitn(5, '.');
        Some(At {
            store: parts.next()?.to_owned(),
            change: parts.next()?.parse().ok()?,
            key: parts.next()?.parse().ok()?,
            count: parts.next()?.parse().ok()?,
            tag: parts.next()?.to_owned(),
        })
    }
}

/// The time of the row that holds the change `change`, where it is now:
/// an event, in the store or the archive, a removal, or an event a delete
/// took with its case. Empty when none does, such as for 0.
fn tag(tx: &Transaction<'_>, change: i64) -> Result<String, Error> {
    let found: Option<String> = tx
        .query_row(
            "SELECT at FROM events WHERE change = ?1
             UNION ALL SELECT at FROM archive_events WHERE change = ?1
             UNION ALL SELECT at FROM removed WHERE change = ?1
             UNION ALL SELECT at FROM gone WHERE change = ?1
             LIMIT 1",
            [change],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.unwrap_or_default())
}

/// The store's version, as [`Store::version`](cases_core::Store::version)
/// gives it: `<store>.<change>.<key>.<count>.<tag>`, of the store's id, the
/// highest change, the highest key, the number of cases, and the time of
/// the change's row.
///
/// Why two states never share a version. Every event stored and every
/// case taken out of `cases` raises the change, from one counter that
/// never goes back ([`REMOVALS`](crate::schema::REMOVALS)); a write that
/// fails rolls it back with the rest. What the store never does itself,
/// putting a case back from the archive by hand, changes the count. The
/// store's id tells it from a store made again at the path, and the tag
/// from a copy put back from before the change and written past it again.
/// That last is a guard, not a proof: the tag is the row's time as stored,
/// so a copy put back by hand and written past by a row stamped with the
/// very time of the row it replaced, such as a record replayed with its
/// own time, is taken for the same store. A stronger tag would need every
/// writer, older builds included, to write one.
pub(crate) fn version(tx: &Transaction<'_>) -> Result<Version, Error> {
    Ok(At::read(tx)?.version())
}

/// The cases changed since `since`, as
/// [`Store::rows_since`](cases_core::Store::rows_since) gives them. What
/// changed, from the rows stored and the removals logged since, when
/// `since` is a version of this store that it can still read changes from:
/// its id, a change it has reached, the same row at that change, every
/// case it had there still here but for those logged as removed, and no
/// case put in or back by hand since. Every case otherwise.
pub(crate) fn changes(
    tx: &Transaction<'_>,
    path: &Path,
    since: Option<&Version>,
) -> Result<Changes<Key>, Error> {
    let now = At::read(tx)?;
    let version = now.version();
    if since == Some(&version) {
        return Ok(Changes::Unchanged { version });
    }
    let from = since.and_then(At::parse).filter(|from| {
        from.store == now.store && !from.store.is_empty() && from.change <= now.change
    });
    let Some(from) = from else {
        return Ok(Changes::All {
            version,
            cases: load_cases(tx, path, None)?,
        });
    };
    let removed = query_keys(
        tx,
        path,
        "SELECT key FROM removed WHERE change > ?1 ORDER BY change",
        from.change,
    )?;
    let mut seen = BTreeSet::new();
    let removed: Vec<Key> = removed
        .into_iter()
        .filter(|key| seen.insert(*key))
        .collect();
    let old = |key: &Key| i64::try_from(key.get()).is_ok_and(|k| k <= from.key);
    let taken_out =
        i64::try_from(removed.iter().filter(|key| old(key)).count()).unwrap_or(i64::MAX);
    // The cases here now: those with keys given by `since`, and those with
    // keys given after it, each of which has a row stored after it.
    let (still_here, new): (i64, i64) = tx.query_row(
        "SELECT coalesce(sum(key <= ?1), 0), coalesce(sum(key > ?1), 0) FROM cases",
        [from.key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cases = load_changed(tx, path, from.change)?;
    let changed_new =
        i64::try_from(cases.iter().filter(|(key, _)| !old(key)).count()).unwrap_or(i64::MAX);
    // Every case `since` had is here or logged as removed, no other case
    // with a key it had is here, every case with a later key has a row
    // stored since, and none is both changed and removed: otherwise a case
    // was put in or back by hand, and what changed is not all in the logs.
    let whole = tag(tx, from.change)? == from.tag
        && still_here.saturating_add(taken_out) == from.count
        && new == changed_new
        && !cases.iter().any(|(key, _)| seen.contains(key));
    if !whole {
        return Ok(Changes::All {
            version,
            cases: load_cases(tx, path, None)?,
        });
    }
    Ok(Changes::Since {
        version,
        cases,
        removed,
    })
}

/// The keys a query with one argument gives, one to a row.
fn query_keys(
    tx: &Transaction<'_>,
    path: &Path,
    query: &str,
    argument: i64,
) -> Result<Vec<Key>, Error> {
    let mut statement = tx.prepare(query)?;
    let found = statement.query_map([argument], |row| row.get::<_, i64>(0))?;
    found.map(|key| from_row(key?, path)).collect()
}

/// The row in columns 1 to 4 of `found`, or `None` when they are empty: a
/// case with no rows.
fn read_row(found: &rusqlite::Row<'_>, path: &Path) -> Result<Option<Row>, Error> {
    let Some(seq) = found.get::<_, Option<i64>>(1)? else {
        return Ok(None);
    };
    let seq = u64::try_from(seq).map_err(|_| Error::Damaged {
        path: path.to_owned(),
        what: format!("a row has the seq {seq}"),
    })?;
    let data: Vec<u8> = found.get(4)?;
    Ok(Some(Row {
        seq,
        author: found.get(2)?,
        event: found.get(3)?,
        // The store writes UTF-8. Bytes that are not, which only another
        // writer can store, read as U+FFFD, so the rest of the record reads.
        data: String::from_utf8(data)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cases_core::{Category, Changes, Store, StoreError, Version};

    use crate::key::RowId;
    use crate::testing::{count, create, event, note, scratch, sql};
    use crate::{Error, Key, Writer};

    #[test]
    fn the_version_moves_on_every_event_archive_and_delete_and_not_on_listening() {
        let (_dir, db) = scratch();
        let zero = Version::new("0");
        // A store not made yet is at 0, with no cases.
        assert_eq!(db.version().unwrap(), zero);
        assert_eq!(
            db.rows_since(None).unwrap(),
            Changes::All {
                version: zero.clone(),
                cases: Vec::new()
            }
        );
        assert_eq!(
            db.rows_since(Some(&zero)).unwrap(),
            Changes::Unchanged {
                version: zero.clone()
            }
        );

        let mut seen = HashSet::from([zero]);
        let mut moved = |db: &crate::Db, why: &str| {
            assert!(seen.insert(db.version().unwrap()), "{why}");
        };
        let one = create(&db);
        moved(&db, "an open");
        note(&db, one);
        moved(&db, "a note");
        let two = create(&db);
        moved(&db, "a second open");

        // Listening writes no event: a listen, its heartbeat, a doing line.
        let before = db.version().unwrap();
        db.listen("w1", "lead", &[], &[], Some("reading")).unwrap();
        db.listen("w1", "lead", &[], &[], None).unwrap();
        db.set_doing("lead", "writing").unwrap();
        db.set_doing("lead", "").unwrap();
        assert_eq!(db.version().unwrap(), before);

        // An archive takes the case out of the cases table.
        db.archive(&two, None).unwrap();
        assert_eq!(count(&db, "cases"), 1);
        moved(&db, "an archive");
        db.delete(&one, None).unwrap();
        // No cases, but the store was written: not 0 again.
        moved(&db, "a delete of the last case");
        // An open after a delete has a new key and a new change.
        let three = create(&db);
        moved(&db, "an open after the delete");
        db.delete(&three, None).unwrap();
        moved(&db, "a delete after that");
    }

    /// A write that is refused, or fails, stores nothing, so the version
    /// stays.
    #[test]
    fn a_refused_write_archive_or_delete_leaves_the_version() {
        let (_dir, db) = scratch();
        let one = create(&db);
        note(&db, one);
        let before = db.version().unwrap();
        let stale = |r: Result<(), StoreError<Error>>| {
            let Err(StoreError::Refused(error)) = r else {
                panic!("not refused: {r:?}");
            };
            assert_eq!(error.category(), Category::Stale);
        };
        stale(db.archive(&one, Some(1)));
        stale(db.delete(&one, Some(1)));
        let missing = Key::new(99).unwrap();
        assert!(matches!(
            db.archive(&missing, None),
            Err(StoreError::NoCase)
        ));
        assert!(matches!(db.delete(&missing, None), Err(StoreError::NoCase)));
        // A case whose key the archive holds already, as after a case is put
        // back from the archive by hand, is not archived.
        sql(&db)
            .execute(
                "INSERT INTO archive_cases (key, opened_at, archived_at) VALUES (?1, '', '')",
                [one.sql()],
            )
            .unwrap();
        assert!(matches!(
            db.archive(&one, None),
            Err(StoreError::Store(Error::Archived { .. }))
        ));
        // An event the writer refuses: a close of a case not picked up.
        let close = event("agent", "close", r#"{"outcome": "Done"}"#);
        assert!(db.append(&one, &close, &Writer::new(None), None).is_err());
        // Writes whose commit fails roll back, the change they took too.
        db.fail_commit
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let more = event("agent", "note", r#"{"body": "More."}"#);
        assert!(db.append(&one, &more, &Writer::new(None), None).is_err());
        assert!(db.delete(&one, None).is_err());
        db.fail_commit
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(db.version().unwrap(), before);
        assert_eq!(count(&db, "cases"), 1);
    }

    /// The trait's actors and listening are the store's own.
    #[test]
    fn the_trait_gives_the_store_s_actors_and_listening() {
        let (_dir, db) = scratch();
        create(&db);
        let store: &dyn Store<Key = Key, Writer = Writer, Error = Error> = &db;
        assert_eq!(store.actors().unwrap(), db.actors().unwrap());
        assert_eq!(store.actors().unwrap().len(), 1);
        store
            .listen("w1", "lead", &["ci".into()], &[], Some("reading"))
            .unwrap();
        store.set_doing("lead", "writing").unwrap();
        let waits = store.waits().unwrap();
        assert_eq!(waits.len(), 1);
        assert_eq!(
            (waits[0].id.as_str(), waits[0].worker.as_str()),
            ("w1", "lead")
        );
        let listening = store.listeners().unwrap();
        assert_eq!(listening[0].doing.as_deref(), Some("writing"));
        assert!(store.set_doing("other", "x").is_err());
    }

    /// The id in `meta`.
    fn store_id(db: &crate::Db) -> String {
        sql(db)
            .query_row("SELECT store_id FROM meta", [], |row| row.get(0))
            .unwrap()
    }

    /// The version's parts: the store's id, the highest change, the highest
    /// key, the count of cases, and the time of the change's row.
    #[test]
    fn the_version_is_the_store_the_change_the_key_the_count_and_the_row_s_time() {
        let (_dir, db) = scratch();
        let one = create(&db);
        let id = store_id(&db);
        assert_eq!(id.len(), 22);
        let at: String = sql(&db)
            .query_row("SELECT at FROM events WHERE change = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(db.version().unwrap().as_str(), format!("{id}.1.1.1.{at}"));
        note(&db, one);
        create(&db);
        db.delete(&one, None).unwrap();
        let at: String = sql(&db)
            .query_row("SELECT at FROM removed WHERE change = 4", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(db.version().unwrap().as_str(), format!("{id}.4.2.1.{at}"));
    }

    #[test]
    fn a_case_taken_out_and_another_put_back_by_hand_moves_the_version() {
        let (_dir, db) = scratch();
        let one = create(&db);
        create(&db);
        let before = db.version().unwrap();
        // The same count and the same highest change, but other keys.
        let conn = sql(&db);
        conn.execute("DELETE FROM events WHERE case_key = ?1", [one.sql()])
            .unwrap();
        conn.execute("DELETE FROM cases WHERE key = ?1", [one.sql()])
            .unwrap();
        conn.execute("INSERT INTO cases (key, opened_at) VALUES (9, '')", [])
            .unwrap();
        assert_ne!(db.version().unwrap(), before);
    }

    #[test]
    fn rows_since_gives_what_changed_since_a_version_of_this_store() {
        let (_dir, db) = scratch();
        let one = create(&db);
        let two = create(&db);
        let three = create(&db);
        let then = db.version().unwrap();
        assert_eq!(
            db.rows_since(Some(&then)).unwrap(),
            Changes::Unchanged {
                version: then.clone()
            }
        );
        note(&db, two);
        db.archive(&one, None).unwrap();
        let four = create(&db);
        let five = create(&db);
        db.delete(&five, None).unwrap();
        db.delete(&three, None).unwrap();
        let Changes::Since {
            version,
            cases,
            removed,
        } = db.rows_since(Some(&then)).unwrap()
        else {
            panic!("not what changed");
        };
        let keys: Vec<Key> = cases.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, [two, four]);
        assert_eq!(cases[0].1.len(), 2, "the case with its note");
        assert_eq!(removed, [one, five, three]);
        assert_eq!(
            count(&db, "gone"),
            2,
            "the events deleted, not those archived"
        );
        assert_eq!(version, db.version().unwrap());
        // With no version, or one of another form, every case.
        for since in [
            None,
            Some(Version::new("0")),
            Some(Version::new("x.1.1.1.t")),
        ] {
            let changes = db.rows_since(since.as_ref()).unwrap();
            assert!(matches!(changes, Changes::All { .. }), "{since:?}");
            assert_eq!(changes.cases(), db.list().unwrap());
        }
    }

    /// Many changed cases are read in one statement, as a few are, with no
    /// key bound for each.
    #[test]
    fn rows_since_gives_many_changed_cases_as_it_gives_a_few() {
        let (_dir, db) = scratch();
        let first = create(&db);
        let then = db.version().unwrap();
        let added: Vec<Key> = (0..150).map(|_| create(&db)).collect();
        let Changes::Since { cases, .. } = db.rows_since(Some(&then)).unwrap() else {
            panic!("not what changed");
        };
        let keys: Vec<Key> = cases.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, added);
        assert!(!keys.contains(&first));
    }

    /// Every case, not what changed, when the version may not be this
    /// store's past: another store's, a copy put back from before it, or a
    /// case put back by hand.
    #[test]
    fn rows_since_gives_every_case_for_a_past_it_cannot_trust() {
        let (_one, first) = scratch();
        let (_two, second) = scratch();
        create(&first);
        create(&second);
        let other = second.version().unwrap();
        assert!(matches!(
            first.rows_since(Some(&other)).unwrap(),
            Changes::All { .. }
        ));
        assert_ne!(
            first.version().unwrap().as_str().split_once('.').unwrap().0,
            other.as_str().split_once('.').unwrap().0,
            "two stores, two ids"
        );

        // A copy of the store from before a note, put back and written past
        // the version read after the note: the same change, another row.
        let (dir, db) = scratch();
        let key = create(&db);
        let backup = dir.path().join("backup.db");
        sql(&db)
            .execute("VACUUM INTO ?1", [backup.to_str().unwrap()])
            .unwrap();
        note(&db, key);
        let after_note = db.version().unwrap();
        drop(db);
        std::fs::copy(&backup, dir.path().join("cases.db")).unwrap();
        let db = crate::Db::new(dir.path().join("cases.db"));
        let note = event("agent", "note", r#"{"body": "Other."}"#);
        db.append(&key, &note, &Writer::new(Some("x")), None)
            .unwrap();
        let now = db.version().unwrap();
        assert_ne!(now, after_note);
        assert!(matches!(
            db.rows_since(Some(&after_note)).unwrap(),
            Changes::All { .. }
        ));

        // A case put back from the archive by hand.
        let (_dir, db) = scratch();
        let one = create(&db);
        create(&db);
        db.archive(&one, None).unwrap();
        let then = db.version().unwrap();
        let conn = sql(&db);
        for statement in [
            "INSERT INTO cases (key, opened_at) SELECT key, opened_at FROM archive_cases WHERE key = ?1",
            "INSERT INTO events (change, case_key, seq, author, event, at, data)
             SELECT change, case_key, seq, author, event, at, data FROM archive_events WHERE case_key = ?1",
        ] {
            conn.execute(statement, [one.sql()]).unwrap();
        }
        assert_ne!(db.version().unwrap(), then);
        let changes = db.rows_since(Some(&then)).unwrap();
        assert!(matches!(changes, Changes::All { .. }));
        assert_eq!(changes.cases().len(), 2);

        // A case opened since, archived, and put back by hand: it has rows
        // stored since and is logged as removed, and a key is never in both.
        let (_dir, db) = scratch();
        create(&db);
        let then = db.version().unwrap();
        let later = create(&db);
        db.archive(&later, None).unwrap();
        let conn = sql(&db);
        for statement in [
            "INSERT INTO cases (key, opened_at) SELECT key, opened_at FROM archive_cases WHERE key = ?1",
            "INSERT INTO events (change, case_key, seq, author, event, at, data)
             SELECT change, case_key, seq, author, event, at, data FROM archive_events WHERE case_key = ?1",
        ] {
            conn.execute(statement, [later.sql()]).unwrap();
        }
        let changes = db.rows_since(Some(&then)).unwrap();
        assert!(matches!(changes, Changes::All { .. }), "{changes:?}");
        assert_eq!(changes.cases().len(), 2);

        // A case put in by hand with a key past the version's, and rows
        // with changes from before it: nothing in the logs names it.
        let (_dir, db) = scratch();
        let one = create(&db);
        let then = db.version().unwrap();
        let conn = sql(&db);
        conn.execute("INSERT INTO cases (key, opened_at) VALUES (9, '')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO events (change, case_key, seq, author, event, at, data)
             SELECT 0, 9, seq, author, event, at, data FROM events WHERE case_key = ?1",
            [one.sql()],
        )
        .unwrap();
        let changes = db.rows_since(Some(&then)).unwrap();
        assert!(matches!(changes, Changes::All { .. }), "{changes:?}");
        assert_eq!(changes.cases().len(), 2);
    }
}
