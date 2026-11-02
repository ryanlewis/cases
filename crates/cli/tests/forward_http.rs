//! The CLI on a store forwarded over HTTP: `cases` with
//! `CASES_FORWARD=http://127.0.0.1:<port>` and a `cases serve` on that
//! port, against a store file of its own, as `tailscale serve --http=PORT`
//! on the far side passes a request on to it. A relay stands in for the
//! proxy: it passes each request on with the headers `tailscale serve`
//! adds, `X-Forwarded-Host` and, for a tailnet user, `Tailscale-User-Login`.
//! The store routes' checks are then tried by hand, request by request.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{Shutdown, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use common::serve::{Request, Serve};
use common::{Env, Output, spawn};

/// A token of the far side's `store-tokens-file`.
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// Another, which is not.
const WRONG: &str = "fedcba9876543210fedcba9876543210";

/// The far side, a serve on its store, and the near side, a home that
/// forwards to it over HTTP.
struct Http {
    far: Env,
    serve: Serve,
    near: Env,
}

impl Http {
    /// A serve on a store of its own with `config` as its config file and
    /// `vars` in its environment, and a home forwarding to it, straight to
    /// its loopback address.
    fn new(config: &str, vars: &[(&str, &str)]) -> Http {
        let mut far = Env::new();
        for (var, value) in vars {
            far.set(var, value);
        }
        let tokens = far.path("store-tokens");
        fs::write(&tokens, format!("# the store's tokens\n{TOKEN}\n")).unwrap();
        fs::set_permissions(&tokens, fs::Permissions::from_mode(0o600)).unwrap();
        far.config(&format!(
            "store-tokens-file = \"{}\"\n{config}",
            tokens.display()
        ));
        let serve = far.serve(&["--no-open"]);
        let mut near = Env::new();
        near.set("CASES_FORWARD", &format!("http://{}", serve.addr));
        Http { far, serve, near }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.near.run(args)
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.code, 0, "cases {args:?}: {out:?}");
        out.stdout
    }

    /// Runs `cases --store <far>` on the far side's file itself.
    fn far(&self, args: &[&str]) -> String {
        self.far.ok(args)
    }

    /// The near side's forward-token-file, holding `token`.
    fn send_token(&mut self, token: &str) {
        let path = self.near.path("forward-token");
        fs::write(&path, format!("{token}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        self.near
            .set("CASES_FORWARD_TOKEN_FILE", path.to_str().unwrap());
    }

    /// Forwards the near side through a relay that adds `headers` to each
    /// request, as `tailscale serve` does.
    fn through_proxy(&mut self, headers: &[(&str, &str)]) {
        let relay = relay(&self.serve.addr, headers);
        self.near.set("CASES_FORWARD", &format!("http://{relay}"));
    }

    fn open_decision(&self) -> String {
        self.ok(&[
            "open",
            "--kind",
            "decision",
            "--urgency",
            "today",
            "--title",
            "Ship it?",
            "--option",
            "yes",
            "--option",
            "no",
            "--worker",
            "lead",
        ])
        .trim()
        .to_owned()
    }

    /// Opens a case, waits on it with a pickup, answers it, and checks
    /// that the wait took it and the far side's store has it all.
    fn round_trip(&self) {
        let id = self.open_decision();
        assert!(self.far(&["list"]).contains("Ship it?"));
        assert!(!self.near.store.exists());
        let waiting = spawn(
            self.near
                .command([
                    "wait",
                    "--id",
                    &id,
                    "--pickup",
                    "--worker",
                    "lead",
                    "--timeout",
                    "60s",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        thread::sleep(Duration::from_millis(500));
        let answered = Instant::now();
        assert_eq!(
            self.ok(&["answer", &id, "--option", "1", "--as", "ryan"]),
            format!("{id} answered\n")
        );
        let waited: Output = waiting.wait_with_output().unwrap().into();
        assert_eq!(waited.code, 0, "{waited:?}");
        assert!(
            waited.stdout.contains(r#""state":"pickedup""#),
            "{waited:?}"
        );
        // The held poll wakes the wait.
        assert!(answered.elapsed() < Duration::from_secs(5));
        assert_eq!(
            self.ok(&["close", &id, "--outcome", "shipped"]),
            format!("{id} closed\n")
        );
        let shown = self.ok(&["show", &id]);
        assert!(shown.contains("state:    closed"), "{shown}");
        assert!(shown.contains("by ryan"), "{shown}");
        assert_eq!(self.far(&["show", &id]), shown);
    }

    /// The far side's answer to `request` on the store route `route`.
    fn store_route(&self, request: &Request) -> common::serve::Response {
        self.serve.send(request)
    }
}

/// The path of the store route `route`.
fn route(route: &str) -> String {
    format!("/v1/stores/st_dialstdio0000000/store/{route}")
}

/// A relay on a port of its own that passes each request on to `to` with
/// `headers` added, and each answer back, as a proxy does: the address it
/// listens on.
fn relay(to: &str, headers: &[(&str, &str)]) -> String {
    let listener = common::listen("127.0.0.1:0");
    let addr = listener.local_addr().unwrap().to_string();
    let added = headers
        .iter()
        .fold(String::new(), |mut added, (name, value)| {
            let _ = write!(added, "{name}: {value}\r\n");
            added
        });
    let to = to.to_owned();
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let server = common::serve::connect(&to);
            let added = added.clone();
            let (mut back, mut answers) =
                (client.try_clone().unwrap(), server.try_clone().unwrap());
            thread::spawn(move || {
                let _ = std::io::copy(&mut answers, &mut back);
                let _ = back.shutdown(Shutdown::Both);
            });
            thread::spawn(move || pass_on(client, server, &added));
        }
    });
    addr
}

/// Passes each request read from `client` on to `server`, with `added`
/// after its request line.
fn pass_on(client: TcpStream, mut server: TcpStream, added: &str) {
    let mut client = BufReader::new(client);
    loop {
        let mut head = Vec::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            if client.read_line(&mut line).unwrap_or(0) == 0 {
                let _ = server.shutdown(Shutdown::Write);
                return;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap();
            }
            let end = line == "\r\n";
            let first = head.is_empty();
            head.extend_from_slice(line.as_bytes());
            // The headers go after the request line.
            if first {
                head.extend_from_slice(added.as_bytes());
            }
            if end {
                break;
            }
        }
        let mut body = vec![0; length];
        if client.read_exact(&mut body).is_err() {
            return;
        }
        head.extend(body);
        if server.write_all(&head).is_err() {
            return;
        }
    }
}

#[test]
fn a_case_goes_round_over_http() {
    let h = Http::new("", &[]);
    assert_eq!(h.ok(&["list"]), "No cases.\n");
    h.round_trip();
    // The inbox's own page is served as before, next to the store routes.
    assert_eq!(h.serve.get("/").status, 200);
}

#[test]
fn a_case_goes_round_through_a_proxy_with_a_token() {
    let mut h = Http::new("", &[]);
    h.through_proxy(&[("X-Forwarded-Host", "devbox.example.ts.net:7338")]);
    h.send_token(TOKEN);
    h.round_trip();
}

#[test]
fn a_case_goes_round_through_a_proxy_from_a_listed_login() {
    let mut h = Http::new("tailscale-logins = [\"ryan@github\"]\n", &[]);
    h.through_proxy(&[
        ("X-Forwarded-Host", "devbox.example.ts.net:7338"),
        ("Tailscale-User-Login", "ryan@github"),
    ]);
    h.round_trip();
}

#[test]
fn a_proxied_command_with_no_token_and_no_login_is_refused_and_says_why() {
    let mut h = Http::new("tailscale-logins = [\"ryan@github\"]\n", &[]);
    // A tagged device: the proxy names no login.
    h.through_proxy(&[("X-Forwarded-Host", "devbox.example.ts.net:7338")]);
    let out = h.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("refused") && out.stderr.contains("forward-token-file"),
        "{out:?}"
    );
    h.send_token(WRONG);
    let out = h.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("not one of the store-tokens-file"),
        "{out:?}"
    );
    assert!(!out.stderr.contains(WRONG), "{out:?}");
    assert!(!h.far.store.exists());
}

/// Each check of a store route, by hand: one straight to loopback is let
/// in; one through the proxy needs a good token or a listed login; funnel
/// and a browser are refused whatever they carry.
#[test]
fn the_store_routes_let_in_only_loopback_a_token_or_a_listed_login() {
    let h = Http::new(
        "allow-hosts = [\"devbox.example.ts.net:7337\"]\ntailscale-logins = [\"ryan@github\"]\n",
        &[],
    );
    let get = || Request::new("GET", &route("changes"));
    let proxied = || get().header("X-Forwarded-Host", "devbox.example.ts.net:7338");
    let refused = |request: Request, why: &str| {
        let answer = h.store_route(&request);
        assert_eq!(answer.status, 401, "{request:?}: {answer:?}");
        assert_eq!(answer.category(), "auth", "{request:?}");
        let message = answer.json()["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(message.contains(why), "{request:?}: {message}");
        assert!(
            !message.contains(TOKEN) && !message.contains(WRONG),
            "{message}"
        );
    };
    let taken = |request: Request| {
        let answer = h.store_route(&request);
        assert_eq!(answer.status, 200, "{request:?}: {answer:?}");
        assert!(answer.json()["version"].is_string(), "{answer:?}");
    };

    taken(get());
    taken(proxied().header("Authorization", &format!("Bearer {TOKEN}")));
    taken(proxied().header("Tailscale-User-Login", "ryan@github"));
    // A Host of allow-hosts is through the proxy too.
    let allowed = || get().header("Host", "devbox.example.ts.net:7337");
    taken(allowed().header("Authorization", &format!("Bearer {TOKEN}")));

    refused(proxied(), "needs a token");
    refused(allowed(), "needs a token");
    refused(
        proxied().header("Authorization", &format!("Bearer {WRONG}")),
        "not one of the store-tokens-file",
    );
    refused(
        proxied().header("Authorization", &format!("Bearer {}", &TOKEN[1..])),
        "not one of the store-tokens-file",
    );
    refused(
        proxied().header("Authorization", TOKEN),
        "not one of the store-tokens-file",
    );
    // A wrong token is not saved by a good login.
    refused(
        proxied()
            .header("Authorization", &format!("Bearer {WRONG}"))
            .header("Tailscale-User-Login", "ryan@github"),
        "not one of the store-tokens-file",
    );
    refused(
        proxied().header("Tailscale-User-Login", "eve@github"),
        "needs a token",
    );
    for request in [
        get(),
        proxied().header("Authorization", &format!("Bearer {TOKEN}")),
        proxied().header("Tailscale-User-Login", "ryan@github"),
    ] {
        refused(
            request.clone().header("Tailscale-Funnel-Request", "?1"),
            "funnel",
        );
        refused(
            request.clone().header("Origin", "https://evil.example"),
            "browser",
        );
        refused(request.header("Sec-Fetch-Site", "cross-site"), "browser");
    }
    refused(
        get()
            .header("Host", "evil.example")
            .header("Authorization", &format!("Bearer {TOKEN}")),
        "Host is not this server",
    );
    // A write is checked as a read is.
    let open = r#"{"writer": null, "append": {"author": "agent", "event": "open", "data": "{}"}}"#;
    refused(
        Request::new("POST", &route("cases"))
            .header("Content-Type", "application/json")
            .header("X-Forwarded-Host", "devbox.example.ts.net:7338")
            .body(open.as_bytes()),
        "needs a token",
    );
    assert!(!h.far.store.exists());
    // The inbox's own checks are as they were: a proxied read from a
    // listed login passes, and a token is no way in.
    let inbox = |request: Request| h.serve.send(&request).status;
    assert_eq!(
        inbox(
            Request::new("GET", "/v1/cases")
                .header("Host", "devbox.example.ts.net:7337")
                .header("Tailscale-User-Login", "ryan@github")
        ),
        200
    );
    assert_eq!(
        inbox(
            Request::new("GET", "/v1/cases")
                .header("Host", "devbox.example.ts.net:7337")
                .header("Authorization", &format!("Bearer {TOKEN}"))
        ),
        403
    );
}

/// With no tokens and no logins, nothing through the proxy is let in.
#[test]
fn with_no_tokens_and_no_logins_a_proxied_store_route_is_refused() {
    let far = Env::new();
    far.config("allow-hosts = [\"devbox.example.ts.net:7337\"]\n");
    let serve = far.serve(&["--no-open"]);
    for request in [
        Request::new("GET", &route("changes")).header("X-Forwarded-Host", "devbox"),
        Request::new("GET", &route("changes")).header("Host", "devbox.example.ts.net:7337"),
        Request::new("GET", &route("changes"))
            .header("X-Forwarded-Host", "devbox")
            .header("Tailscale-User-Login", "ryan@github"),
    ] {
        assert_eq!(serve.send(&request).status, 401, "{request:?}");
    }
    assert_eq!(
        serve.send(&Request::new("GET", &route("changes"))).status,
        200
    );
}

#[test]
fn a_store_tokens_file_that_is_not_one_stops_serve() {
    let far = Env::new();
    let tokens = far.path("store-tokens");
    fs::write(&tokens, "too-short\n").unwrap();
    far.config(&format!("store-tokens-file = \"{}\"\n", tokens.display()));
    let out = far.run([
        "--store",
        far.store.to_str().unwrap(),
        "serve",
        "--listen",
        "127.0.0.1:0",
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("store-tokens-file") && out.stderr.contains("line 1"),
        "{out:?}"
    );
    assert!(!out.stderr.contains("too-short"), "{out:?}");
}

/// A pickup wait over HTTP holds its locks on a lease, renewed by id: it
/// holds them past the lease while it runs, lets them go at once when it
/// ends, and one killed outright keeps them until the lease runs out.
#[test]
fn a_pickup_wait_over_http_takes_renews_and_lets_go_of_its_lease() {
    let mut h = Http::new("", &[("CASES_TEST_LEASE", "3000")]);
    h.open_decision();
    h.through_proxy(&[("X-Forwarded-Host", "devbox.example.ts.net:7338")]);
    h.send_token(TOKEN);
    let locks = h.far.store.with_file_name("cases.db-waits");
    let first = picking_up(&h, &locks);
    let (lock, first_holder) = taken(&locks, "worker-lead-");
    let again = h.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    thread::sleep(Duration::from_millis(3500));
    let again = h.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    kill("TERM", first.id());
    let waited = first.wait_with_output().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&waited.status),
        Some(15),
        "{waited:?}"
    );
    // Read off the lock, as a second wait is started some time before it
    // takes it.
    let ended = Instant::now();
    while kept(&lock, &first_holder) {
        assert!(
            ended.elapsed() < Duration::from_millis(1500),
            "not let go at once"
        );
        thread::sleep(Duration::from_millis(10));
    }
    starts_within(&h, Duration::from_secs(10));

    // The lock file names the last wait until another takes it.
    let before = holder(&locks, "worker-lead-");
    let mut killed = picking_up(&h, &locks);
    let until = Instant::now() + Duration::from_secs(10);
    while holder(&locks, "worker-lead-").is_none_or(|now| Some(&now) == before.as_ref()) {
        assert!(Instant::now() < until, "the wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    let (lock, killed_holder) = taken(&locks, "worker-lead-");
    // Renewed a second at most before it was killed, the lease runs out two
    // seconds after at the soonest: a second in, it is held still.
    let early = kept_at(
        &lock,
        &killed_holder,
        Instant::now() + Duration::from_secs(1),
    );
    killed.kill().unwrap();
    killed.wait().unwrap();
    assert!(early.join().unwrap(), "let go within 1 s of the kill");
    starts_within(&h, Duration::from_secs(10));
    // Renewals that went well are not logged; the locks are.
    let log = h.serve.stderr();
    assert!(!log.contains("/renew"), "{log}");
    assert!(log.contains("/store/lock "), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
}

/// Starts a pickup wait on worker lead, and gives it once it holds its
/// locks, in `locks`, on the far side.
fn picking_up(h: &Http, locks: &std::path::Path) -> std::process::Child {
    let started = spawn(
        h.near
            .command(["wait", "--pickup", "--worker", "lead", "--timeout", "60s"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let until = Instant::now() + Duration::from_secs(10);
    while !held(locks, "worker-lead-") {
        assert!(Instant::now() < until, "the wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    started
}

/// Whether a lock file in `dir` whose name starts with `prefix` names a
/// holder.
fn held(dir: &std::path::Path, prefix: &str) -> bool {
    holder(dir, prefix).is_some()
}

/// What the lock file in `dir` whose name starts with `prefix` says of its
/// holder, if it names one.
fn holder(dir: &std::path::Path, prefix: &str) -> Option<String> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        if !entry.file_name().to_string_lossy().starts_with(prefix) {
            return None;
        }
        let text = fs::read_to_string(entry.path()).ok()?;
        (!text.trim().is_empty()).then_some(text)
    })
}

/// The lock file in `dir` whose name starts with `prefix`, and the line its
/// holder's take wrote in it, once all of it is there: a take writes it in
/// parts. As `tests/forward.rs` has it.
fn taken(dir: &std::path::Path, prefix: &str) -> (std::path::PathBuf, String) {
    let lock = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .unwrap()
        .path();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let holder = fs::read_to_string(&lock).unwrap();
        if holder.ends_with('\n') {
            return (lock, holder);
        }
        assert!(
            Instant::now() < until,
            "its holder is not written: {holder:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether, at `at`, the lock `lock` is held still by the wait whose take
/// wrote `holder` in it, read on a thread of its own so that nothing the
/// test does meanwhile makes it late.
fn kept_at(lock: &std::path::Path, holder: &str, at: Instant) -> thread::JoinHandle<bool> {
    let (lock, holder) = (lock.to_owned(), holder.to_owned());
    thread::spawn(move || {
        thread::sleep(at.saturating_duration_since(Instant::now()));
        kept(&lock, &holder)
    })
}

/// Whether the lock `lock` is held still by the wait whose take wrote
/// `holder` in it: another's take writes its own. When it is free, taking
/// it to see lets it go again at once. A take that has just locked it and
/// not yet written is taken as the holder still, for those few
/// microseconds.
fn kept(lock: &std::path::Path, holder: &str) -> bool {
    let file = fs::File::open(lock).unwrap();
    let busy = matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock));
    busy && fs::read_to_string(lock).unwrap() == holder
}

/// Tries a pickup wait on worker lead every 100 ms, for up to `within`,
/// until one is not refused, and gives when the one that was not started.
fn starts_within(h: &Http, within: Duration) -> Instant {
    let until = Instant::now() + within;
    loop {
        let start = Instant::now();
        let again = h.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
        if !again.stderr.contains("already picking up") {
            return start;
        }
        assert!(Instant::now() < until, "the locks were kept: {again:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

/// Sends `signal` to the process `pid`.
fn kill(signal: &str, pid: u32) {
    let sent = std::process::Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status()
        .unwrap();
    assert!(sent.success(), "kill -{signal} {pid}");
}

/// A proxy of the environment, `HTTP_PROXY` and the like, is not used:
/// the forward goes straight to the host, over the tailnet, and its token
/// never goes to the proxy in the clear.
#[test]
fn a_proxy_of_the_environment_is_not_used() {
    let mut h = Http::new("", &[]);
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", proxy.local_addr().unwrap());
    proxy.set_nonblocking(true).unwrap();
    for var in ["ALL_PROXY", "HTTP_PROXY", "http_proxy"] {
        h.near.set(var, &address);
    }
    h.send_token(TOKEN);
    assert_eq!(h.ok(&["list"]), "No cases.\n");
    assert!(proxy.accept().is_err(), "the proxy was sent a request");
}

/// A stop of serve is not held up by a held poll on the store routes,
/// such as a forwarded wait's: it stops within its grace, as without one.
#[test]
fn a_held_poll_does_not_hold_up_a_stop() {
    let h = Http::new("", &[]);
    let version = h.serve.send(&Request::new("GET", &route("changes")));
    let version = version.json()["version"].as_str().unwrap().to_owned();
    let mut held = common::serve::connect(&h.serve.addr);
    write!(
        held,
        "GET {}?after={version}&timeout=60 HTTP/1.1\r\nHost: {}\r\n\r\n",
        route("changes"),
        h.serve.addr
    )
    .unwrap();
    thread::sleep(Duration::from_millis(500));
    let stopping = Instant::now();
    let Http { serve, .. } = h;
    serve.raise(rustix::process::Signal::TERM);
    let out = serve.exit_within(Duration::from_secs(20));
    assert_eq!(out.code, 0, "{out:?}");
    // Within the grace for its connections, 5 s, and a second for the
    // poll: not the minute it holds for.
    assert!(
        stopping.elapsed() < Duration::from_secs(10),
        "took {:?}",
        stopping.elapsed()
    );
    drop(held);
}
