//! `cases login` (AC-03, AC-05, SV-01): the `cases` binary against the
//! remote store crate's fake of the hosted server, which plays the
//! approval page, the `/device` page and the person behind them.
//!
//! The test is the browser: it reads the approval address from the
//! stand-in opener, or from stderr, has the fake approve it, and GETs the
//! address the fake sends it back to on the CLI's loopback port. No test
//! opens a real browser: `$PATH` holds only a stand-in `open` and
//! `xdg-open`, which write down the address. No test reaches the real
//! keychain: `tests/common` sets `CASES_NO_KEYCHAIN` for every run.
#![cfg(feature = "remote")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
// The remote store crate's fake server, shared by path as `remote.rs`
// shares it.
#[path = "../../store-remote/tests/fake/mod.rs"]
mod fake;

use std::fs;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cases_store_remote::{Server, Signin};
use common::{Env, Output, spawn, write_file};
use fake::{ACCOUNT, Fake, Fault, TOKEN};

/// A fake server with the stores `me` (the account's default), `team-x`
/// and `ops`, and a home that points at it with `CASES_SERVER`, with no
/// token of its own, and a stand-in browser opener first on `$PATH`.
struct Home {
    fake: Fake,
    env: Env,
    /// Where the stand-in opener writes each address it is given.
    opened: PathBuf,
}

impl Home {
    fn new() -> Home {
        let fake = Fake::start();
        for name in ["me", "team-x", "ops"] {
            fake.add_store(name);
        }
        let mut env = Env::new();
        env.set("CASES_SERVER", &fake.url());
        let bin = env.path("bin");
        let opened = env.path("opened");
        for name in ["open", "xdg-open"] {
            let path = bin.join(name);
            write_file(
                &path,
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\n",
                    opened.display()
                ),
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        env.set("PATH", bin.to_str().unwrap());
        Home { fake, env, opened }
    }

    /// Starts `cases` with `args`, its stderr read as it comes.
    fn start(&self, args: &[&str]) -> Running {
        let mut child = spawn(
            self.env
                .command(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let stderr = child.stderr.take().unwrap();
        let (send, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut all = String::new();
            for line in BufReader::new(stderr).lines() {
                let line = line.unwrap();
                all.push_str(&line);
                all.push('\n');
                let _ = send.send(line);
            }
            all
        });
        Running {
            child,
            lines,
            reader,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.env.run(args)
    }

    /// The addresses the stand-in opener was given.
    fn opened(&self) -> Vec<String> {
        fs::read_to_string(&self.opened)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The address the stand-in opener was given, once it has run.
    fn opened_url(&self) -> String {
        assert!(
            soon(|| self.opened().len() == 1),
            "the browser was not opened"
        );
        self.opened().remove(0)
    }

    /// The credentials file.
    fn credentials(&self) -> PathBuf {
        self.env.path("config/cases/credentials.toml")
    }

    /// The account's list of stores, as kept in the state directory.
    fn kept_list(&self) -> PathBuf {
        let host = self.fake.url().trim_start_matches("http://").to_owned();
        self.env
            .path("state/cases/accounts")
            .join(format!("{host}.json"))
    }

    /// How many times `GET /v1/cli/whoami` was asked.
    fn whoamis(&self) -> usize {
        self.fake
            .lines()
            .iter()
            .filter(|line| line.starts_with("GET /v1/cli/whoami "))
            .count()
    }

    /// The tokens the fake minted, in turn, after the fixed one.
    fn minted(&self) -> Vec<fake::Minted> {
        self.fake.tokens().into_iter().skip(1).collect()
    }

    /// The user code of the one device sign-in that waits, once it waits.
    fn user_code(&self) -> String {
        assert!(
            soon(|| self.fake.user_codes().len() == 1),
            "no device sign-in waits"
        );
        self.fake.user_codes().remove(0)
    }

    /// Runs a loopback sign-in with `args` after `login` to its end, as
    /// the person who approves it, and gives what it did and the page the
    /// browser was shown.
    fn log_in(&self, args: &[&str]) -> (Output, String) {
        let mut all = vec!["login"];
        all.extend(args);
        let running = self.start(&all);
        let url = running.url();
        let page = browse(&self.fake.approve(&url).unwrap());
        (running.finish(), page)
    }
}

/// A `cases` that runs on, with its stderr read as it comes.
struct Running {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: JoinHandle<String>,
}

impl Running {
    /// The approval address printed on stderr, once it is.
    fn url(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .expect("an approval address on stderr");
            if let Some(url) = line.trim().strip_prefix("http")
                && url.contains("/cli/authorize?")
            {
                return format!("http{url}");
            }
        }
    }

    /// Waits for the end, and gives what it did.
    fn finish(self) -> Output {
        let out = self.child.wait_with_output().unwrap();
        Output {
            code: out.status.code().unwrap(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: self.reader.join().unwrap(),
        }
    }
}

/// Whether `done` holds within 10 s.
fn soon(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

/// The browser's trip to `url`, an `http://127.0.0.1:<port>/…` address:
/// the whole answer, head and page.
fn browse(url: &str) -> String {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, target) = rest.split_at(rest.find('/').unwrap());
    let mut stream = TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {host}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut page = String::new();
    stream.read_to_string(&mut page).unwrap();
    page
}

/// The value of `name` in the query of `url`, decoded.
fn param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?').unwrap().1;
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The mode of the file at `path`.
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// A token the fake mints through a device sign-in it approves at once.
fn mint(fake: &Fake) -> String {
    let server: Server = fake.url().parse().unwrap();
    let signin = Signin::new(server);
    let device = signin.device_start("an old login", None).unwrap();
    fake.approve_device(device.user_code()).unwrap();
    signin
        .device_poll(&device)
        .unwrap()
        .token
        .expose()
        .to_owned()
}

/// Keeps `token` for the fake in `home`'s credentials file, with the
/// account beside it when given, as `cases login` keeps one.
fn keep_login(home: &Home, token: &str, account: Option<&str>) {
    let path = home.credentials();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let account = account.map_or(String::new(), |id| format!("account = \"{id}\"\n"));
    let entry = format!("[\"{}\"]\n{account}token = \"{token}\"\n", home.fake.url());
    fs::write(&path, entry).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// Checks that no output holds a secret of the sign-in: the token, the
/// code or the verifier, which only the exchange's body may hold, and that
/// no request carries the token but in its `Authorization` header.
fn no_secret_shown(home: &Home, out: &Output, code: &str) {
    let token = home.minted().last().unwrap().token.clone();
    let mut verifier = None;
    for request in home.fake.requests() {
        if request.starts_with("POST /v1/cli/token ") {
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            verifier = body["code_verifier"].as_str().map(str::to_owned);
            continue;
        }
        for line in request.lines().filter(|line| line.contains(&token)) {
            assert!(
                line.to_ascii_lowercase()
                    .starts_with("authorization: bearer "),
                "{request}"
            );
        }
    }
    let verifier = verifier.expect("an exchange");
    assert_eq!(verifier.len(), 43);
    for text in [&out.stdout, &out.stderr] {
        assert!(!text.contains(&token), "{out:?}");
        assert!(!text.contains(code), "{out:?}");
        assert!(!text.contains(&verifier), "{out:?}");
    }
    assert!(
        home.fake
            .requests()
            .iter()
            .filter(|r| r.contains(&verifier))
            .all(|r| r.starts_with("POST /v1/cli/token ")),
    );
}

#[test]
fn login_signs_in_in_the_browser_and_keeps_the_token_and_the_list() {
    let home = Home::new();
    let running = home.start(&["login"]);
    // The stand-in browser was given the approval page, which the CLI also
    // printed.
    let url = home.opened_url();
    assert_eq!(running.url(), url);
    assert!(
        url.starts_with(&format!("{}/cli/authorize?", home.fake.url())),
        "{url}"
    );
    assert_eq!(param(&url, "code_challenge_method").unwrap(), "S256");
    assert!(
        param(&url, "name").unwrap().starts_with("cases CLI on "),
        "{url}"
    );
    assert_eq!(param(&url, "stores"), None);
    let redirect = param(&url, "redirect_uri").unwrap();
    let port = redirect
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix("/callback"))
        .unwrap();
    assert!(port.parse::<u16>().unwrap() > 0, "{redirect}");

    let back = home.fake.approve(&url).unwrap();
    let code = param(&back, "code").unwrap();
    let page = browse(&back);
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    // The browser's request was held until the token was checked and kept.
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    assert!(
        page.contains("Signed in as ryan. You can close this tab."),
        "{page}"
    );
    assert!(
        page.contains("Content-Security-Policy: default-src 'none'\r\n"),
        "{page}"
    );
    assert!(!page.contains("<script"), "{page}");

    let minted = home.minted();
    assert_eq!(minted.len(), 1);
    let token = &minted[0];
    assert!(token.name.starts_with("cases CLI on "), "{token:?}");
    assert_eq!(token.stores, None);
    assert!(!token.revoked);
    let credentials = home.credentials();
    assert_eq!(
        out.stdout,
        format!(
            "Signed in to {} as ryan ({ACCOUNT}).\nToken {:?}, for every store, kept in {}.\n\
             Stores: me (default), team-x, ops.\n",
            home.fake.url(),
            token.name,
            credentials.display()
        )
    );
    if cfg!(target_os = "macos") {
        assert!(
            out.stderr
                .contains("CASES_NO_KEYCHAIN is set, so the token for"),
            "{out:?}"
        );
    }
    // Kept in the credentials file, which only its owner reads, with the
    // account beside it.
    let kept = fs::read_to_string(&credentials).unwrap();
    assert!(kept.contains(&token.token), "{kept}");
    assert!(kept.contains(&format!("account = \"{ACCOUNT}\"")), "{kept}");
    assert_eq!(mode(&credentials), 0o600);
    // The list of stores is kept from the check's answer.
    let list: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.kept_list()).unwrap()).unwrap();
    assert_eq!(list["account"]["id"], ACCOUNT);
    assert_eq!(list["stores"].as_array().unwrap().len(), 3);
    assert_eq!(list["stores"][0]["default"], true);
    // Login writes no config.
    assert!(!home.env.path("config/cases/config.toml").exists());
    no_secret_shown(&home, &out, &code);

    // The next command uses the token, and the kept list: no whoami.
    let whoamis = home.whoamis();
    let opened = home.run(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "T",
    ]);
    assert_eq!(opened.code, 0, "{opened:?}");
    assert_eq!(home.whoamis(), whoamis);
    let last = home.fake.requests().pop().unwrap();
    assert!(
        last.to_ascii_lowercase().contains(&format!(
            "authorization: bearer {}",
            token.token.to_ascii_lowercase()
        )),
        "{last}"
    );
}

#[test]
fn no_browser_prints_the_address_and_opens_nothing() {
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    let page = browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(page.contains("Signed in as ryan."), "{page}");
    assert!(home.opened().is_empty());
    assert!(
        out.stderr.contains(&format!(
            "Open this page to approve the sign-in:\n  {url}\n"
        )),
        "{out:?}"
    );
    // With no opener on $PATH the address is printed the same way.
    let mut home = Home::new();
    home.env.unset("PATH");
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains("Open this page to approve the sign-in:"),
        "{out:?}"
    );
}

#[test]
fn a_callback_with_another_state_is_refused_and_the_sign_in_goes_on() {
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    let back = home.fake.approve(&url).unwrap();
    let state = param(&back, "state").unwrap();
    let other = format!(
        "{}{}",
        if state.starts_with('A') { "B" } else { "A" },
        &state[1..]
    );
    for _ in 0..2 {
        let stray = browse(&back.replace(&state, &other));
        assert!(stray.starts_with("HTTP/1.1 400 "), "{stray}");
        assert!(
            stray.contains("This is not the sign-in the terminal started."),
            "{stray}"
        );
    }
    // The stray's code is not used: nothing was exchanged.
    assert!(
        !home
            .fake
            .lines()
            .iter()
            .any(|line| line.starts_with("POST /v1/cli/token "))
    );
    let page = browse(&back);
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(page.contains("Signed in as ryan."), "{page}");
    assert_eq!(
        out.stderr.matches("with another sign-in's state").count(),
        1,
        "{out:?}"
    );
}

#[test]
fn a_denied_sign_in_exits_1_and_keeps_nothing() {
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    let page = browse(&home.fake.deny(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        page.contains("The sign-in was denied. You can close this tab."),
        "{page}"
    );
    assert!(
        out.stderr
            .contains("Error: the sign-in was denied (access_denied)"),
        "{out:?}"
    );
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(home.minted().is_empty());
    assert!(!home.credentials().exists());
    assert!(!home.kept_list().exists());
}

#[test]
fn a_failed_check_keeps_nothing_and_revokes_the_new_token() {
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    // The exchange goes through; whoami names the token otherwise.
    home.fake.fault(Fault::Slow(Duration::ZERO));
    home.fake.fault(Fault::Body(
        serde_json::json!({
            "account": {"id": ACCOUNT, "handle": "ryan"},
            "token": {"name": "another", "stores": null, "expires": null},
            "stores": [],
        })
        .to_string(),
    ));
    let page = browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        page.contains("The sign-in failed: see the terminal."),
        "{page}"
    );
    assert!(
        out.stderr
            .contains("the new token failed its check: the token is named \"another\""),
        "{out:?}"
    );
    assert!(
        out.stderr
            .contains("Nothing was kept, and the new token is revoked."),
        "{out:?}"
    );
    assert!(home.minted()[0].revoked);
    assert!(!home.credentials().exists());
    assert!(!home.kept_list().exists());

    // A revoke that fails says so, and names the token page.
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    home.fake.fault(Fault::Slow(Duration::ZERO));
    home.fake.fault(Fault::Revoked);
    home.fake.fault(Fault::Status(500));
    browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .contains("the new token failed its check: the server refused the token: revoked"),
        "{out:?}"
    );
    assert!(
        out.stderr.contains(&format!(
            "Nothing was kept, but the new token could not be revoked (the server failed (500): \
             the fake failed): if {}/tokens lists it, revoke it there.",
            home.fake.url()
        )),
        "{out:?}"
    );
    assert!(!home.minted()[0].revoked);
    assert!(!home.credentials().exists());
}

#[test]
fn an_exchange_rides_out_a_503_and_a_lost_answer_names_the_token_page() {
    // A store could not take the token: the code stays usable, and the
    // exchange is sent again.
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    home.fake.fault(Fault::Unavailable);
    let page = browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(page.contains("Signed in as ryan."), "{page}");

    // The token was made and its answer lost; the code is spent by then.
    let home = Home::new();
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    home.fake.fault(Fault::Lost);
    let page = browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 1, "{out:?}");
    assert!(page.contains("The sign-in failed"), "{page}");
    assert_eq!(
        out.stderr.lines().last().unwrap(),
        format!(
            "Error: the sign-in may have made a token that never arrived: revoke any you don't \
             recognise at {}/tokens, then sign in again",
            home.fake.url()
        )
    );
    assert!(!home.credentials().exists());
}

#[test]
fn signing_in_again_revokes_the_token_it_replaces() {
    let home = Home::new();
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    let minted = home.minted();
    assert_eq!(minted.len(), 2);
    assert!(minted[0].revoked, "{minted:?}");
    assert!(!minted[1].revoked, "{minted:?}");
    let kept = fs::read_to_string(home.credentials()).unwrap();
    assert!(kept.contains(&minted[1].token) && !kept.contains(&minted[0].token));
    assert!(!out.stderr.contains("not revoked"), "{out:?}");

    // An old login with no account beside it, as the keychain keeps one,
    // is revoked in the account the server gives for it; a revoke that
    // fails is warned about, and the new login is kept all the same.
    let home = Home::new();
    let old = mint(&home.fake);
    keep_login(&home, &old, None);
    let running = home.start(&["login", "--no-browser"]);
    let url = running.url();
    // The exchange, the check, the old token's whoami, then its revoke.
    for fault in [
        Fault::Slow(Duration::ZERO),
        Fault::Slow(Duration::ZERO),
        Fault::Slow(Duration::ZERO),
        Fault::Status(500),
    ] {
        home.fake.fault(fault);
    }
    browse(&home.fake.approve(&url).unwrap());
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: the token this login replaced is not revoked (the server failed (500): the \
             fake failed): revoke the old token at {}/tokens",
            home.fake.url()
        )),
        "{out:?}"
    );
    let minted = home.minted();
    assert!(!minted[0].revoked);
    let kept = fs::read_to_string(home.credentials()).unwrap();
    assert!(kept.contains(&minted[1].token) && !kept.contains(&old));
    let deletes: Vec<String> = home
        .fake
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("DELETE "))
        .collect();
    assert_eq!(
        deletes,
        [format!("DELETE /v1/accounts/{ACCOUNT}/cli/token HTTP/1.1")]
    );
}

#[test]
fn device_signs_in_with_a_code_typed_elsewhere() {
    let home = Home::new();
    let running = home.start(&["login", "--device"]);
    let code = home.user_code();
    home.fake.approve_device(&code).unwrap();
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr.lines().take(6).collect::<Vec<_>>(),
        [
            format!(
                "Open {}/device in a browser signed in to your account,",
                home.fake.url()
            ),
            "and type the code:".to_owned(),
            String::new(),
            format!("    {code}"),
            String::new(),
            "The code lasts 10 minutes. Type it only if you started this sign-in.".to_owned(),
        ]
    );
    assert!(out.stdout.starts_with("Signed in to "), "{out:?}");
    // The browser is never opened, and the code is in no address.
    assert!(home.opened().is_empty());
    assert!(!out.stderr.contains('?'), "{out:?}");
    // The device code is a secret too.
    assert!(!out.stderr.contains("ac_fake.device") && !out.stdout.contains("ac_fake.device"));
    let token = &home.minted()[0].token;
    assert!(!out.stderr.contains(token) && !out.stdout.contains(token));
    assert!(
        fs::read_to_string(home.credentials())
            .unwrap()
            .contains(token)
    );
    assert!(home.kept_list().exists());
}

#[test]
fn a_device_sign_in_denied_or_run_out_exits_1() {
    for (end, want) in [
        (
            Fake::deny_device as fn(&Fake, &str) -> Result<(), String>,
            "Error: the sign-in was denied\n",
        ),
        (
            Fake::expire_device,
            "Error: the code ran out before it was approved: run cases login --device again\n",
        ),
    ] {
        let home = Home::new();
        let running = home.start(&["login", "--device"]);
        end(&home.fake, &home.user_code()).unwrap();
        let out = running.finish();
        assert_eq!(out.code, 1, "{out:?}");
        assert!(out.stderr.ends_with(want), "{out:?}");
        assert!(home.minted().is_empty());
        assert!(!home.credentials().exists());
    }
}

#[test]
fn slow_down_widens_the_device_s_interval() {
    let home = Home::new();
    let started = Instant::now();
    let running = home.start(&["login", "--device"]);
    let code = home.user_code();
    // The first poll is told to slow down; the next comes 5 s later.
    home.fake.fault(Fault::Refuse(400, "slow_down"));
    home.fake.approve_device(&code).unwrap();
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    let polls = home
        .fake
        .lines()
        .iter()
        .filter(|line| line.starts_with("POST /v1/cli/device/token "))
        .count();
    assert_eq!(polls, 2);
    assert!(
        started.elapsed() >= Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn file_writes_a_second_token_and_leaves_the_login_alone() {
    let home = Home::new();
    let old = mint(&home.fake);
    keep_login(&home, &old, Some(ACCOUNT));
    let credentials = fs::read_to_string(home.credentials()).unwrap();
    let path = home.env.path("agent.token");
    let path_text = path.to_str().unwrap();
    let running = home.start(&[
        "--store", "team-x", "login", "--device", "--file", path_text,
    ]);
    home.fake.approve_device(&home.user_code()).unwrap();
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    let minted = home.minted();
    let token = minted.last().unwrap();
    assert!(token.name.starts_with("cases CLI on "), "{token:?}");
    assert!(token.name.ends_with(", agent.token"), "{token:?}");
    assert_eq!(token.stores.as_ref().unwrap().len(), 1);
    assert_eq!(
        out.stdout,
        format!(
            "Wrote a token for ryan ({ACCOUNT}) to {path_text}.\nToken {:?}, for team-x only.\n\
             Use it with token-file = {path_text:?}, or CASES_TOKEN_FILE.\n",
            token.name
        )
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{}\n", token.token)
    );
    assert_eq!(mode(&path), 0o600);
    assert!(!out.stderr.contains(&token.token) && !out.stdout.contains(&token.token));
    // The login, its kept list and the old token are as they were.
    assert_eq!(fs::read_to_string(home.credentials()).unwrap(), credentials);
    assert!(!home.kept_list().exists());
    assert!(home.fake.tokens().iter().all(|t| !t.revoked));
    assert!(!out.stderr.contains("rather than this login"), "{out:?}");
    // The file opens team-x, and no other store.
    let mut agent = Env::new();
    agent.set("CASES_SERVER", &home.fake.url());
    agent.set("CASES_TOKEN_FILE", path_text);
    let listed = agent.run(["--store", "team-x", "list"]);
    assert_eq!(listed.code, 0, "{listed:?}");
    let refused = agent.run(["--store", "me", "list"]);
    assert_eq!(refused.code, 1, "{refused:?}");

    // A file already there is refused before anything is sent.
    let requests = home.fake.requests().len();
    let out = home.run(&["login", "--device", "--file", path_text]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!("Error: {path_text} is already there: add --force to replace it\n")
    );
    assert_eq!(home.fake.requests().len(), requests);
    // So is a symbolic link, even forced, and a directory that is not there.
    let link = home.env.path("link.token");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let out = home.run(&[
        "login",
        "--device",
        "--force",
        "--file",
        link.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("is a symbolic link"), "{out:?}");
    let out = home.run(&[
        "login",
        "--device",
        "--file",
        home.env.path("none/agent.token").to_str().unwrap(),
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.ends_with("is not there: make it first\n"),
        "{out:?}"
    );
    assert_eq!(home.fake.requests().len(), requests);
    // --force replaces it, and still leaves the login alone.
    let running = home.start(&["login", "--device", "--force", "--file", path_text]);
    home.fake.approve_device(&home.user_code()).unwrap();
    let out = running.finish();
    assert_eq!(out.code, 0, "{out:?}");
    let replaced = home.minted().last().unwrap().token.clone();
    assert_ne!(replaced, token.token);
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{replaced}\n"));
    assert_eq!(mode(&path), 0o600);
    assert!(out.stdout.contains(", for every store."), "{out:?}");
    assert_eq!(fs::read_to_string(home.credentials()).unwrap(), credentials);
    assert!(home.fake.tokens().iter().all(|t| !t.revoked));
    // --force needs --file.
    let out = home.run(&["login", "--force"]);
    assert_eq!(out.code, 1, "{out:?}");
}

#[test]
fn the_scope_comes_from_the_store_flag_alone() {
    let mut home = Home::new();
    home.env.set("CASES_STORE", "ops");
    // The config's store does not count either.
    let config = home.env.config("store = \"ops\"\n");
    assert!(config.exists());
    let (out, _) = home.log_in(&["--name", "agent one"]);
    assert_eq!(out.code, 0, "{out:?}");
    let url = home.opened_url();
    assert_eq!(param(&url, "stores"), None);
    assert_eq!(param(&url, "name").unwrap(), "agent one");
    assert!(out.stdout.contains("for every store"), "{out:?}");

    let home = Home::new();
    let (out, _) = home.log_in(&["--store", "team-x,ops"]);
    assert_eq!(out.code, 0, "{out:?}");
    let url = home.opened_url();
    assert_eq!(param(&url, "stores").unwrap(), "team-x,ops");
    assert!(
        out.stdout.contains(", for team-x, ops only, kept in"),
        "{out:?}"
    );
    assert!(out.stdout.contains("Stores: team-x, ops.\n"), "{out:?}");
    assert_eq!(home.minted()[0].stores.as_ref().unwrap().len(), 2);

    // An id, a path or a URL is refused, before anything is sent.
    for bad in [
        "st_0000000000000002",
        "./cases.db",
        "https://x.example/team-x",
    ] {
        let out = home.run(&["--store", bad, "login"]);
        assert_eq!(out.code, 1, "{bad}: {out:?}");
        assert!(
            out.stderr
                .starts_with("Error: --store for login takes store names"),
            "{bad}: {out:?}"
        );
    }
    let out = home.run(&["login", "--name", "a\tb"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(home.minted().len(), 1);
}

#[test]
fn the_server_must_be_named_and_login_writes_no_config() {
    let mut home = Home::new();
    home.env.unset("CASES_SERVER");
    let out = home.run(&["login"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: name the server: cases login https://… (or set CASES_SERVER or the config \
         file's server)\n"
    );
    let out = home.run(&["login", "http://example.com"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("plain http is only for"), "{out:?}");

    // Named, with no server set: it says how to set it.
    let server = home.fake.url();
    let (out, _) = home.log_in(&[&server]);
    assert_eq!(out.code, 0, "{out:?}");
    let config = home.env.path("config/cases/config.toml");
    assert!(
        out.stderr.contains(&format!(
            "warning: commands use the local store until a server is set: add server = \
             \"{server}\" to {}, or set CASES_SERVER\n",
            config.display()
        )),
        "{out:?}"
    );
    assert!(!config.exists());

    // Another server set: it says which, and the line.
    home.env.set("CASES_SERVER", "https://other.example");
    let (out, _) = home.log_in(&[&server]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: commands use https://other.example, from CASES_SERVER, not this server: \
             set server = \"{server}\" in {}, or CASES_SERVER, to use it\n",
            config.display()
        )),
        "{out:?}"
    );
    assert!(!config.exists());

    // The config's server is used with none named, and nothing is said.
    let mut home = Home::new();
    home.env.unset("CASES_SERVER");
    home.env
        .config(&format!("server = \"{}\"\n", home.fake.url()));
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!out.stderr.contains("commands use"), "{out:?}");
}

#[test]
fn a_token_that_wins_over_the_new_login_is_named() {
    let mut home = Home::new();
    home.env.set("CASES_TOKEN", TOKEN);
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains("warning: CASES_TOKEN is set, and commands use it rather than this login\n"),
        "{out:?}"
    );

    let mut home = Home::new();
    let file = home.env.path("agent.token");
    write_file(&file, &format!("{TOKEN}\n"));
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    home.env.set("CASES_TOKEN_FILE", file.to_str().unwrap());
    let (out, _) = home.log_in(&[]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: the token file {} is set (CASES_TOKEN_FILE), and commands use it rather \
             than this login\n",
            file.display()
        )),
        "{out:?}"
    );
}
