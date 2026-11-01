//! `cases inbox`: the running web inbox's URL, printed or opened (AG-15).
//! No test opens a browser: `$PATH` holds only a stand-in `open` and
//! `xdg-open`, which record the URL they are given.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]
#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Env, write_file};

/// Puts a stand-in `open` and `xdg-open` on the test's `$PATH`, and gives
/// the file each appends the URL it is given to.
fn fake_opener(env: &mut Env) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = env.path("bin");
    let log = env.path("opened");
    for name in ["open", "xdg-open"] {
        let path = bin.join(name);
        write_file(
            &path,
            &format!("#!/bin/sh\necho \"$1\" >> '{}'\n", log.display()),
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.set("PATH", bin.to_str().unwrap());
    log
}

/// What the stand-in opener was given, once it has run. The opener is not
/// waited for, so it may still be starting.
fn opened(log: &PathBuf) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(log)
            && text.ends_with('\n')
        {
            return text;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("nothing was opened");
}

#[test]
fn inbox_print_prints_the_running_url() {
    let env = Env::new();
    let (_listener, url) = env.fake_serve("127.0.0.1:0");
    assert_eq!(env.ok(&["inbox", "--print"]), format!("{url}\n"));
}

#[test]
fn inbox_print_prints_a_case_s_page() {
    let env = Env::new();
    let (_listener, url) = env.fake_serve("127.0.0.1:0");
    let id = env.open_decision();
    let want = format!("{url}cases/{id}-pin-bun\n");
    assert_eq!(env.ok(&["inbox", "--print", &id]), want);
    // Part of an id, or the id with a slug, names the case, as for show.
    assert_eq!(env.ok(&["inbox", "--print", &id[1..]]), want);
    assert_eq!(
        env.ok(&["inbox", "--print", &format!("{id}-pin-bun")]),
        want
    );
}

#[test]
fn inbox_print_refuses_an_unknown_case() {
    let env = Env::new();
    let (_listener, _) = env.fake_serve("127.0.0.1:0");
    env.open_decision();
    let out = env.cases(&["inbox", "--print", "nope"]);
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (1, "", "Error: no case id contains \"nope\"\n")
    );
}

#[test]
fn inbox_opens_the_browser_when_not_printing() {
    let mut env = Env::new();
    let log = fake_opener(&mut env);
    let (_listener, url) = env.fake_serve("127.0.0.1:0");
    let out = env.cases(&["inbox"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, ""), "{out:?}");
    assert_eq!(opened(&log), format!("{url}\n"));

    let id = env.open_decision();
    std::fs::remove_file(&log).unwrap();
    assert_eq!(env.cases(&["inbox", &id]).code, 0);
    assert_eq!(opened(&log), format!("{url}cases/{id}-pin-bun\n"));
}

#[test]
fn inbox_without_a_browser_opener_fails() {
    let mut env = Env::new();
    let (_listener, _) = env.fake_serve("127.0.0.1:0");
    // No $PATH: the opener is not looked for in /usr/bin and /bin.
    let out = env.cases(&["inbox"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("$PATH is not set"), "{out:?}");
    env.set("PATH", env.path("bin").to_str().unwrap());
    let out = env.cases(&["inbox"]);
    assert!(out.stderr.contains("on $PATH"), "{out:?}");
}

#[test]
fn inbox_without_serve_says_how_to_start_one() {
    let mut env = Env::new();
    let log = fake_opener(&mut env);
    for args in [&["inbox", "--print"][..], &["inbox"]] {
        let out = env.cases(args);
        assert_eq!(
            (out.code, out.stdout.as_str(), out.stderr.as_str()),
            (
                1,
                "",
                "not running; start one with `cases serve`, or `cases service install` for an \
                 always-on inbox\n"
            ),
            "{args:?}"
        );
    }
    assert!(!log.exists());
}

/// `--print` prints the link and opens nothing, even with a serve running
/// and a browser opener on `$PATH` (AG-15).
#[test]
fn inbox_print_opens_nothing() {
    let mut env = Env::new();
    let log = fake_opener(&mut env);
    let (_listener, url) = env.fake_serve("127.0.0.1:0");
    let id = env.open_decision();
    assert_eq!(env.ok(&["inbox", "--print"]), format!("{url}\n"));
    assert_eq!(
        env.ok(&["inbox", "--print", &id]),
        format!("{url}cases/{id}-pin-bun\n")
    );
    // The opener is not waited for, so give one a moment to write.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !log.exists(),
        "--print opened {}",
        std::fs::read_to_string(&log).unwrap()
    );
}

/// With a link in the instance file, an allowed host's, `--print` gives it,
/// for another device, while the browser opened here goes to the loopback
/// address, which needs no proxy (IN-11).
#[test]
fn inbox_prints_the_link_and_opens_the_loopback_address() {
    let mut env = Env::new();
    let log = fake_opener(&mut env);
    let (_listener, url) = env.fake_serve("127.0.0.1:0");
    let file = env.instance_file();
    let mut info: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    info["link"] = "https://devbox.example.ts.net:7337/".into();
    write_file(&file, &info.to_string());
    let id = env.open_decision();
    assert_eq!(
        env.ok(&["inbox", "--print", &id]),
        format!("https://devbox.example.ts.net:7337/cases/{id}-pin-bun\n")
    );
    assert_eq!(env.cases(&["inbox", &id]).code, 0);
    assert_eq!(opened(&log), format!("{url}cases/{id}-pin-bun\n"));
}
