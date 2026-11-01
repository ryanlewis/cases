//! A `cases serve` a test starts, and a plain HTTP/1.1 client to talk to it.
//! The client writes each request by hand, so that a test can send any
//! `Host`, `Origin` or body it likes. No test opens a browser: serve opens
//! one only when stdout is a terminal, and a test's stdout is a pipe.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{Env, Output};

/// How long a test waits for serve to do what it should.
pub(crate) const WAIT: Duration = Duration::from_secs(5);

/// A running `cases serve`, stopped when dropped.
pub(crate) struct Serve {
    child: Option<Child>,
    /// The inbox, such as `http://127.0.0.1:53211/`.
    pub(crate) url: String,
    /// The address serve listens on, such as `127.0.0.1:53211`.
    pub(crate) addr: String,
    /// The first line serve printed.
    pub(crate) line: String,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    /// The threads that read serve's stdout and stderr, which end when serve
    /// does.
    readers: Vec<JoinHandle<()>>,
}

impl Env {
    /// Starts `cases --store <store> serve` with `args`, and waits for the
    /// line that says where it listens. Without a `--listen` in `args`, it
    /// listens on a port of its own on 127.0.0.1.
    pub(crate) fn serve(&self, args: &[&str]) -> Serve {
        self.serve_as(Path::new(env!("CARGO_BIN_EXE_cases")), args)
    }

    /// As [`Env::serve`], with the binary at `program`.
    pub(crate) fn serve_as(&self, program: &Path, args: &[&str]) -> Serve {
        let mut all = self.store_args(&["serve"]);
        if !args.contains(&"--listen") {
            all.extend(["--listen", "127.0.0.1:0"]);
        }
        all.extend(args);
        let mut child = super::spawn(
            self.command_as(program, &all)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let (first, stdout, out) = lines(child.stdout.take().expect("a stdout"));
        let (stderr, err) = collect(child.stderr.take().expect("a stderr"));
        let Ok(line) = first.recv_timeout(WAIT) else {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "serve printed no line; stderr: {}",
                stderr.lock().expect("stderr")
            );
        };
        let url = line
            .rsplit(' ')
            .next()
            .filter(|url| url.starts_with("http://"))
            .unwrap_or_else(|| panic!("serve printed no URL: {line:?}"))
            .to_owned();
        let addr = url
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_owned();
        Serve {
            child: Some(child),
            url,
            addr,
            line,
            stdout,
            stderr,
            readers: vec![out, err],
        }
    }

    /// Starts `cases` with `args`, with only the environment the test set,
    /// its output piped to the test.
    pub(crate) fn spawn(&self, args: &[&str]) -> Child {
        super::spawn(
            self.command(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
    }
}

/// A connection to `addr`, made while no process starts, so that no process
/// another test starts holds it open (see `FDS` in the module above).
pub(crate) fn connect(addr: &str) -> TcpStream {
    let _alone = super::FDS.lock().unwrap_or_else(PoisonError::into_inner);
    TcpStream::connect(addr).expect("serve accepts")
}

/// Sends the first line of `stdout` on the channel it gives, and keeps the
/// rest.
fn lines(stdout: ChildStdout) -> (Receiver<String>, Arc<Mutex<String>>, JoinHandle<()>) {
    let (first, receiver) = mpsc::channel();
    let rest = Arc::new(Mutex::new(String::new()));
    let kept = Arc::clone(&rest);
    let reader = thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        if let Some(Ok(line)) = lines.next() {
            let _ = first.send(line);
        }
        for line in lines.map_while(Result::ok) {
            let mut kept = kept.lock().expect("stdout");
            kept.push_str(&line);
            kept.push('\n');
        }
    });
    (receiver, rest, reader)
}

/// Keeps what `input` gives until it ends.
fn collect(mut input: impl Read + Send + 'static) -> (Arc<Mutex<String>>, JoinHandle<()>) {
    let text = Arc::new(Mutex::new(String::new()));
    let kept = Arc::clone(&text);
    let reader = thread::spawn(move || {
        let mut buf = [0; 4096];
        while let Ok(n) = input.read(&mut buf) {
            if n == 0 {
                break;
            }
            kept.lock()
                .expect("stderr")
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    (text, reader)
}

impl Serve {
    /// The serve's process id.
    pub(crate) fn pid(&self) -> u32 {
        self.child.as_ref().expect("serve runs").id()
    }

    /// What serve wrote to stderr so far.
    pub(crate) fn stderr(&self) -> String {
        self.stderr.lock().expect("stderr").clone()
    }

    /// Waits for serve to exit by itself, which it must within `within`.
    pub(crate) fn exit_within(mut self, within: Duration) -> Output {
        let child = self.child.take().expect("serve runs");
        self.wait(child, within)
    }

    /// Sends serve `signal`, as `kill` does, and waits for it to exit.
    #[cfg(unix)]
    pub(crate) fn signal(mut self, signal: rustix::process::Signal) -> Output {
        self.raise(signal);
        let child = self.child.take().expect("serve runs");
        self.wait(child, WAIT)
    }

    /// Sends serve `signal`, as `kill` does, and does not wait.
    #[cfg(unix)]
    pub(crate) fn raise(&self, signal: rustix::process::Signal) {
        let child = self.child.as_ref().expect("serve runs");
        rustix::process::kill_process(rustix::process::Pid::from_child(child), signal)
            .expect("the signal is sent");
    }

    /// Stops serve as a service manager does, with SIGTERM, and gives what
    /// it did.
    #[cfg(unix)]
    pub(crate) fn stop(self) -> Output {
        self.signal(rustix::process::Signal::TERM)
    }

    /// Waits up to `within` for serve to exit.
    fn wait(&mut self, mut child: Child, within: Duration) -> Output {
        let deadline = Instant::now() + within;
        let status = loop {
            if let Some(status) = child.try_wait().expect("serve is waited for") {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("serve did not stop; stderr: {}", self.stderr());
            }
            thread::sleep(Duration::from_millis(20));
        };
        // The readers take what is left, and end as the pipes close.
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        Output {
            code: status.code().unwrap_or(-1),
            stdout: format!("{}\n{}", self.line, self.stdout.lock().expect("stdout")),
            stderr: self.stderr(),
        }
    }

    /// `GET path`, as the page does.
    pub(crate) fn get(&self, path: &str) -> Response {
        self.send(&Request::new("GET", path))
    }

    /// The CSRF token a page reads from `GET /v1`.
    pub(crate) fn csrf(&self) -> String {
        let index = self.get("/v1").json();
        index["csrf"].as_str().expect("a token").to_owned()
    }

    /// `POST path` with the JSON `body`, as the page sends a write: from the
    /// inbox itself, with its CSRF token.
    pub(crate) fn post(&self, path: &str, body: &str) -> Response {
        let token = self.csrf();
        self.send(&Request::write(path, body).header("X-Cases-CSRF", &token))
    }

    /// Sends `request` to serve, addressed to its own host unless the
    /// request names another.
    pub(crate) fn send(&self, request: &Request) -> Response {
        let mut stream = connect(&self.addr);
        stream.set_read_timeout(Some(WAIT)).expect("a timeout");
        stream
            .write_all(&request.bytes(&self.addr))
            .expect("the request is sent");
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .expect("the response is read");
        Response::parse(&bytes)
    }

    /// Opens the event stream with `query`, such as `?after=V`, as the
    /// browser does, sending `last` as `Last-Event-ID` when given.
    pub(crate) fn stream(&self, query: &str, last: Option<&str>) -> Stream {
        let mut request = Request::new("GET", &format!("/v1/changes{query}"));
        if let Some(last) = last {
            request = request.header("Last-Event-ID", last);
        }
        Stream::open(&self.addr, &request)
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A request, written out by hand.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    method: String,
    path: String,
    /// The headers, `Host` among them when the test sets it; without one,
    /// the request names serve's own address.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    pub(crate) fn new(method: &str, path: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A write as the inbox's page sends one, without its token: JSON, from
    /// the same origin.
    pub(crate) fn write(path: &str, body: &str) -> Request {
        Request::new("POST", path)
            .header("Content-Type", "application/json")
            .header("Sec-Fetch-Site", "same-origin")
            .body(body.as_bytes())
    }

    /// The request with the header `name` set to `value`, in place of any
    /// it had.
    pub(crate) fn header(mut self, name: &str, value: &str) -> Request {
        self.headers
            .retain(|(have, _)| !have.eq_ignore_ascii_case(name));
        self.headers.push((name.into(), value.into()));
        self
    }

    pub(crate) fn body(mut self, body: &[u8]) -> Request {
        self.body = body.to_vec();
        self
    }

    /// The request as it goes on the wire to `addr`.
    fn bytes(&self, addr: &str) -> Vec<u8> {
        let mut head = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        if !self
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            let _ = write!(head, "Host: {addr}\r\n");
        }
        for (name, value) in &self.headers {
            let _ = write!(head, "{name}: {value}\r\n");
        }
        if !self.body.is_empty() || self.method == "POST" {
            let _ = write!(head, "Content-Length: {}\r\n", self.body.len());
        }
        head.push_str("Connection: close\r\n\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend(&self.body);
        bytes
    }
}

/// A response, read whole.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Response {
    fn parse(bytes: &[u8]) -> Response {
        let text = String::from_utf8_lossy(bytes);
        let (head, body) = text
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("no response head: {text:?}"));
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("no status: {head:?}"));
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
            .collect();
        let chunked = headers
            .iter()
            .any(|(name, value)| name == "transfer-encoding" && value == "chunked");
        let body = if chunked {
            unchunk(body)
        } else {
            body.to_owned()
        };
        Response {
            status,
            headers,
            body,
        }
    }

    /// The header `name`, if the response has it.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(have, _)| *have == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body, as JSON.
    pub(crate) fn json(&self) -> serde_json::Value {
        super::json(&self.body)
    }

    /// The error body's category, for a refusal.
    pub(crate) fn category(&self) -> String {
        self.json()["error"]["category"]
            .as_str()
            .unwrap_or_else(|| panic!("no error category: {self:?}"))
            .to_owned()
    }
}

/// A chunked body, put back together.
fn unchunk(mut body: &str) -> String {
    let mut text = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).expect("a chunk size");
        if size == 0 {
            break;
        }
        text.push_str(&rest[..size]);
        body = rest[size..].trim_start_matches("\r\n");
    }
    text
}

/// What an open event stream has given.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Message {
    /// A message, by its id.
    Id(String),
    /// The stream ended: cleanly, with the end of its body, or cut off.
    End { clean: bool },
}

/// An open event stream, read on a thread of its own.
pub(crate) struct Stream {
    messages: Receiver<Message>,
    socket: TcpStream,
}

impl Stream {
    fn open(addr: &str, request: &Request) -> Stream {
        let mut socket = connect(addr);
        socket
            .write_all(&request.bytes(addr))
            .expect("the request is sent");
        let mut reader = BufReader::new(socket.try_clone().expect("a second handle"));
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("the head is read");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            head.push_str(&line);
        }
        let head = head.to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(
            head.contains("content-type: text/event-stream\r\n"),
            "{head}"
        );
        assert!(head.contains("cache-control: no-store\r\n"), "{head}");
        assert!(head.contains("transfer-encoding: chunked\r\n"), "{head}");
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut text = String::new();
            let clean = loop {
                let mut size = String::new();
                if reader.read_line(&mut size).unwrap_or(0) == 0 {
                    break false;
                }
                let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
                    break false;
                };
                if size == 0 {
                    break true;
                }
                let mut chunk = vec![0; size + 2];
                if reader.read_exact(&mut chunk).is_err() {
                    break false;
                }
                text.push_str(&String::from_utf8_lossy(&chunk[..size]));
                while let Some((message, rest)) = text.split_once("\n\n") {
                    let (id, data) = message.split_once('\n').unwrap_or((message, ""));
                    let id = id.strip_prefix("id: ").unwrap_or_default().to_owned();
                    assert_eq!(data, format!("data: {id}"), "a message is its version");
                    let _ = sender.send(Message::Id(id));
                    text = rest.to_owned();
                }
            };
            let _ = sender.send(Message::End { clean });
        });
        Stream { messages, socket }
    }

    /// The id of the next message, which must come within [`WAIT`].
    pub(crate) fn next(&self) -> String {
        match self.messages.recv_timeout(WAIT) {
            Ok(Message::Id(id)) => id,
            Ok(end) => panic!("the stream ended ({end:?}) before a message"),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                panic!("no message on the stream")
            }
        }
    }

    /// Checks that no message comes for `time`.
    pub(crate) fn quiet(&self, time: Duration) {
        match self.messages.recv_timeout(time) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("the stream gave {other:?}, want nothing"),
        }
    }

    /// How the stream ends, which it must within [`WAIT`], after any
    /// messages still to come.
    pub(crate) fn end(&self) -> Message {
        loop {
            match self.messages.recv_timeout(WAIT) {
                Ok(Message::Id(_)) => {}
                Ok(end) => return end,
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    panic!("the stream did not end")
                }
            }
        }
    }

    /// Goes away, as a tab that is closed does.
    pub(crate) fn close(self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
