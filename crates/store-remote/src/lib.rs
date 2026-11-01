//! A cases store on a hosted server, over its HTTP API.
//!
//! [`RemoteStore`] implements the core's [`Store`](cases_core::Store) over
//! the store routes of the agent API, `{server}/v1/stores/{id}/store/…`:
//! the server keeps each case's rows and runs the core's writer under the
//! store's lock, and the client folds the rows with its own copy of the
//! core, so a remote store gives exactly what a local one does. A
//! [`Client`] holds the one HTTP agent, the [`Server`] and the [`Token`],
//! and reads the account with [`Client::whoami`]. What goes wrong outside
//! the core's refusals is an [`Error`].
//!
//! [`Signin`] holds the sign-in routes, which take no token: the loopback
//! sign-in's approval address and its exchange, with the secrets of
//! [`Pkce`], and the device code. Both end with a [`TokenAnswer`], and
//! [`Client::revoke`] revokes a token.
//!
//! A forwarded store is the same store routes over a pipe in place of a
//! socket: [`Client::dial`] runs a child, such as `ssh host cases
//! dial-stdio`, for each connection, and speaks HTTP/1.1 over its stdin
//! and stdout ([`Dial`]). Its one store is [`DIAL_STORE`], and it records
//! listening as a local store does.
//!
//! Without the `hosted` feature, on by default, the crate is the store
//! protocol and the pipe alone: no sign-in, no whoami, no revoke and no
//! TLS.
//!
//! The client is synchronous: a request runs on the calling thread, with no
//! runtime. It never reads the environment, a configuration or a file:
//! finding the server, the token and the store is the caller's job.
//!
//! ```no_run
//! use cases_core::{Poller, Store};
//! use cases_store_remote::{Client, Server, Token};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let server: Server = "https://cases.sh".parse()?;
//! let token: Token = "csu_000000000000000000000000000000000000".parse()?;
//! let store = Client::new(server, token).store("st_4kQ9mZr2pX7wLc0t")?;
//! let mut poller = Poller::new(&store);
//! println!("{} cases", poller.poll()?.len());
//! # Ok(())
//! # }
//! ```
#![warn(missing_docs)]

mod client;
mod error;
mod hold;
mod pipe;
mod shape;
#[cfg(feature = "hosted")]
mod signin;
mod store;

#[cfg(test)]
#[path = "../tests/fake/mod.rs"]
mod fake;
#[cfg(test)]
mod pipe_tests;
#[cfg(all(test, feature = "hosted"))]
mod signin_tests;
#[cfg(test)]
mod tests;

pub use client::{
    ANSWER_LIMIT, CONNECT_TIMEOUT, Client, DEADLINE_FLOOR, EXCHANGE_RETRIES, POLL_GRACE, RATE_WAIT,
    READ_RETRIES, REQUEST_LIMIT, REQUEST_TIMEOUT, SIGNIN_TIMEOUT,
};
#[cfg(feature = "hosted")]
pub use client::{Account, StoreInfo, TokenInfo, Whoami};
pub use error::{Error, SigninError};
pub use hold::{Hold, LEASE_MIN};
pub use pipe::Dial;
pub use shape::{Invalid, Server, Token, store_id};
#[cfg(feature = "hosted")]
pub use signin::{
    DeviceCode, POLL_FLOOR, Pkce, SLOW_DOWN, Signin, StoreRef, TokenAnswer, redirect_uri,
};
pub use store::{DIAL_STORE, HOLD_MAX, RemoteStore};
