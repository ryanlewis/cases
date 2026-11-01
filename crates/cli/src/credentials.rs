//! Where `cases login` keeps the token for a server (AC-03): the macOS
//! login keychain, or `credentials.toml` in the config directory, which
//! only its owner can read. One login per server, keyed by the server's
//! address as [`Server`] writes it.
//!
//! On macOS the keychain is used unless `CASES_NO_KEYCHAIN` is set and not
//! empty; then it is never touched, not even read. Every test sets it, so
//! no test run reaches the real keychain. A keychain that cannot be written
//! sends the token to the file, with a warning. Elsewhere the file is used.
//!
//! The file is written by hand, each string quoted as JSON, which TOML
//! reads as a basic string, so the `toml` crate is used only to read:
//!
//! ```toml
//! # cases login's tokens, one per server. Keep this file private.
//! ["https://cases.example"]
//! account = "ac_…"
//! token = "csu_…"
//! ```
//!
//! Nothing here prints a token, and no error holds one.

use std::collections::BTreeMap;
use std::env;
use std::fmt::{self, Write as _};
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use cases_store_remote::{Server, Token};

use crate::config;
use crate::ctx;
use crate::error::Error;

/// The variable that keeps the keychain out, set and not empty: the token
/// is kept in, and read from, the file alone.
pub(crate) const NO_KEYCHAIN: &str = "CASES_NO_KEYCHAIN";

/// The keychain item's service. Its account is the server.
#[cfg(target_os = "macos")]
const SERVICE: &str = "cases";

/// The file's first line.
const HEADER: &str = "# cases login's tokens, one per server. Keep this file private.\n";

/// Where a login is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// The macOS login keychain.
    Keychain,
    /// The credentials file.
    File(PathBuf),
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::Keychain => f.write_str("the macOS keychain"),
            Place::File(path) => path.display().fmt(f),
        }
    }
}

/// The login kept for a server.
#[derive(Clone, Debug)]
pub(crate) struct Login {
    /// The token. Its `Debug` and `Display` hide it.
    pub(crate) token: Token,
    /// The account's id, when the file has it. The keychain item holds the
    /// token alone: logout finds the account in the kept list, or asks the
    /// server.
    pub(crate) account: Option<String>,
    /// Where it was found.
    pub(crate) place: Place,
}

/// One server's entry in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    account: Option<String>,
    token: Token,
}

/// Whether the keychain is used: on macOS, unless `CASES_NO_KEYCHAIN` is
/// set and not empty.
fn keychain_on() -> bool {
    cfg!(target_os = "macos") && env::var_os(NO_KEYCHAIN).is_none_or(|value| value.is_empty())
}

/// The credentials file, `credentials.toml` in the config directory. It is
/// there even when `--config` or `CASES_CONFIG` names another config file,
/// since that file may be shared and the credentials are not.
///
/// # Errors
///
/// When neither `$XDG_CONFIG_HOME` nor `$HOME` is set.
pub(crate) fn file() -> Result<PathBuf, Error> {
    config::config_dir()
        .map(|dir| dir.join("credentials.toml"))
        .ok_or_else(|| {
            Error::new(
                "cannot find the credentials file: neither $XDG_CONFIG_HOME nor $HOME is set",
            )
        })
}

/// The login kept for `server`, if there is one: in the keychain when it is
/// used, then in the file. Off a terminal, a keychain read that would need
/// the human's approval fails at once rather than showing a dialog nobody
/// sees.
///
/// # Errors
///
/// When the keychain or the file cannot be read, or what it holds for
/// `server` is not a token. Neither is ever skipped: a source skipped
/// quietly could send another token.
pub(crate) fn read(server: &Server) -> Result<Option<Login>, Error> {
    read_at(&file()?, keychain_on(), server)
}

/// Keeps `token` for `server`, with the account's id beside it in the
/// file, in place of any login kept for it before, and says where. In the
/// keychain, the file's entry for `server` goes, so only one copy is ever
/// read. When the keychain is not used on macOS, or cannot be written, the
/// token goes to the file with a warning that says why.
///
/// # Errors
///
/// When the token cannot be kept: the file cannot be read or written.
pub(crate) fn write(server: &Server, account: &str, token: &Token) -> Result<Place, Error> {
    write_at(&file()?, keychain_on(), server, account, token)
}

/// Forgets the login kept for `server`, in the keychain when it is used and
/// in the file, and gives where one was found.
///
/// # Errors
///
/// When the keychain or the file cannot be read or written.
pub(crate) fn remove(server: &Server) -> Result<Vec<Place>, Error> {
    remove_at(&file()?, keychain_on(), server)
}

/// [`read()`], with the file at `path`, and the keychain when `keychain`.
fn read_at(path: &Path, keychain: bool, server: &Server) -> Result<Option<Login>, Error> {
    if keychain && let Some(token) = keychain::read(server)? {
        return Ok(Some(Login {
            token,
            account: None,
            place: Place::Keychain,
        }));
    }
    Ok(read_file(path)?.remove(server.as_str()).map(|entry| Login {
        token: entry.token,
        account: entry.account,
        place: Place::File(path.to_owned()),
    }))
}

/// [`write()`], with the file at `path`, and the keychain when `keychain`.
fn write_at(
    path: &Path,
    keychain: bool,
    server: &Server,
    account: &str,
    token: &Token,
) -> Result<Place, Error> {
    if keychain {
        match keychain::write(server, token) {
            Ok(()) => {
                let mut entries = read_file(path)?;
                if entries.remove(server.as_str()).is_some() {
                    write_file(path, &entries)?;
                }
                return Ok(Place::Keychain);
            }
            Err(why) => ctx::warn(format_args!(
                "the keychain cannot be written ({why}), so the token for {server} goes in {}",
                path.display()
            )),
        }
    } else if cfg!(target_os = "macos") {
        ctx::warn(format_args!(
            "{NO_KEYCHAIN} is set, so the token for {server} goes in {}, not the keychain",
            path.display()
        ));
    }
    let mut entries = read_file(path)?;
    entries.insert(
        server.as_str().to_owned(),
        Entry {
            account: Some(account.to_owned()),
            token: token.clone(),
        },
    );
    write_file(path, &entries)?;
    Ok(Place::File(path.to_owned()))
}

/// [`remove()`], with the file at `path`, and the keychain when `keychain`.
fn remove_at(path: &Path, keychain: bool, server: &Server) -> Result<Vec<Place>, Error> {
    let mut found = Vec::new();
    if keychain && keychain::remove(server)? {
        found.push(Place::Keychain);
    }
    let mut entries = read_file(path)?;
    if entries.remove(server.as_str()).is_some() {
        write_file(path, &entries)?;
        found.push(Place::File(path.to_owned()));
    }
    Ok(found)
}

/// The logins in the file at `path`, by server; none when it is not there.
/// A file or directory others can reach is warned about.
///
/// # Errors
///
/// When the file cannot be read, is not TOML, or an entry is not a server's
/// table with a token. The error names the file and the line or the server,
/// never what the file holds.
fn read_file(path: &Path) -> Result<BTreeMap<String, Entry>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(Error::new(format!(
                "the credentials file {} cannot be read: {error}",
                path.display()
            )));
        }
    };
    warn_if_open(path);
    if let Some(dir) = path.parent() {
        warn_if_open(dir);
    }
    let bad = |why: String| {
        Error::new(format!(
            "the credentials file {} {why}: fix it, or remove it and run cases login again",
            path.display()
        ))
    };
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
        // toml's message can quote the file, so only the line is given.
        let line = error.span().map_or(0, |span| {
            text.get(..span.start)
                .unwrap_or(&text)
                .matches('\n')
                .count()
                + 1
        });
        bad(format!("is not TOML at line {line}"))
    })?;
    let mut entries = BTreeMap::new();
    for (server, value) in table {
        let shape = || bad(format!("has no token for {server:?}"));
        let toml::Value::Table(entry) = value else {
            return Err(shape());
        };
        let token = entry
            .get("token")
            .and_then(toml::Value::as_str)
            .ok_or_else(shape)?
            .parse::<Token>()
            .map_err(|_| bad(format!("holds a token for {server:?} that is not one")))?;
        let account = match entry.get("account") {
            None => None,
            Some(toml::Value::String(account)) => Some(account.clone()),
            Some(_) => {
                return Err(bad(format!(
                    "holds an account for {server:?} that is not text"
                )));
            }
        };
        entries.insert(server, Entry { account, token });
    }
    Ok(entries)
}

/// Warns when others than the owner can read or write the file at `path`,
/// or write to the directory at `path`, which lets them replace the file in
/// it, with the `chmod` to fix it.
#[cfg(unix)]
fn warn_if_open(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    let mode = meta.permissions().mode();
    let (open, can, fix) = if meta.is_dir() {
        (mode & 0o022 != 0, "written to", "700")
    } else {
        (mode & 0o077 != 0, "read", "600")
    };
    if open {
        ctx::warn(format_args!(
            "{} can be {can} by others: make it private with chmod {fix} {}",
            path.display(),
            path.display()
        ));
    }
}

/// Off unix, a file's mode says nothing to warn about.
#[cfg(not(unix))]
fn warn_if_open(_path: &Path) {}

/// The file's text for `entries`, each string quoted as JSON, which TOML
/// reads as a basic string.
fn render(entries: &BTreeMap<String, Entry>) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut text = String::from(HEADER);
    for (server, entry) in entries {
        let _ = write!(text, "\n[{}]\n", quote(server));
        if let Some(account) = &entry.account {
            let _ = writeln!(text, "account = {}", quote(account));
        }
        let _ = writeln!(text, "token = {}", quote(entry.token.expose()));
    }
    text
}

/// Writes `entries` to the file at `path`, readable by its owner alone: to
/// a temporary file beside it, named with the process id and made with
/// mode 0600 and `O_EXCL`, then renamed over it. A failed write removes the
/// temporary file, so no copy of a token is left behind. A directory made
/// on the way is its owner's alone (0700); one there before that others
/// can write to is warned about.
fn write_file(path: &Path, entries: &BTreeMap<String, Entry>) -> Result<(), Error> {
    let fail = |error: io::Error| {
        Error::new(format!(
            "the credentials file {} cannot be written: {error}",
            path.display()
        ))
    };
    if let Some(dir) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(dir).map_err(fail)?;
        warn_if_open(dir);
    }
    let partial = temporary(path);
    // One left by an earlier process with this id is stale.
    let _ = fs::remove_file(&partial);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let written = options.open(&partial).and_then(|mut out| {
        out.write_all(render(entries).as_bytes())?;
        out.sync_all()?;
        drop(out);
        fs::rename(&partial, path)
    });
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written.map_err(fail)
}

/// The temporary file a write to `path` goes through first, named with the
/// process id, so that two writes at once never rename each other's
/// half-written file into place.
fn temporary(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// The macOS login keychain: a generic password item with service `cases`,
/// account the server, and the token as the password.
#[cfg(target_os = "macos")]
mod keychain {
    use std::io::{self, IsTerminal as _};

    use cases_store_remote::{Server, Token};
    use security_framework::base::Error as KeychainError;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password, generic_password, set_generic_password,
    };

    use super::SERVICE;
    use crate::error::Error;

    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;
    /// `errSecInteractionNotAllowed`: the read needs the human's approval,
    /// and interaction is off.
    const NEEDS_APPROVAL: i32 = -25308;

    /// What reading an item gave, other than the token.
    #[derive(Debug)]
    pub(crate) enum Unread {
        /// There is no item.
        Missing,
        /// The read needs the human's approval, and no dialog may show.
        NeedsApproval,
        /// The keychain failed, with why.
        Failed(String),
    }

    /// The password of the item `service`, `account`. With `quiet`, the
    /// keychain shows no dialog: a read that needs one fails at once.
    pub(crate) fn get(service: &str, account: &str, quiet: bool) -> Result<Vec<u8>, Unread> {
        // The switch is the process's, and is put back when the lock goes.
        let _lock = if quiet {
            Some(
                SecKeychain::disable_user_interaction()
                    .map_err(|error| Unread::Failed(error.to_string()))?,
            )
        } else {
            None
        };
        generic_password(PasswordOptions::new_generic_password(service, account)).map_err(
            |error: KeychainError| match error.code() {
                NOT_FOUND => Unread::Missing,
                NEEDS_APPROVAL => Unread::NeedsApproval,
                _ => Unread::Failed(error.to_string()),
            },
        )
    }

    /// Sets the password of the item `service`, `account`, making it or
    /// replacing what it held.
    pub(crate) fn set(service: &str, account: &str, password: &[u8]) -> Result<(), String> {
        set_generic_password(service, account, password).map_err(|error| error.to_string())
    }

    /// Deletes the item `service`, `account`, and gives whether there was
    /// one.
    pub(crate) fn delete(service: &str, account: &str) -> Result<bool, String> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(true),
            Err(error) if error.code() == NOT_FOUND => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The token in the keychain for `server`, if there is one. Off a
    /// terminal, a read that needs the human's approval fails at once.
    pub(super) fn read(server: &Server) -> Result<Option<Token>, Error> {
        let quiet = !io::stdin().is_terminal();
        match get(SERVICE, server.as_str(), quiet) {
            Ok(bytes) => String::from_utf8(bytes)
                .ok()
                .and_then(|text| text.trim().parse().ok())
                .map(Some)
                .ok_or_else(|| {
                    Error::new(format!(
                        "the keychain's item for {server} is not a token: run cases login {server}"
                    ))
                }),
            Err(Unread::Missing) => Ok(None),
            Err(Unread::NeedsApproval) => Err(Error::new(format!(
                "the keychain needs your approval to give cases its token for {server}: run \
                 cases login status in a terminal and choose Always Allow, or give agents a \
                 token file (cases login --file)"
            ))),
            Err(Unread::Failed(why)) => Err(Error::new(format!(
                "the keychain cannot be read for {server}: {why}"
            ))),
        }
    }

    /// Keeps `token` for `server`.
    pub(super) fn write(server: &Server, token: &Token) -> Result<(), String> {
        set(SERVICE, server.as_str(), token.expose().as_bytes())
    }

    /// Forgets the token for `server`, and gives whether there was one.
    pub(super) fn remove(server: &Server) -> Result<bool, Error> {
        delete(SERVICE, server.as_str())
            .map_err(|why| Error::new(format!("the keychain cannot be written: {why}")))
    }
}

/// No keychain off macOS: [`keychain_on`] is false there, so these are
/// never called.
#[cfg(not(target_os = "macos"))]
mod keychain {
    use cases_store_remote::{Server, Token};

    use crate::error::Error;

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the same signature as the macOS keychain's"
    )]
    pub(super) fn read(_server: &Server) -> Result<Option<Token>, Error> {
        Ok(None)
    }

    pub(super) fn write(_server: &Server, _token: &Token) -> Result<(), String> {
        Err("there is no keychain on this system".to_owned())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the same signature as the macOS keychain's"
    )]
    pub(super) fn remove(_server: &Server) -> Result<bool, Error> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(c: char) -> Token {
        format!("csu_{}", c.to_string().repeat(36)).parse().unwrap()
    }

    fn entry(account: &str, c: char) -> Entry {
        Entry {
            account: Some(account.into()),
            token: token(c),
        }
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_file_keeps_one_login_a_server_and_only_its_owner_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases").join("credentials.toml");
        let mut entries = BTreeMap::new();
        entries.insert("https://cases.example".to_owned(), entry("ac_1", 'a'));
        entries.insert("http://127.0.0.1:8787".to_owned(), entry("ac_\"2\"", 'b'));
        write_file(&path, &entries).unwrap();
        assert_eq!(read_file(&path).unwrap(), entries);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(
            text.contains(&format!(
                "[\"https://cases.example\"]\naccount = \"ac_1\"\ntoken = \"{}\"\n",
                token('a').expose()
            )),
            "{text}"
        );
        #[cfg(unix)]
        {
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
        // A write replaces the file whole, and leaves no temporary file.
        entries.remove("https://cases.example");
        write_file(&path, &entries).unwrap();
        assert_eq!(read_file(&path).unwrap(), entries);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["credentials.toml"]);
        // None at all when the file is not there.
        assert!(read_file(&dir.path().join("none.toml")).unwrap().is_empty());
    }

    #[test]
    fn a_login_is_kept_read_replaced_and_forgotten_per_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases").join("credentials.toml");
        let a: Server = "https://a.example".parse().unwrap();
        let b: Server = "http://127.0.0.1:8787".parse().unwrap();
        assert!(read_at(&path, false, &a).unwrap().is_none());
        let place = write_at(&path, false, &a, "ac_1", &token('a')).unwrap();
        assert_eq!(place, Place::File(path.clone()));
        write_at(&path, false, &b, "ac_2", &token('b')).unwrap();
        let login = read_at(&path, false, &a).unwrap().unwrap();
        assert_eq!(
            (login.token, login.account.as_deref(), login.place),
            (token('a'), Some("ac_1"), Place::File(path.clone()))
        );
        // A second login replaces the first, and leaves the other server's.
        write_at(&path, false, &a, "ac_3", &token('c')).unwrap();
        let login = read_at(&path, false, &a).unwrap().unwrap();
        assert_eq!(
            (login.token, login.account.as_deref()),
            (token('c'), Some("ac_3"))
        );
        assert_eq!(
            read_at(&path, false, &b).unwrap().unwrap().token,
            token('b')
        );
        // Forgetting one leaves the other.
        assert_eq!(
            remove_at(&path, false, &a).unwrap(),
            [Place::File(path.clone())]
        );
        assert!(remove_at(&path, false, &a).unwrap().is_empty());
        assert!(read_at(&path, false, &a).unwrap().is_none());
        assert_eq!(
            read_at(&path, false, &b).unwrap().unwrap().token,
            token('b')
        );
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn a_write_goes_through_a_temporary_file_named_by_the_process() {
        let path = Path::new("/x/credentials.toml");
        assert_eq!(
            temporary(path),
            PathBuf::from(format!("/x/credentials.toml.{}.tmp", std::process::id()))
        );
        // A stale one from an earlier process with this id is replaced, and
        // one that cannot be renamed into place is removed.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        fs::write(temporary(&path), "stale").unwrap();
        let entries = BTreeMap::from([("https://a.example".to_owned(), entry("ac_1", 'a'))]);
        write_file(&path, &entries).unwrap();
        assert!(!temporary(&path).exists());
        let blocked = dir.path().join("dir");
        fs::create_dir_all(blocked.join("in")).unwrap();
        assert!(write_file(&blocked, &entries).is_err());
        assert!(!temporary(&blocked).exists());
    }

    #[test]
    fn a_file_others_could_read_is_made_private_by_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        fs::write(&path, HEADER).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            let entries = BTreeMap::from([("https://a.example".to_owned(), entry("ac_1", 'a'))]);
            write_file(&path, &entries).unwrap();
            assert_eq!(mode(&path), 0o600);
        }
    }

    #[test]
    fn a_file_that_is_not_one_is_an_error_that_holds_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let secret = token('s');
        let secret = secret.expose();
        for (text, says) in [
            (
                format!("[\"https://a\"]\ntoken = \"{secret}"),
                "is not TOML at line 2",
            ),
            (
                format!("\"https://a\" = \"{secret}\"\n"),
                "has no token for \"https://a\"",
            ),
            (
                "[\"https://a\"]\naccount = \"ac_1\"\n".to_owned(),
                "has no token",
            ),
            (
                format!("[\"https://a\"]\ntoken = \"{}\"\n", &secret[..20]),
                "holds a token for \"https://a\" that is not one",
            ),
            (
                format!("[\"https://a\"]\ntoken = \"{secret}\"\naccount = 1\n"),
                "holds an account for \"https://a\" that is not text",
            ),
        ] {
            fs::write(&path, &text).unwrap();
            let error = read_file(&path).unwrap_err().to_string();
            assert!(error.contains(says), "{text}: {error}");
            assert!(error.contains("run cases login again"), "{error}");
            assert!(!error.contains(&secret[4..20]), "{error}");
        }
        // A file that cannot be read at all: here, a directory.
        let error = read_file(dir.path()).unwrap_err().to_string();
        assert!(error.contains("cannot be read"), "{error}");
    }

    #[test]
    fn an_entry_without_an_account_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let secret = token('t');
        fs::write(
            &path,
            format!("[\"https://a.example\"]\ntoken = \"{}\"\n", secret.expose()),
        )
        .unwrap();
        let entries = read_file(&path).unwrap();
        assert_eq!(
            entries["https://a.example"],
            Entry {
                account: None,
                token: secret
            }
        );
    }

    /// The real login keychain, which only Ryan runs, by hand:
    /// `cargo test -p cases -- --ignored keychain`. It stores, reads and
    /// deletes a token under a throwaway service, `cases-test-<pid>`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the real login keychain: run by hand with --ignored keychain"]
    fn keychain_by_hand_stores_reads_and_deletes_a_token() {
        let service = format!("cases-test-{}", std::process::id());
        let account = "https://cases.example";
        let secret = token('k');
        assert!(matches!(
            keychain::get(&service, account, true),
            Err(keychain::Unread::Missing)
        ));
        keychain::set(&service, account, secret.expose().as_bytes()).unwrap();
        let read = keychain::get(&service, account, false).unwrap();
        assert_eq!(read, secret.expose().as_bytes());
        // Set again replaces it.
        let other = token('m');
        keychain::set(&service, account, other.expose().as_bytes()).unwrap();
        let read = keychain::get(&service, account, true).unwrap();
        assert_eq!(read, other.expose().as_bytes());
        assert!(keychain::delete(&service, account).unwrap());
        assert!(!keychain::delete(&service, account).unwrap());
        assert!(matches!(
            keychain::get(&service, account, true),
            Err(keychain::Unread::Missing)
        ));
    }
}
