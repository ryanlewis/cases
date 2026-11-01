//! A forwarded store whose far side is this build's `cases dial-stdio`,
//! run as the child in place of `ssh host cases dial-stdio`, or by `sh -c`
//! as an `exec:` forward runs its command, or this build's `cases serve` on
//! loopback, reached over HTTP as an `http://` forward reaches it, against
//! a store file of its own, with a clock the test sets: the debug build's
//! `CASES_TEST_CLOCK` names the file the far side reads its time from.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]
// Each test binary uses part of this module.
#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use cases_core::Time;
use cases_store_remote::{Client, DIAL_STORE, Dial, RemoteStore, Server};
use cases_store_sqlite::Db;
use tempfile::TempDir;

/// How the far side is reached.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Over {
    /// `cases dial-stdio` run as the child itself, in place of ssh.
    Ssh,
    /// `sh -c <command>`, as `exec:<command>` runs it.
    Exec,
    /// HTTP to `cases serve` on loopback, as `http://host:port` reaches
    /// it.
    Http,
}

/// The store at the far side, the forwarded store that reaches it, and
/// the file that holds the far side's clock.
pub(crate) struct Far {
    dir: TempDir,
    clock: PathBuf,
    pub(crate) store: RemoteStore,
    /// The far side's serve, over HTTP, stopped when dropped.
    serve: Option<Child>,
}

impl Drop for Far {
    fn drop(&mut self) {
        if let Some(mut serve) = self.serve.take() {
            let _ = serve.kill();
            let _ = serve.wait();
        }
    }
}

/// Starts `cases serve` on a port of its own on 127.0.0.1, against the
/// store `store`, with its clock in `clock` and a home of its own in
/// `dir`, with the variables `vars` too: the serve, and the address it
/// prints, such as `http://127.0.0.1:53211`.
pub(crate) fn serve(
    dir: &Path,
    store: &Path,
    clock: &Path,
    vars: &[(&str, &str)],
) -> (Child, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cases"));
    command
        .args(["serve", "--listen", "127.0.0.1:0", "--no-open"])
        .env_clear()
        .envs(
            std::env::vars_os()
                .filter(|(name, _)| name.to_str().is_some_and(|name| name.starts_with("LLVM_"))),
        )
        .env("CASES_STORE", store)
        .env("CASES_CONFIG", dir.join("config.toml"))
        .env("CASES_TEST_CLOCK", clock)
        .env("CASES_NO_KEYCHAIN", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (var, sub) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
    ] {
        command.env(var, dir.join(sub));
    }
    command.envs(vars.iter().copied());
    let mut child = command.spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    // The rest is read, so that serve never waits on a full pipe.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    });
    let url = line
        .trim()
        .rsplit(' ')
        .next()
        .filter(|url| url.starts_with("http://"))
        .unwrap_or_else(|| panic!("serve printed no address: {line:?}"))
        .trim_end_matches('/')
        .to_owned();
    (child, url)
}

impl Far {
    /// A new, empty store, with its clock at `now`.
    pub(crate) fn new(now: Time) -> Far {
        Far::over(now, Over::Ssh)
    }

    /// A new, empty store, with its clock at `now`, reached `over` the
    /// stand-in for ssh or a command.
    pub(crate) fn over(now: Time, over: Over) -> Far {
        let dir = tempfile::tempdir().unwrap();
        let clock = dir.path().join("clock");
        fs::write(&clock, now.to_string()).unwrap();
        let bin = env!("CARGO_BIN_EXE_cases");
        if let Over::Http = over {
            // serve's sweep makes a first pass as it starts, and would close a
            // briefing a vector writes with an old date: it is off here.
            fs::write(
                dir.path().join("config.toml"),
                "briefing-close-after = \"off\"\n",
            )
            .unwrap();
            let (child, url) = serve(dir.path(), &dir.path().join("cases.db"), &clock, &[]);
            let server = Server::forward(&url).unwrap();
            let store = Client::forward(server, None).store(DIAL_STORE).unwrap();
            return Far {
                dir,
                clock,
                store,
                serve: Some(child),
            };
        }
        let dial = match over {
            Over::Ssh => Dial::new(bin, "ssh://far").args(["dial-stdio"]),
            Over::Http => unreachable!("served above"),
            Over::Exec => {
                let command = format!("exec '{bin}' dial-stdio");
                Dial::new("sh", format!("exec:{command}")).args(["-c".to_owned(), command])
            }
        };
        let dial = dial
            .env("CASES_STORE", dir.path().join("cases.db"))
            .env("CASES_CONFIG", dir.path().join("config.toml"))
            .env("CASES_TEST_CLOCK", &clock);
        let store = Client::dial(dial).store(DIAL_STORE).unwrap();
        Far {
            dir,
            clock,
            store,
            serve: None,
        }
    }

    /// Sets the far side's clock.
    pub(crate) fn set_clock(&self, now: Time) {
        fs::write(&self.clock, now.to_string()).unwrap();
    }

    /// The far side's store file itself, to write with none of the
    /// store's checks.
    pub(crate) fn db(&self) -> Db {
        Db::new(self.dir.path().join("cases.db"))
    }
}
