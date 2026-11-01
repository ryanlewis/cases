//! Where a running `cases serve` is: the instance file it keeps in the
//! state directory while it runs (SV-02), which `show`, `status` and `inbox`
//! read.

use std::env;
use std::fs;
use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::output;

/// What the instance file records about a running serve, and what
/// `status --json` prints (AG-15).
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Info {
    /// The serve's process.
    pub(crate) pid: i32,
    /// The inbox's address, such as `http://127.0.0.1:7337/`.
    pub(crate) url: String,
    /// The inbox's address in the links cases gives, such as a case's `url`:
    /// `https://<host>/` for the first of the config file's `allow-hosts`,
    /// so that a link opened on another device of the tailnet reaches this
    /// inbox, else `url`. Empty, and left out, in a file from before it
    /// was recorded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) link: String,
    /// The address serve listens on.
    pub(crate) addr: String,
    /// The store serve runs on, as an absolute path.
    #[serde(default)]
    pub(crate) store: String,
    /// When serve started, in RFC 3339 UTC to the second.
    #[serde(default)]
    pub(crate) started_at: String,
    /// The version of cases serve is.
    #[serde(default)]
    pub(crate) version: String,
}

impl Info {
    /// The inbox's address in links: `link`, or `url` in a file that has
    /// none.
    pub(crate) fn link(&self) -> &str {
        if self.link.is_empty() {
            &self.url
        } else {
            &self.link
        }
    }
}

/// The inbox's address in the links cases gives, for a serve at `url` with
/// the config file's `allow-hosts` as `proxied`: `https://` the first of
/// them, the proxy's, which every device of the tailnet can reach, or `url`
/// when there are none.
pub(crate) fn link(url: &str, proxied: &[String]) -> String {
    match proxied.first() {
        Some(host) => format!("https://{host}/"),
        None => url.to_owned(),
    }
}

/// How long to wait for the serve's address to accept a connection.
const DIAL_TIMEOUT: Duration = Duration::from_millis(500);

/// The state directory: `$XDG_STATE_HOME/cases`, or `~/.local/state/cases`.
///
/// # Errors
///
/// When neither `$XDG_STATE_HOME` nor `$HOME` is set.
pub(crate) fn dir() -> Result<PathBuf, Error> {
    let base = if let Some(dir) = env::var_os("XDG_STATE_HOME").filter(|dir| !dir.is_empty()) {
        PathBuf::from(dir)
    } else {
        let home = env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .ok_or_else(|| {
                Error::new(
                    "cannot find the state directory: neither $XDG_STATE_HOME nor $HOME is set",
                )
            })?;
        PathBuf::from(home).join(".local").join("state")
    };
    Ok(base.join("cases"))
}

/// The instance file for the store at `store`, in the state directory:
/// `serve-<file name>-<hash>.json`, where the hash is of the store's absolute
/// path, so that serves on two stores never share a file, and the file name
/// is there for people. It is the same for every build of cases, since
/// each finds the serve another started.
pub(crate) fn path(store: &Path) -> Result<PathBuf, Error> {
    let store = resolve(&std::path::absolute(store)?);
    let name = output::slug(&store.file_name().unwrap_or_default().to_string_lossy());
    let hash = fnv1a(store.as_os_str().as_encoded_bytes());
    Ok(dir()?.join(format!("serve-{name}-{hash:016x}.json")))
}

/// The serve recorded for the store at `store`, if it is still running: its
/// process is there and its address accepts a connection. The second check
/// covers a process id the system gave to another process after serve
/// stopped without removing the file.
///
/// # Errors
///
/// When the file is there but cannot be read, or is not an instance file.
pub(crate) fn running(store: &Path) -> Result<Option<Info>, Error> {
    let path = path(store)?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::new(format!("read {}: {error}", path.display()))),
    };
    let info: Info = serde_json::from_str(&text)
        .map_err(|error| Error::new(format!("read {}: {error}", path.display())))?;
    if info.pid <= 0 || !alive(info.pid) || !accepts(&info.addr) {
        return Ok(None);
    }
    Ok(Some(info))
}

/// `path` with each `..` taking off the name before it, so that
/// `a/../cases.db` and `cases.db` name one store and so one instance file.
/// `std::path::absolute` keeps `..`.
pub(crate) fn clean(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean
}

/// The absolute path `path` with its symlinks resolved, so that every path
/// to one store gives one instance file: `link/../cases.db`, where `link` is
/// a symlink to a directory, is the `cases.db` beside that directory, not
/// the one beside `link`. A path that is not there yet, such as a store
/// serve has still to make, resolves as far as it is there, and the rest is
/// cleaned as text.
fn resolve(path: &Path) -> PathBuf {
    for there in path.ancestors() {
        if let Ok(real) = fs::canonicalize(there) {
            let rest = path.strip_prefix(there).unwrap_or(Path::new(""));
            return clean(&real.join(rest));
        }
    }
    clean(path)
}

/// The page of the case `id` in the inbox at `base`.
pub(crate) fn case_url(base: &str, id: &str) -> String {
    format!("{}/cases/{id}", base.trim_end_matches('/'))
}

/// Whether something accepts a connection at `addr`.
fn accepts(addr: &str) -> bool {
    let addrs: Vec<SocketAddr> = addr.to_socket_addrs().into_iter().flatten().collect();
    addrs
        .iter()
        .any(|addr| TcpStream::connect_timeout(addr, DIAL_TIMEOUT).is_ok())
}

/// Whether the process `pid` is there. One that belongs to another user is.
#[cfg(unix)]
fn alive(pid: i32) -> bool {
    use rustix::io::Errno;
    use rustix::process::{Pid, test_kill_process};
    Pid::from_raw(pid)
        .is_some_and(|pid| matches!(test_kill_process(pid), Ok(()) | Err(Errno::PERM)))
}

/// Off unix, the address alone says whether serve is running.
#[cfg(not(unix))]
fn alive(_pid: i32) -> bool {
    true
}

/// The 64-bit FNV-1a hash of `bytes`, which never changes between builds.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Records `info` as the serve running on the store at `store`, in place of
/// any file there.
///
/// # Errors
///
/// When the state directory cannot be found or made, or the file cannot be
/// written.
pub(crate) fn write(store: &Path, info: &Info) -> Result<(), Error> {
    let path = path(store)?;
    write_at(&path, info).map_err(|error| Error::new(format!("{}: {error}", path.display())))
}

/// Writes `info` to `path`, by way of a file beside it that is renamed into
/// place, so that a reader never finds half a file. The state directory is
/// made for this user alone.
fn write_at(path: &Path, info: &Info) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)?;
    let mut text = serde_json::to_string_pretty(info)?;
    text.push('\n');
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Removes the file for the store at `store` if it still records `pid`, so
/// that a serve never removes a file another serve wrote. A file that does
/// not read as an instance file is removed.
///
/// # Errors
///
/// When the file is there and cannot be read or removed.
pub(crate) fn remove(store: &Path, pid: i32) -> Result<(), Error> {
    let path = path(store)?;
    remove_at(&path, pid).map_err(|error| Error::new(format!("{}: {error}", path.display())))
}

fn remove_at(path: &Path, pid: i32) -> io::Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if serde_json::from_str::<Info>(&text).is_ok_and(|info| info.pid != pid) {
        return Ok(());
    }
    fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_the_published_hash() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_through_a_parent_is_the_same_store() {
        assert_eq!(
            clean(Path::new("/a/b/../c/./cases.db")),
            Path::new("/a/c/cases.db")
        );
        assert_eq!(clean(Path::new("/../cases.db")), Path::new("/cases.db"));
        if dir().is_ok() {
            assert_eq!(
                path(Path::new("/tmp/x/../cases.db")).unwrap(),
                path(Path::new("/tmp/cases.db")).unwrap()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_through_a_symlink_and_a_parent_is_the_same_store() {
        let dir = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(real.join("a").join("b")).unwrap();
        std::os::unix::fs::symlink(real.join("a").join("b"), real.join("link")).unwrap();
        let through = real.join("link").join("..").join("cases.db");
        let store = real.join("a").join("cases.db");
        assert_eq!(resolve(&through), store, "a store that is not there yet");
        fs::write(&store, "").unwrap();
        assert_eq!(resolve(&through), store);
        assert_eq!(resolve(&store), store);
        let missing = real.join("x").join("..").join("cases.db");
        assert_eq!(resolve(&missing), real.join("cases.db"));
    }

    #[test]
    fn a_case_url_is_under_the_inbox() {
        assert_eq!(
            case_url("http://127.0.0.1:7337/", "UkLWZ"),
            "http://127.0.0.1:7337/cases/UkLWZ"
        );
        assert_eq!(
            case_url("http://127.0.0.1:7337", "UkLWZ"),
            "http://127.0.0.1:7337/cases/UkLWZ"
        );
    }

    #[test]
    fn a_link_names_the_first_allowed_host_or_the_loopback_address() {
        let url = "http://127.0.0.1:7337/";
        assert_eq!(link(url, &[]), url);
        let hosts = ["devbox.example.ts.net:7337".to_owned(), "devbox".to_owned()];
        assert_eq!(link(url, &hosts), "https://devbox.example.ts.net:7337/");
        let mut info = info(42);
        assert_eq!(info.link(), url, "a file with no link");
        info.link = link(url, &hosts);
        assert_eq!(info.link(), "https://devbox.example.ts.net:7337/");
    }

    #[cfg(unix)]
    #[test]
    fn this_process_is_alive() {
        let pid = i32::try_from(std::process::id()).unwrap();
        assert!(alive(pid));
        assert!(!alive(0));
    }

    fn info(pid: i32) -> Info {
        Info {
            pid,
            url: "http://127.0.0.1:7337/".into(),
            link: String::new(),
            addr: "127.0.0.1:7337".into(),
            store: "/data/cases.db".into(),
            started_at: "2026-09-25T12:00:00Z".into(),
            version: "0.0.0".into(),
        }
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("state")
            .join("cases")
            .join("serve-cases-db-0.json");
        write_at(&path, &info(42)).unwrap();
        write_at(&path, &info(43)).unwrap();
        let read: Info = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((read.pid, read.store.as_str()), (43, "/data/cases.db"));
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [path.file_name().unwrap()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn a_write_fails_when_the_state_directory_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::write(&state, "").unwrap();
        assert!(write_at(&state.join("serve-cases-db-0.json"), &info(42)).is_err());
    }

    #[test]
    fn a_serve_removes_only_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve-cases-db-0.json");
        write_at(&path, &info(42)).unwrap();
        remove_at(&path, 43).unwrap();
        assert!(path.exists(), "the file of pid 42 was removed by pid 43");
        remove_at(&path, 42).unwrap();
        assert!(!path.exists());
        remove_at(&path, 42).unwrap();
        fs::write(&path, "not an instance file").unwrap();
        remove_at(&path, 42).unwrap();
        assert!(!path.exists(), "a file that does not read was left");
    }
}
