//! The TUI against a store of its own: what each filter lists, what the
//! keys do, and what the screen shows, drawn to a buffer.

use cases_core::{
    Amend, Answer, Append, Author, Close, Note, Open, Park, Pickup, Record, Resume, State,
    Store as _,
};
use cases_store_sqlite::{Db, Key, Writer};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tempfile::TempDir;

use super::app::{App, Filter, Focus, Group, Refusal, Sending, Verb};
use super::field::Field;
use super::form::{Form, RowPick, Which};
use super::reader::{Write, Wrote};
use super::{read, view};
use crate::store::AnyStore;

struct Store {
    dir: TempDir,
    db: AnyStore,
}

impl Store {
    fn new() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        Store {
            dir,
            db: AnyStore::Local(db),
        }
    }

    fn open(&self, kind: &str, urgency: &str, title: &str, with: impl FnOnce(&mut Open)) -> Key {
        let mut open = Open::default();
        kind.clone_into(&mut open.kind);
        urgency.clone_into(&mut open.urgency);
        title.clone_into(&mut open.title);
        "worker".clone_into(&mut open.worker);
        with(&mut open);
        let append = Append::new(Author::Agent, &Record::Open(open));
        self.db
            .create(&append, &Writer::new(Some("worker")))
            .unwrap()
            .0
    }

    fn human(&self, key: Key, record: &Record) {
        let append = Append::new(Author::Human, record);
        self.db
            .append(&key, &append, &Writer::new(Some("ryan")), None)
            .unwrap();
    }

    fn agent(&self, key: Key, record: &Record) {
        let append = Append::new(Author::Agent, record);
        self.db
            .append(&key, &append, &Writer::new(Some("worker")), None)
            .unwrap();
    }

    fn answer(&self, key: Key, with: impl FnOnce(&mut Answer)) {
        let mut answer = Answer::default();
        with(&mut answer);
        self.human(key, &Record::Answer(answer));
    }

    fn pickup(&self, key: Key) {
        let mut pickup = Pickup::default();
        "worker".clone_into(&mut pickup.by);
        self.agent(key, &Record::Pickup(pickup));
    }

    fn close(&self, key: Key, outcome: &str) {
        let mut close = Close::default();
        outcome.clone_into(&mut close.outcome);
        self.agent(key, &Record::Close(close));
    }

    /// An app that has read the store once.
    fn app(&self) -> App {
        let mut app = App::new("cases.db".into());
        read(&mut app, &mut self.db.poller(), &self.db).unwrap();
        app
    }
}

fn decision(open: &mut Open) {
    open.options = vec!["Pin it".into(), "Float it".into()];
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn titles(app: &App, filter: Filter) -> Vec<String> {
    app.shown(filter)
        .iter()
        .map(|(_, case)| case.title().to_string())
        .collect()
}

/// The screen as text, one line per row.
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn each_filter_lists_its_cases_in_the_web_inboxs_order() {
    let store = Store::new();
    store.open("fyi", "whenever", "later", |_| {});
    store.open("fyi", "blocking", "now", |_| {});
    let parked = store.open("stuck", "today", "parked", |_| {});
    store.human(parked, &Record::Park(Park::default()));
    let answered = store.open("decision", "today", "answered", decision);
    store.answer(answered, |a| a.choice = 1);
    let picked = store.open("decision", "today", "picked up", decision);
    store.answer(picked, |a| a.choice = 2);
    store.pickup(picked);
    let closed = store.open("fyi", "today", "closed", |_| {});
    store.answer(closed, |a| a.ack = true);
    store.pickup(closed);
    store.close(closed, "Done.");

    let app = store.app();
    // Inbox: urgency, then open order; parked is listed.
    assert_eq!(titles(&app, Filter::Inbox), ["now", "parked", "later"]);
    // In flight: latest first.
    assert_eq!(titles(&app, Filter::Flight), ["picked up", "answered"]);
    assert_eq!(titles(&app, Filter::Done), ["closed"]);
    assert_eq!(
        titles(&app, Filter::All),
        ["now", "parked", "later", "picked up", "answered", "closed"]
    );
    // Parked does not wait on the human.
    assert_eq!(app.waiting(), (2, 1));
}

#[test]
fn a_message_sent_heads_in_flight() {
    let store = Store::new();
    let answered = store.open("decision", "today", "answered", decision);
    store.answer(answered, |a| a.choice = 1);
    let mut message = Open::default();
    "message".clone_into(&mut message.kind);
    "whenever".clone_into(&mut message.urgency);
    "do the thing".clone_into(&mut message.title);
    "worker".clone_into(&mut message.worker);
    let append = Append::new(Author::Human, &Record::Open(message));
    store
        .db
        .create(&append, &Writer::new(Some("ryan")))
        .unwrap();
    let app = store.app();
    assert_eq!(titles(&app, Filter::Flight), ["do the thing", "answered"]);
    assert_eq!(app.waiting(), (0, 0));
}

#[test]
fn keys_move_filter_focus_and_quit() {
    let store = Store::new();
    store.open("fyi", "today", "first", |_| {});
    store.open("fyi", "today", "second", |_| {});
    let mut app = store.app();
    let selected = |app: &App| app.selected_case().map(|(_, c)| c.title().to_string());
    assert_eq!(selected(&app).as_deref(), Some("first"));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(selected(&app).as_deref(), Some("second"));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(
        selected(&app).as_deref(),
        Some("second"),
        "moved past the end"
    );
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(selected(&app).as_deref(), Some("first"));

    // ] and [ step through the filters, round from the end.
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.filter, Filter::Flight);
    press(&mut app, KeyCode::Char('['));
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.filter, Filter::Briefings);
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.filter, Filter::All);
    press(&mut app, KeyCode::Char(']'));
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.filter, Filter::Inbox);

    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.focus, Focus::Case);
    // In the case, j scrolls rather than moving.
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(selected(&app).as_deref(), Some("first"));
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help);
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.help && !app.quit, "a key closes the help first");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::List);
    assert!(!app.quit);
    press(&mut app, KeyCode::Esc);
    assert!(app.quit);

    let mut app = store.app();
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.quit);
}

#[test]
fn keys_scroll_the_case_and_jump_through_the_list() {
    let store = Store::new();
    for title in ["first", "second", "third"] {
        store.open("fyi", "today", title, |_| {});
    }
    let mut app = store.app();
    let selected = |app: &App| app.selected_case().unwrap().1.title().to_string();
    // A key let go of is not a key pressed.
    app.key(KeyEvent::new_with_kind(
        KeyCode::Char('j'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));
    assert_eq!(selected(&app), "first");

    // From the list, the case scrolls by the page.
    let steps: &[(KeyCode, KeyModifiers, u16)] = &[
        (KeyCode::Char('J'), KeyModifiers::NONE, 10),
        (KeyCode::Char(' '), KeyModifiers::NONE, 20),
        (KeyCode::PageDown, KeyModifiers::NONE, 30),
        (KeyCode::Char('K'), KeyModifiers::NONE, 20),
        (KeyCode::PageUp, KeyModifiers::NONE, 10),
        (KeyCode::Char('d'), KeyModifiers::CONTROL, 20),
        (KeyCode::Char('u'), KeyModifiers::CONTROL, 10),
        (KeyCode::Char('u'), KeyModifiers::CONTROL, 0),
        (KeyCode::Char('K'), KeyModifiers::NONE, 0),
        (KeyCode::Char('J'), KeyModifiers::NONE, 10),
        // Without Ctrl, d and u do not scroll.
        (KeyCode::Char('d'), KeyModifiers::NONE, 10),
        (KeyCode::Char('u'), KeyModifiers::NONE, 10),
    ];
    for &(code, modifiers, scroll) in steps {
        app.key(KeyEvent::new(code, modifiers));
        assert_eq!(app.scroll, scroll, "{code:?} {modifiers:?}");
    }
    assert_eq!(selected(&app), "first");
    // A move to another case scrolls it back to its top; staying on the
    // same case keeps the scroll.
    let moves = [
        (KeyCode::Char('G'), "third", 0),
        (KeyCode::Char('k'), "second", 0),
        (KeyCode::Up, "first", 0),
        (KeyCode::Up, "first", 10),
        (KeyCode::End, "third", 0),
        (KeyCode::Home, "first", 0),
        (KeyCode::Down, "second", 0),
    ];
    for (code, title, scroll) in moves {
        app.scroll = 0;
        press(&mut app, KeyCode::Char('J'));
        press(&mut app, code);
        assert_eq!(selected(&app), title, "{code:?}");
        assert_eq!(app.scroll, scroll, "{code:?}");
    }

    // In the case, the arrows scroll by the line.
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.focus, Focus::Case);
    let lines = [
        (KeyCode::Char('j'), 1),
        (KeyCode::Down, 2),
        (KeyCode::Char('k'), 1),
        (KeyCode::Up, 0),
        (KeyCode::Up, 0),
    ];
    for (code, scroll) in lines {
        press(&mut app, code);
        assert_eq!(app.scroll, scroll, "{code:?}");
    }
    assert_eq!(selected(&app), "second");
    for (code, focus) in [
        (KeyCode::Char('h'), Focus::List),
        (KeyCode::Right, Focus::Case),
        (KeyCode::Left, Focus::List),
    ] {
        press(&mut app, code);
        assert_eq!(app.focus, focus, "{code:?}");
    }
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.filter, Filter::Briefings);
    assert!(!app.quit);

    // With nothing to select, Enter does not open the case pane.
    let mut app = Store::new().app();
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::List);
}

#[test]
fn a_form_or_a_message_takes_the_keys_but_the_case_still_scrolls() {
    let store = Store::new();
    store.open("fyi", "today", "first", |_| {});
    store.open("fyi", "today", "second", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    assert!(app.form.is_some());
    // a scrolls to the answer, at the foot of the case: from the top.
    assert_eq!(app.scroll, u16::MAX);
    app.scroll = 0;
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.scroll, 10);
    // j is the form's, and does not scroll or move.
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.scroll, 10);
    assert_eq!(app.selected_case().unwrap().1.title(), "first");
    // Out of its fields, q quits.
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);

    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.focus, Focus::Case);
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll, 10);
    press(&mut app, KeyCode::PageUp);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.scroll, 0);
    // In a field, q is typed.
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert_eq!(app.compose.as_ref().unwrap().to.text(), "q");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.confirm_quit && !app.quit);
    press(&mut app, KeyCode::Char('Y'));
    assert!(app.quit);

    // Any key closes the help, c too; Ctrl-C quits from it.
    let mut app = store.app();
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Char('c'));
    assert!(!app.help && !app.quit);
    press(&mut app, KeyCode::Char('?'));
    ctrl(&mut app, 'c');
    assert!(app.quit);
}

#[test]
fn a_case_that_leaves_the_list_gives_its_place_to_the_next() {
    let store = Store::new();
    store.open("fyi", "today", "first", |_| {});
    let second = store.open("fyi", "today", "second", |_| {});
    store.open("fyi", "today", "third", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('j'));
    // A new case above it keeps the same case selected.
    store.open("fyi", "blocking", "urgent", |_| {});
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(app.selected, Some(second));
    // Answered elsewhere, it leaves the inbox: the case below takes its
    // place.
    store.answer(second, |a| a.ack = true);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    let (_, case) = app.selected_case().unwrap();
    assert_eq!(case.title(), "third");
}

#[test]
fn the_screen_shows_the_list_and_the_case_side_by_side() {
    let store = Store::new();
    store.open("decision", "blocking", "Pin bun?", |open| {
        decision(open);
        open.recommend = 1;
        "Bun **1.2** broke the lockfile.\n\n- one\n- two".clone_into(&mut open.body);
        open.labels = vec!["deps".into()];
    });
    store.open("fyi", "whenever", "Renovate ran", |_| {});
    let mut app = store.app();
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("2 waiting · 1 blocking"), "{text}");
    assert!(text.contains("Inbox 2"), "{text}");
    assert!(text.contains("● Pin bun?"), "{text}");
    assert!(text.contains("Renovate ran"), "{text}");
    assert!(
        text.contains("blocking · decision · worker #deps"),
        "{text}"
    );
    assert!(text.contains("Bun 1.2 broke the lockfile."), "{text}");
    assert!(text.contains("• one"), "{text}");
    assert!(text.contains(" 1  Pin it  ★ recommended"), "{text}");
    assert!(text.contains("agent open  by worker"), "{text}");
}

#[test]
fn a_narrow_terminal_shows_one_pane_at_a_time() {
    let store = Store::new();
    store.open("question", "today", "Which name?", |open| {
        "The body text.".clone_into(&mut open.body);
    });
    let mut app = store.app();
    let list = screen(&mut app, 80, 20);
    assert!(
        list.contains("Which name?") && !list.contains("The body text."),
        "{list}"
    );
    press(&mut app, KeyCode::Enter);
    let case = screen(&mut app, 80, 20);
    assert!(case.contains("The body text."), "{case}");
}

#[test]
fn control_characters_in_a_case_are_shown_not_sent() {
    let store = Store::new();
    store.open("fyi", "today", "evil \x1b]0;owned\x07 title", |open| {
        "body \x1b[31mred\x1b[0m".clone_into(&mut open.body);
    });
    let mut app = store.app();
    let text = screen(&mut app, 120, 20);
    assert!(!text.contains('\x1b') && !text.contains('\x07'), "{text:?}");
    assert!(text.contains("evil ␛]0;owned␇ title"), "{text}");
    assert!(text.contains("body ␛[31mred␛[0m"), "{text}");
}

#[test]
fn inbox_zero_says_what_is_still_out() {
    let store = Store::new();
    let answered = store.open("decision", "today", "Pin or float?", decision);
    store.answer(answered, |a| a.choice = 1);
    let mut app = store.app();
    let text = screen(&mut app, 120, 20);
    assert!(text.contains("nothing is waiting on you."), "{text}");
    assert!(text.contains("nothing needs you."), "{text}");
    assert!(text.contains("STILL WITH AN AGENT"), "{text}");
    assert!(text.contains("answered   Pin or float?"), "{text}");
}

#[test]
fn a_note_after_an_answer_shows_the_follow_up_first() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin or float?", decision);
    store.answer(key, |a| {
        a.choice = 2;
        "float is fine".clone_into(&mut a.note);
    });
    store.pickup(key);
    let mut note = Note::default();
    "Float to what, **1.x**?".clone_into(&mut note.body);
    note.reopen = true;
    store.agent(key, &Record::Note(note));
    let mut app = store.app();
    let text = screen(&mut app, 120, 40);
    let follow = text
        .find("FOLLOW-UP, NEW SINCE YOUR ANSWER")
        .unwrap_or_else(|| panic!("{text}"));
    let options = text.find("YOUR ANSWER").unwrap_or_else(|| panic!("{text}"));
    assert!(follow < options, "{text}");
    assert!(text.contains("Float to what, 1.x?"), "{text}");
    assert!(text.contains("chose 2  Float it"), "{text}");
    assert!(text.contains("Float it  your last pick"), "{text}");
}

#[test]
fn a_closed_case_shows_its_answer_and_outcome() {
    let store = Store::new();
    let key = store.open("signoff", "today", "Ship the README?", |_| {});
    store.answer(key, |a| "accept".clone_into(&mut a.signoff));
    store.pickup(key);
    store.close(key, "Merged in [#12](https://example.com/12).");
    let mut app = store.app();
    show(&mut app, Filter::Done);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("✓ Ship the README?"), "{text}");
    assert!(text.contains("accepted"), "{text}");
    assert!(text.contains("ryan, just now"), "{text}");
    assert!(
        text.contains("Merged in #12 (https://example.com/12)."),
        "{text}"
    );
}

#[test]
fn a_table_in_a_body_is_drawn_aligned() {
    let store = Store::new();
    store.open("fyi", "today", "Timings", |open| {
        "| job | time |\n|---|---|\n| test | 4m |\n| mutants | 31m |".clone_into(&mut open.body);
    });
    let mut app = store.app();
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("job     │ time"), "{text}");
    assert!(text.contains("test    │ 4m"), "{text}");
    assert!(text.contains("mutants │ 31m"), "{text}");
}

#[test]
fn a_store_that_is_not_there_is_an_empty_inbox() {
    let store = Store::new();
    let mut app = store.app();
    assert!(app.cases.is_empty() && app.problem.is_none());
    assert!(
        !store.dir.path().join("cases.db").exists(),
        "reading made the store"
    );
    let text = screen(&mut app, 120, 20);
    assert!(text.contains("nothing needs you"), "{text}");
}

#[test]
fn a_store_that_cannot_be_read_is_a_line_on_the_screen() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the store should be.
    let db = AnyStore::Local(Db::new(dir.path()));
    let mut app = App::new("cases.db".into());
    read(&mut app, &mut db.poller(), &db).unwrap();
    assert!(app.problem.is_some());
    let text = screen(&mut app, 120, 20);
    assert!(
        text.contains("cannot read the store, trying again"),
        "{text}"
    );
}

/// Sends `send` to `db` as `writer`, as the reads' thread does.
fn write(db: &AnyStore, writer: &Writer, send: &super::app::Send) -> Result<(), Refusal> {
    let write = Write::Append {
        key: send.key,
        record: send.record.clone(),
        revision: send.revision,
    };
    match write.send(db, writer) {
        Wrote::Append(result) => result,
        other @ Wrote::Create(_) => panic!("{other:?}"),
    }
}

/// Sends what the app asked for, as the loop and the reads' thread do: the
/// write, a read of the store, and the app told how it went.
fn send(app: &mut App, store: &Store) {
    let send = app.pending.take().expect("a write to send");
    let wrote = Wrote::Append(write(&store.db, &Writer::new(Some("ryan")), &send));
    app.sending = Some(Sending::Event(Box::new(send)));
    read(app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(wrote);
    assert!(app.sending.is_none(), "still sending");
}

/// Steps the filters with ] to `filter`, as the human does (TU-12).
fn show(app: &mut App, filter: Filter) {
    for _ in Filter::ALL {
        if app.filter == filter {
            return;
        }
        press(app, KeyCode::Char(']'));
    }
    assert_eq!(app.filter, filter);
}

/// Types `text`: a new line by shift+enter, which a field takes as one,
/// and a tab by tab.
fn typing(app: &mut App, text: &str) {
    for c in text.chars() {
        match c {
            '\n' => app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            '\t' => press(app, KeyCode::Tab),
            c => press(app, KeyCode::Char(c)),
        }
    }
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn case(store: &Store, key: Key) -> cases_core::Case {
    cases_core::Case::fold(&store.db.rows(&key).unwrap().unwrap()).unwrap()
}

fn flash(app: &App) -> String {
    app.said().unwrap_or_default().to_string()
}

#[test]
fn a_decision_is_answered_with_its_option_and_note_then_the_next_case_shows() {
    let store = Store::new();
    let key = store.open("decision", "blocking", "Pin or float?", decision);
    let next = store.open("fyi", "today", "next one", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none(), "sent with no choice");
    assert!(flash(&app).contains("choose an option"), "{}", flash(&app));
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('n'));
    typing(&mut app, "  float is fine ");
    press(&mut app, KeyCode::Esc);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("YOUR ANSWER"), "{text}");
    assert!(text.contains("● 2  Float it"), "{text}");
    assert!(text.contains("float is fine"), "{text}");
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);

    let answered = case(&store, key);
    let answer = answered.answer().unwrap();
    assert_eq!((answer.choice, answer.note.as_str()), (2, "float is fine"));
    assert_eq!(answered.state(), State::Answered);
    assert!(app.form.is_none());
    assert_eq!(app.selected, Some(next));
    assert_eq!(flash(&app), format!("recorded answer on {key}"));
}

#[test]
fn a_reply_is_typed_in_place_and_trimmed() {
    let store = Store::new();
    let key = store.open("question", "today", "Sign-in line?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    // Typing starts in the reply; q is a letter there, not quit.
    typing(&mut app, "  Quick, plain.\nNothing else.  ");
    assert!(!app.quit);
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).answer().unwrap().text,
        "Quick, plain.\nNothing else."
    );
}

#[test]
fn a_signoff_asking_for_changes_needs_a_note() {
    let store = Store::new();
    let key = store.open("signoff", "today", "README?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none());
    assert!(flash(&app).contains("needs a note"), "{}", flash(&app));
    press(&mut app, KeyCode::Char('n'));
    typing(&mut app, "shorter intro");
    ctrl(&mut app, 's');
    send(&mut app, &store);
    let answer = case(&store, key).answer().cloned().unwrap();
    assert_eq!(
        (answer.signoff.as_str(), answer.note.as_str()),
        ("changes", "shorter intro")
    );
}

#[test]
fn an_fyi_is_acknowledged_and_a_drop_asks_first() {
    let store = Store::new();
    let fyi = store.open("fyi", "today", "FYI", |_| {});
    let dropped = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert!(case(&store, fyi).answer().unwrap().ack);

    assert_eq!(app.selected, Some(dropped));
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    assert!(screen(&mut app, 120, 40).contains("The agent stops the work."));
    press(&mut app, KeyCode::Char('n'));
    assert!(app.pending.is_none(), "a drop went without a yes");
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert!(case(&store, dropped).answer().unwrap().drop);
}

#[test]
fn a_stuck_case_parks_with_a_note_and_resumes() {
    let store = Store::new();
    let key = store.open("stuck", "today", "Stuck on herdr", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('p'));
    typing(&mut app, "after the release notes");
    ctrl(&mut app, 's');
    send(&mut app, &store);
    let parked = case(&store, key);
    assert_eq!(parked.state(), State::Parked);
    assert_eq!(parked.park().unwrap().note, "after the release notes");

    press(&mut app, KeyCode::Char('r'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Open);

    // Only a stuck case parks, and only a parked one resumes.
    let fyi = store.open("fyi", "whenever", "an fyi", |_| {});
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.selected = Some(fyi);
    press(&mut app, KeyCode::Char('p'));
    assert!(app.form.is_none() && flash(&app).contains("only a stuck case parks"));
    press(&mut app, KeyCode::Char('r'));
    assert!(app.pending.is_none() && flash(&app).contains("only a parked case resumes"));
}

#[test]
fn an_answered_case_is_withdrawn_after_a_yes() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    store.answer(key, |a| a.choice = 1);
    let mut app = store.app();
    show(&mut app, Filter::Flight);
    press(&mut app, KeyCode::Char('w'));
    typing(&mut app, "session died");
    ctrl(&mut app, 's');
    assert!(app.pending.is_none(), "a withdraw went without a yes");
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    let withdrawn = case(&store, key);
    assert_eq!(withdrawn.state(), State::Withdrawn);
}

/// A withdraw form drawn on an answered decision, with a reason typed.
fn withdrawing() -> (Store, Key, App) {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    store.answer(key, |a| a.choice = 1);
    let mut app = store.app();
    show(&mut app, Filter::Flight);
    press(&mut app, KeyCode::Char('w'));
    typing(&mut app, "session died");
    (store, key, app)
}

#[test]
fn a_pickup_between_two_reads_leaves_the_withdraw_able_to_send() {
    let (store, key, mut app) = withdrawing();
    store.pickup(key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Withdrawn);
}

#[test]
fn a_reopen_and_new_answer_between_two_reads_makes_the_withdraw_stale() {
    let (store, key, mut app) = withdrawing();
    // Reopened, answered again elsewhere and picked up: the case is in
    // another state the withdraw fits, by way of the one it was drawn in.
    note(&store, key, "Are you sure?", true);
    store.answer(key, |a| a.choice = 2);
    store.pickup(key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::PickedUp,
        "a withdraw was recorded past an answer it never saw"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
}

#[test]
fn an_amend_that_changes_the_question_makes_the_send_stale() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    let mut amend = Amend::default();
    "Pin, now that 1.3 is out?".clone_into(&mut amend.body);
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::Open,
        "a stale answer was written"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    // The form keeps the choice, and now sends against the case as read.
    assert_eq!(app.form.as_ref().and_then(|f| f.choice), Some(1));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().choice, 1);
}

#[test]
fn a_note_on_the_case_leaves_the_form_able_to_send() {
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "cases");
    let mut note = Note::default();
    "Lower case, please.".clone_into(&mut note.body);
    store.agent(key, &Record::Note(note));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(app.form.as_ref().map(|f| f.text.text()), Some("cases"));
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().text, "cases");
}

/// An agent's note on `key`, reopening it if `reopen`.
fn note(store: &Store, key: Key, body: &str, reopen: bool) {
    let mut note = Note::default();
    body.clone_into(&mut note.body);
    note.reopen = reopen;
    store.agent(key, &Record::Note(note));
}

#[test]
fn several_notes_in_one_read_leave_the_form_able_to_send() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    note(&store, key, "One thing.", false);
    note(&store, key, "And another.", false);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().choice, 1);
}

#[test]
fn an_answer_and_reopen_between_two_reads_makes_the_send_stale() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    // Answered elsewhere, then reopened, and the TUI reads only after both:
    // the case is open again with the same amend seq.
    store.answer(key, |answer| answer.choice = 2);
    note(
        &store,
        key,
        "Please reconsider after the other answer",
        true,
    );
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::Open,
        "the pre-cycle draft was recorded"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    // The case as it is now has been shown: the form keeps the choice and
    // sends against it.
    assert_eq!(app.form.as_ref().and_then(|f| f.choice), Some(1));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().choice, 1);
}

#[test]
fn a_row_the_fold_skipped_makes_the_send_stale() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    skip_row(&store, key);
    note(&store, key, "Still open.", false);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(case(&store, key).problems().len(), 1);
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::Open,
        "a draft was recorded past a row it never saw"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
}

/// A row on `key` that does not read, as a hand edit could leave it.
fn skip_row(store: &Store, key: Key) {
    let seq = case(store, key).last_seq() + 1;
    let conn = rusqlite::Connection::open(store.dir.path().join("cases.db")).unwrap();
    conn.execute(
        "INSERT INTO events (case_key, seq, author, event, at, data) VALUES (?1, ?2, 'human', 'answer', '', ?3)",
        rusqlite::params![
            i64::try_from(key.get()).unwrap(),
            i64::try_from(seq).unwrap(),
            b"{".as_slice()
        ],
    )
    .unwrap();
}

#[test]
fn a_park_and_resume_between_two_reads_makes_the_send_stale() {
    let store = Store::new();
    let key = store.open("stuck", "today", "Stuck on herdr", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "use the old socket");
    // Parked and resumed elsewhere: open again, by way of parked.
    store.human(key, &Record::Park(Park::default()));
    store.human(key, &Record::Resume(Resume::default()));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert!(
        case(&store, key).answer().is_none(),
        "an answer was recorded past a park it never saw"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).answer().unwrap().text,
        "use the old socket"
    );
}

#[test]
fn a_case_answered_elsewhere_closes_the_form() {
    let store = Store::new();
    let key = store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    store.answer(key, |a| a.ack = true);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.form.is_none());
    assert!(flash(&app).contains("is now answered"), "{}", flash(&app));
}

#[test]
fn quitting_with_typed_text_asks_first() {
    let store = Store::new();
    store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "draft");
    ctrl(&mut app, 'c');
    assert!(app.confirm_quit && !app.quit);
    press(&mut app, KeyCode::Char('n'));
    assert!(!app.confirm_quit && !app.quit);
    ctrl(&mut app, 'c');
    press(&mut app, KeyCode::Char('y'));
    assert!(app.quit);
    // With nothing typed, q quits at once.
    let mut app = store.app();
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn an_approval_takes_a_verdict_and_a_note_for_each_row() {
    let store = Store::new();
    let row = |id: &str| {
        let mut row = cases_core::ApprovalRow::default();
        id.clone_into(&mut row.id);
        format!("Run {id}").clone_into(&mut row.label);
        "true".clone_into(&mut row.script);
        "https://example.com".clone_into(&mut row.link);
        row
    };
    let key = store.open("approval", "today", "Cut the release?", |open| {
        open.rows = vec![row("tag"), row("brew"), row("post")];
    });
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('y'));
    // y moved to the next row: hold it, with a note.
    press(&mut app, KeyCode::Char('h'));
    press(&mut app, KeyCode::Char('k'));
    press(&mut app, KeyCode::Char('r'));
    typing(&mut app, "after the tap's CI is green ");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none(), "sent with a row left");
    assert!(
        flash(&app).contains("a verdict for every row"),
        "{}",
        flash(&app)
    );
    press(&mut app, KeyCode::Char('j'));
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("▸ post"), "{text}");
    assert!(text.contains("hold"), "{text}");
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    let rows = case(&store, key).answer().unwrap().rows.clone();
    let got: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.verdict.as_str(), r.note.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("tag", "approve", ""),
            ("brew", "hold", "after the tap's CI is green"),
            ("post", "reject", "")
        ]
    );
}

#[test]
fn a_row_the_human_carries_out_is_theirs_and_its_approve_reads_done() {
    let store = Store::new();
    let row = |id: &str, by: &str, script: &str| {
        let mut row = cases_core::ApprovalRow::default();
        id.clone_into(&mut row.id);
        format!("Run {id}").clone_into(&mut row.label);
        script.clone_into(&mut row.script);
        by.clone_into(&mut row.by);
        "https://example.com".clone_into(&mut row.link);
        row
    };
    let key = store.open("approval", "today", "Move the host?", |open| {
        open.rows = vec![
            row("dns", "human", "Point the name at the new host"),
            row("tag", "", "git tag v1"),
        ];
    });
    let mut app = store.app();
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("Run dns  you"), "{text}");
    assert!(!text.contains("Run tag  you"), "{text}");
    // The human's step is said in words, with no shell prompt.
    assert!(!text.contains("$ Point the name"), "{text}");
    assert!(text.contains("Point the name at the new host"), "{text}");
    assert!(text.contains("$ git tag v1"), "{text}");
    press(&mut app, KeyCode::Char('a'));
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("y done"), "{text}");
    assert!(text.contains("x won't do"), "{text}");
    press(&mut app, KeyCode::Char('y'));
    // On the agent's row, the keys read as before.
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("y approve"), "{text}");
    assert!(text.contains("x reject"), "{text}");
    press(&mut app, KeyCode::Char('x'));
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("● done  ○ hold  ○ won't do"), "{text}");
    assert!(text.contains("○ approve  ○ hold  ● reject"), "{text}");
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    let rows = case(&store, key).answer().unwrap().rows.clone();
    let got: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.verdict.as_str()))
        .collect();
    assert_eq!(got, [("dns", "approve"), ("tag", "reject")]);
    show(&mut app, Filter::Flight);
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("dns done"), "{text}");
    assert!(text.contains("tag reject"), "{text}");
}

/// An approval with the rows tag, brew and post, its form open with a
/// verdict on each, and `with` done to the form; then an amend adds the row
/// ship and the send is refused as stale.
fn amended_approval(with: impl FnOnce(&mut Form)) -> (Store, Key, App) {
    let store = Store::new();
    let row = |id: &str| {
        let mut row = cases_core::ApprovalRow::default();
        id.clone_into(&mut row.id);
        format!("Run {id}").clone_into(&mut row.label);
        "true".clone_into(&mut row.script);
        "https://example.com".clone_into(&mut row.link);
        row
    };
    let key = store.open("approval", "today", "Cut the release?", |open| {
        open.rows = vec![row("tag"), row("brew"), row("post")];
    });
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('y'));
    press(&mut app, KeyCode::Char('h'));
    press(&mut app, KeyCode::Char('k'));
    press(&mut app, KeyCode::Char('r'));
    typing(&mut app, "after CI");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('x'));
    with(app.form.as_mut().unwrap());
    let mut amend = Amend::default();
    amend.rows = vec![row("ship")];
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Open);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    (store, key, app)
}

/// The form's rows as (id, verdict, note).
fn picks(app: &App) -> Vec<(String, Option<&'static str>, String)> {
    let form = app.form.as_ref().expect("the form stays open");
    form.rows
        .iter()
        .map(|r| (r.id.clone(), r.verdict, r.note.text().to_string()))
        .collect()
}

#[test]
fn a_row_an_amend_adds_shows_in_the_form_after_a_stale_send() {
    let (store, key, mut app) = amended_approval(|_| {});
    assert_eq!(
        picks(&app),
        [
            ("tag".into(), Some("approve"), String::new()),
            ("brew".into(), Some("hold"), "after CI".into()),
            ("post".into(), Some("reject"), String::new()),
            ("ship".into(), None, String::new()),
        ]
    );
    // No row is one the human carries out.
    let human: Vec<bool> = app
        .form
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.human)
        .collect();
    assert_eq!(human, [false; 4]);
    let text = screen(&mut app, 120, 50);
    assert!(text.contains("Run ship"), "{text}");
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none(), "sent with the new row left");
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('y'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    let rows = case(&store, key).answer().unwrap().rows.clone();
    let got: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.verdict.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("tag", "approve"),
            ("brew", "hold"),
            ("post", "reject"),
            ("ship", "approve")
        ]
    );
}

#[test]
fn a_row_the_case_no_longer_has_leaves_the_form_after_a_stale_send() {
    let (_store, _key, app) = amended_approval(|form| {
        let gone = RowPick {
            id: "gone".into(),
            verdict: Some("approve"),
            note: Field::default(),
            human: false,
        };
        form.rows.insert(0, gone);
        // The keys are on post, and brew's note is open.
        form.row = 3;
        form.editing = Some(Which::Row(2));
    });
    let ids: Vec<String> = picks(&app).into_iter().map(|(id, ..)| id).collect();
    assert_eq!(ids, ["tag", "brew", "post", "ship"]);
    let form = app.form.as_ref().unwrap();
    assert_eq!((form.row, form.editing), (2, Some(Which::Row(1))));
}

#[test]
fn a_renamed_row_takes_no_verdict_from_its_old_name_after_a_stale_send() {
    let (_store, _key, app) = amended_approval(|form| {
        "posted".clone_into(&mut form.rows[2].id);
        form.row = 2;
        form.editing = Some(Which::Row(2));
    });
    // The note that was open was posted's: it closes.
    let form = app.form.as_ref().unwrap();
    assert_eq!((form.row, form.editing), (2, None));
    assert_eq!(
        picks(&app),
        [
            ("tag".into(), Some("approve"), String::new()),
            ("brew".into(), Some("hold"), "after CI".into()),
            ("post".into(), None, String::new()),
            ("ship".into(), None, String::new()),
        ]
    );
}

#[test]
fn an_option_an_amend_adds_can_be_chosen_after_a_stale_send() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin or float?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    press(&mut app, KeyCode::Char('n'));
    typing(&mut app, "kept");
    press(&mut app, KeyCode::Esc);
    let mut amend = Amend::default();
    amend.options = vec!["Hold it".into()];
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    let answer = case(&store, key).answer().unwrap().clone();
    assert_eq!((answer.choice, answer.note.as_str()), (3, "kept"));
}

#[test]
fn an_option_an_amend_adds_can_be_chosen_once_it_shows() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin or float?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    let mut amend = Amend::default();
    amend.options = vec!["Hold it".into()];
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.form.as_ref().unwrap().choice, Some(3));
    // The send is still refused as stale, and goes the second time.
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().choice, 3);
}

#[test]
fn a_message_is_not_answered() {
    let store = Store::new();
    let mut message = Open::default();
    "message".clone_into(&mut message.kind);
    "today".clone_into(&mut message.urgency);
    "do it".clone_into(&mut message.title);
    "w1".clone_into(&mut message.worker);
    let append = Append::new(Author::Human, &Record::Open(message));
    store
        .db
        .create(&append, &Writer::new(Some("ryan")))
        .unwrap();
    let mut app = store.app();
    show(&mut app, Filter::Flight);
    press(&mut app, KeyCode::Char('a'));
    assert!(app.form.is_none());
    assert!(flash(&app).contains("is a message"), "{}", flash(&app));
}

/// Sends the new message the app asked for, as the loop and the reads'
/// thread do.
fn send_message(app: &mut App, store: &Store) {
    let record = app.pending_message.take().expect("a message to send");
    let wrote = Write::Create(record).send(&store.db, &Writer::new(Some("ryan")));
    app.sending = Some(Sending::Message(Box::new(Record::Open(Open::default()))));
    read(app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(wrote);
    assert!(app.sending.is_none(), "still sending");
}

#[test]
fn a_message_goes_to_a_worker_with_its_title_from_the_body() {
    let store = Store::new();
    store.open("fyi", "whenever", "seed", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.focus, Focus::Case);
    typing(&mut app, "bun-pins");
    press(&mut app, KeyCode::Enter);
    // Enter moves from who it is to, to the title, then to the body.
    press(&mut app, KeyCode::Enter);
    typing(&mut app, "## Pin bun to 1.2.3\nand rerun the lockfile.");
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("MESSAGE"), "{text}");
    assert!(text.contains("no session is listening now"), "{text}");
    ctrl(&mut app, 's');
    send_message(&mut app, &store);
    assert!(app.compose.is_none());
    assert!(
        flash(&app).contains("no session is listening for it now"),
        "{}",
        flash(&app)
    );
    let (key, sent) = app
        .cases
        .iter()
        .find(|(_, c)| c.kind() == cases_core::Kind::Message)
        .cloned()
        .unwrap();
    assert_eq!(sent.state(), State::Sent);
    assert_eq!(
        (sent.worker(), sent.title()),
        ("bun-pins", "Pin bun to 1.2.3")
    );
    assert_eq!(sent.body(), "## Pin bun to 1.2.3\nand rerun the lockfile.");
    assert!(flash(&app).contains(&key.to_string()));
}

#[test]
fn a_message_to_a_label_needs_someone_to_go_to() {
    let store = Store::new();
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('b'));
    typing(&mut app, "hello");
    ctrl(&mut app, 's');
    assert!(app.pending_message.is_none());
    assert!(flash(&app).contains("say who it is to"), "{}", flash(&app));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('t'));
    typing(&mut app, "#release");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('u'));
    press(&mut app, KeyCode::Enter);
    send_message(&mut app, &store);
    let (_, sent) = &app.cases[0];
    assert_eq!(sent.labels(), ["release"]);
    assert_eq!(sent.worker(), "");
    assert_eq!(sent.urgency(), cases_core::Urgency::Whenever);
    assert_eq!(sent.title(), "hello");
}

#[test]
fn a_sent_message_is_edited_until_picked_up_and_withdrawn() {
    let store = Store::new();
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    typing(&mut app, "w1");
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    typing(&mut app, "first draft");
    ctrl(&mut app, 's');
    send_message(&mut app, &store);
    let key = app.cases[0].0;

    show(&mut app, Filter::Flight);
    assert_eq!(app.selected, Some(key));
    press(&mut app, KeyCode::Char('e'));
    // The body is the one field; the cursor is at its end.
    typing(&mut app, ", then more");
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(case(&store, key).body(), "first draft, then more");
    assert!(app.compose.is_none());

    press(&mut app, KeyCode::Char('w'));
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Withdrawn);
    show(&mut app, Filter::All);
    app.selected = Some(key);
    press(&mut app, KeyCode::Char('e'));
    assert!(
        app.compose.is_none(),
        "a withdrawn message was opened to edit"
    );
}

#[test]
fn an_edit_closes_when_an_agent_picks_the_message_up() {
    let store = Store::new();
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    typing(&mut app, "w1\t\thello");
    ctrl(&mut app, 's');
    send_message(&mut app, &store);
    let key = app.cases[0].0;
    show(&mut app, Filter::Flight);
    press(&mut app, KeyCode::Char('e'));
    typing(&mut app, " again");
    let mut pickup = Pickup::default();
    "w1".clone_into(&mut pickup.by);
    store.agent(key, &Record::Pickup(pickup));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.compose.is_none());
    assert!(
        flash(&app).contains("the edit was not sent"),
        "{}",
        flash(&app)
    );
}

/// A message sent to w1 from the app, selected in flight.
fn sent() -> (Store, Key, App) {
    let store = Store::new();
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    typing(&mut app, "w1\t\thello");
    ctrl(&mut app, 's');
    send_message(&mut app, &store);
    let key = app.cases[0].0;
    show(&mut app, Filter::Flight);
    assert_eq!(app.selected, Some(key));
    (store, key, app)
}

/// A link added to the message `key` elsewhere: it leaves the message as it
/// was.
fn link_elsewhere(store: &Store, key: Key) {
    let mut amend = Amend::default();
    amend.links = vec!["https://example.com/pr/1".into()];
    store.human(key, &Record::Amend(amend));
}

/// The human's edit of the message `key` to `body`, made elsewhere.
fn edit_elsewhere(store: &Store, key: Key, body: &str) {
    let mut amend = Amend::default();
    body.clone_into(&mut amend.body);
    store.human(key, &Record::Amend(amend));
}

#[test]
fn a_link_on_a_sent_message_leaves_the_edit_able_to_send() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('e'));
    typing(&mut app, " again");
    link_elsewhere(&store, key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(case(&store, key).body(), "hello again");
}

#[test]
fn an_edit_made_elsewhere_makes_the_edit_stale() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('e'));
    typing(&mut app, " again");
    edit_elsewhere(&store, key, "hello from the web");
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).body(),
        "hello from the web",
        "an edit was written over one it never saw"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    assert!(
        flash(&app).contains("your edit was replaced by the message as it is now"),
        "{}",
        flash(&app)
    );
    // The edit drops what was typed and shows the message as it is now,
    // as the web inbox does: a second send has nothing to save.
    let compose = app.compose.as_ref().expect("the edit stays open");
    assert_eq!(compose.body.text(), "hello from the web");
    ctrl(&mut app, 's');
    assert!(app.pending.is_none(), "the old draft was sent again");
    assert!(flash(&app).contains("as it was"), "{}", flash(&app));
    assert_eq!(case(&store, key).body(), "hello from the web");
}

/// An edit closed while it is being sent, then refused as stale, replaced
/// nothing: the line does not say it did.
#[test]
fn an_edit_closed_while_sent_then_refused_does_not_say_it_was_replaced() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('e'));
    typing(&mut app, " again");
    edit_elsewhere(&store, key, "hello from the web");
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    app.compose = None;
    send(&mut app, &store);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    assert!(!flash(&app).contains("replaced"), "{}", flash(&app));
}

#[test]
fn an_edit_refused_as_stale_sends_against_the_message_as_it_is_now() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('e'));
    typing(&mut app, " again");
    edit_elsewhere(&store, key, "hello from the web");
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    // An edit typed on the message as it is now goes.
    typing(&mut app, ", and more");
    ctrl(&mut app, 's');
    assert!(app.pending.is_some(), "{}", flash(&app));
    send(&mut app, &store);
    assert_eq!(case(&store, key).body(), "hello from the web, and more");
}

#[test]
fn a_withdraw_refused_as_stale_leaves_an_edit_opened_meanwhile_stale() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('w'));
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    let withdraw = app.pending.take().expect("a withdraw to send");
    app.sending = Some(Sending::Event(Box::new(withdraw)));
    // While the withdraw is out, the body is edited elsewhere, and the
    // human leaves the withdraw for an edit of the message as last read.
    edit_elsewhere(&store, key, "hello from the web");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('e'));
    assert!(app.compose.is_some(), "{}", flash(&app));
    let Some(Sending::Event(withdraw)) = app.sending.take() else {
        panic!("the withdraw is not being sent");
    };
    let wrote = Wrote::Append(write(&store.db, &Writer::new(Some("ryan")), &withdraw));
    app.sending = Some(Sending::Event(withdraw));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(wrote);
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
    typing(&mut app, " again");
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).body(),
        "hello from the web",
        "an edit was written over one it never saw"
    );
}

#[test]
fn the_humans_own_edit_leaves_the_withdraw_of_a_message_able_to_send() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('w'));
    typing(&mut app, "not needed");
    edit_elsewhere(&store, key, "hello, edited");
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Withdrawn);
}

#[test]
fn a_pickup_between_two_reads_leaves_the_withdraw_of_a_message_able_to_send() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('w'));
    let mut pickup = Pickup::default();
    "w1".clone_into(&mut pickup.by);
    store.agent(key, &Record::Pickup(pickup));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Withdrawn);
}

#[test]
fn a_row_the_fold_skipped_makes_the_withdraw_of_a_message_stale() {
    let (store, key, mut app) = sent();
    press(&mut app, KeyCode::Char('w'));
    skip_row(&store, key);
    link_elsewhere(&store, key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::Sent,
        "a withdraw was recorded past a row it never saw"
    );
    assert!(
        flash(&app).contains("changed since you read it"),
        "{}",
        flash(&app)
    );
}

/// While a write is being sent, the footer says so, the keys still work,
/// and a second send waits for it; how it went is said once the read after
/// it lands (TU-04).
#[test]
fn a_send_waits_for_the_one_being_sent() {
    let store = Store::new();
    let fyi = store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    let first = app.pending.take().expect("a write to send");
    let wrote = Wrote::Append(write(&store.db, &Writer::new(Some("ryan")), &first));
    app.sending = Some(Sending::Event(Box::new(first)));
    let text = screen(&mut app, 120, 20);
    assert!(
        text.contains(&format!("⋯ sending answer on {fyi}")),
        "{text}"
    );

    // The form is still open: a second send is refused, and says why.
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none(), "a second send went");
    assert!(flash(&app).contains("wait for it"), "{}", flash(&app));
    // The keys still work: the form closes, and help opens.
    press(&mut app, KeyCode::Esc);
    assert!(app.form.is_none());
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help);
    press(&mut app, KeyCode::Esc);

    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(wrote);
    assert!(app.sending.is_none());
    assert!(
        flash(&app).contains(&format!("recorded answer on {fyi}")),
        "{}",
        flash(&app)
    );
    assert_eq!(case(&store, fyi).state(), State::Answered);
}

/// A message is not sent while a write is.
#[test]
fn a_message_waits_for_the_write_being_sent() {
    let store = Store::new();
    let fyi = store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    let first = app.pending.take().expect("a write to send");
    app.sending = Some(Sending::Event(Box::new(first)));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('m'));
    assert!(app.compose.is_some(), "no message opened");
    typing(&mut app, "w1");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    typing(&mut app, "Ship it");
    ctrl(&mut app, 's');
    assert!(app.pending_message.is_none(), "a message went");
    assert!(
        flash(&app).contains(&format!("sending answer on {fyi}: wait for it")),
        "{}",
        flash(&app)
    );
    assert!(app.compose.is_some(), "the message was lost");
}

/// Moves the selection to `key`, by j, then by k.
fn select_case(app: &mut App, key: Key) {
    for code in [KeyCode::Char('j'), KeyCode::Char('k')] {
        for _ in 0..5 {
            if app.selected == Some(key) {
                return;
            }
            press(app, code);
        }
    }
    assert_eq!(app.selected, Some(key), "not selected");
}

/// A form opened on another case while a write is sent keeps that case
/// selected once the write lands: the form is drawn only on its case, and
/// would take keys unseen (TU-04).
#[test]
fn a_write_that_lands_keeps_a_form_opened_since_on_its_case() {
    let store = Store::new();
    let a = store.open("fyi", "today", "Alpha", |_| {});
    store.open("fyi", "today", "Bravo", |_| {});
    let c = store.open("fyi", "today", "Charlie", |_| {});
    let mut app = store.app();
    select_case(&mut app, a);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    let first = app.pending.take().expect("a write to send");
    let wrote = Wrote::Append(write(&store.db, &Writer::new(Some("ryan")), &first));
    app.sending = Some(Sending::Event(Box::new(first)));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    select_case(&mut app, c);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.form.as_ref().map(|f| f.key), Some(c));

    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(wrote);
    assert_eq!(case(&store, a).state(), State::Answered);
    assert_eq!(app.selected, Some(c), "the selection moved off the form");
    assert_eq!(app.form.as_ref().map(|f| f.key), Some(c));
    assert!(
        flash(&app).contains(&format!("recorded answer on {a}")),
        "{}",
        flash(&app)
    );
}

/// What was typed in a form being sent is on its way: Ctrl-C quits at
/// once, without asking first, as it does not wait for the write (TU-11).
#[test]
fn ctrl_c_quits_at_once_while_what_was_typed_is_sent() {
    let store = Store::new();
    store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "Plain.");
    ctrl(&mut app, 's');
    let first = app.pending.take().expect("a write to send");
    app.sending = Some(Sending::Event(Box::new(first)));
    ctrl(&mut app, 'c');
    assert!(!app.confirm_quit, "asked first");
    assert!(app.quit);
}

/// A refusal other than a stale one, at the revision the case has, is said
/// as the store gives it, and the form stays open to be read again.
#[test]
fn a_refusal_that_is_not_stale_is_said_as_it_is() {
    let store = Store::new();
    let fyi = store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    // A park of a case that is not stuck, which the form never sends, at
    // the case's own revision: refused for its transition.
    let mut send = app.pending.take().expect("a write to send");
    send.record = Record::Park(Park::default());
    let result = write(&store.db, &Writer::new(Some("ryan")), &send);
    let Err(super::app::Refusal::Other(why)) = &result else {
        panic!("{result:?}");
    };
    assert!(why.contains("park"), "{why}");
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.sent(&send, result);
    let flash = flash(&app);
    assert!(flash.starts_with("nothing sent: "), "{flash}");
    assert!(!flash.contains("changed since you read it"), "{flash}");
    assert!(app.form.is_some(), "the form closed");
    assert_eq!(case(&store, fyi).state(), State::Open);
}

/// Cases that do not fold are left out of the list and counted on the rule
/// under the filters.
#[test]
fn cases_that_cannot_be_read_are_counted_on_the_rule() {
    let store = Store::new();
    store.open("fyi", "today", "Readable", |_| {});
    let conn = rusqlite::Connection::open(store.dir.path().join("cases.db")).unwrap();
    // Two cases whose open does not read, as a hand edit could leave them.
    for _ in 0..2 {
        conn.execute("INSERT INTO cases (opened_at) VALUES ('')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO events (case_key, seq, author, event, at, data) VALUES (?1, 1, 'agent', 'open', '', ?2)",
            rusqlite::params![conn.last_insert_rowid(), b"{".as_slice()],
        )
        .unwrap();
    }
    let mut app = store.app();
    assert_eq!(app.unfolded, 2);
    assert_eq!(app.cases.len(), 1);
    let text = screen(&mut app, 120, 20);
    assert!(text.contains("2 cases cannot be read"), "{text}");
}

/// Opens a briefing with a body, and `left_out` as its left-out items.
fn briefing(store: &Store, urgency: &str, title: &str, landed: bool, left_out: &[&str]) -> Key {
    store.open("briefing", urgency, title, |open| {
        "## What changed\n\nThe parser.".clone_into(&mut open.body);
        open.landed = landed;
        open.left_out = left_out.iter().map(|item| (*item).to_string()).collect();
    })
}

#[test]
fn briefings_have_a_tab_and_a_count_of_their_own() {
    let store = Store::new();
    store.open("fyi", "today", "ordinary", |_| {});
    briefing(&store, "blocking", "landed blocking", true, &[]);
    briefing(&store, "whenever", "gated whenever", false, &[]);
    briefing(&store, "today", "gated today", false, &[]);
    let answered = briefing(&store, "today", "answered", true, &[]);
    store.answer(answered, |a| "accept".clone_into(&mut a.briefing));
    let picked = briefing(&store, "today", "picked up", true, &[]);
    store.answer(picked, |a| "accept".clone_into(&mut a.briefing));
    store.pickup(picked);
    let closed = briefing(&store, "today", "closed", true, &[]);
    store.answer(closed, |a| "accept".clone_into(&mut a.briefing));
    store.pickup(closed);
    store.close(closed, "Done.");

    let mut app = store.app();
    // Briefings are in none of the filters until closed or withdrawn.
    assert_eq!(titles(&app, Filter::Inbox), ["ordinary"]);
    assert!(titles(&app, Filter::Flight).is_empty());
    assert_eq!(titles(&app, Filter::Done), ["closed"]);
    assert_eq!(titles(&app, Filter::All), ["ordinary", "closed"]);
    // Nor in the header's count, or herdr's.
    assert_eq!(app.waiting(), (1, 0));
    // Gated before landed, then urgency, then open order; the answered
    // ones, picked up or not, are folded away until f.
    assert_eq!(
        titles(&app, Filter::Briefings),
        ["gated today", "gated whenever", "landed blocking"]
    );
    show(&mut app, Filter::Briefings);
    assert_eq!(app.filter, Filter::Briefings);
    let text = screen(&mut app, 120, 30);
    assert!(text.contains(" Inbox 1 "), "{text}");
    assert!(text.contains(" Briefings 3 "), "{text}");
    assert!(text.contains("1 waiting"), "{text}");
    // One of the two is still to be picked up.
    assert!(
        text.contains("▸ 2 answered · 1 not picked up  f to show"),
        "{text}"
    );
    assert!(!text.contains(&answered.to_string()), "{text}");
    assert!(text.contains("waiting to land"), "{text}");
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected_case().unwrap().1.title(), "landed blocking");
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(
        titles(&app, Filter::Briefings),
        [
            "gated today",
            "gated whenever",
            "landed blocking",
            "picked up",
            "answered"
        ]
    );
    let text = screen(&mut app, 120, 30);
    assert!(
        text.contains("▾ 2 answered · 1 not picked up  f to fold"),
        "{text}"
    );
    // The group's line comes between the open and the answered ones, and
    // the selection stays on its case.
    let at = |needle: &str| text.find(needle).unwrap();
    let row = answered.to_string();
    assert!(at("landed blocking") < at("▾ 2 answered") && at("▾ 2 answered") < at(&row));
    assert_eq!(app.selected_case().unwrap().1.title(), "landed blocking");
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected_case().unwrap().1.title(), "answered");
    // Folding it away selects the case that took its place.
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.selected_case().unwrap().1.title(), "landed blocking");
    // f folds only in the briefings' tab.
    show(&mut app, Filter::Inbox);
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.answered, Group::Folded);
}

#[test]
fn a_picked_up_briefing_stays_in_its_tab_and_can_be_withdrawn() {
    let store = Store::new();
    let key = briefing(&store, "today", "picked up", true, &[]);
    store.answer(key, |a| "accept".clone_into(&mut a.briefing));
    store.pickup(key);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.selected_case().unwrap().1.title(), "picked up");
    press(&mut app, KeyCode::Char('w'));
    typing(&mut app, "wrong branch");
    ctrl(&mut app, 's');
    press(&mut app, KeyCode::Char('y'));
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Withdrawn);
}

#[test]
fn a_briefing_is_accepted_with_the_left_out_items_ticked_to_file() {
    let store = Store::new();
    let key = briefing(
        &store,
        "today",
        "Parser split",
        false,
        &["docs", "bench", "fuzz"],
    );
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("waiting to land"), "{text}");
    // Its left-out items are in its answer, to tick.
    assert!(text.contains("file what was left out"), "{text}");
    assert!(text.contains("[ ] 3  fuzz"), "{text}");
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none(), "sent with no verdict");
    assert_eq!(
        flash(&app),
        "nothing sent: y to accept, or s to send it back"
    );
    // On its items, a number ticks the item it names, again unticks it,
    // and one past the list does nothing.
    press(&mut app, KeyCode::Tab);
    for c in ['3', '2', '1', '2', '4'] {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Char('y'));
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("● accept"), "{text}");
    assert!(text.contains("[x] 1  docs"), "{text}");
    assert!(text.contains("[ ] 2  bench"), "{text}");
    assert!(text.contains("[x] 3  fuzz"), "{text}");
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    let answered = case(&store, key);
    let answer = answered.answer().unwrap();
    assert_eq!(
        (answer.briefing.as_str(), answer.file.as_slice()),
        ("accept", &[1, 3][..])
    );
    assert_eq!(answered.state(), State::Answered);
    assert_eq!(flash(&app), format!("recorded answer on {key}"));
    // It stays in the tab, under the folded group.
    assert!(titles(&app, Filter::Briefings).is_empty());
    assert_eq!(app.briefings().1.len(), 1);
}

#[test]
fn sending_a_briefing_back_needs_a_reason() {
    let store = Store::new();
    let key = briefing(&store, "today", "Parser split", true, &["docs"]);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    press(&mut app, KeyCode::Char('a'));
    // s goes to the note, which is the reason.
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.form.as_ref().unwrap().editing, Some(Which::Note));
    typing(&mut app, "   ");
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("● send back (reason required)"), "{text}");
    assert!(text.contains("reason"), "{text}");
    ctrl(&mut app, 's');
    assert!(app.pending.is_none(), "sent with no reason");
    assert_eq!(
        flash(&app),
        "nothing sent: sending back needs a reason: write it in the note"
    );
    typing(&mut app, "the undo step is missing ");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Char('1'));
    ctrl(&mut app, 's');
    send(&mut app, &store);
    let answered = case(&store, key);
    let answer = answered.answer().unwrap();
    assert_eq!(
        (
            answer.briefing.as_str(),
            answer.note.as_str(),
            answer.file.as_slice()
        ),
        ("send_back", "the undo step is missing", &[1][..])
    );
    // The answer reads back as the verdict and the item filed.
    press(&mut app, KeyCode::Char('f'));
    app.selected = Some(key);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("sent back"), "{text}");
    assert!(text.contains("file 1  docs"), "{text}");
}

#[test]
fn a_new_left_out_list_drops_the_ticks_after_a_stale_send() {
    let store = Store::new();
    let key = briefing(&store, "today", "Parser split", false, &["docs", "bench"]);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('y'));
    let mut amend = Amend::default();
    amend.left_out = vec!["bench".into(), "docs".into(), "fuzz".into()];
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(
        case(&store, key).state(),
        State::Open,
        "a stale answer was written"
    );
    // Item 2 is another item now: nothing is ticked, and the verdict stays.
    let form = app.form.as_ref().unwrap();
    assert_eq!(form.file, [false, false, false]);
    assert_eq!(form.briefing, Some("accept"));
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().file, [3]);
}

#[test]
fn a_new_left_out_list_shows_in_the_form_at_once_with_no_ticks() {
    let store = Store::new();
    let key = briefing(&store, "today", "Parser split", false, &["a", "b", "c"]);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('2'));
    let mut amend = Amend::default();
    amend.left_out = ["x", "y", "z", "w", "v"].map(String::from).to_vec();
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    // Item 2 of the old list is not item 2 of the new one.
    let text = screen(&mut app, 120, 40);
    assert!(!text.contains("to file"), "{text}");
    // Every new item can be ticked, and the ticks hold through the stale
    // send that the amend makes.
    press(&mut app, KeyCode::Char('5'));
    press(&mut app, KeyCode::Char('y'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).state(), State::Open);
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert_eq!(case(&store, key).answer().unwrap().file, [5]);
}

/// One case (TU-11): only it is shown, with no filters, the keys that would
/// leave it do nothing, and it has left once it is answered.
#[test]
fn one_case_shows_only_it_and_has_left_once_answered() {
    let store = Store::new();
    store.open("fyi", "blocking", "Another case", |_| {});
    let key = store.open("fyi", "today", "The one case", |_| {});
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    for filter in Filter::ALL {
        assert_eq!(titles(&app, filter), ["The one case"]);
    }
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("The one case"), "{shown}");
    assert!(!shown.contains("Another case"), "{shown}");
    assert!(!shown.contains("In flight"), "{shown}");

    for code in [
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Char('m'),
        KeyCode::Char('3'),
    ] {
        press(&mut app, code);
    }
    assert_eq!(app.focus, Focus::Case);
    assert_eq!(app.filter, Filter::Inbox);
    assert!(app.compose.is_none());
    assert!(!app.quit);
    assert!(!app.left());

    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert!(app.left());
}

/// One case's screen (TU-11) is about that case: its header does not count
/// what else waits in the store, its bar offers a park only on a stuck
/// case, and its help lists only the keys that work there.
#[test]
fn one_case_says_only_what_works_on_it() {
    let store = Store::new();
    store.open("fyi", "blocking", "Another case", |_| {});
    let stuck = store.open("stuck", "today", "Stuck here", |_| {});
    let fyi = store.open("fyi", "today", "Read this", |_| {});
    for (key, park) in [(stuck, true), (fyi, false)] {
        let mut app = App::one("cases.db".into(), key);
        read(&mut app, &mut store.db.poller(), &store.db).unwrap();
        let shown = screen(&mut app, 120, 40);
        let header = shown.lines().next().unwrap();
        assert!(!header.contains("waiting"), "{header}");
        assert!(!header.contains("blocking"), "{header}");
        assert!(!header.contains("nothing needs you"), "{header}");
        let footer = shown.lines().last().unwrap();
        assert_eq!(shown.contains(" Park "), park, "{shown}");
        // The bar's park is not the footer's too; the answer's keys
        // lead the footer.
        assert!(!footer.contains("park"), "{footer}");
        assert!(footer.starts_with("on: case · tab → "), "{footer}");
        assert!(footer.contains("n note   ⏎ answer"), "{footer}");
        // p does what the bar says: nothing on a case that cannot park.
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.form.is_some(), park, "p on {key}");
        assert!(app.toasts.is_empty(), "p flashed {:?}", app.toasts);
        app.form = None;
        assert!(
            shown.contains(" Answer ") && footer.contains("? keys"),
            "{shown}"
        );

        press(&mut app, KeyCode::Char('?'));
        let help = screen(&mut app, 120, 40);
        for gone in [
            "filter",
            "briefings",
            "message",
            "withdraw",
            "resume",
            "the list",
            "a tab",
        ] {
            assert!(!help.contains(gone), "help lists {gone:?}:\n{help}");
        }
        for listed in ["into the answer", "park it", "scroll the case", "quit"] {
            assert!(help.contains(listed), "help leaves out {listed:?}:\n{help}");
        }
    }
    // The inbox's header still counts.
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    assert!(
        shown.lines().next().unwrap().contains("3 waiting"),
        "{shown}"
    );
}

/// One case's screen takes only its own keys (TU-11): an answer, a park,
/// help and scrolling. The list's, the filters', messages', a withdraw's
/// and a resume's do nothing.
#[test]
fn one_case_takes_only_its_own_keys() {
    let store = Store::new();
    store.open("fyi", "blocking", "Another case", |_| {});
    let key = store.open("stuck", "today", "Stuck here", |open| {
        let long: Vec<String> = (1..=80).map(|n| format!("line {n}")).collect();
        open.body = long.join("\n\n");
    });
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    screen(&mut app, 80, 20);
    for code in [
        KeyCode::Char('m'),
        KeyCode::Char('e'),
        KeyCode::Char('f'),
        KeyCode::Char('w'),
        KeyCode::Char('r'),
        KeyCode::Char('['),
        KeyCode::Char(']'),
        KeyCode::Char('5'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char('x'),
        KeyCode::Esc,
    ] {
        press(&mut app, code);
        assert!(
            app.form.is_none() && app.compose.is_none(),
            "{code:?} opened a form"
        );
        assert!(app.toasts.is_empty(), "{code:?} flashed {:?}", app.toasts);
        assert_eq!(
            (app.filter, app.focus),
            (Filter::Inbox, Focus::Case),
            "{code:?}"
        );
        assert_eq!(app.selected, Some(key), "{code:?} moved off the case");
    }
    // Tab, shift-tab and enter go into its answer, and esc back to it.
    for code in [KeyCode::Tab, KeyCode::BackTab, KeyCode::Enter] {
        press(&mut app, code);
        assert_eq!(
            app.form.as_ref().map(|f| f.act),
            Some(super::form::Act::Answer),
            "{code:?}"
        );
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert!(app.form.is_none(), "{code:?}");
        assert_eq!(app.focus, Focus::Case, "{code:?}");
    }
    app.scroll = 0;
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll, 10);
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.scroll, 0);
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help);
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(
        app.form.as_ref().map(|f| f.act),
        Some(super::form::Act::Park)
    );
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert!(app.form.is_none());
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(
        app.form.as_ref().map(|f| f.act),
        Some(super::form::Act::Answer)
    );
}

/// The briefings' answered group is newest answer first, which a pickup
/// since does not change, as the web inbox orders it; its heading says
/// how many no agent has picked up yet (IN-14).
#[test]
fn the_answered_briefings_are_newest_answer_first_and_say_how_many_wait_for_a_pickup() {
    let store = Store::new();
    let first = briefing(&store, "today", "answered first", true, &[]);
    let second = briefing(&store, "today", "answered second", true, &[]);
    let third = briefing(&store, "today", "answered third", true, &[]);
    for key in [first, second, third] {
        store.answer(key, |a| "accept".clone_into(&mut a.briefing));
    }
    // Picked up last, but answered first.
    store.pickup(first);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(
        titles(&app, Filter::Briefings),
        ["answered third", "answered second", "answered first"]
    );
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("3 answered · 2 not picked up"), "{shown}");
    store.pickup(second);
    store.pickup(third);
    let mut app = store.app();
    show(&mut app, Filter::Briefings);
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("3 answered  f to show"), "{shown}");
}

/// A case open in a form that is answered elsewhere closes the form on
/// the next read, and then its options take no click, as its keys take
/// none: the click and the keys agree.
#[test]
fn a_case_answered_elsewhere_takes_neither_clicks_nor_keys_on_its_options() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    assert!(app.form.is_some());
    store.answer(key, |a| a.choice = 1);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.form.is_none(), "the form stayed on an answered case");
    let said = flash(&app);
    show(&mut app, Filter::All);
    app.selected = Some(key);
    click(&mut app, "Float it");
    assert!(app.form.is_none(), "a click opened the answer");
    assert_eq!(flash(&app), said, "a click flashed");
    press(&mut app, KeyCode::Char('a'));
    assert!(app.form.is_none(), "a key opened the answer");
}

/// One case has left when it is withdrawn, or gone from the store; the
/// inbox never has.
#[test]
fn one_case_has_left_when_withdrawn_or_gone() {
    let store = Store::new();
    let key = store.open("fyi", "today", "The one case", |_| {});
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(!app.left());
    store.agent(key, &Record::Withdraw(cases_core::Withdraw::default()));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.left());
    let now = app.now;
    app.update(Vec::new(), 0, crate::output::Actors::default(), now);
    assert!(app.left());
    assert!(!store.app().left());
}

/// One case has not left before a read has worked, nor when a read
/// worked but some case did not fold, which may be it.
#[test]
fn one_case_has_not_left_before_a_read_works_or_when_a_case_does_not_fold() {
    let store = Store::new();
    let key = store.open("fyi", "today", "The one case", |_| {});
    let mut app = App::one("cases.db".into(), key);
    assert!(!app.left(), "a first read that failed was taken as gone");
    let now = app.now;
    app.update(Vec::new(), 1, crate::output::Actors::default(), now);
    assert!(!app.left(), "a case that did not fold was taken as gone");
    app.update(Vec::new(), 0, crate::output::Actors::default(), now);
    assert!(app.left());
}

/// Where `text` is first drawn on `screen`, as a column and a row.
fn spot(screen: &str, text: &str) -> (u16, u16) {
    for (row, line) in screen.lines().enumerate() {
        if let Some(at) = line.find(text) {
            let column = line[..at].chars().count();
            return (u16::try_from(column).unwrap(), u16::try_from(row).unwrap());
        }
    }
    panic!("{text:?} is not on the screen:\n{screen}");
}

fn mouse(app: &mut App, kind: MouseEventKind, (column, row): (u16, u16)) {
    app.mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

/// Draws the screen, then clicks where `text` is drawn.
fn click(app: &mut App, text: &str) {
    let shown = screen(app, 120, 40);
    mouse(
        app,
        MouseEventKind::Down(MouseButton::Left),
        spot(&shown, text),
    );
}

/// A click on a tab switches to it (TU-12).
#[test]
fn a_click_on_a_tab_switches_to_it() {
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    click(&mut app, "Done");
    assert_eq!(app.filter, Filter::Done);
    click(&mut app, "Briefings");
    assert_eq!(app.filter, Filter::Briefings);
    click(&mut app, "Inbox");
    assert_eq!(app.filter, Filter::Inbox);
}

/// A click on a case selects it, and a click on the selected case opens
/// it, as enter does; on a narrow terminal the case then has the screen.
#[test]
fn a_click_on_a_case_selects_it_and_a_second_opens_it() {
    let store = Store::new();
    let first = store.open("fyi", "blocking", "First case", |_| {});
    let second = store.open("fyi", "today", "Second case", |_| {});
    let mut app = store.app();
    assert_eq!(app.selected, Some(first));
    click(&mut app, "Second case");
    assert_eq!(app.selected, Some(second));
    assert_eq!(app.focus, Focus::List);

    let narrow = screen(&mut app, 80, 30);
    let at = spot(&narrow, "Second case");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
    assert_eq!(app.focus, Focus::Case);
}

/// The wheel moves the selection over the list, and scrolls the case over
/// the case.
#[test]
fn the_wheel_moves_in_the_list_and_scrolls_the_case() {
    let store = Store::new();
    let first = store.open("fyi", "blocking", "First case", |open| {
        let long: Vec<String> = (1..=80).map(|n| format!("line {n}")).collect();
        open.body = long.join("\n\n");
    });
    let second = store.open("fyi", "today", "Second case", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    mouse(
        &mut app,
        MouseEventKind::ScrollDown,
        spot(&shown, "Second case"),
    );
    assert_eq!(app.selected, Some(second));
    mouse(
        &mut app,
        MouseEventKind::ScrollUp,
        spot(&shown, "Second case"),
    );
    assert_eq!(app.selected, Some(first));

    let shown = screen(&mut app, 120, 40);
    let body = spot(&shown, "line 1");
    mouse(&mut app, MouseEventKind::ScrollDown, body);
    mouse(&mut app, MouseEventKind::ScrollDown, body);
    assert_eq!(app.scroll, 6);
    assert_eq!(
        app.selected,
        Some(first),
        "the wheel over the case moved the list"
    );
    mouse(&mut app, MouseEventKind::ScrollUp, body);
    assert_eq!(app.scroll, 3);
}

/// In the answer's panel, a click on an option chooses it, on the note
/// gives it the keys, and on its send button sends it: nothing to open
/// first.
#[test]
fn a_click_in_the_answer_chooses_types_and_sends() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    // An option clicked with no answer open opens it.
    click(&mut app, "Float it");
    assert_eq!(app.form.as_ref().map(|f| f.choice), Some(Some(2)));
    click(&mut app, "Pin it");
    assert_eq!(app.form.as_ref().map(|f| f.choice), Some(Some(1)));
    click(&mut app, "note     empty");
    assert_eq!(app.form.as_ref().and_then(|f| f.editing), Some(Which::Note));
    typing(&mut app, "pinned");
    click(&mut app, "⏎ Send");
    send(&mut app, &store);
    let answer = case(&store, key).answer().cloned().unwrap();
    assert_eq!(answer.choice, 1);
    assert_eq!(answer.note, "pinned");
}

/// A click on an option or a left-out item of a case that is no longer
/// open does nothing: no form, and no flash to hide the footer's keys.
#[test]
fn a_click_on_a_closed_cases_option_or_item_does_nothing() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    store.answer(key, |a| a.choice = 1);
    store.pickup(key);
    store.close(key, "pinned");
    let briefing = briefing(&store, "today", "Brief", true, &["Leftover thing"]);
    store.answer(briefing, |a| "accept".clone_into(&mut a.briefing));
    let mut app = store.app();
    for (case, tab, text) in [
        (key, Filter::All, "Float it"),
        (briefing, Filter::Briefings, "Leftover thing"),
    ] {
        show(&mut app, tab);
        app.selected = Some(case);
        click(&mut app, text);
        assert!(app.form.is_none(), "a click on {text:?} opened the answer");
        assert!(
            app.toasts.is_empty(),
            "a click on {text:?} flashed {:?}",
            app.toasts
        );
    }
}

/// In one case's screen there are no tabs to click, and an option is
/// chosen as in the inbox.
#[test]
fn one_case_takes_clicks_on_what_it_shows() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("Done"), "{shown}");
    click(&mut app, "Float it");
    assert_eq!(app.form.as_ref().map(|f| f.choice), Some(Some(2)));
}

/// A click while help is up closes it, and does nothing else.
#[test]
fn a_click_closes_help() {
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('?'));
    click(&mut app, "Done");
    assert!(!app.help);
    assert_eq!(app.filter, Filter::Inbox);
}

/// A click on another case's option never reaches the answer of the
/// first one: it goes to the case it is drawn on, and the first case's
/// answer is kept as its draft.
#[test]
fn a_click_on_another_cases_option_leaves_the_form_alone() {
    let store = Store::new();
    let first = store.open("decision", "blocking", "First?", decision);
    let second = store.open("decision", "today", "Second?", decision);
    let mut app = store.app();
    assert_eq!(app.selected, Some(first));
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    // The selection moves while the form stays, as a read can move it.
    app.selected = Some(second);
    click(&mut app, "Float it");
    let form = app.form.as_ref().unwrap();
    assert_eq!((form.key, form.choice), (second, Some(2)));
    let draft = app.answer_of(first).unwrap();
    assert_eq!(draft.choice, Some(1), "the click chose on the first case");
}

/// A click while the form or the message asks a question takes it back,
/// as any key but y does.
#[test]
fn a_click_takes_back_a_question() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    assert!(app.form.as_ref().unwrap().confirm.is_some());
    click(&mut app, "Pin?");
    assert!(
        app.form.as_ref().unwrap().confirm.is_none(),
        "the question stayed"
    );
    assert!(app.pending.is_none());

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Char('i'));
    typing(&mut app, "Hello");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert!(app.compose.as_ref().unwrap().confirm, "no question asked");
    click(&mut app, "Pin?");
    assert!(
        !app.compose.as_ref().unwrap().confirm,
        "the question stayed"
    );
}

/// On a terminal as narrow as a phone's, the filters and the footer wrap
/// rather than lose a tab or a key off the end: each is drawn whole, and a
/// click on it does what it says (TU-02, TU-12).
#[test]
fn a_narrow_screen_wraps_the_tabs_and_keys_and_each_still_clicks() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    for width in [40, 50, 80] {
        let mut app = store.app();
        let shown = screen(&mut app, width, 24);
        for filter in Filter::ALL {
            let tab = format!(" {} ", filter.name());
            assert!(shown.contains(&tab), "{tab:?} at {width}:\n{shown}");
        }
        for keys in ["1-3 choose", "⏎ answer", "? keys", "q quit"] {
            assert!(shown.contains(keys), "{keys:?} at {width}:\n{shown}");
        }
        for filter in [Filter::Briefings, Filter::All, Filter::Inbox] {
            let at = spot(&screen(&mut app, width, 24), filter.name());
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
            assert_eq!(app.filter, filter, "a click on {filter:?} at {width}");
        }

        press(&mut app, KeyCode::Char('a'));
        let shown = screen(&mut app, width, 24);
        for keys in ["1-3 choose", "tab → note", "esc leave"] {
            assert!(shown.contains(keys), "{keys:?} at {width}:\n{shown}");
        }
        let at = spot(&shown, "esc leave");
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
        assert!(
            app.form.is_none(),
            "a click on esc at {width} left the panel"
        );

        let at = spot(&screen(&mut app, width, 24), "q quit");
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
        assert!(app.quit, "a click on q at {width} did not quit");
    }
}

/// A line the footer says wraps on a narrow terminal, rather than lose
/// its end: here, the question before throwing away what was typed.
#[test]
fn a_narrow_screen_wraps_what_the_footer_says() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('n'));
    typing(&mut app, "half a thought");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.confirm_quit);
    let shown = screen(&mut app, 40, 24);
    assert!(shown.contains("any other key no"), "{shown}");
}

/// A time `secs` seconds before the clock now.
fn ago(secs: i64) -> cases_core::Time {
    let now = crate::output::now().unwrap();
    cases_core::Time::from_unix(now.unix_secs() - secs, 0).unwrap()
}

/// The rows of the screen that hold `text`.
fn rows_with<'a>(shown: &'a str, text: &str) -> Vec<&'a str> {
    shown.lines().filter(|row| row.contains(text)).collect()
}

/// A store of three stuck cases opened an hour ago, two of them answered
/// half an hour ago, and a message, which the strip leaves out.
fn counted() -> Store {
    let store = Store::new();
    let keys: Vec<Key> = (1..=3)
        .map(|n| {
            store.open("stuck", "today", &format!("Case {n}"), |open| {
                open.opened_at = Some(ago(3_600));
            })
        })
        .collect();
    for key in &keys[..2] {
        store.answer(*key, |answer| {
            "Try the mirror.".clone_into(&mut answer.text);
            answer.answered_at = Some(ago(1_800));
        });
    }
    let mut open = Open::default();
    "message".clone_into(&mut open.kind);
    "today".clone_into(&mut open.urgency);
    "A message".clone_into(&mut open.title);
    "worker".clone_into(&mut open.worker);
    store
        .db
        .create(
            &Append::new(Author::Human, &Record::Open(open)),
            &Writer::new(Some("ryan")),
        )
        .unwrap();
    store
}

/// The strip under the filters counts the store's cases (TU-13): opened,
/// answered and the median time to answer, from the read.
#[test]
fn the_stats_strip_counts_the_cases_the_store_holds() {
    let store = counted();
    let mut app = store.app();
    let shown = screen(&mut app, 100, 30);
    let rows: Vec<&str> = shown.lines().collect();
    // Under the header, the filters and their rule.
    assert_eq!(rows[3].trim_end(), " 14 days, UTC · median answer 30m");
    assert!(rows[4].starts_with(" opened   "), "{}", rows[4]);
    assert!(rows[4].trim_end().ends_with("  3"), "{}", rows[4]);
    assert!(rows[5].starts_with(" answered "), "{}", rows[5]);
    assert!(rows[5].trim_end().ends_with("  2"), "{}", rows[5]);
    // A day a sparkline character: the three were opened on one day, the
    // day of an hour ago, which may be yesterday just after midnight UTC.
    let spark: String = rows[4].chars().skip(10).take(14).collect();
    let days: Vec<char> = spark.chars().filter(|c| *c != '·').collect();
    assert_eq!(days, ['█'], "{spark}");
}

/// s hides the strip and shows it again (TU-13).
#[test]
fn s_hides_and_shows_the_stats_strip() {
    let store = counted();
    let mut app = store.app();
    assert_eq!(
        rows_with(&screen(&mut app, 100, 30), "median answer").len(),
        1
    );
    press(&mut app, KeyCode::Char('s'));
    assert!(!app.strip.shown);
    let hidden = screen(&mut app, 100, 30);
    assert!(rows_with(&hidden, "median answer").is_empty(), "{hidden}");
    assert!(rows_with(&hidden, " opened ").is_empty(), "{hidden}");
    // The list moves up into its rows.
    assert!(
        hidden.lines().nth(4).unwrap().contains("Case 3"),
        "{hidden}"
    );
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(
        rows_with(&screen(&mut app, 100, 30), "median answer").len(),
        1
    );
}

/// Below 24 rows the strip is not drawn, so the list keeps its room
/// (TU-13).
#[test]
fn the_stats_strip_hides_itself_below_24_rows() {
    let store = counted();
    let mut app = store.app();
    assert!(rows_with(&screen(&mut app, 100, 23), "median answer").is_empty());
    assert_eq!(
        rows_with(&screen(&mut app, 100, 24), "median answer").len(),
        1
    );
}

/// At a phone's width the strip's lines fit whole (TU-13).
#[test]
fn the_stats_strip_fits_at_40_columns() {
    let store = counted();
    let mut app = store.app();
    let shown = screen(&mut app, 40, 30);
    let caption = rows_with(&shown, "median answer");
    assert_eq!(caption.len(), 1, "{shown}");
    assert_eq!(caption[0].trim_end(), " 14 days, UTC · median answer 30m");
    let opened = rows_with(&shown, " opened ");
    assert!(opened[0].trim_end().ends_with("  3"), "{shown}");
    let answered = rows_with(&shown, " answered ");
    assert!(answered[0].trim_end().ends_with("  2"), "{shown}");
}

/// The config file's `tui-stats = "off"` starts with no strip, and s
/// shows it (TU-13).
#[test]
fn tui_stats_off_starts_with_no_strip() {
    let store = counted();
    let mut app = store.app();
    app.strip.shown = super::stats::Mode::of(Some("off")).unwrap().shown();
    assert!(rows_with(&screen(&mut app, 100, 30), "median answer").is_empty());
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(
        rows_with(&screen(&mut app, 100, 30), "median answer").len(),
        1
    );
}

/// One case's screen (TU-11) has no strip, and s does nothing there.
#[test]
fn one_case_has_no_stats_strip() {
    let store = counted();
    let key = store.open("stuck", "today", "The one", |_| {});
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(rows_with(&screen(&mut app, 100, 30), "median answer").is_empty());
    press(&mut app, KeyCode::Char('s'));
    assert!(app.strip.shown);
}

/// ^s sends a form; with none open it leaves the strip as it is, so a
/// second ^s after a send does not hide it (TU-13).
#[test]
fn ctrl_s_with_no_form_leaves_the_stats_strip() {
    let store = counted();
    let mut app = store.app();
    ctrl(&mut app, 's');
    assert!(app.strip.shown);
}

/// With the case in focus, the footer offers s too (TU-13).
#[test]
fn the_case_s_footer_offers_s() {
    let store = counted();
    let mut app = store.app();
    press(&mut app, KeyCode::Char('l'));
    let shown = screen(&mut app, 60, 30);
    let footer = shown.lines().last().unwrap();
    assert!(
        shown
            .lines()
            .rev()
            .take(2)
            .any(|row| row.contains("s stats")),
        "{footer}"
    );
}

/// With no room for the strip, s does nothing and the footer does not
/// offer it (TU-13).
#[test]
fn s_does_nothing_where_the_strip_has_no_room() {
    let store = counted();
    let mut app = store.app();
    let short = screen(&mut app, 100, 23);
    assert!(!short.contains("s stats"), "{short}");
    press(&mut app, KeyCode::Char('s'));
    assert!(app.strip.shown, "s at 23 rows hid the strip");
    let tall = screen(&mut app, 100, 24);
    assert!(tall.contains("s stats"), "{tall}");
    assert_eq!(rows_with(&tall, "median answer").len(), 1);
}

/// As an image (TU-13), the series' rows leave blank the cells the image
/// goes over, and say where they are; not while help covers them.
#[test]
fn an_image_strip_leaves_the_image_its_cells() {
    let store = counted();
    let mut app = store.app();
    app.strip.image = true;
    let shown = screen(&mut app, 100, 30);
    let rows: Vec<&str> = shown.lines().collect();
    assert_eq!(rows[3].trim_end(), " 14 days, UTC · median answer 30m");
    assert_eq!(
        rows[4],
        format!(" opened   {}  3{}", " ".repeat(28), " ".repeat(59))
    );
    assert_eq!(
        rows[5],
        format!(" answered {}  2{}", " ".repeat(28), " ".repeat(59))
    );
    assert_eq!(
        app.strip.area,
        Some(ratatui::layout::Rect::new(10, 4, 28, 2))
    );
    // At a phone's width, a day a column, as the text, under the filters'
    // three lines, beside the message button.
    screen(&mut app, 40, 30);
    assert_eq!(
        app.strip.area,
        Some(ratatui::layout::Rect::new(10, 6, 14, 2))
    );
    app.help = true;
    screen(&mut app, 100, 30);
    assert_eq!(app.strip.area, None);
    app.help = false;
    press(&mut app, KeyCode::Char('s'));
    screen(&mut app, 100, 30);
    assert_eq!(app.strip.area, None, "a hidden strip has no image");
}

/// As text, the strip has no cells for an image.
#[test]
fn a_text_strip_has_no_image_cells() {
    let store = counted();
    let mut app = store.app();
    screen(&mut app, 100, 30);
    assert_eq!(app.strip.area, None);
}

/// Where the image would not fit whole beside the labels, the strip is
/// text, with no image put over it at a size of its own (TU-13).
#[test]
fn an_image_strip_too_narrow_for_its_image_is_text() {
    let store = counted();
    let mut app = store.app();
    app.strip.image = true;
    let shown = screen(&mut app, 20, 30);
    assert_eq!(app.strip.area, None);
    // The text's sparkline, cut at the edge as text is.
    let opened = rows_with(&shown, " opened ");
    assert!(opened[0].contains('·'), "{shown}");
}

/// The background of the cell at `at`, as `app` is drawn now.
fn bg_at(app: &mut App, at: (u16, u16)) -> ratatui::style::Color {
    bg_in(app, (120, 40), at)
}

/// The background of the cell at `at`, as `app` is drawn now at `size`.
fn bg_in(
    app: &mut App,
    (width, height): (u16, u16),
    (column, row): (u16, u16),
) -> ratatui::style::Color {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view::draw(frame, app)).unwrap();
    terminal.backend().buffer()[(column, row)].bg
}

/// Whether any cell of the screen is lit, as what the pointer is over.
fn lit_anywhere(app: &mut App) -> bool {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| view::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .any(|cell| cell.bg == view::HOVER_BG)
}

/// The pointer over a key in the footer, a tab or a case lights it, and
/// over nothing a click acts on lights nothing (TU-12).
#[test]
fn the_pointer_lights_what_a_click_acts_on() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    store.open("fyi", "today", "Other one", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    assert!(!lit_anywhere(&mut app), "lit before the pointer moved");
    for text in ["q quit", "In flight", "Other one"] {
        let at = spot(&shown, text);
        mouse(&mut app, MouseEventKind::Moved, at);
        assert_eq!(bg_at(&mut app, at), view::HOVER_BG, "{text:?} is not lit");
    }
    // The case pane's text is not clicked: nothing is lit over it.
    mouse(&mut app, MouseEventKind::Moved, spot(&shown, "THREAD"));
    assert_eq!(app.hover, None);
    assert!(!lit_anywhere(&mut app), "lit over the case's text");
    // The pointer stays, the footer under it changes: what is lit follows
    // the frame drawn, not the one the pointer moved over.
    let quit = spot(&shown, "q quit");
    mouse(&mut app, MouseEventKind::Moved, quit);
    press(&mut app, KeyCode::Char('a'));
    screen(&mut app, 120, 40);
    let lit = app.hover;
    let shown = screen(&mut app, 120, 40);
    let under = shown.lines().nth(usize::from(quit.1)).unwrap();
    assert!(!under.contains("q quit"), "{shown}");
    assert_ne!(
        lit,
        Some(super::app::Hit::Key(KeyEvent::from(KeyCode::Char('q'))))
    );
}

/// A question is a dialog with a button for each answer: a click on one
/// answers it, on the dialog's face does nothing, and anywhere else is
/// the safe answer, as any key but y is (TU-08, TU-12).
#[test]
fn a_question_is_a_dialog_whose_buttons_answer_it() {
    let store = Store::new();
    store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "draft");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('q'));
    let shown = screen(&mut app, 120, 40);
    assert!(app.confirm_quit, "{shown}");
    for text in [
        "Quit and throw away what you typed?",
        "esc Keep typing",
        "y Quit",
    ] {
        assert!(shown.contains(text), "{text:?}:\n{shown}");
    }
    // The pointer over a button lights it.
    let keep = spot(&shown, "esc Keep typing");
    let before = bg_at(&mut app, keep);
    mouse(&mut app, MouseEventKind::Moved, keep);
    assert_ne!(bg_at(&mut app, keep), before, "the button is not lit");
    // Its face: still asking.
    let title = spot(&shown, "Quit and throw");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), title);
    assert!(app.confirm_quit && !app.quit);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), keep);
    assert!(!app.confirm_quit && !app.quit);
    assert!(app.form.is_some(), "keep typing lost the form");
    // Off the dialog: no.
    press(&mut app, KeyCode::Char('q'));
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), (0, 0));
    assert!(!app.confirm_quit && !app.quit);
    press(&mut app, KeyCode::Char('q'));
    click(&mut app, "y Quit");
    assert!(app.quit);

    // The form's: a drop.
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("The agent stops the work."), "{shown}");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), (0, 0));
    assert!(app.form.as_ref().unwrap().confirm.is_none());
    assert!(app.pending.is_none(), "a drop went without a yes");
    press(&mut app, KeyCode::Char('d'));
    click(&mut app, "y Drop");
    assert!(app.pending.as_ref().is_some_and(|send| matches!(
        &send.record,
        Record::Answer(answer) if answer.drop
    )));

    // The message's: throw it away.
    let mut app = store.app();
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Char('i'));
    typing(&mut app, "A title");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    click(&mut app, "esc Keep typing");
    assert!(app.compose.as_ref().is_some_and(|c| !c.confirm));
    press(&mut app, KeyCode::Esc);
    click(&mut app, "y Discard");
    assert!(app.compose.is_none());
}

/// What a write did, or why a key did nothing, is a toast in the corner,
/// which goes after a few seconds or on a click; three at most (TU-12).
#[test]
fn a_toast_says_what_happened_then_goes() {
    use std::time::{Duration, Instant};
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('r'));
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("only a parked case resumes"), "{shown}");
    // The footer's keys stay.
    assert!(shown.contains("q quit"), "{shown}");
    app.expire_toasts(Instant::now() + Duration::from_secs(3));
    assert_eq!(app.toasts.len(), 1, "gone too soon");
    app.expire_toasts(Instant::now() + Duration::from_secs(5));
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("only a parked case resumes"), "{shown}");

    for n in 1..=4 {
        app.flash(&format!("toast {n}"));
    }
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("toast 1"), "{shown}");
    assert!(
        spot(&shown, "toast 4").1 < spot(&shown, "toast 2").1,
        "{shown}"
    );
    click(&mut app, "toast 3");
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("toast 3"), "{shown}");
    assert!(
        shown.contains("toast 4") && shown.contains("toast 2"),
        "{shown}"
    );

    // A write that lands says so.
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    send(&mut app, &store);
    assert!(
        app.toasts.last().is_some_and(|t| t.done),
        "{:?}",
        app.toasts
    );
    assert!(screen(&mut app, 120, 40).contains("✓ recorded answer"));
}

/// What a click would not act on is not lit: the list and the tabs while
/// a field of the answer is typed in, and anything under help (TU-12).
#[test]
fn the_pointer_lights_nothing_a_click_would_not_act_on() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    store.open("fyi", "today", "Other one", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('n'));
    let shown = screen(&mut app, 120, 40);
    for text in ["In flight", "Other one"] {
        mouse(&mut app, MouseEventKind::Moved, spot(&shown, text));
        assert!(!lit_anywhere(&mut app), "{text:?} is lit while typing");
    }
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('?'));
    let shown = screen(&mut app, 120, 40);
    mouse(&mut app, MouseEventKind::Moved, spot(&shown, "q quit"));
    assert!(!lit_anywhere(&mut app), "a footer key is lit under help");
}

/// The wheel over a toast scrolls what is under it.
#[test]
fn the_wheel_over_a_toast_scrolls_what_is_under_it() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    app.focus = Focus::Case;
    app.flash("a toast over the case");
    let shown = screen(&mut app, 120, 40);
    mouse(
        &mut app,
        MouseEventKind::ScrollDown,
        spot(&shown, "a toast over"),
    );
    assert!(app.scroll > 0, "the wheel over a toast did nothing");
}

/// On a terminal too short for the whole dialog, its buttons are still
/// drawn, and clicked.
#[test]
fn a_dialog_on_a_short_terminal_keeps_its_buttons() {
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    let mut terminal = Terminal::new(TestBackend::new(30, 6)).unwrap();
    terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let shown: String = (0..6)
        .map(|y| (0..30).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        spot(&shown, "y Drop"),
    );
    assert!(app.pending.is_some(), "{shown}");
}

/// A message from the human to `worker`, sent: no agent has picked it up.
fn message(store: &Store, title: &str) -> Key {
    let mut message = Open::default();
    "message".clone_into(&mut message.kind);
    "whenever".clone_into(&mut message.urgency);
    title.clone_into(&mut message.title);
    "worker".clone_into(&mut message.worker);
    let append = Append::new(Author::Human, &Record::Open(message));
    store
        .db
        .create(&append, &Writer::new(Some("ryan")))
        .unwrap()
        .0
}

/// A store with a case of each kind and state the bar tells apart, and
/// the acts its bar offers on each, as its keys would do them.
fn every_state() -> (Store, Vec<(Key, Vec<Verb>)>) {
    use Verb::{Answer, Edit, Park, Resume, Withdraw};
    let store = Store::new();
    let decision_open = store.open("decision", "today", "decision open", decision);
    let stuck_open = store.open("stuck", "today", "stuck open", |_| {});
    let stuck_parked = store.open("stuck", "today", "stuck parked", |_| {});
    store.human(stuck_parked, &Record::Park(cases_core::Park::default()));
    let briefing = store.open("briefing", "today", "briefing open", |open| {
        "What landed.".clone_into(&mut open.body);
    });
    let answered = store.open("question", "today", "answered", |_| {});
    store.answer(answered, |a| "yes".clone_into(&mut a.text));
    let picked = store.open("fyi", "today", "picked up", |_| {});
    store.answer(picked, |a| a.ack = true);
    store.pickup(picked);
    let closed = store.open("fyi", "today", "closed", |_| {});
    store.answer(closed, |a| a.ack = true);
    store.pickup(closed);
    store.close(closed, "done");
    let sent = message(&store, "message sent");
    let message_picked = message(&store, "message picked up");
    store.pickup(message_picked);
    let verbs = vec![
        (decision_open, vec![Answer]),
        (stuck_open, vec![Answer, Park]),
        (stuck_parked, vec![Resume]),
        (briefing, vec![Answer]),
        (answered, vec![Withdraw]),
        (picked, vec![Withdraw]),
        (closed, vec![]),
        (sent, vec![Withdraw, Edit]),
        (message_picked, vec![Withdraw]),
    ];
    (store, verbs)
}

/// The case's bar offers a button for each act its keys do on the case,
/// by its kind and state, and none while a form is open (TU-12).
#[test]
fn the_bar_offers_what_the_keys_do_by_kind_and_state() {
    let (store, verbs) = every_state();
    let mut app = store.app();
    app.filter = Filter::All;
    for (key, want) in verbs {
        app.selected = Some(key);
        assert_eq!(app.verbs(), want, "{key}");
        let shown = screen(&mut app, 120, 40);
        for verb in Verb::ALL {
            let drawn = shown.contains(&format!(" {} ", verb.label()));
            assert_eq!(drawn, want.contains(&verb), "{verb:?} on {key}:\n{shown}");
        }
    }
    let (store, verbs) = every_state();
    let mut app = store.app();
    // The answer's panel keeps the bar; a park's form does not.
    app.selected = Some(verbs[0].0);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.verbs(), [Verb::Answer], "no bar beside the answer");
    app.form = None;
    app.selected = Some(verbs[1].0);
    press(&mut app, KeyCode::Char('p'));
    assert!(app.verbs().is_empty(), "a bar beside the park");
}

/// A click on each of the bar's buttons does what its key does.
#[test]
fn a_click_on_each_of_the_bars_buttons_presses_its_key() {
    use super::form::Act;
    let (store, verbs) = every_state();
    for (key, want) in verbs {
        for verb in want {
            let mut app = store.app();
            app.filter = Filter::All;
            app.selected = Some(key);
            click(&mut app, &format!(" {} ", verb.label()));
            let done = match verb {
                Verb::Answer => app.form.as_ref().is_some_and(|f| f.act == Act::Answer),
                Verb::Park => app.form.as_ref().is_some_and(|f| f.act == Act::Park),
                Verb::Withdraw => app.form.as_ref().is_some_and(|f| f.act == Act::Withdraw),
                Verb::Resume => app
                    .pending
                    .as_ref()
                    .is_some_and(|send| matches!(send.record, Record::Resume(_))),
                Verb::Edit => app
                    .compose
                    .as_ref()
                    .is_some_and(|c| c.editing.is_some_and(|(k, _)| k == key)),
            };
            assert!(done, "a click on {verb:?} on {key}");
        }
    }
}

/// One case's bar has only what that screen takes: an answer, and a
/// park on a stuck case (TU-11).
#[test]
fn one_cases_bar_offers_only_what_its_screen_takes() {
    let store = Store::new();
    let stuck = store.open("stuck", "today", "Stuck here", |_| {});
    let mut app = App::one("cases.db".into(), stuck);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(app.verbs(), [Verb::Answer, Verb::Park]);
    click(&mut app, " Park ");
    assert!(app.form.is_some(), "a click on park");
}

/// The tabs are pills: the chosen one filled, another lit under the
/// pointer, and a click on one switches to it. The message button at the
/// end of their row starts a message, and does nothing while a form is
/// open (TU-12).
#[test]
fn the_tabs_are_pills_beside_a_message_button() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    let inbox = spot(&shown, "Inbox");
    assert_eq!(bg_at(&mut app, inbox), super::view::ACCENT);
    let done = spot(&shown, "Done");
    assert_eq!(bg_at(&mut app, done), ratatui::style::Color::Reset);
    mouse(&mut app, MouseEventKind::Moved, done);
    assert_eq!(bg_at(&mut app, done), view::HOVER_BG);
    let row = shown.lines().nth(1).unwrap();
    assert!(row.trim_end().ends_with("+ Message"), "{row}");
    click(&mut app, "+ Message");
    assert!(app.compose.is_some(), "a click on + Message");
    app.compose = None;
    press(&mut app, KeyCode::Char('a'));
    click(&mut app, "+ Message");
    assert!(app.compose.is_none(), "+ Message with a form open");
    press(&mut app, KeyCode::Esc);
    click(&mut app, "Done");
    assert_eq!(app.filter, Filter::Done);
}

/// At 80 columns, one pane at a time: the list's footer says how to
/// answer, as no button for it is drawn; the case's bar has the answer,
/// and its footer leaves it to the bar. At 20, the bar wraps rather than
/// lose a button.
#[test]
fn at_80_columns_the_bar_and_the_footer_share_the_keys() {
    let store = Store::new();
    store.open("stuck", "today", "Stuck here", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 80, 24);
    let row = shown.lines().nth(1).unwrap();
    assert!(
        row.contains("Briefings") && row.contains("+ Message"),
        "{shown}"
    );
    // The footer, over its two lines.
    let footer = shown.lines().rev().take(2).collect::<Vec<_>>().concat();
    assert!(
        footer.contains("⏎ answer") && footer.contains("^k commands"),
        "{shown}"
    );
    assert!(
        shown.lines().next().unwrap().contains("^k Commands"),
        "{shown}"
    );
    assert!(!shown.contains(" Answer "), "{shown}");
    press(&mut app, KeyCode::Char('l'));
    let shown = screen(&mut app, 80, 24);
    assert!(shown.contains(" Answer   Park "), "{shown}");
    // The footer still leads with how to answer, from the case.
    let footer = shown.lines().rev().take(2).collect::<Vec<_>>().concat();
    assert!(footer.contains("on: case · tab → guidance"), "{shown}");
    assert!(!footer.contains(" p park"), "{shown}");

    let store = Store::new();
    message(&store, "A message");
    let mut app = store.app();
    app.filter = Filter::Flight;
    app.selected = app.shown(Filter::Flight).first().map(|(k, _)| *k);
    app.focus = Focus::Case;
    let shown = screen(&mut app, 20, 30);
    assert!(
        shown.contains(" Withdraw ") && shown.contains(" Edit "),
        "{shown}"
    );
    assert_ne!(
        spot(&shown, " Withdraw ").1,
        spot(&shown, " Edit ").1,
        "{shown}"
    );
}

/// The pointer over one of the bar's buttons lights it.
#[test]
fn the_pointer_lights_a_button_of_the_bar() {
    let store = Store::new();
    store.open("stuck", "today", "Stuck here", |_| {});
    let mut app = store.app();
    for at in [" Answer ", " Park "] {
        let shown = screen(&mut app, 120, 40);
        let (x, y) = spot(&shown, at);
        let before = bg_at(&mut app, (x + 2, y));
        mouse(&mut app, MouseEventKind::Moved, (x + 2, y));
        assert_ne!(bg_at(&mut app, (x + 2, y)), before, "{at:?} is not lit");
    }
    // One pane at a time, the case read.
    press(&mut app, KeyCode::Char('l'));
    for at in [" Answer ", " Park "] {
        let shown = screen(&mut app, 80, 24);
        let (x, y) = spot(&shown, at);
        let before = bg_in(&mut app, (80, 24), (x + 2, y));
        mouse(&mut app, MouseEventKind::Moved, (x + 2, y));
        let after = bg_in(&mut app, (80, 24), (x + 2, y));
        assert_ne!(after, before, "{at:?} is not lit at 80 columns:\n{shown}");
    }
}

/// On a pane too short for the title, the bar and some of the case, the
/// case still scrolls to its options; and the footer offers what no
/// button on the screen does.
#[test]
fn a_short_pane_keeps_the_case_in_reach_and_the_keys_said() {
    let store = Store::new();
    store.open(
        "stuck",
        "today",
        &"A long title that wraps ".repeat(8),
        |open| {
            "the body".clone_into(&mut open.body);
        },
    );
    let mut app = store.app();
    app.strip.shown = false;
    press(&mut app, KeyCode::Char('l'));
    // Somewhere above the answer at its foot.
    let reached = (0..30).any(|at| {
        app.scroll = at;
        screen(&mut app, 40, 14).contains("the body")
    });
    let shown = screen(&mut app, 40, 14);
    assert!(reached, "the body is out of reach:\n{shown}");
    let answer = shown.contains(" Answer ") || shown.contains("⏎ answer");
    assert!(answer, "nothing says how to answer:\n{shown}");
}

/// An approval row, `id`, its step run by `by`.
fn step(id: &str, by: &str) -> cases_core::ApprovalRow {
    let mut row = cases_core::ApprovalRow::default();
    id.clone_into(&mut row.id);
    format!("Run {id}").clone_into(&mut row.label);
    by.clone_into(&mut row.by);
    "true".clone_into(&mut row.script);
    "https://example.com".clone_into(&mut row.link);
    row
}

/// Clicks the `n`th place on the screen where `text` is drawn, from 0.
fn click_nth(app: &mut App, text: &str, n: usize) {
    let shown = screen(app, 120, 50);
    let at = shown
        .lines()
        .enumerate()
        .flat_map(|(row, line)| {
            line.match_indices(text).map(move |(at, _)| {
                let column = line[..at].chars().count();
                (u16::try_from(column).unwrap(), u16::try_from(row).unwrap())
            })
        })
        .nth(n)
        .unwrap_or_else(|| panic!("{text:?} is not on the screen {n} times:\n{shown}"));
    mouse(app, MouseEventKind::Down(MouseButton::Left), at);
}

/// An approval is answered by clicks alone: a verdict for each row,
/// then send, with nothing opened first (TU-05, TU-12).
#[test]
fn an_approvals_row_verdicts_are_picked_by_click() {
    let store = Store::new();
    let key = store.open("approval", "today", "Move the host?", |open| {
        open.rows = vec![step("dns", "human"), step("tag", ""), step("post", "")];
    });
    let mut app = store.app();
    assert!(app.form.is_none());
    click_nth(&mut app, "○ done", 0);
    assert!(
        app.form.is_some(),
        "a click on a verdict did not enter the answer"
    );
    click_nth(&mut app, "○ hold", 1);
    click_nth(&mut app, "○ reject", 1);
    click(&mut app, "⏎ Send");
    send(&mut app, &store);
    let rows: Vec<(String, String)> = case(&store, key)
        .answer()
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.id.clone(), r.verdict.clone()))
        .collect();
    let want = [("dns", "approve"), ("tag", "hold"), ("post", "reject")];
    assert_eq!(
        rows,
        want.map(|(id, verdict)| (id.to_string(), verdict.to_string()))
    );
}

/// Tab goes round one ring (TU-12): from the list into the answer, along
/// its parts, and out of the last back to the list; shift-tab the other
/// way. Esc leaves the field, then the answer, for the list. The answer
/// keeps what is in it, and a, its button or a click in it put the keys
/// back (TU-05, TU-08).
#[test]
fn tab_and_esc_move_the_keys_in_and_out_of_the_answer() {
    use super::form::{Press, Spot};
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    let spot = |app: &App| app.form.as_ref().map(super::form::Form::spot);
    // From the list, tab is the answer's first part.
    press(&mut app, KeyCode::Tab);
    assert_eq!(spot(&app), Some(Spot::Choices));
    assert_eq!(app.filter, Filter::Inbox);
    // In it, a number chooses, and tab moves to the note, typing there.
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(spot(&app), Some(Spot::Field(Which::Note)));
    typing(&mut app, "q1");
    assert!(!app.quit, "q typed into the note quit");
    // Clicking a pill while typing does nothing.
    click(&mut app, "In flight");
    assert_eq!(app.filter, Filter::Inbox);
    let mut ring = Vec::new();
    for _ in 0..4 {
        press(&mut app, KeyCode::Tab);
        ring.push(spot(&app));
    }
    assert_eq!(
        ring,
        [
            Some(Spot::Button(Press::Send)),
            Some(Spot::Button(Press::Drop)),
            Some(Spot::Button(Press::Discard)),
            // Past the last, back to the list, keeping the draft.
            None,
        ]
    );
    assert_eq!(app.focus, Focus::List);
    let draft = app.answer_of(app.selected.unwrap()).unwrap();
    assert_eq!((draft.choice, draft.note.text()), (Some(2), "q1"));
    // Shift-tab from the list is the answer's last part, and goes back
    // round to the list before its first.
    press(&mut app, KeyCode::BackTab);
    assert_eq!(spot(&app), Some(Spot::Button(Press::Discard)));
    for _ in 0..3 {
        press(&mut app, KeyCode::BackTab);
    }
    assert_eq!(spot(&app), Some(Spot::Field(Which::Note)));
    press(&mut app, KeyCode::BackTab);
    assert_eq!(spot(&app), Some(Spot::Choices));
    press(&mut app, KeyCode::BackTab);
    assert_eq!((spot(&app), app.focus), (None, Focus::List));
    // Esc leaves the field, then the answer, for the list.
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.form.as_ref().unwrap().editing, Some(Which::Note));
    press(&mut app, KeyCode::Esc);
    assert!(app.form.as_ref().is_some_and(|f| f.editing.is_none()));
    press(&mut app, KeyCode::Esc);
    assert!(app.form.is_none());
    assert_eq!(app.focus, Focus::List);
    // Out of it, ] and [ switch filters.
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.filter, Filter::Flight);
    press(&mut app, KeyCode::Char('['));
    // The bar's Answer, and a click in the answer, put the keys back.
    click(&mut app, " Answer ");
    assert!(app.form.is_some());
    press(&mut app, KeyCode::Esc);
    click(&mut app, "Pin it");
    assert_eq!(app.form.as_ref().and_then(|f| f.choice), Some(1));
    assert_eq!(app.form.as_ref().unwrap().note.text(), "q1");
}

/// What is chosen or typed in a case's answer stays with that case when
/// another is selected, and through the reads of the store.
#[test]
fn a_draft_stays_with_its_case_across_a_switch_and_a_read() {
    let store = Store::new();
    let first = store.open("decision", "blocking", "First?", decision);
    let second = store.open("question", "today", "Second?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('n'));
    typing(&mut app, "half a thought");
    press(&mut app, KeyCode::Esc);
    // A click on the other case in the list takes the keys out.
    click(&mut app, "Second?");
    assert_eq!(app.selected, Some(second));
    assert!(app.form.is_none());
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("half a thought"), "{shown}");
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "the reply");
    // Esc leaves the field, then the answer, back to the list.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::List);
    // A read later, both drafts are there.
    let mut note = Note::default();
    "still deciding".clone_into(&mut note.body);
    store.agent(first, &Record::Note(note));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(app.answer_of(second).unwrap().text.text(), "the reply");
    // Up to the first case.
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.selected, Some(first));
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("● 2  Float it"), "{shown}");
    assert!(shown.contains("half a thought"), "{shown}");
    // Quitting with typed drafts asks first.
    press(&mut app, KeyCode::Char('q'));
    assert!(app.confirm_quit && !app.quit);
    // And a draft sent after the read goes, with the note's revision.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('a'));
    ctrl(&mut app, 's');
    send(&mut app, &store);
    assert_eq!(case(&store, first).state(), State::Answered);
    assert!(app.answer_of(first).is_none());
}

/// At 80 columns, the case read: its bar, its answer at its foot with the
/// buttons under it, and a footer that says how to get in.
#[test]
fn at_80_columns_the_answer_is_at_the_foot_of_the_case() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    app.strip.shown = false;
    press(&mut app, KeyCode::Char('l'));
    let shown = screen(&mut app, 80, 24);
    for text in [
        "Answer",
        "YOUR ANSWER",
        "○ 1  Pin it",
        "Other, see note",
        "⏎ Send   d Drop",
    ] {
        assert!(shown.contains(text), "{text:?}:\n{shown}");
    }
    press(&mut app, KeyCode::Char('a'));
    let shown = screen(&mut app, 80, 24);
    let footer = shown.lines().last().unwrap();
    assert!(
        footer.contains("tab → ") || shown.contains("esc leave"),
        "{shown}"
    );
    assert!(shown.contains("▸ ○ 1  Pin it"), "{shown}");
}

/// The answer sent by clicks alone is the answer the keys send: the same
/// events in the store, kind by kind.
#[test]
fn an_answer_by_clicks_records_what_the_keys_record() {
    let answers = |by_click: bool| {
        let store = Store::new();
        let key = store.open("decision", "today", "Pin?", decision);
        let mut app = store.app();
        if by_click {
            click(&mut app, "Float it");
            click(&mut app, "note     empty");
            typing(&mut app, " fine ");
            click(&mut app, "⏎ Send");
        } else {
            press(&mut app, KeyCode::Char('a'));
            press(&mut app, KeyCode::Char('2'));
            press(&mut app, KeyCode::Char('n'));
            typing(&mut app, " fine ");
            ctrl(&mut app, 's');
        }
        let pending = app.pending.as_ref().map(|send| send.record.clone());
        send(&mut app, &store);
        let mut answer = case(&store, key).answer().cloned().unwrap();
        // When, and the actor's id, which each store makes afresh.
        answer.answered_at = None;
        answer.actor = None;
        (pending, answer)
    };
    assert_eq!(answers(true), answers(false));
}

/// On a pane too narrow for an approval row's verdicts on one line, each
/// is on a line of its own, and a click on one picks it.
#[test]
fn a_narrow_answer_has_a_verdict_to_a_line_and_each_clicks() {
    let store = Store::new();
    store.open("approval", "today", "Move?", |open| {
        open.rows = vec![step("tag", "")];
    });
    let mut app = store.app();
    app.focus = Focus::Case;
    app.strip.shown = false;
    app.scroll = u16::MAX;
    let shown = screen(&mut app, 30, 30);
    for verdict in ["○ approve", "○ hold", "○ reject"] {
        let line = shown.lines().find(|l| l.contains(verdict)).unwrap();
        assert_eq!(line.trim(), verdict, "{shown}");
    }
    for (text, verdict) in [("○ reject", "reject"), ("○ hold", "hold")] {
        app.scroll = u16::MAX;
        let shown = screen(&mut app, 30, 30);
        let (x, y) = spot(&shown, text);
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            (x + 4, y),
        );
        let picked = app.form.as_ref().and_then(|f| f.rows[0].verdict);
        assert_eq!(picked, Some(verdict), "{shown}");
    }
}

/// Review repro: a draft whose case can no longer be answered goes, and
/// says so, as the form does.
#[test]
fn a_draft_whose_case_leaves_says_so() {
    let store = Store::new();
    let first = store.open("question", "blocking", "First?", |_| {});
    store.open("question", "today", "Second?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "a long reply");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    store.answer(first, |a| "elsewhere".clone_into(&mut a.text));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.answer_of(first).is_none());
    assert!(flash(&app).contains("nothing was sent"), "{}", flash(&app));
}

/// Review repro: a send refused as stale takes the case as amended into
/// its draft too, when the keys left the answer while it was sent; and
/// quitting then does not ask about it while it is being sent.
#[test]
fn a_draft_sent_then_left_is_not_asked_about_and_takes_a_refusal() {
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "a reply");
    ctrl(&mut app, 's');
    let pending = app.pending.take().unwrap();
    app.sending = Some(Sending::Event(Box::new(pending)));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert!(app.form.is_none() && app.answer_of(key).is_some());
    press(&mut app, KeyCode::Char('q'));
    assert!(
        app.quit && !app.confirm_quit,
        "asked about a draft being sent"
    );
    // Refused as stale: the draft takes the case as it is now.
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "a reply");
    ctrl(&mut app, 's');
    let send = app.pending.take().unwrap();
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    let mut amend = Amend::default();
    "Name it, please?".clone_into(&mut amend.title);
    store.agent(key, &Record::Amend(amend));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.sent(&send, Err(Refusal::Stale));
    let revision = case(&store, key).revision();
    assert_eq!(app.answer_of(key).unwrap().revision, revision);
}

/// Review repro: what is typed into the answer stays in sight as it
/// grows, on a case longer than its pane.
#[test]
fn the_field_being_typed_in_stays_in_sight() {
    let store = Store::new();
    store.open("question", "today", "Name?", |open| {
        open.body = (0..40)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n\n");
    });
    let mut app = store.app();
    app.strip.shown = false;
    press(&mut app, KeyCode::Char('a'));
    screen(&mut app, 80, 24);
    for n in 0..8 {
        typing(&mut app, &format!("typed {n}"));
        press(&mut app, KeyCode::Enter);
    }
    typing(&mut app, "the last");
    let shown = screen(&mut app, 80, 24);
    assert!(shown.contains("the last"), "{shown}");
}

/// A write that lands says so, once: the read that shows the case
/// answered does not first say nothing was sent (TU-04).
#[test]
fn a_write_that_lands_is_not_said_to_have_sent_nothing() {
    let store = Store::new();
    let key = store.open("decision", "today", "Pin?", decision);
    store.open("fyi", "today", "Next", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('1'));
    ctrl(&mut app, 's');
    send(&mut app, &store);
    let said: Vec<&str> = app.toasts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(said, [format!("recorded answer on {key}")], "{said:?}");
}

/// Discard throws away what was chosen or typed in an answer, after the
/// dialog, and leaves the case open; drop is still the case's drop.
#[test]
fn discard_throws_away_a_draft_and_the_case_stays_open() {
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("Discard"), "nothing to discard:\n{shown}");
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "half a reply");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert!(app.answer_of(key).is_some());
    click(&mut app, " Discard ");
    assert!(app.asking(), "no question before discarding");
    click(&mut app, "y Discard");
    assert!(app.answer_of(key).is_none(), "the draft is still there");
    assert!(app.pending.is_none(), "discarding sent something");
    assert_eq!(case(&store, key).state(), State::Open);
    let shown = screen(&mut app, 120, 40);
    assert!(!shown.contains("half a reply"), "{shown}");
}

/// A toast said again is the same toast up for longer; and a click puts
/// away the toast it lands on, whichever have gone meanwhile.
#[test]
fn a_toast_said_again_is_refreshed_and_a_click_names_its_own() {
    let store = Store::new();
    store.open("fyi", "today", "FYI", |_| {});
    let mut app = store.app();
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('r'));
    }
    assert_eq!(app.toasts.len(), 1, "{:?}", app.toasts);
    app.flash("second");
    app.flash("third");
    let shown = screen(&mut app, 120, 40);
    let third = spot(&shown, "third");
    // The oldest goes before the click lands.
    app.toasts.remove(0);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), third);
    let said: Vec<&str> = app.toasts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(said, ["second"]);
}

/// What the pointer lights is not lit through a toast over it.
#[test]
fn a_toast_over_what_is_lit_stays_as_it_is() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    app.strip.shown = false;
    app.flash(&"a toast over the list ".repeat(5));
    // One pane: the list's first row runs under the toast.
    let shown = screen(&mut app, 80, 24);
    let row = spot(&shown, "Pin?");
    let line = shown.lines().nth(usize::from(row.1)).unwrap();
    let x = line.find("toast").map(|at| line[..at].chars().count());
    let (x, y) = (
        u16::try_from(x.expect("no toast on the row")).unwrap(),
        row.1,
    );
    mouse(&mut app, MouseEventKind::Moved, row);
    assert!(app.hover.is_some());
    assert_eq!(bg_in(&mut app, (80, 24), row), view::HOVER_BG);
    assert_ne!(
        bg_in(&mut app, (80, 24), (x, y)),
        view::HOVER_BG,
        "lit through the toast"
    );
}

/// A move that leaves the pointer over the same thing asks for no new
/// frame; one onto something else does. A click that opens help leaves
/// nothing lit under it.
#[test]
fn a_move_over_the_same_thing_draws_nothing_new_and_help_lights_nothing() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    let shown = screen(&mut app, 120, 40);
    let (x, y) = spot(&shown, "? keys");
    let moved = |app: &mut App, at: (u16, u16)| {
        app.mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        })
    };
    assert!(moved(&mut app, (x, y)), "onto a key");
    assert!(!moved(&mut app, (x + 1, y)), "along the same key");
    assert!(moved(&mut app, (0, 0)), "off it");
    moved(&mut app, (x, y));
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), (x, y));
    assert!(app.help);
    assert_eq!(app.hover, None, "lit under help");
}

/// One case's screen has no filters, and no row kept for them.
#[test]
fn one_cases_screen_has_no_blank_row_for_filters() {
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    let shown = screen(&mut app, 80, 24);
    let second = shown.lines().nth(1).unwrap();
    assert!(second.starts_with("────"), "{shown}");
}

/// Review repro: an answer refused because its case was withdrawn
/// meanwhile does not leave the answer open on a withdrawn case.
#[test]
fn an_answer_refused_on_a_withdrawn_case_closes() {
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    store.answer(key, |a| "first".clone_into(&mut a.text));
    let mut note = Note::default();
    "again?".clone_into(&mut note.body);
    note.reopen = true;
    store.agent(key, &Record::Note(note));
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    typing(&mut app, "a reply");
    ctrl(&mut app, 's');
    let send = app.pending.take().unwrap();
    app.sending = Some(Sending::Event(Box::new(send)));
    store.agent(key, &Record::Withdraw(cases_core::Withdraw::default()));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    app.wrote(Wrote::Append(Err(Refusal::Stale)));
    assert!(
        app.form.is_none(),
        "the answer is still open on a withdrawn case"
    );
}

/// Review repro: a toast that expires says the screen is to be drawn
/// again, as a move of the pointer may be all that happens meanwhile.
#[test]
fn a_toast_that_expires_asks_for_a_new_frame() {
    use std::time::{Duration, Instant};
    let mut app = App::new("cases.db".into());
    app.flash("going");
    assert!(!app.expire_toasts(Instant::now()));
    assert!(app.expire_toasts(Instant::now() + Duration::from_secs(5)));
}

/// Review repro: on a narrow pane the keys on an approval's row bring its
/// verdicts, a line each, into sight.
#[test]
fn a_narrow_approvals_verdicts_come_into_sight() {
    let store = Store::new();
    store.open("approval", "today", "Move?", |open| {
        open.rows = (0..6).map(|n| step(&format!("r{n}"), "")).collect();
    });
    let mut app = store.app();
    app.strip.shown = false;
    press(&mut app, KeyCode::Char('a'));
    app.scroll = 0;
    for _ in 0..5 {
        press(&mut app, KeyCode::Char('j'));
        screen(&mut app, 30, 24);
    }
    let shown = screen(&mut app, 30, 24);
    let row = shown
        .lines()
        .position(|l| l.contains("▸ r5"))
        .expect(&shown);
    assert!(
        shown.lines().skip(row).any(|l| l.contains("○ reject")),
        "{shown}"
    );
}

/// Review repro: once discard is gone, as what was chosen was taken back,
/// the keys are not left on it.
#[test]
fn the_keys_leave_a_discard_that_is_gone() {
    use super::form::{Press, Spot};
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('o'));
    press(&mut app, KeyCode::Esc);
    // From the note: send, drop, then discard.
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    let form = app.form.as_mut().unwrap();
    assert_eq!(form.spot(), Spot::Button(Press::Discard));
    form.other = false;
    assert_ne!(form.spot(), Spot::Button(Press::Discard));
}

/// The palette's match: the typed characters in order, in any case; a
/// match from the start, then one with them together, first.
#[test]
fn the_palette_matches_in_order_and_ranks_starts_first() {
    use super::actions::{Action, fuzzy, ranked};
    assert_eq!(fuzzy("", "Answer"), Some(0));
    assert_eq!(fuzzy("ANS", "Answer"), Some(0));
    assert!(fuzzy("pk", "Park").is_some());
    assert_eq!(fuzzy("kp", "Park"), None);
    assert_eq!(fuzzy("x", "Park"), None);
    assert!(fuzzy("pa", "Park") < fuzzy("pk", "Park"), "together first");
    assert!(
        fuzzy("s", "Stats") < fuzzy("s", "First case"),
        "from the start first"
    );
    let store = Store::new();
    store.open("stuck", "today", "Stuck here", |_| {});
    let app = store.app();
    // Nothing typed: the table's order, the case's acts first.
    let all = ranked(&app, "");
    assert_eq!(all[..2], [Action::Answer, Action::Park]);
    assert_eq!(ranked(&app, "par")[0], Action::Park);
    assert_eq!(ranked(&app, "show d")[0], Action::Filter(Filter::Done));
    assert!(ranked(&app, "zzz").is_empty());
}

/// What fits, by kind, state and screen, is what the bar offers and the
/// keys do: each act that fits is done by its key, and one that does not
/// fit does nothing but say why.
#[test]
fn the_actions_that_fit_are_the_bars_and_the_keys() {
    use super::actions::Action;
    let (store, verbs) = every_state();
    for (key, want) in verbs {
        let mut app = store.app();
        app.filter = Filter::All;
        app.selected = Some(key);
        let acts: Vec<Verb> = Action::fitting(&app)
            .into_iter()
            .filter_map(Action::verb)
            .collect();
        assert_eq!(acts, want, "{key}");
        assert_eq!(app.verbs(), want, "{key}");
        for action in Action::ALL.into_iter().filter(|a| a.verb().is_some()) {
            let mut app = store.app();
            app.filter = Filter::All;
            app.selected = Some(key);
            app.key(KeyEvent::from(action.code()));
            let acted = app.form.is_some() || app.compose.is_some() || app.pending.is_some();
            assert_eq!(
                acted,
                want.contains(&action.verb().unwrap()),
                "{action:?} on {key}"
            );
        }
    }
    // The rest: filters but the one shown, fold only on briefings, first
    // and last with a list, and help and quit always.
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let app = store.app();
    let fits = Action::fitting(&app);
    assert!(!fits.contains(&Action::Filter(Filter::Inbox)));
    assert!(fits.contains(&Action::Filter(Filter::Done)));
    assert!(!fits.contains(&Action::Fold));
    for action in [
        Action::First,
        Action::Last,
        Action::Help,
        Action::Quit,
        Action::Message,
    ] {
        assert!(fits.contains(&action), "{action:?}");
    }
    // One case's screen: only its own.
    let stuck = store.open("stuck", "today", "Stuck", |_| {});
    let mut one = App::one("cases.db".into(), stuck);
    read(&mut one, &mut store.db.poller(), &store.db).unwrap();
    assert_eq!(
        Action::fitting(&one),
        [Action::Answer, Action::Park, Action::Help, Action::Quit]
    );
}

/// ^k opens the palette; typing finds, the arrows choose, enter runs,
/// esc closes. A click runs a row; a click off it closes it. It names
/// the case it acts on (TU-12).
#[test]
fn the_palette_finds_chooses_and_runs() {
    use super::form::Act;
    let store = Store::new();
    store.open("stuck", "today", "Cannot reach the cache", |_| {});
    let mut app = store.app();
    ctrl(&mut app, 'k');
    assert!(app.palette.is_some());
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("on: Cannot reach the cache"), "{shown}");
    assert!(shown.contains("commands"), "{shown}");
    typing(&mut app, "park");
    press(&mut app, KeyCode::Enter);
    assert!(app.palette.is_none());
    assert_eq!(app.form.as_ref().map(|f| f.act), Some(Act::Park));
    // Out of its field, then closed.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    // The arrows, then esc.
    ctrl(&mut app, 'k');
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Esc);
    assert!(app.palette.is_none() && app.form.is_none());
    // By click: a row runs; hover lights it.
    ctrl(&mut app, 'k');
    let shown = screen(&mut app, 120, 40);
    let at = spot(&shown, "Show done");
    mouse(&mut app, MouseEventKind::Moved, at);
    assert_eq!(bg_at(&mut app, at), view::HOVER_BG, "not lit");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
    assert_eq!(app.filter, Filter::Done);
    assert!(app.palette.is_none());
    // A click off it closes it, and does nothing under it.
    ctrl(&mut app, 'k');
    screen(&mut app, 120, 40);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), (1, 1));
    assert!(app.palette.is_none());
    assert_eq!(app.filter, Filter::Done, "the click went through");
    // The button opens it too.
    click(&mut app, "^k Commands");
    assert!(app.palette.is_some());
}

/// ^k is the field's while one is typed in, and opens nothing over a
/// question.
#[test]
fn ctrl_k_opens_nothing_while_typing_or_asked() {
    let store = Store::new();
    store.open("question", "today", "Name?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('a'));
    assert!(app.form.as_ref().is_some_and(|f| f.editing.is_some()));
    ctrl(&mut app, 'k');
    assert!(app.palette.is_none(), "opened while typing");
    typing(&mut app, "x");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.confirm_quit);
    ctrl(&mut app, 'k');
    assert!(app.palette.is_none(), "opened over a question");
}

/// A right-click on a row selects it and opens its menu at the pointer,
/// kept inside the screen; a click or enter runs an item, esc or a click
/// off it closes it, and a right-click elsewhere does nothing (TU-12).
#[test]
fn a_right_click_on_a_row_opens_its_menu() {
    use super::form::Act;
    let store = Store::new();
    store.open("decision", "blocking", "First?", decision);
    let stuck = store.open("stuck", "today", "Stuck here", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 80, 24);
    let row = spot(&shown, "Stuck here");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), row);
    assert_eq!(app.selected, Some(stuck));
    assert!(app.menu.is_some());
    let shown = screen(&mut app, 80, 24);
    assert!(
        shown.contains("Answer") && shown.contains("Park"),
        "{shown}"
    );
    // Enter runs the item the keys are on: answer, the first.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.form.as_ref().map(|f| f.act), Some(Act::Park));
    app.form = None;
    // Esc, and a click off it, close it.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), row);
    press(&mut app, KeyCode::Esc);
    assert!(app.menu.is_none());
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), row);
    screen(&mut app, 80, 24);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), (79, 0));
    assert!(app.menu.is_none());
    // A click runs one, lit under the pointer.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), row);
    let shown = screen(&mut app, 80, 24);
    let park = shown
        .lines()
        .enumerate()
        .filter_map(|(y, l)| {
            l.find("Park").map(|x| {
                (
                    u16::try_from(l[..x].chars().count()).unwrap(),
                    u16::try_from(y).unwrap(),
                )
            })
        })
        .last()
        .unwrap();
    mouse(&mut app, MouseEventKind::Moved, park);
    assert_eq!(bg_in(&mut app, (80, 24), park), view::HOVER_BG);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), park);
    assert_eq!(app.form.as_ref().map(|f| f.act), Some(Act::Park));
    app.form = None;
    // Elsewhere, a right-click does nothing.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), (2, 0));
    assert!(app.menu.is_none());
    // At the screen's corner, the menu is kept inside it.
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let rect = view::menu_rect(area, ratatui::layout::Position::new(78, 23), 20, 5);
    assert_eq!(
        (rect.x, rect.y, rect.right(), rect.bottom()),
        (60, 19, 80, 24)
    );
    let rect = view::menu_rect(area, ratatui::layout::Position::new(3, 4), 20, 5);
    assert_eq!((rect.x, rect.y), (3, 5), "below and right of the pointer");
}

/// One case's screen has the palette, with only its own actions, and no
/// menu, as it has no list.
#[test]
fn one_cases_screen_has_the_palette_and_no_menu() {
    use super::actions::{Action, ranked};
    let store = Store::new();
    let key = store.open("question", "today", "Name?", |_| {});
    let mut app = App::one("cases.db".into(), key);
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    let shown = screen(&mut app, 80, 24);
    assert!(
        shown.lines().next().unwrap().contains("^k Commands"),
        "{shown}"
    );
    ctrl(&mut app, 'k');
    assert_eq!(
        ranked(&app, ""),
        [Action::Answer, Action::Help, Action::Quit]
    );
    press(&mut app, KeyCode::Esc);
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Right),
        spot(&shown, "Name?"),
    );
    assert!(app.menu.is_none());
}

/// Review repro: a row's menu acts on the case it was opened on, or on
/// none: when a read moves the selection off it, it closes.
#[test]
fn a_menu_closes_when_its_case_leaves() {
    let store = Store::new();
    let first = store.open("stuck", "blocking", "First stuck", |_| {});
    store.open("stuck", "today", "Second stuck", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 80, 24);
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Right),
        spot(&shown, "First stuck"),
    );
    assert_eq!(app.selected, Some(first));
    store.answer(first, |a| "elsewhere".clone_into(&mut a.text));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(app.menu.is_none(), "the menu stayed open off its case");
    // Enter is the list's again: into the second case's answer.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(
        app.form.as_ref().is_some_and(|form| form.key != first),
        "the menu acted on another case"
    );
}

/// Review repro: when fewer actions fit than the one the palette's keys
/// were on, enter runs the last, which is the one marked.
#[test]
fn the_palette_runs_the_marked_action_when_fewer_fit() {
    use super::form::Act;
    let store = Store::new();
    let key = store.open("stuck", "today", "Stuck", |_| {});
    let mut app = store.app();
    ctrl(&mut app, 'k');
    typing(&mut app, "a");
    let before = super::actions::ranked(&app, "a").len();
    for _ in 0..before {
        press(&mut app, KeyCode::Down);
    }
    store.answer(key, |a| "elsewhere".clone_into(&mut a.text));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    assert!(super::actions::ranked(&app, "a").len() < before);
    press(&mut app, KeyCode::Enter);
    assert!(app.palette.is_none(), "enter did nothing");
    assert_ne!(app.form.as_ref().map(|f| f.act), Some(Act::Answer));
}

/// Review repro: a click on a menu's item runs what was drawn there, or
/// nothing, never another that took its place.
#[test]
fn a_click_on_a_menu_item_runs_what_was_drawn() {
    let store = Store::new();
    let key = store.open("stuck", "today", "Stuck", |_| {});
    let mut app = store.app();
    let shown = screen(&mut app, 80, 24);
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Right),
        spot(&shown, "Stuck"),
    );
    let shown = screen(&mut app, 80, 24);
    // Answer is the menu's first item, drawn below the row.
    let answer = shown
        .lines()
        .enumerate()
        .filter_map(|(y, l)| {
            l.find("Answer ").map(|x| {
                (
                    u16::try_from(l[..x].chars().count()).unwrap(),
                    u16::try_from(y).unwrap(),
                )
            })
        })
        .last()
        .unwrap();
    // Parked elsewhere: answer no longer fits, resume does.
    store.human(key, &Record::Park(cases_core::Park::default()));
    read(&mut app, &mut store.db.poller(), &store.db).unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), answer);
    assert!(app.pending.is_none(), "a click on answer resumed the case");
}

/// A step of the keys a test types.
#[derive(Clone, Copy)]
enum Typed {
    Key(KeyCode),
    Ctrl(char),
    Text(&'static str),
}

/// Types `steps`.
fn type_in(app: &mut App, steps: &[Typed]) {
    for step in steps {
        match step {
            Typed::Key(code) => press(app, *code),
            Typed::Ctrl(c) => ctrl(app, *c),
            Typed::Text(text) => typing(app, text),
        }
    }
}

/// The answer sent by the new keys (TU-12), a number or n or enter from
/// the list and enter to send, is the answer the form's own keys send
/// after a: the same events in the store, kind by kind.
#[test]
fn an_answer_by_the_new_keys_records_what_the_old_keys_record() {
    use KeyCode::{Char, Enter};
    use Typed::{Ctrl, Key, Text};
    let open = |store: &Store, kind: &str| match kind {
        "decision" => store.open("decision", "today", "Pin?", decision),
        "approval" => store.open("approval", "today", "Move?", |open| {
            open.rows = vec![step("tag", ""), step("brew", "")];
        }),
        "briefing" => briefing(store, "today", "Parser split", true, &["docs"]),
        kind => store.open(kind, "today", "Well?", |_| {}),
    };
    let answers = |kind: &str, steps: &[Typed]| {
        let store = Store::new();
        let key = open(&store, kind);
        let mut app = store.app();
        if kind == "briefing" {
            show(&mut app, Filter::Briefings);
        }
        type_in(&mut app, steps);
        let pending = app.pending.as_ref().map(|send| send.record.clone());
        assert!(pending.is_some(), "{kind}: nothing to send");
        send(&mut app, &store);
        let mut answer = case(&store, key).answer().cloned().unwrap();
        answer.answered_at = None;
        answer.actor = None;
        (pending, answer)
    };
    let a = Key(Char('a'));
    let cases: &[(&str, &[Typed], &[Typed])] = &[
        (
            "decision",
            &[a, Key(Char('2')), Key(Char('n')), Text("fine"), Ctrl('s')],
            &[Key(Char('2')), Key(Char('n')), Text("fine"), Key(Enter)],
        ),
        // ↓ 2 ⏎: down the list, then the option, then send.
        (
            "decision",
            &[a, Key(Char('2')), Ctrl('s')],
            &[Key(KeyCode::Down), Key(Char('2')), Key(Enter)],
        ),
        (
            "decision",
            &[a, Key(Char('o')), Text("neither"), Ctrl('s')],
            &[Key(Char('3')), Text("neither"), Key(Enter)],
        ),
        (
            "signoff",
            &[a, Key(Char('y')), Ctrl('s')],
            &[Key(Char('1')), Key(Enter)],
        ),
        (
            "signoff",
            &[a, Key(Char('c')), Text("redo"), Ctrl('s')],
            &[Key(Char('2')), Key(Char('n')), Text("redo"), Key(Enter)],
        ),
        (
            "briefing",
            &[a, Key(Char('s')), Text("why"), Ctrl('s')],
            &[Key(Char('2')), Key(Char('n')), Text("why"), Key(Enter)],
        ),
        (
            "approval",
            &[a, Key(Char('y')), Key(Char('x')), Ctrl('s')],
            &[
                Key(Char('2')),
                Key(Char('x')),
                Key(Char('1')),
                Key(Char('y')),
                Key(Enter),
            ],
        ),
        (
            "question",
            &[a, Text("yes"), Ctrl('s')],
            &[Key(Enter), Text("yes"), Key(Enter)],
        ),
        ("fyi", &[a, Ctrl('s')], &[Key(Enter), Key(Enter)]),
        (
            "fyi",
            &[a, Key(Char('n')), Text("seen"), Ctrl('s')],
            &[Key(Char('n')), Text("seen"), Key(Enter)],
        ),
    ];
    for (kind, old, new) in cases {
        assert_eq!(answers(kind, new), answers(kind, old), "{kind}");
    }
}

/// Tab goes round one ring per kind (TU-12): the list, the answer's
/// parts in order, then the list. A case with no answer to give has the
/// case's bar's buttons in the ring instead, and enter presses one.
#[test]
fn the_tab_ring_follows_the_kind() {
    use super::form::{Press, Spot};
    let ring = |kind: &str| {
        let store = Store::new();
        match kind {
            "decision" => store.open(kind, "today", "Pin?", decision),
            "approval" => store.open(kind, "today", "Move?", |open| {
                open.rows = vec![step("tag", "")];
            }),
            "briefing" => briefing(&store, "today", "Split", true, &["docs"]),
            _ => store.open(kind, "today", "Well?", |_| {}),
        };
        let mut app = store.app();
        if kind == "briefing" {
            show(&mut app, Filter::Briefings);
        }
        let mut parts = Vec::new();
        loop {
            press(&mut app, KeyCode::Tab);
            match app.form.as_ref() {
                Some(form) => parts.push(form.spot()),
                None => break,
            }
            assert!(parts.len() < 10, "{kind}: no way out");
        }
        assert_eq!(app.focus, Focus::List, "{kind}");
        parts
    };
    let note = Spot::Field(Which::Note);
    let [send, park, drop] = [Press::Send, Press::Park, Press::Drop].map(Spot::Button);
    assert_eq!(ring("decision"), [Spot::Choices, note, send, drop]);
    assert_eq!(ring("signoff"), [Spot::Choices, note, send, drop]);
    assert_eq!(
        ring("briefing"),
        [Spot::Choices, Spot::Items, note, send, drop]
    );
    assert_eq!(ring("approval"), [Spot::Rows, note, send, drop]);
    assert_eq!(
        ring("question"),
        [Spot::Field(Which::Text), note, send, drop]
    );
    assert_eq!(
        ring("stuck"),
        [Spot::Field(Which::Text), note, send, park, drop]
    );
    assert_eq!(ring("fyi"), [note, send, drop]);

    // A message has no answer: the bar's buttons are the ring.
    let store = Store::new();
    message(&store, "A message");
    let mut app = store.app();
    show(&mut app, Filter::Flight);
    let mut stops = Vec::new();
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
        stops.push(app.focus);
    }
    assert_eq!(
        stops,
        [
            Focus::Bar(Verb::Withdraw),
            Focus::Bar(Verb::Edit),
            Focus::List
        ]
    );
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Bar(Verb::Edit));
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("▸ Edit"), "{shown}");
    assert!(shown.contains("on: Edit · tab → list"), "{shown}");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::List);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.form.as_ref().map(|f| f.act),
        Some(super::form::Act::Withdraw)
    );
}

/// A number answers the selected case from the list, with the keys put
/// on what it picked there and the case scrolled to it; n puts them in
/// the note (TU-12).
#[test]
fn a_number_from_the_list_answers_and_puts_the_keys_on_it() {
    use super::form::Spot;
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    press(&mut app, KeyCode::Char('2'));
    let form = app.form.as_ref().unwrap();
    assert_eq!((form.choice, form.spot()), (Some(2), Spot::Choices));
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("▸ ● 2  Float it"), "{shown}");
    assert!(shown.contains("on: option 2 · tab → note"), "{shown}");
    // Its number past the options picks other, with the keys in the note.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('3'));
    let form = app.form.as_ref().unwrap();
    assert!(form.other && form.editing == Some(Which::Note));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    // One past that does nothing, and leaves the keys in the list.
    press(&mut app, KeyCode::Char('4'));
    assert!(app.form.is_none());
    assert_eq!(app.focus, Focus::List);

    let store = Store::new();
    store.open("signoff", "today", "Ship?", |_| {});
    let mut app = store.app();
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(app.form.as_ref().unwrap().signoff, Some("changes"));
    // Sending changes with no note still refuses.
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_none());
    assert!(flash(&app).contains("needs a note"), "{}", flash(&app));

    let store = Store::new();
    store.open("approval", "today", "Move?", |open| {
        open.rows = (0..12).map(|n| step(&format!("r{n}"), "")).collect();
    });
    let mut app = store.app();
    screen(&mut app, 120, 20);
    press(&mut app, KeyCode::Char('9'));
    let form = app.form.as_ref().unwrap();
    assert_eq!((form.row, form.spot()), (8, Spot::Rows));
    // The case scrolled to the row, its verdicts in sight.
    let shown = screen(&mut app, 120, 20);
    assert!(shown.contains("▸ r8"), "{shown}");
    press(&mut app, KeyCode::Char('h'));
    assert_eq!(app.form.as_ref().unwrap().rows[8].verdict, Some("hold"));

    // A case with nothing to answer says so.
    let store = Store::new();
    message(&store, "A message");
    let mut app = store.app();
    show(&mut app, Filter::Flight);
    press(&mut app, KeyCode::Char('1'));
    assert!(app.form.is_none());
    assert!(flash(&app).contains("is a message"), "{}", flash(&app));
}

/// The keys' place is always drawn (TU-12): the region with them has an
/// edge and says so, the others are dim, and the part of the answer they
/// are on is filled, apart from the pointer's light and a chosen dot.
#[test]
fn the_focus_is_drawn_on_each_region_and_part() {
    let store = Store::new();
    store.open("decision", "today", "Pin?", decision);
    let mut app = store.app();
    let title = |app: &mut App| {
        let shown = screen(app, 120, 40);
        let has = |text: &str| shown.contains(text);
        (
            has("▍List · keys here"),
            has("▍Case · keys here"),
            has("▍YOUR ANSWER · keys here"),
        )
    };
    assert_eq!(title(&mut app), (true, false, false));
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(title(&mut app), (false, true, false));
    press(&mut app, KeyCode::Char('h'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(title(&mut app), (false, false, true));
    // The option the keys are on is filled across its line.
    let shown = screen(&mut app, 120, 40);
    let (x, y) = spot(&shown, "1  Pin it");
    assert_eq!(bg_at(&mut app, (x, y)), view::FOCUS_BG);
    assert_eq!(bg_at(&mut app, (x + 30, y)), view::FOCUS_BG);
    let (x, y) = spot(&shown, "2  Float it");
    assert_ne!(bg_at(&mut app, (x, y)), view::FOCUS_BG);
    // The pointer over another part lights it, not as the focus.
    mouse(&mut app, MouseEventKind::Moved, (x, y));
    assert_eq!(bg_at(&mut app, (x, y)), view::HOVER_BG);
    // Then the note, filled and pointed at, with the cursor in it.
    press(&mut app, KeyCode::Tab);
    let shown = screen(&mut app, 120, 40);
    let (x, y) = spot(&shown, "▸ note");
    assert_eq!(bg_at(&mut app, (x + 2, y)), view::FOCUS_BG);
    assert!(shown.contains("on: note · tab → Send"), "{shown}");
    // Then a button: pointed at by its own face, not filled.
    press(&mut app, KeyCode::Tab);
    let shown = screen(&mut app, 120, 40);
    assert!(shown.contains("▸ ⏎ Send"), "{shown}");
    assert!(shown.contains("on: Send · tab → Drop"), "{shown}");
    // An approval's row: its line and its verdicts.
    let store = Store::new();
    store.open("approval", "today", "Move?", |open| {
        open.rows = vec![step("tag", ""), step("brew", "")];
    });
    let mut app = store.app();
    press(&mut app, KeyCode::Char('2'));
    let shown = screen(&mut app, 120, 40);
    let (x, y) = spot(&shown, "▸ brew");
    assert_eq!(bg_at(&mut app, (x, y)), view::FOCUS_BG);
    assert_eq!(bg_at(&mut app, (x + 4, y + 1)), view::FOCUS_BG);
    let (x, y) = spot(&shown, "tag  Run tag");
    assert_ne!(bg_at(&mut app, (x, y)), view::FOCUS_BG);
}

/// Whenever a case with an answer to give is selected, the footer leads
/// with where the keys are, where tab goes, and the answer's keys
/// (TU-12).
#[test]
fn the_footer_leads_with_the_keys_place_and_the_answer() {
    let footer = |kind: &str| {
        let store = Store::new();
        match kind {
            "decision" => store.open(kind, "today", "Pin?", decision),
            "approval" => store.open(kind, "today", "Move?", |open| {
                open.rows = vec![step("tag", ""), step("brew", "")];
            }),
            _ => store.open(kind, "today", "Well?", |_| {}),
        };
        let mut app = store.app();
        let shown = screen(&mut app, 200, 30);
        shown.lines().last().unwrap().trim().to_string()
    };
    let rest = "j/k move   [ ] filter   s stats   ^k commands   ? keys   q quit";
    for (kind, lead) in [
        (
            "decision",
            "on: list · tab → options │ 1-3 choose   n note   ⏎ answer",
        ),
        (
            "signoff",
            "on: list · tab → answers │ 1 accept   2 changes   n note   ⏎ answer",
        ),
        (
            "approval",
            "on: list · tab → rows │ 1-2 row   n note   ⏎ answer",
        ),
        ("question", "on: list · tab → reply │ n note   ⏎ answer"),
        ("fyi", "on: list · tab → note │ n note   ⏎ answer"),
    ] {
        assert_eq!(footer(kind), format!("{lead}   {rest}"), "{kind}");
    }
}

/// Review repro: a paste goes into the field being typed in whole, its
/// new lines too, rather than an enter in it sending; in a one-line part
/// of a message its new lines are spaces; and in no field it is dropped,
/// not read as keys.
#[test]
fn a_paste_goes_into_the_field_being_typed_in() {
    let store = Store::new();
    store.open("stuck", "today", "Stuck", |_| {});
    let mut app = store.app();
    app.paste("q");
    assert!(!app.quit && app.form.is_none(), "a paste was read as keys");
    press(&mut app, KeyCode::Enter);
    app.paste("use\r\nthe mirror");
    assert!(app.pending.is_none());
    let form = app.form.as_ref().unwrap();
    assert_eq!(form.text.text(), "use\nthe mirror");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('m'));
    app.paste("two\nlines");
    let compose = app.compose.as_mut().unwrap();
    let part = compose.part.unwrap();
    assert_ne!(part, super::compose::Part::Body);
    assert_eq!(compose.field(part).text(), "two lines");
}
