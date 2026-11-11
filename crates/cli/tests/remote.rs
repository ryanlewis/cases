//! The CLI on a remote store (ST-11, SV-01, AC-04, HU-06, AG-13): the
//! `cases` binary against the remote store crate's fake of the hosted
//! server, which runs in this process on a loopback port.
#![cfg(feature = "remote")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
// The remote store crate's fake server, shared by path as that crate's own
// tests share it: everything it uses is already in this crate's tree.
#[path = "../../store-remote/tests/fake/mod.rs"]
mod fake;

use std::fs;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Store as _, Time};
use cases_store_remote::{Server, Signin};
use common::{Env, Output, json, private_dir_all, spawn};
use fake::{Fake, Fault, TOKEN};

/// A fake server with the stores `names`, the first the account's
/// default, and a home that points at it with `CASES_SERVER` and
/// `CASES_TOKEN`.
struct Remote {
    fake: Fake,
    env: Env,
    ids: Vec<String>,
}

impl Remote {
    fn new(names: &[&str]) -> Remote {
        let fake = Fake::start();
        let ids = names.iter().map(|name| fake.add_store(name)).collect();
        let mut env = Env::new();
        env.set("CASES_SERVER", &fake.url());
        env.set("CASES_TOKEN", TOKEN);
        // A login name, which a remote store must not record (HU-06).
        env.set("USER", "loginname");
        Remote { fake, env, ids }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.env.run(args)
    }

    /// Runs `args`, which must succeed, and gives stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.code, 0, "{args:?}: {out:?}");
        out.stdout
    }

    /// Opens an fyi case on the store `store`, or the default with none,
    /// and gives its id.
    fn open(&self, store: Option<&str>, title: &str) -> String {
        let mut args = Vec::new();
        if let Some(store) = store {
            args.extend(["--store", store]);
        }
        args.extend([
            "open",
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            title,
        ]);
        self.ok(&args).trim().to_owned()
    }

    /// How many live cases the store `id` holds, read on the fake.
    fn cases(&self, id: &str) -> usize {
        self.fake.db(id).list().map_or(0, |list| list.len())
    }

    /// How many times the account's list was read.
    fn whoamis(&self) -> usize {
        self.fake
            .lines()
            .iter()
            .filter(|line| line.starts_with("GET /v1/cli/whoami "))
            .count()
    }

    /// The account's list as kept in the state directory.
    fn kept(&self) -> String {
        let host = self.fake.url().trim_start_matches("http://").to_owned();
        let path = self
            .env
            .path("state")
            .join("cases/accounts")
            .join(format!("{host}.json"));
        fs::read_to_string(path).unwrap()
    }
}

#[test]
fn a_name_is_found_in_the_account_s_list_which_is_kept() {
    let r = Remote::new(&["me", "team"]);
    r.open(Some("team"), "Into team");
    assert_eq!((r.cases(&r.ids[0]), r.cases(&r.ids[1])), (0, 1));
    assert_eq!(r.whoamis(), 1);
    // The list is kept, with the token's SHA-256 and never the token.
    let kept = json(&r.kept());
    assert_eq!(kept["account"]["handle"], "ryan");
    assert_eq!(kept["stores"][1]["name"], "team");
    assert_eq!(kept["token_sha256"].as_str().unwrap().len(), 64);
    assert!(!r.kept().contains(TOKEN));
    // The next command reads it from the state directory.
    let out = r.ok(&["--store", "team", "list", "--json"]);
    assert_eq!(json(&out)["list"].as_array().unwrap().len(), 1);
    assert_eq!(r.whoamis(), 1);

    // A name the kept list does not have reads the list again, once.
    let new = r.fake.add_store("new");
    r.open(Some("new"), "Into new");
    assert_eq!((r.cases(&new), r.whoamis()), (1, 2));
    let out = r.run(&["--store", "nope", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: no store named nope in account ryan on {}\n",
            r.fake.url()
        )
    );
    assert_eq!(r.whoamis(), 3);

    // Another token reads the list again: the kept one is not its.
    let mut other = Remote::new(&["me"]);
    other
        .env
        .set("XDG_STATE_HOME", r.env.path("state").to_str().unwrap());
    other.env.set("CASES_SERVER", &r.fake.url());
    other
        .env
        .set("CASES_TOKEN", &format!("csu_{}", "a".repeat(36)));
    let out = other.run(&["--store", "me", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("refused the token"), "{out:?}");
    assert_eq!(r.whoamis(), 4);
}

#[test]
fn an_id_is_never_looked_up() {
    let r = Remote::new(&["me", "team"]);
    let team = r.ids[1].clone();
    r.open(Some(&team), "By id");
    assert_eq!(r.cases(&team), 1);
    assert_eq!(r.whoamis(), 0);
    // A 401 on a store given by id fails at once, and says the token is
    // not for that store.
    r.fake.fault(Fault::Revoked);
    let out = r.run(&["--store", &team, "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: the server refused the token in CASES_TOKEN for the store {team} on {}: \
             revoked\n",
            r.fake.url()
        )
    );
    assert_eq!(r.whoamis(), 0);
    // An id the account does not have is the server's to refuse.
    let out = r.run(&["--store", "st_9999999999999999", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("no store st_9999999999999999"),
        "{out:?}"
    );
}

#[test]
fn a_renamed_store_is_found_again_after_a_401_or_a_410() {
    let r = Remote::new(&["work"]);
    let old = r.ids[0].clone();
    r.ok(&["--store", "work", "list"]);
    // `work` is now another store: the kept list still says the old one,
    // which refuses the token.
    r.fake.rename(&old, "old");
    let work = r.fake.add_store("work");
    r.fake.fault(Fault::Revoked);
    r.open(Some("work"), "After a rename");
    assert_eq!((r.cases(&old), r.cases(&work)), (0, 1));
    let lines = r.fake.lines();
    let last: Vec<&str> = lines[lines.len() - 3..]
        .iter()
        .map(|line| line.split(" HTTP").next().unwrap())
        .collect();
    assert_eq!(
        last,
        [
            format!("POST /v1/stores/{old}/store/cases"),
            "GET /v1/cli/whoami".to_owned(),
            format!("POST /v1/stores/{work}/store/cases"),
        ]
    );

    // A store removed (410) is found again the same way.
    r.fake.remove(&work);
    let again = r.fake.add_store("work");
    r.open(Some("work"), "After a removal");
    assert_eq!(r.cases(&again), 1);

    // When the name is gone from the account, it says so.
    r.fake.remove(&again);
    let out = r.run(&["--store", "work", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .contains("no store named work in account ryan on"),
        "{out:?}"
    );
}

#[test]
fn with_no_store_named_the_default_store_is_used() {
    // The account's default.
    let r = Remote::new(&["me", "team"]);
    r.open(None, "To the default");
    assert_eq!(r.cases(&r.ids[0]), 1);
    // The config's default-store, a name or an id, comes first.
    r.env.config("default-store = \"team\"\n");
    r.open(None, "To default-store");
    assert_eq!(r.cases(&r.ids[1]), 1);
    r.env.config(&format!("default-store = \"{}\"\n", r.ids[0]));
    r.open(None, "To default-store by id");
    assert_eq!(r.cases(&r.ids[0]), 2);

    // With no default, the only store the token opens.
    let solo = Remote::new(&["solo"]);
    solo.fake.set_default(&solo.ids[0], false);
    solo.open(None, "To the only store");
    assert_eq!(solo.cases(&solo.ids[0]), 1);

    // With several and no default, nothing is guessed: exit 1, and the
    // stores are listed.
    let two = Remote::new(&["me", "team"]);
    two.fake.set_default(&two.ids[0], false);
    let out = two.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: no store given, and account ryan on {} has 2 stores and no default: name one \
             with --store, CASES_STORE or the config file's default-store\n  me    {}\n  team  \
             {}\n",
            two.fake.url(),
            two.ids[0],
            two.ids[1]
        )
    );
}

#[test]
fn a_store_s_address_names_it_for_one_call() {
    let r = Remote::new(&["me", "team"]);
    let mut env = Env::new();
    env.set("CASES_TOKEN", TOKEN);
    // No server is set: the address gives it, with a name or an id, and
    // the scheme in any case.
    let url = r.fake.url();
    let by_name = format!("{url}/team");
    let out = env.run([
        "--store",
        &by_name,
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "One-off",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    let by_id = format!("{}/{}", url.replacen("http", "HTTP", 1), r.ids[1]);
    let out = env.run(["--store", &by_id, "list", "--json"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out.stdout)["list"].as_array().unwrap().len(), 1);
    // Plain http is only for a loopback host, and an address names a store.
    for bad in [
        "http://cases.sh/me",
        &format!("{url}/"),
        &format!("{url}/a/b"),
    ] {
        let out = env.run(["--store", bad, "list"]);
        assert_eq!(out.code, 1, "{bad}: {out:?}");
        assert!(!env.path("data").exists(), "{bad} made a local store");
    }
}

#[test]
fn a_path_is_a_local_store_even_with_a_server_set() {
    let r = Remote::new(&["me"]);
    for path in ["./local.db", "sub/dir", "x.db"] {
        if let Some(dir) = std::path::Path::new(path).parent() {
            fs::create_dir_all(r.env.path(dir.to_str().unwrap())).unwrap();
        }
        r.open(Some(path), "Local");
        assert!(r.env.path(path).exists(), "{path}");
    }
    // A config store that is a path wins over the server, and config show
    // says so.
    r.env.config("store = \"~/mine.db\"\n");
    r.open(None, "Local by config");
    assert!(r.env.path("home/mine.db").exists());
    assert_eq!(r.cases(&r.ids[0]), 0);
    let out = r.ok(&["config", "show"]);
    assert!(
        out.contains(&format!(
            "The store is the file {}, which wins over the server {}.",
            r.env.path("home/mine.db").display(),
            r.fake.url()
        )),
        "{out}"
    );
    assert_eq!(r.fake.lines(), Vec::<String>::new());

    // With no server, a bare name is a path, as it always was.
    let mut local = Env::new();
    local.set("CASES_TOKEN", TOKEN);
    let out = local.run([
        "--store",
        "work",
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "File",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(local.path("work").exists());
}

#[test]
fn refusals_exit_as_on_a_local_store() {
    let r = Remote::new(&["me"]);
    let id = r.open(None, "Exit codes");
    // A stale write exits 4, a transition the state refuses 3, and the
    // rest 1 (CL-01).
    let out = r.run(&["note", &id, "--body", "x", "--revision", "9"]);
    assert_eq!(out.code, 4, "{out:?}");
    let out = r.run(&["pickup", &id]);
    assert_eq!(out.code, 3, "{out:?}");
    let out = r.run(&[
        "open",
        "--kind",
        "poll",
        "--urgency",
        "today",
        "--title",
        "x",
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    let out = r.run(&["show", "zzzzz"]);
    assert_eq!(out.code, 1, "{out:?}");
    // A server that fails, or answers with a redirect, exits 1.
    r.fake.fault(Fault::Redirect);
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("redirect"), "{out:?}");
    // A write that got no answer may have been stored, and it says so.
    r.fake.fault(Fault::Hangup);
    let out = r.run(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Lost?",
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .contains("may or may not have been stored: check with cases show"),
        "{out:?}"
    );
}

/// A token the fake mints, through a device sign-in it approves at once:
/// one it takes beside [`TOKEN`], so a test can tell which was sent.
fn mint(fake: &Fake) -> String {
    let server: Server = fake.url().parse().unwrap();
    let signin = Signin::new(server);
    let device = signin.device_start("a test", None).unwrap();
    fake.approve_device(device.user_code()).unwrap();
    signin
        .device_poll(&device)
        .unwrap()
        .token
        .expose()
        .to_owned()
}

/// The token each request to `fake` carried, in turn.
fn sent_tokens(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .filter_map(|request| {
            request
                .lines()
                .find_map(|line| line.strip_prefix("authorization: Bearer "))
                .map(str::to_owned)
        })
        .collect()
}

/// The credentials file in `env`'s config directory.
fn credentials(env: &Env) -> std::path::PathBuf {
    env.path("config").join("cases").join("credentials.toml")
}

/// Keeps `token` for `server` in `env`'s credentials file, as `cases login`
/// does with `CASES_NO_KEYCHAIN` set, beside what the file holds.
fn keep_login(env: &Env, server: &str, token: &str) {
    let path = credentials(env);
    private_dir_all(path.parent().unwrap());
    let text = fs::read_to_string(&path).unwrap_or_default();
    let entry = format!(
        "[\"{server}\"]\naccount = \"{}\"\ntoken = \"{token}\"\n",
        fake::ACCOUNT
    );
    fs::write(&path, text + &entry).unwrap();
    set_mode(&path, 0o600);
}

#[test]
fn the_token_is_cases_token_then_a_token_file_then_the_stored_login() {
    let mut r = Remote::new(&["me"]);
    let minted = mint(&r.fake);
    let url = r.fake.url();
    let used = |r: &Remote| {
        let before = sent_tokens(&r.fake).len();
        let out = r.run(&["list"]);
        assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
        let sent = sent_tokens(&r.fake);
        assert!(sent.len() > before, "{sent:?}");
        sent[before..].to_vec()
    };
    let only = |tokens: Vec<String>, token: &str| {
        assert!(tokens.iter().all(|t| t == token), "{tokens:?}");
    };

    // 3. The stored login, when nothing comes before it.
    r.env.unset("CASES_TOKEN");
    keep_login(&r.env, &url, &minted);
    only(used(&r), &minted);
    // 2. The config file's token-file comes before it.
    let file = r.env.path("home/token");
    fs::create_dir_all(r.env.path("home")).unwrap();
    fs::write(&file, format!("  {TOKEN}\n")).unwrap();
    set_mode(&file, 0o600);
    r.env.config("token-file = \"~/token\"\n");
    only(used(&r), TOKEN);
    // CASES_TOKEN_FILE comes before the config file's, and empty is unset.
    let env_file = r.env.path("home/env-token");
    fs::write(&env_file, format!("{minted}\n")).unwrap();
    set_mode(&env_file, 0o600);
    r.env.set("CASES_TOKEN_FILE", env_file.to_str().unwrap());
    only(used(&r), &minted);
    r.env.set("CASES_TOKEN_FILE", " ");
    only(used(&r), TOKEN);
    r.env.set("CASES_TOKEN_FILE", "~/env-token");
    only(used(&r), &minted);
    // A token file that others can read is warned about.
    set_mode(&env_file, 0o644);
    let out = r.run(&["list"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "can be read by others: make it private with chmod 600 {}",
            env_file.display()
        )),
        "{out:?}"
    );
    set_mode(&env_file, 0o600);
    // 1. CASES_TOKEN comes first, and empty is unset.
    r.env.set("CASES_TOKEN", "");
    only(used(&r), &minted);
    r.env.set("CASES_TOKEN", TOKEN);
    only(used(&r), TOKEN);
    // A CASES_TOKEN that is not a token is an error, and never printed.
    r.env.set("CASES_TOKEN", "not a token");
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.starts_with("Error: CASES_TOKEN: "), "{out:?}");
    assert!(!out.stderr.contains("not a token\n"), "{out:?}");
    // A token file that is not there is an error, not skipped.
    r.env.unset("CASES_TOKEN");
    r.env.set("CASES_TOKEN_FILE", "~/none");
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.starts_with(&format!(
            "Error: the token file {} cannot be read: ",
            r.env.path("home/none").display()
        )),
        "{out:?}"
    );
    for out in [r.run(&["list"]), r.run(&["config", "show"])] {
        assert!(!out.stdout.contains(&minted) && !out.stderr.contains(&minted));
        assert!(!out.stdout.contains(TOKEN) && !out.stderr.contains(TOKEN));
    }
    // config show names the token file in use, and where it came from.
    let out = r.ok(&["config", "show"]);
    assert!(
        out.contains(&format!(
            "  token-file            {}  env\n",
            r.env.path("home/none").display()
        )),
        "{out}"
    );
}

#[test]
fn with_no_token_the_error_names_cases_login() {
    let mut r = Remote::new(&["me"]);
    r.env.unset("CASES_TOKEN");
    let url = r.fake.url();
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: not signed in to {url}: run cases login {url}, or set CASES_TOKEN or a token \
             file (CASES_TOKEN_FILE, or the config file's token-file)\n"
        )
    );
    // The stored login of another server is not this one's.
    keep_login(&r.env, "https://cases.example", TOKEN);
    assert_eq!(r.run(&["list"]).stderr, out.stderr);
    assert_eq!(r.fake.lines(), Vec::<String>::new());
}

#[test]
fn a_one_off_url_to_another_server_gets_only_its_stored_login() {
    // The configured server, a, with a token of its own in CASES_TOKEN
    // and in a token file, and another server, b.
    let mut a = Remote::new(&["me"]);
    let a_token = mint(&a.fake);
    a.env.set("CASES_TOKEN", &a_token);
    let b = Fake::start();
    b.add_store("there");
    let b_url = b.url();
    let one_off = format!("{b_url}/there");
    let list = |a: &Remote| a.run(&["--store", &one_off, "list", "--json"]);

    // Neither CASES_TOKEN nor a token file goes to b.
    let out = list(&a);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: not signed in to {b_url}: run cases login {b_url} (CASES_TOKEN and a token \
             file go only to {})\n",
            a.fake.url()
        )
    );
    let file = a.env.path("home/token");
    fs::create_dir_all(a.env.path("home")).unwrap();
    fs::write(&file, &a_token).unwrap();
    set_mode(&file, 0o600);
    a.env.set("CASES_TOKEN_FILE", file.to_str().unwrap());
    assert_eq!(list(&a).code, 1);
    a.env.unset("CASES_TOKEN");
    assert_eq!(list(&a).code, 1);
    assert_eq!(b.lines(), Vec::<String>::new());

    // b's stored login does.
    keep_login(&a.env, &b_url, TOKEN);
    let out = list(&a);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    a.env.set("CASES_TOKEN", &a_token);
    assert_eq!(list(&a).code, 0);
    let sent = sent_tokens(&b);
    assert!(
        !sent.is_empty() && sent.iter().all(|t| t == TOKEN),
        "{sent:?}"
    );

    // A one-off URL on the configured server takes CASES_TOKEN, as the
    // configured server does.
    let before = sent_tokens(&a.fake).len();
    let own = format!("{}/me", a.fake.url());
    let out = a.run(&["--store", &own, "list"]);
    assert_eq!(out.code, 0, "{out:?}");
    let sent = sent_tokens(&a.fake);
    assert!(sent[before..].iter().all(|t| *t == a_token), "{sent:?}");
}

#[test]
fn a_stored_login_that_cannot_be_read_is_an_error() {
    let mut r = Remote::new(&["me"]);
    r.env.unset("CASES_TOKEN");
    let url = r.fake.url();
    let path = credentials(&r.env);
    private_dir_all(path.parent().unwrap());
    // Not TOML: the error names the file and the line, never what it holds.
    fs::write(&path, format!("[\"{url}\"]\ntoken = \"{TOKEN}\n")).unwrap();
    set_mode(&path, 0o600);
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: the credentials file {} is not TOML at line 2: fix it, or remove it and run \
             cases login again\n",
            path.display()
        )
    );
    // An entry for the server that is not a token.
    fs::write(&path, format!("[\"{url}\"]\ntoken = \"csu_short\"\n")).unwrap();
    let out = r.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("that is not one"), "{out:?}");
    assert!(!out.stderr.contains("csu_short"), "{out:?}");
    // A file that cannot be read.
    #[cfg(unix)]
    {
        fs::write(&path, "").unwrap();
        keep_login(&r.env, &url, TOKEN);
        set_mode(&path, 0o000);
        let out = r.run(&["list"]);
        // Root reads any file; everyone else is refused.
        if out.code == 1 {
            assert!(
                out.stderr.starts_with(&format!(
                    "Error: the credentials file {} cannot be read: ",
                    path.display()
                )),
                "{out:?}"
            );
        }
        set_mode(&path, 0o600);
    }
    assert_eq!(r.fake.lines(), Vec::<String>::new());
    // CASES_TOKEN comes first, so the file is not read at all.
    fs::write(&path, "not toml [").unwrap();
    r.env.set("CASES_TOKEN", TOKEN);
    let out = r.run(&["list"]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");

    // A file others can read, or a directory others can write to, is
    // warned about, with the chmod that fixes it.
    r.env.unset("CASES_TOKEN");
    fs::write(&path, "").unwrap();
    keep_login(&r.env, &url, TOKEN);
    set_mode(&path, 0o644);
    let out = r.run(&["list"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: {} can be read by others: make it private with chmod 600 {}\n",
            path.display(),
            path.display()
        )),
        "{out:?}"
    );
    set_mode(&path, 0o600);
    let dir = path.parent().unwrap();
    set_mode(dir, 0o777);
    let out = r.run(&["list"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "warning: {} can be written to by others: make it private with chmod 700 {}\n",
            dir.display(),
            dir.display()
        )),
        "{out:?}"
    );
    set_mode(dir, 0o700);
    for out in [out, r.run(&["list"])] {
        assert!(!out.stdout.contains(TOKEN) && !out.stderr.contains(TOKEN));
    }
}

#[test]
fn a_refused_token_says_where_it_came_from() {
    let mut r = Remote::new(&["me"]);
    let url = r.fake.url();
    let me = r.ids[0].clone();
    let refused = |r: &Remote, args: &[&str]| {
        r.fake.fault(Fault::Revoked);
        let out = r.run(args);
        assert_eq!(out.code, 1, "{out:?}");
        assert!(!out.stderr.contains(TOKEN), "{out:?}");
        out.stderr
    };

    // CASES_TOKEN: no login hint, since a login would not change it.
    let stores = refused(&r, &["stores"]);
    assert_eq!(
        stores,
        format!("Error: {url}: the server refused the token in CASES_TOKEN: revoked\n")
    );
    let list = refused(&r, &["--store", &me, "list"]);
    assert_eq!(
        list,
        format!(
            "Error: the server refused the token in CASES_TOKEN for the store {me} on {url}: \
             revoked\n"
        )
    );

    // A token file.
    r.env.unset("CASES_TOKEN");
    let file = r.env.path("home/token");
    fs::create_dir_all(r.env.path("home")).unwrap();
    fs::write(&file, TOKEN).unwrap();
    set_mode(&file, 0o600);
    r.env.set("CASES_TOKEN_FILE", file.to_str().unwrap());
    assert_eq!(
        refused(&r, &["stores"]),
        format!(
            "Error: {url}: the server refused the token in {} (CASES_TOKEN_FILE): revoked\n",
            file.display()
        )
    );
    r.env.unset("CASES_TOKEN_FILE");
    r.env.config("token-file = \"~/token\"\n");
    assert_eq!(
        refused(&r, &["stores"]),
        format!(
            "Error: {url}: the server refused the token in the token file {}: revoked\n",
            file.display()
        )
    );

    // The stored login: run cases login.
    r.env.config("");
    keep_login(&r.env, &url, TOKEN);
    let path = credentials(&r.env);
    assert_eq!(
        refused(&r, &["stores"]),
        format!(
            "Error: {url}: the server refused the token cases login kept in {}: revoked. Run \
             cases login {url}\n",
            path.display()
        )
    );
    assert_eq!(
        refused(&r, &["--store", &me, "list"]),
        format!(
            "Error: the server refused the token cases login kept in {} for the store {me} on \
             {url}: revoked. Run cases login {url}\n",
            path.display()
        )
    );
}

/// Sets the mode of the file at `path`.
fn set_mode(path: &std::path::Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

#[test]
fn skill_config_and_help_need_no_token() {
    let mut r = Remote::new(&["me"]);
    r.env.unset("CASES_TOKEN");
    for args in [
        &["skill", "list"][..],
        &["skill", "show", "claude"],
        &["config", "show"],
        &["config", "path"],
        &["--help"],
        &["list", "--help"],
        &["serve"],
    ] {
        let out = r.run(args);
        assert_eq!(out.code, 0, "{args:?}: {out:?}");
    }
    let out = r.ok(&["config", "show"]);
    assert!(
        out.contains(&format!("  server                {}  env\n", r.fake.url())),
        "{out}"
    );
    assert!(
        out.contains("  store                 the account's default"),
        "{out}"
    );
    r.env
        .config("default-store = \"team\"\ntoken-file = \"~/token\"\n");
    let out = r.ok(&["config", "show"]);
    assert!(out.contains("  default-store         team"), "{out}");
    let token = r.env.path("home/token");
    assert!(
        out.contains(&format!("  token-file            {}", token.display())),
        "{out}"
    );
    assert_eq!(r.fake.lines(), Vec::<String>::new());
}

#[test]
fn no_output_error_or_kept_file_holds_the_token() {
    let r = Remote::new(&["me", "team"]);
    let id = r.open(Some("team"), "Secret?");
    let mut outs = vec![
        r.run(&["show", &id, "--json"]),
        r.run(&["list"]),
        r.run(&["status", "--json"]),
        r.run(&["config", "show"]),
        r.run(&["pickup", &id]),
    ];
    for fault in [
        Fault::Revoked,
        Fault::Gone,
        Fault::Status(500),
        Fault::Redirect,
        Fault::Garbage,
    ] {
        r.fake.fault(fault);
        outs.push(r.run(&["--store", &r.ids[1], "list"]));
    }
    // A token of the right shape that the server refuses.
    let mut wrong = Remote::new(&["me"]);
    wrong
        .env
        .set("CASES_TOKEN", &format!("csu_{}", "W".repeat(36)));
    outs.push(wrong.run(&["list"]));
    // One of the wrong shape.
    wrong.env.set("CASES_TOKEN", "csu_short");
    outs.push(wrong.run(&["list"]));
    for out in &outs {
        assert!(
            !out.stdout.contains(TOKEN) && !out.stderr.contains(TOKEN),
            "{out:?}"
        );
        assert!(!out.stderr.contains("csu_short"), "{out:?}");
        assert!(!out.stderr.contains(&"W".repeat(36)), "{out:?}");
    }
    assert!(!r.kept().contains(TOKEN));
    // On the wire, only in the Authorization header.
    let requests = r.fake.requests();
    assert!(requests.iter().all(|request| request.contains(TOKEN)));
    for request in requests {
        for line in request.lines().filter(|line| line.contains(TOKEN)) {
            assert_eq!(
                line,
                format!("authorization: Bearer {TOKEN}").as_str(),
                "{request}"
            );
        }
    }
}

#[test]
fn the_human_is_the_unnamed_human_on_a_remote_store() {
    let r = Remote::new(&["me"]);
    r.env.config("name = \"Configured\"\n");
    let id = r.open(None, "Who answers?");
    r.ok(&["answer", &id, "--ack", "--as", "Bob"]);
    let shown = json(&r.ok(&["show", &id, "--json"]));
    let actor = &shown["events"][1]["actor"];
    assert_eq!(actor["kind"], "human", "{shown}");
    assert!(actor.get("name").is_none(), "{shown}");
    // withdraw --as still makes the withdraw the human's, as the same
    // unnamed human.
    let other = r.open(None, "Withdrawn by the human");
    r.ok(&["withdraw", &other, "--as", "Ryan"]);
    let shown = json(&r.ok(&["show", &other, "--json"]));
    let last = shown["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["author"], "human", "{shown}");
    assert_eq!(last["actor"]["id"], actor["id"], "{shown}");
    assert!(last["actor"].get("name").is_none(), "{shown}");
}

#[test]
fn status_and_inbox_give_the_hosted_inbox() {
    let r = Remote::new(&["me", "team"]);
    let url = r.fake.url();
    let team = &r.ids[1];
    let id = r.open(Some("team"), "Pin the bun");
    let before = r.fake.lines().len();
    let out = r.ok(&["--store", "team", "status"]);
    assert_eq!(
        out,
        format!("Remote store team ({team}) on {url}, inbox at {url}/\n")
    );
    let out = json(&r.ok(&["--store", "team", "status", "--json"]));
    assert_eq!(
        out,
        serde_json::json!({
            "server": url,
            "store": {"id": team, "name": "team"},
            "url": format!("{url}/"),
        })
    );
    // Neither sent a request: the list was kept.
    assert_eq!(r.fake.lines().len(), before);

    let page = format!("{url}/cases/{team}-team/{id}-pin-the-bun");
    assert_eq!(
        r.ok(&["--store", "team", "inbox", "--print"]),
        format!("{url}/\n")
    );
    assert_eq!(
        r.ok(&["--store", "team", "inbox", "--print", &id]),
        format!("{page}\n")
    );
    // show --json's url is the case's hosted page.
    let shown = json(&r.ok(&["--store", "team", "show", &id, "--json"]));
    assert_eq!(shown["url"], page);
    let shown = r.ok(&["--store", "team", "show", &id]);
    assert!(shown.contains(&page), "{shown}");
}

#[test]
fn serve_and_service_install_nothing_on_a_remote_store() {
    let mut r = Remote::new(&["me"]);
    // Neither needs a token, nor sends a request.
    r.env.unset("CASES_TOKEN");
    let url = r.fake.url();
    let out = r.run(&["serve"]);
    assert_eq!((out.code, out.stdout), (0, format!("{url}/\n")));
    let out = r.run(&["service", "install"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        format!("Error: a remote store needs no local service; its inbox is at {url}/\n")
    );
    // service status is about the installed unit, as on a local store: it
    // reads no store and needs no token.
    let out = r.run(&["service", "status"]);
    assert!(!out.stderr.contains("token"), "{out:?}");
    assert_eq!(r.fake.lines(), Vec::<String>::new());
}

#[test]
fn prune_sweep_wait_and_doing_work_on_a_remote_store() {
    let r = Remote::new(&["me"]);
    let me = &r.ids[0];
    // sweep withdraws the open cases as the human.
    let open = r.open(None, "Swept");
    let out = r.ok(&["sweep", "--yes"]);
    assert_eq!(out, format!("{open} withdrawn\n"));
    // prune archives the withdrawn case, and --delete removes one.
    let out = r.ok(&["prune", "--age", "0", "--yes"]);
    assert_eq!(out, format!("{open} archived\n"));
    assert_eq!(r.cases(me), 0);
    let gone = r.open(None, "Deleted");
    r.ok(&["withdraw", &gone]);
    let out = r.run(&["prune", "--age", "0"]);
    assert!(out.stderr.contains("moved to the archive in"), "{out:?}");
    r.ok(&["prune", "--age", "0", "--delete", "--yes"]);
    assert_eq!(r.cases(me), 0);

    // wait wakes on an answer, and is not shown as listening.
    let id = r.open(None, "Waited on");
    // The answer lands after the open, by the store's clock.
    r.fake
        .set_clock(me, "2026-09-15T09:05:00Z".parse::<Time>().unwrap());
    r.ok(&["answer", &id, "--ack"]);
    let out = r.ok(&["wait", "--id", &id, "--since", &id, "--timeout", "5s"]);
    assert_eq!(json(out.lines().next().unwrap())["id"], id.as_str());
    let out = r.run(&["wait", "--id", &id, "--timeout", "1s"]);
    assert_eq!(out.code, 2, "{out:?}");
    // doing is refused until the listening wait comes (ST-16).
    let out = r.run(&["doing", "--worker", "w", "Reading"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not listening"), "{out:?}");
    // A message is sent with a warning that no one listens.
    let out = r.run(&["message", "--as", "Ryan", "--to", "w", "--title", "Hi"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stderr.contains("no session is listening"), "{out:?}");
    // tui takes the store as every command does, and needs a terminal.
    let out = r.run(&["tui"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("needs a terminal"), "{out:?}");
}

/// The held polls the fake has been sent, each as its `timeout`.
fn holds(fake: &Fake) -> Vec<u64> {
    fake.lines()
        .iter()
        .filter_map(|line| line.split_once("/store/changes?after="))
        .map(|(_, query)| {
            let query = query.split(' ').next().unwrap();
            let (_, secs) = query.split_once("&timeout=").unwrap();
            secs.parse().unwrap()
        })
        .collect()
}

/// Waits up to `within` for `done`, and gives whether it came.
fn soon(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    done()
}

impl Remote {
    /// Starts `cases --store <the first store> wait` with `args`.
    fn wait(&self, args: &[&str]) -> Child {
        let mut all = vec!["--store", self.ids[0].as_str(), "wait"];
        all.extend(args);
        spawn(
            self.env
                .command(all)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
    }
}

/// Lets the next request through as the fake would answer it.
const PASS: Fault = Fault::Slow(Duration::ZERO);

/// A wait on a remote store holds the poll, up to 25 s and never past its
/// timeout, and wakes as soon as the version moves, rather than reading the
/// store once a second.
#[test]
fn a_remote_wait_wakes_within_a_held_poll() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Held");
    let wait = r.wait(&["--id", &id, "--timeout", "60s"]);
    assert!(soon(Duration::from_secs(10), || holds(&r.fake).len() == 1));
    assert_eq!(holds(&r.fake), [25]);
    thread::sleep(Duration::from_millis(300));
    let answered = Instant::now();
    r.ok(&["--store", &r.ids[0], "answer", &id, "--ack"]);
    let out = Output::from(wait.wait_with_output().unwrap());
    let took = answered.elapsed();
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(out.stdout.lines().next().unwrap())["id"], id.as_str());
    assert!(took < Duration::from_secs(3), "{took:?}");
    assert_eq!(
        holds(&r.fake).len(),
        1,
        "it polled again: {:?}",
        r.fake.lines()
    );
    assert_eq!(out.stderr, "");
}

/// A held poll never runs past the wait's timeout: its hold is the time
/// left, and the wait ends at its timeout, not at the server's 25 s.
#[test]
fn a_remote_wait_keeps_to_its_timeout() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Never answered");
    let start = Instant::now();
    let out = Output::from(
        r.wait(&["--id", &id, "--timeout", "3s"])
            .wait_with_output()
            .unwrap(),
    );
    let took = start.elapsed();
    assert_eq!(out.code, 2, "{out:?}");
    assert!(
        took >= Duration::from_secs(3) && took < Duration::from_secs(5),
        "{took:?}"
    );
    let holds = holds(&r.fake);
    assert!(
        !holds.is_empty() && holds.iter().all(|&secs| secs <= 3),
        "{holds:?}"
    );
    assert!(!out.stderr.contains("warning"), "{out:?}");
}

/// After its first read, a wait rides out a server that fails for a while,
/// says so once, and still wakes on the answer.
#[test]
fn a_remote_wait_rides_out_a_503_after_its_first_read() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Through an outage");
    // The first read, then the first held poll and both its retries.
    r.fake.fault(PASS);
    for _ in 0..3 {
        r.fake.fault(Fault::Status(503));
    }
    let wait = r.wait(&["--id", &id, "--timeout", "60s"]);
    assert!(soon(Duration::from_secs(20), || holds(&r.fake).len() == 4));
    thread::sleep(Duration::from_millis(300));
    r.ok(&["--store", &r.ids[0], "answer", &id, "--ack"]);
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(out.stdout.lines().next().unwrap())["id"], id.as_str());
    assert_eq!(
        out.stderr,
        "warning: the server failed (503): the fake failed; the wait goes on, trying again \
         until the timeout\n"
    );
}

/// A read of the store that fails after the first, once a held poll has
/// said it changed, is ridden out too: the wait says so once, reads again,
/// and wakes on the answer.
#[test]
fn a_remote_wait_rides_out_a_503_on_a_read_after_its_first() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Read through an outage");
    // The first read and the first held poll, then the read it wakes to
    // and both its retries.
    r.fake.fault(PASS);
    r.fake.fault(PASS);
    for _ in 0..3 {
        r.fake.fault(Fault::Status(503));
    }
    let wait = r.wait(&["--id", &id, "--timeout", "60s"]);
    assert!(soon(Duration::from_secs(20), || holds(&r.fake).len() == 1));
    let answer = cases_core::Append {
        author: "human".into(),
        event: "answer".into(),
        data: r#"{"ack": true}"#.into(),
    };
    r.fake
        .db(&r.ids[0])
        .append(
            &common::key(&id),
            &answer,
            &cases_core::Writer::new(None),
            None,
        )
        .unwrap();
    r.fake.wake();
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(out.stdout.lines().next().unwrap())["id"], id.as_str());
    assert_eq!(
        out.stderr,
        "warning: the server failed (503): the fake failed; the wait goes on, trying again \
         until the timeout\n"
    );
}

/// A wait whose first read fails fails at once, even for a failure it
/// would ride out later, so that a wrong store is not waited on.
#[test]
fn a_remote_wait_whose_first_read_fails_fails_at_once() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Unreachable");
    for _ in 0..3 {
        r.fake.fault(Fault::Status(503));
    }
    let start = Instant::now();
    let out = Output::from(
        r.wait(&["--id", &id, "--timeout", "60s"])
            .wait_with_output()
            .unwrap(),
    );
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("the server failed (503)"), "{out:?}");
    assert!(!out.stderr.contains("goes on"), "{out:?}");
    assert!(start.elapsed() < Duration::from_secs(10));
}

/// A 429 whose `Retry-After` runs past the timeout is not waited out: the
/// wait ends at its timeout, having said why once.
#[test]
fn a_remote_wait_does_not_overshoot_on_a_429() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Limited");
    r.fake.fault(PASS);
    r.fake.fault(Fault::Rate(Some(10)));
    let start = Instant::now();
    let out = Output::from(
        r.wait(&["--id", &id, "--timeout", "3s"])
            .wait_with_output()
            .unwrap(),
    );
    let took = start.elapsed();
    assert_eq!(out.code, 2, "{out:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert_eq!(
        out.stderr
            .matches("the server is limiting requests; try again in 10 s")
            .count(),
        1,
        "{out:?}"
    );
}

/// A steady wait on a remote store with a case still open holds its poll
/// no longer than its quiet time, and prints once that passes with no
/// event new to it, well before its timeout (AG-18).
#[test]
fn a_steady_remote_wait_prints_at_the_end_of_its_quiet_time() {
    let r = Remote::new(&["me"]);
    let answered = r.open(Some(&r.ids[0]), "Answered");
    let left = r.open(Some(&r.ids[0]), "Left open");
    let wait = r.wait(&[
        "--id",
        &answered,
        "--id",
        &left,
        "--mood",
        "steady",
        "--timeout",
        "40s",
    ]);
    assert!(soon(Duration::from_secs(10), || !holds(&r.fake).is_empty()));
    let start = Instant::now();
    r.ok(&["--store", &r.ids[0], "answer", &answered, "--ack"]);
    // The holds sent before the answer landed, the last of which it ends.
    let before = holds(&r.fake).len();
    let out = Output::from(wait.wait_with_output().unwrap());
    let took = start.elapsed();
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<_> = out.stdout.lines().map(json).collect();
    assert_eq!(lines.len(), 1, "{out:?}");
    assert_eq!(lines[0]["id"], answered.as_str());
    // Its quiet time is 10 s, from the read that found the answer.
    assert!(
        took >= Duration::from_secs(9) && took < Duration::from_secs(16),
        "{took:?}"
    );
    // The holds after the answer are cut to the quiet time left.
    let holds = holds(&r.fake);
    assert!(holds[before..].iter().all(|&secs| secs <= 10), "{holds:?}");
}

/// A steady wait whose server fails, in a way it rides out, at the read
/// after its timeout still prints what it collected, exit 0 (AG-18).
#[test]
fn a_steady_remote_wait_that_cannot_read_at_its_timeout_prints_what_it_has() {
    let r = Remote::new(&["me"]);
    let answered = r.open(Some(&r.ids[0]), "Answered");
    let left = r.open(Some(&r.ids[0]), "Left open");
    // The first read and held poll, the read the answer wakes it to and
    // the held poll to its timeout; then every read fails.
    for _ in 0..4 {
        r.fake.fault(PASS);
    }
    for _ in 0..20 {
        r.fake.fault(Fault::Status(503));
    }
    let wait = r.wait(&[
        "--id",
        &answered,
        "--id",
        &left,
        "--mood",
        "steady",
        "--timeout",
        "4s",
    ]);
    assert!(soon(Duration::from_secs(10), || holds(&r.fake).len() == 1));
    let answer = cases_core::Append {
        author: "human".into(),
        event: "answer".into(),
        data: r#"{"ack": true}"#.into(),
    };
    r.fake
        .db(&r.ids[0])
        .append(
            &common::key(&answered),
            &answer,
            &cases_core::Writer::new(None),
            None,
        )
        .unwrap();
    r.fake.wake();
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<_> = out.stdout.lines().map(json).collect();
    assert_eq!(lines.len(), 1, "{out:?}");
    assert_eq!(lines[0]["id"], answered.as_str());
}

/// A steady wait that holds a message, and whose server then fails until its
/// timeout, cannot take it: it ends with the failure, exit 1, and the message
/// is still sent for the next wait (AG-16, AG-18).
#[test]
fn a_steady_remote_wait_that_cannot_take_its_message_at_its_timeout_leaves_it_sent() {
    let r = Remote::new(&["me"]);
    let store = r.ids[0].as_str();
    // A case it waits on still open, so that it lingers after the message.
    r.ok(&[
        "--store",
        store,
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Left open",
        "--worker",
        "w9",
    ]);
    let wait = r.wait(&[
        "--pickup",
        "--worker",
        "w9",
        "--by",
        "w9",
        "--mood",
        "steady",
        "--timeout",
        "6s",
    ]);
    assert!(soon(Duration::from_secs(10), || !holds(&r.fake).is_empty()));
    let sent = r.ok(&[
        "--store",
        store,
        "message",
        "--as",
        "Ryan",
        "--title",
        "Rebase onto main",
        "--to",
        "w9",
        "--body",
        "Then run the tests.",
    ]);
    let sent = sent.trim();
    // Time for the read it wakes to; every request after that fails.
    thread::sleep(Duration::from_secs(2));
    for _ in 0..50 {
        r.fake.fault(Fault::Status(503));
    }
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "", "{out:?}");
    let rows = r.fake.db(store).rows(&common::key(sent)).unwrap();
    assert_eq!(rows.map(|rows| rows.len()), Some(1), "it was picked up");
}

/// A steady wait holding one message, on a remote store, with its take at
/// the end of the quiet time to be met by `fault`. Gives the store, the
/// message and the wait.
fn holding_a_message(fault: Fault) -> (Remote, String, Child) {
    let r = Remote::new(&["me"]);
    let store = r.ids[0].clone();
    r.ok(&[
        "--store",
        &store,
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Left open",
        "--worker",
        "w9",
    ]);
    let wait = r.wait(&[
        "--pickup",
        "--worker",
        "w9",
        "--by",
        "w9",
        "--mood",
        "steady",
        "--timeout",
        "30s",
    ]);
    assert!(soon(Duration::from_secs(10), || !holds(&r.fake).is_empty()));
    let sent = r.ok(&[
        "--store",
        &store,
        "message",
        "--as",
        "Ryan",
        "--title",
        "Rebase onto main",
        "--to",
        "w9",
        "--body",
        "Then run the tests.",
    ]);
    r.fake.fault_event(fault);
    (r, sent.trim().to_owned(), wait)
}

/// A pickup that was stored but whose answer was lost is this wait's: it is
/// read back and printed, for no later wait takes the message again
/// (AG-16).
#[test]
fn a_steady_wait_whose_pickup_answer_is_lost_still_prints_the_message() {
    let (_r, sent, wait) = holding_a_message(Fault::Lost);
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<_> = out.stdout.lines().map(json).collect();
    assert_eq!(lines.len(), 1, "{out:?}");
    assert_eq!(lines[0]["id"], sent.as_str(), "{out:?}");
    assert_eq!(lines[0]["state"], "pickedup", "{out:?}");
}

/// A take that fails at the end of the quiet time is tried again at the next
/// read, not when something else changes the store.
#[test]
fn a_steady_wait_tries_a_failed_take_again_at_once() {
    let start = Instant::now();
    let (r, sent, wait) = holding_a_message(Fault::Status(503));
    let out = Output::from(wait.wait_with_output().unwrap());
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains(&sent), "{out:?}");
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let posts = r.fake.lines();
    let posts = posts
        .iter()
        .filter(|line| line.starts_with("POST ") && line.contains("/events"));
    assert_eq!(posts.count(), 2);
}

/// A steady wait on a remote store holds the poll while it collects, and
/// prints what it has at its timeout, exit 0 (AG-18).
#[test]
fn a_steady_remote_wait_collects_until_its_timeout() {
    let r = Remote::new(&["me"]);
    let answered = r.open(Some(&r.ids[0]), "Answered");
    let left = r.open(Some(&r.ids[0]), "Left open");
    let start = Instant::now();
    let wait = r.wait(&[
        "--id",
        &answered,
        "--id",
        &left,
        "--mood",
        "steady",
        "--timeout",
        "4s",
    ]);
    assert!(soon(Duration::from_secs(10), || !holds(&r.fake).is_empty()));
    r.ok(&["--store", &r.ids[0], "answer", &answered, "--ack"]);
    let out = Output::from(wait.wait_with_output().unwrap());
    let took = start.elapsed();
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<_> = out.stdout.lines().map(json).collect();
    assert_eq!(lines.len(), 1, "{out:?}");
    assert_eq!(lines[0]["id"], answered.as_str());
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(7),
        "{took:?}"
    );
    assert!(
        holds(&r.fake).iter().all(|&secs| secs <= 4),
        "{:?}",
        holds(&r.fake)
    );
}

/// A steady wait, its server failing from the read it found an answer at:
/// it holds the answer, and nothing new can come. Gives it with the time
/// it was woken to that read, before which its quiet time cannot start.
fn collecting_through_an_outage(timeout: &[&str]) -> (Remote, String, Child, Instant) {
    let r = Remote::new(&["me"]);
    let answered = r.open(Some(&r.ids[0]), "Answered");
    let left = r.open(Some(&r.ids[0]), "Left open");
    // The first read and held poll, and the read the answer wakes it to;
    // then every request fails.
    for _ in 0..3 {
        r.fake.fault(PASS);
    }
    for _ in 0..100 {
        r.fake.fault(Fault::Status(503));
    }
    let mut args = vec!["--id", &answered, "--id", &left, "--mood", "steady"];
    args.extend(timeout);
    let wait = r.wait(&args);
    assert!(soon(Duration::from_secs(10), || holds(&r.fake).len() == 1));
    let answer = cases_core::Append {
        author: "human".into(),
        event: "answer".into(),
        data: r#"{"ack": true}"#.into(),
    };
    r.fake
        .db(&r.ids[0])
        .append(
            &common::key(&answered),
            &answer,
            &cases_core::Writer::new(None),
            None,
        )
        .unwrap();
    r.fake.wake();
    (r, answered, wait, Instant::now())
}

/// A steady wait that holds an answer, and whose server then fails every
/// request, prints the answer at the end of its quiet time, exit 0: not at
/// its timeout and, with none, not never (AG-18).
#[test]
fn a_steady_remote_wait_that_cannot_read_while_it_collects_prints_at_the_end_of_its_quiet_time() {
    let mut waits = [
        collecting_through_an_outage(&["--timeout", "60s"]),
        collecting_through_an_outage(&[]),
    ];
    // When each ended, watched together, so that neither is timed by the
    // other. Its quiet time is 10 s; well past it, a wait is stopped.
    let mut ended = [None, None];
    let until = Instant::now() + Duration::from_secs(40);
    while ended.contains(&None) && Instant::now() < until {
        for (wait, ended) in waits.iter_mut().zip(&mut ended) {
            if ended.is_none() && wait.2.try_wait().unwrap().is_some() {
                *ended = Some(Instant::now());
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    for ((_r, answered, mut wait, woke), ended) in waits.into_iter().zip(ended) {
        let Some(ended) = ended else {
            wait.kill().unwrap();
            let out = wait.wait_with_output().unwrap();
            panic!("still waiting {:?} after it woke: {out:?}", woke.elapsed());
        };
        let out = Output::from(wait.wait_with_output().unwrap());
        assert_eq!(out.code, 0, "{out:?}");
        let lines: Vec<_> = out.stdout.lines().map(json).collect();
        assert_eq!(lines.len(), 1, "{out:?}");
        assert_eq!(lines[0]["id"], answered.as_str());
        let took = ended - woke;
        assert!(took >= Duration::from_secs(9), "{took:?}");
        assert_eq!(
            out.stderr,
            "warning: the server failed (503): the fake failed; the wait goes on, trying again \
             until it prints what it has\n"
        );
    }
}

/// A steady wait that holds nothing rides a failing server out until its
/// timeout, as an eager one does, and ends there with nothing, exit 2.
#[test]
fn a_steady_remote_wait_with_nothing_collected_rides_out_a_503_until_its_timeout() {
    let r = Remote::new(&["me"]);
    let id = r.open(Some(&r.ids[0]), "Unanswered");
    r.fake.fault(PASS);
    for _ in 0..100 {
        r.fake.fault(Fault::Status(503));
    }
    let start = Instant::now();
    let out = Output::from(
        r.wait(&["--id", &id, "--mood", "steady", "--timeout", "3s"])
            .wait_with_output()
            .unwrap(),
    );
    let took = start.elapsed();
    assert_eq!(out.code, 2, "{out:?}");
    assert_eq!(out.stdout, "", "{out:?}");
    assert!(
        took >= Duration::from_secs(3) && took < Duration::from_secs(6),
        "{took:?}"
    );
    assert!(
        out.stderr
            .contains("the server failed (503): the fake failed; the wait goes on, trying again until the timeout\n"),
        "{out:?}"
    );
}

/// The reads of cases that `args` sends, as request lines.
fn case_reads(r: &Remote, args: &[&str]) -> Vec<String> {
    let before = r.fake.lines().len();
    r.ok(args);
    r.fake.lines()[before..]
        .iter()
        .filter(|line| line.starts_with("GET ") && line.contains("/store/cases"))
        .cloned()
        .collect()
}

#[test]
fn show_answer_and_inbox_read_the_case_they_name_once() {
    let r = Remote::new(&["me"]);
    let id = r.open(None, "Read once");
    let part = &id[1..id.len() - 1];
    let store = format!("GET /v1/stores/{}/store/cases", r.ids[0]);
    // The case by its key, for a whole id; the list, for part of one.
    let (one, all) = (format!("{store}/1 HTTP/1.1"), format!("{store} HTTP/1.1"));
    for (args, want) in [
        (vec!["show", id.as_str()], &one),
        (vec!["show", part], &all),
        (vec!["inbox", "--print", id.as_str()], &one),
        (vec!["answer", id.as_str(), "--ack"], &one),
    ] {
        assert_eq!(
            case_reads(&r, &args),
            std::slice::from_ref(want),
            "{args:?}"
        );
    }
}
