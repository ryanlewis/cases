//! serve's own binary, watched so that serve stops when an upgrade or a
//! reinstall replaces it, and launchd or systemd starts the new one.
//!
//! The file is known by its device, inode, size and modification time. An
//! install that renames a new file over the old gives a new inode; one that
//! writes over it in place gives a new size or time; a link moved to a new
//! version points at another file. A file that is missing, as it is for a
//! moment in some installs, is waited out, and a change counts only once two
//! looks in a row agree on it, so that serve does not stop halfway through a
//! copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::time::MissedTickBehavior;

/// How often serve looks at its binary: a stat, so cheap.
const POLL: Duration = Duration::from_secs(2);

/// What tells one file at a path from another.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

/// What tells one file at a path from another.
#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

/// The file at `path`, following links, or `None` when it is not there.
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(Stamp {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.size(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
        })
    }
    #[cfg(not(unix))]
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// The binary serve was started from, as it was then.
#[derive(Debug)]
pub(crate) struct Binary {
    path: PathBuf,
    started: Stamp,
    /// What the last look found.
    last: Option<Stamp>,
}

impl Binary {
    /// The file at `path` as it is now, or `None` when it is not there,
    /// and there is nothing to watch.
    pub(crate) fn new(path: PathBuf) -> Option<Binary> {
        let started = stamp(&path)?;
        Some(Binary {
            path,
            started,
            last: Some(started),
        })
    }

    /// Looks again, and says whether another file has taken the binary's
    /// place and was there at the last look too.
    fn replaced(&mut self) -> bool {
        let now = stamp(&self.path);
        let replaced = now.is_some() && now != Some(self.started) && now == self.last;
        self.last = now;
        replaced
    }

    /// Resolves once the binary is replaced; never, when it is not.
    pub(crate) async fn watch(mut self) -> PathBuf {
        let mut ticks = tokio::time::interval(POLL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            // A stat can wait on a slow disk, and must not hold the thread
            // that serves the inbox.
            let Ok((binary, replaced)) = tokio::task::spawn_blocking(move || {
                let replaced = self.replaced();
                (self, replaced)
            })
            .await
            else {
                return std::future::pending().await;
            };
            self = binary;
            if replaced {
                return self.path;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A binary in a directory of its own, and its path.
    fn binary(text: &str) -> (tempfile::TempDir, PathBuf, Binary) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cases");
        fs::write(&path, text).unwrap();
        let binary = Binary::new(path.clone()).unwrap();
        (dir, path, binary)
    }

    /// Puts a new file with `text` at `path` as an installer does: written
    /// beside it, then renamed over it.
    fn install(path: &Path, text: &str) {
        let new = path.with_extension("new");
        fs::write(&new, text).unwrap();
        fs::rename(&new, path).unwrap();
    }

    #[test]
    fn a_binary_left_alone_is_not_replaced() {
        let (_dir, _path, mut binary) = binary("v1");
        for _ in 0..3 {
            assert!(!binary.replaced());
        }
    }

    #[test]
    fn a_file_renamed_over_the_binary_replaces_it_once_two_looks_agree() {
        // The same bytes, so only the inode differs.
        let (_dir, path, mut binary) = binary("v1");
        install(&path, "v1");
        assert!(!binary.replaced(), "a first look only notes the change");
        assert!(binary.replaced());
    }

    #[test]
    fn a_file_still_being_written_is_waited_for() {
        let (_dir, path, mut binary) = binary("v1");
        fs::write(&path, "v2 part").unwrap();
        assert!(!binary.replaced());
        fs::write(&path, "v2 part and the rest").unwrap();
        assert!(!binary.replaced(), "the file changed between looks");
        assert!(binary.replaced());
    }

    #[test]
    fn a_binary_missing_for_a_moment_is_waited_out() {
        let (_dir, path, mut binary) = binary("v1");
        fs::remove_file(&path).unwrap();
        assert!(!binary.replaced());
        assert!(!binary.replaced(), "a binary that is gone is not replaced");
        // Another length: the new file can reuse the freed inode, and
        // within one tick of the kernel's clock, its mtime too.
        install(&path, "v2, longer");
        assert!(!binary.replaced());
        assert!(binary.replaced());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_moved_to_another_version_replaces_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let (v1, v2, link) = (
            dir.path().join("v1"),
            dir.path().join("v2"),
            dir.path().join("cases"),
        );
        fs::write(&v1, "v1").unwrap();
        fs::write(&v2, "v2").unwrap();
        std::os::unix::fs::symlink(&v1, &link).unwrap();
        let mut binary = Binary::new(link.clone()).unwrap();
        assert!(!binary.replaced());
        let moved = dir.path().join("cases.new");
        std::os::unix::fs::symlink(&v2, &moved).unwrap();
        fs::rename(&moved, &link).unwrap();
        assert!(!binary.replaced());
        assert!(binary.replaced());
    }

    #[test]
    fn a_binary_that_is_not_there_is_not_watched() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Binary::new(dir.path().join("cases")).is_none());
    }
}
