//! `cases tui`: the inbox in the terminal (TU-01). The screen itself is
//! tested in the binary, drawn to a buffer; here, what happens without a
//! terminal, when its terminal closes, and how it follows a remote store
//! and quits while it holds a poll.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]
#![cfg_attr(
    not(feature = "remote"),
    allow(
        unused_imports,
        dead_code,
        reason = "they serve the remote store's test"
    )
)]

mod common;
// The remote store crate's fake server, shared by path as tests/remote.rs
// shares it.
#[cfg(feature = "remote")]
#[path = "../../store-remote/tests/fake/mod.rs"]
mod fake;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Append, Author, Open, Record, State, Store as _};
use cases_store_sqlite::Writer;
use common::{Env, spawn};
#[cfg(feature = "remote")]
use fake::{Fake, TOKEN};

#[test]
fn tui_refuses_to_run_without_a_terminal() {
    let env = Env::new();
    let out = env.cases(&["tui"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.stdout, "", "it drew something");
    assert!(out.stderr.contains("needs a terminal"), "{out:?}");
    assert!(!env.store.exists(), "it made the store");
}

/// The shell `script` runs: it sizes its terminal, which starts with no
/// rows or columns when `script`'s input is not a terminal, so that the
/// screen is drawn, and says its pid, which `exec` hands to `cases`. With
/// `STATUS` set, a shell of its own says its pid and execs `cases`, and
/// this one, left to wait through a Ctrl-C, writes how `cases` exited to
/// `STATUS`. With
/// `RESIZE` set to a path, the terminal is made smaller once that path is
/// made. `ARGS`, split on spaces, follows `tui`.
const RUN: &str = r#"/bin/stty rows 40 cols 120
if [ -n "$RESIZE" ]; then
  (while [ ! -e "$RESIZE" ]; do /bin/sleep 0.1; done; /bin/stty rows 30 cols 100 </dev/tty) &
fi
if [ -n "$STATUS" ]; then
  trap : INT
  /bin/sh -c 'echo "pid $$"; exec "$CASES" --store "$STORE" tui $ARGS'
  echo $? > "$STATUS"
else
  echo "pid $$"; exec "$CASES" --store "$STORE" tui $ARGS
fi"#;

/// `script`, which runs `RUN` in a terminal of its own, as a terminal
/// window or a herdr pane runs a shell. The forms differ by system.
fn script(env: &Env) -> Command {
    script_on(env, env.store.to_str().expect("a UTF-8 path"), &[])
}

/// [`script`], on the store `store`, with the variables `vars` set too.
fn script_on(env: &Env, store: &str, vars: &[(&str, &str)]) -> Command {
    let mut command = Command::new("/usr/bin/script");
    if cfg!(target_os = "linux") {
        command.args(["-q", "-c", RUN, "/dev/null"]);
    } else {
        command.args(["-q", "/dev/null", "/bin/sh", "-c", RUN]);
    }
    command
        .env_clear()
        .envs(common::coverage_vars())
        .env("SHELL", "/bin/sh")
        .env("TERM", "xterm-256color")
        // No test may reach the real keychain.
        .env("CASES_NO_KEYCHAIN", "1")
        .env("CASES", env!("CARGO_BIN_EXE_cases"))
        .env("STORE", store)
        .env("HOME", env.path("home"))
        .env("XDG_STATE_HOME", env.path("state"))
        .envs(vars.iter().copied())
        .current_dir(env.path(""));
    command
}

/// Runs `command`, a [`script`], with its stdin and stdout piped, and gives
/// it, what it writes as it writes it, and the pid of the `cases tui` in
/// it once the TUI has switched to the alternate screen, which it does once
/// the terminal is raw and its loop is about to start, and what it wrote
/// until then.
fn start(command: &mut Command) -> (Child, Receiver<Vec<u8>>, String, Vec<u8>) {
    // stdin stays open: `script` may end when its input does.
    let mut script = spawn(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    let mut stdout = script.stdout.take().expect("a stdout");
    let (chunks, received) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        while let Ok(n) = stdout.read(&mut buffer) {
            if n == 0 || chunks.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(chunk) = received.recv_timeout(left) else {
            panic!("the TUI did not start: {}", String::from_utf8_lossy(&seen));
        };
        seen.extend(chunk);
        let text = String::from_utf8_lossy(&seen);
        let pid = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid "))
            .map(str::to_string);
        if let Some(pid) = pid
            && text.contains("\x1b[?1049h")
        {
            break pid;
        }
    };
    (script, received, pid, seen)
}

/// Waits up to `within` for `pid` to end, kills it if it has not, and
/// gives whether it ended.
fn ends_within(pid: &str, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while !ended(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let gone = ended(pid);
    if !gone {
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
    gone
}

/// Whether `pid` has ended: gone, or a zombie waiting to be reaped.
fn ended(pid: &str) -> bool {
    let ps = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .expect("ps runs");
    !ps.status.success() || String::from_utf8_lossy(&ps.stdout).trim().starts_with('Z')
}

/// Closing the terminal ends the TUI. It used to stay, reading nothing at
/// full CPU, because crossterm's read never returns once the terminal is
/// gone and SIGHUP only set a flag the loop could not see.
#[cfg(unix)]
#[test]
fn a_closed_terminal_ends_it() {
    let env = Env::new();
    let (mut script, _received, pid, _) = start(&mut script(&env));
    // Let the loop reach its wait for a key.
    thread::sleep(Duration::from_millis(500));

    script.kill().expect("script is killed");
    script.wait().expect("script ends");

    assert!(
        ends_within(&pid, Duration::from_secs(5)),
        "cases tui (pid {pid}) is still running after its terminal closed"
    );
}

/// The requests the fake has been sent whose line has `part`.
#[cfg(feature = "remote")]
fn sent(fake: &Fake, part: &str) -> usize {
    fake.lines()
        .iter()
        .filter(|line| line.contains(part))
        .count()
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

/// On a remote store the TUI holds the store's poll on a thread of its own,
/// reads what changed when the version moves rather than once a second, and
/// quits at once while a poll is held, with the terminal put back.
#[cfg(all(unix, feature = "remote"))]
#[test]
fn on_a_remote_store_it_follows_the_held_poll_and_quits_at_once() {
    let fake = Fake::start();
    let id = fake.add_store("me");
    let env = Env::new();
    let url = fake.url();
    let vars = [("CASES_SERVER", url.as_str()), ("CASES_TOKEN", TOKEN)];
    let (mut script, received, pid, _) = start(&mut script_on(&env, &id, &vars));
    let holds = "/store/changes?after=";
    let reads = "/store/cases";
    assert!(soon(Duration::from_secs(5), || sent(&fake, holds) == 1));
    // Nothing changed: no read while the poll is held.
    let read = sent(&fake, reads);
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(sent(&fake, reads), read, "{:?}", fake.lines());

    // A case lands: the screen reads what changed, and holds the poll again.
    let mut open = Open::default();
    "fyi".clone_into(&mut open.kind);
    "today".clone_into(&mut open.urgency);
    "Over the poll".clone_into(&mut open.title);
    let append = Append::new(Author::Agent, &Record::Open(open));
    fake.db(&id).create(&append, &Writer::new(None)).unwrap();
    fake.wake();
    assert!(
        soon(Duration::from_secs(3), || sent(&fake, reads) > read
            && sent(&fake, holds) == 2),
        "{:?}",
        fake.lines()
    );

    // q quits while the poll is held, which the server keeps for 25 s.
    thread::sleep(Duration::from_millis(300));
    let quit = Instant::now();
    let mut stdin = script.stdin.take().expect("a stdin");
    stdin.write_all(b"q").expect("q is typed");
    stdin.flush().expect("q is sent");
    let gone = ends_within(&pid, Duration::from_secs(3));
    let took = quit.elapsed();
    let _ = script.kill();
    let _ = script.wait();
    assert!(gone, "cases tui (pid {pid}) is still running after q");
    assert!(took < Duration::from_secs(2), "{took:?}");
    let shown: Vec<u8> = received.try_iter().flatten().collect();
    assert!(
        String::from_utf8_lossy(&shown).contains("\x1b[?1049l"),
        "the terminal was not put back"
    );
}

/// The stand-in for ssh on a forward: it drops the options and the host,
/// and runs this build's `cases dial-stdio` on the store `@FAR@`, with what
/// it is sent logged to `@LOG@`, a request line for each request.
const SSH: &str = r#"#!/bin/sh
while [ $# -gt 0 ] && [ "$1" != "cases" ]; do shift; done
shift
tee -a "@LOG@" | CASES_STORE="@FAR@" "@BIN@" "$@"
"#;

/// A directory with the stand-in ssh of [`SSH`] in it, forwarding to the
/// store of `env`, and the log of what it is sent: the PATH to run with,
/// and the log.
#[cfg(unix)]
fn forwarding(env: &Env) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = env.path("ssh-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = env.path("ssh.log");
    let script = SSH
        .replace("@LOG@", log.to_str().unwrap())
        .replace("@FAR@", env.store.to_str().unwrap())
        .replace("@BIN@", env!("CARGO_BIN_EXE_cases"));
    let ssh = bin.join("ssh");
    std::fs::write(&ssh, script).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    (format!("{}:/usr/bin:/bin", bin.display()), log)
}

/// The requests in `log`, as [`forwarding`] logs them, whose line has
/// `part`.
fn requests(log: &Path, part: &str) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("GET ") && line.contains(part))
        .count()
}

/// On a forwarded store the TUI holds the store's poll on a thread of its
/// own, as on a remote one: it does not read over ssh once a second, so a
/// slow link holds up the screen only when something changed, and it quits
/// at once while a poll is held.
#[cfg(unix)]
#[test]
fn on_a_forwarded_store_it_follows_the_held_poll_and_quits_at_once() {
    let env = Env::new();
    env.open_fyi("Before");
    let (path, log) = forwarding(&env);
    let mut tui = Tui::start(&mut script_on(&env, "ssh://devbox", &[("PATH", &path)]));
    let holds = "/store/changes?after=";
    let reads = "/store/cases";
    assert!(
        soon(Duration::from_secs(5), || requests(&log, holds) == 1),
        "{:?}",
        std::fs::read_to_string(&log)
    );
    assert!(tui.shows("Before"), "{}", tui.drawn());
    // Nothing changed: no read while the poll is held.
    let read = requests(&log, reads);
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        requests(&log, reads),
        read,
        "{:?}",
        std::fs::read_to_string(&log)
    );

    // A case lands on the far side: the screen reads what changed, and holds
    // the poll again.
    tui.mark();
    env.open_fyi("Over the forward");
    assert!(tui.shows("Over the forward"), "{}", tui.drawn());
    assert!(
        soon(Duration::from_secs(3), || requests(&log, reads) > read
            && requests(&log, holds) == 2),
        "{:?}",
        std::fs::read_to_string(&log)
    );

    // q quits while the poll is held, which the far side keeps for 25 s.
    thread::sleep(Duration::from_millis(300));
    let quit = Instant::now();
    tui.quits();
    assert!(
        quit.elapsed() < Duration::from_secs(3),
        "{:?}",
        quit.elapsed()
    );
}

/// A stand-in ssh as [`SSH`] whose far side holds back each part of its
/// answers by 2 s while the file `@SLOW@` is there: a slow link.
const SLOW_SSH: &str = r#"#!/bin/sh
while [ $# -gt 0 ] && [ "$1" != "cases" ]; do shift; done
shift
tee -a "@LOG@" | CASES_STORE="@FAR@" "@BIN@" "$@" | perl -e '
  while (sysread(STDIN, $b, 65536)) { sleep 2 if -e $ARGV[0]; syswrite(STDOUT, $b) }
' "@SLOW@"
"#;

/// The TUI on a store forwarded over the stand-in ssh of [`SLOW_SSH`], once
/// it has read the store and holds its poll: it, the log of what it was
/// sent, and the file that makes the link slow while it is there.
#[cfg(unix)]
fn on_a_slow_link(env: &Env) -> (Tui, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let (path, log) = forwarding(env);
    let slow = env.path("slow");
    let script = SLOW_SSH
        .replace("@LOG@", log.to_str().unwrap())
        .replace("@FAR@", env.store.to_str().unwrap())
        .replace("@BIN@", env!("CARGO_BIN_EXE_cases"))
        .replace("@SLOW@", slow.to_str().unwrap());
    let ssh = env.path("ssh-bin").join("ssh");
    std::fs::write(&ssh, script).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut tui = Tui::start(&mut script_on(env, "ssh://devbox", &[("PATH", &path)]));
    assert!(tui.shows("Before"), "{}", tui.drawn());
    assert!(
        soon(Duration::from_secs(5), || requests(
            &log,
            "/store/changes?after="
        ) == 1),
        "{:?}",
        std::fs::read_to_string(&log)
    );
    (tui, log, slow)
}

/// While a read of a forwarded store is slow, the keys still work and the
/// screen is still drawn; what was read shows once the read is done
/// (TU-04).
#[cfg(unix)]
#[test]
fn a_slow_read_does_not_hold_up_the_keys() {
    let env = Env::new();
    env.open_fyi("Before");
    let (mut tui, log, slow) = on_a_slow_link(&env);
    let reads = "/store/cases";

    // The link turns slow, and a case lands on the far side: the held poll
    // wakes the screen, which reads.
    std::fs::write(&slow, "").unwrap();
    let read = requests(&log, reads);
    tui.mark();
    env.open_fyi("Over a slow link");
    assert!(
        soon(Duration::from_secs(10), || requests(&log, reads) > read),
        "{:?}",
        std::fs::read_to_string(&log)
    );

    // While that read waits on the far side, a key opens help at once.
    let asked = Instant::now();
    tui.send(b"?");
    assert!(tui.shows("╭ keys ─"), "{}", tui.drawn());
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "help took {:?}: the key waited on the read",
        asked.elapsed()
    );
    assert!(!tui.drawn().contains("Over a slow link"), "{}", tui.drawn());
    tui.keys(&[b"?"]);

    // The read lands once the far side answers.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !tui.shows("Over a slow link") {
        assert!(
            Instant::now() < deadline,
            "the read never landed: {}",
            tui.drawn()
        );
    }
    std::fs::remove_file(&slow).unwrap();
    tui.quits();
}

/// While a write to a forwarded store is slow, the keys still work and the
/// screen says it is being sent; how it went shows once the read after it
/// is done (TU-04).
#[cfg(unix)]
#[test]
fn a_slow_write_does_not_hold_up_the_keys() {
    let env = Env::new();
    let id = env.open_fyi("Before");
    let (mut tui, log, slow) = on_a_slow_link(&env);

    // The link turns slow, and the human acknowledges the case.
    std::fs::write(&slow, "").unwrap();
    tui.mark();
    tui.keys(&[b"a"]);
    tui.send(b"\r");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );

    // While the write waits on the far side, the keys work at once: the
    // form closes, and help opens.
    let asked = Instant::now();
    tui.keys(&[ESC]);
    tui.send(b"?");
    assert!(tui.shows("╭ keys ─"), "{}", tui.drawn());
    assert!(
        asked.elapsed() < Duration::from_millis(1500),
        "help took {:?}: the keys waited on the write",
        asked.elapsed()
    );
    assert!(!tui.drawn().contains("recorded answer"), "{}", tui.drawn());
    tui.keys(&[b"?"]);

    // How it went shows once the read after it lands: the case leaves the
    // inbox, and the footer says it was recorded. Only the cells that
    // changed are drawn, so the line is looked for by its start.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !tui.shows("nothing needs you") {
        assert!(
            Instant::now() < deadline,
            "the write never landed: {:?} {}",
            std::fs::read_to_string(&log),
            tui.drawn()
        );
    }
    assert!(tui.shows("recor"), "{}", tui.drawn());
    assert_eq!(env.case(&id).state(), State::Answered);
    std::fs::remove_file(&slow).unwrap();
    tui.quits();
}

/// Ctrl-C while a write to a forwarded store is slow quits at once, with
/// the terminal put back: the write may land or not.
#[cfg(unix)]
#[test]
fn ctrl_c_quits_at_once_while_a_slow_write_is_sent() {
    let env = Env::new();
    let id = env.open_fyi("Before");
    let (mut tui, _log, slow) = on_a_slow_link(&env);

    std::fs::write(&slow, "").unwrap();
    tui.mark();
    tui.keys(&[b"a"]);
    tui.send(b"\r");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );
    let quit = Instant::now();
    tui.send(b"\x03");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !tui.ended() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let took = quit.elapsed();
    assert!(took < Duration::from_secs(1), "Ctrl-C took {took:?}");
    tui.ends();
}

/// A stand-in ssh as [`SSH`] that holds back a request to write, a POST, by
/// 3 s on its way to the far side while the file `@SLOW@` is there: a slow
/// link the write is sent over, while reads sent after it on another pipe
/// go at once.
const SLOW_WRITE_SSH: &str = r#"#!/bin/sh
while [ $# -gt 0 ] && [ "$1" != "cases" ]; do shift; done
shift
perl -e '
  $| = 1;
  while (sysread(STDIN, $b, 65536)) { sleep 3 if -e $ARGV[0] && $b =~ /^POST /m; syswrite(STDOUT, $b) }
' "@SLOW@" | tee -a "@LOG@" | CASES_STORE="@FAR@" "@BIN@" "$@"
"#;

/// `cases tui ARGS`, such as an id for one case (TU-11), on a store
/// forwarded over the stand-in ssh of [`SLOW_WRITE_SSH`], with the link
/// slow: it, where its exit status is written, and the log of what it was
/// sent.
#[cfg(unix)]
fn over_a_slow_write(env: &Env, args: &str) -> (Tui, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let (path, log) = forwarding(env);
    let slow = env.path("slow");
    let script = SLOW_WRITE_SSH
        .replace("@LOG@", log.to_str().unwrap())
        .replace("@FAR@", env.store.to_str().unwrap())
        .replace("@BIN@", env!("CARGO_BIN_EXE_cases"))
        .replace("@SLOW@", slow.to_str().unwrap());
    let ssh = env.path("ssh-bin").join("ssh");
    std::fs::write(&ssh, script).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&slow, "").unwrap();
    let status = env.path("status");
    let vars = [
        ("PATH", path.as_str()),
        ("STATUS", status.to_str().expect("a UTF-8 path")),
        ("ARGS", args),
    ];
    let mut tui = Tui::start(&mut script_on(env, "ssh://devbox", &vars));
    assert!(tui.shows("Before"), "{}", tui.drawn());
    (tui, status, log)
}

/// On one case, q while the answer is being sent waits for it to land, and
/// exits as it left the case: an agent is not told the human quit without
/// answering when the answer lands (TU-11).
#[cfg(unix)]
#[test]
fn on_one_case_q_while_the_answer_is_sent_waits_for_it() {
    let env = Env::new();
    let id = env.open_fyi("Before");
    let (mut tui, status, log) = over_a_slow_write(&env, &id);
    tui.mark();
    tui.keys(&[b"a"]);
    tui.send(b"\r");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );
    tui.send(b"q");
    assert_eq!(
        exit_status(&status),
        "0",
        "{:?} {}",
        std::fs::read_to_string(&log),
        tui.drawn()
    );
    assert!(tui.shows(&format!("{id} answered")), "{}", tui.drawn());
    tui.ends();
    assert_eq!(env.case(&id).state(), State::Answered);
}

/// On one case, Ctrl-C while the answer is being sent does not wait for
/// it: it exits 2 at once, saying the answer may land yet (TU-11).
#[cfg(unix)]
#[test]
fn on_one_case_ctrl_c_while_the_answer_is_sent_exits_2_at_once() {
    let env = Env::new();
    let id = env.open_fyi("Before");
    let (mut tui, status, _log) = over_a_slow_write(&env, &id);
    tui.mark();
    tui.keys(&[b"a"]);
    tui.send(b"\r");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );
    let quit = Instant::now();
    tui.send(b"\x03");
    assert_eq!(exit_status(&status), "2");
    let took = quit.elapsed();
    assert!(took < Duration::from_secs(2), "Ctrl-C took {took:?}");
    assert!(tui.shows("may land yet"), "{}", tui.drawn());
    tui.ends();
}

/// In the inbox, q while an answer is being sent waits for it to land,
/// saying so, so that a quick answer on a slow link is not lost (TU-04).
#[cfg(unix)]
#[test]
fn in_the_inbox_q_while_an_answer_is_sent_waits_for_it() {
    let env = Env::new();
    let id = env.open_fyi("Before");
    let (mut tui, status, log) = over_a_slow_write(&env, "");
    tui.mark();
    tui.keys(&[b"a"]);
    tui.send(b"\r");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );
    tui.send(b"q");
    assert!(tui.shows("waiting for it to land"), "{}", tui.drawn());
    assert_eq!(
        exit_status(&status),
        "0",
        "{:?} {}",
        std::fs::read_to_string(&log),
        tui.drawn()
    );
    // The answer was in the store before the TUI exited, and it says so.
    assert_eq!(env.case(&id).state(), State::Answered);
    assert!(tui.shows("recorded answer"), "{}", tui.drawn());
    tui.ends();
}

/// The inbox over [`over_a_slow_write`], with a stuck case answered with
/// `text` and the answer held on the link: it, where its exit status is
/// written, and the stuck case's id.
#[cfg(unix)]
fn answering_over_a_slow_write(env: &Env, text: &str) -> (Tui, PathBuf, String) {
    env.open_fyi("Before");
    let id = env.open_stuck();
    let (mut tui, status, _log) = over_a_slow_write(env, "");
    // The stuck case is blocking: it is first, and selected.
    tui.mark();
    tui.keys(&[b"a", text.as_bytes(), SEND]);
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}")),
        "{}",
        tui.drawn()
    );
    tui.keys(&[ESC]);
    (tui, status, id)
}

/// In the inbox, q while an answer is being sent that the store then
/// refuses says so on the way out, and gives back what was typed, so that
/// it is not lost.
#[cfg(unix)]
#[test]
fn in_the_inbox_q_says_when_the_answer_it_waited_for_was_refused() {
    let env = Env::new();
    let (mut tui, status, id) = answering_over_a_slow_write(&env, "use the mirror");
    // The case changes while the answer is held on the link: it is
    // refused as stale once it reaches the store.
    env.ok(&["amend", &id, "--body", "Changed meanwhile."]);
    tui.mark();
    tui.send(b"q");
    assert_eq!(exit_status(&status), "0", "{}", tui.drawn());
    assert!(tui.shows("nothing sent"), "{}", tui.drawn());
    assert!(tui.shows("what you typed:"), "{}", tui.drawn());
    assert!(tui.shows("    use the mirror"), "{}", tui.drawn());
    assert_eq!(env.case(&id).state(), State::Open);
    tui.ends();
}

/// In the inbox, Ctrl-C while q waits for an answer to land stops the
/// wait: the TUI exits at once, with the terminal put back, saying the
/// answer may land yet and giving back what was typed.
#[cfg(unix)]
#[test]
fn in_the_inbox_ctrl_c_stops_the_wait_after_q() {
    let env = Env::new();
    let (mut tui, status, id) = answering_over_a_slow_write(&env, "use the mirror");
    tui.mark();
    tui.send(b"q");
    assert!(tui.shows("waiting for it to land"), "{}", tui.drawn());
    let quit = Instant::now();
    tui.send(b"\x03");
    assert_eq!(exit_status(&status), "0", "{}", tui.drawn());
    let took = quit.elapsed();
    assert!(took < Duration::from_secs(1), "Ctrl-C took {took:?}");
    assert!(
        tui.shows(&format!("⋯ sending answer on {id}: it may land yet")),
        "{}",
        tui.drawn()
    );
    assert!(tui.shows("    use the mirror"), "{}", tui.drawn());
    tui.ends();
}

/// A forwarded store not made yet is read as no cases, and read again
/// until it is: the case that makes it shows.
#[cfg(unix)]
#[test]
fn on_a_forwarded_store_not_made_yet_the_case_that_makes_it_shows() {
    let env = Env::new();
    let (path, _log) = forwarding(&env);
    let mut tui = Tui::start(&mut script_on(&env, "ssh://devbox", &[("PATH", &path)]));
    thread::sleep(Duration::from_millis(500));
    tui.mark();
    env.open_fyi("The first case");
    assert!(tui.shows("The first case"), "{}", tui.drawn());
    tui.quits();
}

/// A store on the far side of a forward made by a newer cases than the one
/// there ends the TUI with the reason, naming the far side as the one to
/// update, as a local store from a newer cases does (TU-01, ST-07).
#[cfg(unix)]
#[test]
fn on_a_forwarded_store_from_a_newer_cases_it_exits_with_the_reason() {
    let env = Env::new();
    env.open_fyi("Before");
    let conn = rusqlite::Connection::open(&env.store).unwrap();
    conn.execute("UPDATE meta SET schema_version = 99", [])
        .unwrap();
    drop(conn);
    let (path, _log) = forwarding(&env);
    let status = env.path("status");
    let vars = [
        ("PATH", path.as_str()),
        ("STATUS", status.to_str().unwrap()),
    ];
    let mut script = spawn(
        script_on(&env, "ssh://devbox", &vars)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    assert_eq!(exit_status(&status), "1");
    let _ = script.kill();
    let mut shown = String::new();
    let _ = script.stdout.take().unwrap().read_to_string(&mut shown);
    let _ = script.wait();
    assert!(shown.contains("schema version 99"), "{shown}");
    assert!(
        shown.contains("on the far side reads: update cases there"),
        "{shown}"
    );
    assert!(shown.contains("ssh://devbox"), "{shown}");
}

/// A `cases tui` running in [`script`], typed at as a human would, and
/// what it has drawn so far.
struct Tui {
    script: Child,
    received: Receiver<Vec<u8>>,
    pid: String,
    shown: Vec<u8>,
    /// How many of the TUI's asks for the cursor's position were answered.
    answered: usize,
    /// Where in `shown` [`Tui::shows`] looks from.
    from: usize,
}

impl Tui {
    fn start(command: &mut Command) -> Tui {
        let (script, received, pid, shown) = start(command);
        Tui {
            script,
            received,
            pid,
            shown,
            answered: 0,
            from: 0,
        }
    }

    /// Types each of `keys`, a little apart, so that each is read as a key
    /// of its own and the loop runs between them.
    fn keys(&mut self, keys: &[&[u8]]) {
        for key in keys {
            self.send(key);
            self.settle();
        }
    }

    /// Takes what the TUI draws for 200 ms, long enough for its loop to
    /// read a key and draw what follows.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(200);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            self.take(left);
        }
    }

    /// Types Ctrl-E, which opens the editor, and waits for the TUI to take
    /// the terminal back, which it does by clearing it. A key typed while
    /// the editor runs reaches a terminal that is not raw, where Ctrl-S
    /// stops its output.
    fn edit(&mut self) {
        let before = self.answered;
        self.send(b"\x05");
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.answered == before {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(self.take(left), "the TUI did not come back from the editor");
        }
        self.settle();
    }

    fn send(&mut self, bytes: &[u8]) {
        let stdin = self.script.stdin.as_mut().expect("a stdin");
        stdin.write_all(bytes).expect("the bytes are typed");
        stdin.flush().expect("the bytes are sent");
    }

    /// Takes what the TUI drew within `within`, and answers its asks for
    /// the cursor's position, as a terminal does: it asks when it clears
    /// the screen after the editor. Gives whether it drew.
    fn take(&mut self, within: Duration) -> bool {
        let Ok(chunk) = self.received.recv_timeout(within) else {
            return false;
        };
        self.shown.extend(chunk);
        let asked = self.shown.windows(4).filter(|w| w == b"\x1b[6n").count();
        while self.answered < asked {
            self.send(b"\x1b[1;1R");
            self.answered += 1;
        }
        true
    }

    /// Whether the screen shows `text` within 5 s, drawn since the last
    /// [`Tui::mark`].
    fn shows(&mut self, text: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if String::from_utf8_lossy(&self.shown[self.from..]).contains(text) {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if !self.take(left) {
                return false;
            }
        }
    }

    /// Has [`Tui::shows`] look only at what is drawn from now on.
    fn mark(&mut self) {
        self.from = self.shown.len();
    }

    /// What was drawn since the last [`Tui::mark`], for a failure.
    fn drawn(&self) -> String {
        String::from_utf8_lossy(&self.shown[self.from..]).into_owned()
    }

    /// Whether `cases tui` has ended.
    fn ended(&self) -> bool {
        ended(&self.pid)
    }

    /// Waits for `cases tui` to end, which it must within 5 s, with the
    /// terminal put back, and ends `script`.
    fn ends(mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.ended() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        // What it was doing, for a failure, before it is killed.
        let state = Command::new("ps")
            .args(["-o", "stat=,wchan=", "-p", &self.pid])
            .output()
            .map(|ps| String::from_utf8_lossy(&ps.stdout).trim().to_owned());
        let gone = ends_within(&self.pid, Duration::ZERO);
        let _ = self.script.kill();
        let _ = self.script.wait();
        if !gone {
            // Only taken: `script` is gone, so an ask cannot be answered.
            while let Ok(chunk) = self.received.recv_timeout(Duration::from_millis(100)) {
                self.shown.extend(chunk);
            }
            let tail = self.shown.len().saturating_sub(2000);
            panic!(
                "cases tui (pid {}) is still running, in state {state:?}, having drawn last: {:?}",
                self.pid,
                String::from_utf8_lossy(&self.shown[tail..])
            );
        }
        // The editor leaves the alternate screen too: what counts is
        // leaving it after it was last entered.
        let entered = self
            .shown
            .windows(8)
            .rposition(|w| w == b"\x1b[?1049h")
            .expect("the TUI entered the alternate screen");
        self.from = entered;
        assert!(self.shows("\x1b[?1049l"), "the terminal was not put back");
        // And the mouse is no longer reported (TU-12).
        let captured = self
            .shown
            .windows(8)
            .rposition(|w| w == b"\x1b[?1000h")
            .expect("the TUI took the mouse");
        self.from = captured;
        assert!(self.shows("\x1b[?1000l"), "the mouse was not given back");
    }

    /// Types q, and checks that it quits.
    fn quits(mut self) {
        self.keys(&[b"q"]);
        self.ends();
    }
}

/// A test that fails part way leaves no TUI running.
impl Drop for Tui {
    fn drop(&mut self) {
        if !self.ended() {
            let _ = Command::new("kill").args(["-9", &self.pid]).status();
        }
        let _ = self.script.kill();
        let _ = self.script.wait();
    }
}

/// Ctrl-S, which sends.
const SEND: &[u8] = b"\x13";
const ESC: &[u8] = b"\x1b";

/// An editor, at `name` in the test's directory, that runs `body`, the
/// file to edit being `$1`.
fn editor(env: &Env, name: &str, body: &str) -> String {
    let path = env.path(name);
    common::write_file(&path, &format!("#!/bin/sh\n{body}\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path.to_str().expect("a UTF-8 path").to_owned()
}

/// The TUI with the editor `editor`.
fn script_with_editor(env: &Env, editor: &str) -> Command {
    let store = env.store.to_str().expect("a UTF-8 path");
    script_on(env, store, &[("VISUAL", editor)])
}

/// An answer typed in the editor is sent with Ctrl-S, as the human, and the
/// screen reads the case again.
#[cfg(unix)]
#[test]
fn an_answer_from_the_editor_is_sent() {
    let env = Env::new();
    let id = env.open_stuck();
    let editor = editor(&env, "editor", r#"printf 'use the mirror\n' > "$1""#);
    let mut tui = Tui::start(&mut script_with_editor(&env, &editor));
    assert!(tui.shows("Blocked"));

    tui.keys(&[b"a"]);
    tui.edit();
    tui.mark();
    tui.keys(&[SEND]);
    assert!(tui.shows("answered"), "{}", tui.drawn());
    tui.quits();
    let case = env.case(&id);
    assert_eq!(case.state(), State::Answered);
    assert_eq!(case.answer().unwrap().text, "use the mirror");
}

/// A message to a worker, written in the editor, is sent with Ctrl-S, as
/// the human.
#[cfg(unix)]
#[test]
fn a_message_from_the_editor_is_sent() {
    let env = Env::new();
    let editor = editor(&env, "editor", r#"printf 'Ship it\n\nToday.\n' > "$1""#);
    let mut tui = Tui::start(&mut script_with_editor(&env, &editor));

    // A message opens in its "to" field.
    tui.keys(&[b"m", b"w1", ESC]);
    tui.edit();
    tui.keys(&[SEND]);
    assert!(tui.shows("sent message"), "{}", tui.drawn());
    tui.quits();
    let list = env.ok(&["list", "--state", "sent", "--worker", "w1"]);
    assert!(list.contains("Ship it"), "{list}");
}

/// The edit of a sent message refused as stale, as the message was edited
/// elsewhere meanwhile, drops what was typed and shows the message as it
/// is now, as the web inbox does: a second Ctrl-S sends nothing (IN-05).
#[cfg(unix)]
#[test]
fn an_edit_refused_as_stale_shows_the_message_as_it_is_now() {
    let env = Env::new();
    let id = common::json(&env.ok(&[
        "message", "--as", "Ryan", "--to", "w1", "--title", "Ship it", "--body", "Today.", "--json",
    ]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut tui = Tui::start(&mut script(&env));
    // In flight, the message is the one case; e edits its body.
    tui.keys(&[b"2"]);
    assert!(tui.shows("Ship it"), "{}", tui.drawn());
    tui.keys(&[b"e", b" Mine."]);
    env.ok(&["amend", &id, "--as", "Ryan", "--body", "From the web."]);
    // The screen reads the edit made elsewhere, then the send is refused.
    tui.mark();
    assert!(tui.shows("replaced the body"), "{}", tui.drawn());
    tui.mark();
    tui.keys(&[SEND]);
    assert!(tui.shows("nothing sent"), "{}", tui.drawn());
    assert!(tui.shows("replaced by the"), "{}", tui.drawn());
    assert_eq!(env.case(&id).body(), "From the web.");
    // A second send has nothing to save: the old draft is gone.
    tui.mark();
    tui.keys(&[SEND]);
    assert!(tui.shows("the body is as"), "{}", tui.drawn());
    assert_eq!(env.case(&id).body(), "From the web.");
    tui.keys(&[ESC]);
    tui.quits();
}

/// An editor that cannot be run is a line on the screen, from the form and
/// from a message alike, and the text is kept as it was.
#[cfg(unix)]
#[test]
fn an_editor_that_cannot_run_is_a_line_on_the_screen() {
    let env = Env::new();
    let id = env.open_stuck();
    let missing = env.path("no-editor");
    let mut tui = Tui::start(&mut script_with_editor(
        &env,
        missing.to_str().expect("a UTF-8 path"),
    ));
    assert!(tui.shows("Blocked"));

    tui.keys(&[b"a"]);
    tui.mark();
    tui.edit();
    assert!(tui.shows("the editor"), "{}", tui.drawn());
    // A stuck case's form opens in its field: one Esc leaves the field,
    // and the next the form.
    tui.mark();
    tui.keys(&[ESC, ESC, b"m"]);
    assert!(tui.shows("MESSAGE"), "{}", tui.drawn());
    tui.mark();
    tui.edit();
    assert!(tui.shows("the editor"), "{}", tui.drawn());
    tui.keys(&[ESC]);
    tui.quits();
    assert_eq!(env.case(&id).state(), State::Open);
}

/// An editor that exits non-zero leaves the text as it was.
#[cfg(unix)]
#[test]
fn an_editor_that_fails_changes_nothing() {
    let env = Env::new();
    let id = env.open_stuck();
    let editor = editor(&env, "editor", r#"printf 'not this\n' > "$1"; exit 1"#);
    let mut tui = Tui::start(&mut script_with_editor(&env, &editor));
    assert!(tui.shows("Blocked"));

    tui.keys(&[b"a"]);
    tui.edit();
    tui.keys(&[SEND]);
    assert!(tui.shows("write the guidance first"), "{}", tui.drawn());
    tui.mark();
    tui.keys(&[ESC, ESC, b"m", b"b", b"kept", ESC]);
    assert!(tui.shows("MESSAGE"), "{}", tui.drawn());
    tui.mark();
    tui.edit();
    assert!(tui.shows("kept"), "{}", tui.drawn());
    assert!(!tui.drawn().contains("not this"), "{}", tui.drawn());
    tui.keys(&[ESC, b"y"]);
    tui.quits();
    assert_eq!(env.case(&id).state(), State::Open);
}

/// q with a message typed asks first: n stays, y quits.
#[cfg(unix)]
#[test]
fn quitting_with_a_message_typed_asks_first() {
    let env = Env::new();
    let mut tui = Tui::start(&mut script(&env));

    tui.keys(&[b"m", b"b", b"x", ESC, b"q", b"n"]);
    thread::sleep(Duration::from_millis(300));
    assert!(!tui.ended(), "n quit");
    tui.keys(&[b"q", b"y"]);
    tui.ends();
}

/// A case written while the TUI runs on a local store shows within about a
/// second.
#[cfg(unix)]
#[test]
fn a_local_store_is_read_again() {
    let env = Env::new();
    let mut tui = Tui::start(&mut script(&env));
    thread::sleep(Duration::from_millis(300));
    env.open_fyi("Written meanwhile");
    assert!(tui.shows("meanwhile"), "{}", tui.drawn());
    tui.quits();
}

/// A smaller terminal has the whole screen drawn again, at its new size.
#[cfg(unix)]
#[test]
fn a_resize_draws_the_screen_again() {
    let env = Env::new();
    env.open_fyi("Before the resize");
    let store = env.store.to_str().expect("a UTF-8 path");
    let resize = env.path("resize");
    let resize = resize.to_str().expect("a UTF-8 path");
    let mut tui = Tui::start(&mut script_on(&env, store, &[("RESIZE", resize)]));
    assert!(tui.shows("resize"), "{}", tui.drawn());
    tui.mark();
    common::write_file(Path::new(resize), "");
    // The resize clears the screen and draws it again. What follows a mark
    // can still be the end of the first draw, which has the title twice, so
    // the redraw is what follows the clear.
    assert!(
        tui.shows("\x1b[2J"),
        "it did not clear for the resize: {}",
        tui.drawn()
    );
    // From the clear: the redraw can come in the same read.
    tui.from = tui
        .shown
        .windows(4)
        .rposition(|w| w == b"\x1b[2J")
        .expect("the clear");
    assert!(
        tui.shows("resize"),
        "it did not draw again: {}",
        tui.drawn()
    );
    tui.quits();
}

/// SIGTERM ends the loop, and the terminal is put back.
#[cfg(unix)]
#[test]
fn sigterm_ends_it_with_the_terminal_put_back() {
    let env = Env::new();
    let tui = Tui::start(&mut script(&env));
    thread::sleep(Duration::from_millis(300));
    let term = Command::new("kill").args(["-TERM", &tui.pid]).status();
    assert!(term.expect("kill runs").success());
    tui.ends();
}

/// In a herdr pane the TUI reports the pane as blocked while a case waits,
/// and releases it on quit.
#[cfg(unix)]
#[test]
fn in_herdr_it_reports_the_pane_and_releases_it() {
    let env = Env::new();
    env.open_stuck();
    let log = env.path("herdr.log");
    let herdr = editor(
        &env,
        "herdr",
        &format!(r#"echo "$@" >> "{}""#, log.display()),
    );
    let store = env.store.to_str().expect("a UTF-8 path");
    let vars = [
        ("HERDR_ENV", "1"),
        ("HERDR_BIN_PATH", herdr.as_str()),
        ("HERDR_PANE_ID", "p1"),
    ];
    let mut tui = Tui::start(&mut script_on(&env, store, &vars));
    let reported = || std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        soon(Duration::from_secs(5), || reported()
            .contains("--state blocked --message 1 waiting, 1 blocking")),
        "{}",
        reported()
    );
    assert!(tui.shows("Blocked"));
    tui.quits();
    assert!(
        soon(Duration::from_secs(2), || reported()
            .contains("release-agent")),
        "{}",
        reported()
    );
}

/// The store is named on screen from `~` when it is under the home
/// directory, and in full when it is not, even beside it.
#[cfg(unix)]
#[test]
fn the_store_is_named_from_home_when_it_is_under_it() {
    let env = Env::new();
    for (store, shown) in [
        (env.path("home/inbox.db"), "~/inbox.db".to_owned()),
        (
            env.path("homely/inbox.db"),
            env.path("homely/inbox.db").display().to_string(),
        ),
    ] {
        let mut tui = Tui::start(&mut script_on(
            &env,
            store.to_str().expect("a UTF-8 path"),
            &[],
        ));
        assert!(tui.shows(&shown), "{}", tui.drawn());
        tui.quits();
    }
}

/// Starts the TUI with `STATUS` set, and gives it and where its exit
/// status is written.
fn with_status(env: &Env) -> (Tui, PathBuf) {
    let status = env.path("status");
    let store = env.store.to_str().expect("a UTF-8 path");
    let vars = [("STATUS", status.to_str().expect("a UTF-8 path"))];
    let tui = Tui::start(&mut script_on(env, store, &vars));
    // Let the loop reach its wait for a key.
    thread::sleep(Duration::from_millis(300));
    (tui, status)
}

/// The exit status written to `status`, within 5 s.
fn exit_status(status: &Path) -> String {
    let read = || std::fs::read_to_string(status).unwrap_or_default();
    assert!(soon(Duration::from_secs(5), || read().ends_with('\n')));
    read().trim().to_owned()
}

/// A second SIGINT or SIGTERM, before the loop has seen the first, exits at
/// once with 128 and the signal's number, and puts the terminal back.
#[cfg(unix)]
#[test]
fn a_second_signal_exits_at_once_with_its_number() {
    let env = Env::new();
    let (tui, status) = with_status(&env);
    // One after the other, faster than the loop's tick. Two of the same
    // signal could arrive as one.
    let sh = Command::new("/bin/sh")
        .args(["-c", "kill -INT $0; kill -TERM $0", &tui.pid])
        .status();
    assert!(sh.expect("sh runs").success());
    let code = exit_status(&status);
    assert!(["130", "143"].contains(&code.as_str()), "{code}");
    tui.ends();
}

/// Two SIGINTs sent together, which can reach the TUI as one, end it with
/// the terminal put back: at once with 130, or by the loop with 0.
#[cfg(unix)]
#[test]
fn two_sigints_at_once_end_it_with_the_terminal_put_back() {
    let env = Env::new();
    let (tui, status) = with_status(&env);
    let sh = Command::new("/bin/sh")
        .args(["-c", "kill -INT $0 $0", &tui.pid])
        .status();
    assert!(sh.expect("sh runs").success());
    let code = exit_status(&status);
    assert!(["0", "130"].contains(&code.as_str()), "{code}");
    tui.ends();
}

/// SIGHUP exits at once with 129.
#[cfg(unix)]
#[test]
fn a_hangup_exits_with_129() {
    let env = Env::new();
    let (tui, status) = with_status(&env);
    let hup = Command::new("kill").args(["-HUP", &tui.pid]).status();
    assert!(hup.expect("kill runs").success());
    assert_eq!(exit_status(&status), "129");
}

/// q exits 0.
#[cfg(unix)]
#[test]
fn q_exits_0() {
    let env = Env::new();
    let (mut tui, status) = with_status(&env);
    tui.keys(&[b"q"]);
    // Before `script` is ended, which would hang up the shell that writes
    // the status.
    assert_eq!(exit_status(&status), "0");
    tui.ends();
}

/// Starts `cases tui` on the one case `id` (TU-11) with `STATUS` set, and
/// gives it and where its exit status is written.
fn on_one_case(env: &Env, id: &str) -> (Tui, PathBuf) {
    let status = env.path("status");
    let store = env.store.to_str().expect("a UTF-8 path");
    let vars = [
        ("STATUS", status.to_str().expect("a UTF-8 path")),
        ("ARGS", id),
    ];
    let mut tui = Tui::start(&mut script_on(env, store, &vars));
    assert!(tui.shows("FYI"), "{}", tui.drawn());
    (tui, status)
}

/// Answered in the TUI, it prints the case's id and state and exits 0.
#[cfg(unix)]
#[test]
fn on_one_case_an_answer_exits_0() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let (mut tui, status) = on_one_case(&env, &id);
    tui.keys(&[b"a", b"\r"]);
    assert_eq!(exit_status(&status), "0");
    assert!(tui.shows(&format!("{id} answered")), "{}", tui.drawn());
    tui.ends();
    assert!(env.case(&id).answer().unwrap().ack);
}

/// Answered elsewhere while it is open, it exits 0 too.
#[cfg(unix)]
#[test]
fn on_one_case_an_answer_elsewhere_exits_0() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let (mut tui, status) = on_one_case(&env, &id);
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(exit_status(&status), "0");
    assert!(tui.shows(&format!("{id} answered")), "{}", tui.drawn());
    tui.ends();
}

/// Withdrawn while it is open, it exits 3, as a refused transition.
#[cfg(unix)]
#[test]
fn on_one_case_a_withdraw_exits_3() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let (mut tui, status) = on_one_case(&env, &id);
    env.ok(&["withdraw", &id]);
    assert_eq!(exit_status(&status), "3");
    assert!(tui.shows(&format!("{id} withdrawn")), "{}", tui.drawn());
    tui.ends();
}

/// Quitting without an answer exits 2, as a wait that gives up.
#[cfg(unix)]
#[test]
fn on_one_case_quitting_exits_2() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let (mut tui, status) = on_one_case(&env, &id);
    tui.keys(&[b"q"]);
    assert_eq!(exit_status(&status), "2");
    tui.ends();
    assert_eq!(env.case(&id).state(), State::Open);
}

/// An unknown id fails before the terminal is checked or taken, and so
/// does a case the human cannot answer, exiting 3 as answer does.
#[test]
fn on_one_case_a_wrong_id_fails_before_the_screen_opens() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let out = env.cases(&["tui", "zzzzzzz"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("no case id contains"), "{out:?}");
    assert!(!out.stderr.contains("terminal"), "{out:?}");

    env.ok(&["withdraw", &id]);
    let out = env.cases(&["tui", &id]);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains("nothing to answer"), "{out:?}");
    assert_eq!(out.stdout, "");
}

/// In a terminal, an unknown id never opens the screen.
#[cfg(unix)]
#[test]
fn on_one_case_an_unknown_id_never_opens_the_screen() {
    let env = Env::new();
    env.open_fyi("FYI");
    let store = env.store.to_str().expect("a UTF-8 path");
    let out = common::output(
        script_on(&env, store, &[("ARGS", "zzzzzzz")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(!shown.contains("\x1b[?1049h"), "{shown:?}");
    assert!(shown.contains("no case id contains"), "{shown:?}");
}

/// On a forwarded store, the held poll sees the answer given on the far
/// side, and it exits 0.
#[cfg(unix)]
#[test]
fn on_one_case_of_a_forwarded_store_an_answer_elsewhere_exits_0() {
    let env = Env::new();
    let id = env.open_fyi("FYI");
    let (path, _log) = forwarding(&env);
    let status = env.path("status");
    let vars = [
        ("PATH", path.as_str()),
        ("STATUS", status.to_str().expect("a UTF-8 path")),
        ("ARGS", id.as_str()),
    ];
    let mut tui = Tui::start(&mut script_on(&env, "ssh://devbox", &vars));
    assert!(tui.shows("FYI"), "{}", tui.drawn());
    thread::sleep(Duration::from_millis(300));
    env.ok(&["answer", &id, "--ack"]);
    assert_eq!(exit_status(&status), "0");
    assert!(tui.shows(&format!("{id} answered")), "{}", tui.drawn());
    tui.ends();
}

/// A click, as the terminal reports it (SGR), on a tab switches to it; on
/// quitting the mouse is no longer reported, which [`Tui::ends`] checks.
#[cfg(unix)]
#[test]
fn a_click_on_a_tab_switches_to_it() {
    let env = Env::new();
    env.open_fyi("FYI");
    let mut tui = Tui::start(&mut script(&env));
    assert!(tui.shows("FYI"), "{}", tui.drawn());
    assert!(tui.shows("\x1b[?1000h"), "the mouse is not reported");
    tui.mark();
    // "Done" is drawn from column 34, row 2, counted from 1.
    tui.keys(&[b"\x1b[<0;35;2M", b"\x1b[<0;35;2m"]);
    assert!(tui.shows("no cases here."), "{}", tui.drawn());
    tui.quits();
}
