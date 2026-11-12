//! `cases tui`: the inbox in the terminal (TU-01 to TU-09). The list of
//! cases beside the case, under the web inbox's filters, read again from a
//! local store about once a second, and from a remote or forwarded one when
//! its held poll says it changed (TU-04). The reads and writes run on a
//! thread of their own, so that a slow store never holds up the keys or the
//! screen. `cases tui ID` shows that one case, and ends once it is no longer
//! open (TU-11).

mod actions;
mod app;
mod compose;
mod editor;
mod field;
mod follow;
mod form;
mod herdr;
mod kitty;
mod markdown;
mod reader;
mod safe;
mod select;
pub(crate) mod stats;
mod view;
mod wrap;

use std::io::{self, IsTerminal, Write as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cases_core::{Kind, State};
use cases_store_sqlite::{Db, Key};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, Event, KeyCode, KeyModifiers,
    KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;

use crate::commands::write;
use crate::ctx::{self, Ctx};
use crate::error::{Error, Status};
use crate::store::{AnyStore, Target};

use app::{App, Newline, Sending};
use follow::Follow;
use form::Act;
use herdr::Herdr;
use reader::{Reader, Snapshot, Write};

/// The arguments of `cases tui`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Show only this case, by its id or any part of it that names one
    /// case, and exit once it is no longer open: 0 when the human answered
    /// or parked it, 3 when it was withdrawn, 2 when the human quit without
    /// answering. It must be open. For an agent that puts one case in front
    /// of the human in a terminal of its own.
    id: Option<String>,

    /// With an id: print the id, state and revision as JSON on the way out,
    /// as answer --json does.
    #[arg(short, long, requires = "id")]
    json: bool,
}

/// How often a local store is read again (TU-04).
const READ_EVERY: Duration = Duration::from_secs(1);

/// How often a remote or forwarded store that could not be read is tried
/// again. Once read, it is followed by its held poll (TU-01, TU-04).
const RETRY_EVERY: Duration = Duration::from_secs(2);

/// The longest the loop waits for a key, so that a signal is seen soon.
const TICK: Duration = Duration::from_millis(100);

/// The most moves of the pointer taken as one before the screen is drawn.
const MOVES: usize = 64;

/// Runs the inbox until the human quits or a signal stops it; or, given an
/// id, the one case until it leaves open (TU-11).
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    // Before the terminal is checked or taken, so that a wrong id says so.
    let only = args
        .id
        .as_deref()
        .map(|id| answerable(ctx, id))
        .transpose()?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::new(
            "cases tui needs a terminal: stdin and stdout must both be one",
        ));
    }
    // What the screen calls the store, and the --store that opens it again.
    let (shown, again) = match &ctx.target {
        Target::Local { path } => {
            let path = path.display().to_string();
            (home_relative(&path), path)
        }
        Target::Remote { server, store } => (
            format!("{store} on {}", server.host()),
            format!("{server}/{}", store.id),
        ),
        Target::Server { server } => (server.to_string(), server.to_string()),
        Target::Forward { forward } => (format!("the store on {forward}"), forward.to_string()),
    };
    let mut app = match only {
        Some(key) => App::one(shown, key),
        None => App::new(shown),
    };
    let mode = stats::Mode::of(ctx.config.get("tui-stats"))?;
    app.strip.shown = mode.shown();
    // The store is read and written on a thread of its own: the screen never
    // waits on the store (TU-04). The human's events are written as `answer`
    // writes them (HU-06).
    let writer = ctx.human(None);
    let store = Arc::new(std::mem::replace(
        &mut ctx.store,
        AnyStore::Local(Db::new(PathBuf::new())),
    ));
    let follow = store.follows().then(Follow::start).flatten().map(Arc::new);
    let mut reader = Reader::start(Arc::clone(&store), writer, follow.clone())?;
    // The first read, waited for before the terminal is taken, so that a
    // store from a newer cases says so on the main screen.
    let first = reader.ask();
    let never = AtomicBool::new(false);
    let read = match reader.wait(first, &never, TICK) {
        Some(snapshot) => apply(&mut app, snapshot)?,
        None => false,
    };
    let stop = signals()?;
    // One case is not the inbox: its count is not herdr's to report.
    let mut herdr = only.is_none().then(|| Herdr::from_env(&again)).flatten();

    // Whether the strip is an image: asked of the terminal before anything
    // reads keys, and only when the config file leaves it to the terminal.
    app.strip.image = mode.asks() && only.is_none() && kitty::supported();
    IMAGES.store(app.strip.image, Ordering::SeqCst);
    // Whether the terminal speaks the kitty keyboard protocol, which tells
    // shift+enter from enter: asked as the strip's images are, before
    // anything reads keys. A terminal that does not answer within crossterm's
    // wait is taken as one that does not.
    let keyboard = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    KEYBOARD.store(keyboard, Ordering::SeqCst);
    if keyboard {
        app.newline = Newline::ShiftEnter;
    }
    let mut terminal = init()?;
    let mut run = Run {
        reader: &mut reader,
        follow: follow.as_deref(),
        read,
        interrupted: false,
        stop: &stop,
        herdr: herdr.as_mut(),
    };
    let result = event_loop(&mut terminal, &mut app, &mut run);
    let interrupted = run.interrupted;
    restore();
    if let Some(herdr) = herdr {
        herdr.release();
    }
    result?;
    // Not waiting for a read that runs: it ends with the process. q waits
    // for a write being sent, so that an answer sent just before it is not
    // lost on a slow store; Ctrl-C or a signal does not, and the write may
    // land or not. On one case, the exit says how the case went.
    let landed = if app.quit && !interrupted && !stop.load(Ordering::Relaxed) {
        land(&mut reader, &mut app, &stop)?
    } else {
        None
    };
    if let Some(key) = only {
        return ended(ctx, &store, &app, key, args.json);
    }
    // The screen is gone: that the write was recorded is said here instead.
    if let (Some(true), Some(said)) = (landed, app.said_done()) {
        eprintln!("{said}");
    }
    Ok(())
}

/// How long q waits for a write being sent to land: as long as a request
/// to a remote or forwarded store may take.
const LAND_WITHIN: Duration = Duration::from_secs(30);

/// Waits, with the terminal back, for a write being sent and the read after
/// it, so that a human who sent an answer and quit at once does not lose it
/// on a slow store, and on one case's screen is not told to the agent as
/// one who quit without answering (TU-11). Ctrl-C, a signal again now, or
/// [`LAND_WITHIN`], ends the wait.
///
/// Gives whether the write landed, or `None` when there was none to wait
/// for. When it was refused, or may land yet, it says so, and gives back
/// what the human typed into it, so that it can be copied and sent again.
fn land(reader: &mut Reader, app: &mut App, stop: &AtomicBool) -> Result<Option<bool>, Error> {
    let (Some(line), typed) = (app.sending_line(), app.sending_typed()) else {
        return Ok(None);
    };
    eprintln!("{line}: waiting for it to land; Ctrl-C stops waiting");
    let mut landed = None;
    if let Some(snapshot) = reader.settle(stop, TICK, LAND_WITHIN) {
        // A store that can no longer be read, as when the link dies, ends
        // the program, but not before what was typed is given back.
        if let Err(error) = apply(app, snapshot) {
            eprintln!("{line}: it may land yet");
            give_back(typed);
            return Err(error);
        }
        if let Some(wrote) = reader.wrote() {
            landed = Some(wrote.landed());
            app.wrote(wrote);
        }
    }
    match landed {
        Some(true) => return Ok(Some(true)),
        Some(false) => {
            if let Some(said) = app.said() {
                eprintln!("{said}");
            }
        }
        None => eprintln!("{line}: it may land yet"),
    }
    give_back(typed);
    Ok(Some(false))
}

/// Prints what the human typed into a write that did not land, indented,
/// so that it can be copied and sent again.
fn give_back(typed: Vec<String>) {
    if typed.is_empty() {
        return;
    }
    eprintln!("what you typed:");
    for part in typed {
        for text in part.lines() {
            eprintln!("    {text}");
        }
        eprintln!();
    }
}

/// The case `id` names, which the human must be able to answer: an open
/// case that is not a message. Exit 3 otherwise, as `answer` exits.
fn answerable(ctx: &Ctx, id: &str) -> Result<Key, Error> {
    let key = ctx.find_case(id)?;
    let case = ctx.case(key)?;
    if Act::Answer.fits(&case) {
        return Ok(key);
    }
    let why = if case.kind() == Kind::Message {
        format!("{key} is a message: an agent picks it up")
    } else {
        format!("{key} is {}: nothing to answer", case.state())
    };
    Err(Error::with_status(Status::Transition, why))
}

/// How the one case's screen ended (TU-11). Answered or parked, here or
/// elsewhere: the case as a write prints it, exit 0. Withdrawn: the same,
/// then exit 3. Gone from the store: exit 1. Still open, because the human
/// quit or a signal stopped it: exit 2, as a wait that gives up, saying so
/// when a write being sent may land yet.
fn ended(ctx: &mut Ctx, db: &AnyStore, app: &App, key: Key, json: bool) -> Result<(), Error> {
    // Read once more: the case may have been answered since the last read,
    // just before the human quit.
    let now = ctx::case_of(db, key).ok();
    let case = now
        .as_ref()
        .or_else(|| app.cases.iter().find(|(k, _)| *k == key).map(|(_, c)| c));
    match case {
        None => Err(Error::new(format!("{key} is gone from the store"))),
        Some(case) if case.state() == State::Open => {
            let why = if app.sending.is_some() {
                "the screen closed while a write was being sent, which may land yet"
            } else {
                "the screen closed without an answer"
            };
            Err(Error::with_status(
                Status::Timeout,
                format!("{key} is still open: {why}"),
            ))
        }
        Some(case) => {
            write::print(ctx, key, case, json)?;
            if case.state() == State::Withdrawn {
                return Err(Error::with_status(
                    Status::Transition,
                    format!("{key} was withdrawn, not answered"),
                ));
            }
            Ok(())
        }
    }
}

/// Takes the terminal as ratatui does, raw on the alternate screen, and has
/// it report the mouse (TU-12). A panic puts it back as [`restore`] does.
fn init() -> io::Result<DefaultTerminal> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    static IMAGE_HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        // ratatui's hook, set by `try_init`, wraps this one: it leaves the
        // alternate screen, then this stops the mouse reports.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
            hook(info);
        }));
    });
    TAKEN.store(true, Ordering::SeqCst);
    let terminal = ratatui::try_init()?;
    IMAGE_HOOK.call_once(|| {
        // Over ratatui's hook, so that the strip's image is deleted, and
        // the keys given back, while the alternate screen, which holds
        // them, is still up.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            unshow();
            pop_keyboard();
            hook(info);
        }));
    });
    // Clicks, the wheel and every move of the pointer, in SGR form: the
    // moves light what the pointer is over. The loop takes the moves that
    // queue up as one, so that the screen is not drawn again for each.
    // And pastes bracketed, so that a new line in one is not enter, which
    // sends (TU-12).
    if let Err(error) = io::stdout()
        .write_all(b"\x1b[?1000h\x1b[?1003h\x1b[?1006h\x1b[?2004h")
        .and_then(|()| io::stdout().flush())
    {
        restore();
        return Err(error);
    }
    // Keys that tell shift+enter, a new line, from enter, which sends,
    // where the terminal speaks the kitty keyboard protocol (TU-12).
    if KEYBOARD.load(Ordering::SeqCst)
        && let Err(error) = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
    {
        restore();
        return Err(error);
    }
    Ok(terminal)
}

/// Gives the terminal its keys back as they were, when [`init`] asked for
/// the kitty keyboard protocol's.
fn pop_keyboard() {
    if KEYBOARD.load(Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
}

/// Puts the terminal back as it was: the strip's image deleted, no mouse
/// reports, then as ratatui restores it.
fn restore() {
    unshow();
    pop_keyboard();
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    TAKEN.store(false, Ordering::SeqCst);
}

/// Deletes the strip's image, when the strip draws one (TU-13).
fn unshow() {
    if IMAGES.load(Ordering::SeqCst) {
        let mut out = io::stdout();
        let _ = out
            .write_all(&kitty::delete(kitty::id()))
            .and_then(|()| out.flush());
    }
}

/// Whether the strip draws a kitty image, so that putting the terminal
/// back deletes it (TU-13).
static IMAGES: AtomicBool = AtomicBool::new(false);

/// Whether the screen asks for the kitty keyboard protocol's keys, so that
/// putting the terminal back gives the old ones back (TU-12).
static KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Whether the screen has the terminal: from [`init`] until [`restore`],
/// so that a second signal puts it back, and leaves alone the terminal of
/// an editor the screen gave it to.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// A flag that SIGINT or SIGTERM sets, so that the loop ends and the
/// terminal is put back rather than left raw. In raw mode Ctrl-C is a key,
/// not a signal; the flag covers a `kill`. A second one exits at once, in
/// case the loop is stuck, with the terminal put back first if the screen
/// has it.
///
/// SIGHUP exits at once, even where it was ignored: the terminal is gone, so
/// there is nothing to put back, and the loop cannot be relied on to see a
/// flag. Once the terminal hangs up, crossterm's `event::poll` reads nothing
/// in a loop that never returns, at full CPU (crossterm-rs/crossterm#793).
fn signals() -> Result<Arc<AtomicBool>, Error> {
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        use signal_hook::flag;
        use signal_hook::iterator::Signals;
        flag::register_conditional_shutdown(SIGHUP, 128 + SIGHUP, Arc::new(AtomicBool::new(true)))?;
        for signal in [SIGINT, SIGTERM] {
            // The first stops the loop at once, from the handler.
            flag::register(signal, Arc::clone(&stop))?;
        }
        // The second is seen on a thread of its own, as the handler could
        // not put the terminal back safely. Two of the same signal sent
        // together can reach it as one, which the loop then ends on; only
        // a stuck loop needs one more.
        let mut caught = Signals::new([SIGINT, SIGTERM])?;
        std::thread::spawn(move || {
            for (seen, signal) in (1..).zip(caught.forever()) {
                if seen >= 2 {
                    if TAKEN.load(Ordering::SeqCst) {
                        put_back();
                    }
                    // Not process::exit, which would flush what the loop
                    // left in stdout's buffer over the main screen.
                    signal_hook::low_level::exit(128 + signal);
                }
            }
        });
    }
    Ok(stop)
}

/// Puts the terminal back from the signal thread, as [`restore`] does,
/// while the loop may be stuck holding stdout: written to its file
/// descriptor, past the lock. The keys as they were, no mouse reports,
/// the main screen, the cursor shown, then not raw.
#[cfg(unix)]
fn put_back() {
    if IMAGES.load(Ordering::SeqCst) {
        let _ = rustix::io::write(io::stdout(), &kitty::delete(kitty::id()));
    }
    if KEYBOARD.load(Ordering::SeqCst) {
        let _ = rustix::io::write(io::stdout(), b"\x1b[<1u");
    }
    let _ = rustix::io::write(
        io::stdout(),
        b"\x1b[?2004l\x1b[?1006l\x1b[?1003l\x1b[?1000l\x1b[?1049l\x1b[?25h",
    );
    let _ = ratatui::crossterm::terminal::disable_raw_mode();
}

/// What the loop runs with.
struct Run<'a> {
    /// The thread that reads and writes the store.
    reader: &'a mut Reader,
    /// The thread that holds a remote or forwarded store's poll.
    follow: Option<&'a Follow>,
    /// Whether the last read of the store worked.
    read: bool,
    /// Whether Ctrl-C quit, which waits for nothing: not even a write
    /// being sent (TU-11).
    interrupted: bool,
    stop: &'a AtomicBool,
    herdr: Option<&'a mut Herdr>,
}

impl Run<'_> {
    /// Takes a read into the app, then how a write went if this is the
    /// read after it, so that the app is told with the case as the write
    /// left it.
    fn apply(&mut self, app: &mut App, snapshot: Snapshot) -> Result<(), Error> {
        self.read = apply(app, snapshot)?;
        if let Some(wrote) = self.reader.wrote() {
            app.wrote(wrote);
        }
        Ok(())
    }

    /// Hands `write` to the reads' thread; the app waits for how it went.
    fn send(&mut self, app: &mut App, write: Write, sending: Sending) {
        self.reader.write(write);
        // What the toasts said was before this write: what it does is said
        // after it, alone, so that q tells it on the way out.
        app.toasts.clear();
        app.sending = Some(sending);
    }

    /// How long after a read the store is read again: a local store about
    /// once a second, and a remote or forwarded one only when its last read
    /// failed, as its held poll wakes the screen otherwise.
    fn every(&self) -> Option<Duration> {
        match (self.follow, self.read) {
            (None, _) => Some(READ_EVERY),
            (Some(_), false) => Some(RETRY_EVERY),
            (Some(_), true) => None,
        }
    }
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, run: &mut Run) -> Result<(), Error> {
    let mut last_read = Instant::now();
    // The strip's image as the terminal has it (TU-13).
    let mut image = kitty::Shown::default();
    // Whether the screen is drawn on this turn: not after only moves of
    // the pointer that leave it over the same thing.
    let mut draw = true;
    while !app.quit && !run.stop.load(Ordering::Relaxed) && !app.left() {
        if let Some(snapshot) = run.reader.take() {
            run.apply(app, snapshot)?;
            last_read = Instant::now();
            draw = true;
            // The one case may have left with this read.
            continue;
        }
        if let Some(herdr) = run.herdr.as_deref_mut() {
            herdr.report(app.waiting());
        }
        draw |= app.expire_toasts(Instant::now());
        if draw {
            terminal.draw(|frame| view::draw(frame, app))?;
            if app.strip.image {
                show(&mut image, app)?;
            }
        }
        // While a read runs, a look for it every tick.
        let wait = match run.every() {
            _ if run.reader.busy() => TICK,
            Some(every) => every.saturating_sub(last_read.elapsed()).min(TICK),
            None => TICK,
        };
        let mut next = if event::poll(wait)? {
            Some(event::read()?)
        } else {
            None
        };
        // The pointer's moves and drags that queue up are taken as one:
        // only where it ends up is lit. A few at most, so that a pointer
        // that keeps moving does not hold up the screen and the reads.
        let mut moves = 0;
        let mut lit = false;
        while let Some(Event::Mouse(mouse)) = next
            && matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Drag(_))
            && moves < MOVES
        {
            moves += 1;
            lit |= app.mouse(mouse);
            next = if event::poll(Duration::ZERO)? {
                Some(event::read()?)
            } else {
                None
            };
        }
        draw = next.is_some() || moves == 0 || lit;
        if let Some(event) = next {
            match event {
                Event::Key(key) => {
                    app.key(key);
                    run.interrupted = app.quit
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c');
                }
                Event::Mouse(mouse) => {
                    app.mouse(mouse);
                }
                Event::Paste(text) => app.paste(&text),
                // The screen is drawn afresh: the image is deleted, so that
                // none is left where the strip was, and sent again.
                Event::Resize(..) => {
                    let mut out = io::stdout();
                    out.write_all(&image.reset(kitty::id()))?;
                    out.flush()?;
                }
                _ => {}
            }
        }
        // What the mouse selected, to the clipboard (TU-12).
        if let Some(text) = app.copied.take() {
            let mut out = io::stdout();
            out.write_all(&select::osc52(&text))?;
            out.flush()?;
        }
        if let Some(record) = app.pending_message.take() {
            run.send(
                app,
                Write::Create(record.clone()),
                Sending::Message(Box::new(record)),
            );
        }
        if let Some(part) = app.edit_message.take() {
            restore();
            let edited = app
                .compose
                .as_mut()
                .map(|compose| editor::edit(compose.field(part).text()));
            *terminal = init()?;
            terminal.clear()?;
            image.forget();
            match edited {
                Some(Ok(Some(text))) => {
                    if let Some(compose) = &mut app.compose {
                        compose.field(part).set(text);
                    }
                }
                Some(Ok(None)) | None => {}
                Some(Err(why)) => app.flash(&format!("the editor: {why}")),
            }
        }
        if let Some(send) = app.pending.take() {
            let write = Write::Append {
                key: send.key,
                record: send.record.clone(),
                revision: send.revision,
            };
            run.send(app, write, Sending::Event(Box::new(send)));
        }
        if let Some(which) = app.edit.take() {
            restore();
            let edited = app
                .form
                .as_mut()
                .map(|form| editor::edit(form.field(which).text()));
            *terminal = init()?;
            terminal.clear()?;
            image.forget();
            match edited {
                Some(Ok(Some(text))) => {
                    if let Some(form) = &mut app.form {
                        form.field(which).set(text);
                    }
                }
                Some(Ok(None)) | None => {}
                Some(Err(why)) => app.flash(&format!("the editor: {why}")),
            }
        }
        // One read at a time: a wake while one runs is left for after it.
        if !run.reader.busy()
            && (run.follow.is_some_and(Follow::woke)
                || run
                    .every()
                    .is_some_and(|every| last_read.elapsed() >= every))
        {
            run.reader.ask();
        }
    }
    Ok(())
}

/// Brings the terminal's image of the strip in step with the frame just
/// drawn: sent, put, moved or deleted (TU-13).
fn show(image: &mut kitty::Shown, app: &App) -> io::Result<()> {
    let want = app
        .strip
        .area
        .zip(app.strip.stats.as_ref())
        .map(|(area, stats)| (stats.opened(), stats.answered(), area));
    let escapes = image.sync(kitty::id(), want, kitty::cell());
    if escapes.is_empty() {
        return Ok(());
    }
    let mut out = io::stdout();
    out.write_all(&escapes)?;
    out.flush()
}

/// Takes a read into the app, and gives whether it read. A store that
/// cannot be read is a line on the screen, tried again on the next read; a
/// store from a newer cases, here or on a forward's far side, or a clock
/// out of range, ends the program (TU-01).
fn apply(app: &mut App, snapshot: Snapshot) -> Result<bool, Error> {
    match snapshot {
        Snapshot::Read {
            cases,
            unfolded,
            actors,
            listeners,
            now,
            stats,
        } => {
            app.listeners = listeners;
            app.strip.stats = Some(stats);
            app.update(cases, unfolded, actors, now);
            Ok(true)
        }
        Snapshot::Unreadable { why, now } => {
            app.unreadable(&why, now);
            Ok(false)
        }
        Snapshot::Fatal(error) => Err(error),
    }
}

/// Reads the store into the app here, as the reads' thread does, and gives
/// whether it read.
#[cfg(test)]
fn read(
    app: &mut App,
    poller: &mut crate::store::Poller<'_>,
    db: &AnyStore,
) -> Result<bool, Error> {
    apply(app, reader::snapshot(poller, db))
}

/// `~/x` for a path under the home directory.
fn home_relative(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.into(),
    }
}

#[cfg(test)]
mod tests;
