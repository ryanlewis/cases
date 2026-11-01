//! Telling herdr whether cases wait on the human (TU-09, case `SiOIs`).
//! Inside a herdr pane the TUI reports its own pane as an agent: `blocked`
//! while cases wait, `idle` when none do, and released on quit. Reports run
//! on a thread of their own, and their failures are ignored: herdr missing,
//! older or refusing changes nothing in the TUI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The source the reports are made under: stable, and not `herdr:`.
const SOURCE: &str = "cases-tui";
/// The agent the pane is reported as.
const AGENT: &str = "cases";
/// The first herdr whose report takes the command that resumes the pane.
const RESUME_FROM: (u64, u64, u64) = (0, 10, 0);
/// How long quitting waits for the release to run.
const RELEASE_WAIT: Duration = Duration::from_secs(1);

/// What the reporter thread is asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    Report { blocked: bool, message: String },
    Release,
}

/// A herdr pane to report to.
#[derive(Debug)]
pub(crate) struct Herdr {
    asks: Sender<Ask>,
    thread: JoinHandle<()>,
    /// The last count reported, so that only a change is reported again.
    last: Option<(usize, usize)>,
}

impl Herdr {
    /// The pane this runs in, when it runs in herdr (`HERDR_ENV=1`, with
    /// `HERDR_BIN_PATH` and `HERDR_PANE_ID` set); `None` otherwise, and then
    /// nothing is ever run.
    pub(crate) fn from_env(store: &str) -> Option<Herdr> {
        let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        if var("HERDR_ENV").as_deref() != Some("1") {
            return None;
        }
        let bin = PathBuf::from(var("HERDR_BIN_PATH")?);
        let pane = var("HERDR_PANE_ID")?;
        let resume = vec![
            "cases".to_string(),
            "--store".to_string(),
            store.to_owned(),
            "tui".to_string(),
        ];
        let (asks, received) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("herdr".into())
            .spawn(move || reporter(&bin, &pane, &resume, &received))
            .ok()?;
        Some(Herdr {
            asks,
            thread,
            last: None,
        })
    }

    /// Reports `waiting` open cases, `blocking` of them blocking, if that
    /// is not what was last reported.
    pub(crate) fn report(&mut self, (waiting, blocking): (usize, usize)) {
        if self.last == Some((waiting, blocking)) {
            return;
        }
        self.last = Some((waiting, blocking));
        let _ = self.asks.send(report(waiting, blocking));
    }

    /// Releases the pane, waiting a moment for it to run.
    pub(crate) fn release(self) {
        let _ = self.asks.send(Ask::Release);
        drop(self.asks);
        let deadline = std::time::Instant::now() + RELEASE_WAIT;
        while !self.thread.is_finished() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The report for a count: `blocked` with "3 waiting, 1 blocking", or
/// `idle`.
fn report(waiting: usize, blocking: usize) -> Ask {
    if waiting == 0 {
        return Ask::Report {
            blocked: false,
            message: String::new(),
        };
    }
    let message = if blocking > 0 {
        format!("{waiting} waiting, {blocking} blocking")
    } else {
        format!("{waiting} waiting")
    };
    Ask::Report {
        blocked: true,
        message,
    }
}

/// Runs each report as it comes, the latest only when several queue up.
fn reporter(bin: &Path, pane: &str, resume: &[String], asks: &Receiver<Ask>) {
    let takes_resume = version(bin).is_some_and(|v| v >= RESUME_FROM);
    while let Ok(mut ask) = asks.recv() {
        while let Ok(next) = asks.try_recv() {
            if ask == Ask::Release {
                break;
            }
            ask = next;
        }
        let release = ask == Ask::Release;
        let _ = command(bin, pane, &ask, takes_resume.then_some(resume))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if release {
            return;
        }
    }
}

fn command(bin: &Path, pane: &str, ask: &Ask, resume: Option<&[String]>) -> Command {
    let mut command = Command::new(bin);
    command.arg("pane");
    match ask {
        Ask::Report { blocked, message } => {
            command.args(["report-agent", "--source", SOURCE, "--agent", AGENT]);
            command.args(["--state", if *blocked { "blocked" } else { "idle" }]);
            if !message.is_empty() {
                command.args(["--message", message]);
            }
            command.args(["--seq", &seq().to_string(), pane]);
            if let Some(resume) = resume {
                command.arg("--").args(resume);
            }
        }
        Ask::Release => {
            command.args(["release-agent", "--source", SOURCE, "--agent", AGENT]);
            command.args(["--seq", &seq().to_string(), pane]);
        }
    }
    command
}

/// A sequence number that only goes up: the time in milliseconds.
fn seq() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// herdr's version, from `herdr --version` ("herdr 0.9.3").
fn version(bin: &Path) -> Option<(u64, u64, u64)> {
    let out = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let word = text
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_blocked_with_a_message_and_none_is_idle() {
        assert_eq!(
            report(3, 1),
            Ask::Report {
                blocked: true,
                message: "3 waiting, 1 blocking".into()
            }
        );
        assert_eq!(
            report(1, 0),
            Ask::Report {
                blocked: true,
                message: "1 waiting".into()
            }
        );
        assert_eq!(
            report(0, 0),
            Ask::Report {
                blocked: false,
                message: String::new()
            }
        );
    }

    #[test]
    fn the_resume_command_goes_after_the_pane() {
        let resume = ["cases".to_string(), "tui".to_string()];
        let ask = report(2, 0);
        let args = |c: &Command| -> Vec<String> {
            c.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let with = args(&command(Path::new("herdr"), "p1", &ask, Some(&resume)));
        assert_eq!(&with[..2], ["pane", "report-agent"]);
        assert!(with.windows(2).any(|w| w == ["--state", "blocked"]));
        assert!(with.windows(2).any(|w| w == ["--message", "2 waiting"]));
        assert_eq!(&with[with.len() - 4..], ["p1", "--", "cases", "tui"]);
        let without = args(&command(Path::new("herdr"), "p1", &ask, None));
        assert_eq!(without.last().map(String::as_str), Some("p1"));
        let release = args(&command(Path::new("herdr"), "p1", &Ask::Release, None));
        assert_eq!(&release[..2], ["pane", "release-agent"]);
    }

    #[test]
    fn versions_parse_from_herdrs_line() {
        assert_eq!(parse_version("herdr 0.9.3\n"), Some((0, 9, 3)));
        assert_eq!(parse_version("herdr 0.10.0-rc.1"), Some((0, 10, 0)));
        assert_eq!(parse_version("herdr 1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("herdr"), None);
    }

    #[test]
    fn a_version_is_the_first_word_that_starts_with_a_digit() {
        assert_eq!(parse_version("herdr 0.10.2+build.5"), Some((0, 10, 2)));
        assert_eq!(parse_version("v1.2.3 4.5.6"), Some((4, 5, 6)));
        assert_eq!(parse_version("herdr (dev) 2.0.1"), Some((2, 0, 1)));
        assert_eq!(parse_version("herdr 7"), None);
        assert_eq!(parse_version("herdr 1.x.3"), None);
        assert_eq!(parse_version("herdr 1.2.x"), Some((1, 2, 0)));
        assert_eq!(parse_version("herdr 12.34.56"), Some((12, 34, 56)));
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn the_resume_threshold_is_0_10_0() {
        assert!((0, 9, 99) < RESUME_FROM);
        assert!((0, 10, 0) >= RESUME_FROM);
        assert!((1, 0, 0) >= RESUME_FROM);
    }

    #[test]
    fn seq_is_the_time_in_milliseconds() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let seq = seq();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        assert!(before <= seq && seq <= after);
    }

    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    /// Writing a script and running it while another thread forks can fail
    /// with "text file busy", so the tests that do serialise.
    static FAKES: Mutex<()> = Mutex::new(());

    /// A `herdr` that answers `--version` with `version` (and exits with
    /// `status`) and otherwise appends its arguments to a log, one line per
    /// run, each argument followed by `|`.
    struct Fake {
        dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Fake {
        fn new(version: &str, status: i32) -> Fake {
            let guard = FAKES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '{version}'; exit {status}; fi\n\
                 l=; for a in \"$@\"; do l=\"$l$a|\"; done; echo \"$l\" >> '{log}'\n",
                log = dir.path().join("log").display()
            );
            let bin = dir.path().join("herdr");
            std::fs::write(&bin, script).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            Fake { dir, _guard: guard }
        }

        fn bin(&self) -> PathBuf {
            self.dir.path().join("herdr")
        }

        /// The runs so far, as their arguments.
        fn runs(&self) -> Vec<Vec<String>> {
            let log = std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default();
            log.lines()
                .map(|line| {
                    line.strip_suffix('|')
                        .unwrap()
                        .split('|')
                        .map(str::to_owned)
                        .collect()
                })
                .collect()
        }
    }

    fn resume() -> Vec<String> {
        ["cases", "--store", "/s", "tui"].map(String::from).to_vec()
    }

    /// Runs the reporter over the asks, queued before it starts, to the end.
    fn run_reporter(fake: &Fake, asks: Vec<Ask>) {
        let (sender, receiver) = mpsc::channel();
        for ask in asks {
            sender.send(ask).unwrap();
        }
        drop(sender);
        reporter(&fake.bin(), "p1", &resume(), &receiver);
    }

    fn seq_of(run: &[String]) -> u128 {
        let at = run.iter().position(|a| a == "--seq").unwrap();
        run[at + 1].parse().unwrap()
    }

    #[test]
    fn version_reads_what_the_binary_prints() {
        {
            let fake = Fake::new("herdr 0.10.1", 0);
            assert_eq!(version(&fake.bin()), Some((0, 10, 1)));
        }
        assert_eq!(version(Path::new("/nonexistent/herdr")), None);
        let odd = Fake::new("no version here", 0);
        assert_eq!(version(&odd.bin()), None);
    }

    #[test]
    fn a_report_carries_the_state_message_seq_and_pane() {
        let fake = Fake::new("herdr 0.9.0", 0);
        run_reporter(
            &fake,
            vec![Ask::Report {
                blocked: true,
                message: "3 waiting, 1 blocking".into(),
            }],
        );
        let runs = fake.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        let run = &runs[0];
        let head: Vec<&str> = run[..9].iter().map(String::as_str).collect();
        assert_eq!(
            head,
            [
                "pane",
                "report-agent",
                "--source",
                "cases-tui",
                "--agent",
                "cases",
                "--state",
                "blocked",
                "--message"
            ]
        );
        assert_eq!(run[9], "3 waiting, 1 blocking");
        assert_eq!(run[10], "--seq");
        assert!(seq_of(run) > 0);
        assert_eq!(&run[12..], ["p1"], "no resume before 0.10.0");
    }

    #[test]
    fn idle_has_no_message() {
        let fake = Fake::new("herdr 0.9.0", 0);
        run_reporter(
            &fake,
            vec![Ask::Report {
                blocked: false,
                message: String::new(),
            }],
        );
        let runs = fake.runs();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].windows(2).any(|w| w == ["--state", "idle"]));
        assert!(!runs[0].iter().any(|a| a == "--message"));
    }

    #[test]
    fn a_new_enough_herdr_gets_the_resume_command() {
        for (version, resumes) in [
            ("herdr 0.9.9", false),
            ("herdr 0.10.0", true),
            ("herdr 0.10.0-rc.1", true),
            ("herdr 1.0.0", true),
            ("garbage", false),
        ] {
            let fake = Fake::new(version, 0);
            run_reporter(&fake, vec![report(1, 0)]);
            let runs = fake.runs();
            assert_eq!(runs.len(), 1, "{version}: {runs:?}");
            if resumes {
                let tail = &runs[0][runs[0].len() - 6..];
                assert_eq!(
                    tail,
                    ["p1", "--", "cases", "--store", "/s", "tui"],
                    "{version}"
                );
            } else {
                assert_eq!(runs[0].last().map(String::as_str), Some("p1"), "{version}");
                assert!(!runs[0].iter().any(|a| a == "--"), "{version}");
            }
        }
    }

    #[test]
    fn the_exit_status_of_the_version_check_is_ignored() {
        let fake = Fake::new("herdr 0.10.0", 1);
        run_reporter(&fake, vec![report(1, 0)]);
        let runs = fake.runs();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].iter().any(|a| a == "--"));
    }

    #[test]
    fn queued_reports_collapse_to_the_latest() {
        let fake = Fake::new("herdr 0.9.0", 0);
        run_reporter(&fake, vec![report(1, 0), report(2, 0), report(3, 1)]);
        let runs = fake.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert!(runs[0].iter().any(|a| a == "3 waiting, 1 blocking"));
    }

    #[test]
    fn a_release_stops_the_reporter_and_drops_what_queued_behind_it() {
        let fake = Fake::new("herdr 0.9.0", 0);
        run_reporter(&fake, vec![Ask::Release, report(2, 0)]);
        let runs = fake.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(&runs[0][..2], ["pane", "release-agent"]);
        assert!(runs[0].iter().any(|a| a == "p1"));
        assert!(!runs[0].iter().any(|a| a == "--state"));
    }

    #[test]
    fn a_release_queued_after_a_report_wins() {
        let fake = Fake::new("herdr 0.9.0", 0);
        run_reporter(&fake, vec![report(2, 0), Ask::Release]);
        let runs = fake.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(&runs[0][..2], ["pane", "release-agent"]);
    }

    #[test]
    fn a_release_is_sent_without_a_resume_command_or_message() {
        let fake = Fake::new("herdr 0.10.0", 0);
        run_reporter(&fake, vec![Ask::Release]);
        let runs = fake.runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0][..6],
            [
                "pane",
                "release-agent",
                "--source",
                "cases-tui",
                "--agent",
                "cases"
            ]
        );
        assert_eq!(runs[0][6], "--seq");
        assert_eq!(&runs[0][8..], ["p1"]);
    }

    #[test]
    fn the_reporter_ends_when_the_asks_do_and_survives_a_missing_binary() {
        let (sender, receiver) = mpsc::channel();
        sender.send(report(1, 0)).unwrap();
        sender.send(Ask::Release).unwrap();
        drop(sender);
        reporter(Path::new("/nonexistent/herdr"), "p1", &resume(), &receiver);
        let (sender, receiver) = mpsc::channel::<Ask>();
        drop(sender);
        reporter(Path::new("/nonexistent/herdr"), "p1", &resume(), &receiver);
    }

    /// A `Herdr` over the fake, as `from_env` makes one.
    fn herdr_over(fake: &Fake) -> Herdr {
        let (asks, received) = mpsc::channel();
        let bin = fake.bin();
        let thread = thread::spawn(move || reporter(&bin, "p1", &resume(), &received));
        Herdr {
            asks,
            thread,
            last: None,
        }
    }

    /// Waits for the fake to have been run `n` times.
    fn wait_for_runs(fake: &Fake, n: usize) -> Vec<Vec<String>> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let runs = fake.runs();
            if runs.len() >= n || std::time::Instant::now() > deadline {
                return runs;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn only_a_change_of_count_is_reported_again_and_release_comes_last() {
        let fake = Fake::new("herdr 0.9.0", 0);
        let mut herdr = herdr_over(&fake);
        herdr.report((1, 0));
        wait_for_runs(&fake, 1);
        herdr.report((1, 0));
        herdr.report((1, 1));
        let runs = wait_for_runs(&fake, 2);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs[0].iter().any(|a| a == "1 waiting"));
        assert!(runs[1].iter().any(|a| a == "1 waiting, 1 blocking"));
        assert!(seq_of(&runs[1]) >= seq_of(&runs[0]));
        herdr.report((0, 0));
        wait_for_runs(&fake, 3);
        herdr.report((1, 0));
        let runs = wait_for_runs(&fake, 4);
        assert_eq!(runs.len(), 4, "{runs:?}");
        assert!(runs[2].windows(2).any(|w| w == ["--state", "idle"]));
        assert!(runs[3].windows(2).any(|w| w == ["--state", "blocked"]));
        herdr.release();
        let runs = fake.runs();
        assert_eq!(runs.len(), 5, "{runs:?}");
        assert_eq!(&runs[4][..2], ["pane", "release-agent"]);
    }

    #[test]
    fn release_waits_for_the_run_to_finish_but_not_for_long() {
        // The run is in flight when release is called: release returns only
        // once the thread has finished, so the log has the line by then.
        let fake = Fake::new("herdr 0.9.0", 0);
        let herdr = herdr_over(&fake);
        herdr.release();
        let runs = fake.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(&runs[0][..2], ["pane", "release-agent"]);
    }

    /// Runs `from_env_child` in a process of its own with exactly the
    /// variables given, since a test cannot set them in this one.
    fn from_env_in_child(scenario: &str, vars: &[(&str, &str)]) {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.env_clear();
        for (name, value) in std::env::vars() {
            if name.starts_with("LLVM_") {
                child.env(name, value);
            }
        }
        child
            .args([
                "--exact",
                "commands::tui::herdr::tests::from_env_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("CASES_HERDR_SCENARIO", scenario)
            .envs(vars.iter().copied());
        let out = child.output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && text.contains(&format!("scenario {scenario} ok")),
            "{scenario}: {text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The other half of `from_env_in_child`: a no-op unless it is the child.
    #[test]
    fn from_env_child() {
        let Ok(scenario) = std::env::var("CASES_HERDR_SCENARIO") else {
            return;
        };
        let found = Herdr::from_env("/s");
        assert!(scenario == "ready" || found.is_none(), "{scenario}");
        if scenario == "ready" {
            let bin = std::env::var("CASES_HERDR_FAKE").unwrap();
            let log = std::env::var("CASES_HERDR_LOG").unwrap();
            let mut herdr = found.expect("in herdr");
            assert_eq!(herdr.last, None);
            herdr.report((2, 1));
            // Release only after the report has run: one queued behind a
            // release is dropped.
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while !std::path::Path::new(&log).exists() {
                assert!(std::time::Instant::now() < deadline, "no report ran");
                thread::sleep(Duration::from_millis(5));
            }
            herdr.release();
            let log = std::fs::read_to_string(log).unwrap();
            let lines: Vec<&str> = log.lines().collect();
            assert_eq!(lines.len(), 2, "{bin}: {log}");
            assert!(lines[0].starts_with("pane|report-agent|"), "{log}");
            assert!(lines[0].contains("|2 waiting, 1 blocking|"), "{log}");
            assert!(lines[0].ends_with("|%7|--|cases|--store|/s|tui|"), "{log}");
            assert!(lines[1].starts_with("pane|release-agent|"), "{log}");
        } else {
            assert!(found.is_none(), "{scenario}");
        }
        println!("scenario {scenario} ok");
    }

    #[test]
    fn from_env_needs_herdr_env_1_a_binary_and_a_pane() {
        let none: &[(&str, &str)] = &[];
        from_env_in_child("unset", none);
        from_env_in_child(
            "not_1",
            &[
                ("HERDR_ENV", "0"),
                ("HERDR_BIN_PATH", "/b"),
                ("HERDR_PANE_ID", "%7"),
            ],
        );
        from_env_in_child(
            "empty_env",
            &[
                ("HERDR_ENV", ""),
                ("HERDR_BIN_PATH", "/b"),
                ("HERDR_PANE_ID", "%7"),
            ],
        );
        from_env_in_child("no_bin", &[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "%7")]);
        from_env_in_child(
            "empty_bin",
            &[
                ("HERDR_ENV", "1"),
                ("HERDR_BIN_PATH", ""),
                ("HERDR_PANE_ID", "%7"),
            ],
        );
        from_env_in_child("no_pane", &[("HERDR_ENV", "1"), ("HERDR_BIN_PATH", "/b")]);
        from_env_in_child(
            "empty_pane",
            &[
                ("HERDR_ENV", "1"),
                ("HERDR_BIN_PATH", "/b"),
                ("HERDR_PANE_ID", ""),
            ],
        );
    }

    #[test]
    fn from_env_in_herdr_reports_with_a_resume_command_for_the_store() {
        let fake = Fake::new("herdr 0.10.0", 0);
        let bin = fake.bin();
        let log = fake.dir.path().join("log");
        from_env_in_child(
            "ready",
            &[
                ("HERDR_ENV", "1"),
                ("HERDR_BIN_PATH", bin.to_str().unwrap()),
                ("HERDR_PANE_ID", "%7"),
                ("CASES_HERDR_FAKE", bin.to_str().unwrap()),
                ("CASES_HERDR_LOG", log.to_str().unwrap()),
            ],
        );
    }
}
