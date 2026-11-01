//! The inbox's API (spec/api.md), over HTTP to a running `cases serve`:
//! reads (IN-01, IN-02, IN-07), the human's writes and every status code
//! pinned for them (IN-03, IN-04, IN-05), the event stream (IN-06), the
//! notification feed (IN-08) and the checks that keep the API to this
//! machine (IN-11).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::io::Write as _;
use std::time::{Duration, Instant};

use cases_core::Store as _;
use common::serve::{Request, Response, Serve};
use common::{Env, json};

/// A write of `body` to the case `id`'s `route`, as the inbox's page sends
/// it.
fn write(serve: &Serve, id: &str, route: &str, body: &str) -> Response {
    serve.post(&format!("/v1/cases/{id}/{route}"), body)
}

/// Checks that `response` is the refusal `status` of `category`, and gives
/// its error.
fn refused(response: &Response, status: u16, category: &str) -> serde_json::Value {
    assert_eq!(response.status, status, "{response:?}");
    assert_eq!(response.category(), category, "{response:?}");
    assert_eq!(response.header("content-type"), Some("application/json"));
    response.json()["error"].clone()
}

#[test]
fn v1_gives_the_build_and_a_token_new_each_run() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let index = serve.get("/v1");
    assert_eq!(index.status, 200);
    assert_eq!(index.header("cache-control"), Some("no-store"));
    let index = index.json();
    let build = index["version"].as_str().unwrap();
    assert!(
        build.starts_with(concat!(env!("CARGO_PKG_VERSION"), "+")),
        "{build}"
    );
    let token = index["csrf"].as_str().unwrap().to_owned();
    assert!(
        token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "{token}"
    );
    assert_eq!(serve.stop().code, 0);
    assert_ne!(env.serve(&[]).csrf(), token, "the token outlived its serve");
}

#[test]
fn every_response_carries_the_security_headers_and_no_cors() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    let responses = [
        serve.get("/v1"),
        serve.get(&format!("/v1/cases/{id}")),
        serve.get("/nothing-here"),
        serve.send(&Request::new("GET", "/v1").header("Origin", "http://evil.example")),
        serve.send(&Request::write(&format!("/v1/cases/{id}/answer"), "{}")),
        serve.get("/favicon.svg"),
    ];
    for response in &responses {
        let csp = response
            .header("content-security-policy")
            .unwrap_or_default();
        assert!(csp.starts_with("default-src 'self'"), "{response:?}");
        assert!(csp.contains("frame-ancestors 'none'"), "{response:?}");
        assert!(!csp.contains("unsafe"), "{response:?}");
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(response.header("referrer-policy"), Some("same-origin"));
        assert!(
            !response
                .headers
                .iter()
                .any(|(name, _)| name.starts_with("access-control-")),
            "{response:?}"
        );
    }
}

/// A request must name this server as its Host, reads included, so that a
/// name rebound to 127.0.0.1 cannot read the API (IN-11).
#[test]
fn the_host_must_be_this_server() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let port = serve.addr.rsplit(':').next().unwrap();
    for host in ["evil.example", "127.0.0.1:9", "192.168.1.10"] {
        let host = host.replace("evil.example", &format!("evil.example:{port}"));
        let response = serve.send(&Request::new("GET", "/v1/cases").header("Host", &host));
        refused(&response, 403, "request");
    }
    let response = serve.send(&Request::new("GET", "/v1/cases").header("Host", ""));
    assert_eq!(response.status, 403, "{response:?}");
    for host in [
        format!("localhost:{port}"),
        format!("LOCALHOST:{port}"),
        serve.addr.clone(),
    ] {
        let response = serve.send(&Request::new("GET", "/v1/cases").header("Host", &host));
        assert_eq!(response.status, 200, "{host}: {response:?}");
    }
}

/// A host in the config file's allow-hosts, a proxy's such as tailscale
/// serve's, is answered, with writes from its https origin, and with
/// tailscale-logins set only for a request from one of them (IN-11).
#[test]
fn a_proxys_host_is_answered_when_the_config_file_allows_it() {
    let env = Env::new();
    let id = env.open(&["--kind", "stuck", "--urgency", "today", "--title", "Stuck"]);
    env.config(
        "allow-hosts = [\"Devbox.example.ts.net:7337\"]\ntailscale-logins = [\"ryan@github\"]\n",
    );
    let serve = env.serve(&[]);
    let host = "devbox.example.ts.net:7337";
    let read = |login: &str| {
        let mut request = Request::new("GET", "/v1/cases").header("Host", host);
        if !login.is_empty() {
            request = request.header("Tailscale-User-Login", login);
        }
        serve.send(&request)
    };
    assert_eq!(read("ryan@github").status, 200);
    refused(&read(""), 403, "request");
    refused(&read("eve@github"), 403, "request");
    let other = serve.send(&Request::new("GET", "/v1/cases").header("Host", "evil.example:7337"));
    refused(&other, 403, "request");
    // The loopback address needs no login.
    assert_eq!(serve.get("/v1/cases").status, 200);

    let park = format!("/v1/cases/{id}/park");
    let write = |origin: &str| {
        serve.send(
            &Request::write(&park, r#"{"revision": 1}"#)
                .header("Host", host)
                .header("Tailscale-User-Login", "ryan@github")
                .header("Origin", origin)
                .header("X-Cases-CSRF", &serve.csrf()),
        )
    };
    refused(&write(&format!("http://{host}")), 403, "request");
    let response = write(&format!("https://{host}"));
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.json()["state"], "parked");
}

/// With allow-hosts set, the links cases gives name the first of them over
/// https, so that one opened on another device of the tailnet reaches the
/// inbox: the case's url in show, the API and a notification, and the
/// instance file's link. serve still listens, and status says it is, at the
/// loopback address (IN-11).
#[test]
fn links_name_the_first_allowed_host() {
    let env = Env::new();
    env.config("allow-hosts = [\"Devbox.example.ts.net:7337\", \"devbox.local:7337\"]\n");
    let serve = env.serve(&[]);
    let id = env.open_fyi("Arrived");
    let page = format!("https://devbox.example.ts.net:7337/cases/{id}-arrived");
    let shown = json(&env.ok(&["show", "--json", &id]));
    assert_eq!(shown["url"], page.as_str());
    assert!(env.ok(&["show", &id]).contains(&page));
    assert_eq!(env.ok(&["inbox", "--print", &id]).trim_end(), page);
    assert_eq!(
        env.ok(&["inbox", "--print"]).trim_end(),
        "https://devbox.example.ts.net:7337/"
    );
    assert_eq!(serve.get(&format!("/v1/cases/{id}")).json(), shown);
    let info = json(&std::fs::read_to_string(env.instance_file()).unwrap());
    assert_eq!(info["link"], "https://devbox.example.ts.net:7337/");
    assert_eq!(info["url"], serve.url.as_str());
    assert_eq!(
        json(&env.ok(&["status", "--json"]))["url"],
        serve.url.as_str()
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let item = loop {
        let notes = serve.get("/v1/notifications").json();
        if let Some(item) = notes["items"].get(0) {
            break item.clone();
        }
        assert!(Instant::now() < deadline, "no notification: {notes}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(item["case"]["url"], page.as_str());
    assert_eq!(item["url"], format!("/cases/{id}-arrived"));
}

/// A request `tailscale funnel` passed on from the internet is refused,
/// even through an allowed host from a named login (IN-11).
#[test]
fn a_request_through_tailscale_funnel_is_refused() {
    let env = Env::new();
    env.config(
        "allow-hosts = [\"devbox.example.ts.net:7337\"]\ntailscale-logins = [\"ryan@github\"]\n",
    );
    let serve = env.serve(&[]);
    for host in ["devbox.example.ts.net:7337", serve.addr.as_str()] {
        let request = Request::new("GET", "/v1/cases")
            .header("Host", host)
            .header("Tailscale-User-Login", "ryan@github")
            .header("Tailscale-Funnel-Request", "?1");
        refused(&serve.send(&request), 403, "request");
    }
    assert_eq!(serve.get("/v1/cases").status, 200);
}

/// The list is `list --json`'s, in inbox order: most urgent first, then in
/// the order the cases were opened (IN-01, AG-14).
#[test]
fn the_list_is_list_json_in_inbox_order() {
    let env = Env::new();
    let urgency = |title: &str, urgency: &str| {
        env.open(&["--kind", "fyi", "--urgency", urgency, "--title", title])
    };
    let today = urgency("Today", "today");
    let first = urgency("First blocking", "blocking");
    let whenever = urgency("Whenever", "whenever");
    let second = urgency("Second blocking", "blocking");
    let answered = urgency("Answered", "today");
    env.write(&answered, "human", "answer", r#"{"ack": true}"#);
    let parked = env.open(&["--kind", "stuck", "--urgency", "today", "--title", "Parked"]);
    env.write(&parked, "human", "park", "{}");
    let serve = env.serve(&[]);

    let ids = |query: &str| -> Vec<String> {
        let response = serve.get(&format!("/v1/cases{query}"));
        assert_eq!(response.status, 200, "{query}: {response:?}");
        assert_eq!(response.header("cache-control"), Some("no-store"));
        response.json()["list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids(""), [first, second, today, parked.clone(), whenever]);
    assert_eq!(ids("?state=answered"), std::slice::from_ref(&answered));
    assert_eq!(
        ids("?state=answered%2Cparked"),
        [answered.clone(), parked.clone()]
    );
    assert_eq!(ids("?state=parked&state=answered"), [answered, parked]);
    // The same list, item for item, as list --json gives.
    assert_eq!(
        serve.get("/v1/cases?state=open,parked,answered").json(),
        json(&env.ok(&["list", "--json", "--state", "open,parked,answered"]))
    );
    for bad in ["?state=done", "?state=", "?state=open,"] {
        refused(&serve.get(&format!("/v1/cases{bad}")), 400, "request");
    }
}

#[test]
fn a_store_not_made_yet_lists_no_cases() {
    let env = Env::new();
    let serve = env.serve(&[]);
    assert_eq!(
        serve.get("/v1/cases").json(),
        serde_json::json!({"list": []})
    );
    refused(&serve.get("/v1/cases/UkLWZ"), 404, "request");
    let response = write(&serve, "UkLWZ", "resume", r#"{"revision": 1}"#);
    refused(&response, 404, "request");
    assert!(
        !env.store.exists(),
        "a read or a refused write made the store"
    );
}

/// A case is `show --json`'s, url and all, named by its whole id with or
/// without a title slug, never by part of one (AG-13, ST-10, HU-07).
#[test]
fn a_case_is_what_show_json_gives() {
    let env = Env::new();
    let id = env.open_decision();
    env.write_as(
        &id,
        "agent",
        "note",
        r#"{"body": "More."}"#,
        Some("bun-pins"),
    );
    let serve = env.serve(&[]);
    let response = serve.get(&format!("/v1/cases/{id}"));
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let case = response.json();
    assert_eq!(case, json(&env.ok(&["show", "--json", &id])));
    assert_eq!(case["url"], format!("{}cases/{id}-pin-bun", serve.url));
    assert_eq!(case["revision"], 2);
    assert_eq!(serve.get(&format!("/v1/cases/{id}-pin-bun")).json(), case);
    // An id that is not text is no case's id either.
    for other in [&id[..3], "zzzzz", "UkLWZ-x/../y", "%FF", "UkLWZ-caf%C3"] {
        refused(&serve.get(&format!("/v1/cases/{other}")), 404, "request");
    }
}

/// An answer writes one `human answer` event with serve's human, stamped
/// by serve, and gives the case as it is after it (IN-03, HU-06, CL-02).
#[test]
fn an_answer_writes_one_event_as_serves_human() {
    let env = Env::new();
    let id = env.open_decision();
    let serve = env.serve(&["--as", "Ryan"]);
    let response = write(
        &serve,
        &id,
        "answer",
        r#"{"revision": 1, "answer": {"choice": 2, "note": "  watch the lockfile  ", "amend_seq": 0}}"#,
    );
    assert_eq!(response.status, 200, "{response:?}");
    let case = response.json();
    assert_eq!(case["state"], "answered");
    assert_eq!(case["revision"], 2);
    assert_eq!(case["url"], format!("{}cases/{id}-pin-bun", serve.url));
    // The note is kept as sent: the page trims what is typed, the API does
    // not.
    assert_eq!(case["answer"]["note"], "  watch the lockfile  ");
    let event = &case["events"][1];
    assert_eq!(
        (&event["author"], &event["event"]),
        (&"human".into(), &"answer".into())
    );
    assert_eq!(event["actor"]["name"], "Ryan");
    assert_eq!(event["actor"]["kind"], "human");
    assert!(event["data"]["answered_at"].is_string(), "{event}");
    assert_eq!(env.revision(&id), 2);
    assert_eq!(case, json(&env.ok(&["show", "--json", &id])));
}

/// Park and resume are writes of their own (HU-03, HU-04).
#[test]
fn park_and_resume_are_writes_of_their_own() {
    let env = Env::new();
    let stuck = env.open(&["--kind", "stuck", "--urgency", "today", "--title", "Stuck"]);
    let decision = env.open_decision();
    let serve = env.serve(&["--as", "Ryan"]);
    let parked = write(
        &serve,
        &stuck,
        "park",
        r#"{"revision": 1, "note": "Monday"}"#,
    );
    assert_eq!(parked.status, 200, "{parked:?}");
    assert_eq!(parked.json()["state"], "parked");
    assert_eq!(parked.json()["park"]["note"], "Monday");
    let resumed = write(&serve, &stuck, "resume", r#"{"revision": 2}"#);
    assert_eq!(resumed.status, 200, "{resumed:?}");
    assert_eq!(resumed.json()["state"], "open");
    assert!(resumed.json().get("park").is_none());
    assert_eq!(resumed.json()["events"][2]["event"], "resume");
    // Each is serve's human's (HU-06).
    for seq in [1, 2] {
        let actor = &resumed.json()["events"][seq]["actor"];
        assert_eq!(
            (&actor["kind"], &actor["name"]),
            (&"human".into(), &"Ryan".into())
        );
    }

    // Both are guarded by the revision, as an answer is (IN-05).
    refused(
        &write(&serve, &stuck, "park", r#"{"revision": 1}"#),
        409,
        "stale",
    );
    refused(
        &write(&serve, &stuck, "resume", r#"{"revision": 1}"#),
        409,
        "stale",
    );
    let error = refused(
        &write(&serve, &decision, "park", r#"{"revision": 1}"#),
        422,
        "transition",
    );
    assert_eq!(error["case"]["id"], decision.as_str());
    refused(
        &write(&serve, &stuck, "resume", r#"{"revision": 3}"#),
        422,
        "transition",
    );
    assert_eq!(env.revision(&decision), 1);
    assert_eq!(env.revision(&stuck), 3);
}

/// The human withdraws a case from the inbox: one an agent has, answered or
/// picked up, which keeps its answer and pickup, or an open one, with an
/// optional reason, as serve's human. It is guarded by the revision, and a
/// case that is closed or withdrawn already refuses it (ST-14, IN-03, IN-04,
/// IN-05).
#[test]
fn withdraw_is_a_write_of_its_own() {
    let env = Env::new();
    let open = env.open_decision();
    let answered = env.open_decision();
    env.write(&answered, "human", "answer", r#"{"choice": 1}"#);
    let picked = env.open_decision();
    env.write(&picked, "human", "answer", r#"{"choice": 2}"#);
    env.write(&picked, "agent", "pickup", r#"{"by": "bun-pins"}"#);
    let closed = env.open_decision();
    env.write(&closed, "human", "answer", r#"{"choice": 1}"#);
    env.write(&closed, "agent", "pickup", "{}");
    env.write(&closed, "agent", "close", r#"{"outcome": "Pinned."}"#);
    let serve = env.serve(&["--as", "Ryan"]);

    let response = write(
        &serve,
        &picked,
        "withdraw",
        r#"{"revision": 3, "reason": "its session died"}"#,
    );
    assert_eq!(response.status, 200, "{response:?}");
    let case = response.json();
    assert_eq!(case["state"], "withdrawn");
    assert_eq!(case["revision"], 4);
    assert_eq!(case["answer"]["choice"], 2);
    assert_eq!(case["pickup"]["by"], "bun-pins");
    let event = &case["events"][3];
    assert_eq!(
        (&event["author"], &event["event"]),
        (&"human".into(), &"withdraw".into())
    );
    assert_eq!(event["data"]["reason"], "its session died");
    assert!(event["data"]["withdrawn_at"].is_string(), "{event}");
    assert_eq!(
        (&event["actor"]["kind"], &event["actor"]["name"]),
        (&"human".into(), &"Ryan".into())
    );
    assert_eq!(case, json(&env.ok(&["show", "--json", &picked])));
    for (id, revision) in [(&answered, 2), (&open, 1)] {
        let body = format!(r#"{{"revision": {revision}}}"#);
        let response = write(&serve, id, "withdraw", &body);
        assert_eq!(response.status, 200, "{response:?}");
        assert_eq!(response.json()["state"], "withdrawn");
        assert!(
            response.json()["events"][revision]["data"]
                .get("reason")
                .is_none()
        );
    }
    assert_eq!(env.case(&answered).answer().unwrap().choice, 1);

    let error = refused(
        &write(&serve, &closed, "withdraw", r#"{"revision": 3}"#),
        409,
        "stale",
    );
    assert_eq!(error["case"]["revision"], 4);
    for (id, revision) in [(&closed, 4), (&answered, 3)] {
        let body = format!(r#"{{"revision": {revision}}}"#);
        let error = refused(&write(&serve, id, "withdraw", &body), 422, "transition");
        assert_eq!(error["case"]["id"], id.as_str());
    }
    refused(
        &write(
            &serve,
            &closed,
            "withdraw",
            r#"{"revision": 4, "withdrawn_at": "2026-09-25T12:00:00Z"}"#,
        ),
        400,
        "request",
    );
    assert_eq!(env.revision(&closed), 4);
    assert_eq!(env.revision(&answered), 3);
}

/// Each refusal has the status spec/api.md pins, and writes nothing (IN-04,
/// IN-05, IN-11).
#[test]
fn every_refusal_has_its_status_and_writes_nothing() {
    let env = Env::new();
    let id = env.open_decision();
    let withdrawn = env.open_fyi("Gone");
    env.write(&withdrawn, "agent", "withdraw", "{}");
    let serve = env.serve(&[]);
    let token = serve.csrf();
    let answer = format!("/v1/cases/{id}/answer");
    let good = r#"{"revision": 1, "answer": {"choice": 1}}"#;
    let with_token = |request: Request| request.header("X-Cases-CSRF", &token);
    let send = |request: Request| serve.send(&request);

    // 400: the body is not the JSON the route takes.
    let big = format!(
        r#"{{"revision": 1, "answer": {{"choice": 1, "note": "{}"}}}}"#,
        "x".repeat(1 << 20)
    );
    for (name, request) in [
        ("not JSON", Request::write(&answer, "{")),
        ("not an object", Request::write(&answer, "[1]")),
        (
            "not application/json",
            Request::write(&answer, good).header("Content-Type", "text/plain"),
        ),
        (
            "a form",
            Request::write(&answer, "choice=1")
                .header("Content-Type", "application/x-www-form-urlencoded"),
        ),
        ("over 1 MiB", Request::write(&answer, &big)),
        ("no answer", Request::write(&answer, r#"{"revision": 1}"#)),
        (
            "an answer that is not an object",
            Request::write(&answer, r#"{"revision": 1, "answer": 1}"#),
        ),
        (
            "an actor",
            Request::write(
                &answer,
                r#"{"revision": 1, "answer": {"choice": 1, "actor": {"id": "AAAAAAAAAAAAAAAAAAAAAA"}}}"#,
            ),
        ),
        (
            "a time",
            Request::write(
                &answer,
                r#"{"revision": 1, "answer": {"choice": 1, "answered_at": "2026-09-25T12:00:00Z"}}"#,
            ),
        ),
        (
            "an actor beside the answer",
            Request::write(
                &answer,
                r#"{"revision": 1, "actor": {}, "answer": {"choice": 1}}"#,
            ),
        ),
        (
            "a revision that is not a count",
            Request::write(&answer, r#"{"revision": "1", "answer": {"choice": 1}}"#),
        ),
    ] {
        let response = send(with_token(request));
        assert_eq!(response.status, 400, "{name}: {response:?}");
        assert_eq!(response.category(), "request", "{name}");
        assert!(response.json()["error"].get("case").is_none(), "{name}");
    }
    let malformed = send(with_token(Request::write(
        &answer,
        r#"{"revision": 1, "answer": {"choice": "two"}}"#,
    )));
    refused(&malformed, 400, "malformed");

    // 403: not from the inbox itself, or without its token.
    let port = serve.addr.rsplit(':').next().unwrap();
    for (name, request, category) in [
        (
            "a foreign Host",
            with_token(Request::write(&answer, good))
                .header("Host", &format!("evil.example:{port}")),
            "request",
        ),
        (
            "a cross-site fetch",
            with_token(Request::write(&answer, good)).header("Sec-Fetch-Site", "cross-site"),
            "request",
        ),
        (
            "a same-site fetch",
            with_token(Request::write(&answer, good)).header("Sec-Fetch-Site", "same-site"),
            "request",
        ),
        (
            "a foreign Origin",
            with_token(Request::write(&answer, good)).header("Origin", "http://evil.example"),
            "request",
        ),
        (
            "an Origin on another port",
            with_token(Request::write(&answer, good)).header("Origin", "http://127.0.0.1:9"),
            "request",
        ),
        (
            "a null Origin",
            with_token(Request::write(&answer, good)).header("Origin", "null"),
            "request",
        ),
        ("no token", Request::write(&answer, good), "csrf"),
        (
            "a wrong token",
            Request::write(&answer, good).header("X-Cases-CSRF", "0".repeat(32).as_str()),
            "csrf",
        ),
        (
            "an empty token",
            Request::write(&answer, good).header("X-Cases-CSRF", ""),
            "csrf",
        ),
    ] {
        let response = send(request);
        assert_eq!(response.status, 403, "{name}: {response:?}");
        assert_eq!(response.category(), category, "{name}");
    }

    // 404: no such case, or no such path.
    for other in ["zzzzz", "%FF"] {
        refused(
            &send(with_token(Request::write(
                &format!("/v1/cases/{other}/answer"),
                good,
            ))),
            404,
            "request",
        );
    }
    refused(
        &send(with_token(Request::write(
            &format!("/v1/cases/{id}/drop"),
            good,
        ))),
        404,
        "request",
    );
    refused(&serve.get("/v1/nothing"), 404, "request");
    refused(
        &send(with_token(Request::write("/v1/notifications", "{}"))),
        405,
        "request",
    );

    // 409: the write was made from an older view of the case.
    for body in [
        r#"{"answer": {"choice": 1}}"#,
        r#"{"revision": 0, "answer": {"choice": 1}}"#,
        r#"{"revision": 2, "answer": {"choice": 1}}"#,
    ] {
        let error = refused(
            &send(with_token(Request::write(&answer, body))),
            409,
            "stale",
        );
        assert_eq!(error["case"]["revision"], 1, "{body}");
    }

    // 422: the core refuses the event.
    for (body, category) in [
        (r#"{"revision": 1, "answer": {}}"#, "invalid"),
        (
            r#"{"revision": 1, "answer": {"note": "just a note"}}"#,
            "invalid",
        ),
        (r#"{"revision": 1, "answer": {"choice": 3}}"#, "invalid"),
        (
            r#"{"revision": 1, "answer": {"choice": 1, "drop": true}}"#,
            "invalid",
        ),
    ] {
        let error = refused(
            &send(with_token(Request::write(&answer, body))),
            422,
            category,
        );
        assert_eq!(error["case"]["state"], "open", "{body}");
    }
    let response = write(
        &serve,
        &withdrawn,
        "answer",
        r#"{"revision": 2, "answer": {"ack": true}}"#,
    );
    refused(&response, 422, "transition");

    assert_eq!(env.revision(&id), 1, "a refused write changed the case");
    assert_eq!(env.revision(&withdrawn), 2);
    // The same write from the inbox itself goes through: without Origin or
    // with its own, and with Sec-Fetch-Site same-origin or none.
    let own = send(
        with_token(Request::write(&answer, good))
            .header("Origin", &format!("http://{}", serve.addr))
            .header("Sec-Fetch-Site", "none"),
    );
    assert_eq!(own.status, 200, "{own:?}");
}

/// Opens one case of each kind, and gives their ids.
fn one_of_each_kind(env: &Env) -> Vec<String> {
    let mut ids = vec![
        env.open_decision(),
        env.open_approval(),
        env.open_fyi("Heads up"),
    ];
    for kind in ["signoff", "stuck", "question"] {
        ids.push(env.open(&["--kind", kind, "--urgency", "today", "--title", kind]));
    }
    ids
}

/// An answer with no response, or a note alone, is refused for every kind,
/// and writes nothing; the page never sends one (IN-03).
#[test]
fn an_answer_without_its_response_is_refused_for_every_kind() {
    let env = Env::new();
    let ids = one_of_each_kind(&env);
    let serve = env.serve(&[]);
    for id in &ids {
        for answer in [
            "{}",
            r#"{"note": "just a note"}"#,
            r#"{"text": "", "choice": 0}"#,
        ] {
            let body = format!(r#"{{"revision": 1, "answer": {answer}}}"#);
            let error = refused(&write(&serve, id, "answer", &body), 422, "invalid");
            assert_eq!(error["case"]["id"], id.as_str(), "{answer}");
        }
        assert_eq!(env.revision(id), 1, "{id}");
    }
}

/// Drop answers a case of every kind an agent opens (HU-02).
#[test]
fn drop_answers_every_kind() {
    let env = Env::new();
    let ids = one_of_each_kind(&env);
    let serve = env.serve(&[]);
    for id in &ids {
        let body = r#"{"revision": 1, "answer": {"drop": true, "note": "not now"}}"#;
        let response = write(&serve, id, "answer", body);
        assert_eq!(response.status, 200, "{id}: {response:?}");
        assert_eq!(response.json()["state"], "answered");
        assert_eq!(response.json()["answer"]["drop"], true);
    }
}

/// The checks come in the order spec/api.md gives: Host, cross-site and
/// token, then the case, then the body, then the revision, then the core.
#[test]
fn the_checks_come_in_the_contract_s_order() {
    let env = Env::new();
    let id = env.open_decision();
    let serve = env.serve(&[]);
    let token = serve.csrf();
    let send = |path: &str, body: &str| {
        serve.send(&Request::write(path, body).header("X-Cases-CSRF", &token))
    };
    let answer = format!("/v1/cases/{id}/answer");
    // No token, and no case: the token first.
    refused(
        &serve.send(&Request::write("/v1/cases/zzzzz/answer", "{")),
        403,
        "csrf",
    );
    // No case, and a body that is not JSON: the case first.
    refused(&send("/v1/cases/zzzzz/answer", "{"), 404, "request");
    // A body that is no answer, from an older view: the body first.
    refused(
        &send(&answer, r#"{"revision": 0, "answer": 1}"#),
        400,
        "request",
    );
    // An invalid answer from an older view: the revision first.
    refused(
        &send(&answer, r#"{"revision": 0, "answer": {}}"#),
        409,
        "stale",
    );
    refused(&send(&answer, r#"{"answer": {}}"#), 409, "stale");
    assert_eq!(env.revision(&id), 1);

    // A case whose state takes no answer, from an older view: the revision
    // before the state (AG-12).
    env.ok(&["withdraw", &id]);
    refused(
        &send(&answer, r#"{"revision": 1, "answer": {"choice": 1}}"#),
        409,
        "stale",
    );
    refused(
        &send(&answer, r#"{"revision": 2, "answer": {"choice": 1}}"#),
        422,
        "transition",
    );
    assert_eq!(env.revision(&id), 2);
}

/// A refused write gives the case as it is now, read after the refusal, so
/// the page can show it and send again (IN-05).
#[test]
fn a_refusal_carries_the_case_as_it_is_now() {
    let env = Env::new();
    let id = env.open_decision();
    let serve = env.serve(&[]);
    env.write(
        &id,
        "agent",
        "note",
        r#"{"body": "Pin to which **patch**?"}"#,
    );
    let error = refused(
        &write(
            &serve,
            &id,
            "answer",
            r#"{"revision": 1, "answer": {"choice": 1}}"#,
        ),
        409,
        "stale",
    );
    assert_eq!(error["case"]["revision"], 2);
    assert_eq!(
        error["case"]["events"][1]["data"]["body"],
        "Pin to which **patch**?"
    );
    let error = refused(
        &write(&serve, &id, "answer", r#"{"revision": 2, "answer": {}}"#),
        422,
        "invalid",
    );
    assert_eq!(error["case"]["revision"], 2);
    let sent = write(
        &serve,
        &id,
        "answer",
        r#"{"revision": 2, "answer": {"choice": 1}}"#,
    );
    assert_eq!(sent.status, 200, "{sent:?}");
}

/// An answer written without seeing the latest amend that changed the
/// question is stale, even at the case's revision (ST-04, IN-06).
#[test]
fn an_answer_that_missed_an_amend_is_stale() {
    let env = Env::new();
    let id = env.open_decision();
    env.write(&id, "agent", "amend", r#"{"options": ["Vendor it"]}"#);
    let serve = env.serve(&[]);
    let case = serve.get(&format!("/v1/cases/{id}")).json();
    assert_eq!(
        (case["revision"].as_u64(), case["amend_seq"].as_u64()),
        (Some(2), Some(2))
    );
    let error = refused(
        &write(
            &serve,
            &id,
            "answer",
            r#"{"revision": 2, "answer": {"choice": 3, "amend_seq": 0}}"#,
        ),
        409,
        "stale",
    );
    assert_eq!(error["case"]["amend_seq"], 2);
    let sent = write(
        &serve,
        &id,
        "answer",
        r#"{"revision": 2, "answer": {"choice": 3, "amend_seq": 2}}"#,
    );
    assert_eq!(sent.status, 200, "{sent:?}");
}

/// A write that has begun is made even if the page goes away first, as when
/// a tab is closed while the write waits for the store's lock (IN-03).
#[test]
fn a_write_is_made_even_when_the_page_goes_away() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    let token = serve.csrf();
    // Another writer holds the store's lock, so the write waits for it.
    let mut lock = rusqlite::Connection::open(&env.store).unwrap();
    let held = lock
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let body = r#"{"revision": 1, "answer": {"ack": true}}"#;
    let request = format!(
        "POST /v1/cases/{id}/answer HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
         X-Cases-CSRF: {token}\r\nContent-Length: {}\r\n\r\n{body}",
        serve.addr,
        body.len()
    );
    let mut page = common::serve::connect(&serve.addr);
    page.write_all(request.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    drop(page);
    std::thread::sleep(Duration::from_millis(200));
    held.rollback().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while env.revision(&id) == 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(env.case(&id).state().as_str(), "answered");
}

/// A write is made, and logged, even when its page goes away before the
/// write has begun: here it waits its turn behind another write, which
/// waits for the store's lock (IN-03, SV-02).
#[test]
fn a_write_waiting_its_turn_is_made_and_logged_when_its_page_goes_away() {
    let env = Env::new();
    let first = env.open_fyi("First");
    let second = env.open_fyi("Second");
    let serve = env.serve(&[]);
    let token = serve.csrf();
    let mut lock = rusqlite::Connection::open(&env.store).unwrap();
    let held = lock
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let body = r#"{"revision": 1, "answer": {"ack": true}}"#;
    let send = |id: &str| {
        let request = format!(
            "POST /v1/cases/{id}/answer HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             X-Cases-CSRF: {token}\r\nContent-Length: {}\r\n\r\n{body}",
            serve.addr,
            body.len()
        );
        let mut page = common::serve::connect(&serve.addr);
        page.write_all(request.as_bytes()).unwrap();
        page
    };
    // The first write waits for the lock, and holds serve's store while it
    // does; the second is sent in full, and waits for the store.
    let waiting = send(&first);
    std::thread::sleep(Duration::from_millis(500));
    let gone = send(&second);
    std::thread::sleep(Duration::from_millis(300));
    drop(gone);
    std::thread::sleep(Duration::from_millis(200));
    held.rollback().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while env.revision(&second) == 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(waiting);
    assert_eq!(env.case(&second).state().as_str(), "answered");
    let log = serve.stop().stderr;
    assert!(
        log.contains(&format!("POST /v1/cases/{second}/answer 200 ")),
        "{log}"
    );
}

/// A write that is made is a 200 with the case, even when the actors cannot
/// be read after it: they are then shown by their ids.
#[test]
fn a_write_made_is_a_200_even_when_the_actors_cannot_be_read() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    // A row that no cases writes: a name that is not UTF-8.
    rusqlite::Connection::open(&env.store)
        .unwrap()
        .execute(
            "INSERT INTO actors (id, kind, name, created) \
             VALUES ('AAAAAAAAAAAAAAAAAAAAAA', 'agent', CAST(x'ff' AS TEXT), '2026-09-25T12:00:00Z')",
            [],
        )
        .unwrap();
    let response = write(
        &serve,
        &id,
        "answer",
        r#"{"revision": 1, "answer": {"ack": true}}"#,
    );
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.json()["state"], "answered");
    assert_eq!(env.revision(&id), 2);
}

/// The stream sends the store's version when it moves, and a page that
/// reconnects says which version it had: one that missed a change is sent
/// it at once, and one that did not is sent nothing (IN-06).
#[test]
fn the_stream_sends_each_change_and_catches_up_after_a_reconnect() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    let first = serve.stream("", None).next();
    let open = serve.stream(&format!("?after={first}"), None);
    open.quiet(Duration::from_millis(300));

    env.write(&id, "human", "answer", r#"{"ack": true}"#);
    let second = open.next();
    assert_ne!(second, first);
    // A reconnect's Last-Event-ID counts over the page's own version.
    let again = serve.stream(&format!("?after={first}"), Some(&second));
    again.quiet(Duration::from_millis(300));
    env.open_fyi("Another");
    let third = open.next();
    assert_eq!(again.next(), third);
    // A page drawn before changes it was not sent is sent the version at
    // once.
    assert_eq!(serve.stream(&format!("?after={first}"), None).next(), third);

    // The version depends only on the store: a page keeps it across a
    // restart of serve, and is sent nothing if nothing moved meanwhile.
    assert_eq!(serve.stop().code, 0);
    let restarted = env.serve(&[]);
    restarted
        .stream(&format!("?after={third}"), None)
        .quiet(Duration::from_millis(1_500));
    assert_eq!(restarted.stream("", None).next(), third);
}

/// A stream ends when its page goes away, as when its tab is closed:
/// serve closes the connection rather than hold it.
#[test]
fn a_stream_ends_when_its_page_goes_away() {
    use std::io::Read as _;
    let env = Env::new();
    let serve = env.serve(&[]);
    let mut page = common::serve::connect(&serve.addr);
    let request = format!("GET /v1/changes HTTP/1.1\r\nHost: {}\r\n\r\n", serve.addr);
    page.write_all(request.as_bytes()).unwrap();
    page.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut head = [0; 64];
    let read = page.read(&mut head).unwrap();
    assert!(
        head[..read].starts_with(b"HTTP/1.1 200"),
        "{:?}",
        String::from_utf8_lossy(&head[..read])
    );
    // The page goes: it sends nothing more, and serve sees its end.
    page.shutdown(std::net::Shutdown::Write).unwrap();
    let mut rest = Vec::new();
    let started = Instant::now();
    let read = page.read_to_end(&mut rest);
    assert!(
        read.is_ok(),
        "serve held the stream open for {:?}",
        started.elapsed()
    );
}

/// A burst of writes makes one message a second, with the version as it is
/// when the message goes, so nothing is missed (IN-06).
#[test]
fn the_stream_holds_its_messages_a_second_apart() {
    // A burst has to fit in the hold, or serve rightly sends a second
    // message part way through it. How long three notes take depends on the
    // load of the machine (under instrumented coverage with other builds
    // running, over a second), so a run too slow to make a burst is tried
    // again on a fresh serve. Only that precondition is retried: with the
    // hold removed every note is a message of its own, and every run fails
    // the checks on the gap.
    const FAST: Duration = Duration::from_millis(500);
    let mut bursts = Vec::new();
    for _ in 0..5 {
        let env = Env::new();
        let id = env.open(&["--kind", "question", "--urgency", "today", "--title", "Q"]);
        let serve = env.serve(&[]);
        let version = serve.stream("", None).next();
        let stream = serve.stream(&format!("?after={version}"), None);
        // A read of the API reads the store at once, so the stream learns of
        // each note as it is written: only the gap holds a message back.
        let note = || {
            env.write(&id, "agent", "note", r#"{"body": "More."}"#);
            assert_eq!(serve.get("/v1/cases").status, 200);
        };
        note();
        stream.next();
        let first = Instant::now();
        for _ in 0..3 {
            note();
        }
        let burst = first.elapsed();
        if burst >= FAST {
            bursts.push(burst);
            continue;
        }
        let last = stream.next();
        let gap = first.elapsed();
        assert!(
            gap >= Duration::from_millis(900),
            "a second message {gap:?} after the first"
        );
        assert_eq!(
            last,
            serve.stream("", None).next(),
            "the message missed a write"
        );
        // The burst made one message.
        stream.quiet(Duration::from_millis(1_200));
        return;
    }
    panic!("no burst of three notes took under {FAST:?}: {bursts:?}");
}

/// The notification feed: nothing for what was in the store when serve
/// started, then each case that lands on the human, by number (IN-08).
#[test]
fn the_notification_feed_numbers_what_lands_on_the_human() {
    let env = Env::new();
    env.open_fyi("Already here");
    let serve = env.serve(&[]);
    let first = serve.get("/v1/notifications");
    assert_eq!(first.status, 200);
    assert_eq!(first.header("cache-control"), Some("no-store"));
    let first = first.json();
    assert_eq!(
        (
            first["latest"].as_u64(),
            first["items"].as_array().map(Vec::len)
        ),
        (Some(0), Some(0))
    );
    let boot = first["boot"].as_str().unwrap().to_owned();
    assert!(!boot.is_empty());

    let id = env.open(&[
        "--kind",
        "decision",
        "--urgency",
        "blocking",
        "--title",
        "Pin bun?",
        "--option",
        "Pin",
        "--option",
        "Float",
        "--worker",
        "bun-pins",
    ]);
    let page = serve.get("/v1/notifications?after=0").json();
    assert_eq!(
        (page["boot"].as_str(), page["latest"].as_u64()),
        (Some(boot.as_str()), Some(1))
    );
    let item = &page["items"][0];
    assert_eq!(item["id"], 1);
    assert_eq!(item["sink"], "browser");
    assert_eq!(item["title"], "Pin bun?");
    assert_eq!(item["body"], "blocking decision · from bun-pins");
    assert_eq!(item["url"], format!("/cases/{id}-pin-bun"));
    assert_eq!(item["tag"], format!("{id}/0001-agent-open.json"));
    assert_eq!(item["event"]["name"], "open");
    assert_eq!(item["event"]["file"], "0001-agent-open.json");
    assert_eq!(item["case"]["urgency"], "blocking");
    assert_eq!(item["case"]["id"], id.as_str());
    // The case is show --json's, its page in this inbox and all.
    assert_eq!(
        item["case"]["url"],
        format!("{}cases/{id}-pin-bun", serve.url)
    );
    let after = serve.get("/v1/notifications?after=1").json();
    assert_eq!(
        (
            after["latest"].as_u64(),
            after["items"].as_array().map(Vec::len)
        ),
        (Some(1), Some(0))
    );
    // An empty after is none, as the Go inbox read it, and the first after
    // counts.
    for query in ["?after=", "?after=0&after=1"] {
        let page = serve.get(&format!("/v1/notifications{query}")).json();
        assert_eq!(page["items"].as_array().map(Vec::len), Some(1), "{query}");
    }
    for bad in ["x", "-1", "1.5"] {
        refused(
            &serve.get(&format!("/v1/notifications?after={bad}")),
            400,
            "request",
        );
    }
    assert_eq!(serve.stop().code, 0);
    let other = env.serve(&[]).get("/v1/notifications").json();
    assert_ne!(
        other["boot"].as_str(),
        Some(boot.as_str()),
        "the boot id outlived its serve"
    );
}

#[test]
fn the_tab_icon_is_drawn_for_its_query() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let icon = serve.get("/favicon.svg?n=3&blocking=1");
    assert_eq!(icon.status, 200);
    assert_eq!(icon.header("content-type"), Some("image/svg+xml"));
    assert_eq!(icon.header("cache-control"), Some("max-age=3600"));
    assert!(
        icon.body.contains(">3</text>") && icon.body.contains("#a11b2b"),
        "{}",
        icon.body
    );
    for bad in ["n=x", "n=-1", "n=1&blocking=yes"] {
        refused(&serve.get(&format!("/favicon.svg?{bad}")), 400, "request");
    }
    // An empty value is none, and the first value counts, as the Go inbox
    // read them.
    let mark = serve.get("/favicon.svg").body;
    assert_eq!(serve.get("/favicon.svg?n=&blocking=").body, mark);
    let ink = serve.get("/favicon.svg?n=3&blocking=&n=7");
    assert_eq!(ink.status, 200, "{ink:?}");
    assert!(
        ink.body.contains(">3</text>") && !ink.body.contains("#a11b2b"),
        "{}",
        ink.body
    );
}

/// The API changes only by adding within /v1 (IN-12): every field each read
/// gives today is still there. A field may be added; this list is only ever
/// added to.
#[test]
fn each_read_keeps_the_fields_it_gives() {
    let env = Env::new();
    let id = env.open_decision();
    env.ok(&["amend", &id, "--recommend", "2"]);
    env.ok(&["answer", &id, "--option", "1", "--note", "n"]);
    let serve = env.serve(&[]);
    let has = |value: &serde_json::Value, keys: &[&str], what: &str| {
        for key in keys {
            assert!(value.get(key).is_some(), "{what} lost {key}: {value}");
        }
    };
    has(&serve.get("/v1").json(), &["version", "csrf"], "/v1");
    let case_keys = [
        "id",
        "state",
        "kind",
        "urgency",
        "title",
        "options",
        "recommend",
        "opened_at",
        "updated_at",
        "answer",
        "events",
        "revision",
        "amend_seq",
        "last_seq",
        "url",
    ];
    let case = serve.get(&format!("/v1/cases/{id}")).json();
    has(&case, &case_keys, "/v1/cases/{id}");
    has(
        &case["events"][1],
        &[
            "seq", "author", "event", "from", "file", "at", "actor", "data",
        ],
        "an event",
    );
    has(&case["events"][1]["actor"], &["id", "kind"], "an actor");
    let list = serve.get("/v1/cases?state=answered").json();
    has(&list, &["list"], "/v1/cases");
    has(
        &list["list"][0],
        &case_keys[..case_keys.len() - 1],
        "a listed case",
    );
    has(
        &serve.get("/v1/notifications").json(),
        &["boot", "latest", "items"],
        "/v1/notifications",
    );
    env.db()
        .listen(
            "bun-pins",
            "bun-pins",
            &["release".to_owned()],
            &[],
            Some("rebasing"),
        )
        .unwrap();
    let listeners = serve.get("/v1/listeners").json();
    has(&listeners, &["list"], "/v1/listeners");
    has(
        &listeners["list"][0],
        &[
            "worker", "labels", "since", "heard", "doing", "doing_at", "waits",
        ],
        "a listener",
    );
    has(
        &listeners["list"][0]["waits"][0],
        &["labels", "kinds"],
        "a listener's wait",
    );
}

/// A message's open record as the inbox's composer sends it, to `worker`,
/// with the fields `more` adds.
fn message(worker: &str, more: &str) -> String {
    format!(
        r#"{{"kind": "message", "urgency": "today", "title": "Rebase onto main", "body": "Then run the tests.", "worker": "{worker}"{more}}}"#
    )
}

/// The human sends a message from the inbox: one `human open` event with
/// serve's human, stamped by serve, and the case, sent. The first message
/// makes the store. `re` names a case by its whole id, with or without its
/// title slug, and is kept bare (IN-13, HU-08).
#[test]
fn a_message_is_sent_as_serves_human() {
    let env = Env::new();
    let serve = env.serve(&["--as", "Ryan"]);
    let response = serve.post(
        "/v1/cases",
        &message("bun-pins", r#", "labels": ["release"]"#),
    );
    assert_eq!(response.status, 200, "{response:?}");
    let case = response.json();
    let id = case["id"].as_str().unwrap().to_owned();
    assert_eq!(
        (&case["kind"], &case["state"], &case["revision"]),
        (&"message".into(), &"sent".into(), &1.into())
    );
    let event = &case["events"][0];
    assert_eq!(
        (&event["author"], &event["event"]),
        (&"human".into(), &"open".into())
    );
    assert_eq!(
        (&event["actor"]["kind"], &event["actor"]["name"]),
        (&"human".into(), &"Ryan".into())
    );
    assert!(event["data"]["opened_at"].is_string(), "{event}");
    assert_eq!(event["data"]["worker"], "bun-pins");
    assert_eq!(event["data"]["labels"], serde_json::json!(["release"]));
    assert_eq!(case, json(&env.ok(&["show", "--json", &id])));
    // It waits on an agent: the inbox's list leaves it out, so it counts in
    // no tally, title or icon, and in flight's list has it.
    assert_eq!(
        serve.get("/v1/cases").json(),
        serde_json::json!({"list": []})
    );
    let sent = serve.get("/v1/cases?state=sent").json();
    assert_eq!(sent["list"][0]["id"], id.as_str());

    let slugged = case["url"].as_str().unwrap().rsplit('/').next().unwrap();
    assert_ne!(slugged, id, "{case}");
    for re in [id.as_str(), slugged] {
        let response = serve.post(
            "/v1/cases",
            &message("bun-pins", &format!(r#", "re": "{re}""#)),
        );
        assert_eq!(response.status, 200, "{re}: {response:?}");
        assert_eq!(response.json()["re"], id.as_str(), "{re}");
    }
}

/// A message the route or the core refuses has the status spec/api.md pins
/// and makes no case, nor the store: another kind from the human, a message
/// to no one, a stamp, and a `re` that names no case by its whole id (IN-04,
/// IN-13, HU-07).
#[test]
fn a_refused_message_makes_no_case() {
    let env = Env::new();
    let serve = env.serve(&["--as", "Ryan"]);
    let token = serve.csrf();
    let send =
        |body: &str| serve.send(&Request::write("/v1/cases", body).header("X-Cases-CSRF", &token));
    let refusals = |about: &str| {
        let part = about.get(1..).unwrap_or_default();
        vec![
            ("not JSON", "{".to_owned(), 400, "request"),
            ("not an object", "[1]".to_owned(), 400, "request"),
            (
                "a time",
                message("w", r#", "opened_at": "2026-09-27T12:00:00Z""#),
                400,
                "request",
            ),
            (
                "an actor",
                message("w", r#", "actor": {"id": "AAAAAAAAAAAAAAAAAAAAAA"}"#),
                400,
                "request",
            ),
            (
                "a re of no case",
                message("w", r#", "re": "gbHJd""#),
                400,
                "request",
            ),
            (
                "a re of part of an id",
                message("w", &format!(r#", "re": "{part}""#)),
                400,
                "request",
            ),
            (
                "a title that is not text",
                r#"{"kind": "message", "urgency": "today", "title": 1, "worker": "w"}"#.to_owned(),
                400,
                "malformed",
            ),
            (
                "another kind",
                r#"{"kind": "decision", "urgency": "today", "title": "T", "options": ["a", "b"]}"#
                    .to_owned(),
                422,
                "author",
            ),
            (
                "to no one",
                r#"{"kind": "message", "urgency": "today", "title": "T", "worker": " "}"#
                    .to_owned(),
                422,
                "invalid",
            ),
        ]
    };
    for (name, body, status, category) in refusals("UkLWZ") {
        let error = refused(&send(&body), status, category);
        assert!(error.get("case").is_none(), "{name}: {error}");
    }
    let good = message("w", "");
    let no_token = serve.send(&Request::write("/v1/cases", &good));
    refused(&no_token, 403, "csrf");
    let cross = serve.send(
        &Request::write("/v1/cases", &good)
            .header("X-Cases-CSRF", &token)
            .header("Sec-Fetch-Site", "cross-site"),
    );
    refused(&cross, 403, "request");
    assert!(!env.store.exists(), "a refused message made the store");

    let about = env.open_fyi("Nightly build moved");
    for (name, body, status, category) in refusals(&about) {
        refused(&send(&body), status, category);
        assert!(
            env.db().rows(&common::key("gbHJd")).unwrap().is_none(),
            "{name} made a case"
        );
    }
    let response = send(&message("w", &format!(r#", "re": "{about}""#)));
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.json()["id"], "gbHJd");
}

/// The human edits a message until an agent takes it, guarded by the
/// revision, and withdraws it, sent or taken. A case an agent opened is the
/// agent's to amend (IN-13, ST-14, AG-17).
#[test]
fn the_human_edits_and_withdraws_a_message() {
    let env = Env::new();
    let decision = env.open_decision();
    let serve = env.serve(&["--as", "Ryan"]);
    let sent = |body: &str| {
        let response = serve.post("/v1/cases", body);
        assert_eq!(response.status, 200, "{response:?}");
        response.json()["id"].as_str().unwrap().to_owned()
    };
    let id = sent(&message("bun-pins", ""));
    let response = write(
        &serve,
        &id,
        "amend",
        r#"{"revision": 1, "body": "Rebase, then test.", "labels": ["release"]}"#,
    );
    assert_eq!(response.status, 200, "{response:?}");
    let case = response.json();
    assert_eq!(
        (&case["state"], &case["revision"]),
        (&"sent".into(), &2.into())
    );
    let event = &case["events"][1];
    assert_eq!(
        (&event["author"], &event["event"], &event["actor"]["name"]),
        (&"human".into(), &"amend".into(), &"Ryan".into())
    );
    assert!(event["data"]["amended_at"].is_string(), "{event}");
    assert_eq!(case, json(&env.ok(&["show", "--json", &id])));
    assert_eq!(env.case(&id).body(), "Rebase, then test.");

    let error = refused(
        &write(&serve, &id, "amend", r#"{"revision": 1, "body": "x"}"#),
        409,
        "stale",
    );
    assert_eq!(error["case"]["revision"], 2);
    let error = refused(
        &write(
            &serve,
            &decision,
            "amend",
            r#"{"revision": 1, "body": "x"}"#,
        ),
        422,
        "author",
    );
    assert_eq!(error["case"]["id"], decision.as_str());
    refused(
        &write(
            &serve,
            &id,
            "amend",
            r#"{"revision": 2, "body": "x", "amended_at": "2026-09-27T12:00:00Z"}"#,
        ),
        400,
        "request",
    );
    env.ok(&["pickup", &id, "--by", "bun-pins"]);
    let error = refused(
        &write(&serve, &id, "amend", r#"{"revision": 3, "body": "x"}"#),
        422,
        "transition",
    );
    assert_eq!(error["case"]["state"], "pickedup");
    assert_eq!(env.revision(&decision), 1);

    let other = sent(&message("lead-2", ""));
    for (id, revision) in [(&id, 3), (&other, 1)] {
        let body = format!(r#"{{"revision": {revision}, "reason": "done by hand"}}"#);
        let response = write(&serve, id, "withdraw", &body);
        assert_eq!(response.status, 200, "{id}: {response:?}");
        assert_eq!(response.json()["state"], "withdrawn");
    }
    assert_eq!(env.case(&id).pickup().unwrap().by, "bun-pins");
}

/// The workers listening for messages now, by serve's clock, with their
/// labels and doing lines: none for a store not made yet, and a record that
/// has lapsed is not listed (ST-16, IN-13).
#[test]
fn listeners_are_the_workers_listening_now() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let listeners = || {
        let response = serve.get("/v1/listeners");
        assert_eq!(response.status, 200, "{response:?}");
        assert_eq!(response.header("content-type"), Some("application/json"));
        response.json()["list"].clone()
    };
    assert_eq!(listeners(), serde_json::json!([]));
    assert!(!env.store.exists(), "a read made the store");

    env.open_fyi("A case, so that the store is there");
    assert_eq!(listeners(), serde_json::json!([]));
    let db = env.db();
    db.listen("lead-2", "lead-2", &[], &[], None).unwrap();
    db.listen(
        "bun-pins",
        "bun-pins",
        &["release".to_owned()],
        &[],
        Some("rebasing onto main"),
    )
    .unwrap();
    let list = listeners();
    assert_eq!(list.as_array().map(Vec::len), Some(2), "{list}");
    let (first, second) = (&list[0], &list[1]);
    assert_eq!(first["worker"], "bun-pins");
    assert_eq!(first["labels"], serde_json::json!(["release"]));
    assert_eq!(first["doing"], "rebasing onto main");
    assert_eq!(
        first["waits"],
        serde_json::json!([{"labels": ["release"], "kinds": []}])
    );
    for field in ["since", "heard", "doing_at"] {
        assert!(first[field].is_string(), "{field}: {first}");
    }
    assert_eq!(
        (&second["worker"], &second["labels"]),
        (&"lead-2".into(), &serde_json::json!([]))
    );
    for field in ["doing", "doing_at"] {
        assert!(second.get(field).is_none(), "{field}: {second}");
    }

    // As if 50 s had passed since lead-2's last heartbeat.
    let conn = rusqlite::Connection::open(&env.store).unwrap();
    conn.execute(
        "UPDATE waits SET heard = ?1 WHERE worker = 'lead-2'",
        [common::from_now(-50)],
    )
    .unwrap();
    let list = listeners();
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    assert_eq!(list[0]["worker"], "bun-pins");
}
