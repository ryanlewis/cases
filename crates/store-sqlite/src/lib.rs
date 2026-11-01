//! A cases store in one SQLite file.
//!
//! [`Db`] is the store at a path. It implements the core's
//! [`Store`](cases_core::Store): it keeps each case's rows as the writer made
//! them, and runs the writer, [`Case::open`](cases_core::Case::open) or
//! [`Case::append`](cases_core::Case::append), under the store's lock, with
//! the store's clock and the actor for who writes. Each case has a [`Key`],
//! which displays as the case's public id. [`Writer`] says who writes, and
//! [`Db::actors`] gives the table of actors, by id. A [`Poller`] reads the
//! store over and over, for `cases wait` and `cases serve`. [`Db::listen`]
//! records a `cases wait` that takes messages, [`Db::waits`] gives the waits
//! listening now, and [`Db::listeners`] the workers they listen for, with
//! what each says it is doing.
//!
//! The file holds SQLite's STRICT tables and a schema version, and is kept in
//! write-ahead logging, with every commit synced. Only creating a case makes
//! it.
#![warn(missing_docs)]

mod actors;
mod db;
mod error;
mod key;
mod listen;
mod poll;
mod read;
mod schema;
mod write;

#[cfg(test)]
mod testing;

// The core's types a store and its readers share, as the store's own API
// has always given them.
pub use cases_core::{
    ActorRow, Cases, DOING_MAX, HEARTBEAT, Key, LAPSE, Listening, UnknownId, Wait, Writer,
};
pub use db::Db;
pub use error::Error;
pub use listen::{doing_line, new_wait};
pub use poll::Poller;
