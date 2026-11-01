//! One store on a server, through the core's `Store` trait.

use std::fmt::Write as _;
use std::time::Duration;

use cases_core::{
    ActorRow, Append, Case, Changes, Key, Listening, Row, Store, StoreError, Time, Version, Wait,
    Writer, doing_line,
};
use serde::{Deserialize, Serialize};

use crate::client::{Call, Client, Failure, Retry, protocol_of};
use crate::error::Error;
use crate::hold::Hold;

/// The longest hold of a held poll.
pub const HOLD_MAX: Duration = Duration::from_secs(25);

/// The id of the one store a forwarded store's far side serves, `cases
/// dial-stdio`: the local store there.
pub const DIAL_STORE: &str = "st_dialstdio0000000";

/// One store on a server, by its id: the core's [`Store`] over the store
/// routes of the agent API, `{server}/v1/stores/{id}/store/…`.
///
/// The server runs the core's writer under the store's lock, so a stale
/// write comes back as a refusal of category `stale`, and nothing is
/// stored; the client never checks a revision itself. A refusal of the
/// core's comes back as [`StoreError::Refused`], with the category the
/// server named, so a remote store refuses what a local one does.
///
/// The held poll is the listening record of a hosted store (ST-16), so
/// [`Store::listen`] records nothing there. A forwarded store, reached over a
/// pipe ([`Client::dial`]), records it as a local store does, with
/// `POST store/listen`. Beside the trait, [`RemoteStore::changes`] is the
/// held poll itself.
///
/// A read that fails for a transient reason is sent again, up to twice; a
/// write is sent again only after a 429, once, since a write that got no
/// answer may have been stored.
#[derive(Clone, Debug)]
pub struct RemoteStore {
    client: Client,
    id: String,
}

/// A row on the wire.
#[derive(Serialize, Deserialize)]
struct RowWire {
    seq: u64,
    author: String,
    event: String,
    data: String,
}

/// A case on the wire: its key and its rows.
#[derive(Deserialize)]
struct CaseWire {
    key: u64,
    rows: Vec<RowWire>,
}

/// `GET store/cases`, with or without `since`.
#[derive(Deserialize)]
struct CasesWire {
    version: String,
    cases: Vec<CaseWire>,
    removed: Option<Vec<u64>>,
}

#[derive(Deserialize)]
struct VersionWire {
    version: String,
}

/// The answer to `GET store/inbox`.
#[derive(Deserialize)]
struct InboxWire {
    link: Option<String>,
}

#[derive(Deserialize)]
struct ListWire<T> {
    list: Vec<T>,
}

#[derive(Deserialize)]
struct ActorWire {
    id: String,
    kind: String,
    name: Option<String>,
    created: String,
}

#[derive(Deserialize)]
struct WaitWire {
    id: String,
    worker: String,
    labels: Vec<String>,
    kinds: Vec<String>,
    since: String,
    heard: String,
}

#[derive(Deserialize)]
struct ListenerWire {
    worker: String,
    labels: Vec<String>,
    since: String,
    heard: String,
    #[serde(default)]
    doing: Option<String>,
    #[serde(default)]
    doing_at: Option<String>,
    waits: Vec<WaitWire>,
}

#[derive(Serialize)]
struct AppendWire<'a> {
    author: &'a str,
    event: &'a str,
    data: &'a str,
}

impl<'a> AppendWire<'a> {
    fn of(append: &'a Append) -> AppendWire<'a> {
        AppendWire {
            author: &append.author,
            event: &append.event,
            data: &append.data,
        }
    }
}

#[derive(Serialize)]
struct CreateBody<'a> {
    writer: Option<&'a str>,
    append: AppendWire<'a>,
}

#[derive(Serialize)]
struct AppendBody<'a> {
    writer: Option<&'a str>,
    append: AppendWire<'a>,
    revision: Option<u64>,
}

#[derive(Serialize)]
struct RevisionBody {
    revision: Option<u64>,
}

#[derive(Serialize)]
struct DoingBody<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct ListenBody<'a> {
    wait: &'a str,
    worker: &'a str,
    labels: &'a [String],
    kinds: &'a [String],
    doing: Option<&'a str>,
}

/// An error for an answer that read as JSON but not as the API's.
fn bad(message: impl Into<String>) -> Error {
    Error::Protocol {
        status: 200,
        message: message.into(),
    }
}

fn key_of(n: u64) -> Result<Key, Error> {
    Key::new(n).ok_or_else(|| bad(format!("{n} is not a case's key")))
}

fn time_of(text: &str) -> Result<Time, Error> {
    text.parse()
        .map_err(|_| bad(format!("{text:?} is not a time")))
}

fn rows_of(rows: Vec<RowWire>) -> Vec<Row> {
    rows.into_iter()
        .map(|row| Row {
            seq: row.seq,
            author: row.author,
            event: row.event,
            data: row.data,
        })
        .collect()
}

fn case_of(case: CaseWire) -> Result<(Key, Vec<Row>), Error> {
    Ok((key_of(case.key)?, rows_of(case.rows)))
}

fn wait_of(wait: WaitWire) -> Result<Wait, Error> {
    Ok(Wait::new(
        wait.id,
        wait.worker,
        wait.labels,
        wait.kinds,
        time_of(&wait.since)?,
        time_of(&wait.heard)?,
    ))
}

/// The waits, with one for each id: a client that reconnects before its
/// old socket closes can hold one wait in two polls for a moment, and the
/// server then lists it twice. The one heard last is kept, where the first
/// of its id stood.
fn waits_of(waits: Vec<WaitWire>) -> Result<Vec<Wait>, Error> {
    let mut kept: Vec<Wait> = Vec::new();
    for wait in waits {
        let wait = wait_of(wait)?;
        match kept
            .iter_mut()
            .find(|k| k.id == wait.id && k.worker == wait.worker)
        {
            Some(k) if wait.heard > k.heard => *k = wait,
            Some(_) => {}
            None => kept.push(wait),
        }
    }
    Ok(kept)
}

/// The JSON of a request's body.
fn json(body: &impl Serialize) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(body).map_err(|error| bad(error.to_string()))
}

/// `text` as it goes in a path or a query: each byte that is not
/// unreserved (RFC 3986) is percent-encoded.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// A write's failure, as the trait gives it.
fn write_error(failure: Failure) -> StoreError<Error> {
    match failure {
        Failure::Refused(error) => StoreError::Refused(error),
        Failure::NoCase => StoreError::NoCase,
        Failure::Error(error) => StoreError::Store(error),
    }
}

impl RemoteStore {
    pub(crate) fn new(client: Client, id: String) -> RemoteStore {
        RemoteStore { client, id }
    }

    /// The store's id, `st_…`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The client the store is reached by.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The path of `tail` under the store, such as `/v1/stores/{id}/store/cases`.
    fn path(&self, tail: &str) -> String {
        format!("/v1/stores/{}/store/{tail}", self.id)
    }

    /// A read of `tail`, as `T`.
    fn get<T: serde::de::DeserializeOwned>(
        &self,
        tail: &str,
        timeout: Option<Duration>,
    ) -> Result<T, Failure> {
        let path = self.path(tail);
        self.client.json(&Call {
            method: "GET",
            path: &path,
            body: None,
            timeout,
            retry: Retry::Read,
        })
    }

    /// A read that can give neither a refusal nor a missing case.
    fn read<T: serde::de::DeserializeOwned>(&self, tail: &str) -> Result<T, Error> {
        self.get(tail, None).map_err(|f| protocol_of(&f))
    }

    /// A write of `body` to `tail`, as `T`.
    fn write<T: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        tail: &str,
        body: Vec<u8>,
    ) -> Result<T, Failure> {
        let path = self.path(tail);
        self.client.json(&Call {
            method,
            path: &path,
            body: Some(body),
            timeout: None,
            retry: Retry::Write,
        })
    }

    /// A write that gives a case, folded.
    fn write_case(&self, tail: &str, body: Vec<u8>) -> Result<(Key, Case), StoreError<Error>> {
        let case: CaseWire = self.write("POST", tail, body).map_err(write_error)?;
        let (key, rows) = case_of(case).map_err(StoreError::Store)?;
        let case = Case::fold(&rows)
            .map_err(|error| StoreError::Store(bad(format!("the case does not fold: {error}"))))?;
        Ok((key, case))
    }

    /// Takes a pickup wait's locks (AG-05) on the machine of a forwarded
    /// store, as `cases wait --pickup` takes them on a local store: one on
    /// each of `locks`, a `label`, `worker` or `id` and its text. They are
    /// held, on a connection of their own, until the [`Hold`] lets them go
    /// or is dropped, or the connection ends. They are asked for on a
    /// lease, which the hold renews: a far side that gives one lets them go
    /// if the renewals stop, and [`Hold::check`] says when they may have
    /// gone. The hold says it lets them go itself, so such a far side keeps
    /// them past a connection that ends without that, until the lease runs
    /// out. A far side too old for leases holds them while the connection
    /// lasts.
    ///
    /// # Errors
    ///
    /// [`Error::Held`] when another wait holds one of them; the client's
    /// errors when the request fails, and [`Error::Request`] for a store
    /// that is not reached over a pipe.
    pub fn hold(&self, locks: &[(&str, &str)]) -> Result<Hold, Error> {
        let locks: Vec<_> = locks
            .iter()
            .map(|(on, text)| serde_json::json!({"on": on, "text": text}))
            .collect();
        let body = json(&serde_json::json!({ "locks": locks, "lease": true, "release": true }))?;
        self.client.hold(&self.path("lock"), &body)
    }

    /// The link of the inbox a serve on a forwarded store's machine gives
    /// (`GET store/inbox`), such as `https://devbox.example.ts.net/`, or
    /// `None` when no serve is running there, the one running gives none,
    /// or the far side is too old to say (404).
    ///
    /// # Errors
    ///
    /// Gives the client's errors when the request fails.
    pub fn inbox_link(&self) -> Result<Option<String>, Error> {
        match self.get::<InboxWire>("inbox", None) {
            Ok(answer) => Ok(answer.link.filter(|link| !link.is_empty())),
            Err(Failure::Error(Error::NoStore | Error::Protocol { status: 404, .. })) => Ok(None),
            Err(failure) => Err(protocol_of(&failure)),
        }
    }

    /// The held poll: the store's version as soon as it is not `after`, or
    /// after `hold` with it still `after`. The hold is in whole seconds, at
    /// most [`HOLD_MAX`]; one under a second asks for the version at once,
    /// so a deadline is not overshot.
    ///
    /// The poll carries no listening parameters: it does not make a wait
    /// listen.
    ///
    /// # Errors
    ///
    /// Gives the client's errors when the request fails.
    pub fn changes(&self, after: &Version, hold: Duration) -> Result<Version, Error> {
        let secs = hold.min(HOLD_MAX).as_secs();
        if secs == 0 {
            return self.version();
        }
        let tail = format!("changes?after={}&timeout={secs}", encode(after.as_str()));
        let timeout = Duration::from_secs(secs) + self.client.grace();
        let answer: VersionWire = self
            .get(&tail, Some(timeout))
            .map_err(|f| protocol_of(&f))?;
        Ok(Version::new(answer.version))
    }
}

impl Store for RemoteStore {
    type Key = Key;
    type Writer = Writer;
    type Error = Error;

    fn create(&self, open: &Append, writer: &Writer) -> Result<(Key, Case), StoreError<Error>> {
        let body = json(&CreateBody {
            writer: writer.name(),
            append: AppendWire::of(open),
        })
        .map_err(StoreError::Store)?;
        self.write_case("cases", body)
    }

    fn append(
        &self,
        key: &Key,
        append: &Append,
        writer: &Writer,
        at_revision: Option<u64>,
    ) -> Result<Case, StoreError<Error>> {
        let body = json(&AppendBody {
            writer: writer.name(),
            append: AppendWire::of(append),
            revision: at_revision,
        })
        .map_err(StoreError::Store)?;
        let (_, case) = self.write_case(&format!("cases/{}/events", key.get()), body)?;
        Ok(case)
    }

    fn rows(&self, key: &Key) -> Result<Option<Vec<Row>>, Error> {
        match self.get::<CaseWire>(&format!("cases/{}", key.get()), None) {
            Ok(case) => Ok(Some(case_of(case)?.1)),
            Err(Failure::NoCase) => Ok(None),
            Err(failure) => Err(protocol_of(&failure)),
        }
    }

    fn list(&self) -> Result<Vec<(Key, Vec<Row>)>, Error> {
        let answer: CasesWire = self.read("cases")?;
        answer.cases.into_iter().map(case_of).collect()
    }

    fn archive(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Error>> {
        let body = json(&RevisionBody {
            revision: at_revision,
        })
        .map_err(StoreError::Store)?;
        let _: serde_json::Map<String, serde_json::Value> = self
            .write("POST", &format!("cases/{}/archive", key.get()), body)
            .map_err(write_error)?;
        Ok(())
    }

    fn delete(&self, key: &Key, at_revision: Option<u64>) -> Result<(), StoreError<Error>> {
        let body = json(&RevisionBody {
            revision: at_revision,
        })
        .map_err(StoreError::Store)?;
        let _: serde_json::Map<String, serde_json::Value> = self
            .write("POST", &format!("cases/{}/delete", key.get()), body)
            .map_err(write_error)?;
        Ok(())
    }

    fn version(&self) -> Result<Version, Error> {
        let answer: VersionWire = self.read("changes")?;
        Ok(Version::new(answer.version))
    }

    fn rows_since(&self, since: Option<&Version>) -> Result<Changes<Key>, Error> {
        let tail = match since {
            Some(since) => format!("cases?since={}", encode(since.as_str())),
            None => "cases".to_owned(),
        };
        let answer: CasesWire = self.read(&tail)?;
        changes_of(answer, since)
    }

    fn actors(&self) -> Result<Vec<ActorRow>, Error> {
        let answer: ListWire<ActorWire> = self.read("actors")?;
        Ok(answer
            .list
            .into_iter()
            .map(|a| ActorRow::new(a.id, a.kind, a.name, a.created))
            .collect())
    }

    /// Records nothing on a hosted store, where the held poll is the
    /// listening record (ST-16). A forwarded store records it, as a local
    /// store does: a blank worker, or a doing line that is not one, is
    /// refused before anything is sent.
    fn listen(
        &self,
        wait: &str,
        worker: &str,
        labels: &[String],
        kinds: &[String],
        doing: Option<&str>,
    ) -> Result<(), Error> {
        if !self.client.is_forwarded() {
            return Ok(());
        }
        if worker.trim().is_empty() {
            return Err(Error::Invalid(cases_core::Error::new(
                cases_core::Category::Invalid,
                "a worker has a name",
            )));
        }
        if let Some(doing) = doing {
            doing_line(doing).map_err(Error::Invalid)?;
        }
        let body = json(&ListenBody {
            wait,
            worker,
            labels,
            kinds,
            doing,
        })?;
        let _: serde_json::Map<String, serde_json::Value> = self
            .write("POST", "listen", body)
            .map_err(|f| protocol_of(&f))?;
        Ok(())
    }

    fn set_doing(&self, worker: &str, text: &str) -> Result<(), Error> {
        doing_line(text).map_err(Error::Invalid)?;
        if worker.trim().is_empty() {
            return Err(Error::Invalid(cases_core::Error::new(
                cases_core::Category::Invalid,
                "a worker has a name",
            )));
        }
        let body = json(&DoingBody { text })?;
        let _: serde_json::Map<String, serde_json::Value> = self
            .write("PUT", &format!("doing/{}", encode(worker)), body)
            .map_err(|f| protocol_of(&f))?;
        Ok(())
    }

    fn waits(&self) -> Result<Vec<Wait>, Error> {
        let answer: ListWire<WaitWire> = self.read("waits")?;
        waits_of(answer.list)
    }

    fn listeners(&self) -> Result<Vec<Listening>, Error> {
        let answer: ListWire<ListenerWire> = self.read("listeners")?;
        answer
            .list
            .into_iter()
            .map(|l| {
                Ok(Listening::new(
                    l.worker,
                    l.labels,
                    time_of(&l.since)?,
                    time_of(&l.heard)?,
                    l.doing,
                    l.doing_at.as_deref().map(time_of).transpose()?,
                    waits_of(l.waits)?,
                ))
            })
            .collect()
    }
}

/// The answer to `GET store/cases?since=V` as the core's `Changes`.
fn changes_of(answer: CasesWire, since: Option<&Version>) -> Result<Changes<Key>, Error> {
    let version = Version::new(answer.version);
    let cases = answer
        .cases
        .into_iter()
        .map(case_of)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match answer.removed {
        Some(removed) if removed.is_empty() && cases.is_empty() && since == Some(&version) => {
            Changes::Unchanged { version }
        }
        Some(removed) if since.is_some() => Changes::Since {
            version,
            cases,
            removed: removed.into_iter().map(key_of).collect::<Result<_, _>>()?,
        },
        _ => Changes::All { version, cases },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> CasesWire {
        serde_json::from_str(text).unwrap()
    }

    const ROW: &str = r#"{"seq": 1, "author": "agent", "event": "open", "data": "{}"}"#;

    fn key(n: u64) -> Key {
        Key::new(n).unwrap()
    }

    #[test]
    fn rows_since_reads_three_answers() {
        let v7 = Version::new("7");
        let unchanged = answer(r#"{"version": "7", "cases": [], "removed": []}"#);
        assert_eq!(
            changes_of(unchanged, Some(&v7)).unwrap(),
            Changes::Unchanged {
                version: v7.clone()
            }
        );
        // Empty, at another version: what changed is nothing, and the
        // version moved.
        let moved = answer(r#"{"version": "8", "cases": [], "removed": []}"#);
        assert_eq!(
            changes_of(moved, Some(&v7)).unwrap(),
            Changes::Since {
                version: Version::new("8"),
                cases: vec![],
                removed: vec![]
            }
        );
        let since = answer(&format!(
            r#"{{"version": "9", "cases": [{{"key": 2, "rows": [{ROW}]}}], "removed": [1]}}"#
        ));
        let Changes::Since {
            version,
            cases,
            removed,
        } = changes_of(since, Some(&v7)).unwrap()
        else {
            panic!("not what changed");
        };
        assert_eq!((version.as_str(), removed), ("9", vec![key(1)]));
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].0, key(2));
        assert_eq!(cases[0].1[0].data, "{}");
        // At the same version, with something in it: still what changed.
        let odd = answer(r#"{"version": "7", "cases": [], "removed": [3]}"#);
        assert!(matches!(
            changes_of(odd, Some(&v7)).unwrap(),
            Changes::Since { .. }
        ));
        let odd = answer(&format!(
            r#"{{"version": "7", "cases": [{{"key": 2, "rows": [{ROW}]}}], "removed": []}}"#
        ));
        assert!(matches!(
            changes_of(odd, Some(&v7)).unwrap(),
            Changes::Since { .. }
        ));
        let all = answer(&format!(
            r#"{{"version": "7", "cases": [{{"key": 1, "rows": [{ROW}]}}]}}"#
        ));
        let Changes::All { version, cases } = changes_of(all, Some(&v7)).unwrap() else {
            panic!("not every case");
        };
        assert_eq!((version, cases.len()), (v7.clone(), 1));
        // With no since, every case, whatever the server sent.
        let none = answer(r#"{"version": "7", "cases": [], "removed": []}"#);
        assert_eq!(
            changes_of(none, None).unwrap(),
            Changes::All {
                version: v7,
                cases: vec![]
            }
        );
    }

    #[test]
    fn a_key_that_is_none_is_not_the_api_s() {
        let zero = answer(r#"{"version": "1", "cases": [{"key": 0, "rows": []}]}"#);
        assert!(matches!(
            changes_of(zero, None),
            Err(Error::Protocol { .. })
        ));
        let removed = answer(r#"{"version": "1", "cases": [], "removed": [0]}"#);
        assert!(changes_of(removed, Some(&Version::new("0"))).is_err());
    }

    #[test]
    fn text_is_percent_encoded_for_a_path() {
        assert_eq!(encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode("a b/c?d&e=f%"), "a%20b%2Fc%3Fd%26e%3Df%25");
        assert_eq!(encode("é"), "%C3%A9");
    }

    fn wait(id: &str, worker: &str, heard: &str) -> WaitWire {
        WaitWire {
            id: id.into(),
            worker: worker.into(),
            labels: vec![],
            kinds: vec![],
            since: "2026-09-15T09:00:00Z".into(),
            heard: heard.into(),
        }
    }

    #[test]
    fn a_wait_listed_twice_is_kept_once_as_heard_last() {
        let waits = waits_of(vec![
            wait("a", "w", "2026-09-15T09:00:05Z"),
            wait("b", "w", "2026-09-15T09:00:01Z"),
            wait("a", "w", "2026-09-15T09:00:09Z"),
            wait("a", "w", "2026-09-15T09:00:07Z"),
            wait("a", "x", "2026-09-15T09:00:02Z"),
        ])
        .unwrap();
        let got: Vec<(&str, &str, String)> = waits
            .iter()
            .map(|w| (w.id.as_str(), w.worker.as_str(), w.heard.to_string()))
            .collect();
        assert_eq!(
            got,
            [
                ("a", "w", "2026-09-15T09:00:09Z".to_owned()),
                ("b", "w", "2026-09-15T09:00:01Z".to_owned()),
                ("a", "x", "2026-09-15T09:00:02Z".to_owned()),
            ]
        );
        assert!(waits_of(vec![wait("a", "w", "not a time")]).is_err());
        // Heard at the same time: the first is kept.
        let mut second = wait("a", "w", "2026-09-15T09:00:05Z");
        second.labels = vec!["later".into()];
        let waits = waits_of(vec![wait("a", "w", "2026-09-15T09:00:05Z"), second]).unwrap();
        assert_eq!(waits.len(), 1);
        assert!(waits[0].labels.is_empty());
    }

    #[test]
    fn the_dial_store_is_a_store_s_id() {
        assert!(crate::store_id(DIAL_STORE));
        assert_eq!(DIAL_STORE, "st_dialstdio0000000");
    }

    #[test]
    fn the_longest_hold_is_25_seconds() {
        assert_eq!(HOLD_MAX, Duration::from_secs(25));
    }
}
