//! Keys: the number a store gives a case, which its public id encodes.

use core::fmt;
use core::str::FromStr;

/// A case's key: the number the store gives the case when it is opened.
///
/// Keys go up in the order cases are opened, and a store never gives a key
/// twice, not even one of an archived or deleted case. A key displays as the
/// case's public id, its [`cases_id`] encoding, such as `UkLWZ` for key 1, and
/// parses from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(u64);

impl Key {
    /// The largest key: 2⁶³ − 1, SQLite's largest row id, so that every
    /// store can hold every key.
    pub const MAX: u64 = i64::MAX.unsigned_abs();

    /// The key numbered `n`, when a store can give it: from 1 to
    /// [`Key::MAX`].
    #[must_use]
    pub fn new(n: u64) -> Option<Key> {
        (1..=Key::MAX).contains(&n).then_some(Key(n))
    }

    /// The key's number.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The case's public id.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&cases_id::encode(self.0))
    }
}

/// Reads a case's public id, and ignores a title slug after it, as in
/// `UkLWZ-pin-bun-or-float`. Text that is no key's id is [`UnknownId`]: a
/// case no store can have.
impl FromStr for Key {
    type Err = UnknownId;

    fn from_str(text: &str) -> Result<Key, UnknownId> {
        let (id, _slug) = cases_id::split(text);
        cases_id::decode(id).and_then(Key::new).ok_or(UnknownId)
    }
}

/// Text that is no case's id: it does not decode, or does not decode to the
/// id it is, or names a key no store gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownId;

impl fmt::Display for UnknownId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no case has this id")
    }
}

impl core::error::Error for UnknownId {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn a_key_displays_and_parses_as_its_id() {
        let key = Key::new(1).unwrap();
        assert_eq!(key.to_string(), "UkLWZ");
        assert_eq!("UkLWZ".parse(), Ok(key));
        assert_eq!("UkLWZ-pin-bun-or-float".parse(), Ok(key));
        assert_eq!("gMvFo".parse::<Key>().map(Key::get), Ok(1_000_000));
        assert_eq!(key.get(), 1);
    }

    #[test]
    fn a_key_is_one_a_store_can_give() {
        assert_eq!(Key::new(0), None);
        assert_eq!(Key::new(Key::MAX).map(Key::get), Some(Key::MAX));
        assert_eq!(Key::MAX, 9_223_372_036_854_775_807);
        assert_eq!(Key::new(Key::MAX + 1), None);
        // The id of key 0, and of a key past the largest, name no case.
        for id in [cases_id::encode(0), cases_id::encode(Key::MAX + 1)] {
            assert_eq!(id.parse::<Key>(), Err(UnknownId), "{id}");
        }
    }

    #[test]
    fn what_is_no_id_names_no_case() {
        for text in ["", "UkLWA", "UkLWZZ", "../x", "-UkLWZ", "UkLW_"] {
            assert_eq!(text.parse::<Key>(), Err(UnknownId), "{text:?}");
        }
        assert_eq!(UnknownId.to_string(), "no case has this id");
    }
}
