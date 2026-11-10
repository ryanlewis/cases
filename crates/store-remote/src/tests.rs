//! The client against the fake server: what goes on the wire, the error
//! table end to end, retries, limits and the held poll.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Append, Category, Key, Poller, Refolded, Store, StoreError, Version, Writer};

use crate::client::Limits;
use crate::fake::{Fake, Fault, TOKEN};
use crate::{Client, Error, RemoteStore};

/// Limits short enough for a test.
pub(crate) fn quick() -> Limits {
    Limits {
        connect: Duration::from_secs(2),
        request: Duration::from_secs(2),
        grace: Duration::from_secs(2),
        answer: 4096,
        retries: [Duration::from_millis(10), Duration::from_millis(20)],
        rate_wait: Duration::from_secs(2),
        rate_default: Duration::from_millis(10),
        signin: Duration::from_secs(2),
        exchange_retries: [
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ],
    }
}

fn client(fake: &Fake) -> Client {
    Client::with_limits(fake.url().parse().unwrap(), TOKEN.parse().unwrap(), quick())
}

/// A fake with one store, and the store.
fn setup() -> (Fake, RemoteStore) {
    let fake = Fake::start();
    let id = fake.add_store("me");
    let store = client(&fake).store(&id).unwrap();
    (fake, store)
}

fn open() -> Append {
    Append {
        author: "agent".into(),
        event: "open".into(),
        data: r#"{"kind": "fyi", "urgency": "today", "title": "Build moved"}"#.into(),
    }
}

fn note() -> Append {
    Append {
        author: "agent".into(),
        event: "note".into(),
        data: r#"{"body": "More."}"#.into(),
    }
}

fn unnamed() -> Writer {
    Writer::new(None)
}

fn store_error<T: std::fmt::Debug>(result: Result<T, StoreError<Error>>) -> Error {
    match result {
        Err(StoreError::Store(error)) => error,
        other => panic!("{other:?}"),
    }
}

#[test]
#[cfg(feature = "hosted")]
fn whoami_reads_the_account_and_its_stores() {
    let fake = Fake::start();
    let me = fake.add_store("me");
    let team = fake.add_store("team-x");
    let who = client(&fake).whoami().unwrap();
    assert_eq!(
        (who.account.id.as_str(), who.account.handle.as_str()),
        ("ac_fake", "ryan")
    );
    assert_eq!(who.token.name, "tests");
    assert_eq!(who.token.stores, None);
    assert_eq!(who.token.expires.as_deref(), Some("2026-12-31T10:00:00Z"));
    let stores: Vec<_> = who
        .stores
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str(), s.default))
        .collect();
    assert_eq!(
        stores,
        [(me.as_str(), "me", true), (team.as_str(), "team-x", false)]
    );
    assert_eq!(fake.lines(), ["GET /v1/cli/whoami HTTP/1.1"]);
}

#[test]
#[cfg(feature = "hosted")]
fn a_wrong_token_is_refused_with_the_server_s_message() {
    let fake = Fake::start();
    let other = Client::with_limits(
        fake.url().parse().unwrap(),
        format!("csu_{}", "b".repeat(36)).parse().unwrap(),
        quick(),
    );
    assert_eq!(
        other.whoami().unwrap_err(),
        Error::Auth {
            message: "the token is not one".into()
        }
    );
}

#[test]
fn a_store_id_is_checked_before_anything_is_sent() {
    let fake = Fake::start();
    let error = client(&fake).store("st_../../v1/cli").unwrap_err();
    assert_eq!(
        error.to_string(),
        "\"st_../../v1/cli\" is not a store's id: it is st_ and 16 letters and digits"
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn the_token_only_ever_goes_in_the_header() {
    let (fake, store) = setup();
    let (key, _) = store.create(&open(), &Writer::new(Some("lead"))).unwrap();
    store.append(&key, &note(), &unnamed(), Some(1)).unwrap();
    store.rows(&key).unwrap();
    store.rows_since(Some(&Version::new("1"))).unwrap();
    store
        .changes(&Version::new("0"), Duration::from_secs(1))
        .unwrap();
    store.archive(&key, None).unwrap();
    fake.fault(Fault::Revoked);
    let error = store.list().unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TOKEN));
    let requests = fake.requests();
    assert_eq!(requests.len(), 7);
    for request in &requests {
        let lines: Vec<&str> = request.lines().filter(|l| l.contains(TOKEN)).collect();
        assert_eq!(
            lines,
            [format!("authorization: Bearer {TOKEN}")],
            "{request}"
        );
        let agent = concat!("user-agent: cases/", env!("CARGO_PKG_VERSION"));
        assert!(request.lines().any(|l| l == agent), "{request}");
    }
    assert!(!format!("{store:?}").contains(TOKEN));
}

#[test]
fn writes_send_json_and_their_routes() {
    let (fake, store) = setup();
    let id = store.id().to_owned();
    let (key, case) = store.create(&open(), &Writer::new(Some("lead"))).unwrap();
    assert_eq!(key, Key::new(1).unwrap());
    assert_eq!(case.title(), "Build moved");
    let case = store.append(&key, &note(), &unnamed(), Some(1)).unwrap();
    assert_eq!(case.revision(), 2);
    store.delete(&key, Some(2)).unwrap();
    let lines = fake.lines();
    assert_eq!(
        lines,
        [
            format!("POST /v1/stores/{id}/store/cases HTTP/1.1"),
            format!("POST /v1/stores/{id}/store/cases/1/events HTTP/1.1"),
            format!("POST /v1/stores/{id}/store/cases/1/delete HTTP/1.1"),
        ]
    );
    let requests = fake.requests();
    assert!(requests[0].contains("content-type: application/json"));
    assert!(requests[0].ends_with(
        r#"{"writer":"lead","append":{"author":"agent","event":"open","data":"{\"kind\": \"fyi\", \"urgency\": \"today\", \"title\": \"Build moved\"}"}}"#
    ));
    assert!(requests[1].ends_with(r#""revision":1}"#), "{}", requests[1]);
    assert!(requests[1].contains(r#"{"writer":null,"#));
    assert!(requests[2].ends_with(r#"{"revision":2}"#));
    // The writer's name made its own actor.
    let actors = store.actors().unwrap();
    let names: Vec<_> = actors.iter().map(|a| a.name.as_deref()).collect();
    assert_eq!(names, [Some("lead"), None]);
}

#[test]
fn refusals_come_back_by_category() {
    let (_fake, store) = setup();
    let (key, _) = store.create(&open(), &unnamed()).unwrap();
    let category = |result: Result<cases_core::Case, StoreError<Error>>| match result {
        Err(StoreError::Refused(error)) => error.category(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        category(store.append(&key, &note(), &unnamed(), Some(9))),
        Category::Stale
    );
    let answer = Append {
        author: "agent".into(),
        event: "answer".into(),
        data: r#"{"ack": true}"#.into(),
    };
    assert_eq!(
        category(store.append(&key, &answer, &unnamed(), None)),
        Category::Author
    );
    let bad = Append {
        author: "agent".into(),
        event: "note".into(),
        data: "not json".into(),
    };
    assert_eq!(
        category(store.append(&key, &bad, &unnamed(), None)),
        Category::Malformed
    );
    let missing = Key::new(9).unwrap();
    assert!(matches!(
        store.append(&missing, &note(), &unnamed(), None),
        Err(StoreError::NoCase)
    ));
    assert!(matches!(
        store.archive(&missing, None),
        Err(StoreError::NoCase)
    ));
    assert!(matches!(
        store.archive(&key, Some(7)),
        Err(StoreError::Refused(e)) if e.category() == Category::Stale
    ));
    assert_eq!(store.rows(&missing).unwrap(), None);
}

#[test]
fn a_removed_store_and_one_not_there() {
    let fake = Fake::start();
    let id = fake.add_store("me");
    let c = client(&fake);
    let store = c.store(&id).unwrap();
    fake.remove(&id);
    assert_eq!(store.list().unwrap_err(), Error::Gone);
    assert_eq!(store_error(store.create(&open(), &unnamed())), Error::Gone);
    let none = c.store("st_zzzzzzzzzzzzzzzz").unwrap();
    assert_eq!(none.version().unwrap_err(), Error::NoStore);
}

#[test]
fn a_redirect_is_refused_and_not_followed() {
    let (fake, store) = setup();
    fake.fault(Fault::Redirect);
    assert_eq!(store.list().unwrap_err(), Error::Redirect { status: 302 });
    fake.fault(Fault::Redirect);
    assert_eq!(
        store_error(store.create(&open(), &unnamed())),
        Error::Redirect { status: 302 }
    );
    // Nothing went to where it pointed.
    assert_eq!(fake.lines().len(), 2);
    assert!(fake.lines().iter().all(|l| !l.contains("elsewhere")));
}

#[test]
fn a_read_is_tried_twice_more_after_a_transient_failure() {
    let (fake, store) = setup();
    fake.fault(Fault::Status(503));
    fake.fault(Fault::Hangup);
    assert_eq!(store.version().unwrap().as_str(), "0");
    assert_eq!(fake.lines().len(), 3);

    for _ in 0..3 {
        fake.fault(Fault::Status(502));
    }
    assert!(matches!(
        store.version(),
        Err(Error::Server { status: 502, .. })
    ));
    assert_eq!(fake.lines().len(), 6);

    // A failure that is not transient is not tried again.
    fake.fault(Fault::Status(500));
    assert!(matches!(
        store.version(),
        Err(Error::Server { status: 500, .. })
    ));
    assert_eq!(fake.lines().len(), 7);
}

#[test]
fn a_write_is_not_tried_again_after_no_answer() {
    let (fake, store) = setup();
    fake.fault(Fault::Status(503));
    assert!(matches!(
        store_error(store.create(&open(), &unnamed())),
        Error::Server { status: 503, .. }
    ));
    fake.fault(Fault::Hangup);
    let error = store_error(store.create(&open(), &unnamed()));
    assert!(error.is_transient(), "{error:?}");
    assert!(matches!(&error, Error::Transport { server, .. } if *server == fake.url()));
    assert_eq!(fake.lines().len(), 2);
    assert_eq!(store.list().unwrap(), vec![]);
}

#[test]
fn a_429_is_waited_out_once() {
    let (fake, store) = setup();
    fake.fault(Fault::Rate(Some(1)));
    let start = Instant::now();
    store.create(&open(), &unnamed()).unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(fake.lines().len(), 2);

    // Twice is a failure, with when to try again.
    fake.fault(Fault::Rate(None));
    fake.fault(Fault::Rate(Some(1)));
    assert_eq!(
        store_error(store.create(&open(), &unnamed())),
        Error::Rate {
            retry_after: Some(Duration::from_secs(1))
        }
    );
    assert_eq!(fake.lines().len(), 4);

    // Longer than the client waits: a failure at once.
    fake.fault(Fault::Rate(Some(3)));
    let start = Instant::now();
    assert_eq!(
        store.list().unwrap_err(),
        Error::Rate {
            retry_after: Some(Duration::from_secs(3))
        }
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fake.lines().len(), 5);

    // Exactly as long as the client waits: waited out.
    fake.fault(Fault::Rate(Some(2)));
    let start = Instant::now();
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!(fake.lines().len(), 7);
}

#[test]
fn a_429_on_a_read_and_then_transient_failures() {
    let (fake, store) = setup();
    fake.fault(Fault::Rate(None));
    fake.fault(Fault::Status(503));
    fake.fault(Fault::Status(504));
    assert_eq!(store.version().unwrap().as_str(), "0");
    assert_eq!(fake.lines().len(), 4);
}

/// A client with a deadline neither retries a read nor waits out a 429
/// past it, and gives a request no more than the time left.
#[test]
fn a_deadline_cuts_retries_the_429_wait_and_requests() {
    let (fake, store) = setup();
    let limits = Limits {
        retries: [Duration::from_millis(500), Duration::from_secs(2)],
        rate_wait: Duration::from_secs(10),
        request: Duration::from_secs(10),
        ..quick()
    };
    let within = |left: Duration| {
        Client::with_limits(
            fake.url().parse().unwrap(),
            TOKEN.parse().unwrap(),
            limits.clone(),
        )
        .with_deadline(Some(Instant::now() + left))
        .store(store.id())
        .unwrap()
    };

    // The first retry fits in the time left, and the second does not.
    for _ in 0..3 {
        fake.fault(Fault::Status(503));
    }
    let start = Instant::now();
    let error = within(Duration::from_secs(1)).list().unwrap_err();
    assert!(
        matches!(error, Error::Server { status: 503, .. }),
        "{error:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fake.lines().len(), 2);
    // The fake's third 503 is still queued: take it.
    assert!(store.version().is_ok());

    // A 429 the client would wait out is not waited out past it.
    fake.fault(Fault::Rate(Some(5)));
    let start = Instant::now();
    assert_eq!(
        within(Duration::from_secs(2)).list().unwrap_err(),
        Error::Rate {
            retry_after: Some(Duration::from_secs(5))
        }
    );
    assert!(start.elapsed() < Duration::from_secs(1));

    // A slow answer is cut at the deadline, not at the request's limit.
    fake.fault(Fault::Slow(Duration::from_secs(5)));
    let start = Instant::now();
    assert!(matches!(
        within(Duration::from_millis(1500)).list(),
        Err(Error::Transport { .. })
    ));
    let took = start.elapsed();
    assert!(took < Duration::from_secs(3), "{took:?}");

    // With no deadline, the same client waits a 429 out.
    fake.fault(Fault::Rate(Some(1)));
    let start = Instant::now();
    let free = within(Duration::ZERO);
    let free = free
        .client()
        .clone()
        .with_deadline(None)
        .store(store.id())
        .unwrap();
    assert!(free.list().is_ok());
    assert!(start.elapsed() >= Duration::from_secs(1));
}

#[test]
fn a_slow_answer_times_out() {
    let (fake, store) = setup();
    fake.fault(Fault::Slow(Duration::from_secs(5)));
    fake.fault(Fault::Slow(Duration::from_secs(5)));
    fake.fault(Fault::Slow(Duration::from_secs(5)));
    let start = Instant::now();
    assert!(matches!(store.list(), Err(Error::Transport { .. })));
    assert!(start.elapsed() < Duration::from_secs(9));
}

#[test]
fn an_answer_over_the_limit_fails() {
    let (fake, store) = setup();
    fake.fault(Fault::Oversized(5000));
    assert_eq!(store.list().unwrap_err(), Error::AnswerTooLarge);
    // Not transient, so not tried again.
    assert_eq!(fake.lines().len(), 1);
    fake.fault(Fault::Oversized(4000));
    assert!(matches!(
        store.list(),
        Err(Error::Protocol { status: 200, .. })
    ));
}

#[test]
fn an_answer_that_is_not_the_api_s_fails() {
    let (fake, store) = setup();
    fake.fault(Fault::Garbage);
    assert!(matches!(
        store.version(),
        Err(Error::Protocol { status: 200, .. })
    ));
    fake.fault(Fault::Body(
        r#"{"version": "1", "cases": [{"key": 1, "rows": []}]}"#.into(),
    ));
    assert!(matches!(
        store.create(&open(), &unnamed()),
        Err(StoreError::Store(Error::Protocol { .. }))
    ));
}

#[test]
fn a_request_over_1_mib_is_never_sent() {
    let (fake, store) = setup();
    let mut big = open();
    big.data = format!(
        r#"{{"kind": "fyi", "urgency": "today", "title": "{}"}}"#,
        "x".repeat(1024 * 1024)
    );
    let error = store_error(store.create(&big, &unnamed()));
    assert!(matches!(error, Error::TooLarge { size } if size > 1024 * 1024));
    assert!(fake.requests().is_empty());
}

#[test]
fn a_body_of_exactly_1_mib_is_sent() {
    let (fake, store) = setup();
    let empty = serde_json::to_vec(&serde_json::json!({
        "writer": null,
        "append": {"author": "agent", "event": "open", "data": ""}
    }))
    .unwrap()
    .len();
    let mut exact = open();
    exact.data = "x".repeat(1024 * 1024 - empty);
    let refused = store.create(&exact, &unnamed());
    assert!(
        matches!(refused, Err(StoreError::Refused(_))),
        "{refused:?}"
    );
    assert!(fake.requests()[0].contains("content-length: 1048576"));
}

#[test]
fn the_held_poll_answers_when_the_version_moves() {
    let (fake, store) = setup();
    let v0 = store.version().unwrap();
    // At once, when the version already differs, with the hold capped.
    let other = Version::new("other");
    assert_eq!(store.changes(&other, Duration::from_secs(600)).unwrap(), v0);
    let id = store.id().to_owned();
    assert_eq!(
        fake.lines().last().unwrap(),
        &format!("GET /v1/stores/{id}/store/changes?after=other&timeout=25 HTTP/1.1")
    );
    // Held until a write.
    let writer = store.clone();
    let write = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        writer.create(&open(), &unnamed()).unwrap();
    });
    let start = Instant::now();
    let v1 = store.changes(&v0, Duration::from_secs(5)).unwrap();
    assert_ne!(v1, v0);
    assert!(start.elapsed() >= Duration::from_millis(250));
    assert!(start.elapsed() < Duration::from_secs(4));
    write.join().unwrap();
    // After the hold with the version unchanged.
    let start = Instant::now();
    assert_eq!(store.changes(&v1, Duration::from_millis(1999)).unwrap(), v1);
    assert!(start.elapsed() >= Duration::from_millis(900));
    assert!(fake.lines().last().unwrap().contains("timeout=1 "));
}

#[test]
fn a_hold_under_a_second_asks_at_once() {
    let (fake, store) = setup();
    let v = store
        .changes(&Version::new("0"), Duration::from_millis(999))
        .unwrap();
    assert_eq!(v.as_str(), "0");
    let id = store.id();
    assert_eq!(
        fake.lines(),
        [format!("GET /v1/stores/{id}/store/changes HTTP/1.1")]
    );
}

#[test]
fn a_held_poll_may_take_its_hold_and_the_grace() {
    let (fake, store) = setup();
    // 1 s of hold and 2 s of grace: a 2.5 s answer is in time, where a
    // plain request of 2 s would have timed out. Each try is slow, so a
    // retry after a timeout cannot pass in its place.
    for _ in 0..3 {
        fake.fault(Fault::Slow(Duration::from_millis(2500)));
    }
    let v = store
        .changes(&Version::new("x"), Duration::from_secs(1))
        .unwrap();
    assert_eq!(v.as_str(), "0");
    assert_eq!(fake.lines().len(), 1);
}

#[test]
fn a_version_goes_in_the_query_encoded() {
    let (fake, store) = setup();
    store.rows_since(Some(&Version::new("a b&c"))).unwrap();
    assert!(fake.lines()[0].contains("/store/cases?since=a%20b%26c "));
    store
        .changes(&Version::new("0"), Duration::from_secs(1))
        .unwrap();
    assert!(fake.lines()[1].contains("changes?after=0&timeout=1 "));
}

#[test]
fn the_poller_follows_a_remote_store() {
    let (fake, store) = setup();
    let (a, _) = store.create(&open(), &unnamed()).unwrap();
    let (b, _) = store.create(&open(), &unnamed()).unwrap();
    let mut poller = Poller::new(&store);
    assert_eq!(poller.poll().unwrap().len(), 2);
    assert_eq!(poller.refolded(), &Refolded::All);
    store.append(&a, &note(), &unnamed(), None).unwrap();
    store.archive(&b, None).unwrap();
    let cases = poller.poll().unwrap();
    assert_eq!(cases.keys().copied().collect::<Vec<_>>(), [a]);
    assert_eq!(cases[&a].as_ref().unwrap().revision(), 2);
    // The far side says what changed, and the poller what it folded again.
    let only = |folded: &[Key], removed: &[Key]| Refolded::Some {
        folded: folded.to_vec(),
        removed: removed.to_vec(),
    };
    assert_eq!(poller.refolded(), &only(&[a], &[b]));
    assert_eq!(poller.poll().unwrap().len(), 1);
    assert_eq!(poller.refolded(), &only(&[], &[]));
    let lines = fake.lines();
    // Every case at first, and then only what changed.
    assert_eq!(lines.len(), 7);
    assert!(lines[2].ends_with("/store/cases HTTP/1.1"));
    assert!(lines[5].contains("/store/cases?since="));
    assert!(lines[6].contains("/store/cases?since="));
}

#[test]
fn listen_records_nothing_and_sends_nothing() {
    let (fake, store) = setup();
    store
        .listen("w1", "lead", &["x".into()], &[], Some("rebasing"))
        .unwrap();
    assert!(fake.requests().is_empty());
}

#[test]
fn a_doing_line_is_checked_before_it_is_sent() {
    let (fake, store) = setup();
    for text in ["a\nb", &"x".repeat(81), "a\u{202e}b"] {
        let Err(Error::Invalid(error)) = store.set_doing("lead", text) else {
            panic!("{text:?} was taken");
        };
        assert_eq!(error.category(), Category::Invalid);
    }
    assert!(matches!(
        store.set_doing(" ", "rebasing"),
        Err(Error::Invalid(_))
    ));
    assert!(fake.requests().is_empty());
    assert_eq!(
        store.set_doing("lead", "rebasing").unwrap_err(),
        Error::NotListening
    );
}

#[test]
fn waits_and_listeners_read_the_server_s_record() {
    let (fake, store) = setup();
    store.create(&open(), &unnamed()).unwrap();
    let db = fake.db(store.id());
    db.listen("w1", "a/b c", &["x".into()], &["fyi".into()], None)
        .unwrap();
    store.set_doing("a/b c", " rebasing ").unwrap();
    assert!(
        fake.lines()
            .last()
            .unwrap()
            .contains("/store/doing/a%2Fb%20c ")
    );
    let waits = store.waits().unwrap();
    assert_eq!(waits, db.waits().unwrap());
    assert_eq!(waits[0].kinds, ["fyi"]);
    let listeners = store.listeners().unwrap();
    assert_eq!(listeners, db.listeners().unwrap());
    assert_eq!(listeners[0].doing.as_deref(), Some("rebasing"));
    assert!(listeners[0].doing_at.is_some());
    // Cleared, the doing line is left out of the answer.
    store.set_doing("a/b c", "").unwrap();
    let listeners = store.listeners().unwrap();
    assert_eq!(
        (listeners[0].doing.as_ref(), listeners[0].doing_at),
        (None, None)
    );
    assert_eq!(listeners, db.listeners().unwrap());
}

#[test]
fn a_wait_the_server_lists_twice_is_read_once() {
    let (fake, store) = setup();
    let wait = |heard: &str| {
        format!(
            r#"{{"id": "w1", "worker": "lead", "labels": [], "kinds": [], "since": "2026-09-15T09:00:00Z", "heard": "{heard}"}}"#
        )
    };
    let (early, late) = (wait("2026-09-15T09:00:01Z"), wait("2026-09-15T09:00:05Z"));
    fake.fault(Fault::Body(format!(r#"{{"list": [{late}, {early}]}}"#)));
    let waits = store.waits().unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].heard.to_string(), "2026-09-15T09:00:05Z");
    fake.fault(Fault::Body(format!(
        r#"{{"list": [{{"worker": "lead", "labels": [], "since": "2026-09-15T09:00:00Z", "heard": "2026-09-15T09:00:05Z", "waits": [{early}, {late}]}}]}}"#
    )));
    let listeners = store.listeners().unwrap();
    assert_eq!(listeners[0].waits.len(), 1);
    assert_eq!(
        listeners[0].waits[0].heard.to_string(),
        "2026-09-15T09:00:05Z"
    );
    assert_eq!(listeners[0].doing, None);
}

#[test]
fn the_inbox_link_is_the_far_side_s_or_none() {
    let (fake, store) = setup();
    // The fake has no such route, as a far side from before it.
    assert_eq!(store.inbox_link().unwrap(), None);
    assert!(fake.lines().last().unwrap().contains("/store/inbox "));
    fake.fault(Fault::Body(
        r#"{"link":"https://devbox.example.ts.net/"}"#.into(),
    ));
    assert_eq!(
        store.inbox_link().unwrap().as_deref(),
        Some("https://devbox.example.ts.net/")
    );
    for none in [r#"{"link":null}"#, r#"{"link":""}"#] {
        fake.fault(Fault::Body(none.into()));
        assert_eq!(store.inbox_link().unwrap(), None);
    }
    fake.fault(Fault::Status(500));
    assert!(matches!(
        store.inbox_link(),
        Err(Error::Server { status: 500, .. })
    ));
}
