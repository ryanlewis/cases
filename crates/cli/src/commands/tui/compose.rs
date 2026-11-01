//! Writing a message to an agent (TU-07, HU-08): a new one to a worker or a
//! label, or an edit of the body of one sent and not yet picked up.

use cases_core::{Amend, Case, Kind, Open, Record, Since, Urgency};
use cases_store_sqlite::Key;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::field::Field;
use crate::commands::message;

/// Which field has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    To,
    Title,
    Body,
}

/// What a key did to the message.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Stay,
    /// Send this record: a new message, or an edit of `Compose::editing`.
    Send(Box<Record>),
    Cancel,
    Edit(Part),
    Missing(&'static str),
}

/// A message being written.
#[derive(Debug)]
pub(crate) struct Compose {
    /// The sent message being edited, and the revision it was read at.
    pub(crate) editing: Option<(Key, u64)>,
    /// The message's last seq at that revision: the rows above it are the
    /// ones since.
    seq: u64,
    pub(crate) to: Field,
    pub(crate) title: Field,
    pub(crate) body: Field,
    pub(crate) urgency: Urgency,
    pub(crate) part: Option<Part>,
    /// Throw away what was typed? Asked before a cancel loses it.
    pub(crate) confirm: bool,
    /// What the human had before an edit, so that an unchanged one sends
    /// nothing.
    before: String,
}

impl Compose {
    /// A new message, starting at who it is to.
    pub(crate) fn new() -> Compose {
        Compose {
            editing: None,
            seq: 0,
            to: Field::default(),
            title: Field::default(),
            body: Field::default(),
            urgency: Urgency::Today,
            part: Some(Part::To),
            confirm: false,
            before: String::new(),
        }
    }

    /// An edit of the sent message `key`: its body only, as `amend --as`
    /// changes it.
    pub(crate) fn edit(key: Key, case: &Case) -> Compose {
        let mut compose = Compose::new();
        compose.editing = Some((key, case.revision()));
        compose.seq = case.last_seq();
        compose
            .to
            .set(crate::ctx::addressee(case.worker(), case.labels()));
        compose.title.set(case.title().to_string());
        compose.body.set(case.body().to_string());
        compose.before = case.body().to_string();
        compose.part = Some(Part::Body);
        compose
    }

    pub(crate) fn field(&mut self, part: Part) -> &mut Field {
        match part {
            Part::To => &mut self.to,
            Part::Title => &mut self.title,
            Part::Body => &mut self.body,
        }
    }

    /// Follows the message being edited as a new read shows it (IN-05), by
    /// the core's freshness rule ([`Case::since`]): the edit takes the new
    /// revision when the message is unchanged since it was read, as after
    /// a link is added, and its body is the one the edit started from. An
    /// edit made elsewhere since keeps the old revision, so that this one
    /// is refused as stale rather than writing over it. A message that left
    /// sent closes the edit (`App::update`).
    pub(crate) fn follow(&mut self, case: &Case) {
        if let Some((_, revision)) = &mut self.editing
            && case.since(*revision, self.seq) == Since::Unchanged
            && case.amend_seq() <= self.seq
        {
            *revision = case.revision();
            self.seq = case.last_seq();
        }
    }

    /// Whether a cancel would lose anything typed.
    pub(crate) fn typed(&self) -> bool {
        match self.editing {
            Some(_) => self.body.text() != self.before,
            None => !self.to.is_blank() || !self.title.is_blank() || !self.body.is_blank(),
        }
    }

    /// Fills in who the message is to, from the picker.
    pub(crate) fn pick(&mut self, to: &str) {
        if self.editing.is_none() {
            self.to.set(to.to_string());
        }
    }

    pub(crate) fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.confirm {
            self.confirm = false;
            return if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                Outcome::Cancel
            } else {
                Outcome::Stay
            };
        }
        if ctrl && key.code == KeyCode::Char('s') {
            return self.send();
        }
        if ctrl && key.code == KeyCode::Char('e') {
            return Outcome::Edit(Part::Body);
        }
        if let Some(part) = self.part {
            match key.code {
                KeyCode::Esc => self.part = None,
                KeyCode::Tab if self.editing.is_none() => {
                    self.part = Some(match part {
                        Part::To => Part::Title,
                        Part::Title => Part::Body,
                        Part::Body => Part::To,
                    });
                }
                // Who it is to and the title are one line each.
                KeyCode::Enter if part != Part::Body => {
                    self.part = Some(if part == Part::To {
                        Part::Title
                    } else {
                        Part::Body
                    });
                }
                _ if self.editing.is_some() && part != Part::Body => {}
                _ => {
                    self.field(part).key(key);
                }
            }
            return Outcome::Stay;
        }
        let new = self.editing.is_none();
        match key.code {
            KeyCode::Esc if self.typed() => self.confirm = true,
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return self.send(),
            KeyCode::Char('t') if new => self.part = Some(Part::To),
            KeyCode::Char('i') if new => self.part = Some(Part::Title),
            KeyCode::Char('b' | 'n') => self.part = Some(Part::Body),
            KeyCode::Char('u') if new => {
                self.urgency = match self.urgency {
                    Urgency::Blocking => Urgency::Today,
                    Urgency::Today => Urgency::Whenever,
                    _ => Urgency::Blocking,
                };
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// The title the message goes with: the one typed, or the body's first
    /// line, as `cases message` takes it.
    pub(crate) fn title_to_send(&self) -> Option<String> {
        let title = self.title.text().trim();
        if title.is_empty() {
            message::title_of(self.body.text())
        } else {
            Some(title.to_string())
        }
    }

    fn send(&mut self) -> Outcome {
        let body = self.body.text().trim();
        if self.editing.is_some() {
            if body.is_empty() {
                return Outcome::Missing("the body is empty");
            }
            if body == self.before.trim() {
                return Outcome::Missing("the body is as it was");
            }
            let mut amend = Amend::default();
            body.clone_into(&mut amend.body);
            return Outcome::Send(Box::new(Record::Amend(amend)));
        }
        let to = self.to.text().trim();
        if to.is_empty() || to == "#" {
            return Outcome::Missing("say who it is to: a worker, or #label");
        }
        let Some(title) = self.title_to_send() else {
            return Outcome::Missing("write a title or a body");
        };
        let mut open = Open::default();
        Kind::Message.as_str().clone_into(&mut open.kind);
        self.urgency.as_str().clone_into(&mut open.urgency);
        open.title = title;
        body.clone_into(&mut open.body);
        match to.strip_prefix('#') {
            Some(label) => open.labels = vec![label.trim().to_string()],
            None => to.clone_into(&mut open.worker),
        }
        Outcome::Send(Box::new(Record::Open(open)))
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Author, Case, Time};

    use super::*;

    fn press(compose: &mut Compose, code: KeyCode) -> Outcome {
        compose.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(compose: &mut Compose, c: char) -> Outcome {
        compose.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn typing(compose: &mut Compose, text: &str) {
        for c in text.chars() {
            assert_eq!(press(compose, KeyCode::Char(c)), Outcome::Stay);
        }
    }

    /// An edit of a sent message to `worker`.
    fn edit() -> Compose {
        let mut open = Open::default();
        Kind::Message.as_str().clone_into(&mut open.kind);
        "today".clone_into(&mut open.urgency);
        "Rebase".clone_into(&mut open.title);
        "Rebase onto main.".clone_into(&mut open.body);
        "worker".clone_into(&mut open.worker);
        let append = Append::new(Author::Human, &Record::Open(open));
        let (case, _) = Case::open(&append, Time::from_unix(1, 0).unwrap(), None).unwrap();
        Compose::edit(Key::new(1).unwrap(), &case)
    }

    fn opened(outcome: Outcome) -> Open {
        match outcome {
            Outcome::Send(record) => match *record {
                Record::Open(open) => open,
                other => panic!("an open, not {other:?}"),
            },
            other => panic!("a send, not {other:?}"),
        }
    }

    #[test]
    fn tab_and_enter_go_from_field_to_field() {
        let mut compose = Compose::new();
        assert_eq!(compose.part, Some(Part::To));
        for want in [Part::Title, Part::Body, Part::To] {
            assert_eq!(press(&mut compose, KeyCode::Tab), Outcome::Stay);
            assert_eq!(compose.part, Some(want));
        }
        typing(&mut compose, "worker");
        press(&mut compose, KeyCode::Enter);
        assert_eq!(compose.part, Some(Part::Title));
        typing(&mut compose, "Rebase");
        press(&mut compose, KeyCode::Enter);
        assert_eq!(compose.part, Some(Part::Body));
        // The body takes Enter as a new line.
        typing(&mut compose, "one");
        press(&mut compose, KeyCode::Enter);
        typing(&mut compose, "two");
        assert_eq!(
            (compose.to.text(), compose.title.text(), compose.body.text()),
            ("worker", "Rebase", "one\ntwo")
        );
        assert_eq!(ctrl(&mut compose, 'e'), Outcome::Edit(Part::Body));
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Stay);
        assert_eq!(compose.part, None);
        assert_eq!(ctrl(&mut compose, 'e'), Outcome::Edit(Part::Body));

        // Out of the fields, a letter picks one; u steps the urgency.
        for (c, want) in [
            ('t', Part::To),
            ('i', Part::Title),
            ('b', Part::Body),
            ('n', Part::Body),
        ] {
            press(&mut compose, KeyCode::Char(c));
            assert_eq!(compose.part, Some(want), "{c}");
            press(&mut compose, KeyCode::Esc);
        }
        for want in [Urgency::Whenever, Urgency::Blocking, Urgency::Today] {
            press(&mut compose, KeyCode::Char('u'));
            assert_eq!(compose.urgency, want);
        }
        press(&mut compose, KeyCode::Char('x'));
        assert_eq!((compose.part, compose.confirm), (None, false));
    }

    #[test]
    fn a_new_message_needs_who_it_is_to_and_a_title_or_a_body() {
        let mut compose = Compose::new();
        assert_eq!(
            ctrl(&mut compose, 's'),
            Outcome::Missing("say who it is to: a worker, or #label")
        );
        typing(&mut compose, " # ");
        assert_eq!(
            ctrl(&mut compose, 's'),
            Outcome::Missing("say who it is to: a worker, or #label")
        );
        press(&mut compose, KeyCode::Home);
        press(&mut compose, KeyCode::Delete);
        press(&mut compose, KeyCode::Right);
        typing(&mut compose, "ops");
        assert_eq!(compose.to.text(), "#ops ");
        press(&mut compose, KeyCode::Esc);
        assert_eq!(
            press(&mut compose, KeyCode::Enter),
            Outcome::Missing("write a title or a body")
        );
        press(&mut compose, KeyCode::Char('b'));
        typing(&mut compose, " Rebase onto main. ");
        press(&mut compose, KeyCode::Esc);
        press(&mut compose, KeyCode::Char('u'));
        let open = opened(press(&mut compose, KeyCode::Enter));
        assert_eq!(open.kind, "message");
        assert_eq!(open.urgency, "whenever");
        assert_eq!(
            (open.title.as_str(), open.body.as_str()),
            ("Rebase onto main.", "Rebase onto main.")
        );
        assert_eq!(
            (open.worker.as_str(), open.labels),
            ("", vec!["ops".to_string()])
        );

        let mut compose = Compose::new();
        typing(&mut compose, "worker");
        press(&mut compose, KeyCode::Tab);
        typing(&mut compose, " Rebase ");
        let open = opened(ctrl(&mut compose, 's'));
        assert_eq!(
            (open.worker.as_str(), open.title.as_str()),
            ("worker", "Rebase")
        );
        assert_eq!((open.body.as_str(), open.labels.len()), ("", 0));
        assert_eq!(open.urgency, "today");
    }

    #[test]
    fn a_cancel_asks_first_when_something_was_typed() {
        let mut compose = Compose::new();
        press(&mut compose, KeyCode::Esc);
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Cancel);

        let mut compose = Compose::new();
        typing(&mut compose, "w");
        press(&mut compose, KeyCode::Esc);
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Stay);
        assert!(compose.confirm);
        // Any key but y keeps the message, and takes the question away.
        assert_eq!(press(&mut compose, KeyCode::Char('n')), Outcome::Stay);
        assert!(!compose.confirm);
        assert_eq!(compose.part, None);
        press(&mut compose, KeyCode::Esc);
        assert_eq!(ctrl(&mut compose, 's'), Outcome::Stay);
        assert!(!compose.confirm);
        press(&mut compose, KeyCode::Esc);
        assert_eq!(press(&mut compose, KeyCode::Char('Y')), Outcome::Cancel);

        // A body alone is enough to ask.
        let mut compose = Compose::new();
        press(&mut compose, KeyCode::Esc);
        press(&mut compose, KeyCode::Char('b'));
        typing(&mut compose, "w");
        press(&mut compose, KeyCode::Esc);
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Stay);
        assert!(compose.confirm);
    }

    #[test]
    fn an_edit_changes_the_body_only() {
        let mut compose = edit();
        assert_eq!(compose.part, Some(Part::Body));
        assert_eq!(compose.to.text(), "worker");
        // Tab does not leave the body, and is not typed into it.
        press(&mut compose, KeyCode::Tab);
        assert_eq!(compose.part, Some(Part::Body));
        assert_eq!(compose.body.text(), "Rebase onto main.");
        assert_eq!(
            ctrl(&mut compose, 's'),
            Outcome::Missing("the body is as it was")
        );
        press(&mut compose, KeyCode::Char(' '));
        assert_eq!(
            ctrl(&mut compose, 's'),
            Outcome::Missing("the body is as it was")
        );
        press(&mut compose, KeyCode::Esc);
        for c in ['t', 'i', 'u'] {
            press(&mut compose, KeyCode::Char(c));
            assert_eq!(
                (compose.part, compose.urgency),
                (None, Urgency::Today),
                "{c}"
            );
        }
        // The other fields do not take keys.
        compose.part = Some(Part::Title);
        typing(&mut compose, "x");
        assert_eq!(compose.title.text(), "Rebase");
        compose.part = Some(Part::To);
        press(&mut compose, KeyCode::Enter);
        assert_eq!(compose.part, Some(Part::Title));
        assert_eq!(compose.to.text(), "worker");

        // b or n goes back to the body; an unchanged body is cancelled at
        // once.
        for c in ['b', 'n'] {
            press(&mut compose, KeyCode::Esc);
            press(&mut compose, KeyCode::Char(c));
            assert_eq!(compose.part, Some(Part::Body), "{c}");
        }
        press(&mut compose, KeyCode::Backspace);
        press(&mut compose, KeyCode::Esc);
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Cancel);
        press(&mut compose, KeyCode::Char('b'));
        ctrl(&mut compose, 'u');
        assert_eq!(compose.body.text(), "");
        assert_eq!(
            ctrl(&mut compose, 's'),
            Outcome::Missing("the body is empty")
        );
        typing(&mut compose, " Merge it. ");
        let mut amend = Amend::default();
        "Merge it.".clone_into(&mut amend.body);
        assert_eq!(press(&mut compose, KeyCode::Esc), Outcome::Stay);
        assert_eq!(
            press(&mut compose, KeyCode::Enter),
            Outcome::Send(Box::new(Record::Amend(amend)))
        );
    }
}
