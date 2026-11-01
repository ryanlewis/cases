//! A build without the `remote` feature, as a release is: it has no remote
//! stores and no login, and says so rather than reaching for a server.
#![cfg(not(feature = "remote"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::Env;

#[test]
fn a_server_or_a_store_s_address_is_refused() {
    let mut env = Env::new();

    let out = env.run(["--store", "https://cases.example/me", "list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("built without remote stores"),
        "{out:?}"
    );

    env.config("server = \"https://cases.example\"\n");
    let out = env.run(["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("the config file's server"), "{out:?}");

    env.config("");
    env.set("CASES_SERVER", "https://cases.example");
    let out = env.run(["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("CASES_SERVER"), "{out:?}");

    // A path still wins over a server, and is used.
    let out = env.run(env.store_args(&["list"]));
    assert_eq!(out.code, 0, "{out:?}");
}

#[test]
fn there_is_no_login_logout_or_stores() {
    let env = Env::new();
    for command in ["login", "logout", "stores"] {
        let out = env.run([command]);
        assert_eq!(out.code, 1, "{command}: {out:?}");
        assert!(out.stderr.contains("unrecognized subcommand"), "{out:?}");
    }
}
