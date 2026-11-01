//! Case ids: the public name of a case, and the key it stands for.
//!
//! A store numbers its cases with a key, 1, 2, 3 and so on, in the order they
//! are opened. A case's id is the [Sqids](https://sqids.org) encoding of its
//! key, with Sqids's default alphabet, a minimum length of 5 and no
//! blocklist: key 1 is `UkLWZ`, and key 1,000,000 is `gMvFo`. Each key has
//! one id and each id one key, so two cases never share an id, and an id
//! never changes. The id hides the number, but it is not a secret: anyone can
//! decode it.
//!
//! [`encode`] gives a key's id, and [`decode`] gives the key an id names. An
//! id may be followed by a title slug for people to read, as in
//! `UkLWZ-pin-bun-or-float`. [`split`] takes the slug off, so
//! `decode(split(text).0)` finds the case whatever the slug says.
//!
//! Any Sqids implementation gives the same ids with the default alphabet, a
//! minimum length of 5 and an empty blocklist. The choice is case
//! `2026-09-24T23-34-17Z-case-id-encoding-alphabet-min-length-5-own`.
#![no_std]
#![warn(missing_docs)]

extern crate alloc;

use alloc::string::String;

/// The characters of an id: Sqids's default alphabet, in the order Sqids
/// gives it, before the shuffle. It is base64url without `-` and `_`, so no
/// id holds the `-` before a slug, or starts with one.
pub const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// The fewest characters an id has. The ids of keys up to 13,845,840 have
/// exactly this many, and longer ids hold larger keys.
pub const MIN_LENGTH: usize = 5;

/// The number of characters in [`ALPHABET`].
const SIZE: usize = 62;

const _: () = assert!(ALPHABET.len() == SIZE);

/// The id of `key`: at least [`MIN_LENGTH`] characters of [`ALPHABET`].
///
/// ```
/// assert_eq!(cases_id::encode(1), "UkLWZ");
/// assert_eq!(cases_id::encode(1_000_000), "gMvFo");
/// ```
#[must_use]
pub fn encode(key: u64) -> String {
    encode_to(key, MIN_LENGTH)
}

/// The key whose id is exactly `id`, or `None` when `id` is no key's id.
///
/// Only the id [`encode`] gives for a key names it. Text that Sqids would
/// decode to the same key but spells it another way, such as with other
/// padding, names no key, and neither does an id with a slug after it: use
/// [`split`] first to allow one.
///
/// ```
/// assert_eq!(cases_id::decode("UkLWZ"), Some(1));
/// assert_eq!(cases_id::decode("UkLWA"), None);
/// assert_eq!(cases_id::decode("UkLWZ-pin-bun"), None);
/// ```
#[must_use]
pub fn decode(id: &str) -> Option<u64> {
    let (&prefix, rest) = id.as_bytes().split_first()?;
    let mut chars = alphabet();
    let offset = chars.iter().position(|&c| c == prefix)?;
    chars.rotate_left(offset);
    chars.reverse();
    let (&separator, digits) = chars.split_first()?;
    let number = rest.split(|&c| c == separator).next()?;
    if number.is_empty() {
        return None;
    }
    let base = u64::try_from(digits.len()).ok()?;
    let key = number.iter().try_fold(0u64, |key, c| {
        let digit = digits.iter().position(|d| d == c)?;
        key.checked_mul(base)?
            .checked_add(u64::try_from(digit).ok()?)
    })?;
    // Sqids reads past the first number, and takes padding it did not
    // write; only the id the key encodes to names it.
    (encode(key) == id).then_some(key)
}

/// Splits text such as `UkLWZ-pin-bun-or-float` into the id and the slug
/// after it: everything up to the first `-`, and everything after it, if
/// there is a `-`.
///
/// ```
/// assert_eq!(cases_id::split("UkLWZ-pin-bun"), ("UkLWZ", Some("pin-bun")));
/// assert_eq!(cases_id::split("UkLWZ"), ("UkLWZ", None));
/// ```
#[must_use]
pub fn split(text: &str) -> (&str, Option<&str>) {
    match text.split_once('-') {
        Some((id, slug)) => (id, Some(slug)),
        None => (text, None),
    }
}

/// [`ALPHABET`] as Sqids shuffles it before it makes any id.
fn alphabet() -> [u8; SIZE] {
    let mut chars = [0; SIZE];
    chars.copy_from_slice(ALPHABET.as_bytes());
    shuffle(&mut chars);
    chars
}

/// Sqids's shuffle, which gives the same order for the same characters every
/// time.
fn shuffle(chars: &mut [u8]) {
    let len = chars.len();
    let (mut i, mut j) = (0, len.saturating_sub(1));
    while j > 0 {
        let r = (i * j + usize::from(chars[i]) + usize::from(chars[j])) % len;
        chars.swap(i, r);
        i += 1;
        j -= 1;
    }
}

/// Sqids's encoding of the one number `key`, padded to `min_length`
/// characters. Sqids offsets and reverses the alphabet by the number, writes
/// the first character of the result, then the number in the rest of it, and
/// pads the id with a separator and shuffles of the alphabet.
fn encode_to(key: u64, min_length: usize) -> String {
    let mut chars = alphabet();
    let offset = (1 + usize::from(chars[index(key % SIZE as u64)])) % SIZE;
    chars.rotate_left(offset);
    let mut id = String::with_capacity(min_length.max(12));
    id.push(char::from(chars[0]));
    chars.reverse();
    push_number(&mut id, key, &chars[1..]);
    if min_length > id.len() {
        id.push(char::from(chars[0]));
        // Sqids pads from each shuffle in turn, as far as the id needs.
        let short = min_length - id.len();
        let shuffles = core::iter::repeat_with(|| {
            shuffle(&mut chars);
            chars
        });
        id.extend(shuffles.flatten().take(short).map(char::from));
    }
    id
}

/// Writes `n` to `id` in the base of `digits`' length, most significant
/// digit first.
fn push_number(id: &mut String, mut n: u64, digits: &[u8]) {
    let base = digits.len() as u64;
    // A u64 has at most 11 digits in base 61.
    let mut written = [0; 11];
    let mut start = written.len();
    loop {
        start -= 1;
        written[start] = digits[index(n % base)];
        n /= base;
        if n == 0 {
            break;
        }
    }
    id.extend(written[start..].iter().copied().map(char::from));
}

/// A remainder below [`SIZE`], as an index.
#[expect(
    clippy::cast_possible_truncation,
    reason = "every caller passes a remainder of a division by at most 62"
)]
fn index(n: u64) -> usize {
    n as usize
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::vec::Vec;

    use super::*;

    /// The ids of the decision's examples. Ids are a permanent contract:
    /// these never change.
    #[test]
    fn the_ids_of_the_examples_never_change() {
        for (key, id) in [
            (1, "UkLWZ"),
            (2, "gbHJd"),
            (1000, "pndkl"),
            (1_000_000, "gMvFo"),
        ] {
            assert_eq!(encode(key), id, "{key}");
            assert_eq!(decode(id), Some(key), "{id}");
        }
    }

    /// Ids from the `sqids` crate, 0.4.2, the Rust reference
    /// implementation, with the default alphabet, a minimum length of 5 and
    /// an empty blocklist.
    const REFERENCE: &[(u64, &str)] = &[
        (0, "bMZn4"),
        (1, "UkLWZ"),
        (2, "gbHJd"),
        (3, "EfhxL"),
        (4, "VqXmZ"),
        (5, "uw2YK"),
        (6, "OIJLh"),
        (7, "AXs1i"),
        (8, "p6klV"),
        (9, "nJqfP"),
        (10, "98Fqg"),
        (59, "zNcwP"),
        (60, "ccuas"),
        (61, "UkpLW"),
        (62, "bnnZn"),
        (100, "86uR0"),
        (1000, "pndkl"),
        (3720, "bbbZn"),
        (3721, "UkppL"),
        (3722, "gXVXH"),
        (226_980, "ccccu"),
        (226_981, "Ukppp"),
        (1_000_000, "gMvFo"),
        (13_845_840, "bbbbb"),
        (13_845_841, "Ukpppp"),
        (1_000_000_000, "seqWcQz"),
        (4_294_967_296, "V334TK6"),
        (9_223_372_036_854_775_807, "AqkYhRmbHpEX"),
        (18_446_744_073_709_551_614, "HxQoDMba7KlO"),
        (18_446_744_073_709_551_615, "eIkvoXH40Lmd"),
    ];

    /// Ids from the same implementation with other minimum lengths, which pad
    /// the ids differently.
    const REFERENCE_PADDED: &[(usize, u64, &str)] = &[
        (0, 0, "bM"),
        (0, 1_000_000, "gMvFo"),
        (1, 0, "bM"),
        (10, 0, "bMZn4Y5Fq8"),
        (10, 1_000_000, "gMvFoHJdmf"),
        (10, 9_007_199_254_740_991, "ABARpJzdz9"),
        (12, 1_000_000, "gMvFoHJdmfrX"),
        (
            62,
            0,
            "bMZn4Y5Fq8QTCJoLjxPvGfB9Dh6mlz1Sgcu0KpkMyOEiIdrsHRW2VZtweX3aA7",
        ),
        (
            62,
            1_000_000,
            "gMvFoHJdmfrXBVPaH7Dyuvtw6ESeTjU9CAxn1icMlqF8pIG3ZK2W45RQs0kgOb",
        ),
        (
            62,
            9_007_199_254_740_991,
            "ABARpJzdz9s1igzRC6Gs0DnXbv9haf48ojcuO53FTdptlWkMHUqB2YSwA7KJIN",
        ),
        // Past two shuffles of the alphabet.
        (
            130,
            0,
            "bMZn4Y5Fq8QTCJoLjxPvGfB9Dh6mlz1Sgcu0KpkMyOEiIdrsHRW2VZtweX3aA7\
             UNbhFm8ZG04y52lzNU6diPwIVXRTgevKB3tECDo17rJa9cunpfsLSkWOMHAYxQqjb72y",
        ),
        (
            130,
            1_000_000,
            "gMvFoHJdmfrXBVPaH7Dyuvtw6ESeTjU9CAxn1icMlqF8pIG3ZK2W45RQs0kgOb\
             hzoYNLCdED1xT5hs0XOJb7vyqASnj38rwzpGFoKl6uVicaPZBI29gMkYWHNLtfQmUeR4",
        ),
    ];

    #[test]
    fn ids_are_the_reference_implementations() {
        for &(key, id) in REFERENCE {
            assert_eq!(encode(key), id, "{key}");
            assert_eq!(decode(id), Some(key), "{id}");
        }
        for &(min_length, key, id) in REFERENCE_PADDED {
            assert_eq!(encode_to(key, min_length), id, "{key} at {min_length}");
        }
    }

    /// FNV-1a of the ids joined with commas.
    fn fnv(ids: impl Iterator<Item = String>) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (i, id) in ids.enumerate() {
            let comma = if i == 0 { None } else { Some(b',') };
            for byte in comma.into_iter().chain(id.bytes()) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
    }

    /// Numbers spread over the whole range of a u64, the same every run.
    fn spread(n: usize) -> impl Iterator<Item = u64> {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        (0..n).map(move |_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        })
    }

    /// The hashes of many more ids from the reference implementation: those
    /// of keys 0 to 99,999, and of 10,000 keys spread over the range.
    #[test]
    fn many_more_ids_are_the_reference_implementations() {
        assert_eq!(fnv((0..100_000).map(encode)), 0x4dcd_b83c_3c68_3313);
        assert_eq!(fnv(spread(10_000).map(encode)), 0x0f1b_7b9d_811f_0c74);
    }

    /// The published Sqids test vectors that one number and the default
    /// alphabet can take, from sqids/sqids-spec at 49b8ad8: the shuffle, and
    /// the ids of 0 to 9 with no minimum length.
    #[test]
    fn the_published_sqids_vectors_hold() {
        for (from, to) in [
            (
                ALPHABET,
                "fwjBhEY2uczNPDiloxmvISCrytaJO4d71T0W3qnMZbXVHg6eR8sAQ5KkpLUGF9",
            ),
            (
                "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
                "ec38UaynYXvoxSK7RV9uZ1D2HEPw6isrdzAmBNGT5OCJLk0jlFbtqWQ4hIpMgf",
            ),
            (
                "1023456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
                "xI3RUayk1MSolQK7e09zYmFpVXPwHiNrdfBJ6ZAT5uCWbntgcDsEqjv4hLG28O",
            ),
            (
                "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXZY",
                "x038UaykZMSolIK7RzcbYmFpgXEPHiNr1d2VfGAT5uJWQetjvDswqn94hLC6BO",
            ),
            ("0123456789", "4086517392"),
            ("12345", "24135"),
            ("abcdefghijklmnopqrstuvwxyz", "lbfziqvscptmyxrekguohwjand"),
            ("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "ZXBNSIJQEDMCTKOHVWFYUPLRGA"),
        ] {
            let mut chars = from.as_bytes().to_vec();
            shuffle(&mut chars);
            assert_eq!(chars, to.as_bytes(), "{from}");
        }
        let ids = ["bM", "Uk", "gb", "Ef", "Vq", "uw", "OI", "AX", "p6", "nJ"];
        for (key, id) in (0..).zip(ids) {
            assert_eq!(encode_to(key, 0), id, "{key}");
        }
        // Their test of minimum lengths, for its lists of one number.
        for min_length in [0, 1, 5, 10, SIZE] {
            for key in [0, 1_000_000, 9_007_199_254_740_991] {
                let id = encode_to(key, min_length);
                assert!(id.len() >= min_length, "{key} at {min_length}: {id}");
            }
        }
    }

    #[test]
    fn every_key_round_trips() {
        let edges = [0, 1, 13_845_840, 13_845_841, i64::MAX as u64, u64::MAX];
        for key in (0..100_000).chain(spread(100_000)).chain(edges) {
            let id = encode(key);
            assert!(id.len() >= MIN_LENGTH, "{key}: {id}");
            assert!(id.bytes().all(|c| ALPHABET.as_bytes().contains(&c)), "{id}");
            assert_eq!(decode(&id), Some(key), "{key}: {id}");
            assert_eq!(split(&id), (id.as_str(), None), "{id}");
        }
    }

    /// Keys up to 61⁴ − 1 have ids of 5 characters, and up to 61⁵ − 1 of 6.
    #[test]
    fn an_id_grows_only_past_the_largest_key_of_its_length() {
        let ids: Vec<String> = [13_845_840, 13_845_841, 844_596_300, 844_596_301]
            .into_iter()
            .map(encode)
            .collect();
        assert_eq!(ids, ["bbbbb", "Ukpppp", "cccccc", "Ukppppp"]);
    }

    #[test]
    fn only_the_id_a_key_encodes_to_names_it() {
        let names_no_key = [
            // Empty, or characters outside the alphabet.
            "",
            "-",
            "_",
            "*",
            "UkLW_",
            "UkLW-",
            "UkLWé",
            " UkLWZ",
            "UkLWZ ",
            // Paths, which name no key either.
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            ".hidden",
            // Key 1 spelt other ways: other padding, one character more or
            // less, another case, the id with no minimum length.
            "UkLWA",
            "UkLW",
            "UkLWZZ",
            "uKlwz",
            "Uk",
            // Sqids ids of lists of numbers: [1, 2, 3], and [0, 0] padded.
            "86Rf07",
            "86Rf07xd4z",
            "SvIzsqYMyQwI3GWgJAe17URxX8V924Co0DaTZLtFjHriEn5bPhcSkfmvOslpBu",
            // A number past a u64.
            "eIkvoXH40LmdZ",
            "e999999999999999999999",
            // An id and its slug.
            "UkLWZ-pin-bun",
        ];
        for text in names_no_key {
            assert_eq!(decode(text), None, "{text:?}");
        }
        // A prefix whose number part is empty: the separator comes first.
        let separator_first = {
            let mut chars = alphabet();
            let offset = chars.iter().position(|&c| c == b'U').unwrap();
            chars.rotate_left(offset);
            chars.reverse();
            format!("U{}bcd", char::from(chars[0]))
        };
        assert_eq!(decode(&separator_first), None, "{separator_first}");
    }

    #[test]
    fn split_takes_off_the_slug() {
        assert_eq!(split("UkLWZ-pin-bun"), ("UkLWZ", Some("pin-bun")));
        assert_eq!(split("UkLWZ-"), ("UkLWZ", Some("")));
        assert_eq!(split("UkLWZ"), ("UkLWZ", None));
        assert_eq!(split("-pin"), ("", Some("pin")));
        assert_eq!(decode(split("gMvFo-an-old-title").0), Some(1_000_000));
    }
}
