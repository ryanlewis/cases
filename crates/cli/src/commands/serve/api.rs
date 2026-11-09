//! The API under `/v1` (spec/api.md): reading cases, the human's writes
//! (answer, park, resume, withdraw, and a message and its edit), who is
//! listening for messages, and the notification feed.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap};
use axum::response::Response;
use cases_core::{Append, Author, Case, Category, State as CaseState, Store, StoreError, Time};
use cases_store_sqlite::Key;
use serde::Serialize;
use serde_json::{Map, Value};

use super::app::{self, App, Refusal};
use super::page;
use crate::commands::list::{self, Listed};
use crate::instance;
use crate::output::{self, Actors, CaseJson};

/// The most a write's body may hold.
const MAX_BODY: usize = 1 << 20;

/// `GET /v1`: the build, which a page compares with its own (IN-12), and the
/// CSRF token its writes carry (IN-11).
pub(crate) async fn index(State(app): State<Arc<App>>) -> Result<Response, Refusal> {
    #[derive(Serialize)]
    struct Index<'a> {
        version: &'a str,
        csrf: &'a str,
    }
    app::ok(&Index {
        version: &page::BUILD,
        csrf: &app.csrf,
    })
}

/// `GET /v1/cases?state=…`: the cases in the states asked for, `open` and
/// `parked` by default, in inbox order: most urgent first, then in the
/// order they were opened (AG-14, IN-01).
pub(crate) async fn list(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
) -> Result<Response, Refusal> {
    let states = states(query.as_deref().unwrap_or_default())?;
    app::blocking(&app, move |app| {
        let snapshot = app.watch.read().map_err(Refusal::store)?;
        let actors = Actors::read(&app.db).map_err(Refusal::store)?;
        let list = list::inbox_order(&snapshot.cases, |case| states.contains(&case.state()))
            .into_iter()
            .map(|(key, case)| CaseJson::new(*key, case, &actors))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Refusal::store)?;
        app::ok(&Listed { list })
    })
    .await
}

/// The states `?state=` asks for: comma-separated and perhaps repeated, and
/// `open` and `parked` when it is not given.
fn states(query: &str) -> Result<Vec<CaseState>, Refusal> {
    let mut states: Option<Vec<CaseState>> = None;
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        if name != "state" {
            continue;
        }
        for state in value.split(',') {
            let state = state
                .parse()
                .map_err(|_| Refusal::bad(format!("state {state:?} is not a state")))?;
            states.get_or_insert_default().push(state);
        }
    }
    Ok(states.unwrap_or_else(|| list::WAITING.to_vec()))
}

/// `GET /v1/cases/{id}`: one case, as `show --json` gives it, with its page
/// in this inbox as its `url` (AG-13, IN-02).
pub(crate) async fn case(
    State(app): State<Arc<App>>,
    id: Result<Path<String>, PathRejection>,
) -> Result<Response, Refusal> {
    let key = key(id)?;
    app::blocking(&app, move |app| {
        let snapshot = app.watch.read().map_err(Refusal::store)?;
        let (_, case) = snapshot
            .cases
            .iter()
            .find(|(found, _)| *found == key)
            .ok_or_else(Refusal::no_case)?;
        let body = shown(app, key, case)?;
        Ok(app::json(StatusCode::OK, body))
    })
    .await
}

/// The case the path's `{id}` names by its whole id, with or without a
/// title slug after it (ST-10). The API never takes part of an id (HU-07).
/// An `{id}` that is not text, such as `%FF`, is no case's id either, so it
/// is the same 404 rather than axum's own refusal.
fn key(id: Result<Path<String>, PathRejection>) -> Result<Key, Refusal> {
    let Ok(Path(id)) = id else {
        return Err(Refusal::no_case());
    };
    id.parse().map_err(|_| Refusal::no_case())
}

/// The case `key` as `show --json` gives it, with its page in this inbox.
fn shown(app: &App, key: Key, case: &Case) -> Result<Vec<u8>, Refusal> {
    let actors = Actors::read(&app.db).map_err(Refusal::store)?;
    shown_with(app, key, case, &actors)
}

/// The case `key` as `show --json` gives it, with its actors named from
/// `actors`.
fn shown_with(app: &App, key: Key, case: &Case, actors: &Actors) -> Result<Vec<u8>, Refusal> {
    let url = instance::case_url(&app.link, &output::titled(key, case.title()));
    let json = CaseJson::new(key, case, actors)
        .map_err(Refusal::store)?
        .with_url(url);
    serde_json::to_vec(&json).map_err(Refusal::store)
}

/// A write the human makes from the inbox.
#[derive(Clone, Copy, Debug)]
enum Write {
    /// `POST /v1/cases/{id}/answer`: `{"revision": N, "answer": {…}}`, the
    /// answer record as HU-01 and HU-02 give it (IN-03).
    Answer,
    /// `POST /v1/cases/{id}/park`: `{"revision": N, "note": "…"}` (HU-03).
    Park,
    /// `POST /v1/cases/{id}/resume`: `{"revision": N}` (HU-04).
    Resume,
    /// `POST /v1/cases/{id}/withdraw`: `{"revision": N, "reason": "…"}`:
    /// an open case, a message no agent has picked up, or a case an agent
    /// has, answered or picked up (ST-14).
    Withdraw,
    /// `POST /v1/cases/{id}/amend`: `{"revision": N, "body": "…"}`, and the
    /// amend's other fields: the human's edit of a message no agent has
    /// picked up (IN-13).
    Amend,
}

/// `POST /v1/cases/{id}/{write}`: makes a write, checked in the order
/// spec/api.md gives: the case exists (404), the body is the JSON the route
/// takes (400), its revision is the case's (409), then what the core checks
/// (409 for stale, 422 for the rest). A refused write writes nothing. A
/// write the page sent runs to the end even if the page goes away (IN-03):
/// the log runs every request on a task of its own. Any other `{write}` is
/// no path the API has.
pub(crate) async fn write(
    State(app): State<Arc<App>>,
    path: Result<Path<(String, String)>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, Refusal> {
    let Ok(Path((id, write))) = path else {
        return Err(Refusal::no_case());
    };
    let write = match write.as_str() {
        "answer" => Write::Answer,
        "park" => Write::Park,
        "resume" => Write::Resume,
        "withdraw" => Write::Withdraw,
        "amend" => Write::Amend,
        _ => return Err(Refusal::request(StatusCode::NOT_FOUND, "no such path")),
    };
    let key = id.parse::<Key>().map_err(|_| Refusal::no_case())?;
    app::blocking(&app, move |app| match app.db.rows(&key) {
        Ok(Some(_)) => Ok(()),
        Ok(None) | Err(cases_store_sqlite::Error::Missing { .. }) => Err(Refusal::no_case()),
        Err(error) => Err(Refusal::store(error)),
    })
    .await?;
    if !is_json(&headers) {
        return Err(Refusal::bad("a write's body must be application/json"));
    }
    let bytes = read_body(body, app.timeouts.read).await?;
    app::blocking(&app, move |app| {
        let (revision, append) = request(write, &bytes)?;
        let Some(revision) = revision else {
            return Err(refused(
                app,
                key,
                Category::Stale,
                "the write has no revision",
            ));
        };
        match app.db.append(&key, &append, &app.writer, Some(revision)) {
            // The write is made, so it is a 200 even when the actors cannot
            // be read now: they are then shown by their ids alone.
            Ok(case) => {
                let actors = Actors::read(&app.db).unwrap_or_default();
                Ok(app::json(
                    StatusCode::OK,
                    shown_with(app, key, &case, &actors)?,
                ))
            }
            Err(StoreError::Refused(error)) => {
                Err(refused(app, key, error.category(), &error.to_string()))
            }
            Err(
                StoreError::NoCase | StoreError::Store(cases_store_sqlite::Error::Missing { .. }),
            ) => Err(Refusal::no_case()),
            Err(error) => Err(Refusal::store(error)),
        }
    })
    .await
}

/// `POST /v1/cases`: the human sends a message (IN-13): the body is its open
/// record, `{"kind": "message", "urgency", "title", "body", "worker",
/// "labels", "links", "context", "re"}`, which serve writes as its human, and
/// stamps. The checks are a write's, but for the revision, which a new case
/// has none of: the body is the JSON the route takes (400), `re`, when
/// given, names a case in the store by its whole id (400), then what the
/// core checks (422, or 400 for a record that does not read). The core
/// refuses the human's open of any other kind, as `author`.
pub(crate) async fn send(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, Refusal> {
    if !is_json(&headers) {
        return Err(Refusal::bad("a write's body must be application/json"));
    }
    let bytes = read_body(body, app.timeouts.read).await?;
    app::blocking(&app, move |app| {
        let Ok(Value::Object(mut record)) = serde_json::from_slice::<Value>(&bytes) else {
            return Err(Refusal::bad("the body is not a JSON object"));
        };
        stamped(&record)?;
        // The case a message is about is kept by its bare id. A store that
        // cannot say whether it has the case fails as a write's check does,
        // rather than blame the `re`.
        if let Some(Value::String(re)) = record.get("re")
            && !re.is_empty()
        {
            let none = || Refusal::bad("re names no case in this store");
            let key = re.parse::<Key>().map_err(|_| none())?;
            match app.db.rows(&key) {
                Ok(Some(_)) => {}
                Ok(None) | Err(cases_store_sqlite::Error::Missing { .. }) => return Err(none()),
                Err(error) => return Err(Refusal::store(error)),
            }
            record.insert("re".into(), Value::String(key.to_string()));
        }
        let append = Append {
            author: Author::Human.as_str().into(),
            event: "open".into(),
            data: Value::Object(record).to_string(),
        };
        match app.db.create(&append, &app.writer) {
            Ok((key, case)) => {
                let actors = Actors::read(&app.db).unwrap_or_default();
                Ok(app::json(
                    StatusCode::OK,
                    shown_with(app, key, &case, &actors)?,
                ))
            }
            Err(StoreError::Refused(error)) => {
                let status = if error.category() == Category::Malformed {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::UNPROCESSABLE_ENTITY
                };
                Err(Refusal {
                    status,
                    category: error.category().as_str(),
                    message: error.to_string(),
                    case: None,
                })
            }
            Err(error) => Err(Refusal::store(error)),
        }
    })
    .await
}

/// `GET /v1/listeners`: the workers listening for messages now, by serve's
/// clock (ST-16), for the composer's picker and the inbox-zero panel, each
/// with its waits' labels and kinds, which the page matches cases and
/// messages against as each wait does. A store not made yet has none.
pub(crate) async fn listeners(State(app): State<Arc<App>>) -> Result<Response, Refusal> {
    #[derive(Serialize)]
    struct Listener {
        worker: String,
        labels: Vec<String>,
        since: Time,
        heard: Time,
        #[serde(skip_serializing_if = "Option::is_none")]
        doing: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        doing_at: Option<Time>,
        waits: Vec<Wait>,
    }
    /// One of a worker's waits listening now: what the page matches a case
    /// or a message against, as the wait does.
    #[derive(Serialize)]
    struct Wait {
        labels: Vec<String>,
        kinds: Vec<String>,
    }
    #[derive(Serialize)]
    struct Listeners {
        list: Vec<Listener>,
    }
    app::blocking(&app, move |app| {
        let list = match app.db.listeners() {
            Ok(list) => list,
            Err(cases_store_sqlite::Error::Missing { .. }) => Vec::new(),
            Err(error) => return Err(Refusal::store(error)),
        };
        let list = list
            .into_iter()
            .map(|listening| Listener {
                worker: listening.worker,
                labels: listening.labels,
                since: listening.since,
                heard: listening.heard,
                doing: listening.doing,
                doing_at: listening.doing_at,
                waits: listening
                    .waits
                    .into_iter()
                    .map(|wait| Wait {
                        labels: wait.labels,
                        kinds: wait.kinds,
                    })
                    .collect(),
            })
            .collect();
        app::ok(&Listeners { list })
    })
    .await
}

/// Whether the request's body is JSON: a write must be, as a page on another
/// site cannot send it without a CORS preflight (IN-11).
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

/// Reads a write's body, of at most [`MAX_BODY`] bytes, within `timeout`: a
/// request that stalls while it is sent is dropped rather than held (IN-11).
pub(crate) async fn read_body(body: Body, timeout: Duration) -> Result<Bytes, Refusal> {
    match tokio::time::timeout(timeout, axum::body::to_bytes(body, MAX_BODY)).await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(_)) => Err(Refusal::bad("the body is over 1 MiB, or was cut off")),
        Err(_) => Err(Refusal::bad("the body did not arrive in time")),
    }
}

/// The revision a write's body gives, if it gives one, and the event it
/// asks for. The body is a JSON object; `revision`, when given, is a count;
/// an answer is an object; and no field names an actor or a time, which
/// serve stamps.
fn request(write: Write, bytes: &[u8]) -> Result<(Option<u64>, Append), Refusal> {
    let Ok(Value::Object(mut body)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(Refusal::bad("the body is not a JSON object"));
    };
    let revision = match body.remove("revision") {
        None => None,
        Some(revision) => Some(
            revision
                .as_u64()
                .ok_or_else(|| Refusal::bad("revision is not a count"))?,
        ),
    };
    stamped(&body)?;
    let (event, record) = match write {
        Write::Answer => {
            let Some(Value::Object(answer)) = body.remove("answer") else {
                return Err(Refusal::bad("the body has no answer object"));
            };
            stamped(&answer)?;
            ("answer", answer)
        }
        Write::Park => ("park", body),
        Write::Resume => ("resume", body),
        Write::Withdraw => ("withdraw", body),
        Write::Amend => ("amend", body),
    };
    let append = Append {
        author: Author::Human.as_str().into(),
        event: event.into(),
        data: Value::Object(record).to_string(),
    };
    Ok((revision, append))
}

/// Refuses a record that names an actor or a time: serve writes the human's
/// actor and the time itself (HU-06).
fn stamped(record: &Map<String, Value>) -> Result<(), Refusal> {
    match record
        .keys()
        .find(|field| *field == "actor" || field.ends_with("_at"))
    {
        Some(field) => Err(Refusal::bad(format!(
            "{field} is not sent: serve writes who and when"
        ))),
        None => Ok(()),
    }
}

/// The refusal of a write to the case `key` by the core, with the case as it
/// is now, read after the refusal (IN-05): 409 when the write was stale, 400
/// when its record does not read, 404 when the case does not fold, and 422
/// for the rest.
fn refused(app: &App, key: Key, category: Category, message: &str) -> Refusal {
    let status = match category {
        Category::Stale => StatusCode::CONFLICT,
        Category::Malformed => StatusCode::BAD_REQUEST,
        Category::NoOpen => return Refusal::no_case(),
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    let case = (status != StatusCode::BAD_REQUEST)
        .then(|| current(app, key))
        .flatten();
    Refusal {
        status,
        category: category.as_str(),
        message: message.to_owned(),
        case,
    }
}

/// The case `key` as `show --json` gives it now, if it can be read.
fn current(app: &App, key: Key) -> Option<Value> {
    let rows = app.db.rows(&key).ok()??;
    let case = Case::fold(&rows).ok()?;
    serde_json::from_slice(&shown(app, key, &case).ok()?).ok()
}

/// `GET /v1/notifications?after=N`: the notifications after N, the latest
/// number and the boot id (IN-08).
pub(crate) async fn notifications(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
) -> Result<Response, Refusal> {
    // The first `after`, and none when it is empty.
    let after = match form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(name, _)| name == "after")
    {
        Some((_, value)) if !value.is_empty() => value
            .parse()
            .map_err(|_| Refusal::bad("after is not a notification number"))?,
        _ => 0,
    };
    app::blocking(&app, move |app| {
        // Read the store first, so that a tab learns of a case as soon as it
        // asks.
        app.watch.read().map_err(Refusal::store)?;
        let body = app.watch.notifications(after).map_err(Refusal::store)?;
        Ok(app::json(StatusCode::OK, body))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::atomic::AtomicU64;
    use std::task::{Context, Poll};

    use cases_store_sqlite::{Db, Writer};
    use serde_json::json;

    use super::super::http::Timeouts;
    use super::super::log::Log;
    use super::super::watch::Watch;
    use super::*;

    fn refusal(write: Write, body: &str) -> String {
        match request(write, body.as_bytes()) {
            Ok(_) => "ok".into(),
            Err(refusal) => format!("{} {}", refusal.status.as_u16(), refusal.message),
        }
    }

    #[test]
    fn a_write_body_is_a_json_object_with_a_count_for_its_revision() {
        for body in ["", "[]", "3", "{", "{\"revision\": 1} x"] {
            assert_eq!(
                refusal(Write::Resume, body),
                "400 the body is not a JSON object",
                "{body}"
            );
        }
        for revision in ["-1", "1.5", "\"3\"", "null"] {
            let body = format!("{{\"revision\": {revision}}}");
            assert_eq!(
                refusal(Write::Resume, &body),
                "400 revision is not a count",
                "{body}"
            );
        }
        let (revision, append) = request(Write::Resume, br#"{"revision": 5}"#).unwrap();
        assert_eq!(
            (
                revision,
                append.author.as_str(),
                append.event.as_str(),
                append.data.as_str()
            ),
            (Some(5), "human", "resume", "{}")
        );
        let (revision, _) = request(Write::Resume, b"{}").unwrap();
        assert_eq!(revision, None, "no revision is left to the stale check");
    }

    #[test]
    fn an_answer_is_its_own_object_and_names_no_actor_or_time() {
        assert_eq!(
            refusal(Write::Answer, r#"{"revision": 1}"#),
            "400 the body has no answer object"
        );
        assert_eq!(
            refusal(Write::Answer, r#"{"revision": 1, "answer": 2}"#),
            "400 the body has no answer object"
        );
        for body in [
            r#"{"revision": 1, "answer": {"ack": true, "actor": {"id": "x"}}}"#,
            r#"{"revision": 1, "answer": {"ack": true, "answered_at": "2026-09-25T12:00:00Z"}}"#,
            r#"{"revision": 1, "actor": {"id": "x"}, "answer": {"ack": true}}"#,
        ] {
            assert!(refusal(Write::Answer, body).starts_with("400 "), "{body}");
        }
        let (_, append) = request(
            Write::Answer,
            br#"{"revision": 1, "answer": {"choice": 2, "note": "  keep  "}}"#,
        )
        .unwrap();
        assert_eq!(append.event, "answer");
        assert_eq!(append.data, r#"{"choice":2,"note":"  keep  "}"#);
        let (_, park) = request(Write::Park, br#"{"revision": 1, "note": "Monday"}"#).unwrap();
        assert_eq!(
            (park.event.as_str(), park.data.as_str()),
            ("park", r#"{"note":"Monday"}"#)
        );
        assert!(
            refusal(
                Write::Park,
                r#"{"revision": 1, "parked_at": "2026-09-25T12:00:00Z"}"#
            )
            .starts_with("400 ")
        );
        let (_, withdraw) =
            request(Write::Withdraw, br#"{"revision": 3, "reason": "gone"}"#).unwrap();
        assert_eq!(
            (
                withdraw.author.as_str(),
                withdraw.event.as_str(),
                withdraw.data.as_str()
            ),
            ("human", "withdraw", r#"{"reason":"gone"}"#)
        );
        for body in [
            r#"{"revision": 3, "withdrawn_at": "2026-09-25T12:00:00Z"}"#,
            r#"{"revision": 3, "actor": {"id": "x"}}"#,
        ] {
            assert!(refusal(Write::Withdraw, body).starts_with("400 "), "{body}");
        }
    }

    #[test]
    fn a_list_takes_states_by_name_comma_separated_or_repeated() {
        assert_eq!(
            states("").ok(),
            Some(vec![CaseState::Open, CaseState::Parked])
        );
        assert_eq!(
            states("state=closed%2Cwithdrawn&state=answered").ok(),
            Some(vec![
                CaseState::Closed,
                CaseState::Withdrawn,
                CaseState::Answered
            ])
        );
        for query in ["state=done", "state=", "state=open,"] {
            assert_eq!(
                states(query).map_err(|refusal| refusal.status).err(),
                Some(StatusCode::BAD_REQUEST),
                "{query}"
            );
        }
    }

    #[test]
    fn only_a_json_body_is_a_write() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, value.parse().unwrap());
            is_json(&headers)
        };
        assert!(with("application/json"));
        assert!(with("Application/JSON; charset=utf-8"));
        assert!(!with("text/plain"));
        assert!(!with("application/x-www-form-urlencoded"));
        assert!(!is_json(&HeaderMap::new()));
    }

    /// The inbox's server state over the store at `dir/cases.db`, which it
    /// does not make, with reads that give up after `read`.
    fn app_at(dir: &std::path::Path, read: Duration) -> Arc<App> {
        let db = Arc::new(Db::new(dir.join("cases.db")));
        let log = Arc::new(Log::new(false, 0));
        let url = "http://127.0.0.1:7337/".to_owned();
        let (_, stopping) = tokio::sync::watch::channel(false);
        Arc::new(App {
            watch: Watch::new(
                Arc::clone(&db),
                "boot".into(),
                url.clone(),
                Arc::clone(&log),
            )
            .unwrap(),
            db,
            writer: Writer::new(None),
            hosts: Vec::new(),
            proxied: Vec::new(),
            logins: Vec::new(),
            store_tokens: crate::tokens::Tokens::default(),
            leases: crate::dial::Leases::new(crate::dial::LEASE),
            csrf: "token".into(),
            link: url.clone(),
            url,
            log,
            stopping,
            requests: AtomicU64::new(0),
            timeouts: Timeouts {
                read,
                write: Duration::from_secs(5),
                grace: Duration::from_secs(5),
            },
        })
    }

    fn app(dir: &tempfile::TempDir) -> Arc<App> {
        app_at(dir.path(), Duration::from_secs(5))
    }

    /// An app over a file that is not a store, so every read of it fails.
    fn broken(dir: &tempfile::TempDir) -> Arc<App> {
        std::fs::write(
            dir.path().join("cases.db"),
            "not a database, no matter how long ".repeat(50),
        )
        .unwrap();
        app(dir)
    }

    /// Opens a case as an agent with the record `data`, and gives its key.
    fn open(app: &App, data: &str) -> Key {
        let append = Append {
            author: "agent".into(),
            event: "open".into(),
            data: data.into(),
        };
        app.db.create(&append, &Writer::new(None)).unwrap().0
    }

    fn fyi(app: &App) -> Key {
        open(
            app,
            r#"{"kind":"fyi","urgency":"today","title":"Heads up"}"#,
        )
    }

    fn headers(content_type: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        headers
    }

    /// The status and the JSON body of what a handler gave, a refusal as
    /// the page reads it.
    async fn outcome(result: Result<Response, Refusal>) -> (u16, Value) {
        let response = match result {
            Ok(response) => response,
            Err(refusal) => axum::response::IntoResponse::into_response(refusal),
        };
        let status = response.status().as_u16();
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// `POST /v1/cases/{id}/{route}` with `body` as JSON.
    async fn post(app: &Arc<App>, id: &str, route: &str, body: &str) -> (u16, Value) {
        post_as(
            app,
            id,
            route,
            "application/json",
            Body::from(body.to_owned()),
        )
        .await
    }

    async fn post_as(
        app: &Arc<App>,
        id: &str,
        route: &str,
        content_type: &str,
        body: Body,
    ) -> (u16, Value) {
        let path = Ok(Path((id.to_owned(), route.to_owned())));
        outcome(write(State(Arc::clone(app)), path, headers(content_type), body).await).await
    }

    /// `POST /v1/cases` with `body` as JSON.
    async fn sent(app: &Arc<App>, body: &str) -> (u16, Value) {
        let body = Body::from(body.to_owned());
        outcome(send(State(Arc::clone(app)), headers("application/json"), body).await).await
    }

    /// The refusal's category and whether it carries the case.
    fn error(body: &Value) -> (&str, bool) {
        let error = &body["error"];
        (
            error["category"].as_str().unwrap(),
            error.get("case").is_some(),
        )
    }

    /// The case `key`'s revision in the store.
    fn revision(app: &App, key: Key) -> usize {
        app.db.rows(&key).unwrap().unwrap().len()
    }

    /// A body that never arrives.
    struct Stalled;

    impl http_body::Body for Stalled {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn a_write_is_made_as_serves_human_and_gives_the_case() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let key = fyi(&app);
        let (status, case) = post(
            &app,
            &key.to_string(),
            "answer",
            r#"{"revision": 1, "answer": {"ack": true}}"#,
        )
        .await;
        assert_eq!(status, 200, "{case}");
        assert_eq!(
            (&case["id"], &case["state"], &case["revision"]),
            (&json!(key.to_string()), &json!("answered"), &json!(2))
        );
        let event = &case["events"][1];
        assert_eq!(
            (&event["author"], &event["event"]),
            (&json!("human"), &json!("answer"))
        );
        let url = case["url"].as_str().unwrap();
        assert!(url.starts_with("http://127.0.0.1:7337/cases/"), "{url}");
        assert_eq!(revision(&app, key), 2);
    }

    #[tokio::test]
    async fn each_write_route_names_its_event() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let stuck = open(
            &app,
            r#"{"kind":"stuck","urgency":"blocking","title":"Blocked"}"#,
        );
        let id = stuck.to_string();
        for (route, body, state) in [
            ("park", r#"{"revision": 1, "note": "Monday"}"#, "parked"),
            ("resume", r#"{"revision": 2}"#, "open"),
            (
                "withdraw",
                r#"{"revision": 3, "reason": "gone"}"#,
                "withdrawn",
            ),
        ] {
            let (status, case) = post(&app, &id, route, body).await;
            assert_eq!(
                (status, &case["state"]),
                (200, &json!(state)),
                "{route}: {case}"
            );
            let event = case["events"].as_array().unwrap().last().unwrap();
            assert_eq!(
                (&event["author"], &event["event"]),
                (&json!("human"), &json!(route)),
                "{route}: {case}"
            );
        }
        let message = {
            let (status, case) = sent(
                &app,
                r#"{"kind": "message", "urgency": "today", "title": "Rebase", "worker": "w"}"#,
            )
            .await;
            assert_eq!(status, 200, "{case}");
            case["id"].as_str().unwrap().to_owned()
        };
        let (status, case) = post(
            &app,
            &message,
            "amend",
            r#"{"revision": 1, "title": "Rebase now"}"#,
        )
        .await;
        assert_eq!(
            (status, &case["title"]),
            (200, &json!("Rebase now")),
            "{case}"
        );
    }

    #[tokio::test]
    async fn a_write_to_no_case_or_no_route_is_a_404() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        // No store yet: the case is missing, not the store broken.
        let (status, body) = post(&app, "gbHJd", "answer", "{}").await;
        assert_eq!((status, error(&body)), (404, ("request", false)), "{body}");
        assert!(
            !dir.path().join("cases.db").exists(),
            "a refused write made the store"
        );

        let key = fyi(&app);
        for id in ["gbHJd", "nope", ""] {
            let (status, body) = post(&app, id, "answer", "{}").await;
            assert_eq!(status, 404, "{id}: {body}");
            assert_eq!(body["error"]["message"], "no such case", "{id}");
        }
        let (status, body) = post(&app, &key.to_string(), "close", "{}").await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (404, &json!("no such path"))
        );
        let rejected = write(
            State(Arc::clone(&app)),
            Err(PathRejection::MissingPathParams(
                axum::extract::rejection::MissingPathParams::default(),
            )),
            headers("application/json"),
            Body::empty(),
        )
        .await;
        assert_eq!(outcome(rejected).await.0, 404);
        assert_eq!(revision(&app, key), 1);
    }

    #[tokio::test]
    async fn a_write_whose_body_does_not_read_is_a_400_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let key = fyi(&app);
        let id = key.to_string();
        let good = r#"{"revision": 1, "answer": {"ack": true}}"#;
        let (status, body) = post_as(&app, &id, "answer", "text/plain", Body::from(good)).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("a write's body must be application/json"))
        );
        let big = format!(r#"{{"revision": 1, "pad": "{}"}}"#, "x".repeat(MAX_BODY));
        let (status, body) = post(&app, &id, "answer", &big).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("the body is over 1 MiB, or was cut off"))
        );
        for body in ["[]", r#"{"revision": "1"}"#, r#"{"revision": 1}"#] {
            let (status, refusal) = post(&app, &id, "answer", body).await;
            assert_eq!(
                (status, error(&refusal)),
                (400, ("request", false)),
                "{body}"
            );
        }
        assert_eq!(revision(&app, key), 1);
    }

    #[tokio::test]
    async fn a_body_that_stalls_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_at(dir.path(), Duration::from_millis(50));
        let key = fyi(&app);
        let (status, body) = post_as(
            &app,
            &key.to_string(),
            "answer",
            "application/json",
            Body::new(Stalled),
        )
        .await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("the body did not arrive in time"))
        );
    }

    #[tokio::test]
    async fn a_refused_write_has_its_status_and_the_case_as_it_is_now() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let key = fyi(&app);
        let id = key.to_string();
        // No revision, or another one, is stale: a 409 with the case.
        for body in [
            r#"{"answer": {"ack": true}}"#,
            r#"{"revision": 7, "answer": {"ack": true}}"#,
        ] {
            let (status, refusal) = post(&app, &id, "answer", body).await;
            assert_eq!((status, error(&refusal)), (409, ("stale", true)), "{body}");
            assert_eq!(refusal["error"]["case"]["revision"], 1, "{body}");
            assert_eq!(refusal["error"]["case"]["id"], id.as_str(), "{body}");
        }
        let (status, refusal) = post(&app, &id, "resume", "{}").await;
        assert_eq!(
            (status, error(&refusal)),
            (409, ("stale", true)),
            "{refusal}"
        );
        assert_eq!(refusal["error"]["message"], "the write has no revision");
        // A record that does not read: a 400, without the case.
        let (status, refusal) = post(
            &app,
            &id,
            "answer",
            r#"{"revision": 1, "answer": {"ack": "yes"}}"#,
        )
        .await;
        assert_eq!(
            (status, error(&refusal)),
            (400, ("malformed", false)),
            "{refusal}"
        );
        // A write the case's state does not take: a 422 with the case.
        let (status, refusal) = post(&app, &id, "resume", r#"{"revision": 1}"#).await;
        assert_eq!(
            (status, error(&refusal)),
            (422, ("transition", true)),
            "{refusal}"
        );
        assert_eq!(refusal["error"]["case"]["state"], "open");
        assert_eq!(revision(&app, key), 1);
    }

    #[test]
    fn a_refusal_has_the_status_of_its_category() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let key = fyi(&app);
        let refusal = refused(&app, key, Category::NoOpen, "no open");
        assert_eq!(
            (refusal.status, refusal.category, refusal.message.as_str()),
            (StatusCode::NOT_FOUND, "request", "no such case")
        );
        for (category, status) in [
            (Category::Invalid, StatusCode::UNPROCESSABLE_ENTITY),
            (Category::Author, StatusCode::UNPROCESSABLE_ENTITY),
            (Category::Stale, StatusCode::CONFLICT),
        ] {
            let refusal = refused(&app, key, category, "why");
            assert_eq!(
                (refusal.status, refusal.category, refusal.message.as_str()),
                (status, category.as_str(), "why")
            );
            assert_eq!(refusal.case.unwrap()["revision"], 1, "{category}");
        }
        // A case that cannot be read now is left out.
        let gone: Key = "gbHJd".parse().unwrap();
        assert!(refused(&app, gone, Category::Stale, "why").case.is_none());
    }

    #[tokio::test]
    async fn a_store_that_fails_is_a_500() {
        let dir = tempfile::tempdir().unwrap();
        let app = broken(&dir);
        let (status, body) = post(&app, "gbHJd", "answer", "{}").await;
        assert_eq!((status, error(&body)), (500, ("store", false)), "{body}");
        let (status, body) = sent(
            &app,
            r#"{"kind": "message", "urgency": "today", "title": "T", "worker": "w", "re": "gbHJd"}"#,
        )
        .await;
        assert_eq!((status, error(&body)), (500, ("store", false)), "{body}");
        let (status, body) = sent(
            &app,
            r#"{"kind": "message", "urgency": "today", "title": "T", "worker": "w"}"#,
        )
        .await;
        assert_eq!((status, error(&body)), (500, ("store", false)), "{body}");
        let (status, body) = outcome(listeners(State(Arc::clone(&app))).await).await;
        assert_eq!((status, error(&body)), (500, ("store", false)), "{body}");
    }

    fn message(more: &str) -> String {
        format!(
            r#"{{"kind": "message", "urgency": "today", "title": "Rebase onto main", "worker": "bun-pins"{more}}}"#
        )
    }

    #[tokio::test]
    async fn a_message_is_sent_as_serves_human_and_makes_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let (status, case) = sent(&app, &message(r#", "labels": ["release"], "re": """#)).await;
        assert_eq!(status, 200, "{case}");
        assert_eq!(
            (&case["kind"], &case["state"], &case["revision"]),
            (&json!("message"), &json!("sent"), &json!(1))
        );
        let event = &case["events"][0];
        assert_eq!(
            (&event["author"], &event["event"]),
            (&json!("human"), &json!("open"))
        );
        assert!(event["data"]["opened_at"].is_string(), "{event}");
        assert_eq!(event["data"]["labels"], json!(["release"]));
        assert!(
            case.get("re").is_none_or(Value::is_null),
            "an empty re is none: {case}"
        );
        let key: Key = case["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(revision(&app, key), 1);
        assert!(
            case["url"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:7337/cases/")
        );

        // `re` names a case by its whole id, with or without its slug, and
        // is kept bare.
        let about = fyi(&app);
        for re in [about.to_string(), format!("{about}-heads-up")] {
            let (status, case) = sent(&app, &message(&format!(r#", "re": "{re}""#))).await;
            assert_eq!(
                (status, &case["re"]),
                (200, &json!(about.to_string())),
                "{re}: {case}"
            );
        }
    }

    #[tokio::test]
    async fn a_refused_message_has_its_status_and_makes_no_case() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let not_json = send(
            State(Arc::clone(&app)),
            headers("text/plain"),
            Body::from(message("")),
        )
        .await;
        let (status, body) = outcome(not_json).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("a write's body must be application/json"))
        );
        let stalled = send(
            State(app_at(dir.path(), Duration::from_millis(50))),
            headers("application/json"),
            Body::new(Stalled),
        )
        .await;
        let (status, body) = outcome(stalled).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("the body did not arrive in time"))
        );
        // Before the store is made, a `re` names no case.
        let (status, body) = sent(&app, &message(r#", "re": "gbHJd""#)).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (400, &json!("re names no case in this store"))
        );
        assert!(
            !dir.path().join("cases.db").exists(),
            "a refused message made the store"
        );

        let about = fyi(&app);
        let part = &about.to_string()[1..];
        for (body, status, category) in [
            ("{".to_owned(), 400, "request"),
            ("[1]".to_owned(), 400, "request"),
            (
                message(r#", "opened_at": "2026-09-27T12:00:00Z""#),
                400,
                "request",
            ),
            (message(r#", "actor": {"id": "x"}"#), 400, "request"),
            (message(r#", "re": "gbHJd""#), 400, "request"),
            (message(&format!(r#", "re": "{part}""#)), 400, "request"),
            (message(r#", "re": 7"#), 400, "malformed"),
            (
                r#"{"kind": "message", "urgency": "today", "title": 1, "worker": "w"}"#.to_owned(),
                400,
                "malformed",
            ),
            (
                r#"{"kind": "decision", "urgency": "today", "title": "T", "options": ["a", "b"]}"#
                    .to_owned(),
                422,
                "author",
            ),
            (
                r#"{"kind": "message", "urgency": "today", "title": "T", "worker": " "}"#
                    .to_owned(),
                422,
                "invalid",
            ),
        ] {
            let (got, refusal) = sent(&app, &body).await;
            assert_eq!(
                (got, error(&refusal)),
                (status, (category, false)),
                "{body}: {refusal}"
            );
        }
        let next: Key = "gbHJd".parse().unwrap();
        assert!(
            app.db.rows(&next).unwrap().is_none(),
            "a refused message made a case"
        );
    }

    #[tokio::test]
    async fn listeners_are_the_workers_listening_now() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let list = || async { outcome(listeners(State(Arc::clone(&app))).await).await };
        assert_eq!(list().await, (200, json!({"list": []})));
        assert!(
            !dir.path().join("cases.db").exists(),
            "a read made the store"
        );

        fyi(&app);
        assert_eq!(list().await, (200, json!({"list": []})));
        app.db.listen("lead-2", "lead-2", &[], &[], None).unwrap();
        app.db
            .listen(
                "bun-pins",
                "bun-pins",
                &["release".to_owned()],
                &["message".to_owned()],
                Some("rebasing onto main"),
            )
            .unwrap();
        let (status, body) = list().await;
        assert_eq!(status, 200);
        let list = body["list"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{body}");
        let (first, second) = (&list[0], &list[1]);
        assert_eq!(
            (&first["worker"], &first["labels"], &first["doing"]),
            (
                &json!("bun-pins"),
                &json!(["release"]),
                &json!("rebasing onto main")
            )
        );
        assert_eq!(
            first["waits"],
            json!([{"labels": ["release"], "kinds": ["message"]}])
        );
        for field in ["since", "heard", "doing_at"] {
            assert!(first[field].is_string(), "{field}: {first}");
        }
        assert_eq!(
            (&second["worker"], &second["labels"], &second["waits"]),
            (
                &json!("lead-2"),
                &json!([]),
                &json!([{"labels": [], "kinds": []}])
            )
        );
        for field in ["doing", "doing_at"] {
            assert!(second.get(field).is_none(), "{field}: {second}");
        }
    }

    /// `GET /v1/cases/{id}`.
    async fn get_case(app: &Arc<App>, id: &str) -> (u16, Value) {
        outcome(case(State(Arc::clone(app)), Ok(Path(id.to_owned()))).await).await
    }

    /// `GET /v1/cases` with `query`.
    async fn get_list(app: &Arc<App>, query: Option<&str>) -> (u16, Value) {
        let query = RawQuery(query.map(str::to_owned));
        outcome(list(State(Arc::clone(app)), query).await).await
    }

    /// `GET /v1/notifications` with `query`.
    async fn get_notifications(app: &Arc<App>, query: Option<&str>) -> (u16, Value) {
        let query = RawQuery(query.map(str::to_owned));
        outcome(notifications(State(Arc::clone(app)), query).await).await
    }

    #[tokio::test]
    async fn v1_gives_the_build_and_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let (status, body) = outcome(index(State(app)).await).await;
        assert_eq!(
            (status, body),
            (200, json!({"version": *page::BUILD, "csrf": "token"}))
        );
    }

    #[tokio::test]
    async fn a_case_is_the_one_its_id_names() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let first = fyi(&app);
        let second = open(&app, r#"{"kind":"fyi","urgency":"today","title":"Second"}"#);
        for id in [second.to_string(), format!("{second}-second")] {
            let (status, case) = get_case(&app, &id).await;
            assert_eq!(
                (status, &case["id"], &case["title"]),
                (200, &json!(second.to_string()), &json!("Second")),
                "{id}"
            );
            assert_eq!(
                case["url"],
                format!("http://127.0.0.1:7337/cases/{second}-second")
            );
        }
        let (status, case) = get_case(&app, &first.to_string()).await;
        assert_eq!((status, &case["title"]), (200, &json!("Heads up")));
        let none = Key::new(99).unwrap().to_string();
        let (status, body) = get_case(&app, &none).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (404, &json!("no such case"))
        );
        let rejected = super::case(
            State(Arc::clone(&app)),
            Err(PathRejection::MissingPathParams(
                axum::extract::rejection::MissingPathParams::default(),
            )),
        )
        .await;
        let (status, body) = outcome(rejected).await;
        assert_eq!(
            (status, &body["error"]["message"]),
            (404, &json!("no such case"))
        );
    }

    #[tokio::test]
    async fn the_list_is_the_cases_in_the_states_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        assert_eq!(get_list(&app, None).await, (200, json!({"list": []})));
        let key = fyi(&app);
        let (status, body) = get_list(&app, None).await;
        assert_eq!(status, 200);
        let list = body["list"].as_array().unwrap();
        assert_eq!(
            list.iter().map(|case| &case["id"]).collect::<Vec<_>>(),
            [&json!(key.to_string())],
            "{body}"
        );
        assert_eq!(
            get_list(&app, Some("state=closed")).await,
            (200, json!({"list": []}))
        );
        let (status, body) = get_list(&app, Some("state=done")).await;
        assert_eq!((status, error(&body)), (400, ("request", false)), "{body}");
    }

    #[tokio::test]
    async fn the_feed_gives_what_landed_after_the_number_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir);
        let (status, first) = get_notifications(&app, None).await;
        assert_eq!(status, 200, "{first}");
        assert_eq!((&first["latest"], &first["items"]), (&json!(0), &json!([])));
        assert_eq!(first["boot"], "boot");

        let key = open(
            &app,
            r#"{"kind":"decision","urgency":"blocking","title":"Pin bun?","options":["Pin","Float"]}"#,
        );
        let count = |page: &Value| page["items"].as_array().map(Vec::len);
        // No after, an empty one, and 0 are all from the start; the first
        // after counts.
        for query in [
            None,
            Some("after="),
            Some("after=0"),
            Some("after=0&after=1"),
        ] {
            let (status, page) = get_notifications(&app, query).await;
            assert_eq!((status, count(&page)), (200, Some(1)), "{query:?}: {page}");
            assert_eq!(page["latest"], 1, "{query:?}");
            assert_eq!(page["items"][0]["case"]["id"], key.to_string(), "{query:?}");
        }
        for query in ["after=1", "other=0&after=1"] {
            let (status, page) = get_notifications(&app, Some(query)).await;
            assert_eq!((status, count(&page)), (200, Some(0)), "{query}: {page}");
        }
        for bad in ["x", "-1", "1.5"] {
            let (status, body) = get_notifications(&app, Some(&format!("after={bad}"))).await;
            assert_eq!(
                (status, &body["error"]["message"]),
                (400, &json!("after is not a notification number")),
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn a_read_of_a_store_that_fails_is_a_500() {
        let dir = tempfile::tempdir().unwrap();
        let app = broken(&dir);
        for (read, (status, body)) in [
            ("list", get_list(&app, None).await),
            ("case", get_case(&app, "gbHJd").await),
            ("notifications", get_notifications(&app, None).await),
        ] {
            assert_eq!(
                (status, error(&body)),
                (500, ("store", false)),
                "{read}: {body}"
            );
        }
    }
}
