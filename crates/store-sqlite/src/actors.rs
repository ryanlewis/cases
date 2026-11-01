//! Actors: who writes events, by id, with their kind and name in the store's
//! table of actors.

use cases_core::{Actor, ActorRow, Author, Time, Writer};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::db::Db;
use crate::error::Error;

impl Db {
    /// Every actor in the store, in the order the store made them. A reader
    /// looks up an event's actor id here for its kind and name. An id that
    /// is not here is one whose row was deleted.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Missing`] when there is no store, and any other error
    /// of the store.
    pub fn actors(&self) -> Result<Vec<ActorRow>, Error> {
        self.read(|tx, _| {
            let mut statement =
                tx.prepare("SELECT id, kind, name, created FROM actors ORDER BY rowid")?;
            let actors = statement.query_map([], |row| {
                Ok(ActorRow::new(
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            })?;
            Ok(actors.collect::<Result<_, _>>()?)
        })
    }
}

/// The actor for `writer` writing an event of `author`, made at `now` the
/// first time the writer writes as that author. It is made in the write's
/// transaction, so a write that is refused or fails makes no actor. An
/// author that is neither an agent nor a human has no actor: the core refuses
/// its event before it would take one.
pub(crate) fn actor_for(
    tx: &Transaction<'_>,
    author: &str,
    writer: &Writer,
    now: Time,
) -> Result<Option<Actor>, Error> {
    let Ok(author) = author.parse::<Author>() else {
        return Ok(None);
    };
    let kind = author.as_str();
    let found = tx
        .query_row(
            "SELECT id FROM actors WHERE kind = ?1 AND name IS ?2",
            params![kind, writer.name()],
            |row| row.get(0),
        )
        .optional()?;
    let id = if let Some(id) = found {
        id
    } else {
        let id = new_id()?;
        tx.execute(
            "INSERT INTO actors (id, kind, name, created) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind, writer.name(), now.to_string()],
        )?;
        id
    };
    let mut actor = Actor::default();
    actor.id = id;
    Ok(Some(actor))
}

/// A new actor id: 128 random bits from the operating system, in base64url.
pub(crate) fn new_id() -> Result<String, Error> {
    let mut bits = [0; 16];
    getrandom::fill(&mut bits).map_err(|error| Error::Io(std::io::Error::other(error)))?;
    Ok(base64url(&bits))
}

/// `bytes` in the base64url alphabet, without padding.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // The chunk's bits, from the top of the low 24.
        let mut word = [0; 4];
        word[1..=chunk.len()].copy_from_slice(chunk);
        let bits = u32::from_be_bytes(word);
        // Each byte takes a character, and one more holds the rest.
        for i in 0..=chunk.len() {
            let index = bits >> (18 - 6 * i) & 63;
            text.push(char::from(ALPHABET[index as usize]));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_is_the_standard_encoding_without_padding() {
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(base64url(&bytes), "AAECAwQFBgcICQoLDA0ODw");
        assert_eq!(base64url(&[0xff; 16]), "_____________________w");
        assert_eq!(base64url(&[0xfb, 0xef, 0xbe]), "----");
        assert_eq!(base64url(b"Ma"), "TWE");
    }

    #[test]
    fn a_new_id_is_22_base64url_characters_and_new_each_time() {
        let ids: Vec<String> = (0..100).map(|_| new_id().unwrap()).collect();
        for id in &ids {
            assert_eq!(id.len(), 22, "{id}");
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "{id}"
            );
        }
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn an_empty_name_is_no_name() {
        assert_eq!(Writer::new(Some("")).name(), None);
        assert_eq!(Writer::new(None), Writer::new(Some("")));
        assert_eq!(Writer::new(Some("Ryan")).name(), Some("Ryan"));
    }
}
