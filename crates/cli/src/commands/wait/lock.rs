//! The locks a pickup wait holds on what it picks up (AG-05). A pickup wait
//! writes the pickup itself, after which the case is no longer the agent's
//! turn, so a second pickup wait on the same worker, label or id would wait
//! for ever on an answer the first took. Each pickup wait on a local store
//! takes a lock for each worker, label or id it names (wait says which), and
//! one that finds any of them held refuses to start.
//!
//! The locks are the operating system's advisory file locks, on files in a
//! directory beside the store: `<store file>-waits/`, one file to a worker,
//! label or id. The system lets go of them when the process ends, however
//! it ends, so a wait started after the last one exited always starts.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::process;

use crate::error::Error;
use crate::instance::fnv1a;
use crate::output;

/// The locks one pickup wait holds, until it is dropped.
#[derive(Debug)]
pub(crate) struct Held {
    _files: Vec<File>,
}

/// What a lock is on: a label, a worker or an id, and its text.
pub(crate) type Scope = (&'static str, String);

/// What a lock is on, by its name: `label`, `worker` or `id`.
pub(crate) fn what(name: &str) -> Option<&'static str> {
    ["label", "worker", "id"].into_iter().find(|w| *w == name)
}

/// Why the locks were not taken.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// Another process holds one of them, as the error says.
    Held(Error),
    /// The directory or a file of the locks could not be made or locked.
    Failed(Error),
}

impl From<Refusal> for Error {
    fn from(refusal: Refusal) -> Error {
        match refusal {
            Refusal::Held(error) | Refusal::Failed(error) => error,
        }
    }
}

/// Takes the lock on each of `scopes` for the store at `store`.
///
/// # Errors
///
/// When another process holds one of them, naming that one and, when its
/// file says so, the process and when it started; or when the directory or
/// a file cannot be made.
pub(crate) fn take(store: &Path, scopes: &[Scope]) -> Result<Held, Refusal> {
    let dir = dir(store);
    fs::create_dir_all(&dir)
        .map_err(|error| Refusal::Failed(Error::new(format!("{}: {error}", dir.display()))))?;
    let started = output::now().map_err(Refusal::Failed)?;
    // Once each, as a second open of a file this process has locked is
    // refused like another's, and in one order, so that two waits on the
    // same scopes cannot each take one and refuse each other.
    let mut scopes = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    let mut files = Vec::new();
    for (what, text) in &scopes {
        let path = dir.join(name(what, text));
        let failed = |error: std::io::Error| {
            Refusal::Failed(Error::new(format!("{}: {error}", path.display())))
        };
        // Not truncated: the file says who holds the lock until it is taken.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(failed)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let holder = holder(&mut file);
                return Err(Refusal::Held(Error::new(format!(
                    "another wait is already picking up {what} {text:?}{holder}; read its \
                     output instead of starting a second"
                ))));
            }
            Err(TryLockError::Error(error)) => return Err(failed(error)),
        }
        file.set_len(0)
            .and_then(|()| file.seek(SeekFrom::Start(0)))
            .and_then(|_| writeln!(file, "{} {started}", process::id()))
            .map_err(failed)?;
        files.push(file);
    }
    Ok(Held { _files: files })
}

/// The directory of the locks for the store at `store`: beside it, named
/// after it, as SQLite names its `-wal` and `-shm` files.
fn dir(store: &Path) -> PathBuf {
    let mut name = store.file_name().unwrap_or_default().to_owned();
    name.push("-waits");
    store.with_file_name(name)
}

/// The file of the lock on `what` `text`: a slug of the text for people,
/// and a hash of it, since labels and workers are free text that a slug
/// leaves out of, or two can share.
fn name(what: &str, text: &str) -> String {
    let slug = output::slug(text);
    format!("{what}-{slug}-{:016x}", fnv1a(text.as_bytes()))
}

/// Who holds the lock on `file`, as its holder wrote it: ` (pid <pid>, since
/// <time>)`, or nothing when the file does not say, such as just after the
/// lock was taken.
fn holder(file: &mut File) -> String {
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return String::new();
    }
    match text.trim().split_once(' ') {
        Some((pid, since)) if pid.parse::<u32>().is_ok() && !since.is_empty() => {
            format!(" (pid {pid}, since {since})")
        }
        _ => String::new(),
    }
}

/// [`take`], for a test that has just let go of the locks in this process:
/// a lock is refused for a moment longer while a child that another test's
/// thread is starting has the lock's file open, from its fork until its
/// exec, so a refusal is tried again for up to 2 s. Only the tests start
/// children in a process that holds locks: `cases wait` and `cases
/// dial-stdio` start none.
#[cfg(test)]
pub(crate) fn take_freed(store: &Path, scopes: &[Scope]) -> Result<Held, Refusal> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match take(store, scopes) {
            Err(Refusal::Held(_)) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            taken => return taken,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_is_beside_the_store() {
        assert_eq!(
            dir(Path::new("/data/cases.db")),
            Path::new("/data/cases.db-waits")
        );
    }

    #[test]
    fn names_tell_apart_what_a_slug_does_not() {
        assert_ne!(name("label", "Deploy"), name("label", "deploy"));
        assert_ne!(name("label", "a/b"), name("label", "a b"));
        assert_ne!(name("label", "x"), name("worker", "x"));
        assert!(name("label", "../../etc").starts_with("label-etc-"));
    }

    #[test]
    fn what_a_lock_is_on_is_one_of_three() {
        for name in ["label", "worker", "id"] {
            assert_eq!(what(name), Some(name));
        }
        for name in ["", "Label", "../x", "kind"] {
            assert_eq!(what(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_lock_that_cannot_be_made_is_a_failure_not_a_holder() {
        let dir = tempfile::tempdir().unwrap();
        // The directory of the locks is where a file is.
        let store = dir.path().join("cases.db");
        fs::write(dir.path().join("cases.db-waits"), "").unwrap();
        assert!(matches!(
            take(&store, &[("label", "deploy".into())]),
            Err(Refusal::Failed(_))
        ));
    }

    #[test]
    fn a_scope_named_twice_is_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("cases.db");
        let deploy = ("label", "deploy".to_owned());
        drop(take(&store, &[deploy.clone(), deploy]).unwrap());
    }

    #[test]
    fn a_second_take_is_refused_naming_the_holder() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("cases.db");
        let held = take(&store, &[("label", "deploy".into())]).unwrap();
        let Err(Refusal::Held(error)) = take(
            &store,
            &[("worker", "w".into()), ("label", "deploy".into())],
        ) else {
            panic!("the second take was not refused as held");
        };
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "another wait is already picking up label \"deploy\" (pid {}, since ",
                process::id()
            )),
            "{message}"
        );
        // The worker's lock, taken before the refusal, is let go.
        drop(take_freed(&store, &[("worker", "w".into())]).unwrap());
        drop(held);
        drop(take_freed(&store, &[("label", "deploy".into())]).unwrap());
    }
}
