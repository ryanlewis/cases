//! The CLI on a forwarded store: `cases` with `CASES_FORWARD=ssh://devbox`
//! and an `ssh` on the PATH that stands in for the real one. It drops the
//! options and the host, and runs this build's `cases dial-stdio` against
//! the far side's store file, so a whole command goes through the pipe
//! with no ssh and no second machine.
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
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use common::{Env, Output, json, spawn};

/// A home that forwards to a far side whose store is `far`.
struct Forwarding {
    env: Env,
    far: PathBuf,
}

/// The stand-in for ssh: it logs its arguments, and while the file
/// `@DROP@` holds a count above 0, takes one off it and kills its
/// `dial-stdio` a second in, as a dropped connection does. While the file
/// `@SPARE@` holds a count above 0, it takes one off that first and lets
/// the connection be, dropped or not. While the file `@RELAY@` holds a
/// count above 0, it takes one off that first and sends its stdin on to
/// `dial-stdio` through a `cat` whose pid it writes to `@RELAYPID@`, so
/// that a test can stop what is sent, as a network that drops it does.
/// The relayed `dial-stdio` runs in a process group of its own, out of the
/// near side's reach as a far side on another machine is: a dropped pipe
/// whose child has not exited 200 ms after its stdin closed kills the
/// child's group, as a hold that lost its lease does, and that must not let
/// go of the far side's locks before their lease runs out.
const SSH: &str = r#"#!/bin/sh
echo "$*" >> "@LOG@"
while [ $# -gt 0 ] && [ "$1" != "cases" ]; do shift; done
shift
relay=$(cat "@RELAY@" 2>/dev/null || echo 0)
if [ "$relay" -gt 0 ]; then
    echo $((relay - 1)) > "@RELAY@"
    sh -c 'echo $$ > "@RELAYPID@"; exec cat' |
        CASES_STORE="@FAR@" perl -e 'setpgrp; exec @ARGV or die "$ARGV[0]: $!\n"' "@BIN@" "$@"
    exit $?
fi
spare=$(cat "@SPARE@" 2>/dev/null || echo 0)
if [ "$spare" -gt 0 ]; then
    echo $((spare - 1)) > "@SPARE@"
    CASES_STORE="@FAR@" exec "@BIN@" "$@"
fi
drops=$(cat "@DROP@" 2>/dev/null || echo 0)
if [ "$drops" -gt 0 ]; then
    echo $((drops - 1)) > "@DROP@"
    # A dash without job control gives a job in the background /dev/null
    # for its stdin, before its own redirections: the copy on 3 keeps ours.
    exec 3<&0
    CASES_STORE="@FAR@" "@BIN@" "$@" <&3 3<&- &
    pid=$!
    sleep 1
    kill $pid
    exit 255
fi
CASES_STORE="@FAR@" exec "@BIN@" "$@"
"#;

impl Forwarding {
    fn new() -> Forwarding {
        Forwarding::with_ssh(SSH)
    }

    /// A home whose ssh is the script `script`, with `@LOG@`, `@DROP@`,
    /// `@SPARE@`, `@FAR@` and `@BIN@` filled in.
    fn with_ssh(script: &str) -> Forwarding {
        let mut env = Env::new();
        let far = env.path("far.db");
        let bin = env.path("bin");
        fs::create_dir_all(&bin).unwrap();
        let script = script
            .replace("@LOG@", env.path("ssh.log").to_str().unwrap())
            .replace("@DROP@", env.path("drop").to_str().unwrap())
            .replace("@SPARE@", env.path("spare").to_str().unwrap())
            .replace("@RELAYPID@", env.path("relay.pid").to_str().unwrap())
            .replace("@RELAY@", env.path("relay").to_str().unwrap())
            .replace("@FAR@", far.to_str().unwrap())
            .replace("@BIN@", env!("CARGO_BIN_EXE_cases"));
        let ssh = bin.join("ssh");
        fs::write(&ssh, script).unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        env.set("PATH", &format!("{}:/usr/bin:/bin", bin.display()));
        env.set("CASES_FORWARD", "ssh://devbox");
        Forwarding { env, far }
    }

    /// The `exec:` forward that runs this build's `cases dial-stdio` on the
    /// far side's store, through `sh -c`, as a user's command would.
    fn exec(&self) -> String {
        format!(
            "exec:CASES_STORE='{}' exec '{}' dial-stdio",
            self.far.display(),
            env!("CARGO_BIN_EXE_cases")
        )
    }

    /// A home that forwards over the command [`Forwarding::exec`] names,
    /// not ssh.
    fn over_exec() -> Forwarding {
        let mut f = Forwarding::new();
        let exec = f.exec();
        f.env.set("CASES_FORWARD", &exec);
        f
    }

    /// Runs `cases` with `args`, forwarded.
    fn run(&self, args: &[&str]) -> Output {
        self.env.run(args)
    }

    /// Runs `cases` with `args`, which must succeed, and gives its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.code, 0, "cases {args:?}: {out:?}");
        out.stdout
    }

    /// Runs `cases --store <far>` with `args` on the far side's file
    /// itself, and gives its stdout.
    fn far(&self, args: &[&str]) -> String {
        let mut all = vec!["--store", self.far.to_str().unwrap()];
        all.extend(args);
        self.ok(&all)
    }

    /// What the stand-in ssh was run with, a line for each run.
    fn ssh_runs(&self) -> Vec<String> {
        fs::read_to_string(self.env.path("ssh.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn open_decision(&self) -> String {
        self.ok(&[
            "open",
            "--kind",
            "decision",
            "--urgency",
            "today",
            "--title",
            "Ship it?",
            "--option",
            "yes",
            "--option",
            "no",
            "--worker",
            "lead",
        ])
        .trim()
        .to_owned()
    }
}

#[test]
fn a_case_goes_round_over_ssh() {
    let f = Forwarding::new();
    assert_eq!(f.ok(&["list"]), "No cases.\n");
    let id = f.open_decision();
    // The case is in the far side's store, and only there.
    assert!(f.far(&["list"]).contains("Ship it?"));
    assert!(!f.env.store.exists());

    let waiting = spawn(
        f.env
            .command([
                "wait",
                "--id",
                &id,
                "--pickup",
                "--worker",
                "lead",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    thread::sleep(Duration::from_millis(500));
    let answered = Instant::now();
    assert_eq!(
        f.ok(&["answer", &id, "--option", "1", "--as", "ryan"]),
        format!("{id} answered\n")
    );
    let waited: Output = waiting.wait_with_output().unwrap().into();
    assert_eq!(waited.code, 0, "{waited:?}");
    assert!(
        waited.stdout.contains(r#""state":"pickedup""#),
        "{waited:?}"
    );
    // The held poll wakes the wait: it does not sleep out a poll.
    assert!(answered.elapsed() < Duration::from_secs(5));

    assert_eq!(
        f.ok(&["close", &id, "--outcome", "shipped"]),
        format!("{id} closed\n")
    );
    let shown = f.ok(&["show", &id]);
    assert!(shown.contains("state:    closed"), "{shown}");
    // The human is named as on a local store, by --as.
    assert!(shown.contains("by ryan"), "{shown}");
    assert_eq!(f.far(&["show", &id]), shown);

    let runs = f.ssh_runs();
    assert!(!runs.is_empty());
    for run in &runs {
        assert_eq!(run, "-T -o BatchMode=yes -- devbox cases dial-stdio");
    }
}

#[test]
fn a_case_goes_round_over_a_command() {
    let f = Forwarding::over_exec();
    assert_eq!(f.ok(&["list"]), "No cases.\n");
    let id = f.open_decision();
    assert!(f.far(&["list"]).contains("Ship it?"));
    assert!(!f.env.store.exists());
    let waiting = spawn(
        f.env
            .command([
                "wait",
                "--id",
                &id,
                "--pickup",
                "--worker",
                "lead",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    thread::sleep(Duration::from_millis(500));
    f.ok(&["answer", &id, "--option", "1", "--as", "ryan"]);
    let waited: Output = waiting.wait_with_output().unwrap().into();
    assert_eq!(waited.code, 0, "{waited:?}");
    assert!(
        waited.stdout.contains(r#""state":"pickedup""#),
        "{waited:?}"
    );
    assert!(f.far(&["show", &id]).contains("state:    pickedup"));
    // ssh is never run.
    assert!(f.ssh_runs().is_empty());

    // --store takes a command for one command, as it takes ssh://host.
    let mut f = f;
    f.env.unset("CASES_FORWARD");
    let exec = f.exec();
    f.ok(&[
        "--store",
        &exec,
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "one off",
    ]);
    assert!(f.far(&["list"]).contains("one off"));
    assert!(!f.env.store.exists());
    assert!(f.ssh_runs().is_empty());
}

#[test]
fn a_command_that_fails_is_named_and_not_called_ssh() {
    let mut f = Forwarding::new();
    f.env.set(
        "CASES_FORWARD",
        "exec:no-such-command-here box cases dial-stdio",
    );
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.starts_with(
            "Error: could not reach the store on exec:no-such-command-here box cases \
             dial-stdio: a command was not found (exit 127): "
        ),
        "{out:?}"
    );
    assert!(
        out.stderr.contains("no-such-command-here: not found"),
        "{out:?}"
    );
    assert!(
        out.stderr
            .ends_with("; the command must run cases dial-stdio on the far side\n"),
        "{out:?}"
    );
    assert!(!out.stderr.contains("ssh"), "{out:?}");
    // One that exits 255 is not said to be ssh.
    f.env.set(
        "CASES_FORWARD",
        "exec:echo 'Permission denied' >&2; exit 255",
    );
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(!out.stderr.contains("ssh"), "{out:?}");
    assert!(
        out.stderr
            .contains("the connection closed (exit 255): Permission denied"),
        "{out:?}"
    );
    // A command is needed.
    f.env.set("CASES_FORWARD", "exec:");
    let out = f.run(&["list"]);
    assert_eq!(
        out.stderr,
        "Error: CASES_FORWARD: \"exec:\" is not a forward: exec: has no command; it is \
         ssh://[user@]host[:port], exec:<command> for a command that runs cases dial-stdio on \
         the far side, or http://host[:port] for a cases serve there\n"
    );
    assert!(f.ssh_runs().is_empty());
}

/// A pickup wait over a command holds its locks on the far side on a
/// lease, as over ssh: one that ends lets them go at once, and one killed
/// outright keeps them until the lease runs out.
#[test]
fn a_pickup_wait_over_a_command_takes_and_lets_go_of_its_lease() {
    let mut f = leasing(3000);
    let exec = f.exec();
    f.env.set("CASES_FORWARD", &exec);
    let opened_over_ssh = f.ssh_runs().len();
    let first = picking_up(&f);
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    assert!(
        again.stderr.contains(&format!("(the store on {exec})")),
        "{again:?}"
    );
    // It renews the lease, so it holds the locks past it.
    thread::sleep(Duration::from_millis(3500));
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    signal("TERM", first.id());
    let waited = first.wait_with_output().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&waited.status),
        Some(15),
        "{waited:?}"
    );
    let ended = Instant::now();
    let started = starts_within(&f, Duration::from_secs(10));
    assert!(
        started - ended < Duration::from_millis(1500),
        "{:?}",
        started - ended
    );

    // The lock file names the last wait until another takes it.
    let locks = f.far.with_file_name("far.db-waits");
    let before = holder(&locks, "worker-lead-");
    let mut killed = picking_up(&f);
    let until = Instant::now() + Duration::from_secs(10);
    while holder(&locks, "worker-lead-").is_none_or(|now| Some(&now) == before.as_ref()) {
        assert!(Instant::now() < until, "the wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    let (lock, holder) = taken(&f, "worker-lead-");
    let early = kept_at(&lock, &holder, Instant::now() + Duration::from_secs(1));
    killed.kill().unwrap();
    killed.wait().unwrap();
    assert!(early.join().unwrap(), "let go within 1 s of the kill");
    starts_within(&f, Duration::from_secs(10));
    assert_eq!(f.ssh_runs().len(), opened_over_ssh);
}

#[test]
fn a_user_and_a_port_go_to_ssh() {
    let mut f = Forwarding::new();
    f.env.set("CASES_FORWARD", "ssh://ryan@devbox:2222/");
    f.ok(&["list"]);
    assert_eq!(
        f.ssh_runs(),
        ["-T -o BatchMode=yes -p 2222 -- ryan@devbox cases dial-stdio"]
    );
}

#[test]
fn which_store_is_forwarded() {
    let mut f = Forwarding::new();
    let local = f.env.store.to_str().unwrap().to_owned();
    let open = ["open", "--kind", "fyi", "--urgency", "today", "--title"];
    let titled = |title: &str| -> Vec<String> {
        open.iter()
            .map(|s| (*s).to_owned())
            .chain([title.to_owned()])
            .collect()
    };
    let run = |f: &Forwarding, args: &[String]| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        f.ok(&args)
    };

    // CASES_FORWARD comes before CASES_STORE.
    f.env.set("CASES_STORE", &local);
    run(&f, &titled("forwarded"));
    assert!(f.far(&["list"]).contains("forwarded"));
    assert!(!f.env.store.exists());

    // --store comes before CASES_FORWARD.
    let mut flagged = vec!["--store".to_owned(), local.clone()];
    flagged.extend(titled("flagged"));
    run(&f, &flagged);
    assert!(f.env.store.exists());
    assert!(!f.far(&["list"]).contains("flagged"));

    // CASES_STORE comes before the config file's forward.
    f.env.unset("CASES_FORWARD");
    f.env
        .config("forward = \"ssh://devbox\"\nstore = \"/nowhere/cases.db\"\n");
    run(&f, &titled("by env"));
    assert!(!f.far(&["list"]).contains("by env"));

    // The config file's forward comes before its store.
    f.env.unset("CASES_STORE");
    run(&f, &titled("by config"));
    assert!(f.far(&["list"]).contains("by config"));

    // --store takes a forward for one command.
    f.env.config("");
    let mut one_off = vec!["--store".to_owned(), "ssh://devbox".to_owned()];
    one_off.extend(titled("one off"));
    run(&f, &one_off);
    assert!(f.far(&["list"]).contains("one off"));

    // config show says where the forward came from.
    f.env.set("CASES_FORWARD", "ssh://devbox");
    let shown = f.ok(&["config", "show"]);
    assert!(
        shown
            .lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["forward", "ssh://devbox", "env"]),
        "{shown}"
    );
}

#[test]
fn a_forward_that_is_not_one_is_refused() {
    let mut f = Forwarding::new();
    f.env.set("CASES_FORWARD", "ssh://-oProxyCommand=x");
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: CASES_FORWARD: \"ssh://-oProxyCommand=x\" is not a forward: the host is not \
         one; it is ssh://[user@]host[:port], exec:<command> for a command that runs cases \
         dial-stdio on the far side, or http://host[:port] for a cases serve there\n"
    );
    f.env.unset("CASES_FORWARD");
    f.env.config("forward = \"devbox\"\n");
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .starts_with("Error: the config file's forward: \"devbox\" is not a forward"),
        "{out:?}"
    );
    assert!(f.ssh_runs().is_empty());
}

/// A stand-in ssh that fails as `stderr` and `code` say.
fn failing(stderr: &str, code: i32) -> Forwarding {
    Forwarding::with_ssh(&format!(
        "#!/bin/sh\necho \"$*\" >> \"@LOG@\"\necho \"{stderr}\" >&2\nexit {code}\n"
    ))
}

#[test]
fn a_far_side_with_no_cases_is_said_at_once() {
    let f = failing("bash: line 1: cases: command not found", 127);
    for args in [
        &["list"][..],
        &[
            "open",
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            "x",
        ],
    ] {
        let out = f.run(args);
        assert_eq!(out.code, 1, "{out:?}");
        assert_eq!(
            out.stderr,
            "Error: could not reach the store on ssh://devbox: there is no cases on the far \
             side's PATH (exit 127): bash: line 1: cases: command not found\n"
        );
    }
    // Neither the read nor the write is tried again.
    assert_eq!(f.ssh_runs().len(), 2);
}

#[test]
fn a_far_side_too_old_for_dial_stdio_is_said_at_once() {
    let f = failing("error: unrecognized subcommand 'dial-stdio'", 2);
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: could not reach the store on ssh://devbox: the cases on the far side has no \
         dial-stdio: upgrade it there\n"
    );
    assert_eq!(f.ssh_runs().len(), 1);
}

#[test]
fn an_ssh_that_fails_is_tried_again_for_a_read_and_not_a_write() {
    let f = failing(
        "ssh: connect to host devbox port 22: Connection refused",
        255,
    );
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: could not reach the store on ssh://devbox: ssh failed (exit 255): ssh: connect \
         to host devbox port 22: Connection refused\n"
    );
    // The read is sent three times.
    assert_eq!(f.ssh_runs().len(), 3);
    let out = f.run(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "x",
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .ends_with("; the write may or may not have been stored: check with cases show\n"),
        "{out:?}"
    );
    assert_eq!(f.ssh_runs().len(), 4);
}

/// Picks up a new decision with a wait while `drops` connections drop, one
/// after another, then answers it, and gives what the wait did. The pickup
/// wait's first connection, which holds its locks, is spared: the drops
/// are of the connections its reads go over.
fn wait_through_drops(drops: usize) -> (Forwarding, Output) {
    let f = Forwarding::new();
    let id = f.open_decision();
    let opened = f.ssh_runs().len();
    fs::write(f.env.path("spare"), "1").unwrap();
    fs::write(f.env.path("drop"), drops.to_string()).unwrap();
    let waiting = spawn(
        f.env
            .command([
                "wait",
                "--id",
                &id,
                "--pickup",
                "--worker",
                "lead",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    // The locks' connection, then each dropped connection is dialled
    // again, and then one holds.
    let until = Instant::now() + Duration::from_secs(30);
    while f.ssh_runs().len() < opened + 1 + drops + 1 && Instant::now() < until {
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(Duration::from_millis(300));
    f.ok(&["answer", &id, "--option", "2", "--as", "ryan"]);
    let waited: Output = waiting.wait_with_output().unwrap().into();
    assert_eq!(waited.code, 0, "{waited:?}");
    assert!(waited.stdout.contains(r#""choice":2"#), "{waited:?}");
    assert!(
        waited.stdout.contains(r#""state":"pickedup""#),
        "{waited:?}"
    );
    // Its locks were taken on the far side, over the spared connection.
    assert!(held(&f.far.with_file_name("far.db-waits"), "worker-lead-"));
    (f, waited)
}

#[test]
fn a_dropped_connection_is_dialled_again_as_a_read_is_retried() {
    let (_, waited) = wait_through_drops(1);
    assert_eq!(waited.stderr, "", "{waited:?}");
}

#[test]
fn a_wait_rides_out_a_far_side_it_cannot_reach_for_a_while() {
    // The read and its two retries fail: the wait says so once, and goes
    // on until the far side is back.
    let (_, waited) = wait_through_drops(3);
    assert_eq!(
        waited.stderr,
        "warning: could not reach the store on ssh://devbox: ssh failed (exit 255); the wait \
         goes on, trying again until the timeout\n"
    );
}

#[test]
fn a_listening_wait_is_recorded_at_the_far_side() {
    let f = Forwarding::new();
    f.open_decision();
    let mut waiting = spawn(
        f.env
            .command([
                "wait",
                "--pickup",
                "--worker",
                "docs",
                "--doing",
                "reading",
                "--timeout",
                "30s",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    // Its doing line can be set over the pipe once it listens.
    let until = Instant::now() + Duration::from_secs(10);
    let mut set = f.run(&["doing", "--worker", "docs", "writing"]);
    while set.code != 0 && Instant::now() < until {
        thread::sleep(Duration::from_millis(100));
        set = f.run(&["doing", "--worker", "docs", "writing"]);
    }
    let _ = waiting.kill();
    let _ = waiting.wait();
    assert_eq!(set.code, 0, "{set:?}");
    // A worker with no wait is not listening.
    let out = f.run(&["doing", "--worker", "nobody", "writing"]);
    assert_ne!(out.code, 0, "{out:?}");
}

#[test]
fn the_inbox_of_a_forwarded_store_is_served_where_it_is() {
    let f = Forwarding::new();
    assert_eq!(
        f.ok(&["status"]),
        "Forwarded to ssh://devbox: its inbox is served there, by cases serve\n"
    );
    assert_eq!(
        f.ok(&["status", "--json"]),
        "{\n  \"forward\": \"ssh://devbox\"\n}\n"
    );
    for (args, want) in [
        (
            &["serve", "--no-open"][..],
            "Error: serve runs against a local store: run cases serve on ssh://devbox, where \
             the store is\n",
        ),
        (
            &["inbox"][..],
            "Error: a forwarded store's inbox is served on ssh://devbox, where the store is: \
             run cases inbox there\n",
        ),
        (
            &["service", "install"][..],
            "Error: a forwarded store needs no local service: install it on ssh://devbox, \
             where the store is\n",
        ),
    ] {
        let out = f.run(args);
        assert_eq!((out.code, out.stderr.as_str()), (1, want), "{args:?}");
    }
    assert!(f.ssh_runs().is_empty());
}

/// Gives the far side a serve on its store, as `cases` finds one, whose
/// instance file's link is `link`, or that has none when `link` is empty.
/// It runs while the listener is kept.
fn far_serve(f: &Forwarding, link: &str) -> std::net::TcpListener {
    let (listener, _) = f.env.fake_serve_on(&f.far, "127.0.0.1:0");
    let file = f.env.instance_file_of(&f.far);
    let mut info = json(&fs::read_to_string(&file).unwrap());
    if !link.is_empty() {
        info["link"] = link.into();
    }
    fs::write(&file, info.to_string()).unwrap();
    listener
}

/// The `url` that `show --json` gives for the case `id`, forwarded, and
/// what it said on stderr.
fn shown_url(f: &Forwarding, id: &str) -> (String, String) {
    let out = f.run(&["show", id, "--json"]);
    assert_eq!(out.code, 0, "{out:?}");
    let url = json(&out.stdout)["url"].as_str().unwrap().to_owned();
    (url, out.stderr)
}

/// A forwarded command's links are the far side's: its serve's link, an
/// allowed host's, over ssh and over a command alike; none when no serve
/// runs there, or the one there has no allowed host, since its loopback
/// address is no use here (IN-11, ST-11).
#[test]
fn a_forwarded_command_links_to_the_far_sides_inbox() {
    let link = "https://devbox.example.ts.net:7337/";
    for f in [Forwarding::new(), Forwarding::over_exec()] {
        let id = f.open_decision();
        let page = format!("{link}cases/{id}-ship-it");
        assert_eq!(shown_url(&f, &id), (String::new(), String::new()));
        assert!(!f.ok(&["show", &id]).contains("url:"));
        let out = f.run(&["inbox", "--print", &id]);
        assert_eq!(out.code, 1, "{out:?}");
        assert!(out.stderr.contains("run cases inbox there"), "{out:?}");
        // A case that is not there is said, link or not.
        let out = f.run(&["inbox", "--print", "zzzzzz"]);
        assert_eq!(out.code, 1, "{out:?}");
        assert!(!out.stderr.contains("run cases inbox there"), "{out:?}");

        let serve = far_serve(&f, "");
        assert_eq!(shown_url(&f, &id), (String::new(), String::new()));
        drop(serve);

        let _serve = far_serve(&f, link);
        assert_eq!(shown_url(&f, &id), (page.clone(), String::new()));
        assert!(
            f.ok(&["show", &id])
                .contains(&format!("url:      {page}\n")),
            "{id}"
        );
        assert_eq!(f.ok(&["inbox", "--print"]), format!("{link}\n"));
        assert_eq!(f.ok(&["inbox", "--print", &id]), format!("{page}\n"));
        // Plain inbox still opens nothing here.
        assert_eq!(f.run(&["inbox", &id]).code, 1);
    }
}

/// A far side from before `GET store/inbox` answers it 404: no link, and
/// no warning, as before it was asked.
#[test]
fn a_far_side_too_old_to_give_its_link_gives_none() {
    // The far side is this build, with the route's name changed on its way
    // there to one it does not have.
    let f = Forwarding::with_ssh(
        r#"#!/bin/sh
echo "$*" >> "@LOG@"
while [ $# -gt 0 ] && [ "$1" != "cases" ]; do shift; done
shift
sed -u 's#/store/inbox #/store/no-inbox #' | CASES_STORE="@FAR@" exec "@BIN@" "$@"
"#,
    );
    // Opened on the far side itself, so that only reads, whose requests end
    // in a line, go through the sed.
    let id = f.far(&[
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Old",
    ]);
    let id = id.trim();
    let _serve = far_serve(&f, "https://devbox.example.ts.net/");
    assert!(
        f.far(&["show", id, "--json"])
            .contains("https://devbox.example.ts.net/")
    );
    assert_eq!(shown_url(&f, id), (String::new(), String::new()));
}

/// `sweep --stale-briefings` reads the age from the config file, which on
/// a forwarded store is the far side's: it is refused here, and serve
/// there does it each hour (AG-19).
#[test]
fn a_forwarded_sweep_of_stale_briefings_is_refused() {
    let exec = Forwarding::over_exec();
    let forward = exec.exec();
    for (f, forward) in [
        (Forwarding::new(), "ssh://devbox".to_owned()),
        (exec, forward),
    ] {
        let out = f.run(&["sweep", "--stale-briefings", "--yes"]);
        assert_eq!(out.code, 1, "{out:?}");
        assert_eq!(
            out.stderr,
            format!(
                "Error: sweep --stale-briefings runs where the store is: serve on {forward} \
                 closes stale briefings each hour, or run cases sweep --stale-briefings there\n"
            )
        );
        assert!(f.ssh_runs().is_empty());
    }
}

#[test]
fn dial_stdio_serves_the_local_store_alone() {
    let mut f = Forwarding::new();
    let local = f.env.store.to_str().unwrap().to_owned();
    // A forward set where dial-stdio runs is not followed.
    f.env.set("CASES_STORE", &local);
    let request = "GET /v1/stores/st_dialstdio0000000/store/changes HTTP/1.1\r\n\r\n";
    let out = f.env.run_with_stdin(request, ["dial-stdio"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n\
         {\"version\":\"0\"}"
    );
    assert!(f.ssh_runs().is_empty());
    for store in [
        "ssh://devbox",
        "https://cases.sh/me",
        "exec:cases dial-stdio",
    ] {
        let out = f.env.run_with_stdin("", ["--store", store, "dial-stdio"]);
        assert_eq!(out.code, 1, "{out:?}");
        assert_eq!(
            out.stderr,
            format!(
                "Error: {store:?} is not a local store's file: dial-stdio serves a local store\n"
            )
        );
    }
    // It is not in the help.
    let help = f.env.run(["--help"]);
    assert!(!help.stdout.contains("dial-stdio"), "{help:?}");
}

#[test]
fn a_far_side_that_closes_mid_answer_is_a_lost_connection() {
    // An answer cut short: a status line and half a body, then the end.
    let f = Forwarding::with_ssh(
        "#!/bin/sh\necho \"$*\" >> \"@LOG@\"\nhead -c 1 >/dev/null\nprintf 'HTTP/1.1 200 OK\\r\\nContent-Length: 50\\r\\n\\r\\n{\"ver'\nexit 0\n",
    );
    let out = f.run(&["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr
            .starts_with("Error: could not reach the store on ssh://devbox: "),
        "{out:?}"
    );
    let _ = std::io::stderr().flush();
}

/// A second pickup wait on what a first is picking up from the same
/// forwarded store refuses to start, as on a local store (AG-05): the
/// locks are taken on the far side, and held while the first runs.
#[test]
fn a_second_pickup_wait_on_a_forwarded_store_is_refused() {
    let f = Forwarding::new();
    let id = f.open_decision();
    let wait = |worker: &str| {
        spawn(
            f.env
                .command([
                    "wait",
                    "--pickup",
                    "--worker",
                    worker,
                    "--id",
                    &id,
                    "--timeout",
                    "30s",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
    };
    let first = wait("lead");
    // The first holds its locks on the far side once its lock file there
    // names it.
    let locks = f.far.with_file_name("far.db-waits");
    let until = Instant::now() + Duration::from_secs(10);
    while !held(&locks, "worker-lead-") {
        assert!(Instant::now() < until, "the first wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    let second = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert_eq!(second.code, 1, "{second:?}");
    assert_eq!(second.stdout, "", "{second:?}");
    assert!(
        second
            .stderr
            .starts_with("Error: another wait is already picking up worker \"lead\" (pid "),
        "{second:?}"
    );
    assert!(
        second.stderr.ends_with(
            "; read its output instead of starting a second (the store on ssh://devbox)\n"
        ),
        "{second:?}"
    );
    // So is one on the same id under another worker.
    let out = f.run(&[
        "wait",
        "--pickup",
        "--worker",
        "docs",
        "--id",
        &id,
        "--timeout",
        "1s",
    ]);
    assert!(
        out.stderr
            .contains(&format!("another wait is already picking up id \"{id}\"")),
        "{out:?}"
    );
    // A wait on another worker is not.
    let other = f.run(&["wait", "--pickup", "--worker", "docs", "--timeout", "1s"]);
    assert!(!other.stderr.contains("already picking up"), "{other:?}");
    // The first picks up the answer as it comes, and once it has ended a
    // pickup wait on its worker starts.
    f.ok(&["answer", &id, "--option", "1", "--as", "ryan"]);
    let waited: Output = first.wait_with_output().unwrap().into();
    assert_eq!(waited.code, 0, "{waited:?}");
    assert!(waited.stdout.contains(r#""choice":1"#), "{waited:?}");
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(!again.stderr.contains("already picking up"), "{again:?}");
}

/// A pickup wait killed outright, with no chance to let go, keeps its
/// locks on the far side until their lease runs out, and no longer: its
/// connection for them ended, but a pickup of its could still have been on
/// its way over another.
#[test]
fn a_killed_pickup_wait_keeps_its_locks_on_the_far_side_until_their_lease_runs_out() {
    let f = leasing(3000);
    let mut first = picking_up(&f);
    let (lock, holder) = taken(&f, "worker-lead-");
    // Renewed a second at most before it was killed, the lease runs out two
    // seconds after at the soonest: a second in, it is held still.
    let early = kept_at(&lock, &holder, Instant::now() + Duration::from_secs(1));
    first.kill().unwrap();
    first.wait().unwrap();
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    assert!(early.join().unwrap(), "let go within 1 s of the kill");
    starts_within(&f, Duration::from_secs(10));
}

/// A pickup wait whose connection for its locks dies, while the wait goes
/// on, leaves the far side refusing a second pickup wait until their lease
/// runs out, and then taking it: the first could have had a pickup on its
/// way over another connection. The first stops before the second starts.
///
/// Both are read off the lock itself, not off when a second wait was
/// started: a wait is started some time before it takes the lock, the more
/// so on a busy machine.
#[test]
fn a_pickup_wait_whose_locks_connection_is_killed_holds_off_a_second_until_the_lease_ends() {
    let f = leasing(3000);
    fs::write(f.env.path("relay"), "1").unwrap();
    let first = picking_up(&f);
    let (lock, holder) = taken(&f, "worker-lead-");
    let relay: u32 = fs::read_to_string(f.env.path("relay.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // The relay is what carries the connection to the far side's
    // dial-stdio, which sees it end.
    // Renewed a second at most before the kill, the lease runs out two
    // seconds after it at the soonest: a second in, it is held still.
    let early = kept_at(&lock, &holder, Instant::now() + Duration::from_secs(1));
    signal("KILL", relay);
    let ended = thread::spawn({
        let (lock, holder) = (lock.clone(), holder.clone());
        move || {
            let waited: Output = first.wait_with_output().unwrap().into();
            (kept(&lock, &holder), waited)
        }
    });
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(again.stderr.contains("already picking up"), "{again:?}");
    assert!(early.join().unwrap(), "let go within 1 s of the kill");
    starts_within(&f, Duration::from_secs(10));
    let (kept_as_it_ended, waited) = ended.join().unwrap();
    lapsed(&waited);
    assert!(
        kept_as_it_ended,
        "the first ended after its locks were let go"
    );
}

/// The far side's lock file whose name starts with `prefix`, and the line
/// its holder's take wrote in it, once all of it is there: a take writes
/// it in parts.
fn taken(f: &Forwarding, prefix: &str) -> (PathBuf, String) {
    let lock = fs::read_dir(f.far.with_file_name("far.db-waits"))
        .unwrap()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .unwrap()
        .path();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let holder = fs::read_to_string(&lock).unwrap();
        if holder.ends_with('\n') {
            return (lock, holder);
        }
        assert!(
            Instant::now() < until,
            "its holder is not written: {holder:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether, at `at`, the lock `lock` is held still by the wait whose take
/// wrote `holder` in it, read on a thread of its own so that nothing the
/// test does meanwhile makes it late.
fn kept_at(lock: &std::path::Path, holder: &str, at: Instant) -> thread::JoinHandle<bool> {
    let (lock, holder) = (lock.to_owned(), holder.to_owned());
    thread::spawn(move || {
        thread::sleep(at.saturating_duration_since(Instant::now()));
        kept(&lock, &holder)
    })
}

/// Whether the lock `lock` is held still by the wait whose take wrote
/// `holder` in it: another's take writes its own. When it is free, taking
/// it to see lets it go again at once. A take that has just locked it and
/// not yet written is taken as the holder still, for those few
/// microseconds.
fn kept(lock: &std::path::Path, holder: &str) -> bool {
    let file = fs::File::open(lock).unwrap();
    let busy = matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock));
    busy && fs::read_to_string(lock).unwrap() == holder
}

/// A pickup wait that ends, however it ends, lets go of its locks on the
/// far side at once, though their lease there is long: the next pickup
/// wait starts straight after.
#[test]
fn a_pickup_wait_that_ends_lets_go_of_its_locks_at_once() {
    let f = Forwarding::new();
    f.open_decision();
    let free = |after: &str| {
        let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
        assert!(
            !again.stderr.contains("already picking up"),
            "after {after}: {again:?}"
        );
    };
    // One that fails, as one that times out does.
    let timed_out = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert_ne!(timed_out.code, 0, "{timed_out:?}");
    free("a timeout");
    // One stopped by a signal, sent to it alone, or to its process group,
    // as a Ctrl-C at the terminal sends it to its ssh too.
    let locks = f.far.with_file_name("far.db-waits");
    // SIGHUP is a terminal, or a tmux pane, closed.
    for (name, number, group) in [
        ("INT", 2, false),
        ("TERM", 15, false),
        ("HUP", 1, false),
        ("INT", 2, true),
    ] {
        let before = holder(&locks, "worker-lead-");
        let mut command =
            f.env
                .command(["wait", "--pickup", "--worker", "lead", "--timeout", "60s"]);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if group {
            std::os::unix::process::CommandExt::process_group(&mut command, 0);
        }
        let first = spawn(&mut command);
        // Its lock names a holder other than the last wait's.
        let until = Instant::now() + Duration::from_secs(10);
        while holder(&locks, "worker-lead-").is_none_or(|now| Some(&now) == before.as_ref()) {
            assert!(Instant::now() < until, "the first wait took no lock");
            thread::sleep(Duration::from_millis(50));
        }
        let target = if group {
            format!("-{}", first.id())
        } else {
            first.id().to_string()
        };
        let sent = std::process::Command::new("kill")
            .args([format!("-{name}"), "--".to_owned(), target])
            .status()
            .unwrap();
        assert!(sent.success());
        let waited = first.wait_with_output().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&waited.status),
            Some(number),
            "SIG{name}: {waited:?}"
        );
        free(&format!("SIG{name}"));
    }
}

/// A pickup wait stopped by a Ctrl-C lets go of its locks and then dies by
/// the signal, as one with no handler does, so that a bash script around it
/// stops too: bash goes on past a child that exits 130, but not past one
/// that SIGINT ends.
#[test]
fn a_ctrl_c_to_a_script_around_a_pickup_wait_stops_the_script() {
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

    let f = Forwarding::new();
    f.open_decision();
    let locks = f.far.with_file_name("far.db-waits");
    let wait = f
        .env
        .command(["wait", "--pickup", "--worker", "lead", "--timeout", "60s"]);
    let mut script = std::process::Command::new("/bin/bash");
    script
        .args(["-c", r#""$@"; echo "went on past $?""#, "bash"])
        .arg(wait.get_program())
        .args(wait.get_args())
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for (key, value) in wait.get_envs() {
        if let Some(value) = value {
            script.env(key, value);
        }
    }
    if let Some(dir) = wait.get_current_dir() {
        script.current_dir(dir);
    }
    let first = spawn(&mut script);
    let until = Instant::now() + Duration::from_secs(10);
    while holder(&locks, "worker-lead-").is_none() {
        assert!(Instant::now() < until, "the wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    let sent = std::process::Command::new("kill")
        .args(["-INT", "--", &format!("-{}", first.id())])
        .status()
        .unwrap();
    assert!(sent.success());
    let ended = first.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&ended.stdout);
    assert!(!stdout.contains("went on past"), "{ended:?}");
    assert_eq!(ended.status.signal(), Some(2), "{ended:?}");
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(!again.stderr.contains("already picking up"), "{again:?}");
}

/// A pickup wait stopped by a signal after it took a message, and before it
/// printed it, names it on stderr as it dies: the message is picked up, and
/// no later wait takes it again (AG-16).
#[test]
fn a_pickup_wait_stopped_by_a_signal_names_the_messages_it_took() {
    let f = Forwarding::new();
    f.open_decision();
    let id = f
        .ok(&[
            "message", "--as", "Ryan", "--title", "Rebase", "--to", "lead",
        ])
        .trim()
        .to_owned();
    let first = spawn(
        f.env
            .command([
                "wait",
                "--pickup",
                "--worker",
                "lead",
                "--by",
                "lead",
                "--mood",
                "steady",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    // It takes the message at its first read, then lingers.
    let until = Instant::now() + Duration::from_secs(10);
    while !f
        .far(&["show", &id, "--json"])
        .contains(r#""state": "pickedup""#)
    {
        assert!(Instant::now() < until, "the wait took no message");
        thread::sleep(Duration::from_millis(50));
    }
    signal("INT", first.id());
    let waited = first.wait_with_output().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&waited.status),
        Some(2),
        "{waited:?}"
    );
    let stderr = String::from_utf8_lossy(&waited.stderr);
    assert_eq!(
        stderr,
        format!("warning: {id}: picked up, but not printed: a signal ended the wait\n"),
        "{waited:?}"
    );
    let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
    assert!(!again.stderr.contains("already picking up"), "{again:?}");
}

/// A pickup wait started with SIGHUP ignored, as `nohup` starts it, keeps
/// it ignored: its terminal closing does not end it.
#[test]
fn a_pickup_wait_started_under_nohup_outlives_a_hangup() {
    let f = Forwarding::new();
    f.open_decision();
    let mut wait = picking_up_by(
        &f,
        f.env.command_as(
            std::path::Path::new("/bin/sh"),
            [
                "-c",
                "trap '' HUP; exec \"$0\" \"$@\"",
                env!("CARGO_BIN_EXE_cases"),
                "wait",
                "--pickup",
                "--worker",
                "lead",
                "--timeout",
                "60s",
            ],
        ),
    );
    signal("HUP", wait.id());
    thread::sleep(Duration::from_millis(500));
    let ended = wait.try_wait().unwrap();
    if ended.is_none() {
        signal("TERM", wait.id());
    }
    let waited = wait.wait_with_output().unwrap();
    assert_eq!(ended, None, "a hangup ended it: {waited:?}");
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&waited.status),
        Some(15),
        "{waited:?}"
    );
}

/// A signal ends a pickup wait whose stdout is full, such as a pipe nobody
/// reads, rather than wait on it to print.
#[test]
fn a_pickup_wait_stuck_printing_is_ended_by_a_signal() {
    let f = Forwarding::new();
    f.open_decision();
    // More than a pipe holds, so that printing it blocks.
    let body = "x".repeat(100_000);
    let id = f
        .ok(&[
            "message", "--as", "Ryan", "--title", "Big", "--body", &body, "--to", "lead",
        ])
        .trim()
        .to_owned();
    let mut wait = spawn(
        f.env
            .command([
                "wait",
                "--pickup",
                "--worker",
                "lead",
                "--by",
                "lead",
                "--mood",
                "eager",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let until = Instant::now() + Duration::from_secs(10);
    while !f
        .far(&["show", &id, "--json"])
        .contains(r#""state": "pickedup""#)
    {
        assert!(Instant::now() < until, "the wait took no message");
        thread::sleep(Duration::from_millis(50));
    }
    // Long enough to be stuck in the line.
    thread::sleep(Duration::from_millis(500));
    signal("TERM", wait.id());
    let until = Instant::now() + Duration::from_secs(5);
    let mut ended = None;
    while ended.is_none() && Instant::now() < until {
        ended = wait.try_wait().unwrap();
        thread::sleep(Duration::from_millis(50));
    }
    if ended.is_none() {
        signal("KILL", wait.id());
    }
    let waited = wait.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&waited.stderr);
    let ended = ended.unwrap_or_else(|| panic!("one signal did not end it: {stderr}"));
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&ended),
        Some(15),
        "{stderr}"
    );
    // It was cut off mid-line, so it is named as not printed.
    assert!(
        stderr.contains(&format!("{id}: picked up, but not printed")),
        "{stderr}"
    );
}

/// What the lock file in `dir` whose name starts with `prefix` says of its
/// holder, if it names one.
fn holder(dir: &std::path::Path, prefix: &str) -> Option<String> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        if !entry.file_name().to_string_lossy().starts_with(prefix) {
            return None;
        }
        let text = fs::read_to_string(entry.path()).ok()?;
        (!text.trim().is_empty()).then_some(text)
    })
}

/// Sends `signal`, such as `STOP`, to the process `pid`.
fn signal(signal: &str, pid: u32) {
    let sent = std::process::Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status()
        .unwrap();
    assert!(sent.success(), "kill -{signal} {pid}");
}

/// Sends CONT to the relay `pid`, which may have gone: once its stand-in
/// ssh is killed, its process group is orphaned, and the system ends it
/// for being stopped in one.
fn resume(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-CONT", &pid.to_string()])
        .status();
}

/// A home whose far side gives pickup waits a lease of `ms`, with a case
/// in its store, so that the store is there to take locks on.
fn leasing(ms: u32) -> Forwarding {
    let mut f = Forwarding::new();
    f.env.set("CASES_TEST_LEASE", &ms.to_string());
    f.open_decision();
    f
}

/// Starts a pickup wait on worker lead, and gives it once it holds its
/// locks on the far side.
fn picking_up(f: &Forwarding) -> std::process::Child {
    picking_up_by(
        f,
        f.env
            .command(["wait", "--pickup", "--worker", "lead", "--timeout", "60s"]),
    )
}

/// As [`picking_up`], with the wait run by `command`.
fn picking_up_by(f: &Forwarding, mut command: std::process::Command) -> std::process::Child {
    let first = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()));
    let locks = f.far.with_file_name("far.db-waits");
    let until = Instant::now() + Duration::from_secs(10);
    while !held(&locks, "worker-lead-") {
        assert!(Instant::now() < until, "the first wait took no lock");
        thread::sleep(Duration::from_millis(50));
    }
    first
}

/// Tries a pickup wait on worker lead every 100 ms, for up to `within`,
/// until one is not refused, and gives when the one that was not started.
fn starts_within(f: &Forwarding, within: Duration) -> Instant {
    let until = Instant::now() + within;
    loop {
        let start = Instant::now();
        let again = f.run(&["wait", "--pickup", "--worker", "lead", "--timeout", "1s"]);
        if !again.stderr.contains("already picking up") {
            return start;
        }
        assert!(Instant::now() < until, "the locks were kept: {again:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

/// The error a pickup wait whose locks on the far side may have gone ends
/// with.
fn lapsed(waited: &Output) {
    assert_ne!(waited.code, 0, "{waited:?}");
    assert_eq!(waited.stdout, "", "{waited:?}");
    assert!(
        waited
            .stderr
            .starts_with("Error: this wait's locks on the far side may have been let go ("),
        "{waited:?}"
    );
    assert!(
        waited.stderr.ends_with(
            "; it stops, as another wait could now pick up what it would (the store on \
             ssh://devbox)\n"
        ),
        "{waited:?}"
    );
}

/// A pickup wait that stops, as a laptop asleep does, with its connection
/// for its locks open, loses them on the far side once their lease runs
/// out, and the next pickup wait starts. Woken, it fails rather than pick
/// up unlocked.
#[test]
fn a_stopped_pickup_wait_loses_its_locks_once_their_lease_runs_out() {
    let f = leasing(1500);
    let mut first = picking_up(&f);
    signal("STOP", first.id());
    let stopped = Instant::now();
    starts_within(&f, Duration::from_secs(10));
    // Within the lease, and the time a wait takes to start.
    assert!(
        stopped.elapsed() < Duration::from_secs(6),
        "{:?}",
        stopped.elapsed()
    );
    // Its dial-stdio was running all along.
    assert!(first.try_wait().unwrap().is_none());
    signal("CONT", first.id());
    let waited: Output = first.wait_with_output().unwrap().into();
    lapsed(&waited);
}

/// A pickup wait whose connection for its locks dies stops, rather than
/// wait on without them.
#[test]
fn a_pickup_wait_whose_locks_connection_dies_stops() {
    let f = leasing(3000);
    let first = picking_up(&f);
    // The lock file names its holder, the far side's dial-stdio.
    let locks = f.far.with_file_name("far.db-waits");
    let file = fs::read_dir(&locks)
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("worker-lead-")
        })
        .unwrap();
    let text = fs::read_to_string(file.path()).unwrap();
    let pid: u32 = text.split_whitespace().next().unwrap().parse().unwrap();
    let killed = Instant::now();
    signal("KILL", pid);
    let waited: Output = first.wait_with_output().unwrap().into();
    lapsed(&waited);
    assert!(waited.stderr.contains("its connection ended"), "{waited:?}");
    // Its poll held no longer than its locks were good for.
    assert!(
        killed.elapsed() < Duration::from_secs(3),
        "{:?}",
        killed.elapsed()
    );
}

/// A pickup wait whose renewals stop reaching the far side, as on a
/// network that drops them, stops before the far side lets its locks go,
/// so a second never runs beside it.
#[test]
fn a_pickup_wait_whose_renewals_are_lost_stops_before_a_second_can_start() {
    let f = leasing(3000);
    fs::write(f.env.path("relay"), "1").unwrap();
    let first = picking_up(&f);
    let (lock, holder) = taken(&f, "worker-lead-");
    let relay: u32 = fs::read_to_string(f.env.path("relay.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    signal("STOP", relay);
    // Read off the lock, as a second wait is started some time before it
    // takes it.
    let ended = thread::spawn({
        let (lock, holder) = (lock.clone(), holder.clone());
        move || {
            let waited: Output = first.wait_with_output().unwrap().into();
            (kept(&lock, &holder), waited)
        }
    });
    starts_within(&f, Duration::from_secs(15));
    let (kept_as_it_ended, waited) = ended.join().unwrap();
    lapsed(&waited);
    assert!(
        kept_as_it_ended,
        "the first ended after its locks were let go"
    );
    resume(relay);
}

/// A pickup wait whose locks may have gone while it lingers over a message
/// it took prints that message rather than lose it: it was taken while the
/// locks were held, and no later wait takes it again (AG-16).
#[test]
fn a_pickup_wait_whose_locks_lapse_prints_the_messages_it_took() {
    let f = leasing(3000);
    let id = f
        .ok(&[
            "message", "--as", "Ryan", "--title", "Rebase", "--to", "lead",
        ])
        .trim()
        .to_owned();
    fs::write(f.env.path("relay"), "1").unwrap();
    let first = spawn(
        f.env
            .command([
                "wait",
                "--pickup",
                "--worker",
                "lead",
                "--by",
                "lead",
                "--mood",
                "steady",
                "--timeout",
                "60s",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    // It takes the message at its first read, then lingers.
    let until = Instant::now() + Duration::from_secs(10);
    while !f
        .far(&["show", &id, "--json"])
        .contains(r#""state": "pickedup""#)
    {
        assert!(Instant::now() < until, "the wait took no message");
        thread::sleep(Duration::from_millis(50));
    }
    let relay: u32 = fs::read_to_string(f.env.path("relay.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    signal("STOP", relay);
    let waited: Output = first.wait_with_output().unwrap().into();
    resume(relay);
    assert_eq!(waited.code, 0, "{waited:?}");
    assert!(waited.stdout.contains(&id), "{waited:?}");
    // It stopped for its locks, before its linger's end.
    assert!(
        waited
            .stderr
            .starts_with("warning: this wait's locks on the far side may have been let go ("),
        "{waited:?}"
    );
}

/// Whether a lock file in `dir` whose name starts with `prefix` names its
/// holder, as it does once a wait has taken it.
fn held(dir: &std::path::Path, prefix: &str) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_name().to_string_lossy().starts_with(prefix)
                && fs::read_to_string(entry.path()).is_ok_and(|text| !text.trim().is_empty())
        })
    })
}
