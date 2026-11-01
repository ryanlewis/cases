//! The terminal the status screen is on: its height, and reading `q` from
//! it as it is typed.

use std::io;
#[cfg(unix)]
use std::time::Duration;

/// Puts the terminal on stdin back as it was when dropped.
#[derive(Debug)]
pub(crate) struct Keys {
    #[cfg(unix)]
    before: Option<rustix::termios::Termios>,
}

/// Whether this process is in the foreground of the terminal `fd`. A
/// background job that reads the terminal, or changes its modes, is stopped
/// by `SIGTTIN` or `SIGTTOU`, and the server with it.
#[cfg(unix)]
fn foreground(fd: impl std::os::fd::AsFd) -> bool {
    rustix::termios::tcgetpgrp(fd).is_ok_and(|group| group == rustix::process::getpgrp())
}

/// Whether serve may read keys from stdin: it is a terminal, and this
/// process is in the terminal's foreground, which it is not as a background
/// job (`cases serve &`).
#[cfg(unix)]
pub(crate) fn can_read_keys() -> bool {
    use std::io::IsTerminal;
    let stdin = io::stdin();
    stdin.is_terminal() && foreground(&stdin)
}

#[cfg(not(unix))]
pub(crate) fn can_read_keys() -> bool {
    false
}

/// Turns off line buffering and echo on the terminal on stdin, so serve
/// sees each key as it is pressed. Signals stay on: Ctrl-C still stops
/// serve. Without it, `q` still works when Enter follows it.
#[cfg(unix)]
pub(crate) fn keys_raw() -> Keys {
    use rustix::termios::{LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr};
    let stdin = io::stdin();
    let Ok(before) = tcgetattr(&stdin) else {
        return Keys { before: None };
    };
    let mut raw = before.clone();
    raw.local_modes
        .remove(LocalModes::ICANON | LocalModes::ECHO);
    raw.special_codes[SpecialCodeIndex::VMIN] = 1;
    raw.special_codes[SpecialCodeIndex::VTIME] = 0;
    let set = tcsetattr(&stdin, OptionalActions::Now, &raw).is_ok();
    Keys {
        before: set.then_some(before),
    }
}

#[cfg(not(unix))]
pub(crate) fn keys_raw() -> Keys {
    Keys {}
}

impl Keys {
    /// Puts the terminal back as it was, when serve is in its foreground. In
    /// the background the shell has put its own modes back, and changing
    /// them would stop serve with `SIGTTOU`.
    pub(crate) fn restore(&self) {
        #[cfg(unix)]
        if let Some(before) = &self.before
            && foreground(io::stdin())
        {
            let _ = rustix::termios::tcsetattr(
                io::stdin(),
                rustix::termios::OptionalActions::Now,
                before,
            );
        }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.restore();
    }
}

/// How long the key reader waits while serve is in the background before it
/// looks again.
#[cfg(unix)]
const BACKGROUND: Duration = Duration::from_millis(250);

/// Stdin for the keys, read only while serve is in the terminal's
/// foreground. After Ctrl-Z and `bg`, a read would stop serve with
/// `SIGTTIN`, and `kill %1` could not end it, so it waits for input in
/// poll, which a background job may do, and reads once serve is in the
/// foreground again. It reads the file itself, so that no input waits in a
/// buffer that poll cannot see.
#[derive(Debug)]
pub(crate) struct ForegroundStdin;

#[cfg(unix)]
impl io::Read for ForegroundStdin {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, poll};
        let stdin = io::stdin();
        loop {
            let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
            let polled = poll(&mut fds, None).map(|_| fds[0].revents());
            match next(polled, || foreground(&stdin)) {
                Next::Read => return Ok(rustix::io::read(&stdin, &mut *buf)?),
                Next::End => return Ok(0),
                Next::Wait => std::thread::sleep(BACKGROUND),
                Next::Poll => {}
                Next::Fail(error) => return Err(error.into()),
            }
        }
    }
}

/// What the key reader does after a poll of stdin.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Read the input.
    Read,
    /// Input has ended.
    End,
    /// Wait while serve is in the background, and poll again.
    Wait,
    /// Poll again.
    Poll,
    /// Fail with the poll's error.
    Fail(rustix::io::Errno),
}

/// What the key reader does after a poll of stdin that gave `polled`, the
/// events or the error, with `foreground` saying whether serve is in the
/// terminal's foreground.
#[cfg(unix)]
fn next(
    polled: Result<rustix::event::PollFlags, rustix::io::Errno>,
    foreground: impl FnOnce() -> bool,
) -> Next {
    use rustix::event::PollFlags;
    match polled {
        // The terminal hung up: input has ended, even when the poll says
        // there is input too, as Linux does once a pty's other side has
        // closed. There, serve is in no foreground any more, and would wait
        // as if in the background for good.
        Ok(events) if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) => {
            Next::End
        }
        Ok(events) if !events.contains(PollFlags::IN) => Next::Poll,
        Ok(_) if foreground() => Next::Read,
        // The input is the shell's while serve is in the background.
        Ok(_) => Next::Wait,
        Err(rustix::io::Errno::INTR) => Next::Poll,
        Err(error) => Next::Fail(error),
    }
}

#[cfg(not(unix))]
impl io::Read for ForegroundStdin {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use std::io::Read as _;
        io::stdin().read(buf)
    }
}

/// The terminal on stdout, written to as a file, from any thread: every
/// command holds stdout's own lock on the main thread while it runs, and the
/// screen is drawn, and put back, from other threads. Nothing waits in a
/// buffer, so a write is out when it returns.
#[derive(Debug)]
pub(crate) struct Out;

#[cfg(unix)]
impl io::Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(rustix::io::write(io::stdout(), buf)?)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Off unix, stdout itself, which only the main thread may write to.
#[cfg(not(unix))]
impl io::Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use std::io::Write as _;
        io::stdout().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        use std::io::Write as _;
        io::stdout().flush()
    }
}

/// The height of the terminal on stdout, or `None` when it cannot tell.
#[cfg(unix)]
pub(crate) fn rows() -> Option<usize> {
    let size = rustix::termios::tcgetwinsize(io::stdout()).ok()?;
    (size.ws_row > 0).then_some(usize::from(size.ws_row))
}

#[cfg(not(unix))]
pub(crate) fn rows() -> Option<usize> {
    None
}

/// Reads `input` until a `q` or `Q`, and gives whether one came. Input that
/// ends first gives `false`.
pub(crate) fn wait_for_q(mut input: impl io::Read) -> bool {
    let mut buf = [0; 16];
    loop {
        match input.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) if buf[..n].iter().any(|b| matches!(b, b'q' | b'Q')) => return true,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One part a read, then the end.
    struct Parts(Vec<&'static str>);

    impl io::Read for Parts {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let part = self.0.remove(0).as_bytes();
            buf[..part.len()].copy_from_slice(part);
            Ok(part.len())
        }
    }

    #[test]
    fn q_stops_serve_and_other_keys_do_not() {
        for (reads, quits, left) in [
            (vec!["q", "x"], true, 1),
            (vec!["abQc"], true, 0),
            (vec!["a", "b", "q", "x"], true, 1),
            (vec!["a", "x"], false, 0),
            (vec![], false, 0),
        ] {
            let mut input = Parts(reads.clone());
            assert_eq!(wait_for_q(&mut input), quits, "{reads:?}");
            assert_eq!(input.0.len(), left, "{reads:?}");
        }
    }

    /// Fails each read with the next error, then reads `q`.
    struct Failing(Vec<io::ErrorKind>);

    impl io::Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                buf[0] = b'q';
                return Ok(1);
            }
            Err(self.0.remove(0).into())
        }
    }

    #[test]
    fn an_interrupted_read_is_read_again_and_any_other_failure_ends_the_keys() {
        use io::ErrorKind::{BrokenPipe, Interrupted};
        assert!(wait_for_q(Failing(vec![Interrupted, Interrupted])));
        let mut input = Failing(vec![BrokenPipe, Interrupted]);
        assert!(!wait_for_q(&mut input));
        assert_eq!(input.0, [Interrupted], "it read on after a failure");
    }

    #[cfg(unix)]
    #[test]
    fn the_key_reader_reads_in_the_foreground_and_waits_in_the_background() {
        use rustix::event::PollFlags;
        use rustix::io::Errno;
        let foreground = || true;
        let background = || false;
        assert_eq!(next(Ok(PollFlags::IN), foreground), Next::Read);
        // Ctrl-Z and `bg`, then `fg`.
        assert_eq!(next(Ok(PollFlags::IN), background), Next::Wait);
        assert_eq!(next(Ok(PollFlags::IN), foreground), Next::Read);
        assert_eq!(next(Ok(PollFlags::empty()), foreground), Next::Poll);
        assert_eq!(next(Err(Errno::INTR), foreground), Next::Poll);
        assert_eq!(next(Err(Errno::IO), foreground), Next::Fail(Errno::IO));
    }

    /// A terminal that hangs up ends the keys, whatever else the poll says
    /// and wherever serve is. Linux says `IN` and `HUP` on a pty whose other
    /// side has closed, and serve is in no foreground once it has.
    #[cfg(unix)]
    #[test]
    fn a_hang_up_ends_the_keys() {
        use rustix::event::PollFlags;
        let mut wrong = Vec::new();
        for events in [PollFlags::HUP, PollFlags::ERR, PollFlags::NVAL] {
            for also in [PollFlags::empty(), PollFlags::IN] {
                for foreground in [true, false] {
                    let got = next(Ok(events | also), || foreground);
                    if got != Next::End {
                        wrong.push(format!(
                            "{:?}, foreground {foreground}: {got:?}",
                            events | also
                        ));
                    }
                }
            }
        }
        assert_eq!(wrong, Vec::<String>::new());
    }
}
