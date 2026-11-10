//! `cases service`: `cases serve` as a login service (SV-03). The service
//! manager is the system's: launchd on macOS, systemd on Linux. No test
//! reaches the real one: `$PATH` holds only a stand-in `launchctl` and
//! `systemctl`, which record their commands and change nothing, and every
//! file lands in the test's own home.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use common::{Env, json, write_file};

/// Whether this system's service manager is launchd.
const LAUNCHD: bool = cfg!(target_os = "macos");

/// The stand-in service manager. It records each command in `calls`. With
/// `down` there, every command fails as when the service manager cannot be
/// reached, saying so on stderr as the real ones do. With `active` there,
/// every command succeeds; without it,
/// `launchctl print` and `systemctl is-active` answer as for a service that
/// is not there.
const FAKE: &str = r#"#!/bin/sh
dir=${0%/*}
echo "${0##*/} $*" >> "$dir/calls"
if [ -e "$dir/down" ]; then echo "Failed to connect to the service manager" >&2; exit 1; fi
if [ -e "$dir/active" ]; then exit 0; fi
case "${0##*/} $1 $2" in
  "launchctl print "*) echo "Could not find service"; exit 113;;
  "systemctl --user is-active") echo inactive; exit 3;;
esac
exit 0
"#;

/// A test's home with the stand-in service manager on its `$PATH`.
struct Service {
    env: Env,
    bin: PathBuf,
}

impl Service {
    fn new() -> Service {
        let mut env = Env::new();
        let bin = env.path("bin");
        for name in ["launchctl", "systemctl"] {
            let path = bin.join(name);
            write_file(&path, FAKE);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        env.set("PATH", bin.to_str().unwrap());
        Service { env, bin }
    }

    /// Makes the stand-in answer that the service is loaded, or not.
    fn active(&self, active: bool) {
        self.flag("active", active);
    }

    /// Makes every command of the stand-in fail, or not.
    fn down(&self, down: bool) {
        self.flag("down", down);
    }

    fn flag(&self, name: &str, on: bool) {
        let path = self.bin.join(name);
        if on {
            write_file(&path, "");
        } else {
            let _ = fs::remove_file(path);
        }
    }

    /// The commands run since the last look.
    fn calls(&self) -> Vec<String> {
        let path = self.bin.join("calls");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let _ = fs::remove_file(path);
        text.lines().map(str::to_owned).collect()
    }

    /// The plist or unit file.
    fn file(&self) -> PathBuf {
        if LAUNCHD {
            self.env
                .path("home")
                .join("Library/LaunchAgents/sh.cases.serve.plist")
        } else {
            self.env
                .path("config")
                .join("systemd/user/cases-serve.service")
        }
    }

    /// `cases --store <store> service` and then `args`.
    fn run(&self, args: &[&str]) -> common::Output {
        let mut all = vec!["service"];
        all.extend(args);
        self.env.cases(&all)
    }

    /// Installs the service on a free address, which it gives.
    fn install(&self) -> String {
        let (addr, out) = self.install_free(&[]);
        assert_eq!(out.code, 0, "{out:?}");
        addr
    }

    /// `cases service install --listen <a free address>` and then `args`:
    /// the address and what install gave. Another test can take a port
    /// picked free before install checks it, which install then refuses,
    /// so a refusal of the address alone is tried again on another, with
    /// the calls of the refused tries forgotten.
    fn install_free(&self, args: &[&str]) -> (String, common::Output) {
        for _ in 0..TRIES {
            let addr = free_addr();
            let mut all = vec!["install", "--listen", &addr];
            all.extend(args);
            let out = self.run(&all);
            if !taken(&out, &addr) {
                return (addr, out);
            }
            self.calls();
        }
        panic!("no address stayed free in {TRIES} tries");
    }

    /// Installs the service on a free address, as [`Service::install`]
    /// does, and fakes the service's serve on it: the address and the
    /// serve, as [`Env::fake_serve`] gives it. Another test can take the
    /// port between install's check and the serve's, so install is done
    /// again on another address until the serve has it.
    fn serving(&self) -> (String, (TcpListener, String)) {
        for _ in 0..TRIES {
            let addr = self.install();
            match common::try_listen(&addr) {
                Ok(listener) => {
                    let serve = self.env.fake_serve_with(&self.env.store, listener);
                    return (addr, serve);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    self.calls();
                }
                Err(error) => panic!("{addr}: {error}"),
            }
        }
        panic!("no address stayed free in {TRIES} tries");
    }

    /// `cases service`, which must only have asked, and whether it exited 0.
    fn status(&self, args: &[&str]) -> (String, bool) {
        self.calls();
        let out = self.run(args);
        assert!(out.code == 0 || out.code == 1, "{out:?}");
        for call in self.calls() {
            assert!(
                call.starts_with("launchctl print ")
                    || call.starts_with("systemctl --user is-active "),
                "cases service ran {call}, which is not a query"
            );
        }
        (out.stdout, out.code == 0)
    }

    /// The query `cases service` shows for the service manager's view.
    fn check(&self) -> String {
        if LAUNCHD {
            format!("launchctl print gui/{}/sh.cases.serve", uid(&self.env))
        } else {
            "systemctl --user status cases-serve.service".to_owned()
        }
    }
}

/// This user's id, from a file the test made.
fn uid(env: &Env) -> u32 {
    fs::metadata(env.path("bin")).unwrap().uid()
}

/// How many free addresses a test tries before it gives up.
const TRIES: usize = 20;

/// Whether install refused `addr` because something holds it.
fn taken(out: &common::Output, addr: &str) -> bool {
    out.code == 1 && out.stderr.contains(&format!("{addr} is already in use"))
}

/// A loopback address nothing listened on just now, since install refuses
/// one that is taken. Another test can take it before it is used: see
/// [`Service::install_free`].
fn free_addr() -> String {
    let listener = common::listen("127.0.0.1:0");
    listener.local_addr().unwrap().to_string()
}

/// The binary the tests run, as the service runs it.
fn exe() -> String {
    env!("CARGO_BIN_EXE_cases").to_owned()
}

/// The store, as an absolute path.
fn store(env: &Env) -> String {
    std::path::absolute(&env.store)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn assert_lines(out: &str, want: &[&str]) {
    for line in want {
        assert!(out.contains(line), "lacks {line:?}:\n{out}");
    }
}

#[test]
fn install_writes_the_service_and_starts_it() {
    let service = Service::new();
    let env = &service.env;
    let (addr, out) = service.install_free(&["--as", "Ryan Lewis"]);
    assert_eq!(out.code, 0, "{out:?}");
    let file = service.file();
    let body = fs::read_to_string(&file).unwrap();
    let store = store(env);
    let config = env.path("config").join("cases/config.toml");
    let state = env.path("state");
    let log = state.join("cases/serve.log");
    let mut args = vec![
        exe(),
        "--store".into(),
        store.clone(),
        "--config".into(),
        config.display().to_string(),
        "serve".into(),
        "--no-open".into(),
        "--listen".into(),
        addr.clone(),
        "--as".into(),
        "Ryan Lewis".into(),
    ];
    if LAUNCHD {
        // serve writes and rotates its own log; launchd's stdout goes
        // nowhere.
        args.extend(["--log".into(), log.display().to_string()]);
        let strings: Vec<String> = args
            .iter()
            .map(|arg| format!("\t\t<string>{arg}</string>\n"))
            .collect();
        let strings = strings.concat();
        assert_lines(
            &body,
            &[
                &format!("<key>ProgramArguments</key>\n\t<array>\n{strings}\t</array>"),
                &format!(
                    "<key>HOME</key>\n\t\t<string>{}</string>",
                    env.path("home").display()
                ),
                &format!(
                    "<key>XDG_STATE_HOME</key>\n\t\t<string>{}</string>",
                    state.display()
                ),
                "<key>StandardOutPath</key>\n\t<string>/dev/null</string>",
                &format!(
                    "<key>StandardErrorPath</key>\n\t<string>{}</string>",
                    log.display()
                ),
            ],
        );
        assert!(log.parent().unwrap().is_dir());
        let uid = uid(env);
        assert_eq!(
            service.calls(),
            [
                format!("launchctl print gui/{uid}/sh.cases.serve"),
                format!("launchctl bootstrap gui/{uid} {}", file.display()),
            ]
        );
    } else {
        let exec: Vec<String> = args.iter().map(|arg| format!("\"{arg}\"")).collect();
        assert_lines(
            &body,
            &[
                &format!("ExecStart={}\n", exec.join(" ")),
                &format!("Environment=\"HOME={}\"", env.path("home").display()),
                &format!("Environment=\"XDG_STATE_HOME={}\"", state.display()),
            ],
        );
        assert_eq!(
            service.calls(),
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable cases-serve.service",
                "systemctl --user restart cases-serve.service",
            ]
        );
    }
    let (runs, log_line) = if LAUNCHD {
        (
            format!(
                "--listen {addr} --as 'Ryan Lewis' --log {}\n",
                log.display()
            ),
            format!("Log: {}\n", log.display()),
        )
    } else {
        (
            format!("--listen {addr} --as 'Ryan Lewis'\n"),
            "Log: journalctl --user -u cases-serve.service\n".to_owned(),
        )
    };
    assert_lines(
        &out.stdout,
        &[
            &format!("Installed the cases serve service: {}\n", file.display()),
            &runs,
            "Check it with: cases service\n",
            &format!("Service manager: {}\n", service.check()),
            &log_line,
        ],
    );
}

#[test]
fn install_takes_the_address_and_name_from_the_config_file() {
    let service = Service::new();
    // The address is in the file, so a try whose address was taken writes
    // the file again with another.
    let (addr, out) = (0..TRIES)
        .map(|_| {
            let addr = free_addr();
            service
                .env
                .config(&format!("listen = \"{addr}\"\nname = \"From File\"\n"));
            let out = service.run(&["install"]);
            if taken(&out, &addr) {
                service.calls();
            }
            (addr, out)
        })
        .find(|(addr, out)| !taken(out, addr))
        .expect("an address that stayed free");
    assert_eq!(out.code, 0, "{out:?}");
    let body = fs::read_to_string(service.file()).unwrap();
    assert!(body.contains(&addr), "{body}");
    assert!(body.contains("From File"), "{body}");

    // A flag beats the file.
    let (flag, out) = service.install_free(&["--as", "Flag"]);
    assert_eq!(out.code, 0, "{out:?}");
    let body = fs::read_to_string(service.file()).unwrap();
    assert!(body.contains(&flag) && body.contains("Flag") && !body.contains("From File"));

    // Without either, no name is recorded.
    fs::remove_file(service.env.path("config").join("cases/config.toml")).unwrap();
    assert_eq!(service.install_free(&[]).1.code, 0);
    assert!(!fs::read_to_string(service.file()).unwrap().contains("--as"));
}

#[test]
fn install_refuses_an_address_that_is_not_loopback() {
    let service = Service::new();
    for addr in ["0.0.0.0:7337", "192.168.1.2:7337"] {
        let out = service.run(&["install", "--listen", addr]);
        assert_eq!(out.code, 1, "{out:?}");
        assert!(out.stderr.contains("loopback"), "{out:?}");
    }
    service.env.config("listen = \"0.0.0.0:1\"\n");
    assert!(service.run(&["install"]).stderr.contains("loopback"));
    assert!(!service.file().exists());
    assert!(service.calls().is_empty());
}

#[test]
fn uninstall_stops_the_service_and_removes_its_file() {
    let service = Service::new();
    service.install();
    service.calls();
    if LAUNCHD {
        service.active(true);
    }
    let out = service.run(&["uninstall"]);
    assert_eq!(out.code, 0, "{out:?}");
    let file = service.file();
    assert_eq!(
        out.stdout,
        format!(
            "Stopped the cases serve service and removed {}\n",
            file.display()
        )
    );
    assert!(!file.exists());
    let calls = service.calls();
    if LAUNCHD {
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[1].starts_with("launchctl bootout gui/"), "{calls:?}");
    } else {
        assert_eq!(
            calls,
            [
                "systemctl --user disable --now cases-serve.service",
                "systemctl --user daemon-reload",
            ]
        );
    }

    service.active(false);
    let out = service.run(&["uninstall"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stderr,
        "Error: the cases serve service is not installed\n"
    );
}

/// A service manager that cannot be asked about the service stops
/// uninstall before the file is removed, and `cases service` before it
/// reports the service not loaded.
#[test]
fn a_service_manager_that_cannot_be_asked_changes_nothing() {
    let service = Service::new();
    service.active(true);
    service.install();
    let body = fs::read_to_string(service.file()).unwrap();
    service.down(true);
    service.calls();

    let out = service.run(&["uninstall"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("Failed to connect"), "{out:?}");
    assert!(service.file().exists());
    let calls = service.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");

    let out = service.run(&[]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("Failed to connect"), "{out:?}");
    assert!(!out.stdout.contains("Loaded:    no"), "{out:?}");

    let out = service.run(&["install"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("Failed to connect"), "{out:?}");
    if LAUNCHD {
        // launchd is asked before the file is replaced.
        assert_eq!(fs::read_to_string(service.file()).unwrap(), body);
    }
}

/// With no `launchctl` or `systemctl` on `$PATH`, the real one in `/bin` is
/// not looked for: the command fails and touches nothing.
#[test]
fn without_the_service_manager_on_path_nothing_is_touched() {
    let mut env = Env::new();
    for path in [None, Some(env.path("empty"))] {
        match &path {
            None => env.unset("PATH"),
            Some(path) => env.set("PATH", path.to_str().unwrap()),
        }
        for args in [
            &["service", "install"][..],
            &["service", "uninstall"],
            &["service"],
        ] {
            let out = env.cases(args);
            assert_eq!(out.code, 1, "{args:?}: {out:?}");
            assert!(out.stderr.contains("cannot find "), "{args:?}: {out:?}");
        }
    }
    assert!(!env.path("home").join("Library").exists());
    assert!(!env.path("config").join("systemd").exists());
}

#[test]
fn install_refuses_while_a_serve_runs_for_the_store() {
    let service = Service::new();
    let (_listener, url) = service.env.fake_serve("127.0.0.1:0");
    let out = service.run(&["install", "--listen", &free_addr()]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains(&url), "{out:?}");
    assert!(out.stderr.contains("stop it"), "{out:?}");
    assert!(!service.file().exists());
    // Only the query for a service already loaded ran.
    assert!(
        service
            .calls()
            .iter()
            .all(|call| call.contains(" print ") || call.contains(" is-active ")),
    );
}

#[test]
fn install_refuses_an_address_that_is_taken() {
    let service = Service::new();
    let held = common::listen("127.0.0.1:0");
    let addr = held.local_addr().unwrap().to_string();
    let out = service.run(&["install", "--listen", &addr]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!("{addr} is already in use")),
        "{out:?}"
    );
    assert!(!service.file().exists());
}

/// On a reinstall the running serve is the service's, which install
/// restarts.
#[test]
fn reinstall_while_the_service_runs() {
    let service = Service::new();
    let (addr, (_listener, _)) = service.serving();
    service.active(true);
    let out = service.run(&["install", "--listen", &addr]);
    assert_eq!(out.code, 0, "{out:?}");
}

/// A reinstall still checks what the loaded service does not hold: a new
/// address someone else holds, a serve on a new store, and everything when
/// the service is not loaded.
#[test]
fn reinstall_checks_what_changed() {
    let mut service = Service::new();
    let (addr, (_listener, _)) = service.serving();
    service.active(true);

    let held = common::listen("127.0.0.1:0");
    let out = service.run(&[
        "install",
        "--listen",
        &held.local_addr().unwrap().to_string(),
    ]);
    assert!(out.stderr.contains("is already in use"), "{out:?}");

    // Another store with a serve of its own.
    let old = service.env.store.clone();
    service.env.store = service.env.path("other.db");
    let (_other, url) = service.env.fake_serve("127.0.0.1:0");
    let out = service.run(&["install", "--listen", &addr]);
    assert!(out.stderr.contains(&url), "{out:?}");
    service.env.store = old;

    service.active(false);
    let out = service.run(&["install", "--listen", &addr]);
    assert!(out.stderr.contains("stop it"), "{out:?}");
}

#[test]
fn service_status_with_nothing_installed() {
    let service = Service::new();
    let (out, ok) = service.status(&[]);
    assert!(!ok);
    assert_lines(
        &out,
        &[
            &format!("Installed: no ({})\n", service.file().display()),
            &format!("Loaded:    no ({})\n", service.check()),
            "Inbox:     not running\n",
        ],
    );
    assert!(!out.contains("Problem"), "{out}");
}

#[test]
fn service_status_names_a_serve_started_by_hand() {
    let service = Service::new();
    let (_listener, url) = service.env.fake_serve("127.0.0.1:0");
    let (out, ok) = service.status(&[]);
    assert!(!ok);
    assert_lines(
        &out,
        &[
            &format!("Inbox:     {url} (pid {})\n", std::process::id()),
            &format!(
                "Problem:   not installed, but a cases serve started by hand is running (pid \
                 {}); stop it before cases service install\n",
                std::process::id()
            ),
        ],
    );
}

#[test]
fn service_status_when_installed_loaded_and_answering() {
    let service = Service::new();
    let (addr, (_listener, url)) = service.serving();
    service.active(true);
    let store = store(&service.env);
    for args in [&[][..], &["status"]] {
        let (out, ok) = service.status(args);
        assert!(ok, "{out}");
        assert_lines(
            &out,
            &[
                &format!("Installed: {}\n", service.file().display()),
                &format!("Runs:      {} --store {store}", exe()),
                &format!("Store:     {store}\n"),
                &format!("Listen:    {addr}\n"),
                &format!("Loaded:    yes ({})\n", service.check()),
                &format!("Inbox:     {url}"),
            ],
        );
        assert!(!out.contains("Problem"), "{out}");
    }

    for args in [&["--json"][..], &["-j"], &["status", "--json"]] {
        let (out, ok) = service.status(args);
        assert!(ok);
        let got = json(&out);
        assert_eq!(got["installed"], true);
        assert_eq!(got["loaded"], true);
        assert_eq!(got["running"], true);
        assert_eq!(got["store"], store.as_str());
        assert_eq!(got["listen"], addr.as_str());
        assert_eq!(got["url"], url.as_str());
        assert_eq!(got["pid"], std::process::id());
        assert_eq!(got["args"][0], exe().as_str());
        assert!(got.get("problems").is_none(), "{got}");
    }
}

/// The service's serve reads the config file it was installed with, and
/// status shows that file's allow-hosts and tailscale-logins.
#[test]
fn service_status_shows_the_hosts_its_config_file_allows() {
    let service = Service::new();
    let path = service.env.config(
        "allow-hosts = [\"devbox.example.ts.net:7337\", \"cases.example.com\"]\n\
         tailscale-logins = [\"ryan@github\", \"sam@github\"]\n",
    );
    let addr = service.install();
    let (out, _) = service.status(&[]);
    assert_lines(
        &out,
        &[
            &format!("--config {}", path.display()),
            &format!("Listen:    {addr}\n"),
            "Hosts:     devbox.example.ts.net:7337, cases.example.com\n",
            "Logins:    ryan@github, sam@github\n",
        ],
    );
    let (out, _) = service.status(&["--json"]);
    let got = json(&out);
    assert_eq!(
        got["allow_hosts"],
        serde_json::json!(["devbox.example.ts.net:7337", "cases.example.com"])
    );
    assert_eq!(
        got["tailscale_logins"],
        serde_json::json!(["ryan@github", "sam@github"])
    );
}

/// A service file written before the service went quiet without its
/// binary (`KeepAlive` true, no `ConditionPathExists`) still reads.
#[test]
fn service_status_reads_a_file_in_the_old_format() {
    let service = Service::new();
    let (addr, (_listener, url)) = service.serving();
    service.active(true);
    let body = fs::read_to_string(service.file()).unwrap();
    let old = if LAUNCHD {
        let start = body.find("<key>KeepAlive</key>").unwrap();
        let close = "\n\t</dict>\n";
        let end = body[start..].find(close).unwrap() + start + close.len();
        format!(
            "{}<key>KeepAlive</key>\n\t<true/>\n{}",
            &body[..start],
            &body[end..]
        )
    } else {
        let line = format!("ConditionPathExists={}\n", exe());
        assert!(body.contains(&line), "{body}");
        body.replace(&line, "")
    };
    assert_ne!(old, body);
    fs::write(service.file(), &old).unwrap();
    // plutil comes with macOS: the old file is a plist, as it was.
    if LAUNCHD && Path::new("/usr/bin/plutil").exists() {
        let lint = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(service.file())
            .output()
            .unwrap();
        assert!(lint.status.success(), "{lint:?}\n{old}");
    }
    let (out, ok) = service.status(&[]);
    assert!(ok, "{out}");
    assert_lines(
        &out,
        &[
            &format!("Runs:      {} --store {}", exe(), store(&service.env)),
            &format!("Listen:    {addr}\n"),
            &format!("Inbox:     {url}"),
        ],
    );
    assert!(!out.contains("Problem"), "{out}");
}

/// A plist from before serve kept its own log, with launchd adding to the
/// log for ever, still reads, and a reinstall moves it to serve's `--log`.
#[test]
fn a_reinstall_moves_an_old_plist_to_serve_s_own_log() {
    if !LAUNCHD {
        return;
    }
    let service = Service::new();
    let (addr, (_listener, url)) = service.serving();
    let log = service
        .env
        .path("state/cases/serve.log")
        .display()
        .to_string();
    let new = fs::read_to_string(service.file()).unwrap();
    let flag = format!("\t\t<string>--log</string>\n\t\t<string>{log}</string>\n");
    assert!(new.contains(&flag), "{new}");
    let old = new.replace(&flag, "").replace(
        "<string>/dev/null</string>",
        &format!("<string>{log}</string>"),
    );
    fs::write(service.file(), &old).unwrap();
    service.active(true);
    let (out, ok) = service.status(&[]);
    assert!(ok, "{out}");
    assert_lines(
        &out,
        &[&format!("--listen {addr}\n"), &format!("Inbox:     {url}")],
    );
    assert!(!out.contains("Problem"), "{out}");

    let out = service.run(&["install", "--listen", &addr]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fs::read_to_string(service.file()).unwrap(), new);
}

#[test]
fn service_status_when_installed_but_not_loaded() {
    let service = Service::new();
    service.install();
    let (out, ok) = service.status(&[]);
    assert!(!ok);
    let system = if LAUNCHD { "launchd" } else { "systemd" };
    assert_lines(
        &out,
        &[
            "Loaded:    no",
            &format!(
                "Problem:   installed but not loaded in {system}; cases service install loads \
                 it again\n"
            ),
        ],
    );
    // --json exits as the text does.
    let (out, ok) = service.status(&["--json"]);
    assert!(!ok, "{out}");
    let got = json(&out);
    assert_eq!(got["loaded"], false);
    assert_eq!(got["problems"].as_array().unwrap().len(), 1);
}

#[test]
fn service_status_when_loaded_but_not_answering() {
    let service = Service::new();
    service.install();
    service.active(true);
    let (out, ok) = service.status(&[]);
    assert!(!ok);
    assert_lines(
        &out,
        &[
            "Loaded:    yes",
            "Inbox:     not running",
            &format!(
                "Problem:   loaded but the inbox does not answer; see {}",
                service.check()
            ),
        ],
    );
}

#[test]
fn service_status_when_loaded_with_its_file_gone() {
    let service = Service::new();
    service.active(true);
    let (out, ok) = service.status(&[]);
    assert!(!ok);
    assert_lines(
        &out,
        &[
            "Installed: no",
            &format!(
                "Problem:   loaded in {} but its file is missing; cases service uninstall stops \
                 it\n",
                if LAUNCHD { "launchd" } else { "systemd" }
            ),
        ],
    );
}

#[test]
fn service_status_names_another_store_and_another_binary() {
    let mut service = Service::new();
    // The service's own store is the one checked for a running inbox.
    let (_, (_listener, _)) = service.serving();
    service.active(true);
    let own = store(&service.env);
    service.env.store = service.env.path("other.db");
    let other = store(&service.env);
    let (out, ok) = service.status(&[]);
    assert!(ok, "{out}");
    assert_lines(
        &out,
        &[
            &format!("Store:     {own}\n"),
            &format!("Problem:   installed for another store: {own} (this command uses {other})"),
        ],
    );

    let body = fs::read_to_string(service.file()).unwrap();
    fs::write(service.file(), body.replace(&exe(), "/nowhere/cases")).unwrap();
    let (out, _) = service.status(&[]);
    assert_lines(
        &out,
        &["Problem:   installed for another binary: /nowhere/cases"],
    );
}

/// A store named through `..` is the same store: the service's own serve is
/// found, and the service is not taken for another store's.
#[test]
fn a_store_named_through_a_parent_is_the_same_store() {
    let mut service = Service::new();
    let (addr, (_listener, _)) = service.serving();
    service.active(true);
    service.env.store = service.env.path("sub/../cases.db");
    let (out, ok) = service.status(&[]);
    assert!(ok, "{out}");
    assert!(!out.contains("Problem"), "{out}");
    let out = service.run(&["install", "--listen", &addr]);
    assert_eq!(out.code, 0, "{out:?}");
}

/// A store named through a symlink is the same store: status by its real
/// path takes the service for this store's, and a reinstall that names it
/// by its real path restarts the service's own serve.
#[test]
fn a_reinstall_through_a_symlink_is_the_same_store() {
    let mut service = Service::new();
    let real = service.env.store.clone();
    let link = service.env.path("link");
    std::os::unix::fs::symlink(real.parent().unwrap(), &link).unwrap();
    service.env.store = link.join(real.file_name().unwrap());
    let (addr, (_listener, _)) = service.serving();
    service.active(true);
    service.env.store = real;
    let (out, ok) = service.status(&[]);
    assert!(ok, "{out}");
    assert!(!out.contains("Problem"), "{out}");
    let out = service.run(&["install", "--listen", &addr]);
    assert_eq!(out.code, 0, "{out:?}");
}

/// A serve answering for the store does not make a service that is not
/// loaded a working one: status exits 1.
#[test]
fn service_status_when_installed_but_not_loaded_with_a_serve_answering() {
    let service = Service::new();
    let (_, (_listener, url)) = service.serving();
    let (out, ok) = service.status(&[]);
    assert!(!ok, "{out}");
    assert_lines(
        &out,
        &[
            "Loaded:    no",
            &format!("Inbox:     {url}"),
            "Problem:   installed but not loaded in",
        ],
    );
}

/// A service file that names no program is a problem status shows, not
/// an error that hides the rest.
#[test]
fn service_status_names_a_service_file_it_cannot_read() {
    let service = Service::new();
    service.install();
    // An empty plist, and a unit with an empty ExecStart, both name none.
    let body = if LAUNCHD {
        ""
    } else {
        "[Service]\nExecStart=\n"
    };
    fs::write(service.file(), body).unwrap();
    let (out, ok) = service.status(&[]);
    assert!(!ok, "{out}");
    assert_lines(
        &out,
        &[
            &format!("Installed: {}\n", service.file().display()),
            &format!(
                "Problem:   read {}: it names no program\n",
                service.file().display()
            ),
            "Inbox:     not running\n",
        ],
    );
    assert!(!out.contains("Runs:"), "{out}");
}

/// A config file the service's serve cannot read is a problem status
/// shows. The service is installed with a file
/// of its own, so that status itself reads the default one.
#[test]
fn service_status_names_a_config_file_it_cannot_read() {
    let mut service = Service::new();
    let path = service.env.path("service.toml");
    write_file(&path, "allow-hosts = [\"cases.example.com\"]\n");
    service.env.set("CASES_CONFIG", path.to_str().unwrap());
    service.install();
    service.env.unset("CASES_CONFIG");
    fs::write(&path, "allow-hosts = [\n").unwrap();
    let (out, _) = service.status(&[]);
    assert_lines(
        &out,
        &[
            &format!("--config {}", path.display()),
            &format!("Problem:   config file {}: ", path.display()),
        ],
    );
}

/// An instance file that cannot be read is a problem status shows, and
/// the inbox is not taken for running.
#[test]
fn service_status_names_an_instance_file_it_cannot_read() {
    let service = Service::new();
    service.install();
    service.active(true);
    let file = service.env.instance_file_of(&service.env.store);
    write_file(&file, "not json");
    let (out, ok) = service.status(&[]);
    assert!(!ok, "{out}");
    assert_lines(
        &out,
        &[
            "Inbox:     not running\n",
            &format!("Problem:   read {}: ", file.display()),
            "Problem:   loaded but the inbox does not answer",
        ],
    );
}

/// On a forwarded store, which has no file here, status is about the
/// installed service and its store, which it does not take for another.
#[test]
fn service_status_on_a_forwarded_store_is_about_the_installed_service() {
    let mut service = Service::new();
    let (addr, (_listener, url)) = service.serving();
    service.active(true);
    let own = store(&service.env);
    service.env.set("CASES_FORWARD", "ssh://devbox");
    // An ssh that records its runs as the stand-in does.
    common::copy(&service.bin.join("systemctl"), &service.bin.join("ssh"));
    service.calls();
    let out = service.env.run(["service"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_lines(
        &out.stdout,
        &[
            &format!("Store:     {own}\n"),
            &format!("Listen:    {addr}\n"),
            &format!("Inbox:     {url}"),
        ],
    );
    assert!(!out.stdout.contains("Problem"), "{out:?}");
    // Nothing but the service manager's queries ran: no ssh.
    for call in service.calls() {
        assert!(
            call.starts_with("launchctl print ") || call.starts_with("systemctl --user is-active "),
            "{call}"
        );
    }

    // With none installed there is no store to look for an inbox on.
    fs::remove_file(service.file()).unwrap();
    service.active(false);
    let out = service.env.run(["service"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stdout.contains("Inbox:     not running\n"), "{out:?}");
    assert!(!out.stdout.contains("Problem"), "{out:?}");
    for call in service.calls() {
        assert!(
            call.starts_with("launchctl print ") || call.starts_with("systemctl --user is-active "),
            "{call}"
        );
    }
}

/// systemd's user directory is under `$XDG_CONFIG_HOME`, or `~/.config`
/// when that is unset or empty.
#[test]
#[cfg(target_os = "linux")]
fn the_unit_is_under_home_without_xdg_config_home() {
    let mut service = Service::new();
    let unit = service
        .env
        .path("home")
        .join(".config/systemd/user/cases-serve.service");
    for xdg in [None, Some("")] {
        match xdg {
            None => service.env.unset("XDG_CONFIG_HOME"),
            Some(xdg) => service.env.set("XDG_CONFIG_HOME", xdg),
        }
        let (out, _) = service.status(&[]);
        assert_lines(&out, &[&format!("Installed: no ({})\n", unit.display())]);
    }
}

/// With neither `$XDG_CONFIG_HOME` nor `$HOME`, or both empty, there is no
/// systemd user directory: every command says so and touches nothing. The
/// config file is named, so that cases finds it without either.
#[test]
#[cfg(target_os = "linux")]
fn without_xdg_config_home_or_home_there_is_no_unit_directory() {
    let mut service = Service::new();
    let config = service.env.path("config.toml");
    write_file(&config, "");
    service.env.set("CASES_CONFIG", config.to_str().unwrap());
    for value in [None, Some("")] {
        for var in ["XDG_CONFIG_HOME", "HOME"] {
            match value {
                None => service.env.unset(var),
                Some(value) => service.env.set(var, value),
            }
        }
        // Status, which only reads, goes first: a regression that found a
        // home elsewhere fails there, before install could write in it.
        for args in [&[][..], &["uninstall"], &["install"]] {
            let out = service.run(args);
            assert_eq!(out.code, 1, "{args:?}: {out:?}");
            assert!(
                out.stderr.contains(
                    "cannot find the systemd user directory: neither $XDG_CONFIG_HOME nor $HOME \
                     is set"
                ),
                "{args:?}: {out:?}"
            );
        }
    }
    assert!(service.calls().is_empty());
    assert!(!service.env.path(".config").exists());
    assert!(!service.env.path("systemd").exists());
}

impl Service {
    /// Puts `script` on `$PATH` as `launchctl` and `systemctl`, in place of
    /// the stand-in, copied while no process starts so that running it does
    /// not fail as busy.
    fn replace_manager(&self, script: &str) {
        let source = self.env.path("manager.sh");
        write_file(&source, script);
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        for name in ["launchctl", "systemctl"] {
            common::copy(&source, &self.bin.join(name));
        }
    }
}

/// A service manager program that cannot be started is named in the error.
#[test]
fn a_service_manager_that_cannot_be_started_is_named() {
    let service = Service::new();
    service.replace_manager("#!/nowhere/sh\n");
    let program = service
        .bin
        .join(if LAUNCHD { "launchctl" } else { "systemctl" });
    let out = service.run(&[]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!("{}: ", program.display())),
        "{out:?}"
    );
}

/// A query killed by a signal is an error, not a service that is there.
#[test]
fn a_query_killed_by_a_signal_is_an_error() {
    let service = Service::new();
    service.replace_manager("#!/bin/sh\nkill -KILL $$\n");
    let out = service.run(&[]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("killed by a signal"), "{out:?}");
    assert!(!out.stdout.contains("Loaded:"), "{out:?}");
}

/// A query killed by a signal after it printed a state is an error, not
/// that state: status does not report the service loaded, or not loaded.
/// The states are systemd's; launchd's query is judged by its exit alone.
#[test]
fn a_query_killed_after_it_printed_is_an_error() {
    let service = Service::new();
    for state in ["activating", "inactive"] {
        service.replace_manager(&format!("#!/bin/sh\necho {state}\nkill -KILL $$\n"));
        let out = service.run(&[]);
        assert_eq!(out.code, 1, "{state}: {out:?}");
        // The state printed is in the error, so the query's output was read
        // and then not taken for an answer.
        let error = format!("killed by a signal: {state}");
        assert!(out.stderr.contains(&error), "{state}: {out:?}");
        assert!(!out.stdout.contains("Loaded:"), "{state}: {out:?}");
    }
}
