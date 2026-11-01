//! The web inbox's freshness fixtures: each vector of
//! `spec/vectors/freshness.json`, with its cases as `show --json` and the API
//! give them, in `serve/web/test/freshness.json`. Node cannot fold rows, so
//! the web's port of `Case::since` is tested on what this writes.
//!
//! The file is checked in. This test refuses it when it is not what the
//! vectors give now; `CASES_WRITE_FIXTURES=1` writes it instead.

use std::fs;
use std::path::Path;

use cases_core::{Case, Row};
use cases_store_sqlite::Key;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{Actors, CaseJson};

/// The vectors, from the crate's directory.
const VECTORS: &str = "../../spec/vectors/freshness.json";
/// The fixture file, from the crate's directory.
const FIXTURE: &str = "src/commands/serve/web/test/freshness.json";
/// Set to write the fixture file rather than compare it.
const WRITE: &str = "CASES_WRITE_FIXTURES";

/// The fixture file.
#[derive(Serialize)]
struct Fixture<'a> {
    about: &'static str,
    vectors: Vec<Entry<'a>>,
}

/// One vector: the revision and seq its form was drawn at, the case it was
/// drawn from when the vector folds one (`drawn_after`), the case its rows
/// fold to, and how that case changed since the form was drawn.
#[derive(Serialize)]
struct Entry<'a> {
    description: &'a str,
    drawn: Drawn,
    #[serde(skip_serializing_if = "Option::is_none")]
    drawn_case: Option<CaseJson<'a>>,
    case: CaseJson<'a>,
    since: &'a str,
}

#[derive(Serialize)]
struct Drawn {
    revision: u64,
    seq: u64,
}

/// A vector's rows, with a record given as `data` written as JSON, or as
/// `raw`, its exact bytes.
fn rows(vector: &Map<String, Value>) -> Vec<Row> {
    vector["events"]
        .as_array()
        .expect("events is a list")
        .iter()
        .map(|row| Row {
            seq: row["seq"].as_u64().expect("a row has a seq"),
            author: row["author"]
                .as_str()
                .expect("a row has an author")
                .to_owned(),
            event: row["event"]
                .as_str()
                .expect("a row has an event")
                .to_owned(),
            data: match row.get("raw") {
                Some(raw) => raw.as_str().expect("raw is text").to_owned(),
                None => row["data"].to_string(),
            },
        })
        .collect()
}

#[test]
fn the_web_s_freshness_fixtures_are_the_vectors_as_case_json() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(dir.join(VECTORS)).expect("read the freshness vectors");
    let vectors: Vec<Map<String, Value>> = serde_json::from_str(&text).expect("vectors parse");
    let key = Key::new(1).expect("1 is a key");
    let actors = Actors::new(Vec::new());

    let folded: Vec<(Vec<Row>, Case, Option<Case>)> = vectors
        .iter()
        .map(|vector| {
            let rows = rows(vector);
            let case = Case::fold(&rows).expect("a vector's rows fold");
            let drawn = vector.get("drawn_after").map(|n| {
                let n = usize::try_from(n.as_u64().expect("drawn_after is a number"))
                    .expect("drawn_after fits");
                Case::fold(&rows[..n]).expect("the drawn rows fold")
            });
            (rows, case, drawn)
        })
        .collect();
    let entries = vectors
        .iter()
        .zip(&folded)
        .map(|(vector, (_, case, drawn_case))| {
            let description = vector["description"].as_str().expect("a description");
            let drawn = match (vector.get("drawn_at"), drawn_case) {
                (Some(at), _) => Drawn {
                    revision: at["revision"].as_u64().expect("a revision"),
                    seq: at["seq"].as_u64().expect("a seq"),
                },
                (None, Some(c)) => Drawn {
                    revision: c.revision(),
                    seq: c.last_seq(),
                },
                (None, None) => panic!("{description}: no drawn_after or drawn_at"),
            };
            let since = vector["expect"]["since"]
                .as_str()
                .expect("expect gives since");
            // The fixture says what the core says, as the core's vectors do.
            assert_eq!(
                case.since(drawn.revision, drawn.seq).as_str(),
                since,
                "{description}"
            );
            Entry {
                description,
                drawn,
                drawn_case: drawn_case
                    .as_ref()
                    .map(|c| CaseJson::new(key, c, &actors).expect("the drawn case as JSON")),
                case: CaseJson::new(key, case, &actors).expect("the case as JSON"),
                since,
            }
        })
        .collect();
    let fixture = Fixture {
        about: "Generated from spec/vectors/freshness.json by crates/cli/src/output/freshness.rs; \
                do not edit. Regenerate with CASES_WRITE_FIXTURES=1 cargo test -p cases freshness.",
        vectors: entries,
    };
    let mut want = serde_json::to_string_pretty(&fixture).expect("the fixture as JSON");
    want.push('\n');

    let path = dir.join(FIXTURE);
    if std::env::var_os(WRITE).is_some() {
        fs::write(&path, &want).expect("write the fixture");
        return;
    }
    let have = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{FIXTURE} is not what spec/vectors/freshness.json gives now: \
         regenerate it with {WRITE}=1 cargo test -p cases freshness, and check it in"
    );
}
