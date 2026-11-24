//! The pipe a forwarded store is reached by: HTTP/1.1 over a child
//! process's stdin and stdout, such as `ssh host cases dial-stdio`, or a
//! command of the user's that ends in it on the far side.
//!
//! The HTTP client opens one child for each connection it makes, and keeps
//! it while it is idle, as it would keep a socket: a later request reuses
//! it, and a connection that fails is dropped and the next request starts a
//! new child. A child's stdout is read, and its stdin written, on threads
//! of their own, so a read or a write can time out as a socket's does: a
//! request larger than the pipe holds, to a child that does not read it,
//! would otherwise wait for as long as the child lasts. Its stderr is
//! kept, the last part of it, to say why a child ended with no answer.
//!
//! A [`Hold`](crate::Hold) is a child of its own, outside the HTTP client,
//! sent one request and kept open after its answer, for what the far side
//! holds for as long as the connection lasts.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

/// The most of a child's stderr that is kept, from its end.
const STDERR_KEPT: usize = 4096;

/// How long a child that closed its stdout is given to exit, so that its
/// status and stderr can say why.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// How long the end of a child's stderr is waited for once it has exited,
/// so that what it wrote just before is kept. Short, since something the
/// child started can hold its stderr open for much longer.
const STDERR_GRACE: Duration = Duration::from_millis(200);

/// How long a child is given to exit after its stdin is closed, when its
/// connection is dropped with no write stuck, before it is killed.
const CLOSE_GRACE: Duration = Duration::from_millis(200);

/// The most an answer's head may hold, read by [`Pipe::exchange`].
const ANSWER_HEAD_LIMIT: usize = 64 * 1024;

/// How a forwarded store is dialled: the program to run, its arguments and
/// any environment to add, and the name it goes by in an error, such as
/// `ssh://devbox`. The program speaks the store routes over its stdin and
/// stdout, as `cases dial-stdio` does.
///
/// ```
/// use cases_store_remote::Dial;
///
/// let dial = Dial::new("ssh", "ssh://devbox").args(["devbox", "cases", "dial-stdio"]);
/// assert_eq!(dial.to_string(), "ssh://devbox");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dial {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    name: String,
}

impl Dial {
    /// A dial that runs `program`, named `name` in errors.
    #[must_use]
    pub fn new(program: impl Into<OsString>, name: impl Into<String>) -> Dial {
        Dial {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
            name: name.into(),
        }
    }

    /// The dial, with `args` after those it has.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Dial
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// The dial, with `key` set to `value` in the child's environment.
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Dial {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// The program it runs.
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// The arguments it runs the program with.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.args
    }

    /// Whether it is over ssh, named `ssh://...`, whose exit code 255 is
    /// ssh's own failure.
    fn over_ssh(&self) -> bool {
        self.name.starts_with("ssh://")
    }

    /// Starts the child, with its stdin, stdout and stderr piped, in a
    /// process group of its own, so that a Ctrl-C at the terminal stops
    /// this process alone: its children end as their stdin does, once a
    /// request on its way has its answer, and a pickup wait lets go of its
    /// locks over its hold's connection rather than lose it to the signal.
    fn spawn(&self) -> io::Result<Child> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        for (key, value) in &self.envs {
            command.env(key, value);
        }
        command.spawn()
    }
}

impl fmt::Display for Dial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The connector that makes each connection a new child.
#[derive(Debug)]
pub(crate) struct PipeConnector {
    dial: Arc<Dial>,
}

impl PipeConnector {
    pub(crate) fn new(dial: Dial) -> PipeConnector {
        PipeConnector {
            dial: Arc::new(dial),
        }
    }
}

impl Connector<()> for PipeConnector {
    type Out = Pipe;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<()>,
    ) -> Result<Option<Pipe>, ureq::Error> {
        let config = details.config;
        Pipe::start(
            &self.dial,
            config.input_buffer_size(),
            config.output_buffer_size(),
        )
        .map(Some)
    }
}

/// A resolver that looks nothing up: a pipe has no address.
#[derive(Debug)]
pub(crate) struct NoAddress;

impl Resolver for NoAddress {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut addrs = self.empty();
        addrs.truncate(0);
        addrs.push(SocketAddr::from(([127, 0, 0, 1], 0)));
        Ok(addrs)
    }
}

/// What the stdout thread sends: bytes, or the error that ended it. Its
/// end, with the sender dropped, is the end of the child's stdout.
type Chunk = io::Result<Vec<u8>>;

/// A child's stdin, written on a thread of its own: each request is sent
/// to the thread, and its result sent back, so that it can be waited for
/// with a time limit. Dropped, it ends the thread once a write on its way
/// is done, which closes the stdin.
struct Stdin {
    requests: Sender<Vec<u8>>,
    /// In a mutex for the same reason as the pipe's `stdout`.
    written: Mutex<Receiver<io::Result<()>>>,
    /// Set once a write was not done in time. Its bytes may be in the pipe
    /// in part, so the child cannot take another request: each later write
    /// fails at once.
    stuck: bool,
}

impl Stdin {
    fn new(mut stdin: ChildStdin) -> Stdin {
        let (requests, to_write) = mpsc::channel::<Vec<u8>>();
        let (done, written) = mpsc::channel();
        thread::spawn(move || {
            // A write that fails ends the thread, and the connection: the
            // child closed its stdin, or ended. A write the child does not
            // read ends when it is killed, as its pipe is dropped.
            for bytes in to_write {
                let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
                let failed = result.is_err();
                if done.send(result).is_err() || failed {
                    return;
                }
            }
        });
        Stdin {
            requests,
            written: Mutex::new(written),
            stuck: false,
        }
    }
}

/// One connection: a child, its stdin, what it has written to stdout and
/// not been read yet, and its stderr.
pub(crate) struct Pipe {
    dial: Arc<Dial>,
    child: Child,
    /// Boxed, to keep the pipe as small as a stdin alone left it.
    stdin: Option<Box<Stdin>>,
    /// In a mutex only so that the pipe is `Sync`, as a transport must be:
    /// it is reached through `&mut self` alone.
    stdout: Mutex<Receiver<Chunk>>,
    /// Bytes from stdout that did not fit the input buffer.
    pending: VecDeque<u8>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Disconnected once the child's stderr has ended and all of it is
    /// kept. In a mutex for the same reason as `stdout`.
    stderr_ended: Mutex<Receiver<()>>,
    buffers: LazyBuffers,
    /// Set once the child's stdout ended or failed.
    ended: bool,
}

impl fmt::Debug for Pipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipe")
            .field("dial", &self.dial)
            .field("pid", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl Pipe {
    /// Starts a child for `dial`.
    pub(crate) fn start(
        dial: &Arc<Dial>,
        input: usize,
        output: usize,
    ) -> Result<Pipe, ureq::Error> {
        let mut child = dial.spawn().map_err(|error| {
            ureq::Error::Io(io::Error::new(
                error.kind(),
                format!("could not run {}: {error}", dial.program.to_string_lossy()),
            ))
        })?;
        let stdin = child.stdin.take().map(|stdin| Box::new(Stdin::new(stdin)));
        let mut stdout = child.stdout.take().ok_or_else(|| no_pipe("stdout"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| no_pipe("stderr"))?;
        let (send, stdout_chunks) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = vec![0; 16 * 1024];
            loop {
                // A read the child's stdout fails ends the connection, as
                // its end does: the request is sent again, if it may be.
                match stdout.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => {
                        if send.send(Ok(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = send.send(Err(error));
                        return;
                    }
                }
            }
        });
        let kept = Arc::new(Mutex::new(Vec::new()));
        let keeping = Arc::clone(&kept);
        let (ending, stderr_ended) = mpsc::channel::<()>();
        thread::spawn(move || {
            // Dropped when the thread ends, which tells `closed` so.
            let _ending = ending;
            let mut buf = [0; 1024];
            loop {
                let n = match stderr.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => n,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return,
                };
                let mut kept = keeping.lock().unwrap_or_else(PoisonError::into_inner);
                kept.extend_from_slice(&buf[..n]);
                let over = kept.len().saturating_sub(STDERR_KEPT);
                kept.drain(..over);
            }
        });
        Ok(Pipe {
            dial: Arc::clone(dial),
            child,
            stdin,
            stdout: Mutex::new(stdout_chunks),
            pending: VecDeque::new(),
            stderr: kept,
            stderr_ended: Mutex::new(stderr_ended),
            buffers: LazyBuffers::new(input, output),
            ended: false,
        })
    }

    /// Writes `bytes` to the child's stdin, within `within` (no limit for
    /// `None`), or fails as a timeout for `reason`. A write that fails
    /// says why the child closed its stdin, when it exits.
    fn write(
        &mut self,
        bytes: Vec<u8>,
        within: Option<Duration>,
        reason: ureq::Timeout,
    ) -> Result<(), ureq::Error> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(self.closed());
        };
        if stdin.stuck {
            return Err(ureq::Error::Timeout(reason));
        }
        let written = if stdin.requests.send(bytes).is_ok() {
            let written = stdin
                .written
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            match within {
                Some(within) => written.recv_timeout(within),
                None => written.recv().map_err(|_| RecvTimeoutError::Disconnected),
            }
        } else {
            Err(RecvTimeoutError::Disconnected)
        };
        match written {
            Ok(Ok(())) => Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                stdin.stuck = true;
                Err(ureq::Error::Timeout(reason))
            }
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                self.stdin = None;
                Err(self.closed())
            }
        }
    }

    /// The child's status, once it has exited, waiting up to `grace`.
    fn exited(&mut self, grace: Duration) -> Option<ExitStatus> {
        let until = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
                _ => return None,
            }
        }
    }

    /// Whether the child is there still, between exchanges: its stdout has
    /// not ended, nor sent anything not asked for.
    pub(crate) fn is_open(&mut self) -> bool {
        let stdout = self
            .stdout
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        !self.ended && matches!(stdout.try_recv(), Err(TryRecvError::Empty))
    }

    /// The error for a child that closed its stdout, or its stdin, with no
    /// answer: why, from its exit status and the last line of its stderr.
    pub(crate) fn closed(&mut self) -> ureq::Error {
        let status = self.exited(EXIT_GRACE);
        // A child that has exited may have written its last line to stderr
        // just before, and not all of it be kept yet: wait a little for the
        // end of its stderr.
        if status.is_some() {
            let _ = self
                .stderr_ended
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .recv_timeout(STDERR_GRACE);
        }
        let stderr = String::from_utf8_lossy(
            &self
                .stderr
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .into_owned();
        ureq::Error::Io(why_closed(
            self.dial.over_ssh(),
            status.and_then(|s| s.code()),
            &stderr,
        ))
    }
}

impl Pipe {
    /// Sends `request`, as it is, and reads one answer, sent with its
    /// length and a body of at most `limit` bytes, within `timeout`: its
    /// status and body. The connection stays open after it.
    pub(crate) fn exchange(
        &mut self,
        request: &[u8],
        timeout: Duration,
        limit: u64,
    ) -> Result<(u16, Vec<u8>), ureq::Error> {
        let deadline = Instant::now() + timeout;
        self.write(request.to_vec(), Some(timeout), ureq::Timeout::Global)?;
        let mut got = Vec::new();
        loop {
            if let Some(answer) = answer_in(&got, limit)? {
                return Ok(answer);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let stdout = self
                .stdout
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            match stdout.recv_timeout(left) {
                Ok(Ok(bytes)) => got.extend(bytes),
                Ok(Err(error)) => {
                    self.ended = true;
                    return Err(ureq::Error::Io(error));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ureq::Error::Timeout(ureq::Timeout::Global));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    return Err(self.closed());
                }
            }
        }
    }
}

/// The status and body of the answer `got` begins with, once all of it is
/// there, or `None` until it is.
fn answer_in(got: &[u8], limit: u64) -> Result<Option<(u16, Vec<u8>)>, ureq::Error> {
    let bad = |why: &str| ureq::Error::Io(io::Error::new(io::ErrorKind::InvalidData, why));
    let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else {
        if got.len() > ANSWER_HEAD_LIMIT {
            return Err(bad("the answer's head is over 64 KiB"));
        }
        return Ok(None);
    };
    let head =
        std::str::from_utf8(&got[..end]).map_err(|_| bad("the answer's head is not UTF-8"))?;
    let (status, length) = answer_head(head).map_err(bad)?;
    if length > limit {
        return Err(ureq::Error::BodyExceedsLimit(limit));
    }
    let start = end + 4;
    let length = usize::try_from(length).map_err(|_| ureq::Error::BodyExceedsLimit(limit))?;
    if got.len() < start + length {
        return Ok(None);
    }
    Ok(Some((status, got[start..start + length].to_vec())))
}

/// The status and the body's length the answer's `head` gives, read
/// strictly: each line ended by CRLF alone, the version 1.0 or 1.1, a
/// status of three digits, each header's name a token, no transfer coding,
/// and one length, of digits alone; or why it gives none.
fn answer_head(head: &str) -> Result<(u16, u64), &'static str> {
    // A bare CR or LF would end a line for some readers and not for
    // others, so it could hide a header in the one before (RFC 9112, 2.2).
    if head.split("\r\n").any(|line| line.contains(['\r', '\n'])) {
        return Err("the answer's line has a bare CR or LF");
    }
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| {
            let mut parts = line.split(' ');
            matches!(parts.next()?, "HTTP/1.0" | "HTTP/1.1").then_some(())?;
            // Three digits (RFC 9112, 4), of a final answer: an interim
            // one (1xx) would leave the real answer to be read as the next
            // one's, and none is below 100 (RFC 9110, 15).
            let status = parts.next()?;
            (status.len() == 3 && status.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
            status
                .parse::<u16>()
                .ok()
                .filter(|s| (200..=599).contains(s))
        })
        .ok_or("the answer's status line is not one")?;
    let mut length = None;
    for line in lines {
        // A name is a token (RFC 9110, 5.6.2), so not one with space
        // before it or its colon, nor a line that goes on from the last
        // (RFC 9112, 5.2).
        let Some((name, value)) = line.split_once(':').filter(|(name, _)| is_token(name)) else {
            return Err("the answer's header is not one");
        };
        // A transfer coding with a length is an error (RFC 9112, 6.3), and
        // without one the body's end is not known.
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("the answer has a transfer coding");
        }
        if !name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        // One, of digits alone (RFC 9112, 6.3), with spaces and tabs
        // alone around it.
        let value = value.trim_matches([' ', '\t']);
        if length.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err("the answer's length is not one");
        }
        length = Some(value.parse::<u64>().unwrap_or(u64::MAX));
    }
    Ok((status, length.ok_or("the answer has no length")?))
}

/// Whether `text` is a token (RFC 9110, 5.6.2): one or more of the
/// letters, digits and marks a header's name may hold.
fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

/// The error for a pipe the child was not given.
fn no_pipe(which: &str) -> ureq::Error {
    ureq::Error::Io(io::Error::other(format!("the child has no {which}")))
}

/// Why a child closed the connection with no answer, from its exit code
/// (`None` while it runs, or when a signal ended it) and its stderr, and
/// whether it is ssh (`over_ssh`), whose own failures exit 255. A far
/// side with no cases, or one too old for `dial-stdio`, will not do better
/// on another try: its error is of kind [`io::ErrorKind::NotFound`] or
/// [`io::ErrorKind::Unsupported`], which the client gives as
/// [`Error::Dial`](crate::Error::Dial). Any other is
/// [`io::ErrorKind::UnexpectedEof`], a connection lost.
pub(crate) fn why_closed(over_ssh: bool, code: Option<i32>, stderr: &str) -> io::Error {
    let last = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("");
    let said = if last.is_empty() {
        String::new()
    } else {
        format!(": {last}")
    };
    if stderr.contains("unrecognized subcommand") && stderr.contains("dial-stdio") {
        return io::Error::new(
            io::ErrorKind::Unsupported,
            "the cases on the far side has no dial-stdio: upgrade it there",
        );
    }
    let lost = |message: String| io::Error::new(io::ErrorKind::UnexpectedEof, message);
    match code {
        Some(127) if over_ssh => io::Error::new(
            io::ErrorKind::NotFound,
            format!("there is no cases on the far side's PATH (exit 127){said}"),
        ),
        // The shell that runs the command says which it did not find:
        // the command, or cases at the far end of it.
        Some(127) => io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "a command was not found (exit 127){said}; the command must run cases \
                 dial-stdio on the far side"
            ),
        ),
        // The dial passes BatchMode=yes, so ssh asks for nothing: a
        // password, a key's passphrase or a new host key fails it.
        Some(255)
            if over_ssh
                && (stderr.contains("Permission denied")
                    || stderr.contains("Host key verification failed")) =>
        {
            lost(format!(
                "ssh failed (exit 255){said}; cases runs ssh with BatchMode=yes, so it cannot \
                 ask for a password, a passphrase or a new host key: use a key in an ssh \
                 agent, and accept the host's key with a plain ssh to it first"
            ))
        }
        Some(255) if over_ssh => lost(format!("ssh failed (exit 255){said}")),
        Some(code) => lost(format!("the connection closed (exit {code}){said}")),
        None => lost(format!("the connection closed{said}")),
    }
}

impl Transport for Pipe {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let timeout = timeout.for_write();
        if timeout.after.is_zero() {
            return Err(ureq::Error::Timeout(timeout.reason));
        }
        let output = self.buffers.output()[..amount].to_vec();
        self.write(
            output,
            timeout.not_zero().map(|after| *after),
            timeout.reason,
        )
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let timeout = timeout.for_read();
        if self.pending.is_empty() {
            if self.ended {
                return Err(self.closed());
            }
            if timeout.after.is_zero() {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            let stdout = self
                .stdout
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            let chunk = match timeout.not_zero() {
                Some(after) => stdout.recv_timeout(*after),
                None => stdout.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match chunk {
                Ok(Ok(bytes)) => self.pending.extend(bytes),
                Ok(Err(error)) => {
                    self.ended = true;
                    return Err(ureq::Error::Io(error));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ureq::Error::Timeout(timeout.reason));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    return Err(self.closed());
                }
            }
        }
        let input = self.buffers.input_append_buf();
        let n = input.len().min(self.pending.len());
        for (slot, byte) in input.iter_mut().zip(self.pending.drain(..n)) {
            *slot = byte;
        }
        self.buffers.input_appended(n);
        // Progress: bytes were waiting, and the HTTP client asks for input
        // only with room in its buffer, so at least one was appended.
        Ok(true)
    }

    /// Whether the connection can take another request. A pipe that
    /// failed is never asked: the HTTP client drops it.
    fn is_open(&mut self) -> bool {
        // Bytes the child sent past its answer close it, as does a write
        // not done in time.
        if !self.pending.is_empty() || self.stdin.as_ref().is_none_or(|stdin| stdin.stuck) {
            return false;
        }
        // So do bytes it sent since, and its stdout's end.
        let stdout = self
            .stdout
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        match stdout.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(_) | Err(TryRecvError::Disconnected) => {
                self.ended = true;
                return false;
            }
        }
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Pipe {
    /// Closes the child's stdin, which ends `cases dial-stdio`, and kills
    /// the child if it has not exited soon after: with its process group,
    /// its own (it was started in one), so that what it started goes too,
    /// such as the commands of an `exec:` forward's shell that did not
    /// exec the last. The group is killed before the child is reaped, while
    /// its id cannot be another's, and the child itself too, in case it
    /// left the group.
    ///
    /// A child with a write not done in time is killed at once: the write's
    /// thread holds its stdin, so the child would not see it close, and any
    /// grace is only time in which a request given up on could still land.
    fn drop(&mut self) {
        let stuck = self.stdin.as_ref().is_some_and(|stdin| stdin.stuck);
        self.stdin = None;
        let grace = if stuck { Duration::ZERO } else { CLOSE_GRACE };
        if self.exited(grace).is_none() {
            #[cfg(unix)]
            let _ = rustix::process::kill_process_group(
                rustix::process::Pid::from_child(&self.child),
                rustix::process::Signal::KILL,
            );
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write not done in time holds the child's stdin on its thread, so
    /// the child cannot see it close: it is killed, with no grace in which
    /// it could yet read what the request's caller has given up on.
    #[cfg(unix)]
    #[test]
    fn a_stuck_pipe_s_child_is_killed_at_once() {
        let dial = Arc::new(Dial::new("sleep", "exec:sleep").args(["30"]));
        // A grace is never shorter than itself, so one drop inside it is
        // enough, and a busy machine gets three goes at one.
        let inside = (0..3).any(|_| {
            let mut pipe = Pipe::start(&dial, 1024, 1024).unwrap();
            let written = pipe.write(
                vec![0; 1 << 20],
                Some(Duration::from_millis(50)),
                ureq::Timeout::Global,
            );
            assert!(
                matches!(written, Err(ureq::Error::Timeout(_))),
                "{written:?}"
            );
            let start = Instant::now();
            drop(pipe);
            start.elapsed() < CLOSE_GRACE
        });
        assert!(inside);
    }

    /// A pipe that is not stuck closes the child's stdin, and gives it its
    /// grace to end as it will.
    #[cfg(unix)]
    #[test]
    fn a_pipe_not_stuck_gives_its_child_time_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let ended = dir.path().join("ended");
        let dial = Arc::new(
            Dial::new("sh", "exec:sh")
                .args(["-c", ": > \"$READY\"; cat > /dev/null; : > \"$ENDED\""])
                .env("READY", &ready)
                .env("ENDED", &ended),
        );
        let pipe = Pipe::start(&dial, 1024, 1024).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < until, "the child did not start");
            thread::sleep(Duration::from_millis(5));
        }
        drop(pipe);
        assert!(ended.exists());
    }

    #[test]
    fn an_answer_is_read_once_all_of_it_is_there() {
        let answer = b"HTTP/1.1 409 Conflict\r\ncontent-length: 2\r\n\r\n{}";
        for n in 0..answer.len() {
            assert!(answer_in(&answer[..n], 10).unwrap().is_none(), "{n}");
        }
        assert_eq!(answer_in(answer, 10).unwrap(), Some((409, b"{}".to_vec())));
        let empty = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(answer_in(empty, 10).unwrap(), Some((200, Vec::new())));
    }

    #[test]
    fn an_answer_s_head_is_read_strictly() {
        let (status, header, length) = (
            "the answer's status line is not one",
            "the answer's header is not one",
            "the answer's length is not one",
        );
        for (bad, why) in [
            // The version is 1.0 or 1.1, as it is written.
            (&b"HTTP/1.x 200 OK\r\nContent-Length: 0\r\n\r\n"[..], status),
            (b"HTTP/1. 200 OK\r\nContent-Length: 0\r\n\r\n", status),
            (b"HTTP/1.10 200 OK\r\nContent-Length: 0\r\n\r\n", status),
            // A name is a token: not a line that goes on from the last
            // (RFC 9112, 5.2), nor one with space before it or its colon.
            // Each has a length of its own as well, so that it is refused
            // for its name and not for want of one.
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n Content-Length: 5\r\n\r\nhello",
                header,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\tContent-Length: 5\r\n\r\nhello",
                header,
            ),
            (
                b"HTTP/1.1 200 OK\r\n X: a\r\nContent-Length: 0\r\n\r\n",
                header,
            ),
            (
                b"HTTP/1.1 200 OK\r\nX : a\r\nContent-Length: 0\r\n\r\n",
                header,
            ),
            (
                b"HTTP/1.1 200 OK\r\n:x\r\nContent-Length: 0\r\n\r\n",
                header,
            ),
            (
                b"HTTP/1.1 200 OK\r\na b: 1\r\nContent-Length: 0\r\n\r\n",
                header,
            ),
            // A line with no colon is no header.
            (
                b"HTTP/1.1 200 OK\r\njunk\r\nContent-Length: 0\r\n\r\n",
                header,
            ),
            // One length, of digits alone (RFC 9112, 6.3), with spaces and
            // tabs alone around it.
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 2\r\n\r\n{}",
                length,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\ncontent-length: 2\r\n\r\n{}",
                length,
            ),
            (b"HTTP/1.1 200 OK\r\nContent-Length: +2\r\n\r\n{}", length),
            (b"HTTP/1.1 200 OK\r\nContent-Length: 2, 2\r\n\r\n{}", length),
            (b"HTTP/1.1 200 OK\r\nContent-Length: \r\n\r\n", length),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: \x0b2\r\n\r\n{}",
                length,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: \u{3000}2\r\n\r\n{}".as_bytes(),
                length,
            ),
        ] {
            match answer_in(bad, 10) {
                Err(ureq::Error::Io(error)) => {
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{bad:?}");
                    assert_eq!(error.to_string(), why, "{:?}", String::from_utf8_lossy(bad));
                }
                other => panic!("{:?}: {other:?}", String::from_utf8_lossy(bad)),
            }
        }
        // A length too long for a u64 is over the limit.
        let huge = b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\n";
        assert!(matches!(
            answer_in(huge, 10),
            Err(ureq::Error::BodyExceedsLimit(10))
        ));
        // HTTP/1.0, a name of every character a token may hold, and spaces
        // and tabs around a length, are taken.
        let fine = b"HTTP/1.0 200 OK\r\nX-a_1.2~!#$%&'*+^`|: v\r\nContent-Length: \t2 \r\n\r\n{}";
        assert_eq!(answer_in(fine, 10).unwrap(), Some((200, b"{}".to_vec())));
    }

    /// Asserts that `answer_in` refuses each of `bads` as a malformed
    /// answer, for the reason `why`.
    fn refused(bads: &[&[u8]], why: &str) {
        for bad in bads {
            match answer_in(bad, 10) {
                Err(ureq::Error::Io(error)) => {
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{bad:?}");
                    assert_eq!(error.to_string(), why, "{:?}", String::from_utf8_lossy(bad));
                }
                other => panic!("{:?}: {other:?}", String::from_utf8_lossy(bad)),
            }
        }
    }

    #[test]
    fn an_answer_s_status_is_three_digits_of_a_final_answer() {
        refused(
            &[
                b"HTTP/1.1 +200 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 0200 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 1 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 20 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 65535 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 042 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 100 Continue\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 199 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 600 OK\r\nContent-Length: 0\r\n\r\n",
                b"HTTP/1.1 2\xd9\xa000 OK\r\nContent-Length: 0\r\n\r\n",
            ],
            "the answer's status line is not one",
        );
        // Any final status, with or without a reason, is taken.
        for (fine, status) in [
            (&b"HTTP/1.1 599 Odd\r\nContent-Length: 0\r\n\r\n"[..], 599),
            (b"HTTP/1.1 200\r\nContent-Length: 0\r\n\r\n", 200),
            (b"HTTP/1.1 409 \r\nContent-Length: 0\r\n\r\n", 409),
        ] {
            assert_eq!(answer_in(fine, 10).unwrap(), Some((status, Vec::new())));
        }
    }

    #[test]
    fn an_answer_with_a_transfer_coding_fails() {
        // A transfer coding with a length is an error (RFC 9112, 6.3), and
        // the reader knows no coding without one.
        refused(
            &[
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}",
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\ntransfer-encoding: identity\r\n\r\n{}",
                b"HTTP/1.1 200 OK\r\nTRANSFER-ENCODING:\r\nContent-Length: 2\r\n\r\n{}",
            ],
            "the answer has a transfer coding",
        );
    }

    #[test]
    fn an_answer_s_head_ends_its_lines_with_crlf_alone() {
        // A bare LF or CR is neither a line's end nor in one, so it can't
        // hide a header in the value of the last.
        refused(
            &[
                b"HTTP/1.1 200 OK\r\nX: a\nContent-Length: 9\r\nContent-Length: 2\r\n\r\n{}",
                b"HTTP/1.1 200 OK\r\nX: a\rContent-Length: 9\r\nContent-Length: 2\r\n\r\n{}",
                b"HTTP/1.1 200 OK\nX: a\r\nContent-Length: 2\r\n\r\n{}",
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\r\n\r\n{}",
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\n\r\n\r\n{}",
            ],
            "the answer's line has a bare CR or LF",
        );
    }

    #[test]
    fn an_answer_that_is_not_one_fails() {
        for bad in [
            &b"HTTP/1.1 200 OK\r\n\r\n"[..],
            b"HTTP/1.1 OK\r\nContent-Length: 0\r\n\r\n",
            b"SSH-2.0 200\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: x\r\n\r\n",
            b"\xff\r\n\r\n",
        ] {
            let error = answer_in(bad, 10).unwrap_err();
            assert!(matches!(error, ureq::Error::Io(_)), "{bad:?}: {error:?}");
        }
        let large = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n";
        assert!(matches!(
            answer_in(large, 10),
            Err(ureq::Error::BodyExceedsLimit(10))
        ));
        // A body of the limit itself is read.
        let full = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n";
        assert!(answer_in(full, 10).unwrap().is_none());
        // A head is waited for up to 64 KiB, as the error says.
        assert!(answer_in(&vec![b'x'; 64 * 1024], 10).unwrap().is_none());
        let endless = vec![b'x'; ANSWER_HEAD_LIMIT + 1];
        assert!(answer_in(&endless, 10).is_err());
        assert!(
            answer_in(&endless[..ANSWER_HEAD_LIMIT], 10)
                .unwrap()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_write_not_done_in_time_leaves_the_pipe_stuck() {
        // The child reads nothing for 1 s, and then all it is sent.
        let dial = Arc::new(Dial::new("sh", "far").args(["-c", "sleep 1; cat > /dev/null"]));
        let mut pipe = Pipe::start(&dial, 1024, 1024).unwrap();
        let large = vec![b'x'; 200 * 1024];
        let start = Instant::now();
        let written = pipe.write(
            large,
            Some(Duration::from_millis(100)),
            ureq::Timeout::SendBody,
        );
        assert!(
            matches!(written, Err(ureq::Error::Timeout(ureq::Timeout::SendBody))),
            "{written:?}"
        );
        assert!(
            start.elapsed() < Duration::from_millis(900),
            "{:?}",
            start.elapsed()
        );
        // Part of the request is in the pipe, so the child cannot take
        // another.
        assert!(!Transport::is_open(&mut pipe));
        // A later write fails at once: it does not wait, nor take the
        // timed-out write's result, which comes once the child reads, as
        // its own.
        let start = Instant::now();
        let again = pipe.write(
            b"GET".to_vec(),
            Some(Duration::from_secs(5)),
            ureq::Timeout::Global,
        );
        assert!(
            matches!(again, Err(ureq::Error::Timeout(ureq::Timeout::Global))),
            "{again:?}"
        );
        let exchanged = pipe.exchange(b"GET / HTTP/1.1\r\n\r\n", Duration::from_secs(5), 10);
        assert!(
            matches!(exchanged, Err(ureq::Error::Timeout(_))),
            "{exchanged:?}"
        );
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "{:?}",
            start.elapsed()
        );
        assert!(!Transport::is_open(&mut pipe));
    }

    #[cfg(unix)]
    #[test]
    fn an_exchange_with_a_child_that_does_not_read_times_out() {
        // A hold's request, renewal and release: a request larger than
        // the pipe, to a child that never reads, fails at its timeout and
        // not when the child exits, at 3 s.
        let dial = Arc::new(Dial::new("sh", "far").args(["-c", "sleep 3"]));
        let mut pipe = Pipe::start(&dial, 1024, 1024).unwrap();
        let start = Instant::now();
        let exchanged = pipe.exchange(&vec![b'x'; 200 * 1024], Duration::from_millis(300), 10);
        assert!(
            matches!(exchanged, Err(ureq::Error::Timeout(ureq::Timeout::Global))),
            "{exchanged:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_pipe_shows_its_dial_and_its_child() {
        let dial = Arc::new(Dial::new("cat", "ssh://far"));
        let pipe = Pipe::start(&dial, 1024, 1024).unwrap();
        let shown = format!("{pipe:?}");
        assert!(
            shown.starts_with("Pipe { dial: Dial {")
                && shown.contains(&format!("pid: {}", pipe.child.id())),
            "{shown}"
        );
    }
}
