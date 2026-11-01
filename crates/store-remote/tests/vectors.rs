//! Runs every vector in `spec/vectors/` through a remote store over HTTP,
//! as `cases-store-sqlite`'s `tests/vectors.rs` runs them through a file.
//! Each vector is a test of its own, with a store of its own on the fake
//! server.
//!
//! A vector with an append sends it through [`Store::append`], or
//! [`Store::create`] for an open, with the vector's time as the store's
//! clock, so every refusal's category is checked after a trip over HTTP.
//! The store must refuse it with the category the vector gives and store
//! nothing, or take it and give the case the core gives for the same append
//! by the store's actor for the writer, and store the row the core gives,
//! byte for byte.
//!
//! A replay writes the vector's rows straight into the fake's store and
//! reads them back over HTTP: they must come back as stored, byte for byte,
//! raw records, escapes and all.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

#[path = "../../core/tests/common/mod.rs"]
mod vectors;

#[path = "../../store-sqlite/tests/common/mod.rs"]
mod sqlite;

mod fake;

use std::sync::{Arc, OnceLock};

use cases_core::{Actor, Append, Author, Case, Key, Row, Store, StoreError, Time, Writer};
use cases_store_remote::{Client, RemoteStore};
use cases_store_sqlite::Db;
use fake::{Fake, TOKEN};
use libtest_mimic::{Arguments, Failed, Trial};
use serde_json::Value;

fn main() {
    let args = Arguments::from_args();
    let mut trials = Vec::new();
    match vectors::load_all() {
        Ok(files) => {
            for file in files {
                for vector in file.vectors {
                    let name = format!("{}.json: {}", file.name, vector.description);
                    trials.push(Trial::test(name, move || run(&vector)));
                }
            }
        }
        Err(errors) => {
            let errors = errors.join("\n");
            trials.push(Trial::test("the vectors load", move || Err(errors.into())));
        }
    }
    libtest_mimic::run(&args, trials).exit();
}

/// The one fake server the vectors share.
fn fake() -> &'static Fake {
    static FAKE: OnceLock<Fake> = OnceLock::new();
    FAKE.get_or_init(|| Fake::start_kept("vectors"))
}

/// A new store on the fake, with its clock at `now`: the remote store, and
/// the fake's own store behind it.
fn scratch(now: Time) -> (RemoteStore, Arc<Db>) {
    let fake = fake();
    let id = fake.add_store("vector");
    fake.set_clock(&id, now);
    let client = Client::new(fake.url().parse().unwrap(), TOKEN.parse().unwrap());
    (client.store(&id).unwrap(), fake.db(&id))
}

/// The stored row a vector's row gives.
fn row(row: &vectors::Row) -> Row {
    Row {
        seq: row.seq,
        author: row.author.clone(),
        event: row.event.clone(),
        data: row.record(),
    }
}

fn run(vector: &vectors::Vector) -> Result<(), Failed> {
    let rows: Vec<Row> = vector.events.iter().map(row).collect();
    match &vector.append {
        None => replay(&rows),
        Some(append) => write(vector, &rows, append),
    }
}

/// Stores `rows` as a new case in `db`, as given, with none of the store's
/// checks.
fn store(db: &Db, rows: &[Row]) -> Key {
    let rows: Vec<(u64, &str, &str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row.seq,
                row.author.as_str(),
                row.event.as_str(),
                row.data.as_str(),
            )
        })
        .collect();
    sqlite::insert_case(db, &rows)
}

/// The store gives back, over HTTP, the rows written into it.
fn replay(rows: &[Row]) -> Result<(), Failed> {
    let (remote, db) = scratch(fake::START.parse()?);
    remote.create(&sqlite::open_of("fyi"), &Writer::new(None))?;
    let key = store(&db, rows);
    let back = remote.rows(&key)?.ok_or("the case is gone")?;
    if back != rows {
        return Err(format!(
            "the store gave back\n{back:#?}\nnot the rows it was given\n{rows:#?}"
        )
        .into());
    }
    let listed = remote.list()?;
    if listed.iter().find(|(k, _)| *k == key).map(|(_, r)| r) != Some(&back) {
        return Err("the list does not give the rows the case gives".into());
    }
    Ok(())
}

/// Sends the vector's append through the remote store, and checks it
/// against the core's writer.
fn write(vector: &vectors::Vector, rows: &[Row], append: &vectors::Append) -> Result<(), Failed> {
    let now: Time = append.now.parse()?;
    let new = Append {
        author: append.author.clone(),
        event: append.event.clone(),
        data: Value::Object(append.data.clone()).to_string(),
    };
    let writer = Writer::new(None);
    let want = vector.expect_error.as_ref().map(|e| e.category.as_str());
    let (remote, db) = scratch(now);

    if append.event == "open" {
        return match remote.create(&new, &writer) {
            Err(StoreError::Refused(error)) => {
                same_category(error.category().as_str(), want)?;
                let core = Case::open(&new, now, None).err().map(|e| e.category());
                if core != Some(error.category()) {
                    return Err(format!(
                        "the store refused the open as {}, and the core {core:?}",
                        error.category()
                    )
                    .into());
                }
                if !remote.list()?.is_empty() || !remote.actors()?.is_empty() {
                    return Err("the refused open stored something".into());
                }
                Ok(())
            }
            Ok((key, case)) => {
                if let Some(want) = want {
                    return Err(format!("the store took the open, want {want}").into());
                }
                let actor = actor(&remote, &new.author)?.ok_or("the open made no actor")?;
                let (core, row) = Case::open(&new, now, Some(&actor))?;
                same_case(&case, &core)?;
                let stored = remote.rows(&key)?.ok_or("the case is gone")?;
                same_rows(&stored, &[row])
            }
            Err(error) => Err(format!("the store failed: {error:?}").into()),
        };
    }

    // The store, and the case as the vector gives its rows.
    remote.create(&sqlite::open_of("fyi"), &writer)?;
    let key = store(&db, rows);
    let actors = remote.actors()?;
    let outcome = remote.append(&key, &new, &writer, append.at_revision);
    let stored = remote.rows(&key)?.ok_or("the case is gone")?;
    let actor = match actor(&remote, &new.author)? {
        Some(actor) => Some(actor),
        None if new.author.parse::<Author>().is_ok() => {
            let mut stand_in = Actor::default();
            stand_in.id = "AAAAAAAAAAAAAAAAAAAAAA".into();
            Some(stand_in)
        }
        None => None,
    };
    let core = Case::fold(rows).and_then(|mut case| {
        let row = case.append(&new, now, actor.as_ref(), append.at_revision)?;
        Ok((case, row))
    });
    match (outcome, core) {
        (Ok(case), Ok((core, row))) => {
            if let Some(want) = want {
                return Err(format!("the store took the append, want {want}").into());
            }
            same_case(&case, &core)?;
            let mut all = rows.to_vec();
            all.push(row);
            same_rows(&stored, &all)?;
            same_case(&Case::fold(&stored)?, &case)
        }
        (Err(StoreError::Refused(error)), Err(core)) => {
            same_category(error.category().as_str(), want)?;
            if error.category() != core.category() {
                return Err(format!(
                    "the store refused the append as {}, and the core as {}",
                    error.category(),
                    core.category()
                )
                .into());
            }
            if remote.actors()? != actors {
                return Err("the refused append made an actor".into());
            }
            same_rows(&stored, rows)
        }
        (outcome, core) => Err(format!("the store gave {outcome:?}, and the core {core:?}").into()),
    }
}

/// The unnamed actor the store keeps for events of `author`, if it has made
/// it.
fn actor(remote: &RemoteStore, author: &str) -> Result<Option<Actor>, Failed> {
    let found = remote
        .actors()?
        .into_iter()
        .find(|row| row.kind == author && row.name.is_none());
    Ok(found.map(|row| {
        let mut actor = Actor::default();
        actor.id = row.id;
        actor
    }))
}

fn same_category(have: &str, want: Option<&str>) -> Result<(), Failed> {
    match want {
        Some(want) if want == have => Ok(()),
        Some(want) => Err(format!("refused as {have}, want {want}").into()),
        None => Err(format!("refused as {have}, want a case").into()),
    }
}

fn same_case(have: &Case, want: &Case) -> Result<(), Failed> {
    if have == want {
        Ok(())
    } else {
        Err(format!("the store's case\n{have:#?}\nis not the core's\n{want:#?}").into())
    }
}

fn same_rows(have: &[Row], want: &[Row]) -> Result<(), Failed> {
    if have == want {
        Ok(())
    } else {
        Err(format!("the store holds\n{have:#?}\nnot\n{want:#?}").into())
    }
}
