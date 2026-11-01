//! Text from the store, made safe to draw (TU-01): a control character is
//! shown as a visible placeholder, never sent to the terminal, so a case
//! cannot change the terminal's title, colours or clipboard. A tab becomes
//! spaces, since the terminal would move the cursor by its own rule.

use std::borrow::Cow;

/// `text` with each control character other than a line break shown as a
/// placeholder: the Unicode control picture for C0 and DEL (`␛` for escape),
/// and `�` for C1.
pub(crate) fn clean(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| c.is_control() && c != '\n') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            '\u{7f}' => out.push('\u{2421}'),
            c if u32::from(c) < 0x20 => {
                out.push(char::from_u32(0x2400 + u32::from(c)).unwrap_or('\u{fffd}'));
            }
            c if c.is_control() => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_shown_not_sent() {
        assert_eq!(clean("\x1b]0;owned\x07title"), "␛]0;owned␇title");
        assert_eq!(clean("a\tb\x7f\u{9b}c"), "a    b␡\u{fffd}c");
        assert_eq!(clean("two\nlines"), "two\nlines");
        assert!(matches!(clean("plain"), Cow::Borrowed(_)));
    }
}
