//! Helpers for the CLI's tests: the built `cases` binary run against a store
//! and a home of the test's own, and the store read and written directly, as
//! the commands not written yet, a hand edit or a newer cases would.

// Each test binary uses part of this module.
#![allow(dead_code)]

pub(crate) mod serve;

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};

use cases_core::{Append, Case, Store};
use cases_store_sqlite::{Db, Key, Writer};
use tempfile::TempDir;

/// The `LLVM_*` variables set in this process, as `cargo llvm-cov` sets them.
/// A command is run with a cleared environment, so without these the binary
/// it runs writes no profile and its code counts as uncovered. Under plain
/// `cargo test` none is set and nothing is passed on.
pub(crate) fn coverage_vars() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name.to_str().is_some_and(|name| name.starts_with("LLVM_")))
        .collect()
}

/// Making a socket or a pipe and starting a process take turns, across the
/// tests of one binary. On macOS std makes a socket or a pipe and only then
/// marks it close-on-exec, so a process another test starts in between
/// inherits it. An inherited socket holds its port for as long as the
/// process runs: `cases service install` then finds a port in use that the
/// test has let go, and a listener a test dropped still takes connections.
/// An inherited pipe keeps its other end from seeing the end of input or a
/// broken pipe. Starting a process makes its own pipes, so starts take turns
/// with each other too.
static FDS: Mutex<()> = Mutex::new(());

/// A listener on `addr`, made while no process starts.
pub(crate) fn listen(addr: &str) -> TcpListener {
    try_listen(addr).expect("a listener")
}

/// A listener on `addr`, made while no process starts, or why there is
/// none, such as the address being taken.
pub(crate) fn try_listen(addr: &str) -> std::io::Result<TcpListener> {
    let _alone = FDS.lock().unwrap_or_else(PoisonError::into_inner);
    TcpListener::bind(addr)
}

/// A pipe, made while no process starts.
pub(crate) fn pipe() -> (std::io::PipeReader, std::io::PipeWriter) {
    let _alone = FDS.lock().unwrap_or_else(PoisonError::into_inner);
    std::io::pipe().expect("a pipe")
}

/// Copies the file at `from` to `to`, while no process starts: a process
/// started meanwhile would hold `to` open for writing, and running `to`
/// would then fail as busy (ETXTBSY).
pub(crate) fn copy(from: &Path, to: &Path) {
    let _alone = FDS.lock().unwrap_or_else(PoisonError::into_inner);
    std::fs::copy(from, to).expect("a copy");
}

/// Starts `command`, while no other process starts and no socket or pipe
/// is being made.
pub(crate) fn spawn(command: &mut Command) -> Child {
    let _alone = FDS.lock().unwrap_or_else(PoisonError::into_inner);
    command.spawn().expect("the process starts")
}

/// Runs `command` to its end, starting it with [`spawn`]. Unlike
/// [`Command::output`], it pipes only what `command` says to pipe.
pub(crate) fn output(command: &mut Command) -> std::process::Output {
    spawn(command)
        .wait_with_output()
        .expect("the process finishes")
}

/// A scratch home for one test: its own `$HOME` and XDG directories, which
/// nothing else uses, and a store path in it where no file is yet.
pub(crate) struct Env {
    dir: TempDir,
    /// The store the test uses with `--store`.
    pub(crate) store: PathBuf,
    vars: Vec<(String, String)>,
}

/// What one run of `cases` did.
#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl From<std::process::Output> for Output {
    fn from(output: std::process::Output) -> Output {
        Output {
            code: output.status.code().expect("an exit code"),
            stdout: String::from_utf8(output.stdout).expect("UTF-8 on stdout"),
            stderr: String::from_utf8(output.stderr).expect("UTF-8 on stderr"),
        }
    }
}

impl Env {
    pub(crate) fn new() -> Env {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = dir.path().join("cases.db");
        let mut env = Env {
            dir,
            store,
            vars: Vec::new(),
        };
        for (var, sub) in [
            ("HOME", "home"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
        ] {
            let path = env.path(sub);
            env.set(var, path.to_str().expect("a UTF-8 path"));
        }
        // No test may reach the real keychain: a login is kept in, and read
        // from, the credentials file in the test's own config directory.
        env.set("CASES_NO_KEYCHAIN", "1");
        env
    }

    /// `name` in the test's directory.
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Sets an environment variable for every later run.
    pub(crate) fn set(&mut self, var: &str, value: &str) {
        self.unset(var);
        self.vars.push((var.into(), value.into()));
    }

    /// Unsets an environment variable for every later run.
    pub(crate) fn unset(&mut self, var: &str) {
        self.vars.retain(|(name, _)| name != var);
    }

    /// Runs `cases` with `args` and nothing on stdin.
    pub(crate) fn run<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_stdin("", args)
    }

    /// `cases` with `args`, to run in the test's directory with only the
    /// environment the test set.
    pub(crate) fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command_as(Path::new(env!("CARGO_BIN_EXE_cases")), args)
    }

    /// As [`Env::command`], with the binary at `program`.
    pub(crate) fn command_as<I, S>(&self, program: &Path, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(coverage_vars())
            .envs(self.vars.iter().map(|(k, v)| (k, v)))
            .current_dir(self.dir.path());
        command
    }

    /// Runs `cases` with `args` and `stdin`, in the test's directory, with
    /// only the environment the test set.
    pub(crate) fn run_with_stdin<I, S>(&self, stdin: &str, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = spawn(
            self.command(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        // A command refused before it reads stdin may exit before the text
        // is written, so the write meets a closed pipe. That is not a
        // failure: the test checks what the command printed and its exit.
        match child
            .stdin
            .take()
            .expect("a stdin")
            .write_all(stdin.as_bytes())
        {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            result => result.expect("stdin takes the text"),
        }
        child.wait_with_output().expect("cases finishes").into()
    }

    /// Runs `cases --store <store>` with `args`.
    pub(crate) fn cases(&self, args: &[&str]) -> Output {
        self.run(self.store_args(args))
    }

    /// `--store <store>` and then `args`.
    pub(crate) fn store_args<'a>(&'a self, args: &[&'a str]) -> Vec<&'a str> {
        let mut all = vec!["--store", self.store.to_str().expect("a UTF-8 path")];
        all.extend(args);
        all
    }

    /// Runs `cases --store <store>` with `args`, which must succeed, and
    /// gives its stdout.
    pub(crate) fn ok(&self, args: &[&str]) -> String {
        let out = self.cases(args);
        assert_eq!(out.code, 0, "cases {args:?}: {out:?}");
        out.stdout
    }

    /// Opens a case with `args` after `open`, and gives its id.
    pub(crate) fn open(&self, args: &[&str]) -> String {
        let mut all = vec!["open"];
        all.extend(args);
        self.ok(&all).trim().to_owned()
    }

    /// Opens an fyi case titled `title`, urgency today, and gives its id.
    pub(crate) fn open_fyi(&self, title: &str) -> String {
        self.open(&["--kind", "fyi", "--urgency", "today", "--title", title])
    }

    /// Opens a stuck case, which a human can park, and gives its id.
    pub(crate) fn open_stuck(&self) -> String {
        self.open(&[
            "--kind",
            "stuck",
            "--urgency",
            "blocking",
            "--title",
            "Blocked on mirror",
        ])
    }

    /// Opens a two-option decision case and gives its id.
    pub(crate) fn open_decision(&self) -> String {
        self.open(&[
            "--kind",
            "decision",
            "--urgency",
            "today",
            "--title",
            "Pin bun?",
            "--option",
            "Pin to 1.2.3",
            "--option",
            "Float, with renovate",
        ])
    }

    /// The store, to read and write directly.
    pub(crate) fn db(&self) -> Db {
        Db::new(&self.store)
    }

    /// The case `id`, folded.
    pub(crate) fn case(&self, id: &str) -> Case {
        let rows = self
            .db()
            .rows(&key(id))
            .expect("the store reads")
            .expect("the case is in the store");
        Case::fold(&rows).expect("the case folds")
    }

    /// Writes an event to the case `id` as `author`, with no name, as the
    /// commands not written yet would.
    pub(crate) fn write(&self, id: &str, author: &str, event: &str, data: &str) -> Case {
        self.write_as(id, author, event, data, None)
    }

    /// Writes an event to the case `id` as the writer named `name`.
    pub(crate) fn write_as(
        &self,
        id: &str,
        author: &str,
        event: &str,
        data: &str,
        name: Option<&str>,
    ) -> Case {
        let append = Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        };
        self.db()
            .append(&key(id), &append, &Writer::new(name), None)
            .expect("the event is written")
    }

    /// Stores a row as it is, with none of the store's checks, as a hand
    /// edit or a newer cases could: to the case `id`, or a new case when
    /// `id` is `None`. Gives the case's id.
    pub(crate) fn insert(
        &self,
        id: Option<&str>,
        seq: u64,
        author: &str,
        event: &str,
        data: &str,
    ) -> String {
        let conn = rusqlite::Connection::open(&self.store).expect("the store opens");
        let key = if let Some(id) = id {
            key(id)
        } else {
            conn.execute("INSERT INTO cases (opened_at) VALUES ('')", [])
                .expect("a case row");
            Key::new(u64::try_from(conn.last_insert_rowid()).expect("a key")).expect("a key")
        };
        insert_row(&conn, key, seq, author, event, data);
        key.to_string()
    }

    /// Writes a config file at the default place, `$XDG_CONFIG_HOME/cases/config.toml`.
    pub(crate) fn config(&self, text: &str) -> PathBuf {
        let path = self.path("config").join("cases").join("config.toml");
        write_file(&path, text);
        path
    }

    /// The instance file a running serve on the store keeps, as `cases`
    /// finds it.
    pub(crate) fn instance_file(&self) -> PathBuf {
        self.instance_file_of(&self.store)
    }

    /// The instance file a running serve on the store at `store` keeps.
    pub(crate) fn instance_file_of(&self, store: &Path) -> PathBuf {
        // Resolved as cases resolves it: its symlinks, as far as it is
        // there, and the rest as text.
        let store = std::path::absolute(store).expect("an absolute path");
        let there = store
            .ancestors()
            .find_map(|there| Some((there, std::fs::canonicalize(there).ok()?)));
        let store = match there {
            Some((there, real)) => real.join(store.strip_prefix(there).expect("a prefix")),
            None => store,
        };
        let store: PathBuf = store.components().fold(PathBuf::new(), |mut path, part| {
            match part {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    path.pop();
                }
                part => path.push(part),
            }
            path
        });
        let hash = store
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
            });
        // The slug of a plain name such as `other.db`.
        let name = store
            .file_name()
            .expect("a file name")
            .to_str()
            .expect("a UTF-8 name")
            .to_lowercase()
            .replace('.', "-");
        self.path("state")
            .join("cases")
            .join(format!("serve-{name}-{hash:016x}.json"))
    }
}

/// A serve on the store as `cases` finds one: an instance file naming this
/// test's process, which is alive, and a listener of the test's own, which
/// accepts connections until it is dropped. Listens on `addr`, such as
/// `127.0.0.1:0`. Gives the listener and the inbox's URL.
impl Env {
    pub(crate) fn fake_serve(&self, addr: &str) -> (TcpListener, String) {
        self.fake_serve_on(&self.store, addr)
    }

    /// A serve on the store at `store`, as [`Env::fake_serve`] fakes one.
    pub(crate) fn fake_serve_on(&self, store: &Path, addr: &str) -> (TcpListener, String) {
        self.fake_serve_with(store, listen(addr))
    }

    /// A serve on the store at `store`, as [`Env::fake_serve`] fakes one,
    /// on a listener the test bound.
    pub(crate) fn fake_serve_with(
        &self,
        store: &Path,
        listener: TcpListener,
    ) -> (TcpListener, String) {
        let addr = listener.local_addr().expect("an address");
        let url = format!("http://{addr}/");
        let info = serde_json::json!({
            "pid": std::process::id(),
            "url": url,
            "addr": addr.to_string(),
            "store": std::path::absolute(store).expect("an absolute path"),
            "started_at": "2026-09-25T12:00:00Z",
            "version": "0.0.0",
        });
        write_file(&self.instance_file_of(store), &info.to_string());
        (listener, url)
    }

    /// Records a serve of process `pid` on this store whose address takes no
    /// connection, as a serve that stopped without removing its file leaves.
    /// It is at port 0, which takes no connection, ever: a port freed for
    /// the test could be taken by another before cases dials it.
    pub(crate) fn stale_instance(&self, pid: u32) {
        let info = serde_json::json!({
            "pid": pid,
            "url": "http://127.0.0.1:0/",
            "addr": "127.0.0.1:0",
            "store": std::path::absolute(&self.store).expect("an absolute path"),
            "started_at": "2026-09-25T12:00:00Z",
            "version": "0.0.0",
        });
        write_file(&self.instance_file(), &info.to_string());
    }
}

/// The key the id `id` names.
pub(crate) fn key(id: &str) -> Key {
    id.parse().expect("an id")
}

/// Stores a row in the case `key` through `conn` as it is, with none of the
/// store's checks and no time, as a hand edit or another writer could.
pub(crate) fn insert_row(
    conn: &rusqlite::Connection,
    key: Key,
    seq: u64,
    author: &str,
    event: &str,
    data: &str,
) {
    conn.execute(
        "INSERT INTO events (case_key, seq, author, event, at, data) VALUES (?1, ?2, ?3, ?4, '', ?5)",
        rusqlite::params![
            i64::try_from(key.get()).expect("a key"),
            i64::try_from(seq).expect("a seq"),
            author,
            event,
            data.as_bytes()
        ],
    )
    .expect("the row is stored");
}

/// Makes `dir` and its parents owner-only (0700), whatever the umask: the
/// credentials checks warn about a directory others can write to, and a
/// shared devbox umask of 002 would make a plain `create_dir_all` one.
pub(crate) fn private_dir_all(dir: &Path) {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .expect("the directory is made");
}

/// Writes `text` to `path`, making its directory.
pub(crate) fn write_file(path: &Path, text: &str) {
    private_dir_all(path.parent().expect("a directory"));
    std::fs::write(path, text).expect("the file is written");
}

/// Reads `text` as JSON.
pub(crate) fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

impl Env {
    /// Opens a one-row approval case with a context and a label, and gives
    /// its id.
    pub(crate) fn open_approval(&self) -> String {
        self.open(&[
            "--kind",
            "approval",
            "--urgency",
            "today",
            "--title",
            "Scripts",
            "--context",
            "Release 1.4",
            "--label",
            "release",
            "--row",
            r#"{"id":"deps","label":"Install deps","script":"npm ci","link":"https://example.com/deps"}"#,
        ])
    }

    /// The case `id`'s revision.
    pub(crate) fn revision(&self, id: &str) -> u64 {
        self.db()
            .rows(&key(id))
            .expect("the store reads")
            .expect("the case is in the store")
            .len()
            .try_into()
            .expect("a revision")
    }

    /// Runs a write, with `args` after `--store`, that must be refused as
    /// stale, the case `id` having moved on from revision `read` to `now`,
    /// and checks that it wrote nothing. Gives its stderr.
    pub(crate) fn refused_as_stale(&self, id: &str, read: u64, now: u64, args: &[&str]) -> String {
        assert_eq!(self.revision(id), now, "the case is not at revision {now}");
        let out = self.cases(args);
        assert_eq!(out.code, 4, "{args:?}: {out:?}");
        let want = format!("read at revision {read}, now at {now}; it is now ");
        assert!(out.stderr.contains(&want), "{args:?}: {out:?}");
        assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
        assert_eq!(out.stdout, "");
        assert_eq!(self.revision(id), now, "a refused write changed the case");
        out.stderr
    }

    /// Runs a write, with `args` after `--store`, that must be refused, and
    /// checks that it wrote nothing to the case `id`. Gives its stderr.
    pub(crate) fn refused(&self, id: &str, code: i32, args: &[&str]) -> String {
        self.refused_with_stdin(id, code, "", args)
    }

    /// `refused`, with `stdin`.
    pub(crate) fn refused_with_stdin(
        &self,
        id: &str,
        code: i32,
        stdin: &str,
        args: &[&str],
    ) -> String {
        let before = self.revision(id);
        let out = self.run_with_stdin(stdin, self.store_args(args));
        assert_eq!(out.code, code, "{args:?}: {out:?}");
        assert_eq!(out.stdout, "", "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}: {out:?}");
        eprintln!(
            "refused {args:?}: {}",
            out.stderr.lines().next().unwrap_or_default()
        );
        assert_eq!(
            self.revision(id),
            before,
            "{args:?}: a refused write changed the case"
        );
        out.stderr
    }

    /// Each case in the store's archive, by id, with its number of rows.
    pub(crate) fn archived(&self) -> std::collections::BTreeMap<String, u64> {
        let conn = rusqlite::Connection::open(&self.store).expect("the store opens");
        let mut statement = conn
            .prepare("SELECT c.key, count(e.seq) FROM archive_cases c LEFT JOIN archive_events e ON e.case_key = c.key GROUP BY c.key")
            .expect("the query reads");
        statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .expect("the query runs")
            .map(|row| {
                let (key, rows) = row.expect("a row");
                let key = Key::new(u64::try_from(key).expect("a key")).expect("a key");
                (key.to_string(), u64::try_from(rows).expect("a count"))
            })
            .collect()
    }
}

/// RFC 3339 for the time `secs` seconds from now, or before now when
/// negative.
pub(crate) fn from_now(secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs();
    let now = i64::try_from(now).expect("a clock before 2262");
    cases_core::Time::from_unix(now + secs, 0)
        .expect("a time")
        .to_string()
}

impl Env {
    /// Opens an fyi case with its open event dated `at`, and `extra` fields
    /// on it, through the store. Gives its id.
    pub(crate) fn open_at(&self, title: &str, at: &str, extra: &str) -> String {
        let data = format!(
            r#"{{"kind": "fyi", "urgency": "whenever", "title": "{title}", "opened_at": "{at}"{extra}}}"#
        );
        let append = Append {
            author: "agent".into(),
            event: "open".into(),
            data,
        };
        let (key, _) = self
            .db()
            .create(&append, &Writer::new(None))
            .expect("the case opens");
        key.to_string()
    }
}
