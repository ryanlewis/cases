//! `cases serve`: serves the local web inbox (SV-02, IN-01 to IN-12).
//!
//! serve listens on a loopback address and serves the inbox's JSON API
//! (spec/api.md) and its event stream. It records itself in an instance file
//! for `status`, `show` and `inbox`, reads the store about once a second so
//! that open pages and the notification feed follow it, and stops on `q`,
//! Ctrl-C or SIGTERM, or when its binary is replaced, so that a service
//! manager starts the new one. The async runtime starts here and nowhere else: every
//! other command is synchronous.

mod api;
mod app;
mod binary;
mod briefings;
mod favicon;
mod http;
mod log;
#[cfg(test)]
mod looks;
mod notify;
mod page;
mod screen;
mod stream;
mod term;
mod watch;

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cases_core::Time;
use cases_store_sqlite::Db;
#[cfg(unix)]
use tokio::signal::unix::SignalKind;
use tokio::sync::{mpsc, watch as channel};
use tokio::time::MissedTickBehavior;

use self::app::App;
use self::binary::Binary;
use self::http::Timeouts;
use self::log::{Log, Rotating};
use self::screen::{Counts, Screen, View};
use self::watch::Watch;
use crate::ctx::Ctx;
use crate::error::Error;
use crate::instance::{self, Info};
use crate::store::{self, Target};
use crate::tokens::{self, Tokens};
use crate::{browser, config, listen, output};

/// How often serve reads the store by itself, and redraws its screen.
const TICK: Duration = Duration::from_secs(1);

/// How many log lines the status screen shows.
const LOG_LINES: usize = 5;

/// How long a stop waits, once its connections are closed, for the store's
/// reads and writes still running on blocking threads before it leaves
/// them: a write takes far less, and a held poll is let go.
const STOP_LEFT: Duration = Duration::from_secs(1);

/// The arguments of `cases serve`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Address to listen on: a loopback HOST:PORT. Default: the config
    /// file's listen, then 127.0.0.1:7337.
    #[arg(long, value_name = "HOST:PORT")]
    listen: Option<String>,

    /// Do not open the inbox in the browser, which serve does only when
    /// stdout is a terminal. `--no-open=false` opens it even when the config
    /// file's no-open says not to.
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true"
    )]
    no_open: Option<bool>,

    /// Record answers, parks, resumes and withdraws from the inbox as
    /// written by NAME. Default: the config file's name, then the login
    /// name.
    #[arg(long = "as", value_name = "NAME")]
    as_: Option<String>,

    /// Also write the log to FILE, which serve rotates at 1 MiB, keeping
    /// FILE.1 and FILE.2. Off a terminal, stderr goes to FILE too, so that
    /// an error that stops serve is logged there. The launchd service
    /// passes it.
    #[arg(long, value_name = "FILE")]
    log: Option<PathBuf>,
}

/// Serves the inbox until it is told to stop.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let named = match ctx.target.clone() {
        Target::Local { path } => path,
        // A forwarded store's inbox is served where the store is.
        Target::Forward { forward } => {
            return Err(Error::new(format!(
                "serve runs against a local store: run cases serve on {forward}, where the \
                 store is"
            )));
        }
        Target::Remote { server, .. } | Target::Server { server } => {
            return hosted(args, ctx, &server);
        }
    };
    // On a terminal the status screen owns stdout, and shows the log
    // rather than let it scroll the screen away.
    let terminal = io::stdout().is_terminal();
    let log = Arc::new(open_log(args, terminal));
    let listen = listen_address(args, ctx)?;
    let no_open = no_open(args.no_open, ctx.config.get("no-open"));
    let close_after = crate::briefings::close_after(&ctx.config)?;
    let store_tokens = store_tokens(ctx)?;
    let store = std::path::absolute(&named)?;
    refuse_if_running(&store)?;
    let (listener, host, port, addr) = bind(&listen)?;
    let url = format!("http://{addr}/");
    let proxied = ctx.config.list("allow-hosts").to_vec();
    let link = instance::link(&url, &proxied);

    let started = output::now()?;
    let (db, watch) = follow(&store, &link, &log)?;
    let pid = i32::try_from(std::process::id()).unwrap_or_default();
    let info = Info {
        pid,
        url: url.clone(),
        link: link.clone(),
        addr,
        // Absolute and with each `..` taken off, for status to show.
        store: instance::clean(&store).display().to_string(),
        started_at: output::stamp(Some(started)),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let recorded = record(&store, &info, &log);
    let serving = Serving {
        store: &named,
        db,
        watch,
        writer: ctx.human(args.as_.as_deref()),
        listener,
        host,
        port,
        proxied,
        logins: ctx.config.list("tailscale-logins").to_vec(),
        store_tokens,
        close_after,
        url,
        link,
        log,
        started,
        terminal,
        open: terminal && !no_open,
        recorded: recorded.then(|| (store.clone(), pid)),
        // The binary as it is now, before serve says it is up: a test that
        // replaces it once serve is up must find it watched.
        binary: super::service::executable().ok().and_then(Binary::new),
    };
    let result = serving.run();
    if recorded {
        let _ = instance::remove(&store, pid);
    }
    result
}

/// A remote store's inbox is the hosted one: serve starts nothing, says
/// where it is, and on a terminal opens it (ST-11).
fn hosted(args: &Args, ctx: &mut Ctx, server: &store::Server) -> Result<(), Error> {
    let url = store::inbox_url(server);
    writeln!(ctx.out, "{url}")?;
    if io::stdout().is_terminal() && !no_open(args.no_open, ctx.config.get("no-open")) {
        browser::open(&url)?;
    }
    Ok(())
}

/// The log: on a terminal it is kept for the status screen, and goes to
/// stderr only when that is not the terminal too. The log file comes first,
/// so that an error that stops serve from here on goes to it with stderr.
fn open_log(args: &Args, terminal: bool) -> Log {
    let mut log = if terminal {
        Log::new(!io::stderr().is_terminal(), LOG_LINES)
    } else {
        Log::new(true, 0)
    };
    if let Some(path) = &args.log {
        match Rotating::open(path, !io::stderr().is_terminal()) {
            Ok(file) => log = log.with_file(file),
            Err(error) => log.line(format!(
                "could not open the log file {}: {error}",
                path.display()
            )),
        }
    }
    log
}

/// The address to listen on: the flag, else the config file's `listen`,
/// else the default, which must be a loopback one.
fn listen_address(args: &Args, ctx: &Ctx) -> Result<String, Error> {
    let listen = args
        .listen
        .clone()
        .or_else(|| ctx.config.get("listen").map(str::to_owned))
        .unwrap_or_else(|| config::DEFAULT_LISTEN.to_owned());
    listen::check_loopback(&listen)?;
    Ok(listen)
}

/// Refuses to start beside a serve that runs on `store`. A file that cannot
/// be read counts as a serve that is not running, and is replaced.
fn refuse_if_running(store: &Path) -> Result<(), Error> {
    instance::path(store).map_or(Ok(()), |path| refuse_if_running_at(&path))
}

/// [`refuse_if_running`], for the instance file at `path`.
fn refuse_if_running_at(path: &Path) -> Result<(), Error> {
    if let Ok(Some(running)) = instance::running_at(path) {
        return Err(Error::new(format!(
            "cases serve is already running for {} at {} (pid {})",
            running.store, running.url, running.pid
        )));
    }
    Ok(())
}

/// The tokens of the config file's `store-tokens-file`, none when it is
/// not given or blank.
fn store_tokens(ctx: &Ctx) -> Result<Tokens, Error> {
    match ctx.config.get("store-tokens-file") {
        Some(path) if !path.trim().is_empty() => Ok(Tokens::new(tokens::read(
            path.trim(),
            "the config file's store-tokens-file",
        )?)),
        _ => Ok(Tokens::default()),
    }
}

/// Binds `listen`, and gives the listener, the host as given (so localhost
/// stays localhost), the port bound (which differs from the one asked for
/// when that is 0) and the address with the two.
fn bind(listen: &str) -> Result<(TcpListener, &str, u16, String), Error> {
    let listener = TcpListener::bind(listen)
        .map_err(|error| Error::new(format!("listen on {listen}: {error}")))?;
    let port = listener.local_addr()?.port();
    let (host, _) = listen::split(listen).unwrap_or_default();
    let addr = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Ok((listener, host, port, addr))
}

/// Opens the store and follows it, read once already.
fn follow(store: &Path, link: &str, log: &Arc<Log>) -> Result<(Arc<Db>, Watch), Error> {
    let db = Db::new(store);
    #[cfg(debug_assertions)]
    let db = crate::dial::test_clock(&db);
    let db = Arc::new(db);
    let watch = Watch::new(Arc::clone(&db), token(8)?, link.to_owned(), Arc::clone(log))?;
    // The first read is the notifications' baseline: the cases in the store
    // when serve started (IN-08). It comes before serve says it is up, in its
    // instance file and its address line, so that a case opened once it is
    // up is new. A read that fails leaves the baseline to the next one.
    let _ = watch.read();
    Ok((db, watch))
}

/// Records the instance for status, and whether it did. serve runs without
/// the file, as when the state directory cannot be written; status then
/// does not find it.
fn record(store: &Path, info: &Info, log: &Log) -> bool {
    match instance::write(store, info) {
        Ok(()) => true,
        Err(error) => {
            log.line(format!(
                "could not record the instance for cases status: {error}"
            ));
            false
        }
    }
}

/// Whether serve leaves the browser alone: the flag, else the config
/// file's `no-open`, else not.
fn no_open(flag: Option<bool>, config: Option<&str>) -> bool {
    flag.unwrap_or(config == Some("true"))
}

/// What a run of serve needs, once its address is bound and recorded.
struct Serving<'a> {
    /// The store as it was named, to show.
    store: &'a Path,
    /// The store, open until serve stops.
    db: Arc<Db>,
    /// The store as serve follows it, read once already.
    watch: Watch,
    writer: cases_store_sqlite::Writer,
    listener: TcpListener,
    host: &'a str,
    port: u16,
    /// The config file's `allow-hosts`.
    proxied: Vec<String>,
    /// The config file's `tailscale-logins`.
    logins: Vec<String>,
    /// The tokens of the config file's `store-tokens-file`.
    store_tokens: Tokens,
    /// The config file's `briefing-close-after`, unless it is off.
    close_after: Option<Duration>,
    url: String,
    /// The inbox's address in links (`instance::link`).
    link: String,
    log: Arc<Log>,
    started: Time,
    /// Whether stdout is a terminal, for the status screen.
    terminal: bool,
    /// Whether to open the inbox in the browser.
    open: bool,
    /// The binary serve runs, watched for an upgrade.
    binary: Option<Binary>,
    /// The store and process of the instance file, when serve wrote one:
    /// what a stop forced by a signal removes as it exits.
    recorded: Option<(PathBuf, i32)>,
}

impl Serving<'_> {
    fn run(self) -> Result<(), Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        // Where the log file starts on each run, as for a restart.
        self.log.file_only(&format!(
            "{} Serving {} at {}",
            output::stamp(Some(self.started)),
            self.store.display(),
            self.url
        ));
        for host in &self.proxied {
            self.log.line(format!(
                "also serving https://{host}/ through a proxy (allow-hosts)"
            ));
        }
        let served = runtime.block_on(self.serve());
        // A held poll on the store routes (`GET store/changes`, up to
        // 300 s) runs on a blocking thread that the stop does not end: its
        // connection was closed within the grace, so it is left, as is any
        // other still running, rather than holding the stop up.
        runtime.shutdown_timeout(STOP_LEFT);
        served
    }

    async fn serve(self) -> Result<(), Error> {
        self.listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let (stopping, stopped) = channel::channel(false);
        let app = Arc::new(App {
            watch: self.watch,
            db: self.db,
            writer: self.writer,
            hosts: app::hosts(self.host, self.port),
            proxied: self.proxied,
            logins: self.logins,
            store_tokens: self.store_tokens,
            leases: crate::dial::Leases::new(crate::dial::lease()),
            csrf: token(16)?,
            url: self.url.clone(),
            link: self.link,
            log: Arc::clone(&self.log),
            stopping: stopped,
            requests: AtomicU64::new(0),
            timeouts: Timeouts::SERVE,
        });

        // The signals are caught before the address is printed: a test that
        // sends one once it sees the address must not kill the process.
        let (quit, mut quits) = mpsc::unbounded_channel();
        let begun = Arc::new(AtomicBool::new(false));
        let forced: Restore = Arc::default();
        let signals = signals(Arc::clone(&begun), Arc::clone(&forced), self.recorded)?;
        if self.terminal {
            if self.open
                && let Err(error) = browser::open(&self.url)
            {
                self.log
                    .line(format!("could not open the browser: {error}"));
            }
        } else {
            let mut out = io::stdout().lock();
            writeln!(out, "Serving {} at {}", self.store.display(), self.url)?;
            out.flush()?;
        }
        let raw = arm(&forced, self.terminal, || take_keys(self.terminal, quit));
        let keys = raw.is_some();
        let painter = self
            .terminal
            .then(|| Painter::start(self.store.display().to_string(), self.started, keys));
        let ticker = tokio::spawn(tick(Arc::clone(&app), painter));
        let closer = self
            .close_after
            .map(|after| tokio::spawn(briefings::close_stale(Arc::clone(&app), after)));

        // serve stops when its binary is replaced, run by a service manager
        // or by hand: a service manager starts the new one, and by hand the
        // log line says why it stopped.
        let replaced = replaced(self.binary, Arc::clone(&self.log));
        // Without a keyboard to read, nothing sends on `quits`, and it ends
        // at once: that is no reason to stop.
        let stop = async move {
            tokio::select! {
                () = signals => {}
                Some(()) = quits.recv() => {}
                () = replaced => {}
            }
            begun.store(true, Ordering::SeqCst);
        };
        let router = app::router(Arc::clone(&app));
        let left = http::serve(listener, router, app.timeouts, stop, || {
            stopping.send_replace(true);
        })
        .await;
        if left > 0 {
            self.log.line(format!(
                "closed {left} connections still open {} s after the stop",
                app.timeouts.grace.as_secs()
            ));
        }
        // The ticker stops with the streams, and gives the screen back for
        // a last frame, after which the painter puts the terminal back.
        if let Ok(Some(painter)) = ticker.await {
            drop(step(&app, Some(painter)).await);
        }
        if let Some(closer) = closer {
            let _ = closer.await;
        }
        // The keys are put back here, not as `raw` drops: the thread that
        // waits for a second signal holds it until serve exits.
        if let Some(raw) = &raw {
            raw.restore();
        }
        Ok(())
    }
}

/// On a terminal whose keys serve can read, takes them, each as it is
/// typed, and sends on `quit` when `q` is: gives the keys, to put back.
/// Without a keyboard to read it takes nothing, and `quit` is dropped.
fn take_keys(terminal: bool, quit: mpsc::UnboundedSender<()>) -> Option<Arc<term::Keys>> {
    let keys = terminal && term::can_read_keys();
    keys.then(|| {
        let raw = Arc::new(term::keys_raw());
        std::thread::spawn(move || {
            if term::wait_for_q(term::ForegroundStdin) {
                let _ = quit.send(());
            }
        });
        raw
    })
}

/// Takes the keys with `take` and installs what a forced stop puts back,
/// holding the slot throughout: a forced stop that comes between the two
/// waits for it, rather than find nothing to put back.
fn arm(
    forced: &Restore,
    screen: bool,
    take: impl FnOnce() -> Option<Arc<term::Keys>>,
) -> Option<Arc<term::Keys>> {
    let mut slot = forced.lock().unwrap_or_else(PoisonError::into_inner);
    let raw = take();
    *slot = Some(forced_restore(screen, raw.clone()));
    raw
}

/// What a stop forced by a second signal puts back on the terminal before
/// it exits: the cursor and line wrapping, when serve drew its `screen`,
/// and the keys it took (`raw`).
fn forced_restore(screen: bool, raw: Option<Arc<term::Keys>>) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        // The keys first: a terminal whose output is stopped (Ctrl-S) holds
        // the write of the screen's end, and serve exits while it waits.
        if let Some(raw) = &raw {
            raw.restore();
        }
        if screen {
            show(Screen::END);
        }
    })
}

/// Resolves once `binary` is replaced, and logs it; never without a binary
/// to watch.
async fn replaced(binary: Option<Binary>, log: Arc<Log>) {
    let Some(binary) = binary else {
        return std::future::pending().await;
    };
    let path = binary.watch().await;
    log.line(format!(
        "{} was replaced: stopping, so that the new version can start",
        path.display()
    ));
}

/// Writes `text` to the terminal on stdout at once, from any thread.
fn show(text: &str) {
    let _ = term::Out.write_all(text.as_bytes());
}

/// A token of `bytes` random bytes, in hex: the CSRF token, new each time
/// serve starts (IN-11), and the notification feed's boot id (IN-08).
fn token(bytes: usize) -> Result<String, Error> {
    let mut random = vec![0; bytes];
    getrandom::fill(&mut random)
        .map_err(|error| Error::new(format!("no random bytes: {error}")))?;
    Ok(random.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// What a stop forced by a signal puts back on the terminal first, once
/// there is something to put back.
type Restore = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

/// Resolves on Ctrl-C or, on unix, SIGTERM. The handlers are in place once
/// this returns, before the future is first polled.
///
/// Once serve is stopping (`begun`), a second Ctrl-C or SIGTERM ends it at
/// once, as the default would, rather than wait out the stop, which waits
/// for the writes in flight, each of which can wait 5 s on the store's
/// lock. It removes the instance file that `recorded` names, and `restore`
/// puts the terminal back, first. The signals are caught on a thread and a
/// runtime of their own from now until serve exits, so that a second signal
/// counts however soon it follows the first, and while serve's runtime, as
/// it drops, waits for those writes and runs nothing else.
#[cfg(unix)]
fn signals(
    begun: Arc<AtomicBool>,
    restore: Restore,
    recorded: Option<(PathBuf, i32)>,
) -> Result<impl Future<Output = ()>, Error> {
    let (first, stop) = tokio::sync::oneshot::channel();
    let (ready, caught) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (runtime, mut interrupt, mut terminate) =
            match handlers(SignalKind::interrupt(), SignalKind::terminate()) {
                Ok(handlers) => handlers,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
        let _ = ready.send(Ok(()));
        let mut first = Some(first);
        runtime.block_on(async {
            loop {
                // As a shell reports a process that a signal ended: 128 and
                // the signal's number.
                let code = tokio::select! {
                    _ = interrupt.recv() => 130,
                    _ = terminate.recv() => 143,
                };
                if begins_stop(&begun, &mut first) {
                    continue;
                }
                force_exit(code, recorded.as_ref(), &restore);
            }
        });
    });
    caught
        .recv()
        .map_err(|_| Error::new("could not catch Ctrl-C or SIGTERM"))?
        .map_err(|error| Error::new(format!("could not catch Ctrl-C or SIGTERM: {error}")))?;
    Ok(async move {
        if stop.await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

/// A runtime of its own, with handlers of `interrupt` and `terminate` in
/// place on it, or why either cannot be had.
#[cfg(unix)]
fn handlers(
    interrupt: SignalKind,
    terminate: SignalKind,
) -> io::Result<(
    tokio::runtime::Runtime,
    tokio::signal::unix::Signal,
    tokio::signal::unix::Signal,
)> {
    use tokio::signal::unix::signal;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (interrupt, terminate) = {
        let _entered = runtime.enter();
        let interrupt = signal(interrupt)?;
        (interrupt, signal(terminate)?)
    };
    Ok((runtime, interrupt, terminate))
}

/// Whether a signal begins the stop: the first signal does, unless
/// something else has begun it, and says so on `first`. Any after it, or
/// one whose stop nothing waits for any more, forces the exit.
#[cfg(unix)]
fn begins_stop(begun: &AtomicBool, first: &mut Option<tokio::sync::oneshot::Sender<()>>) -> bool {
    !begun.swap(true, Ordering::SeqCst) && first.take().is_some_and(|first| first.send(()).is_ok())
}

/// Ends serve at once with `code`, after it removes the instance file that
/// `recorded` names and `restore` puts the terminal back. A terminal whose
/// output is stopped (Ctrl-S) holds the write that shows the cursor, so the
/// file goes first: serve exits after a moment whatever the write does.
#[cfg(unix)]
fn force_exit(code: i32, recorded: Option<&(PathBuf, i32)>, restore: &Restore) -> ! {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        std::process::exit(code);
    });
    if let Some((store, pid)) = recorded {
        let _ = instance::remove(store, *pid);
    }
    // Held until the exit: keys taken after this found nothing to put back
    // would be left raw, so `arm` waits here until serve is gone.
    let mut slot = restore.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(restore) = slot.take() {
        restore();
    }
    std::process::exit(code);
}

#[cfg(not(unix))]
fn signals(
    _: Arc<AtomicBool>,
    _: Restore,
    _: Option<(PathBuf, i32)>,
) -> Result<impl Future<Output = ()>, Error> {
    Ok(async {
        let _ = tokio::signal::ctrl_c().await;
    })
}

/// Reads the store every [`TICK`] until serve stops, so that the
/// notification feed and the event streams follow it with no page asking,
/// and on a terminal redraws the status screen. Gives the screen back.
async fn tick(app: Arc<App>, mut painter: Option<Painter>) -> Option<Painter> {
    let mut stopping = app.stopping.clone();
    let mut ticks = tokio::time::interval(TICK);
    // A read that waited on the store, or a frame that waited on the
    // terminal, is not followed by the ticks it missed, one after another.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            _ = stopping.changed() => return painter,
        }
        painter = step(&app, painter).await;
    }
}

/// Reads the store and, on a terminal, draws a frame, on a blocking thread:
/// the read can wait on the store's lock, and the frame on a terminal whose
/// output is stopped (Ctrl-S), and neither may hold the one thread that
/// serves the inbox. Gives the screen back, unless the frame panicked; the
/// painter has then put the terminal back as it dropped.
async fn step(app: &Arc<App>, painter: Option<Painter>) -> Option<Painter> {
    let app = Arc::clone(app);
    #[cfg(unix)]
    {
        tokio::task::spawn_blocking(move || {
            let read = app.watch.read().map_err(|error| error.to_string());
            painter.map(|mut painter| {
                painter.paint(&app, read);
                painter
            })
        })
        .await
        .ok()
        .flatten()
    }
    // Off unix, the frame goes to stdout, whose lock the command holds on
    // this thread, so it is drawn here.
    #[cfg(not(unix))]
    {
        let reading = Arc::clone(&app);
        let read = tokio::task::spawn_blocking(move || {
            reading.watch.read().map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|read| read);
        painter.map(|mut painter| {
            painter.paint(&app, read);
            painter
        })
    }
}

/// Draws the status screen.
struct Painter {
    store: String,
    counts: Counts,
    since: Instant,
    keys: bool,
    color: bool,
    screen: Screen,
}

impl Painter {
    /// Takes the terminal on stdout for the screen: the cursor is hidden and
    /// lines do not wrap until the painter drops.
    fn start(store: String, started: Time, keys: bool) -> Painter {
        show(Screen::START);
        Painter {
            store,
            counts: Counts::new(started),
            since: Instant::now(),
            keys,
            color: std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty()),
            screen: Screen::default(),
        }
    }

    fn paint(&mut self, app: &App, read: Result<Arc<watch::Snapshot>, String>) {
        let Ok(now) = output::now() else { return };
        let view = View {
            store: &self.store,
            url: &app.url,
            stats: read.map(|snapshot| self.counts.of(&snapshot)),
            uptime: self.since.elapsed(),
            requests: app.requests.load(Ordering::Relaxed),
            notified: app.watch.notified(),
            log: app.log.last(),
            keys: self.keys,
            color: self.color,
            now,
        };
        let lines = screen::render(&view);
        let _ = self.screen.draw(&mut term::Out, lines, term::rows());
    }
}

impl Drop for Painter {
    /// Puts the cursor and line wrapping back, however the screen ends: a
    /// frame that panicked drops its painter too.
    fn drop(&mut self) {
        show(Screen::END);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    use cases_core::{Append, Store};
    use cases_store_sqlite::Writer;
    use tokio::sync::oneshot;

    use super::*;

    /// A server run in the test's process, with timeouts of the test's own,
    /// on a thread and a runtime of its own.
    struct Running {
        addr: String,
        app: Arc<App>,
        stopping: channel::Sender<bool>,
        stop: Option<oneshot::Sender<()>>,
        done: Option<JoinHandle<usize>>,
    }

    impl Running {
        /// Serves a store holding one fyi case, which it gives.
        fn start(timeouts: Timeouts) -> (Running, String, tempfile::TempDir) {
            let dir = tempfile::tempdir().unwrap();
            let db = Arc::new(Db::new(dir.path().join("cases.db")));
            let open = Append {
                author: "agent".into(),
                event: "open".into(),
                data: r#"{"kind":"fyi","urgency":"today","title":"Heads up"}"#.into(),
            };
            let (key, _) = db.create(&open, &Writer::new(None)).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (stopping, stopped) = channel::channel(false);
            let log = Arc::new(Log::new(false, 0));
            let url = format!("http://127.0.0.1:{port}/");
            let app = Arc::new(App {
                watch: Watch::new(
                    Arc::clone(&db),
                    "boot".into(),
                    url.clone(),
                    Arc::clone(&log),
                )
                .unwrap(),
                db,
                writer: Writer::new(None),
                hosts: app::hosts("127.0.0.1", port),
                proxied: Vec::new(),
                logins: Vec::new(),
                store_tokens: Tokens::default(),
                leases: crate::dial::Leases::new(crate::dial::LEASE),
                csrf: "token".into(),
                link: url.clone(),
                url,
                log,
                stopping: stopped,
                requests: AtomicU64::new(0),
                timeouts,
            });
            let (stop, stopped) = oneshot::channel::<()>();
            let (ready, started) = mpsc::channel();
            let serving = Arc::clone(&app);
            let on_stop = stopping.clone();
            let done = thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async move {
                    listener.set_nonblocking(true).unwrap();
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    ready.send(()).unwrap();
                    let router = app::router(Arc::clone(&serving));
                    http::serve(
                        listener,
                        router,
                        timeouts,
                        async { stopped.await.unwrap_or(()) },
                        move || {
                            on_stop.send_replace(true);
                        },
                    )
                    .await
                })
            });
            started.recv().unwrap();
            let running = Running {
                addr: format!("127.0.0.1:{port}"),
                app,
                stopping,
                stop: Some(stop),
                done: Some(done),
            };
            (running, key.to_string(), dir)
        }

        /// Stops the server, and gives how many connections the stop
        /// closed.
        fn stop(&mut self) -> usize {
            let _ = self.stop.take().unwrap().send(());
            self.done.take().unwrap().join().unwrap()
        }

        /// Connects, and sends `text`.
        fn send(&self, text: &str) -> TcpStream {
            let mut stream = TcpStream::connect(&self.addr).unwrap();
            std::io::Write::write_all(&mut stream, text.as_bytes()).unwrap();
            stream
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            if self.stop.is_some() {
                self.stop();
            }
        }
    }

    /// A write of the case `id` whose body stops a few bytes in.
    fn stalled(addr: &str, id: &str) -> String {
        format!(
            "POST /v1/cases/{id}/answer HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
             X-Cases-CSRF: token\r\nContent-Length: 100\r\n\r\n{{\"revision\""
        )
    }

    /// Reads `stream` until the server closes it, which it must within
    /// `within`, and gives what came.
    fn closed_within(mut stream: TcpStream, within: Duration) -> String {
        stream.set_read_timeout(Some(within)).unwrap();
        let mut text = Vec::new();
        let started = Instant::now();
        let read = stream.read_to_end(&mut text);
        assert!(
            read.is_ok() || started.elapsed() < within,
            "the connection is still open after {within:?}"
        );
        String::from_utf8_lossy(&text).into_owned()
    }

    #[test]
    fn the_no_open_flag_wins_over_the_config_file() {
        assert!(!no_open(None, None));
        assert!(no_open(None, Some("true")));
        assert!(!no_open(None, Some("false")));
        assert!(no_open(Some(true), Some("false")));
        assert!(!no_open(Some(false), Some("true")));
    }

    const SHORT: Timeouts = Timeouts {
        read: Duration::from_millis(200),
        write: Duration::from_secs(10),
        grace: Duration::from_secs(5),
    };

    /// A write whose body stalls is answered and dropped at the read
    /// timeout, and so is a request whose head never ends (IN-11).
    #[test]
    fn a_stalled_request_is_dropped_at_the_read_timeout() {
        let (server, id, _dir) = Running::start(SHORT);
        let answer = closed_within(
            server.send(&stalled(&server.addr, &id)),
            Duration::from_secs(3),
        );
        assert!(answer.starts_with("HTTP/1.1 400 "), "{answer}");
        let head = server.send("GET /v1 HTTP/1.1\r\nHost: ");
        closed_within(head, Duration::from_secs(3));
        assert_eq!(
            server
                .app
                .db
                .rows(&id.parse().unwrap())
                .unwrap()
                .unwrap()
                .len(),
            1
        );
    }

    /// The read timeout bounds reading a request, not an event stream that
    /// outlives it.
    #[test]
    fn a_stream_outlives_the_read_timeout() {
        let (server, _id, _dir) = Running::start(SHORT);
        let version = server.app.watch.read().unwrap().version.clone();
        let mut stream = server.send(&format!(
            "GET /v1/changes?after={version} HTTP/1.1\r\nHost: {}\r\n\r\n",
            server.addr
        ));
        thread::sleep(Duration::from_millis(600));
        let open = Append {
            author: "agent".into(),
            event: "open".into(),
            data: r#"{"kind":"fyi","urgency":"today","title":"Later"}"#.into(),
        };
        server.app.db.create(&open, &Writer::new(None)).unwrap();
        let moved = server.app.watch.read().unwrap().version.clone();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut text = String::new();
        let mut buf = [0; 1024];
        while !text.contains(&format!("data: {moved}")) {
            let n = stream.read(&mut buf).expect("the stream is still open");
            assert!(n > 0, "the stream ended: {text}");
            text.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }

    /// A stop waits the grace period for requests in flight, then closes
    /// their connections, and says how many it closed (SV-02).
    #[test]
    fn a_stop_closes_what_the_grace_period_leaves() {
        let (mut server, id, _dir) = Running::start(Timeouts {
            read: Duration::from_secs(60),
            write: Duration::from_secs(10),
            grace: Duration::from_millis(200),
        });
        let stalled = server.send(&stalled(&server.addr, &id));
        // A request answered and closed before the stop is not one the stop
        // closed.
        let answered = server.send(&format!(
            "GET /v1 HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            server.addr
        ));
        closed_within(answered, Duration::from_secs(3));
        thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        assert_eq!(server.stop(), 1);
        assert!(started.elapsed() < Duration::from_secs(2));
        closed_within(stalled, Duration::from_secs(3));
    }

    /// Once serve is stopping, a stream opened ends at once, with the head
    /// that makes a browser try again rather than give up (IN-06).
    #[test]
    fn a_stream_opened_while_serve_stops_ends_at_once() {
        let (server, _id, _dir) = Running::start(SHORT);
        server.stopping.send_replace(true);
        let stream = server.send(&format!(
            "GET /v1/changes HTTP/1.1\r\nHost: {}\r\n\r\n",
            server.addr
        ));
        let text = closed_within(stream, Duration::from_secs(3));
        assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
        assert!(
            text.contains("content-type: text/event-stream\r\n"),
            "{text}"
        );
        assert!(
            text.ends_with("0\r\n\r\n"),
            "the stream did not end cleanly: {text}"
        );
    }

    /// The first signal begins the stop, and says so; any after it forces
    /// the exit.
    #[cfg(unix)]
    #[test]
    fn the_first_signal_begins_the_stop_and_the_next_forces_it() {
        let begun = AtomicBool::new(false);
        let (sender, mut stop) = oneshot::channel();
        let mut first = Some(sender);
        assert!(begins_stop(&begun, &mut first));
        assert!(begun.load(Ordering::SeqCst));
        assert!(first.is_none());
        assert_eq!(stop.try_recv(), Ok(()));
        assert!(!begins_stop(&begun, &mut first), "a second signal");
        assert!(begun.load(Ordering::SeqCst));
    }

    /// A signal that comes once something else, `q` or a replaced binary,
    /// has begun the stop forces the exit, and leaves `first` alone.
    #[cfg(unix)]
    #[test]
    fn a_signal_after_another_stop_forces_the_exit() {
        let begun = AtomicBool::new(true);
        let (sender, mut stop) = oneshot::channel::<()>();
        let mut first = Some(sender);
        assert!(!begins_stop(&begun, &mut first));
        assert!(first.is_some(), "the signal took the first stop");
        assert_eq!(stop.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    }

    /// A first signal whose stop nothing waits for any more forces the
    /// exit, since nothing else would end serve.
    #[cfg(unix)]
    #[test]
    fn a_first_signal_no_one_waits_for_forces_the_exit() {
        let begun = AtomicBool::new(false);
        let (sender, stop) = oneshot::channel::<()>();
        drop(stop);
        let mut first = Some(sender);
        assert!(!begins_stop(&begun, &mut first));
        assert!(begun.load(Ordering::SeqCst));
        assert!(first.is_none());
    }

    /// The forced restore's slot stays held while the keys are taken, so a
    /// forced stop in between waits, and finds the restore in place.
    #[test]
    fn a_forced_stop_waits_while_the_keys_are_taken() {
        let forced: Restore = Arc::default();
        let raw = arm(&forced, false, || {
            assert!(forced.try_lock().is_err(), "the slot was free");
            None
        });
        assert!(raw.is_none());
        assert!(forced.lock().unwrap().is_some(), "no restore installed");
    }

    /// A signal that cannot be caught says why.
    #[cfg(unix)]
    #[test]
    fn handlers_give_the_cause_of_a_signal_they_cannot_catch() {
        let kill = SignalKind::from_raw(9);
        let error = handlers(kill, SignalKind::terminate()).err().unwrap();
        assert!(error.to_string().contains("Refusing"), "{error}");
    }

    /// Where an instance file goes, in a directory of the test's own: the
    /// guard is given its path, so that no test reads or writes the state
    /// directory.
    struct Record {
        path: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Record {
        fn new() -> Record {
            let dir = tempfile::tempdir().unwrap();
            Record {
                path: dir.path().join("serve-cases-db-0.json"),
                _dir: dir,
            }
        }

        /// Records a serve on `/data/cases.db` with `pid` at `addr`.
        fn write(&self, pid: i32, addr: &str) {
            let info = Info {
                pid,
                url: format!("http://{addr}/"),
                link: String::new(),
                addr: addr.to_owned(),
                store: "/data/cases.db".into(),
                started_at: "2026-11-10T12:00:00Z".into(),
                version: "0.0.0".into(),
            };
            std::fs::write(&self.path, serde_json::to_string(&info).unwrap()).unwrap();
        }
    }

    fn this_pid() -> i32 {
        i32::try_from(std::process::id()).unwrap()
    }

    /// A serve recorded whose process is there and whose address takes a
    /// connection stops a second one, which names the store, the address and
    /// the process (SV-02).
    #[test]
    fn a_serve_running_on_the_store_refuses_another() {
        let record = Record::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        record.write(this_pid(), &addr);
        let error = refuse_if_running_at(&record.path).unwrap_err();
        assert_eq!(error.status(), crate::error::Status::Failure);
        assert_eq!(
            error.to_string(),
            format!(
                "cases serve is already running for /data/cases.db at http://{addr}/ (pid {})",
                this_pid()
            )
        );
    }

    /// With no serve recorded, serve starts.
    #[test]
    fn a_store_with_no_serve_recorded_is_free() {
        let record = Record::new();
        assert!(instance::running_at(&record.path).unwrap().is_none());
        refuse_if_running_at(&record.path).unwrap();
    }

    /// A record left by a serve that stopped without removing it does not
    /// stop serve: its process is gone, or not a process at all, or its
    /// address takes no connection, as when the process id went to another
    /// process.
    #[test]
    fn a_stale_record_does_not_refuse() {
        let record = Record::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap().to_string();
        // Above any pid_max, so no process has it.
        for pid in [0, -1, i32::MAX] {
            record.write(pid, &live);
            refuse_if_running_at(&record.path).unwrap();
        }
        // Port 0 takes no connection.
        record.write(this_pid(), "127.0.0.1:0");
        refuse_if_running_at(&record.path).unwrap();
        record.write(this_pid(), &live);
        assert!(
            refuse_if_running_at(&record.path).is_err(),
            "the check above"
        );
    }

    /// A file that does not read as an instance file, or cannot be read at
    /// all, counts as no serve running, and serve starts.
    #[test]
    fn a_record_that_does_not_read_does_not_refuse() {
        let record = Record::new();
        for text in ["", "not an instance file", r#"{"pid": 1}"#] {
            std::fs::write(&record.path, text).unwrap();
            assert!(instance::running_at(&record.path).is_err(), "{text:?}");
            refuse_if_running_at(&record.path).unwrap();
        }
        std::fs::remove_file(&record.path).unwrap();
        std::fs::create_dir(&record.path).unwrap();
        assert!(instance::running_at(&record.path).is_err(), "a directory");
        refuse_if_running_at(&record.path).unwrap();
    }

    /// A context whose config file holds `text`.
    fn ctx_with(dir: &Path, text: &str) -> Ctx {
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let config = config::load(Some(path.to_str().unwrap()));
        assert!(config.error.is_none(), "{:?}", config.error);
        let store = dir.join("cases.db");
        Ctx {
            store: store::AnyStore::Local(Db::new(&store)),
            target: Target::Local { path: store },
            config,
            store_flag: None,
            out: io::stdout().lock(),
        }
    }

    /// A tokens file with `lines`, private to its owner.
    fn tokens_file(dir: &Path, lines: &str) -> PathBuf {
        let path = dir.join("store-tokens");
        std::fs::write(&path, lines).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    /// With no `store-tokens-file`, or a blank one, serve takes no token.
    #[test]
    fn no_store_tokens_file_is_no_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let token = "a".repeat(32);
        tokens_file(dir.path(), &token);
        for text in [
            "",
            "store-tokens-file = \"\"\n",
            "store-tokens-file = \"  \"\n",
        ] {
            let tokens = store_tokens(&ctx_with(dir.path(), text)).unwrap();
            assert!(!tokens.has(token.as_bytes()), "{text:?}");
            assert_eq!(format!("{tokens:?}"), "Tokens(0 kept)", "{text:?}");
        }
    }

    /// serve takes the tokens of the file `store-tokens-file` names, with
    /// the space around the name trimmed.
    #[test]
    fn a_store_tokens_file_gives_its_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = ("a".repeat(32), "b".repeat(40));
        let path = tokens_file(dir.path(), &format!("# serve's tokens\n{one}\n\n{two}\n"));
        let text = format!("store-tokens-file = \" {} \"\n", path.display());
        let tokens = store_tokens(&ctx_with(dir.path(), &text)).unwrap();
        assert!(tokens.has(one.as_bytes()));
        assert!(tokens.has(two.as_bytes()));
        assert!(!tokens.has("c".repeat(32).as_bytes()));
        assert_eq!(format!("{tokens:?}"), "Tokens(2 kept)");
    }

    /// A `store-tokens-file` that cannot be read, holds no token or holds a
    /// line that is not one stops serve, with an error that names the key
    /// and the file and never the token.
    #[test]
    fn a_store_tokens_file_that_does_not_read_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let refusal = |path: &Path| {
            let text = format!("store-tokens-file = \"{}\"\n", path.display());
            store_tokens(&ctx_with(dir.path(), &text)).unwrap_err()
        };
        let error = refusal(&missing);
        assert_eq!(error.status(), crate::error::Status::Failure);
        assert!(
            error.to_string().starts_with(&format!(
                "the config file's store-tokens-file: cannot read {}: ",
                missing.display()
            )),
            "{error}"
        );

        let empty = tokens_file(dir.path(), "# none yet\n\n");
        let error = refusal(&empty);
        assert_eq!(error.status(), crate::error::Status::Failure);
        assert_eq!(
            error.to_string(),
            format!(
                "the config file's store-tokens-file: {} holds no token; put one on a line of \
                 its own, such as openssl rand -hex 32 gives",
                empty.display()
            )
        );

        let short = tokens_file(dir.path(), &format!("{}\nshort-secret\n", "a".repeat(32)));
        let error = refusal(&short);
        assert_eq!(error.status(), crate::error::Status::Failure);
        assert!(
            error.to_string().starts_with(&format!(
                "the config file's store-tokens-file: line 2 of {}: ",
                short.display()
            )),
            "{error}"
        );
        assert!(!error.to_string().contains("short-secret"), "{error}");
    }
}
