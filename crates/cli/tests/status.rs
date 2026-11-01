//! `cases status`: whether the web inbox is running for the store (AG-15).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, json, write_file};

/// Checks that `status` says serve is not running: exit 1, `not running`
/// alone on stderr, and nothing on stdout.
fn assert_not_running(env: &Env) {
    for args in [&["status"][..], &["status", "--json"]] {
        let out = env.cases(args);
        assert_eq!(
            (out.code, out.stdout.as_str(), out.stderr.as_str()),
            (1, "", "not running\n"),
            "{args:?}"
        );
    }
}

#[test]
fn status_prints_the_running_serve() {
    let env = Env::new();
    let (listener, url) = env.fake_serve("127.0.0.1:0");
    let store = std::path::absolute(&env.store).unwrap();
    let out = env.ok(&["status"]);
    assert_eq!(
        out,
        format!(
            "Serving {} at {url} (pid {}, since 2026-09-25T12:00:00Z)\n",
            store.display(),
            std::process::id()
        )
    );

    for flag in ["--json", "-j"] {
        let got = json(&env.ok(&["status", flag]));
        assert_eq!(
            got,
            serde_json::json!({
                "pid": std::process::id(),
                "url": url,
                "addr": listener.local_addr().unwrap().to_string(),
                "store": store,
                "started_at": "2026-09-25T12:00:00Z",
                "version": "0.0.0",
            })
        );
    }

    // Another store has no serve.
    let mut other = Env::new();
    other.set("XDG_STATE_HOME", env.path("state").to_str().unwrap());
    assert_not_running(&other);
}

#[test]
fn status_without_serve_is_not_running() {
    assert_not_running(&Env::new());
}

#[test]
fn a_stale_instance_file_is_not_running() {
    // Nothing accepts on the address: serve stopped without removing its
    // file, and the system may have given its pid to another process.
    let env = Env::new();
    let (listener, _) = env.fake_serve("127.0.0.1:0");
    drop(listener);
    assert_not_running(&env);

    // A process that is gone, with something else on the address.
    let listener = common::listen("127.0.0.1:0");
    let addr = listener.local_addr().unwrap();
    let mut gone = common::spawn(std::process::Command::new("/bin/sh").args(["-c", "exit 0"]));
    let pid = gone.id();
    gone.wait().unwrap();
    let info = serde_json::json!({
        "pid": pid, "url": format!("http://{addr}/"), "addr": addr.to_string(),
        "store": env.store, "started_at": "2026-09-25T12:00:00Z", "version": "0.0.0",
    });
    write_file(&env.instance_file(), &info.to_string());
    assert_not_running(&env);
}

#[test]
fn an_instance_file_that_cannot_be_read_is_an_error_naming_it() {
    let env = Env::new();
    for text in ["{not json", "{\"pid\": \"one\"}"] {
        write_file(&env.instance_file(), text);
        let out = env.cases(&["status"]);
        assert_eq!((out.code, out.stdout.as_str()), (1, ""), "{text}: {out:?}");
        // The rest is serde_json's wording, which a dependency bump may change.
        let prefix = format!("Error: read {}: ", env.instance_file().display());
        assert!(out.stderr.starts_with(&prefix), "{text}: {out:?}");
        assert_eq!(out.stderr.matches('\n').count(), 1, "{text}: {out:?}");
    }
}
