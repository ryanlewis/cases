//! The pipe a forwarded store is reached by, against children that are
//! small shell scripts: each answers what a far side would, or fails as one
//! can. The store routes themselves are tested against `cases dial-stdio`
//! in the cli's tests, which run every vector through it.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Append, Store, StoreError, Writer};
use tempfile::TempDir;

use crate::client::Limits;
use crate::pipe::why_closed;
use crate::tests::quick;
use crate::{Client, DIAL_STORE, Dial, Error, RemoteStore};

/// A shell script that logs `start` to `$LOG` and then answers each
/// request that has no body with `$ANSWER`, until its stdin ends.
const SERVE: &str = r#"echo start >> "$LOG"
cr=$(printf '\r')
while IFS= read -r line || [ -n "$line" ]; do
    echo "$line" >> "$LOG"
    case "$line" in
        "$cr"|"") printf 'HTTP/1.1 200 OK\r\nContent-Length: %s\r\n\r\n%s' "${#ANSWER}" "$ANSWER";;
    esac
done
"#;

/// A directory for a test's log, and the store over `script` run by `sh`,
/// with `LOG` set to the log and `vars` set too.
fn over(script: &str, vars: &[(&str, &str)], limits: Limits) -> (TempDir, RemoteStore) {
    let dir = tempfile::tempdir().unwrap();
    let mut dial = Dial::new("sh", "ssh://far")
        .args(["-c", script])
        .env("LOG", dir.path().join("log"));
    for (key, value) in vars {
        dial = dial.env(*key, *value);
    }
    let store = Client::dial_with_limits(dial, limits)
        .store(DIAL_STORE)
        .unwrap();
    (dir, store)
}

/// The lines of the log in `dir`.
fn log(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("log"))
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim_end_matches('\r').to_owned())
        .collect()
}

/// How many children were started, by the log in `dir`.
fn starts(dir: &Path) -> usize {
    log(dir).iter().filter(|line| *line == "start").count()
}

fn open() -> Append {
    Append {
        author: "agent".into(),
        event: "open".into(),
        data: r#"{"kind":"fyi","urgency":"today","title":"T"}"#.into(),
    }
}

#[test]
fn requests_go_over_one_child_while_it_lasts() {
    let (dir, store) = over(SERVE, &[("ANSWER", r#"{"version":"7"}"#)], quick());
    assert_eq!(store.version().unwrap().as_str(), "7");
    assert_eq!(store.version().unwrap().as_str(), "7");
    assert_eq!(starts(dir.path()), 1);
    let lines = log(dir.path());
    assert_eq!(
        lines[1],
        format!("GET /v1/stores/{DIAL_STORE}/store/changes HTTP/1.1")
    );
    // No token goes over a pipe.
    assert!(
        !lines
            .iter()
            .any(|l| l.to_ascii_lowercase().starts_with("authorization")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l == &format!("user-agent: cases/{}", env!("CARGO_PKG_VERSION"))),
        "{lines:?}"
    );
}

/// The store over `script`, which answers `{"version":"7"}`, read once,
/// then `after` with the test's directory, and then written to once: a
/// write is not sent again, so it fails if the connection the read left is
/// used when it should not be. Gives how many children were started.
fn read_then_write(script: &str, after: impl FnOnce(&Path)) -> usize {
    let (dir, store) = over(script, &[("ANSWER", r#"{"version":"7"}"#)], quick());
    store.version().unwrap();
    after(dir.path());
    // The answer is an object, which is all an archive reads.
    store
        .archive(&cases_core::Key::new(1).unwrap(), Some(1))
        .unwrap();
    starts(dir.path())
}

/// Waits up to 5 s for the child whose pid the script wrote to `$LOG.pid`
/// in `dir` to exit, and leaves it for the pipe to reap: the test's
/// process is its parent, so it can look without taking its status. A
/// child the pipe reaped already, when it looked at the connection as it
/// went back to the pool, had exited.
fn child_exited(dir: &Path) {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = Pid::from_raw(log_pid(dir)).expect("a pid");
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG;
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match waitid(WaitId::Pid(pid), options) {
            Ok(Some(_)) | Err(rustix::io::Errno::CHILD) => return,
            Ok(None) => assert!(Instant::now() < until, "the child {pid:?} did not exit"),
            Err(error) => panic!("waiting for the child {pid:?}: {error}"),
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// The pid in `$LOG.pid` in `dir`, which the script writes before it
/// answers.
fn log_pid(dir: &Path) -> i32 {
    fs::read_to_string(dir.join("log.pid"))
        .expect("the child's pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// How [`SERVE`] answers.
const ANSWERS: &str =
    r#"printf 'HTTP/1.1 200 OK\r\nContent-Length: %s\r\n\r\n%s' "${#ANSWER}" "$ANSWER";;"#;

#[test]
fn a_child_that_lasts_takes_the_next_request() {
    assert_eq!(read_then_write(SERVE, |_| {}), 1);
}

#[test]
fn a_child_that_exits_after_its_answer_is_not_used_again() {
    // The first child exits after its answer, and says who it is first;
    // the next lasts, so that it reads all of the write before it goes.
    let once = format!("[ -e \"$LOG.once\" ] || echo $$ > \"$LOG.pid\"\n{SERVE}").replace(
        "\"$ANSWER\";;",
        "\"$ANSWER\"; [ -e \"$LOG.once\" ] || { : > \"$LOG.once\"; exit 0; };;",
    );
    assert!(once.contains("exit 0; };;"), "{once}");
    assert_eq!(read_then_write(&once, child_exited), 2);
}

#[test]
fn a_child_that_sends_more_than_its_answer_is_not_used_again() {
    // In the same write as the answer, and in one after it.
    for more in [
        r#"printf 'HTTP/1.1 200 OK\r\nContent-Length: %s\r\n\r\n%sx' "${#ANSWER}" "$ANSWER";;"#,
        r#"printf 'HTTP/1.1 200 OK\r\nContent-Length: %s\r\n\r\n%s' "${#ANSWER}" "$ANSWER"; sleep 0.05; printf x;;"#,
    ] {
        let script = SERVE.replace(ANSWERS, more);
        assert_ne!(script, SERVE);
        // Give the bytes time to reach the pipe's reader: a child that
        // lasts gives no other sign that they have.
        let sent = |_: &Path| thread::sleep(Duration::from_millis(200));
        assert_eq!(read_then_write(&script, sent), 2, "{more}");
    }
}

#[test]
fn a_child_that_closes_its_stdin_says_why_when_written_to() {
    let (_dir, store) = over(
        "exec 0<&-; sleep 0.2; echo 'gave up' >&2; exit 3",
        &[],
        quick(),
    );
    let written = store.archive(&cases_core::Key::new(1).unwrap(), Some(1));
    match written {
        Err(StoreError::Store(Error::Transport { reason, .. })) => {
            assert_eq!(reason, "the connection closed (exit 3): gave up");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_answer_larger_than_the_buffer_is_read_whole() {
    let pad = " ".repeat(300_000);
    let answer = format!(r#"{{"version":"1","cases":[]{pad}}}"#);
    let mut limits = quick();
    limits.answer = 1024 * 1024;
    // Linux takes no environment variable longer than 128 KiB, so the
    // answer goes to the child in a file.
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), &answer).unwrap();
    let script = format!("ANSWER=$(cat \"$ANSWER_FILE\")\n{SERVE}");
    let (_dir, store) = over(
        &script,
        &[("ANSWER_FILE", file.path().to_str().unwrap())],
        limits,
    );
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn a_listen_is_sent_with_its_wait_and_doing_line() {
    let (dir, store) = over(SERVE, &[("ANSWER", "{}")], quick());
    store
        .listen(
            "w1",
            "lead",
            &["a".into()],
            &["fyi".into()],
            Some("reading"),
        )
        .unwrap();
    store.listen("w1", "lead", &[], &[], None).unwrap();
    // The child logs the last body when its stdin ends.
    drop(store);
    let lines = log(dir.path());
    let bodies = lines.join("\n");
    // A body has no line end, so the log runs it into the next request.
    let path = format!("POST /v1/stores/{DIAL_STORE}/store/listen HTTP/1.1");
    assert_eq!(bodies.matches(&path).count(), 2, "{lines:?}");
    assert!(
        bodies.contains(
            r#"{"wait":"w1","worker":"lead","labels":["a"],"kinds":["fyi"],"doing":"reading"}"#
        ),
        "{bodies}"
    );
    assert!(
        bodies.contains(r#"{"wait":"w1","worker":"lead","labels":[],"kinds":[],"doing":null}"#),
        "{bodies}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.eq_ignore_ascii_case("content-type: application/json")),
        "{lines:?}"
    );
}

#[test]
fn a_listen_that_is_not_one_is_refused_before_a_child_starts() {
    let (dir, store) = over(SERVE, &[("ANSWER", "{}")], quick());
    for (worker, doing) in [("  ", None), ("lead", Some("a\nb"))] {
        assert!(matches!(
            store.listen("w1", worker, &[], &[], doing),
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(starts(dir.path()), 0);
}

/// A child that writes `stderr` and exits with `code`, logging each start.
fn failing(stderr: &str, code: i32) -> String {
    format!("echo start >> \"$LOG\"; echo '{stderr}' >&2; exit {code}")
}

#[test]
fn the_last_of_stderr_is_read_after_the_child_exits() {
    // The child exits at once, and what it started, which holds only its
    // stderr, writes to it a little later.
    let (_dir, store) = over(
        "(sleep 0.05; echo 'late' >&2) <&- >/dev/null & exit 127",
        &[],
        quick(),
    );
    assert_eq!(
        store.version().unwrap_err(),
        Error::Dial {
            dial: "ssh://far".into(),
            reason: "there is no cases on the far side's PATH (exit 127): late".into()
        }
    );
}

#[test]
fn a_far_side_with_no_cases_fails_at_once_and_is_not_tried_again() {
    let (dir, store) = over(
        &failing("bash: cases: command not found", 127),
        &[],
        quick(),
    );
    let error = store.version().unwrap_err();
    assert_eq!(
        error,
        Error::Dial {
            dial: "ssh://far".into(),
            reason: "there is no cases on the far side's PATH (exit 127): bash: cases: command \
                     not found"
                .into()
        }
    );
    assert!(!error.is_transient());
    assert_eq!(starts(dir.path()), 1);
}

#[test]
fn a_far_side_too_old_for_dial_stdio_fails_at_once() {
    let (dir, store) = over(
        &failing("error: unrecognized subcommand 'dial-stdio'", 2),
        &[],
        quick(),
    );
    assert_eq!(
        store.version().unwrap_err(),
        Error::Dial {
            dial: "ssh://far".into(),
            reason: "the cases on the far side has no dial-stdio: upgrade it there".into()
        }
    );
    assert_eq!(starts(dir.path()), 1);
}

#[test]
fn a_dial_with_no_program_fails_at_once() {
    let dial = Dial::new("/nowhere/ssh", "ssh://far");
    let store = Client::dial_with_limits(dial, quick())
        .store(DIAL_STORE)
        .unwrap();
    match store.version().unwrap_err() {
        Error::Dial { dial, reason } => {
            assert_eq!(dial, "ssh://far");
            assert!(
                reason.starts_with("could not run /nowhere/ssh: "),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_dial_not_named_ssh_is_not_said_to_be_ssh() {
    let script = failing("Permission denied", 255);
    let dial = Dial::new("sh", "exec:box")
        .args(["-c", script.as_str()])
        .env("LOG", "/dev/null");
    let store = Client::dial_with_limits(dial, quick())
        .store(DIAL_STORE)
        .unwrap();
    assert_eq!(
        store.version().unwrap_err(),
        Error::Transport {
            server: "exec:box".into(),
            reason: "the connection closed (exit 255): Permission denied".into()
        }
    );
}

#[test]
fn a_lost_connection_is_tried_again_for_a_read_and_not_a_write() {
    let (dir, store) = over(&failing("Connection reset by peer", 255), &[], quick());
    let error = store.version().unwrap_err();
    assert_eq!(
        error,
        Error::Transport {
            server: "ssh://far".into(),
            reason: "ssh failed (exit 255): Connection reset by peer".into()
        }
    );
    assert!(error.is_transient());
    assert_eq!(starts(dir.path()), 3);
    let written = store.create(&open(), &Writer::new(None));
    assert!(
        matches!(written, Err(StoreError::Store(Error::Transport { .. }))),
        "{written:?}"
    );
    assert_eq!(starts(dir.path()), 4);
}

#[test]
fn a_read_that_was_lost_once_is_answered_by_a_new_child() {
    // The first child closes on the request; the next answers.
    let first_fails =
        format!("if [ ! -e \"$LOG\" ]; then echo start >> \"$LOG\"; exit 1; fi\n{SERVE}");
    let (dir, store) = over(&first_fails, &[("ANSWER", r#"{"version":"7"}"#)], quick());
    assert_eq!(store.version().unwrap().as_str(), "7");
    assert_eq!(starts(dir.path()), 2);
}

#[test]
fn a_child_that_does_not_answer_times_out() {
    let mut limits = quick();
    limits.request = Duration::from_millis(300);
    let (dir, store) = over("echo start >> \"$LOG\"; cat >/dev/null", &[], limits);
    let start = Instant::now();
    let error = store.version().unwrap_err();
    assert!(matches!(error, Error::Transport { .. }), "{error:?}");
    assert_eq!(starts(dir.path()), 3);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_child_that_does_not_read_a_large_request_times_out() {
    // A request larger than the pipe (64 KiB on Linux) fills it, and the
    // child, which never reads, would leave the write blocked until it
    // exits, at 3 s.
    let mut limits = quick();
    limits.request = Duration::from_millis(300);
    let (_dir, store) = over("sleep 3", &[], limits);
    let large = Append {
        data: format!(
            r#"{{"kind":"fyi","urgency":"today","title":"T","body":"{}"}}"#,
            "x".repeat(200 * 1024)
        ),
        ..open()
    };
    let start = Instant::now();
    let written = store.create(&large, &Writer::new(None));
    assert!(
        matches!(written, Err(StoreError::Store(Error::Transport { .. }))),
        "{written:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_request_read_slowly_fails_at_its_timeout_not_each_part_s() {
    // The child reads 64 KiB each 0.25 s. The request, of about 600 KiB, is
    // written in parts of the 128 KiB buffer, each in about 0.5 s: within a
    // timeout of 1 s each, but the request as a whole is not.
    let mut limits = quick();
    limits.request = Duration::from_secs(1);
    let slow = "while dd bs=65536 count=1 of=/dev/null 2>/dev/null; do sleep 0.25; done";
    let (_dir, store) = over(slow, &[], limits);
    let large = Append {
        data: format!(
            r#"{{"kind":"fyi","urgency":"today","title":"T","body":"{}"}}"#,
            "x".repeat(600 * 1024)
        ),
        ..open()
    };
    let start = Instant::now();
    let written = store.create(&large, &Writer::new(None));
    assert!(
        matches!(written, Err(StoreError::Store(Error::Transport { .. }))),
        "{written:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(1800),
        "{:?}",
        start.elapsed()
    );
}

/// The start of a script that tells, in `$LOG.pid` and `$LOG.stdin`, its
/// pid and the pipe that is its stdin.
#[cfg(target_os = "linux")]
const TELLS: &str = "echo $$ > \"$LOG.pid\"; readlink /proc/$$/fd/0 > \"$LOG.stdin\"\n";

/// What the script told in the file `name` in `dir`, waited for up to 5 s.
#[cfg(target_os = "linux")]
fn told(dir: &Path, name: &str) -> String {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let said = fs::read_to_string(dir.join(name)).unwrap_or_default();
        if said.ends_with('\n') {
            return said.trim().to_owned();
        }
        assert!(Instant::now() < until, "the script did not tell {name}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits up to 5 s for the child that told in `dir` to be reaped, and for
/// no file of this process to be its stdin: the thread that wrote to it
/// has ended, as it alone holds it.
#[cfg(target_os = "linux")]
fn let_go(dir: &Path) {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let stdin = told(dir, "log.stdin");
    let pid = Pid::from_raw(told(dir, "log.pid").parse().unwrap()).unwrap();
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG;
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let held = fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .any(|fd| fs::read_link(fd.path()).is_ok_and(|link| link.as_os_str() == &*stdin));
        let reaped = matches!(
            waitid(WaitId::Pid(pid), options),
            Err(rustix::io::Errno::CHILD)
        );
        if !held && reaped {
            return;
        }
        assert!(
            Instant::now() < until,
            "the child's stdin, {stdin}, held: {held}; the child {pid:?} reaped: {reaped}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// A store over `TELLS` and then `script`, with requests of `request`
/// each, and a create of a 200 KiB body over it.
#[cfg(target_os = "linux")]
fn large_create(script: &str, request: Duration) -> (TempDir, Result<(), StoreError<Error>>) {
    let mut limits = quick();
    limits.request = request;
    let (dir, store) = over(&format!("{TELLS}{script}"), &[], limits);
    let large = Append {
        data: format!(
            r#"{{"kind":"fyi","urgency":"today","title":"T","body":"{}"}}"#,
            "x".repeat(200 * 1024)
        ),
        ..open()
    };
    let created = store.create(&large, &Writer::new(None)).map(|_| ());
    (dir, created)
}

#[cfg(target_os = "linux")]
#[test]
fn a_connection_dropped_mid_write_lets_go_of_its_thread_and_child() {
    let (dir, created) = large_create("sleep 3", Duration::from_millis(300));
    assert!(
        matches!(created, Err(StoreError::Store(Error::Transport { .. }))),
        "{created:?}"
    );
    let_go(dir.path());
}

#[cfg(target_os = "linux")]
#[test]
fn a_child_that_ends_mid_write_says_why_and_is_let_go() {
    for (script, why) in [
        (
            "sleep 0.3; echo 'gave up' >&2; exit 3",
            "the connection closed (exit 3): gave up",
        ),
        ("sleep 0.3; kill -9 $$", "the connection closed"),
    ] {
        let (dir, created) = large_create(script, Duration::from_secs(5));
        match created {
            Err(StoreError::Store(Error::Transport { reason, .. })) => {
                assert_eq!(reason, why, "{script}");
            }
            other => panic!("{script}: {other:?}"),
        }
        let_go(dir.path());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_idle_connection_dropped_lets_go_of_its_thread_and_child() {
    let (dir, store) = over(
        &format!("{TELLS}{SERVE}"),
        &[("ANSWER", r#"{"version":"7"}"#)],
        quick(),
    );
    store.version().unwrap();
    drop(store);
    let_go(dir.path());
}

#[test]
fn a_dropped_connection_kills_a_child_that_does_not_exit() {
    let pid_file: PathBuf;
    let start;
    {
        let stubborn = SERVE
            .replace(
                "\"$cr\"|\"\") printf",
                "\"$cr\"|\"\") echo $$ > \"$LOG.pid\"; printf",
            )
            .replace("\"$ANSWER\";;", "\"$ANSWER\"; exec sleep 30;;");
        let (dir, store) = over(&stubborn, &[("ANSWER", r#"{"version":"7"}"#)], quick());
        store.version().unwrap();
        pid_file = dir.path().join("log.pid");
        let pid = fs::read_to_string(&pid_file).unwrap();
        start = Instant::now();
        drop(store);
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the child {pid} is still running");
        drop(dir);
    }
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_dropped_connection_kills_what_its_child_started_too() {
    // An `exec:` forward's shell that does not exec its command, as for
    // `a | b` or `a; b`, has children of its own; killing the shell alone
    // would leave them running.
    let lingering = format!("{SERVE}sleep 30 &\necho $! > \"$LOG.pid\"\nwait\n");
    let (dir, store) = over(&lingering, &[("ANSWER", r#"{"version":"7"}"#)], quick());
    store.version().unwrap();
    drop(store);
    let pid = fs::read_to_string(dir.path().join("log.pid")).unwrap();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success()
    };
    // Once killed, it is reaped by whoever it was left to.
    let until = Instant::now() + Duration::from_secs(5);
    while alive() && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "what the child started, {pid}, is still running");
}

#[test]
fn a_dial_is_its_program_arguments_environment_and_name() {
    let dial = Dial::new("ssh", "ssh://devbox")
        .args(["-p", "22"])
        .args(["devbox"])
        .env("A", "1");
    assert_eq!(dial.program(), "ssh");
    assert_eq!(dial.arguments(), ["-p", "22", "devbox"]);
    assert_eq!(dial.to_string(), "ssh://devbox");
    let (_dir, store) = over(SERVE, &[("ANSWER", r#"{"version":"x"}"#)], quick());
    assert!(store.client().dialled().is_some());
    let hosted = Client::new(
        "https://cases.sh".parse().unwrap(),
        format!("csu_{}", "a".repeat(36)).parse().unwrap(),
    );
    assert!(hosted.dialled().is_none());
    // The child gets the dial's environment.
    let (_dir, store) = over(
        &SERVE.replace("ANSWER", "GIVEN"),
        &[("GIVEN", r#"{"version":"from env"}"#)],
        quick(),
    );
    assert_eq!(store.version().unwrap().as_str(), "from env");
}

#[test]
fn why_a_child_closed_is_its_exit_and_last_line() {
    let kind = |code, stderr| why_closed(true, code, stderr).kind();
    let text = |code, stderr| why_closed(true, code, stderr).to_string();
    assert_eq!(
        text(Some(127), "noise\n  sh: cases: not found  \n\n"),
        "there is no cases on the far side's PATH (exit 127): sh: cases: not found"
    );
    assert_eq!(kind(Some(127), ""), std::io::ErrorKind::NotFound);
    assert_eq!(
        text(Some(127), ""),
        "there is no cases on the far side's PATH (exit 127)"
    );
    assert_eq!(
        text(Some(2), "error: unrecognized subcommand 'dial-stdio'\n"),
        "the cases on the far side has no dial-stdio: upgrade it there"
    );
    assert_eq!(
        kind(Some(2), "error: unrecognized subcommand 'dial-stdio'"),
        std::io::ErrorKind::Unsupported
    );
    // Another subcommand is not this one.
    assert_eq!(
        text(Some(2), "error: unrecognized subcommand 'other'"),
        "the connection closed (exit 2): error: unrecognized subcommand 'other'"
    );
    assert_eq!(
        text(Some(1), "dial-stdio failed"),
        "the connection closed (exit 1): dial-stdio failed"
    );
    assert_eq!(
        text(Some(255), "ssh: no route\n"),
        "ssh failed (exit 255): ssh: no route"
    );
    assert_eq!(kind(Some(255), ""), std::io::ErrorKind::UnexpectedEof);
    // A prompt ssh could not show, in batch mode, says why it was not.
    let batch = "; cases runs ssh with BatchMode=yes, so it cannot ask for a password, a \
                 passphrase or a new host key: use a key in an ssh agent, and accept the \
                 host's key with a plain ssh to it first";
    assert_eq!(
        text(
            Some(255),
            "ryan@devbox: Permission denied (publickey,password).\n"
        ),
        format!(
            "ssh failed (exit 255): ryan@devbox: Permission denied (publickey,password).{batch}"
        )
    );
    assert_eq!(
        text(Some(255), "Host key verification failed.\n"),
        format!("ssh failed (exit 255): Host key verification failed.{batch}")
    );
    assert_eq!(
        kind(Some(255), "Permission denied"),
        std::io::ErrorKind::UnexpectedEof
    );
    assert_eq!(text(None, ""), "the connection closed");
    assert_eq!(kind(None, ""), std::io::ErrorKind::UnexpectedEof);
    assert_eq!(text(Some(0), "   \n"), "the connection closed (exit 0)");
}

#[test]
fn a_command_that_is_not_ssh_is_not_said_to_be() {
    let kind = |code, stderr| why_closed(false, code, stderr).kind();
    let text = |code, stderr| why_closed(false, code, stderr).to_string();
    assert_eq!(
        text(Some(127), "sh: 1: dokcer: not found\n"),
        "a command was not found (exit 127): sh: 1: dokcer: not found; the command must run \
         cases dial-stdio on the far side"
    );
    assert_eq!(kind(Some(127), ""), std::io::ErrorKind::NotFound);
    assert_eq!(
        text(Some(255), "Permission denied\n"),
        "the connection closed (exit 255): Permission denied"
    );
    assert_eq!(kind(Some(255), ""), std::io::ErrorKind::UnexpectedEof);
    assert_eq!(
        text(Some(2), "error: unrecognized subcommand 'dial-stdio'"),
        "the cases on the far side has no dial-stdio: upgrade it there"
    );
}

/// A hold's child: it answers `$ANSWER` with `status`, and logs `end` once
/// its stdin ends.
fn holding(status: &str) -> String {
    format!("{}echo end >> \"$LOG\"\n", SERVE.replace("200 OK", status))
}

/// Waits up to 5 s for `dir`'s log to have `line`.
fn logged(dir: &Path, line: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if log(dir).iter().any(|l| l == line) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_hold_is_a_child_of_its_own_kept_until_it_is_dropped() {
    let (dir, store) = over(&holding("200 OK"), &[("ANSWER", "{}")], quick());
    let hold = store.hold(&[("worker", "lead"), ("id", "UkLWZ")]).unwrap();
    let lines = log(dir.path());
    assert_eq!(
        lines[1],
        format!("POST /v1/stores/{DIAL_STORE}/store/lock HTTP/1.1")
    );
    assert!(
        lines.contains(&"Content-Type: application/json".to_owned()),
        "{lines:?}"
    );
    // Its answer read, the child is kept: it has not ended.
    thread::sleep(Duration::from_millis(200));
    assert!(!log(dir.path()).contains(&"end".to_owned()));
    // It was given no lease: it is held while the connection lasts.
    assert_eq!(hold.left(), Ok(None));
    assert_eq!(hold.check(), Ok(()));
    drop(hold);
    assert!(logged(dir.path(), "end"), "{:?}", log(dir.path()));
    assert_eq!(starts(dir.path()), 1);
}

/// A far side that answers its first request with a lease of `$LEASE` ms,
/// 300 when it is not set, after `$DELAY` seconds when that is set, and
/// then, as `$THEN` says: `serve` answers each request so, `silent` reads
/// and never answers, `refuse` answers each with 409 `lapsed`, and `exit`
/// ends the connection.
const LEASING: &str = r#"echo start >> "$LOG"
cr=$(printf '\r')
lease="{\"lease_ms\":${LEASE:-300}}"
lapsed='{"error":{"category":"lapsed","message":"the lease ran out"}}'
n=0
while IFS= read -r line || [ -n "$line" ]; do
    echo "$line" >> "$LOG"
    case "$line" in
        "$cr"|"")
            n=$((n + 1))
            if [ "$n" -gt 1 ]; then
                case "$THEN" in
                    silent) exec cat > /dev/null;;
                    refuse) printf 'HTTP/1.1 409 Conflict\r\nContent-Length: %s\r\n\r\n%s' "${#lapsed}" "$lapsed"; continue;;
                    exit) exit 0;;
                esac
            elif [ -n "$DELAY" ]; then
                sleep "$DELAY"
            fi
            printf 'HTTP/1.1 200 OK\r\nContent-Length: %s\r\n\r\n%s' "${#lease}" "$lease"
            if [ "$n" -eq 1 ] && [ "$THEN" = exit ]; then exit 0; fi;;
    esac
done
"#;

/// How many renewals the log in `dir` has.
fn renewals(dir: &Path) -> usize {
    log(dir)
        .iter()
        .filter(|line| line.ends_with("/store/lock/renew HTTP/1.1"))
        .count()
}

/// Waits up to 3 s for `hold` to fail its check, and gives why.
fn lapses(hold: &crate::Hold) -> String {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match hold.check() {
            Err(Error::Lapsed { reason }) => return reason,
            Err(error) => panic!("{error:?}"),
            Ok(()) => assert!(Instant::now() < until, "the hold never lapsed"),
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_hold_on_a_lease_renews_it_over_its_connection() {
    let (dir, store) = over(LEASING, &[("THEN", "serve")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    // Good for two thirds of the lease, and renewed a third of the way in,
    // so it is good for a second, ten leases' time.
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        let left = hold.left().unwrap().unwrap();
        assert!(left <= Duration::from_millis(200), "{left:?}");
        thread::sleep(Duration::from_millis(50));
    }
    // A third of the way in, and not sooner: about ten in the second.
    let renewed = renewals(dir.path());
    assert!(
        (5..=12).contains(&renewed),
        "{renewed}: {:?}",
        log(dir.path())
    );
    // It asked for the lease. Its body, with no line's end, is logged with
    // the line after it.
    let lines = log(dir.path());
    assert!(
        lines.iter().any(|line| line.contains(r#""lease":true"#)),
        "{lines:?}"
    );
    assert_eq!(starts(dir.path()), 1);
}

#[test]
fn a_hold_on_a_lease_says_it_lets_go_itself_and_does_when_dropped() {
    let (dir, store) = over(LEASING, &[("THEN", "serve")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    // Let go, its check fails, and the far side was told before its
    // connection ended. Again, it does nothing.
    hold.release();
    let reason = lapses(&hold);
    assert_eq!(reason, "this wait let them go");
    hold.release();
    // It said it would, with the lock. Its body, with no line's end, is
    // logged with the line after it.
    let lines = log(dir.path());
    assert!(
        lines.iter().any(|line| line.contains(r#""release":true"#)),
        "{lines:?}"
    );
    let released = lines
        .iter()
        .position(|line| line.ends_with("/store/lock/release HTTP/1.1"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        lines[released..].contains(&"Connection: close".to_owned()),
        "{lines:?}"
    );
    drop(hold);
    let releases = log(dir.path())
        .iter()
        .filter(|line| line.ends_with("/store/lock/release HTTP/1.1"))
        .count();
    assert_eq!(releases, 1);
    // Dropped, a hold lets go too, and a far side that does not answer
    // holds the drop up no longer than a third of the lease, or, as this
    // one closes its stdout, the 2 s a child that did is given to exit.
    let (dir, store) = over(LEASING, &[("THEN", "silent")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    let start = Instant::now();
    drop(hold);
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    assert!(
        log(dir.path())
            .iter()
            .any(|line| line.ends_with("/store/lock/release HTTP/1.1")),
        "{:?}",
        log(dir.path())
    );
}

#[test]
fn a_second_release_waits_for_the_first_to_let_go() {
    // A wait that ends lets go as it drops its hold, and a signal then lets
    // go too, before it ends the process: that one returns only once the
    // first has let go, or the process would end before the release was
    // sent, and the far side keep the locks until the lease runs out.
    //
    // The far side never answers, so whichever release goes first waits a
    // third of the lease, 1 s, and the other must wait for it: neither can
    // return sooner than that after the first began. Which of the two goes
    // first is up to the scheduler, so the test does not say.
    let (_dir, store) = over(LEASING, &[("THEN", "silent"), ("LEASE", "3000")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    let timed = || {
        let start = Instant::now();
        hold.release();
        (start, Instant::now())
    };
    thread::scope(|scope| {
        let first = scope.spawn(timed);
        thread::sleep(Duration::from_millis(100));
        let second = timed();
        let first = first.join().unwrap();
        let start = first.0.min(second.0);
        let end = first.1.min(second.1);
        assert!(
            end - start >= Duration::from_millis(900),
            "one returned {:?} after the first began",
            end - start
        );
    });
}

#[test]
fn a_hold_that_may_have_lapsed_is_let_go_without_waiting() {
    // The lock is answered half a lease late, so its first renewal is sent
    // at once, and is not answered: the hold lapses a third of a lease
    // before that renewal gives up.
    let (_dir, store) = over(
        LEASING,
        &[("THEN", "silent"), ("LEASE", "3000"), ("DELAY", "1.5")],
        quick(),
    );
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    assert_eq!(lapses(&hold), "it was not renewed in time");
    let start = Instant::now();
    drop(hold);
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_lease_too_short_to_keep_is_refused() {
    for ms in ["0", "50"] {
        let (_dir, store) = over(LEASING, &[("THEN", "serve"), ("LEASE", ms)], quick());
        let error = store.hold(&[("worker", "lead")]).unwrap_err();
        assert!(
            error.to_string().contains(&format!(
                "the far side gave a lease of {ms} ms, shorter than the 100 ms a wait can keep"
            )),
            "{error}"
        );
    }
    // The shortest it keeps is kept.
    let (_dir, store) = over(LEASING, &[("THEN", "serve"), ("LEASE", "100")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    assert!(hold.left().unwrap().is_some());
}

#[test]
fn a_hold_whose_lock_was_slow_to_answer_renews_in_time() {
    // The lock is answered half the lease after it was sent: the first
    // renewal is due a third of the way from when it was sent, not from
    // its answer, or the hold would count itself lapsed before it.
    let (dir, store) = over(LEASING, &[("THEN", "serve"), ("DELAY", "0.15")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        assert!(hold.check().is_ok(), "{:?}", log(dir.path()));
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_hold_whose_renewals_are_not_answered_lapses_before_its_lease_would_end() {
    let (_dir, store) = over(LEASING, &[("THEN", "silent")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    let start = Instant::now();
    assert_eq!(hold.check(), Ok(()));
    lapses(&hold);
    // Before the far side, which counts the 300 ms from when it had the
    // request, lets go.
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "{:?}",
        start.elapsed()
    );
    let error = hold.check().unwrap_err();
    assert!(!error.is_transient());
    assert!(
        error
            .to_string()
            .starts_with("this wait's locks on the far side may have been let go ("),
        "{error}"
    );
}

#[test]
fn a_hold_the_far_side_says_has_lapsed_has() {
    let (_dir, store) = over(LEASING, &[("THEN", "refuse")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    assert_eq!(lapses(&hold), "the lease ran out");
}

#[test]
fn a_hold_whose_connection_ends_lapses_at_once() {
    // A lease renewed only after a second.
    let (_dir, store) = over(LEASING, &[("THEN", "exit"), ("LEASE", "3000")], quick());
    let hold = store.hold(&[("worker", "lead")]).unwrap();
    let start = Instant::now();
    let reason = lapses(&hold);
    assert!(reason.starts_with("its connection ended"), "{reason}");
    // Seen between renewals, not at the first.
    assert!(
        start.elapsed() < Duration::from_millis(700),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_hold_another_wait_has_is_refused_with_what_the_far_side_said() {
    let said = r#"{"error":{"category":"held","message":"another wait is already picking up worker \"lead\""}}"#;
    let (dir, store) = over(&holding("409 Conflict"), &[("ANSWER", said)], quick());
    let error = store.hold(&[("worker", "lead")]).unwrap_err();
    assert_eq!(
        error,
        Error::Held {
            message: "another wait is already picking up worker \"lead\"".into()
        }
    );
    assert_eq!(
        error.to_string(),
        "another wait is already picking up worker \"lead\""
    );
    // The child is let go, and not tried again.
    assert!(logged(dir.path(), "end"), "{:?}", log(dir.path()));
    assert_eq!(starts(dir.path()), 1);
}

#[test]
fn a_hold_with_no_answer_says_why() {
    let (_dir, store) = over(
        &failing("bash: cases: command not found", 127),
        &[],
        quick(),
    );
    let error = store.hold(&[("worker", "lead")]).unwrap_err();
    assert!(matches!(error, Error::Dial { .. }), "{error:?}");
    let mut limits = quick();
    limits.request = Duration::from_millis(300);
    let (_dir, store) = over("cat >/dev/null", &[], limits);
    let start = Instant::now();
    let error = store.hold(&[("worker", "lead")]).unwrap_err();
    assert!(matches!(error, Error::Transport { .. }), "{error:?}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}
