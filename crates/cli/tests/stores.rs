//! `cases stores` (ST-11, AC-04, SV-01): the account's stores on the
//! server, and which one commands use, against the remote store crate's
//! fake of the hosted server.
#![cfg(feature = "remote")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;
#[path = "../../store-remote/tests/fake/mod.rs"]
mod fake;

use std::fs;

use common::{Env, Output, json};
use fake::{Fake, Fault, TOKEN};
use serde_json::Value;

/// A fake server with the stores `names`, the first the account's default,
/// and a home that points at it with `CASES_SERVER` and `CASES_TOKEN`.
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
        Remote { fake, env, ids }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.env.run(args)
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (0, ""),
            "{args:?}: {out:?}"
        );
        out.stdout
    }

    /// `cases stores --json`, with `args` before it.
    fn shown(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.extend(["stores", "--json"]);
        json(&self.ok(&all))
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
    fn kept(&self) -> Value {
        let host = self.fake.url().trim_start_matches("http://").to_owned();
        let path = self
            .env
            .path("state")
            .join("cases/accounts")
            .join(format!("{host}.json"));
        json(&fs::read_to_string(path).unwrap())
    }
}

#[test]
fn stores_lists_the_account_s_stores_and_the_one_in_use() {
    let r = Remote::new(&["me", "team-x"]);
    let url = r.fake.url();
    let (me, team) = (&r.ids[0], &r.ids[1]);
    assert_eq!(
        r.ok(&["stores"]),
        format!(
            "account ryan (ac_fake) on {url}\n\
             token tests (all stores), expires 2026-12-31\n\
             \n\
             \x20 NAME    ID                   DEFAULT\n\
             * me      {me}  account\n\
             \x20 team-x  {team}\n"
        )
    );
    // The config's default-store is marked, and is the store in use.
    r.env.config("default-store = \"team-x\"\n");
    assert_eq!(
        r.ok(&["stores"]),
        format!(
            "account ryan (ac_fake) on {url}\n\
             token tests (all stores), expires 2026-12-31\n\
             \n\
             \x20 NAME    ID                   DEFAULT\n\
             \x20 me      {me}  account\n\
             * team-x  {team}  config\n"
        )
    );
    assert_eq!(
        r.shown(&[]),
        serde_json::json!({
            "server": url,
            "account": {"id": "ac_fake", "handle": "ryan"},
            "token": {"name": "tests", "stores": null, "expires": "2026-12-31T10:00:00Z"},
            "stores": [
                {"id": me, "name": "me", "default": true},
                {"id": team, "name": "team-x", "default": false},
            ],
            "selected": {"id": team, "name": "team-x", "from": "default-store"},
        })
    );
}

#[test]
fn stores_reads_the_account_afresh_and_keeps_the_list() {
    let r = Remote::new(&["me"]);
    r.ok(&["list"]);
    assert_eq!(r.whoamis(), 1);
    // A store made since: the kept list does not have it, and commands that
    // need no other store do not read the list again.
    let new = r.fake.add_store("new");
    r.ok(&["list"]);
    assert_eq!(r.whoamis(), 1);
    assert_eq!(r.kept()["stores"].as_array().unwrap().len(), 1);
    // stores reads it every time, and keeps what it read.
    let out = r.ok(&["stores"]);
    assert!(out.contains(&format!("  new   {new}\n")), "{out}");
    assert_eq!(r.whoamis(), 2);
    let kept = r.kept();
    assert_eq!(kept["stores"][1]["id"], new.as_str());
    assert_eq!(kept["stores"][1]["name"], "new");
    r.ok(&["stores", "--json"]);
    assert_eq!(r.whoamis(), 3);
    // The next command takes the name from the kept list.
    r.ok(&["--store", "new", "list"]);
    assert_eq!(r.whoamis(), 3);
}

#[test]
fn selected_says_where_the_store_in_use_came_from() {
    let mut r = Remote::new(&["me", "team"]);
    let (me, team) = (r.ids[0].clone(), r.ids[1].clone());
    let selected = |r: &Remote, args: &[&str]| r.shown(args)["selected"].clone();
    let one = |id: &str, name: &str, from: &str| serde_json::json!({"id": id, "name": name, "from": from});
    assert_eq!(selected(&r, &[]), one(&me, "me", "account"));
    assert_eq!(
        selected(&r, &["--store", "team"]),
        one(&team, "team", "flag")
    );
    // By id, the list gives its name.
    assert_eq!(
        selected(&r, &["--store", &team]),
        one(&team, "team", "flag")
    );
    r.env.config("store = \"team\"\n");
    assert_eq!(selected(&r, &[]), one(&team, "team", "config"));
    r.env.set("CASES_STORE", "me");
    assert_eq!(selected(&r, &[]), one(&me, "me", "env"));
    r.env.unset("CASES_STORE");
    r.env.config(&format!("default-store = \"{team}\"\n"));
    assert_eq!(selected(&r, &[]), one(&team, "team", "default-store"));
    // A name the account does not have: no store to use.
    assert_eq!(selected(&r, &["--store", "nope"]), Value::Null);

    // With no default, the only store; with several, none.
    let solo = Remote::new(&["solo"]);
    solo.fake.set_default(&solo.ids[0], false);
    assert_eq!(selected(&solo, &[]), one(&solo.ids[0], "solo", "only"));
    let two = Remote::new(&["me", "team"]);
    two.fake.set_default(&two.ids[0], false);
    assert_eq!(selected(&two, &[]), Value::Null);
    let out = two.ok(&["stores"]);
    assert!(!out.contains('*'), "{out}");
}

#[test]
fn a_store_s_address_lists_its_server() {
    let r = Remote::new(&["me", "team"]);
    let mut env = Env::new();
    env.set("CASES_TOKEN", TOKEN);
    let address = format!("{}/team", r.fake.url());
    let out = env.run(["--store", &address, "stores", "--json"]);
    assert_eq!(out.code, 0, "{out:?}");
    let shown = json(&out.stdout);
    assert_eq!(shown["server"], r.fake.url());
    assert_eq!(shown["selected"]["id"], r.ids[1].as_str());
    assert_eq!(shown["selected"]["from"], "flag");
}

#[test]
fn a_path_that_wins_over_the_server_is_named_last() {
    let r = Remote::new(&["me"]);
    r.env.config("store = \"~/mine.db\"\n");
    let out = r.ok(&["stores"]);
    let mine = r.env.path("home/mine.db");
    assert!(
        out.ends_with(&format!(
            "  me    {}  account\n\nThe store is the file {}, which wins over the server {}.\n",
            r.ids[0],
            mine.display(),
            r.fake.url()
        )),
        "{out}"
    );
    assert_eq!(r.shown(&[])["selected"], Value::Null);
}

#[test]
fn a_scoped_token_lists_only_its_stores() {
    let r = Remote::new(&["me", "team-x", "ops"]);
    let (team, ops) = (&r.ids[1], &r.ids[2]);
    let body = |stores: &str| {
        format!(
            r#"{{"account": {{"id": "ac_fake", "handle": "ryan"}},
                "token": {{"name": "ci", "stores": [{stores}], "expires": null}},
                "stores": [{{"id": "{team}", "name": "team-x", "default": false}},
                           {{"id": "{ops}", "name": "ops", "default": false}}]}}"#
        )
    };
    r.fake
        .fault(Fault::Body(body(&format!(r#""{team}", "{ops}""#))));
    assert_eq!(
        r.ok(&["stores"]),
        format!(
            "account ryan (ac_fake) on {}\n\
             token ci (team-x, ops only), does not expire\n\
             \n\
             \x20 NAME    ID                   DEFAULT\n\
             \x20 team-x  {team}\n\
             \x20 ops     {ops}\n",
            r.fake.url()
        )
    );
    r.fake.fault(Fault::Body(body(&format!(r#""{team}""#))));
    let shown = r.shown(&[]);
    assert_eq!(shown["token"]["stores"], serde_json::json!([team]));
    assert_eq!(shown["token"]["expires"], Value::Null);
    assert_eq!(shown["stores"].as_array().unwrap().len(), 2);
    assert_eq!(shown["selected"], Value::Null);
    // The kept list is the scoped one.
    assert_eq!(r.kept()["stores"][0]["id"], team.as_str());
}

#[test]
fn stores_with_no_server_or_a_refused_token_exits_1() {
    // No server: the store is the local file, and says which.
    let mut local = Env::new();
    local.set("CASES_TOKEN", TOKEN);
    let out = local.run(["stores"]);
    assert_eq!((out.code, out.stdout.as_str()), (1, ""), "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: no server is set (CASES_SERVER or the config's server); the store is the \
             local file {}\n",
            local.path("data/cases/cases.db").display()
        )
    );
    let out = local.run(["--store", "work", "stores"]);
    assert!(
        out.stderr.ends_with("the store is the local file work\n"),
        "{out:?}"
    );

    // A refused token: the server's message, and never the token.
    let r = Remote::new(&["me"]);
    r.fake.fault(Fault::Revoked);
    let out = r.run(&["stores"]);
    assert_eq!((out.code, out.stdout.as_str()), (1, ""), "{out:?}");
    assert_eq!(
        out.stderr,
        format!(
            "Error: {}: the server refused the token in CASES_TOKEN: revoked\n",
            r.fake.url()
        )
    );
    let mut wrong = Remote::new(&["me"]);
    wrong
        .env
        .set("CASES_TOKEN", &format!("csu_{}", "W".repeat(36)));
    let out = wrong.run(&["stores"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("the token is not one"), "{out:?}");
    assert!(!out.stderr.contains(&"W".repeat(36)), "{out:?}");
    // No token at all.
    wrong.env.unset("CASES_TOKEN");
    let out = wrong.run(&["stores"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not signed in to"), "{out:?}");

    // Nothing printed holds the token.
    for args in [&["stores"][..], &["stores", "--json"]] {
        let out = r.run(args);
        assert_eq!(out.code, 0, "{out:?}");
        assert!(!out.stdout.contains(TOKEN) && !out.stderr.contains(TOKEN));
    }
}
