//! Finding a program to run, such as the browser opener or the service
//! manager.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// The program `name`, from the first directory on `$PATH` that has it.
///
/// The search is done here, not left to the system, because with `$PATH`
/// unset the system searches `/usr/bin` and `/bin`. That is where the real
/// `open`, `launchctl` and `systemctl` are, and a test that clears the
/// environment must never reach them.
///
/// # Errors
///
/// When `$PATH` is unset, or no directory on it has `name`.
pub(crate) fn find(name: &str) -> Result<PathBuf, Error> {
    let path = env::var_os("PATH")
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::new(format!("cannot find {name}: $PATH is not set")))?;
    env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
        .ok_or_else(|| Error::new(format!("cannot find {name} on $PATH")))
}

/// Whether `path` is a file this user may run.
#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Whether `path` is a file.
#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn only_a_file_that_may_be_run_is_a_program() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("script");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        assert!(!executable(&script));
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(executable(&script));
        assert!(!executable(dir.path()));
        assert!(!executable(&dir.path().join("none")));
    }
}
