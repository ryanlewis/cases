//! serve's log: one line for each page load, write and failed request, and
//! each warning about the store (SV-02).

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// How big the log file grows before it is rotated. A line is about 100
/// bytes, so this is some ten thousand lines: weeks of use.
const CAP: u64 = 1024 * 1024;

/// How many log files are kept: the log and two rotated away, so that
/// what led up to a burst of lines is still there after it.
const FILES: usize = 3;

/// Where the log's lines go: stderr, a file of serve's own, and, on a
/// terminal, the status screen, which shows the last few.
#[derive(Debug)]
pub(crate) struct Log {
    /// Whether lines go to stderr. On a terminal the screen shows them
    /// instead, unless stderr goes elsewhere.
    stderr: bool,
    /// How many of the last lines to keep for the screen.
    keep: usize,
    last: Mutex<VecDeque<String>>,
    file: Option<Mutex<Rotating>>,
}

impl Log {
    pub(crate) fn new(stderr: bool, keep: usize) -> Log {
        Log {
            stderr,
            keep,
            last: Mutex::new(VecDeque::new()),
            file: None,
        }
    }

    /// The log, writing each line to `file` too. When stderr goes to the
    /// file, lines no longer go to stderr as well.
    pub(crate) fn with_file(mut self, file: Rotating) -> Log {
        if file.redirect.is_some() {
            self.stderr = false;
        }
        self.file = Some(Mutex::new(file));
        self
    }

    /// Logs `line`. A line stderr does not take is still kept for the
    /// screen: there is nowhere else to say it.
    pub(crate) fn line(&self, line: impl Into<String>) {
        let line = line.into();
        self.file_only(&line);
        if self.stderr {
            let _ = writeln!(io::stderr().lock(), "{line}");
        }
        if self.keep > 0 {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            last.push_back(line);
            while last.len() > self.keep {
                last.pop_front();
            }
        }
    }

    /// Writes `line` to the log file alone, if there is one.
    pub(crate) fn file_only(&self, line: &str) {
        if let Some(file) = &self.file {
            file.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .write(line);
        }
    }

    /// The last lines kept, oldest first.
    pub(crate) fn last(&self) -> Vec<String> {
        let last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        last.iter().cloned().collect()
    }
}

/// Points a file descriptor, stderr for serve, at a new log file.
pub(crate) type Redirect = Box<dyn Fn(&File) -> io::Result<()> + Send>;

/// A log file that serve rotates itself: once it reaches its cap, it is
/// renamed with `.1` added, the `.1` before it to `.2`, and so on, the last
/// is dropped, and a new file is started.
pub(crate) struct Rotating {
    path: PathBuf,
    file: File,
    cap: u64,
    files: usize,
    /// Points stderr at each new file, so that what serve writes there
    /// itself, such as the error that stops it, or a panic, is logged.
    redirect: Option<Redirect>,
}

impl fmt::Debug for Rotating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rotating")
            .field("path", &self.path)
            .field("redirect", &self.redirect.is_some())
            .finish_non_exhaustive()
    }
}

impl Rotating {
    /// Opens the log file at `path` to add to it, making its directory, and
    /// with `stderr`, points stderr at it.
    pub(crate) fn open(path: &Path, stderr: bool) -> io::Result<Rotating> {
        #[cfg(unix)]
        let redirect = stderr.then(|| -> Redirect {
            Box::new(|file| rustix::stdio::dup2_stderr(file).map_err(io::Error::from))
        });
        #[cfg(not(unix))]
        let redirect = {
            let _ = stderr;
            None
        };
        Rotating::with(path, CAP, FILES, redirect)
    }

    fn with(
        path: &Path,
        cap: u64,
        files: usize,
        redirect: Option<Redirect>,
    ) -> io::Result<Rotating> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = append(path)?;
        if let Some(redirect) = &redirect {
            redirect(&file)?;
        }
        Ok(Rotating {
            path: path.to_owned(),
            file,
            cap,
            files,
            redirect,
        })
    }

    /// Adds `line`, rotating the file first when it is full. The size is
    /// the file's, so what went to stderr counts too.
    fn write(&mut self, line: &str) {
        if self
            .file
            .metadata()
            .is_ok_and(|meta| meta.len() >= self.cap)
        {
            // A rotation that fails leaves the file as it is, to try again
            // with the next line.
            let _ = self.rotate();
        }
        // One write, so that the line is not split by what goes to stderr
        // meanwhile: the file is opened to append, and each write lands
        // whole at its end.
        let _ = self.file.write_all(format!("{line}\n").as_bytes());
    }

    /// Renames the files one along and starts a new one. Until stderr is
    /// pointed at the new file, what goes there lands in the file renamed
    /// to `.1`: the old file stays open until then, so that no write finds
    /// stderr closed or is lost.
    fn rotate(&mut self) -> io::Result<()> {
        for n in (1..self.files.saturating_sub(1)).rev() {
            match fs::rename(self.numbered(n), self.numbered(n + 1)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        fs::rename(&self.path, self.numbered(1))?;
        let file = append(&self.path)?;
        let redirected = self
            .redirect
            .as_ref()
            .map_or(Ok(()), |redirect| redirect(&file));
        // The lines go to the new file even when stderr could not follow:
        // kept on the old one, the next rotation would rename the empty new
        // file over it.
        self.file = file;
        redirected
    }

    /// The log's path with `.n` added.
    fn numbered(&self, n: usize) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push(format!(".{n}"));
        PathBuf::from(path)
    }
}

/// The file at `path`, made if need be, opened to add to.
fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_keeps_the_last_lines() {
        let log = Log::new(false, 5);
        for n in 1..=7 {
            log.line(n.to_string());
        }
        assert_eq!(log.last(), ["3", "4", "5", "6", "7"]);
        let off_the_screen = Log::new(false, 0);
        off_the_screen.line("GET / 200");
        assert!(off_the_screen.last().is_empty());
    }
}

#[cfg(test)]
#[cfg(unix)]
mod rotating {
    use std::os::fd::OwnedFd;
    use std::sync::Arc;

    use super::*;

    /// A stand-in for stderr: a descriptor the test owns, pointed at a
    /// file of its own until a log file takes it.
    fn stand_in(dir: &Path) -> Arc<Mutex<OwnedFd>> {
        let fd = OwnedFd::from(append(&dir.join("elsewhere")).unwrap());
        Arc::new(Mutex::new(fd))
    }

    /// Writes `text` to `fd`, as a panic writes to stderr.
    fn say(fd: &Mutex<OwnedFd>, text: &str) {
        rustix::io::write(&*fd.lock().unwrap(), text.as_bytes()).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn the_file_is_rotated_at_its_cap_and_only_so_many_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/serve.log");
        let mut log = Rotating::with(&path, 20, 3, None).unwrap();
        for n in 1..=12 {
            log.write(&format!("line {n:02}"));
        }
        // Two lines of 8 bytes fill a file past 20 bytes at the third.
        assert_eq!(read(&path), "line 10\nline 11\nline 12\n");
        assert_eq!(
            read(&dir.path().join("logs/serve.log.1")),
            "line 07\nline 08\nline 09\n"
        );
        assert_eq!(
            read(&dir.path().join("logs/serve.log.2")),
            "line 04\nline 05\nline 06\n"
        );
        assert!(!dir.path().join("logs/serve.log.3").exists());
    }

    #[test]
    fn a_log_file_there_already_is_added_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.log");
        fs::write(&path, "before\n").unwrap();
        Rotating::with(&path, CAP, FILES, None)
            .unwrap()
            .write("after");
        assert_eq!(read(&path), "before\nafter\n");
    }

    /// stderr follows the log file, and what goes to it while the file is
    /// rotated lands in the file rotated away, never nowhere.
    #[test]
    fn stderr_follows_the_file_and_loses_nothing_while_it_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.log");
        let stderr = stand_in(dir.path());
        let target = Arc::clone(&stderr);
        let redirect: Redirect = Box::new(move |file| {
            // What a panic writes between the rename and the redirect.
            say(&target, "mid-rotation\n");
            rustix::io::dup2(file, &mut target.lock().unwrap()).map_err(io::Error::from)
        });
        let mut log = Rotating::with(&path, 10, 3, Some(redirect)).unwrap();
        say(&stderr, "panic 1\n");
        log.write("line 1");
        // The file is full: this line rotates it.
        log.write("line 2");
        say(&stderr, "panic 2\n");
        assert_eq!(read(&dir.path().join("elsewhere")), "mid-rotation\n");
        assert_eq!(
            read(&dir.path().join("serve.log.1")),
            "panic 1\nline 1\nmid-rotation\n"
        );
        assert_eq!(read(&path), "line 2\npanic 2\n");
    }
}
