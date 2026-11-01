//! `cases login status` and `cases logout` (AC-03, SV-01): the `cases`
//! binary against the remote store crate's fake of the hosted server.
//!
//! Logins are kept by hand in the test's credentials file, as `cases
//! login` keeps them, with tokens the fake mints. No test reaches the real
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
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use cases_store_remote::{Server, Signin};
use common::{Env, Output, json, private_dir_all, write_file};
use fake::{ACCOUNT, Fake, Fault, TOKEN};

/// A fake server with the stores `me` (the account's default), `team-x`
/// and `ops`, and a home that points at it with `CASES_SERVER`, with no
/// token of its own.
struct Home {
    fake: Fake,
    env: Env,
    /// Every token minted, to check that none is ever shown.
    secrets: Vec<String>,
}

impl Home {
    fn new() -> Home {
        let fake = Fake::start();
        for name in ["me", "team-x", "ops"] {
            fake.add_store(name);
        }
        let mut env = Env::new();
        env.set("CASES_SERVER", &fake.url());
        Home {
            fake,
            env,
            secrets: vec![TOKEN.to_owned()],
        }
    }

    /// Runs `cases` with `args`, and checks that no output holds a token.
    fn run(&self, args: &[&str]) -> Output {
        let out = self.env.run(args);
        for secret in &self.secrets {
            assert!(!out.stdout.contains(secret.as_str()), "{out:?}");
            assert!(!out.stderr.contains(secret.as_str()), "{out:?}");
        }
        out
    }

    /// A token the fake mints, for every store or for `stores` by name,
    /// through a device sign-in it approves at once.
    fn mint(&mut self, stores: Option<&[&str]>) -> String {
        let server: Server = self.fake.url().parse().unwrap();
        let signin = Signin::new(server);
        let names: Option<Vec<String>> =
            stores.map(|names| names.iter().map(|&name| name.to_owned()).collect());
        let device = signin.device_start("a login", names.as_deref()).unwrap();
        self.fake.approve_device(device.user_code()).unwrap();
        let token = signin
            .device_poll(&device)
            .unwrap()
            .token
            .expose()
            .to_owned();
        self.secrets.push(token.clone());
        token
    }

    /// The credentials file.
    fn credentials(&self) -> PathBuf {
        self.env.path("config/cases/credentials.toml")
    }

    /// Keeps `token` for `server` in the credentials file, with the account
    /// beside it when given, as `cases login` keeps one.
    fn keep_login_for(&self, server: &str, token: &str, account: Option<&str>) {
        let path = self.credentials();
        private_dir_all(path.parent().unwrap());
        let account = account.map_or(String::new(), |id| format!("account = \"{id}\"\n"));
        fs::write(
            &path,
            format!("[\"{server}\"]\n{account}token = \"{token}\"\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// Keeps `token` for the fake, as [`Home::keep_login_for`] does.
    fn keep_login(&self, token: &str, account: Option<&str>) {
        self.keep_login_for(&self.fake.url(), token, account);
    }

    /// Whether the credentials file holds a login for the fake.
    fn logged_in(&self) -> bool {
        fs::read_to_string(self.credentials())
            .is_ok_and(|text| text.contains(&format!("[\"{}\"]", self.fake.url())))
    }

    /// The account's list of stores, as kept in the state directory.
    fn kept_list(&self) -> PathBuf {
        let host = self.fake.url().trim_start_matches("http://").to_owned();
        self.env
            .path("state/cases/accounts")
            .join(format!("{host}.json"))
    }

    /// Whether the fake took `token` and then had it revoked.
    fn revoked(&self, token: &str) -> bool {
        self.fake
            .tokens()
            .iter()
            .find(|t| t.token == token)
            .unwrap()
            .revoked
    }

    /// A private token file at `name` in the test's directory.
    fn token_file(&self, name: &str, token: &str) -> PathBuf {
        let path = self.env.path(name);
        write_file(&path, &format!("{token}\n"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
}

/// An address on `127.0.0.1` where nothing listens.
fn nobody() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[test]
fn status_shows_the_stored_login_and_what_the_server_says() {
    let mut home = Home::new();
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    let url = home.fake.url();
    let credentials = home.credentials();
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "{url}\n  account   ryan ({ACCOUNT})\n  token     \"a login\", for every store, expires \
             2026-12-31\n  stores    me (default), team-x, ops\n  from      {} (cases login)\n  \
             default   me (the account's)\n",
            credentials.display()
        )
    );
    // It always asks the server, which also keeps the account's list.
    assert_eq!(
        home.fake.lines().last().unwrap(),
        "GET /v1/cli/whoami HTTP/1.1"
    );
    assert!(home.kept_list().exists());

    let out = home.run(&["login", "status", "--json", &url]);
    assert_eq!(out.code, 0, "{out:?}");
    let shown = json(&out.stdout);
    assert_eq!(shown["server"], url);
    assert_eq!(
        shown["account"],
        serde_json::json!({"id": ACCOUNT, "handle": "ryan"})
    );
    assert_eq!(
        shown["token"],
        serde_json::json!({"name": "a login", "stores": null, "expires": "2026-12-31T10:00:00Z"})
    );
    assert_eq!(shown["stores"].as_array().unwrap().len(), 3);
    assert_eq!(shown["from"], "credentials");
    assert_eq!(shown["default"]["name"], "me");
    assert_eq!(shown["default"]["from"], "account");
    assert!(shown["default"]["id"].as_str().unwrap().starts_with("st_"));
}

#[test]
fn status_names_each_source_of_the_token_and_the_default_s_rule() {
    let mut home = Home::new();
    let login = home.mint(None);
    home.keep_login(&login, Some(ACCOUNT));
    let credentials = home.credentials();
    let status = |home: &Home| {
        let out = home.run(&["login", "status"]);
        assert_eq!(out.code, 0, "{out:?}");
        let json_out = home.run(&["login", "status", "--json"]);
        assert_eq!(json_out.code, 0, "{json_out:?}");
        (out.stdout, json(&json_out.stdout))
    };

    // CASES_TOKEN wins, and the login it hides is named.
    home.env.set("CASES_TOKEN", TOKEN);
    let (text, shown) = status(&home);
    assert!(
        text.contains("\n  token     \"tests\", for every store,"),
        "{text}"
    );
    assert!(text.contains("\n  from      CASES_TOKEN\n"), "{text}");
    assert!(
        text.ends_with(&format!(
            "\n  unused    the login cases login kept in {}: CASES_TOKEN wins over it\n",
            credentials.display()
        )),
        "{text}"
    );
    assert_eq!(shown["from"], "env");
    home.env.unset("CASES_TOKEN");

    // CASES_TOKEN_FILE, then the config's token-file.
    let file = home.token_file("agent.token", TOKEN);
    home.env.set("CASES_TOKEN_FILE", file.to_str().unwrap());
    let (text, shown) = status(&home);
    assert!(
        text.contains(&format!(
            "\n  from      CASES_TOKEN_FILE {}\n",
            file.display()
        )),
        "{text}"
    );
    assert_eq!(shown["from"], "env-file");
    home.env.unset("CASES_TOKEN_FILE");
    home.env.config(&format!(
        "token-file = \"{}\"\ndefault-store = \"team-x\"\n",
        file.display()
    ));
    let (text, shown) = status(&home);
    assert!(
        text.contains(&format!("\n  from      token-file {}\n", file.display())),
        "{text}"
    );
    assert_eq!(shown["from"], "token-file");
    // The config's default-store wins over the account's default.
    assert!(
        text.contains("\n  default   team-x (the config file's default-store)\n"),
        "{text}"
    );
    assert_eq!(shown["default"]["name"], "team-x");
    assert_eq!(shown["default"]["from"], "default-store");

    // A default-store the token does not open: no default.
    home.env.config("default-store = \"nope\"\n");
    let (text, shown) = status(&home);
    assert!(
        text.contains(
            "\n  default   none: the config file's default-store, nope, is not a store the token \
             opens\n"
        ),
        "{text}"
    );
    assert!(shown["default"].is_null());
    assert!(!text.contains("unused"), "{text}");

    // A token scoped to one store that is not the account's default: that
    // store is the default, as the only one.
    home.env.config("");
    let ops = home.mint(Some(&["ops"]));
    home.keep_login(&ops, Some(ACCOUNT));
    let (text, shown) = status(&home);
    assert!(text.contains(", for ops only, expires "), "{text}");
    assert!(text.contains("\n  stores    ops\n"), "{text}");
    assert!(
        text.contains("\n  default   ops (the only store)\n"),
        "{text}"
    );
    assert_eq!(shown["default"]["from"], "only");
    assert_eq!(shown["token"]["stores"].as_array().unwrap().len(), 1);

    // Scoped to two stores, neither the default: no default.
    let two = home.mint(Some(&["team-x", "ops"]));
    home.keep_login(&two, Some(ACCOUNT));
    let (text, shown) = status(&home);
    assert!(
        text.contains("\n  default   none: name a store with --store\n"),
        "{text}"
    );
    assert!(shown["default"].is_null());
}

#[test]
fn status_exits_1_with_no_token_or_one_the_server_refuses() {
    let mut home = Home::new();
    let url = home.fake.url();
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.starts_with(&format!(
            "Error: not signed in to {url}: run cases login {url}"
        )),
        "{out:?}"
    );
    home.env.unset("CASES_SERVER");
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: name the server: cases login status https://… (or set CASES_SERVER or the \
         config file's server)\n"
    );
    home.env.set("CASES_SERVER", &url);

    // A revoked login: the server's message, where the token came from, and
    // to sign in again.
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    home.fake.revoke(&token);
    let credentials = home.credentials();
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "{url}\n  from      {} (cases login)\n",
            credentials.display()
        )
    );
    assert_eq!(
        out.stderr,
        format!(
            "Error: {url}: the server refused the token cases login kept in {}: the token is \
             revoked. Run cases login {url}\n",
            credentials.display()
        )
    );
    let out = home.run(&["login", "status", "--json"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    // Every other command says the same, and the login is not forgotten.
    let out = home.run(&["--store", "me", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "the server refused the token cases login kept in {}",
            credentials.display()
        )) && out.stderr.contains(&format!("Run cases login {url}")),
        "{out:?}"
    );
    assert!(home.logged_in());

    // An expired one.
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    home.fake.expire(&token);
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .ends_with(&format!(": the token is expired. Run cases login {url}\n")),
        "{out:?}"
    );

    // CASES_TOKEN refused: no login hint, since a login would not change it.
    home.env.set("CASES_TOKEN", &token);
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, format!("{url}\n  from      CASES_TOKEN\n"));
    assert_eq!(
        out.stderr,
        format!(
            "Error: {url}: the server refused the token in CASES_TOKEN: the token is expired\n"
        )
    );
}

#[test]
fn status_exits_1_when_the_server_cannot_be_reached() {
    let mut home = Home::new();
    let token = home.mint(None);
    let dead = nobody();
    home.env.set("CASES_SERVER", &dead);
    home.keep_login_for(&dead, &token, Some(ACCOUNT));
    let out = home.run(&["login", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    // What is known here, then why.
    assert_eq!(
        out.stdout,
        format!(
            "{dead}\n  from      {} (cases login)\n",
            home.credentials().display()
        )
    );
    assert!(
        out.stderr.starts_with(&format!("Error: {dead}: ")),
        "{out:?}"
    );
}

#[test]
fn logout_revokes_the_login_then_forgets_it_and_the_list() {
    let mut home = Home::new();
    let url = home.fake.url();
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    // The kept list, read with the login's token.
    assert_eq!(home.run(&["login", "status"]).code, 0);
    assert!(home.kept_list().exists());
    let before = home.fake.requests().len();
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("Signed out of {url} (ryan).\n"));
    assert_eq!(out.stderr, "");
    assert!(home.revoked(&token));
    assert!(!home.logged_in());
    assert!(!home.kept_list().exists());
    // The account came from the login: one request, the revoke, with the
    // token in its Authorization header alone.
    let sent = &home.fake.requests()[before..];
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].starts_with(&format!("DELETE /v1/accounts/{ACCOUNT}/cli/token ")),
        "{sent:?}"
    );
    for line in sent[0].lines().filter(|line| line.contains(&token)) {
        assert!(
            line.to_ascii_lowercase()
                .starts_with("authorization: bearer "),
            "{line}"
        );
    }

    // Nothing kept now: exit 1, and nothing sent.
    let before = home.fake.requests().len();
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!("Error: not signed in to {url}: there is no stored login\n")
    );
    assert_eq!(home.fake.requests().len(), before);

    // A login with no account beside it, as the keychain keeps one, and no
    // kept list: the account is the one the server gives for it.
    let token = home.mint(None);
    home.keep_login(&token, None);
    let before = home.fake.requests().len();
    let out = home.run(&["logout", &url]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("Signed out of {url} (ryan).\n"));
    assert!(home.revoked(&token));
    assert!(!home.logged_in());
    let lines: Vec<String> = home.fake.lines()[before..].to_vec();
    assert_eq!(
        lines,
        [
            "GET /v1/cli/whoami HTTP/1.1".to_owned(),
            format!("DELETE /v1/accounts/{ACCOUNT}/cli/token HTTP/1.1")
        ]
    );
}

#[test]
fn logout_of_a_token_the_server_already_let_go_forgets_it() {
    let mut home = Home::new();
    let url = home.fake.url();
    let already = format!("Signed out of {url}: the server had already let the token go.\n");
    // Revoked (401), with the account beside it.
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    home.fake.revoke(&token);
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, already);
    assert!(!home.logged_in());

    // Expired (401), with no account beside it: whoami's 401 is enough.
    let token = home.mint(None);
    home.keep_login(&token, None);
    home.fake.expire(&token);
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, already);
    assert!(!home.logged_in());

    // An account the server does not know (404).
    let token = home.mint(None);
    home.keep_login(&token, Some("ac_gone"));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, already);
    assert!(!home.logged_in());
    assert!(!home.revoked(&token));
}

#[test]
fn logout_keeps_the_login_when_the_revoke_fails() {
    let mut home = Home::new();
    let url = home.fake.url();
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    assert_eq!(home.run(&["login", "status"]).code, 0);
    home.fake.fault(Fault::Status(500));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "Error: {url}: the token is not revoked: the server failed (500): the fake failed. \
             Run cases logout again, or cases logout --forget to forget it here only\n"
        )
    );
    assert!(!home.revoked(&token));
    assert!(home.logged_in());
    assert!(home.kept_list().exists());

    // The server down: the same, after the retries.
    let dead = nobody();
    home.env.set("CASES_SERVER", &dead);
    home.keep_login_for(&dead, &token, Some(ACCOUNT));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .starts_with(&format!("Error: {dead}: the token is not revoked: ")),
        "{out:?}"
    );
    assert!(
        fs::read_to_string(home.credentials())
            .unwrap()
            .contains(&dead)
    );

    // Then it goes through.
    home.env.set("CASES_SERVER", &url);
    home.keep_login(&token, Some(ACCOUNT));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(home.revoked(&token));
    assert!(!home.logged_in());
}

#[test]
fn forget_forgets_the_login_and_the_list_with_no_request() {
    let mut home = Home::new();
    let url = home.fake.url();
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    assert_eq!(home.run(&["login", "status"]).code, 0);
    assert!(home.kept_list().exists());
    let before = home.fake.requests().len();
    let out = home.run(&["logout", "--forget"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "Forgot the login for {url}, kept in {}. Its token stays live on the server until it \
             is revoked at {url}/tokens, or goes 90 days unused.\n",
            home.credentials().display()
        )
    );
    assert_eq!(home.fake.requests().len(), before);
    assert!(!home.revoked(&token));
    assert!(!home.logged_in());
    assert!(!home.kept_list().exists());

    // A server that is gone: still no request, and forgotten.
    let dead = nobody();
    home.keep_login_for(&dead, &token, Some(ACCOUNT));
    let out = home.run(&["logout", "--forget", &dead]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        !fs::read_to_string(home.credentials())
            .unwrap()
            .contains(&dead)
    );
}

#[test]
fn logout_never_touches_cases_token_or_a_token_file() {
    let mut home = Home::new();
    let url = home.fake.url();
    // No stored login, and CASES_TOKEN set: it says so, and sends nothing.
    home.env.set("CASES_TOKEN", TOKEN);
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: not signed in to {url}: there is no stored login. Commands use CASES_TOKEN, \
             which logout never touches: revoke it at {url}/tokens\n"
        )
    );
    assert!(home.fake.requests().is_empty());

    // A stored login beside it: the login is revoked, CASES_TOKEN is not.
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        format!("warning: commands use CASES_TOKEN for {url}, which logout leaves alone\n")
    );
    assert!(home.revoked(&token));
    assert!(!home.revoked(TOKEN));
    home.env.unset("CASES_TOKEN");

    // A token file in the config: left, file and token.
    let file = home.token_file("agent.token", TOKEN);
    home.env
        .config(&format!("token-file = \"{}\"\n", file.display()));
    let token = home.mint(None);
    home.keep_login(&token, Some(ACCOUNT));
    let out = home.run(&["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "warning: commands use the token file {} (the config file's token-file) for {url}, \
             which logout leaves alone\n",
            file.display()
        )
    );
    assert!(file.exists());
    assert!(!home.revoked(TOKEN));
    // It still works.
    assert_eq!(home.run(&["login", "status"]).code, 0);
}

#[test]
fn logout_file_revokes_the_token_in_it_then_removes_it() {
    let mut home = Home::new();
    let url = home.fake.url();
    let login = home.mint(None);
    home.keep_login(&login, Some(ACCOUNT));
    let agent = home.mint(Some(&["ops"]));
    let file = home.token_file("agent.token", &agent);
    let out = home.run(&["logout", "--file", file.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "Revoked the token in {} for ryan on {url}, and removed the file.\n",
            file.display()
        )
    );
    assert!(home.revoked(&agent));
    assert!(!file.exists());
    // The stored login is left alone.
    assert!(home.logged_in());
    assert!(!home.revoked(&login));

    // A file whose token the server already let go: removed.
    let agent = home.mint(None);
    let file = home.token_file("old.token", &agent);
    home.fake.revoke(&agent);
    let out = home.run(&["logout", "--file", file.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "Removed {}: {url} had already let its token go.\n",
            file.display()
        )
    );
    assert!(!file.exists());

    // The revoke fails: the file stays.
    let agent = home.mint(None);
    let file = home.token_file("kept.token", &agent);
    home.fake.fault(Fault::Status(500));
    let out = home.run(&["logout", "--file", file.to_str().unwrap()]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "the token in {} is not revoked: the server failed (500): the fake failed. The file \
             is kept",
            file.display()
        )),
        "{out:?}"
    );
    assert!(file.exists());
    assert!(!home.revoked(&agent));

    // With --forget: removed, with no request.
    let before = home.fake.requests().len();
    let out = home.run(&["logout", "--file", file.to_str().unwrap(), "--forget"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout
            .starts_with(&format!("Removed {}. ", file.display())),
        "{out:?}"
    );
    assert!(!file.exists());
    assert_eq!(home.fake.requests().len(), before);
    assert!(!home.revoked(&agent));
    assert!(home.logged_in());
}

#[test]
fn logout_file_refuses_what_is_not_a_token_file() {
    let mut home = Home::new();
    let notes = home.env.path("notes.txt");
    write_file(&notes, "not a token\n");
    let token = home.mint(None);
    let real = home.token_file("real.token", &token);
    let link = home.env.path("link.token");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let dir = home.env.path("home");
    let before = home.fake.requests().len();
    fs::create_dir_all(&dir).unwrap();
    for (path, says) in [
        (&notes, "does not hold a token"),
        (&link, "is a symbolic link"),
        (&dir, "is not a regular file"),
        (&home.env.path("none.token"), "is not there"),
    ] {
        for forget in [false, true] {
            let mut args = vec!["logout", "--file", path.to_str().unwrap()];
            if forget {
                args.push("--forget");
            }
            let out = home.run(&args);
            assert_eq!(out.code, 1, "{out:?}");
            assert!(out.stderr.contains(says), "{says}: {out:?}");
        }
    }
    assert!(notes.exists() && link.exists() && real.exists() && dir.exists());
    assert_eq!(home.fake.requests().len(), before);
    assert!(!home.revoked(&token));

    // The token file commands use: removed, and it says the config still
    // names it.
    home.env.set("CASES_TOKEN_FILE", real.to_str().unwrap());
    let out = home.run(&["logout", "--file", real.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: CASES_TOKEN_FILE still names {}, so commands cannot read their token",
            real.display()
        )),
        "{out:?}"
    );
    assert!(home.revoked(&token));
}

#[test]
fn login_status_and_a_server_parse_beside_each_other() {
    let home = Home::new();
    // A server that is not one, where status would be: refused as not an
    // address, so `status` can never be taken for a server.
    let out = home.run(&["login", "--device", "status"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("\"status\" is not a server's address"),
        "{out:?}"
    );
    // Login's own flags do not go with status.
    let out = home.run(&["login", "status", "--device"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("unexpected argument '--device'"),
        "{out:?}"
    );
    let out = home.run(&["login", "--help"]);
    assert!(out.stdout.contains("status"), "{out:?}");
}
