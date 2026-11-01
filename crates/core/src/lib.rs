//! The cases core: event replay, transition rules and answer validation.
//!
//! A case is one question from an agent to a human, or one message from the
//! human to an agent, kept as the events written to it: `open`, `amend`,
//! `answer`, `pickup`, `note`, `close`, `withdraw`, `park` and `resume`. A
//! store keeps each event as a [`Row`].
//! [`Case::fold`] replays a case's rows into the case, and [`Case::open`] and
//! [`Case::append`] check a new event against the case and give the row to
//! store. [`Store`] is the storage surface a store crate implements, with
//! the types a store and its readers share: a case's [`Key`], who writes
//! ([`Writer`]), the table of actors ([`ActorRow`]), the sessions listening
//! ([`Wait`], [`Listening`]), and the store's [`Version`] and the
//! [`Changes`] since one. A [`Poller`] reads any store over and over, and
//! folds again only the cases that changed.
//!
//! The core does no IO and reads no clock: a writer passes `now` in. It is
//! `no_std`, and needs `alloc`.
//!
//! Its contract is the JSON vectors in `spec/vectors/` of
//! [ryanlewis/cases](https://github.com/ryanlewis/cases), with the rules in their
//! README. An error is compared by its [`Category`]; its message is for
//! people, and not part of the contract.
#![no_std]
#![warn(missing_docs)]

extern crate alloc;

mod case;
mod error;
mod freshness;
mod key;
mod listening;
mod names;
mod poll;
mod record;
mod rules;
mod stats;
mod store;
mod time;

pub use case::{Append, Case, Event, Problem, Row};
pub use error::{Category, Error};
pub use freshness::Since;
pub use key::{Key, UnknownId};
pub use listening::{DOING_MAX, HEARTBEAT, LAPSE, Listening, Wait, doing_line, heard_in_time};
pub use names::{Author, EventType, Kind, State, Urgency};
pub use poll::Poller;
pub use record::{
    Actor, Amend, Answer, ApprovalRow, Close, Note, Open, Park, Pickup, Record, Resume, RowVerdict,
    Withdraw,
};
pub use stats::{Stats, stats};
pub use store::{ActorRow, Cases, Changes, Store, StoreError, Version, Writer};
pub use time::Time;
