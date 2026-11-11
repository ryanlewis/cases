//! `cases serve` (SV-02): where it listens, the instance file it keeps, how
//! it stops, its log, and the store it follows with no page asking.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use common::{Env, json, write_file};
use rustix::process::Signal;

#[test]
fn serve_refuses_an_address_that_is_not_loopback() {
    let env = Env::new();
    for addr in ["0.0.0.0:7337", ":7337", "192.168.1.10:7337", "[::]:7337"] {
        let out = env.cases(&["serve", "--listen", addr]);
        assert_eq!(out.code, 1, "{addr}: {out:?}");
        assert!(out.stderr.contains("loopback"), "{addr}: {out:?}");
        assert!(out.stdout.is_empty(), "{addr}: {out:?}");
    }
    assert!(!env.instance_file().exists());
}

#[test]
fn serve_fails_when_its_address_is_taken() {
    let env = Env::new();
    let taken = common::listen("127.0.0.1:0");
    let addr = taken.local_addr().unwrap().to_string();
    let out = env.cases(&["serve", "--listen", &addr]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains(&addr), "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(
        !env.instance_file().exists(),
        "a serve that did not start recorded itself"
    );
}

/// Off a terminal serve prints one line with its address, and opens no
/// browser: the stand-in opener on `$PATH` is never run.
#[test]
fn off_a_terminal_serve_prints_its_address_and_opens_nothing() {
    let mut env = Env::new();
    let bin = env.path("bin");
    let log = env.path("opened.log");
    for opener in ["open", "xdg-open"] {
        write_file(
            &bin.join(opener),
            &format!("#!/bin/sh\necho \"$@\" >> {}\n", log.display()),
        );
        fs::set_permissions(bin.join(opener), fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.set("PATH", bin.to_str().unwrap());
    let serve = env.serve(&[]);
    let store = env.store.display().to_string();
    assert!(
        serve
            .line
            .starts_with(&format!("Serving {store} at http://127.0.0.1:")),
        "{}",
        serve.line
    );
    assert!(serve.url.ends_with('/'), "{}", serve.url);
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout.trim_end(),
        out.stdout.lines().next().unwrap(),
        "only the one line"
    );
    assert!(!log.exists(), "the browser was opened off a terminal");
}

#[test]
fn serve_records_itself_for_status_and_removes_the_record_when_it_stops() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let file = env.instance_file();
    let info = json(&fs::read_to_string(&file).expect("serve recorded itself"));
    assert_eq!(info["pid"], serve.pid());
    assert_eq!(info["url"], serve.url.as_str());
    assert_eq!(info["addr"], serve.addr.as_str());
    assert_eq!(
        info["store"],
        std::path::absolute(&env.store).unwrap().to_str().unwrap()
    );
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    let started = info["started_at"].as_str().unwrap();
    assert!(
        started.ends_with('Z') && started.parse::<cases_core::Time>().is_ok(),
        "{started}"
    );
    // Only the file: its temporary sibling is gone.
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);

    let status = env.cases(&["status", "--json"]);
    assert_eq!(status.code, 0, "{status:?}");
    assert_eq!(json(&status.stdout)["url"], serve.url.as_str());

    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!file.exists(), "the record outlived serve");
    let status = env.cases(&["status"]);
    assert_eq!(status.code, 1, "{status:?}");
}

/// serve lets go of the store when it stops: with no connection left open,
/// SQLite folds the write-ahead log back into the store's file, so a copy
/// of the file has every write, and the store can be removed and made
/// again.
#[test]
fn serve_closes_the_store_when_it_stops() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    let answer = format!("/v1/cases/{id}/answer");
    let written = serve.post(&answer, r#"{"revision": 1, "answer": {"ack": true}}"#);
    assert_eq!(written.status, 200, "{written:?}");
    let wal = env.path("cases.db-wal");
    assert!(
        wal.exists(),
        "the store keeps a write-ahead log while serve runs"
    );
    assert_eq!(serve.stop().code, 0);
    assert!(!wal.exists(), "serve left the store open");
    fs::remove_file(&env.store).unwrap();
    env.open_fyi("A new store");
}

#[test]
fn serve_stops_on_ctrl_c_and_removes_its_record() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let out = serve.signal(Signal::INT);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!env.instance_file().exists());
}

/// A stop waits for the requests in flight, and a second Ctrl-C ends it at
/// once, rather than wait out the stop.
#[test]
fn a_second_ctrl_c_ends_a_slow_stop_at_once() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let stalled = held(&serve.addr, &serve.csrf());
    serve.raise(Signal::INT);
    std::thread::sleep(Duration::from_millis(300));
    let second = Instant::now();
    let out = serve.signal(Signal::INT);
    assert!(
        second.elapsed() < Duration::from_secs(2),
        "the second Ctrl-C waited {:?}",
        second.elapsed()
    );
    assert_eq!(out.code, 130, "{out:?}");
    drop(stalled);
}

/// serve reads the store before it says it is up, so that a case opened as
/// soon as it is up, before serve has run again, is new and notifies
/// (IN-08).
#[test]
fn a_case_opened_as_soon_as_serve_is_up_notifies() {
    let env = Env::new();
    let serve = env.serve(&[]);
    serve.raise(Signal::STOP);
    let id = env.open_fyi("Arrived");
    serve.raise(Signal::CONT);
    let page = serve.get("/v1/notifications").json();
    assert_eq!(page["latest"], 1, "{page}");
    assert_eq!(page["items"][0]["url"], format!("/cases/{id}-arrived"));
}

/// A request that fails in serve logs why, as well as its line (SV-02).
#[test]
fn a_request_that_fails_logs_why() {
    let env = Env::new();
    env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    // A store a newer cases wrote, which this one cannot read.
    rusqlite::Connection::open(&env.store)
        .unwrap()
        .execute("UPDATE meta SET schema_version = 99", [])
        .unwrap();
    let failed = serve.get("/v1/cases");
    assert_eq!(failed.status, 500, "{failed:?}");
    let log = serve.stop().stderr;
    let cause = log
        .lines()
        .position(|line| line.starts_with("error: ") && line.contains("schema version 99"));
    let request = log
        .lines()
        .position(|line| line.contains("GET /v1/cases 500 "));
    assert!(
        cause.is_some() && cause < request,
        "no cause before the request:\n{log}"
    );
}

/// With `--log`, off a terminal, serve writes its log to the file, with a
/// line for each start, and stderr goes there too rather than repeat it.
#[test]
fn serve_logs_to_its_own_file() {
    let env = Env::new();
    let file = env.path("state/serve.log");
    let serve = env.serve(&["--log", file.to_str().unwrap()]);
    assert_eq!(serve.get("/").status, 200);
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stderr, "");
    let log = fs::read_to_string(&file).unwrap();
    let mut lines = log.lines();
    let (at, first) = lines.next().unwrap().split_once(' ').unwrap();
    assert!(at.parse::<cases_core::Time>().is_ok(), "{log}");
    assert_eq!(
        first,
        format!("Serving {} at {}", env.store.display(), serve_url(&out)),
        "{log}"
    );
    assert!(lines.any(|line| line.contains(" GET / 200 ")), "{log}");
}

/// An error that stops serve goes to its log file too: a service that
/// cannot start says why there.
#[test]
fn an_error_that_stops_serve_goes_to_its_log_file() {
    let env = Env::new();
    let taken = common::listen("127.0.0.1:0");
    let addr = taken.local_addr().unwrap().to_string();
    let file = env.path("serve.log");
    fs::write(&file, "from before\n").unwrap();
    let out = env.cases(&["serve", "--listen", &addr, "--log", file.to_str().unwrap()]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stderr, "");
    let log = fs::read_to_string(&file).unwrap();
    assert!(log.starts_with("from before\n"), "{log}");
    assert!(log.contains(&addr), "{log}");
}

/// The address from serve's first line.
fn serve_url(out: &common::Output) -> &str {
    out.stdout
        .lines()
        .next()
        .unwrap()
        .rsplit(' ')
        .next()
        .unwrap()
}

/// The store is recorded absolute, and with each `..` taken off, for status
/// to show.
#[test]
fn serve_records_its_store_without_dot_dots() {
    let env = Env::new();
    fs::create_dir(env.path("sub")).unwrap();
    let store = env.path("sub").join("..").join("cases.db");
    let mut child = env.spawn(&[
        "--store",
        store.to_str().unwrap(),
        "serve",
        "--listen",
        "127.0.0.1:0",
    ]);
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    let info = fs::read_to_string(env.instance_file());
    child.kill().unwrap();
    child.wait().unwrap();
    let info = json(&info.expect("serve recorded itself"));
    assert_eq!(info["store"], env.store.to_str().unwrap(), "{line}");
}

#[test]
fn a_second_serve_on_the_same_store_is_refused() {
    let env = Env::new();
    let first = env.serve(&[]);
    let out = env.cases(&["serve", "--listen", "127.0.0.1:0"]);
    assert_eq!(out.code, 1, "{out:?}");
    for want in [
        "already running",
        first.url.as_str(),
        &format!("pid {}", first.pid()),
    ] {
        assert!(out.stderr.contains(want), "{want}: {out:?}");
    }
    // The first is still recorded, and a serve on another store is not
    // refused.
    let status = env.cases(&["status", "--json"]);
    assert_eq!(json(&status.stdout)["url"], first.url.as_str());
    let other = Env::new();
    let second = other.serve(&[]);
    assert_ne!(second.url, first.url);
}

/// A record whose process is gone, or whose address takes no connection,
/// counts as no serve, and a new serve replaces it.
#[test]
fn a_stale_record_is_replaced() {
    let mut gone = common::spawn(&mut std::process::Command::new("true"));
    gone.wait().unwrap();
    for pid in [gone.id(), std::process::id()] {
        let env = Env::new();
        env.stale_instance(pid);
        let serve = env.serve(&[]);
        let info = json(&fs::read_to_string(env.instance_file()).unwrap());
        assert_eq!(info["pid"], serve.pid(), "pid {pid}");
        assert_eq!(info["url"], serve.url.as_str(), "pid {pid}");
    }
}

/// serve still serves when it cannot write its record, and says so.
#[test]
fn serve_runs_without_a_state_directory_it_can_write() {
    let mut env = Env::new();
    let state = env.path("state-file");
    write_file(&state, "");
    env.set("XDG_STATE_HOME", state.to_str().unwrap());
    let serve = env.serve(&[]);
    assert_eq!(serve.get("/v1").status, 200);
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains("could not record the instance for cases status: "),
        "{out:?}"
    );
}

/// With neither `$XDG_STATE_HOME` nor `$HOME` there is no state directory
/// to find: serve is not refused for it, serves, and says it could not
/// record itself, and status says why it cannot look.
#[test]
fn serve_runs_with_no_state_directory_to_find() {
    let mut env = Env::new();
    env.unset("XDG_STATE_HOME");
    env.unset("HOME");
    let want = "cannot find the state directory";
    let serve = env.serve(&[]);
    assert_eq!(serve.get("/v1").status, 200);
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "could not record the instance for cases status: {want}"
        )),
        "{out:?}"
    );
    let status = env.cases(&["status"]);
    assert_eq!(status.code, 1, "{status:?}");
    assert!(status.stderr.contains(want), "{status:?}");
}

#[test]
fn the_config_file_gives_the_address_and_the_flag_wins() {
    let env = Env::new();
    env.config("listen = \"127.0.0.1:0\"\n");
    let mut child = env.spawn(&env.store_args(&["serve"]));
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(line.contains(" at http://127.0.0.1:"), "{line}");
    assert!(!line.contains(":7337/"), "{line}");

    env.config("listen = \"0.0.0.0:0\"\n");
    let out = env.cases(&["serve"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("loopback"), "{out:?}");
    let serve = env.serve(&["--listen", "127.0.0.1:0"]);
    assert_eq!(serve.get("/v1").status, 200);
}

/// One line for each page load, write and failed request; none for the
/// reads a page makes by itself that succeed.
#[test]
fn serve_logs_writes_and_failures_but_not_a_page_s_own_reads() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    for path in [
        "/",
        &format!("/cases/{id}"),
        "/static/app.js",
        "/static/style.css",
        "/v1",
        "/v1/cases",
        &format!("/v1/cases/{id}"),
        "/v1/notifications",
        "/favicon.svg?n=1",
    ] {
        assert_eq!(serve.get(path).status, 200, "{path}");
    }
    serve.stream("", None).close();
    assert_eq!(serve.get("/favicon.svg?n=x").status, 400);
    let written = serve.post(
        &format!("/v1/cases/{id}/answer"),
        r#"{"revision": 1, "answer": {"ack": true}}"#,
    );
    assert_eq!(written.status, 200, "{written:?}");
    let log = serve.stop().stderr;
    assert!(log.contains("GET / 200 "), "{log}");
    assert!(log.contains(&format!("GET /cases/{id} 200 ")), "{log}");
    assert!(log.contains("GET /favicon.svg?n=x 400 "), "{log}");
    assert!(
        log.contains(&format!("POST /v1/cases/{id}/answer 200 ")),
        "{log}"
    );
    for quiet in [
        "GET /v1 ",
        "GET /v1/cases",
        "/v1/notifications",
        "/v1/changes",
        "/favicon.svg?n=1",
        "/static/",
    ] {
        assert!(!log.contains(quiet), "{quiet} was logged:\n{log}");
    }
}

/// A page is told of a case the CLI opens by serve's own reads of the
/// store, without asking, and serve stops at once with the page's stream
/// still open, which ends cleanly, so that the browser reconnects to the
/// next serve.
#[test]
fn serve_pushes_changes_and_stops_with_a_stream_open() {
    let env = Env::new();
    env.open_decision();
    let serve = env.serve(&[]);
    let version = serve.stream("", None).next();
    let stream = serve.stream(&format!("?after={version}"), None);
    stream.quiet(Duration::from_millis(300));
    env.open_fyi("New");
    assert_ne!(stream.next(), version);
    let stopped = Instant::now();
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        stopped.elapsed() < Duration::from_secs(3),
        "serve waited on the stream"
    );
    assert_eq!(stream.end(), common::serve::Message::End { clean: true });
}

/// serve stops by itself, cleanly, when an upgrade renames a new binary
/// over the one it runs, so that launchd or systemd starts the new one.
///
/// serve runs from a link in a directory of the test's own to the built
/// binary, a hard one where the directory is on the same file system, and
/// the new binary is a small file: serve only looks at it. Copies of the
/// debug binary, which is over 100 MB, failed where the temporary directory
/// was nearly full, such as a tmpfs /tmp.
#[test]
fn serve_stops_when_its_binary_is_replaced() {
    let env = Env::new();
    let bin = env.path("bin");
    fs::create_dir_all(&bin).unwrap();
    let cases = bin.join("cases");
    if fs::hard_link(env!("CARGO_BIN_EXE_cases"), &cases).is_err() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_cases"), &cases).unwrap();
    }
    let serve = env.serve_as(&cases, &[]);
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(
        serve.get("/v1").status,
        200,
        "serve stopped with its binary untouched"
    );
    let new = bin.join("cases.new");
    write_file(&new, "#!/bin/sh\necho the new cases\n");
    fs::rename(&new, &cases).unwrap();
    let out = serve.exit_within(Duration::from_secs(10));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains(&format!("{} was replaced", cases.display())),
        "{out:?}"
    );
    assert!(!env.instance_file().exists(), "the record outlived serve");
}

/// serve queues a notification for a case that lands on the human while no
/// page asks: its own reads of the store find it.
#[test]
fn serve_queues_notifications_with_no_page_open() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let id = env.open_fyi("Arrived");
    std::thread::sleep(Duration::from_millis(2_500));
    let asked = now();
    let page = serve.get("/v1/notifications").json();
    assert_eq!(page["latest"], 1, "{page}");
    let item = &page["items"][0];
    assert_eq!(item["url"], format!("/cases/{id}-arrived"));
    let queued: cases_core::Time = item["queued_at"].as_str().unwrap().parse().unwrap();
    assert!(
        queued < asked,
        "queued at {queued}, when the page asked at {asked}"
    );
}

/// The time now, as the test's clock reads it.
fn now() -> cases_core::Time {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    cases_core::Time::from_unix(
        i64::try_from(since.as_secs()).unwrap(),
        since.subsec_nanos(),
    )
    .unwrap()
}

/// A terminal of its own for `line`, run by `/bin/sh -c` in
/// `/usr/bin/script` as a terminal window runs a shell: its stdin is the
/// keys typed, and what it writes comes back as it writes it. The forms
/// differ by system.
struct Terminal {
    script: std::process::Child,
    keys: std::process::ChildStdin,
    written: std::sync::mpsc::Receiver<Vec<u8>>,
    seen: String,
}

impl Terminal {
    fn start(env: &Env, line: &str) -> Terminal {
        use std::process::{Command, Stdio};
        let mut command = Command::new("/usr/bin/script");
        if cfg!(target_os = "linux") {
            command.args(["-q", "-c", line, "/dev/null"]);
        } else {
            command.args(["-q", "/dev/null", "/bin/sh", "-c", line]);
        }
        command
            .env_clear()
            .envs(common::coverage_vars())
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env("CASES_NO_KEYCHAIN", "1")
            .env("CASES", env!("CARGO_BIN_EXE_cases"))
            .env("STORE", &env.store)
            .env("HOME", env.path("home"))
            .env("XDG_STATE_HOME", env.path("state"))
            .current_dir(env.path(""))
            // stdin stays open: `script` may end when its input does.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut script = common::spawn(&mut command);
        let keys = script.stdin.take().expect("a stdin");
        let mut stdout = script.stdout.take().expect("a stdout");
        let (chunks, written) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = std::io::Read::read(&mut stdout, &mut buffer) {
                if n == 0 || chunks.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Terminal {
            script,
            keys,
            written,
            seen: String::new(),
        }
    }

    /// An interactive bash, with job control, on a terminal of its own, at
    /// its first prompt.
    fn bash(env: &Env) -> Terminal {
        let mut terminal = Terminal::start(env, "exec /bin/bash --norc --noprofile -i");
        // bash reads it once it is up. Quoted so that the echo of the line
        // typed is not the prompt.
        terminal.type_keys("PS1='bash-ready''> '\n");
        terminal.wait_for(PROMPT);
        terminal
    }

    /// Types `keys`.
    fn type_keys(&mut self, keys: &str) {
        self.keys.write_all(keys.as_bytes()).unwrap();
        self.keys.flush().unwrap();
    }

    /// Waits until the terminal shows `text`, after what it showed before,
    /// and gives what it showed up to it.
    fn wait_for(&mut self, text: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(at) = self.seen.find(text) {
                let shown = self.seen[..at + text.len()].to_owned();
                self.seen.drain(..at + text.len());
                return shown;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(chunk) = self.written.recv_timeout(left) else {
                panic!("the terminal did not show {text:?}: {:?}", self.seen);
            };
            self.seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }

    /// The pid the shell shows after `pid=`, which it prints with
    /// `echo "pid=$!"` or the like: the digits that follow.
    fn pid(&mut self) -> i32 {
        self.wait_for("pid=");
        while !self.seen.contains(|c: char| !c.is_ascii_digit()) {
            let chunk = self.written.recv_timeout(Duration::from_secs(10)).unwrap();
            self.seen.push_str(&String::from_utf8_lossy(&chunk));
        }
        let digits: String = self.seen.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().unwrap()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.script.kill();
        let _ = self.script.wait();
    }
}

/// Ends `pid` with SIGKILL when dropped, as a test that fails might leave
/// it.
struct Reap(i32);

impl Drop for Reap {
    fn drop(&mut self) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.0) {
            let _ = rustix::process::kill_process(pid, Signal::KILL);
        }
    }
}

/// bash's prompt, which nothing else on the terminal shows.
const PROMPT: &str = "bash-ready> ";

/// serve's command line, run from a shell on the test's terminal.
const SERVE: &str = r#""$CASES" --store "$STORE" serve --listen 127.0.0.1:0 --no-open"#;

/// What the status screen shows once serve reads keys.
const KEYS: &str = "\x1b[32mq\x1b[0m quit";

/// The end of the status screen's last line, after [`KEYS`] when it has
/// them.
const LAST: &str = "\x1b[32mCtrl-C\x1b[0m quit";

/// How `pid` stands: its `ps` state, such as `S` or `T`, with its CPU time
/// in seconds, or `None` once it has ended.
fn standing(pid: i32) -> Option<(String, f64)> {
    let ps = common::output(
        std::process::Command::new("ps")
            .args(["-o", "stat=,time=", "-p", &pid.to_string()])
            .stdout(std::process::Stdio::piped()),
    );
    let text = String::from_utf8_lossy(&ps.stdout);
    let mut fields = text.split_whitespace();
    let state = fields.next()?.to_owned();
    if state.starts_with('Z') {
        return None;
    }
    // [[hh:]mm:]ss[.ff]
    let time = fields.next().unwrap().split(':').fold(0.0, |total, part| {
        total * 60.0 + part.parse::<f64>().unwrap()
    });
    Some((state, time))
}

/// Waits up to `within` for `pid` to end, and gives whether it did.
fn ends_within(pid: i32, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while standing(pid).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    standing(pid).is_none()
}

/// On a terminal serve reads each key as it is typed, with no Enter: `q`
/// stops it, and other keys do not. It puts the terminal back as it stops:
/// the cursor shown, and lines buffered and echoed again.
#[test]
fn q_typed_on_its_terminal_stops_serve() {
    let env = Env::new();
    let run = format!(r#"{SERVE}; echo "exit=$?"; stty -a; echo "stty done""#);
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    terminal.type_keys("x");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !terminal.seen.contains("exit="),
        "a key that is not q stopped serve"
    );
    terminal.type_keys("q");
    terminal.wait_for("\x1b[?7h\x1b[?25h");
    terminal.wait_for("exit=0");
    let modes = terminal.wait_for("stty done");
    let modes: Vec<&str> = modes.split_whitespace().collect();
    for mode in ["icanon", "echo"] {
        assert!(modes.contains(&mode), "{mode} is off: {modes:?}");
    }
    assert!(!env.instance_file().exists());
}

/// On a terminal of 4 rows, the status screen draws 3 lines, so that it
/// never scrolls.
#[test]
fn the_status_screen_fits_its_terminal() {
    let env = Env::new();
    let run = format!(r#"stty rows 4 cols 100; {SERVE}; echo "exit=$?""#);
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for("cases serve");
    std::thread::sleep(Duration::from_millis(1500));
    terminal.type_keys("q");
    let shown = terminal.wait_for("exit=0");
    // Below the first 3 lines: the hint to install the skill, and the keys.
    assert!(!shown.contains("Agents:"), "{shown:?}");
    assert!(!shown.contains(KEYS), "{shown:?}");
}

/// After Ctrl-Z and `bg`, the keys are the shell's: serve neither reads
/// them, which would stop it with SIGTTIN, nor quits on a `q` typed to the
/// shell. Back in the foreground, it reads them again.
#[test]
fn in_the_background_serve_leaves_the_keys_to_the_shell() {
    let env = Env::new();
    let mut terminal = Terminal::bash(&env);
    terminal.type_keys(&format!("{SERVE}\n"));
    terminal.wait_for(KEYS);
    terminal.type_keys("\x1a");
    terminal.wait_for(PROMPT);
    // Quoted so that the echo of the line typed has no `pid=`.
    terminal.type_keys("bg; echo \"pid\"=$!\n");
    let pid = terminal.pid();
    let _reap = Reap(pid);
    terminal.wait_for(PROMPT);
    terminal.type_keys("q");
    std::thread::sleep(Duration::from_millis(1000));
    let (state, _) = standing(pid).expect("serve quit on the shell's q");
    assert!(
        !state.starts_with('T'),
        "serve stopped in the background: {state}"
    );
    // Clear the shell's line, and bring serve back. A `q` typed before
    // bash has handed serve the terminal is bash's, which says so and
    // goes on, so type it until serve reads one.
    terminal.type_keys("\x15fg\n");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ends_within(pid, Duration::from_millis(500)) {
        assert!(
            Instant::now() < deadline,
            "q in the foreground again did not stop serve"
        );
        terminal.type_keys("q\n");
    }
}

/// serve started in the background (`cases serve &`) reads no keys, and
/// leaves the terminal's modes to the shell: setting them would stop it
/// with SIGTTOU.
#[test]
fn serve_started_in_the_background_reads_no_keys() {
    let env = Env::new();
    let mut terminal = Terminal::bash(&env);
    terminal.type_keys(&format!("{SERVE} & echo \"pid\"=$!\n"));
    let pid = terminal.pid();
    let _reap = Reap(pid);
    // The first frame, all of it, however it comes in chunks: the keys
    // would be on its last line.
    terminal.wait_for("cases serve");
    let frame = terminal.wait_for(LAST);
    assert!(!frame.contains(KEYS), "{frame:?}");
    std::thread::sleep(Duration::from_millis(1000));
    let (state, _) = standing(pid).expect("serve ended");
    assert!(!state.starts_with('T'), "serve stopped: {state}");
    rustix::process::kill_process(rustix::process::Pid::from_raw(pid).unwrap(), Signal::TERM)
        .unwrap();
    assert!(
        ends_within(pid, Duration::from_secs(5)),
        "serve did not stop on SIGTERM"
    );
}

/// When its terminal goes away, serve keeps serving, and does not spin on
/// the input that has ended. Here it ignores the hang-up, as under `nohup`,
/// so that it outlives the terminal.
#[test]
fn serve_outlives_its_terminal_without_spinning() {
    let env = Env::new();
    let run = format!(r#"echo "pid=$$"; trap "" HUP; exec {SERVE}"#);
    let mut terminal = Terminal::start(&env, &run);
    let pid = terminal.pid();
    let _reap = Reap(pid);
    terminal.wait_for(KEYS);
    terminal.script.kill().unwrap();
    terminal.script.wait().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let (_, before) = standing(pid).expect("serve ended with its terminal");
    std::thread::sleep(Duration::from_secs(2));
    let (_, after) = standing(pid).expect("serve ended with its terminal");
    assert!(
        after - before < 1.0,
        "serve spun: {before} s then {after} s"
    );
    rustix::process::kill_process(rustix::process::Pid::from_raw(pid).unwrap(), Signal::TERM)
        .unwrap();
    assert!(
        ends_within(pid, Duration::from_secs(5)),
        "serve did not stop on SIGTERM"
    );
    assert!(!env.instance_file().exists());
}

/// A log file that cannot be opened is said in the log, and serve serves
/// on without it.
#[test]
fn a_log_file_that_cannot_be_opened_is_logged() {
    let env = Env::new();
    let file = env.path("file");
    write_file(&file, "");
    let log = file.join("serve.log");
    let serve = env.serve(&["--log", log.to_str().unwrap()]);
    assert_eq!(serve.get("/v1").status, 200);
    let out = serve.stop();
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains(&format!("could not open the log file {}: ", log.display())),
        "{out:?}"
    );
    // A stop with nothing in flight closes nothing, and says nothing of it.
    assert!(!out.stderr.contains("connections still open"), "{out:?}");
}

/// An IPv6 address is bracketed in the inbox's address.
#[test]
fn serve_on_ipv6_brackets_its_address() {
    // Not every machine has an IPv6 loopback.
    if std::net::TcpListener::bind("[::1]:0").is_err() {
        return;
    }
    let env = Env::new();
    let serve = env.serve(&["--listen", "[::1]:0"]);
    assert!(serve.url.starts_with("http://[::1]:"), "{}", serve.url);
    assert_eq!(serve.stop().code, 0);
}

/// A stop waits out its grace for the requests in flight, then closes
/// them, and says how many it closed.
#[test]
fn a_stop_closes_what_is_still_open_after_its_grace() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let stalled = held(&serve.addr, &serve.csrf());
    serve.raise(Signal::INT);
    // The grace is 5 s.
    let out = serve.exit_within(Duration::from_secs(15));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains("closed 1 connections still open 5 s after the stop"),
        "{out:?}"
    );
    drop(stalled);
}

/// The address and pid of the serve that `env`'s instance file records,
/// once it does.
fn running(env: &Env) -> (String, i32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let record = loop {
        if let Ok(text) = fs::read_to_string(env.instance_file()) {
            break json(&text);
        }
        assert!(Instant::now() < deadline, "serve recorded nothing");
        std::thread::sleep(Duration::from_millis(50));
    };
    let addr = record["addr"].as_str().unwrap().to_owned();
    (
        addr,
        i32::try_from(record["pid"].as_i64().unwrap()).unwrap(),
    )
}

/// The CSRF token of the serve at `addr`, read as a page reads it.
fn csrf(addr: &str) -> String {
    let mut stream = common::serve::connect(addr);
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET /v1 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut stream, &mut text).unwrap();
    let body = &text[text.find("\r\n\r\n").unwrap() + 4..];
    json(body)["csrf"].as_str().unwrap().to_owned()
}

/// On a terminal serve opens the inbox in the browser, and when it cannot,
/// says why on its screen and serves on.
#[test]
fn on_a_terminal_serve_opens_the_browser() {
    let env = Env::new();
    let bin = env.path("bin");
    let opened = env.path("opened.log");
    for opener in ["open", "xdg-open"] {
        write_file(
            &bin.join(opener),
            &format!("#!/bin/sh\necho \"$@\" >> {}\n", opened.display()),
        );
        fs::set_permissions(bin.join(opener), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let serve = r#""$CASES" --store "$STORE" serve --listen 127.0.0.1:0"#;
    let run = format!(r#"PATH="{}" exec {serve}"#, bin.display());
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (addr, first) = running(&env);
    let reap = Reap(first);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(&opened).is_ok_and(|text| text.contains(&addr)) {
        assert!(Instant::now() < deadline, "the browser was not opened");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        format!("http://{addr}/\n")
    );
    terminal.type_keys("q");
    assert!(ends_within(first, Duration::from_secs(5)));
    // It has ended: no SIGKILL for whatever has its pid next.
    std::mem::forget(reap);

    // `--no-open` opens nothing.
    let run = format!(r#"PATH="{}" exec {serve} --no-open"#, bin.display());
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (_, second) = running(&env);
    assert_ne!(second, first, "the first serve's record was left");
    let reap = Reap(second);
    std::thread::sleep(Duration::from_millis(500));
    terminal.type_keys("q");
    assert!(ends_within(second, Duration::from_secs(5)));
    std::mem::forget(reap);
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        format!("http://{addr}/\n"),
        "--no-open opened the browser"
    );

    let run = format!(r#"PATH="{}" exec {serve}"#, env.path("nothing").display());
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for("could not open the browser: cannot find ");
    terminal.wait_for(KEYS);
    let (_, pid) = running(&env);
    assert_ne!(pid, first, "the first serve's record was left");
    let _reap = Reap(pid);
    terminal.type_keys("q");
    assert!(ends_within(pid, Duration::from_secs(5)));
}

/// A second Ctrl-C ends a slow stop at once on a terminal too, and puts the
/// terminal back first: the cursor shown, and lines buffered and echoed.
#[test]
fn a_second_ctrl_c_on_a_terminal_puts_it_back() {
    let env = Env::new();
    let run = format!(r#"{SERVE}; echo "exit=$?"; stty -a; echo "stty done""#);
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (addr, pid) = running(&env);
    let _reap = Reap(pid);
    let stalled = held(&addr, &csrf(&addr));
    // Sent until it lands, as two that come closer than serve takes the
    // first are one signal. The stalled write holds the stop for its 5 s
    // grace, so an exit before then is a forced one.
    let started = Instant::now();
    while !ends_within(pid, Duration::from_millis(300)) {
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "Ctrl-C did not end serve"
        );
        if let Some(pid) = rustix::process::Pid::from_raw(pid) {
            let _ = rustix::process::kill_process(pid, Signal::INT);
        }
    }
    terminal.wait_for("\x1b[?25h");
    terminal.wait_for("exit=130");
    let modes = terminal.wait_for("stty done");
    let modes: Vec<&str> = modes.split_whitespace().collect();
    for mode in ["icanon", "echo"] {
        assert!(modes.contains(&mode), "{mode} is off: {modes:?}");
    }
    drop(stalled);
}

/// A second Ctrl-C that comes the moment the first has begun the stop ends
/// serve at once: it is counted from the first, not from when serve gets
/// round to listening for it.
#[test]
fn a_second_ctrl_c_straight_after_the_first_ends_serve() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let stalled = held(&serve.addr, &serve.csrf());
    serve.raise(Signal::INT);
    // The stop has begun once serve takes no new connection, and the second
    // comes straight after. (Two that come closer than serve can take the
    // first are one signal: the system delivers a pending signal once.)
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::net::TcpStream::connect(&serve.addr) {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => break,
            // One that came as the listener closed.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            Err(error) => panic!("{error}"),
            Ok(_) => assert!(Instant::now() < deadline, "Ctrl-C did not stop serve"),
        }
    }
    let second = Instant::now();
    serve.raise(Signal::INT);
    let out = serve.exit_within(Duration::from_secs(4));
    assert_eq!(out.code, 130, "the second Ctrl-C was lost: {out:?}");
    assert!(
        second.elapsed() < Duration::from_secs(2),
        "{:?}",
        second.elapsed()
    );
    drop(stalled);
}

/// serve closes as expired a briefing picked up longer ago than
/// briefing-close-after once it starts, and logs it; it leaves a younger one
/// (AG-19).
#[test]
fn serve_closes_a_stale_briefing_when_it_starts() {
    let env = Env::new();
    let open = |title| {
        let id = env.open(&[
            "--kind",
            "briefing",
            "--urgency",
            "today",
            "--title",
            title,
            "--body",
            "Done.",
        ]);
        env.write(&id, "human", "answer", r#"{"briefing": "accept"}"#);
        id
    };
    let old = open("Old");
    let at = format!(
        r#"{{"picked_up_at": "{}"}}"#,
        common::from_now(-8 * 24 * 3600)
    );
    env.write(&old, "agent", "pickup", &at);
    let young = open("Young");
    env.write(&young, "agent", "pickup", "{}");

    let serve = env.serve(&["--as", "Ryan"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while env.case(&old).state() != cases_core::State::Closed {
        assert!(Instant::now() < deadline, "serve did not close {old}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let log = serve.stop().stderr;
    assert!(
        log.contains(&format!(
            "closed briefing {old}: closed after 7 days picked up"
        )),
        "{log}"
    );
    let case = env.case(&old);
    assert_eq!(
        case.events().last().unwrap().author,
        cases_core::Author::Agent
    );
    assert!(case.close().unwrap().expired);
    assert_eq!(env.case(&young).state(), cases_core::State::PickedUp);
}

/// A stop waits for a pass over the briefings that is under way, longer
/// than the 1 s it gives what is left on blocking threads, so that the
/// close the pass is making is made and logged (AG-19).
#[test]
fn a_stop_waits_for_the_briefing_pass_under_way() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "briefing",
        "--urgency",
        "today",
        "--title",
        "Old",
        "--body",
        "Done.",
    ]);
    env.write(&id, "human", "answer", r#"{"briefing": "accept"}"#);
    let at = format!(
        r#"{{"picked_up_at": "{}"}}"#,
        common::from_now(-8 * 24 * 3600)
    );
    env.write(&id, "agent", "pickup", &at);

    // Holding the store's write lock holds the pass in its close, for up to
    // the store's 5 s wait on the lock.
    let lock = rusqlite::Connection::open(&env.store).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let serve = env.serve(&[]);
    // A stop that came before the pass began could skip it.
    store_held(&serve.addr);
    let pid = i32::try_from(serve.pid()).unwrap();
    serve.raise(Signal::TERM);
    assert!(
        !ends_within(pid, Duration::from_millis(1_500)),
        "serve left the pass under way"
    );
    lock.execute_batch("COMMIT").unwrap();
    let out = serve.exit_within(Duration::from_secs(10));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr
            .contains(&format!("closed briefing {id}: closed after 7 days")),
        "{out:?}"
    );
    assert_eq!(env.case(&id).state(), cases_core::State::Closed);
}

/// Waits until a write by serve at `addr` is held on the store's lock: a
/// read of the store then queues behind it, and takes no answer. Reads made
/// before the write begins are answered, and made again.
fn store_held(addr: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut stream = common::serve::connect(addr);
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        write!(
            stream,
            "GET /v1/cases HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = Vec::new();
        match std::io::Read::read_to_end(&mut stream, &mut text) {
            Err(error)
                if text.is_empty()
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
            {
                return;
            }
            read => assert!(read.is_ok(), "{read:?}"),
        }
        assert!(Instant::now() < deadline, "no write held the store");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// With briefing-close-after off, serve leaves the briefings alone; with a
/// value that is not a duration, it does not start.
#[test]
fn serve_leaves_briefings_when_the_setting_is_off() {
    let env = Env::new();
    let id = env.open(&[
        "--kind",
        "briefing",
        "--urgency",
        "today",
        "--title",
        "Old",
        "--body",
        "Done.",
    ]);
    env.write(&id, "human", "answer", r#"{"briefing": "accept"}"#);
    let at = format!(
        r#"{{"picked_up_at": "{}"}}"#,
        common::from_now(-8 * 24 * 3600)
    );
    env.write(&id, "agent", "pickup", &at);

    env.config("briefing-close-after = \"not a duration\"\n");
    let out = env.cases(&["serve", "--listen", "127.0.0.1:0"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("briefing-close-after"), "{out:?}");

    env.config("briefing-close-after = \"off\"\n");
    let serve = env.serve(&[]);
    // The page's read comes after serve's first pass would have.
    assert_eq!(serve.get("/v1/cases").status, 200);
    std::thread::sleep(Duration::from_millis(200));
    serve.stop();
    assert_eq!(env.case(&id).state(), cases_core::State::PickedUp);
}

/// A store-tokens-file that is only white space counts as not given: serve
/// starts, with no tokens, rather than reading a file named by nothing.
#[test]
fn a_blank_store_tokens_file_is_not_read() {
    let env = Env::new();
    env.config("store-tokens-file = \"  \"\n");
    let serve = env.serve(&[]);
    assert_eq!(serve.get("/v1").status, 200);
    assert_eq!(serve.stop().code, 0);
}

/// Against a remote store serve starts nothing, says where its inbox is,
/// and on a terminal opens it, unless `--no-open` says not to.
#[cfg(feature = "remote")]
#[test]
fn on_a_terminal_a_remote_stores_inbox_is_opened_unless_no_open() {
    let env = Env::new();
    let bin = env.path("bin");
    let opened = env.path("opened.log");
    for opener in ["open", "xdg-open"] {
        write_file(
            &bin.join(opener),
            &format!("#!/bin/sh\necho \"$@\" >> {}\n", opened.display()),
        );
        fs::set_permissions(bin.join(opener), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let remote = format!(
        r#"PATH="{}" CASES_SERVER=http://127.0.0.1:9 CASES_TOKEN=t "$CASES" serve"#,
        bin.display()
    );
    let mut terminal = Terminal::start(
        &env,
        &format!(r#"{remote}; code=$?; sleep 1; echo "exit=$code""#),
    );
    terminal.wait_for("http://127.0.0.1:9/");
    terminal.wait_for("exit=0");
    // The opener runs on after serve has ended; the shell stays up a
    // moment so that the terminal's hangup does not end it first.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(&opened).is_ok_and(|text| text == "http://127.0.0.1:9/\n") {
        assert!(Instant::now() < deadline, "the browser was not opened");
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut terminal = Terminal::start(
        &env,
        &format!(r#"{remote} --no-open; code=$?; sleep 1; echo "exit=$code""#),
    );
    terminal.wait_for("http://127.0.0.1:9/");
    terminal.wait_for("exit=0");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        "http://127.0.0.1:9/\n",
        "--no-open opened the browser"
    );
}

/// On a terminal whose stderr goes elsewhere, the log still goes to stderr
/// as well as to the status screen.
#[test]
fn on_a_terminal_with_stderr_redirected_the_log_goes_to_stderr() {
    let env = Env::new();
    env.config("allow-hosts = [\"inbox.example\"]\n");
    let err = env.path("err.log");
    let run = format!(
        r#"XDG_CONFIG_HOME="{}" "$CASES" --store "$STORE" serve --listen 127.0.0.1:0 --no-open 2>"{}""#,
        env.path("config").display(),
        err.display()
    );
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (_, pid) = running(&env);
    let _reap = Reap(pid);
    terminal.type_keys("q");
    assert!(ends_within(pid, Duration::from_secs(5)));
    let log = fs::read_to_string(&err).unwrap();
    assert!(
        log.contains("also serving https://inbox.example/ through a proxy"),
        "{log}"
    );
}

/// Opens a message on serve at `addr` whose body serve is reading, which it
/// says with `100 Continue`, and never gets: it holds a stop for its 5 s
/// grace. Gives the connection once serve has taken it up.
fn held(addr: &str, token: &str) -> std::net::TcpStream {
    let mut stream = common::serve::connect(addr);
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "POST /v1/cases HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         X-Cases-CSRF: {token}\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n"
    )
    .unwrap();
    let mut continued = [0; 25];
    std::io::Read::read_exact(&mut stream, &mut continued).unwrap();
    assert!(
        continued.starts_with(b"HTTP/1.1 100 Continue"),
        "{:?}",
        String::from_utf8_lossy(&continued)
    );
    stream
}

/// Waits until serve at `addr` takes no new connection: its stop has
/// begun.
fn stop_begun(addr: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::net::TcpStream::connect(addr) {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => return,
            // One that came as the listener closed.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            Err(error) => panic!("{error}"),
            Ok(_) => {}
        }
        assert!(Instant::now() < deadline, "the stop did not begin");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The number of the `write` system call, as `/proc` gives it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const WRITE: u32 = 1;
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const WRITE: u32 = 64;

/// Whether a thread of `pid` is held in a write to stdout within `within`,
/// as on a terminal whose output is stopped: `/proc` gives the call a
/// thread is in, by its number and its first argument (the fd).
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn held_writing(pid: i32, within: Duration) -> bool {
    let call = format!("{WRITE} 0x1 ");
    let deadline = Instant::now() + within;
    loop {
        let tasks = fs::read_dir(format!("/proc/{pid}/task")).unwrap();
        if tasks.flatten().any(|task| {
            fs::read_to_string(task.path().join("syscall"))
                .is_ok_and(|text| text.starts_with(&call))
        }) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A second SIGTERM during a slow stop ends serve at once, with the status
/// a shell gives a process SIGTERM ended, 143, as a service manager sends
/// it: not Ctrl-C's 130.
#[test]
fn a_second_sigterm_ends_a_slow_stop_with_143() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let token = serve.csrf();
    let stalled = held(&serve.addr, &token);
    serve.raise(Signal::TERM);
    stop_begun(&serve.addr);
    let second = Instant::now();
    serve.raise(Signal::TERM);
    let out = serve.exit_within(Duration::from_secs(4));
    assert_eq!(out.code, 143, "{out:?}");
    assert!(
        second.elapsed() < Duration::from_secs(2),
        "{:?}",
        second.elapsed()
    );
    drop(stalled);
}

/// A stop forced by a second signal removes serve's record, as one that
/// runs its course does (SV-02).
#[test]
fn a_forced_stop_removes_the_record() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let file = env.instance_file();
    assert!(file.exists(), "serve recorded nothing");
    let stalled = held(&serve.addr, &serve.csrf());
    serve.raise(Signal::INT);
    stop_begun(&serve.addr);
    serve.raise(Signal::INT);
    let out = serve.exit_within(Duration::from_secs(4));
    assert_eq!(out.code, 130, "{out:?}");
    assert!(!file.exists(), "the record outlived serve");
    drop(stalled);
}

/// On a terminal, the status screen's last frame comes after the stop, so
/// that it shows what the stop logged: the connections it closed, which
/// the screen alone shows when stderr is the terminal too.
#[test]
fn on_a_terminal_the_last_frame_shows_what_the_stop_closed() {
    let env = Env::new();
    let run = format!(r#"{SERVE}; echo "exit=$?""#);
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (addr, pid) = running(&env);
    let reap = Reap(pid);
    let stalled = held(&addr, &csrf(&addr));
    terminal.type_keys("q");
    // The grace is 5 s, within wait_for's 10.
    terminal.wait_for("closed 1 connections still open 5 s after the stop");
    terminal.wait_for("exit=0");
    // It has ended: no SIGKILL for whatever has its pid next.
    std::mem::forget(reap);
    drop(stalled);
}

/// A second Ctrl-C ends serve on a terminal whose output is stopped
/// (Ctrl-S), where the write that puts the terminal back cannot finish: it
/// exits a moment later whatever that write does. On Linux, where `/proc`
/// shows that serve's output is held.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn a_second_ctrl_c_ends_serve_on_a_stopped_terminal() {
    let env = Env::new();
    let run = format!(r#"{SERVE}; echo "exit=$?""#);
    let mut terminal = Terminal::start(&env, &run);
    terminal.wait_for(KEYS);
    let (addr, pid) = running(&env);
    let reap = Reap(pid);
    let token = csrf(&addr);
    terminal.type_keys("\x13");
    // A load of the page is counted and logged on the screen, so serve draws
    // a frame within a tick, which the stopped terminal holds. Loaded again
    // if a frame went out before the Ctrl-S was in force.
    let mut loads = 0;
    loop {
        let mut page = common::serve::connect(&addr);
        write!(
            page,
            "GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        std::io::Read::read_to_end(&mut page, &mut Vec::new()).unwrap();
        loads += 1;
        if held_writing(pid, Duration::from_secs(2)) {
            break;
        }
        assert!(loads < 3, "no write of {pid} was held");
    }
    let stalled = held(&addr, &token);
    let signal = |pid: i32| {
        if let Some(pid) = rustix::process::Pid::from_raw(pid) {
            let _ = rustix::process::kill_process(pid, Signal::INT);
        }
    };
    signal(pid);
    stop_begun(&addr);
    signal(pid);
    // Before the 5 s grace is out: the exit is the forced one.
    assert!(
        ends_within(pid, Duration::from_secs(3)),
        "serve did not end on a stopped terminal"
    );
    // The held write puts the terminal back, and does not hold this up.
    assert!(
        !env.instance_file().exists(),
        "the record outlived a forced stop"
    );
    // Ctrl-Q lets the terminal show what was held.
    terminal.type_keys("\x11");
    terminal.wait_for("exit=130");
    // It has ended: no SIGKILL for whatever has its pid next.
    std::mem::forget(reap);
    drop(stalled);
}
