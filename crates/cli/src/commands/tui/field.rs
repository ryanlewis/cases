//! A text field: a line or several, typed in place, with a cursor that moves
//! by character and word (TU-06).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The text and where the cursor is in it, as a character index.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    text: String,
    cursor: usize,
}

impl Field {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The cursor, as a character index into the text.
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replaces the text, as the editor gives it back, with the cursor at
    /// its end.
    pub(crate) fn set(&mut self, text: String) {
        self.cursor = text.chars().count();
        self.text = text;
    }

    pub(crate) fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The byte offset of character `at`.
    fn byte(&self, at: usize) -> usize {
        self.text
            .char_indices()
            .nth(at)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn insert(&mut self, c: char) {
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    /// The start of the word before the cursor.
    fn word_back(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor;
        while at > 0 && chars[at - 1].is_whitespace() {
            at -= 1;
        }
        while at > 0 && !chars[at - 1].is_whitespace() {
            at -= 1;
        }
        at
    }

    /// Takes a paste: its text at the cursor, a new line as one, or as a
    /// space when the field is `one_line`.
    pub(crate) fn paste(&mut self, text: &str, one_line: bool) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for c in text.chars() {
            self.insert(if one_line && c == '\n' { ' ' } else { c });
        }
    }

    /// Takes a key the field understands, and says whether it did. Enter
    /// starts a new line.
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        let altgr = altgr(key, cfg!(windows));
        let ctrl = !altgr && key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = !altgr && key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('w') if ctrl => {
                let from = self.word_back();
                let (start, end) = (self.byte(from), self.byte(self.cursor));
                self.text.replace_range(start..end, "");
                self.cursor = from;
            }
            KeyCode::Char('u') if ctrl => {
                let end = self.byte(self.cursor);
                self.text.replace_range(..end, "");
                self.cursor = 0;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if alt => self.cursor = self.len(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c),
            KeyCode::Enter => self.insert('\n'),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < self.len() => {
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Left if alt || ctrl => self.cursor = self.word_back(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }
}

/// Whether `key` is a character typed with `AltGr`. Windows reports `AltGr`
/// as Ctrl and Alt held, so there a character with just those, and Shift,
/// is text. Elsewhere Ctrl and Alt are a chord.
fn altgr(key: KeyEvent, windows: bool) -> bool {
    windows
        && matches!(key.code, KeyCode::Char(_))
        && key.modifiers.difference(KeyModifiers::SHIFT)
            == KeyModifiers::CONTROL | KeyModifiers::ALT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &[KeyEvent]) -> Field {
        let mut field = Field::default();
        for key in keys {
            field.key(*key);
        }
        field
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_edits_at_the_cursor() {
        let mut keys: Vec<KeyEvent> = "pin bun".chars().map(|c| k(KeyCode::Char(c))).collect();
        keys.extend([k(KeyCode::Home), k(KeyCode::Char('>')), k(KeyCode::End)]);
        keys.extend([
            k(KeyCode::Backspace),
            k(KeyCode::Enter),
            k(KeyCode::Char('é')),
        ]);
        let field = typed(&keys);
        assert_eq!(field.text(), ">pin bu\né");
        assert_eq!(field.cursor(), 9);

        let mut field = typed(
            &"one two  "
                .chars()
                .map(|c| k(KeyCode::Char(c)))
                .collect::<Vec<_>>(),
        );
        field.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(field.text(), "one ");
        field.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!((field.text(), field.cursor()), ("", 0));
        assert!(field.is_blank());
    }

    fn at(text: &str, cursor: usize) -> Field {
        Field {
            text: text.into(),
            cursor,
        }
    }

    #[test]
    fn each_key_moves_or_edits_at_the_cursor_and_says_if_it_took_it() {
        let (none, ctrl, alt) = (KeyModifiers::NONE, KeyModifiers::CONTROL, KeyModifiers::ALT);
        let shift = KeyModifiers::SHIFT;
        // (text, cursor, key, modifiers) -> (text, cursor, taken)
        let cases: &[(&str, usize, KeyCode, KeyModifiers, &str, usize, bool)] = &[
            ("ab cd", 3, KeyCode::Char('a'), ctrl, "ab cd", 0, true),
            ("ab cd", 1, KeyCode::Char('e'), alt, "ab cd", 5, true),
            ("ab", 1, KeyCode::Char('e'), none, "aeb", 2, true),
            ("ab", 1, KeyCode::Char('E'), shift, "aEb", 2, true),
            ("ab", 1, KeyCode::Char('x'), ctrl, "ab", 1, false),
            ("ab", 1, KeyCode::Char('x'), alt, "ab", 1, false),
            ("ab", 1, KeyCode::Char('a'), alt, "ab", 1, false),
            ("ab", 1, KeyCode::Char('e'), ctrl, "ab", 1, false),
            ("ab", 1, KeyCode::Char('w'), alt, "ab", 1, false),
            ("ab", 1, KeyCode::Char('u'), alt, "ab", 1, false),
            ("éa", 1, KeyCode::Backspace, none, "a", 0, true),
            ("ab", 0, KeyCode::Backspace, none, "ab", 0, false),
            ("aéb", 1, KeyCode::Delete, none, "ab", 1, true),
            ("ab", 2, KeyCode::Delete, none, "ab", 2, false),
            ("one two", 6, KeyCode::Left, alt, "one two", 4, true),
            ("one two", 4, KeyCode::Left, ctrl, "one two", 0, true),
            ("ab", 2, KeyCode::Left, none, "ab", 1, true),
            ("ab", 0, KeyCode::Left, none, "ab", 0, true),
            ("ab", 1, KeyCode::Right, none, "ab", 2, true),
            ("ab", 2, KeyCode::Right, none, "ab", 2, true),
            ("ab", 2, KeyCode::Home, none, "ab", 0, true),
            ("aé", 0, KeyCode::End, none, "aé", 2, true),
            ("ab", 1, KeyCode::Enter, none, "a\nb", 2, true),
            ("ab", 1, KeyCode::Tab, none, "ab", 1, false),
            ("ab", 1, KeyCode::Esc, none, "ab", 1, false),
            ("ab", 1, KeyCode::Up, none, "ab", 1, false),
            // Ctrl-W from the middle of a word stops at its start; Ctrl-U
            // takes everything before the cursor, other lines too.
            ("one two", 5, KeyCode::Char('w'), ctrl, "one wo", 4, true),
            ("one  two", 5, KeyCode::Char('w'), ctrl, "two", 0, true),
            ("ab", 0, KeyCode::Char('w'), ctrl, "ab", 0, true),
            ("ab cd", 4, KeyCode::Char('u'), ctrl, "d", 0, true),
        ];
        for &(text, cursor, code, modifiers, want, want_at, taken) in cases {
            let mut field = at(text, cursor);
            let took = field.key(KeyEvent::new(code, modifiers));
            assert_eq!(
                (field.text(), field.cursor(), took),
                (want, want_at, taken),
                "{code:?} {modifiers:?} on {text:?} at {cursor}"
            );
        }
    }

    #[test]
    fn set_puts_the_cursor_at_the_end_in_characters() {
        let mut field = at("x", 0);
        field.set("née\n".into());
        assert_eq!((field.text(), field.cursor()), ("née\n", 4));
        assert!(!field.is_blank());
        field.set(" \n\t".into());
        assert!(field.is_blank());
    }

    #[test]
    fn only_on_windows_is_a_character_with_ctrl_and_alt_altgr() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let cases = [
            (KeyCode::Char('@'), altgr, true),
            (KeyCode::Char('Ł'), altgr | KeyModifiers::SHIFT, true),
            (KeyCode::Char('x'), altgr | KeyModifiers::SUPER, false),
            (KeyCode::Char('x'), KeyModifiers::CONTROL, false),
            (KeyCode::Char('x'), KeyModifiers::ALT, false),
            (KeyCode::Char('x'), KeyModifiers::SHIFT, false),
            (KeyCode::Left, altgr, false),
        ];
        for (code, modifiers, windows) in cases {
            let key = KeyEvent::new(code, modifiers);
            assert_eq!(super::altgr(key, true), windows, "{code:?} {modifiers:?}");
            assert!(!super::altgr(key, false), "{code:?} {modifiers:?}");
        }
    }

    #[test]
    fn ctrl_and_alt_type_text_on_windows_and_are_a_chord_elsewhere() {
        // (text, cursor, key, modifiers) -> (text, cursor, taken) on
        // Windows, then elsewhere.
        type Want = (&'static str, usize, bool);
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let cases: &[(&str, usize, char, KeyModifiers, Want, Want)] = &[
            ("ab", 1, '@', altgr, ("a@b", 2, true), ("ab", 1, false)),
            (
                "ab",
                1,
                'Ł',
                altgr | KeyModifiers::SHIFT,
                ("aŁb", 2, true),
                ("ab", 1, false),
            ),
            ("ab", 1, 'w', altgr, ("awb", 2, true), ("b", 0, true)),
            ("ab", 1, 'a', altgr, ("aab", 2, true), ("ab", 0, true)),
            ("ab", 1, 'e', altgr, ("aeb", 2, true), ("ab", 2, true)),
        ];
        for &(text, cursor, c, modifiers, windows, elsewhere) in cases {
            let mut field = at(text, cursor);
            let took = field.key(KeyEvent::new(KeyCode::Char(c), modifiers));
            let want = if cfg!(windows) { windows } else { elsewhere };
            assert_eq!(
                (field.text(), field.cursor(), took),
                want,
                "{c:?} {modifiers:?} on {text:?} at {cursor}"
            );
        }
    }
}
