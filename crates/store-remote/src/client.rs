//! The client: one HTTP agent, with the server and the token, and how a
//! request goes and its answer is read.

use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use cases_core::{Category, Error as CoreError};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ureq::Agent;
use ureq::http::Response;

use crate::error::{Error, SigninError};
use crate::hold::{self, Hold, Link};
use crate::pipe::{Dial, NoAddress, Pipe, PipeConnector};
#[cfg(feature = "hosted")]
use crate::shape::account_id;
use crate::shape::{Invalid, Server, Token, store_id};
use crate::store::RemoteStore;

/// The most a request's body may hold: the server refuses more.
pub const REQUEST_LIMIT: usize = 1024 * 1024;

/// The most an answer's body may hold. `list` sends every live case in one
/// answer, so it is well above the HTTP client's own 10 MB.
pub const ANSWER_LIMIT: u64 = 256 * 1024 * 1024;

/// How long a connection may take, TLS included.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long any request but a held poll may take, start to end.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How much longer than its hold a held poll may take: the server's grace
/// and the network.
pub const POLL_GRACE: Duration = Duration::from_secs(15);

/// The waits before a read that failed for a transient reason is sent
/// again: two more tries.
pub const READ_RETRIES: [Duration; 2] = [Duration::from_millis(500), Duration::from_secs(2)];

/// The longest `Retry-After` the client waits for, after a 429, before it
/// sends the request once more. A longer one fails at once.
pub const RATE_WAIT: Duration = Duration::from_secs(10);

/// The address a client over a pipe sends its requests to. Nothing looks
/// it up: the pipe is the connection.
const DIAL_SERVER: &str = "http://localhost";

/// The wait after a 429 that gives no `Retry-After`.
const RATE_DEFAULT: Duration = Duration::from_secs(1);

/// The least time a request is given when a deadline is near, so that the
/// last read before it can still be answered.
pub const DEADLINE_FLOOR: Duration = Duration::from_secs(1);

/// How long each sign-in request may take, start to end. The server
/// answers the exchange only once every store the token opens has it, and
/// tries for up to about 10 s first, so this leaves room.
pub const SIGNIN_TIMEOUT: Duration = REQUEST_TIMEOUT;

/// The waits before an exchange that got 503 `unavailable` is sent again:
/// a store could not take the token, and the server kept the code.
pub const EXCHANGE_RETRIES: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// The limits a client works within. Tests shorten them.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(feature = "hosted"),
    allow(dead_code, reason = "only the sign-in routes read these")
)]
pub(crate) struct Limits {
    pub(crate) connect: Duration,
    pub(crate) request: Duration,
    pub(crate) grace: Duration,
    pub(crate) answer: u64,
    pub(crate) retries: [Duration; 2],
    pub(crate) rate_wait: Duration,
    pub(crate) rate_default: Duration,
    pub(crate) signin: Duration,
    pub(crate) exchange_retries: [Duration; 3],
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            connect: CONNECT_TIMEOUT,
            request: REQUEST_TIMEOUT,
            grace: POLL_GRACE,
            answer: ANSWER_LIMIT,
            retries: READ_RETRIES,
            rate_wait: RATE_WAIT,
            rate_default: RATE_DEFAULT,
            signin: SIGNIN_TIMEOUT,
            exchange_retries: EXCHANGE_RETRIES,
        }
    }
}

/// A client of one server, with one token: one HTTP agent, shared by the
/// stores made from it. Cloning it shares the agent.
///
/// Every request sends `Authorization: Bearer <token>` and `User-Agent:
/// cases/<version>`. No redirect is followed. The proxy settings of the
/// environment (`HTTPS_PROXY`, `NO_PROXY`) are kept, as the HTTP client
/// reads them. The client reads no file, configuration or environment of
/// its own: the caller finds the server and the token.
#[derive(Clone)]
pub struct Client {
    agent: Agent,
    server: Server,
    /// `None` only for a [`Signin`](crate::Signin)'s requests, which send
    /// no token.
    token: Option<Token>,
    user_agent: String,
    limits: Limits,
    /// The caller's deadline, which no retry, wait or request goes past.
    deadline: Option<Instant>,
    /// The forwarded store's dial, for a client over a pipe: its errors
    /// name the dial.
    dial: Option<Dial>,
    /// Whether it reaches a forwarded store, over a pipe or over HTTP to a
    /// `cases serve`: its stores record listening (ST-16), and take a
    /// pickup wait's locks.
    forwarded: bool,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("server", &self.server)
            .field("token", &self.token)
            .field("deadline", &self.deadline)
            .field("dial", &self.dial)
            .field("forwarded", &self.forwarded)
            .finish_non_exhaustive()
    }
}

/// How a request goes again after it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "hosted"),
    allow(dead_code, reason = "only the sign-in routes are never sent again")
)]
pub(crate) enum Retry {
    /// A read: sent again after a transient failure, up to twice.
    Read,
    /// A write: sent again only after a 429, once.
    Write,
    /// Never sent again: the caller decides.
    Never,
}

/// A request, before it is sent.
#[derive(Debug)]
pub(crate) struct Call<'a> {
    pub(crate) method: &'static str,
    /// The path after the server, query included.
    pub(crate) path: &'a str,
    pub(crate) body: Option<Vec<u8>>,
    /// How long it may take; [`Limits::request`] when `None`.
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry: Retry,
}

/// Why a request did not give an answer: a refusal of the core's, no case
/// with the key, or an error of the client's own.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Refused(CoreError),
    NoCase,
    Error(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Failure {
        Failure::Error(error)
    }
}

/// The account a token belongs to, as `GET /v1/cli/whoami` gives it.
#[cfg(feature = "hosted")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Whoami {
    /// The account.
    pub account: Account,
    /// The token that was sent.
    pub token: TokenInfo,
    /// The stores the token opens, in the order the account made them.
    pub stores: Vec<StoreInfo>,
}

/// An account, by its id and its handle.
#[cfg(feature = "hosted")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Account {
    /// `ac_…`.
    pub id: String,
    /// The account's handle, such as `ryan`.
    pub handle: String,
}

/// A token, as the server describes it: never the token itself.
#[cfg(feature = "hosted")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TokenInfo {
    /// The name it was given at sign-in.
    pub name: String,
    /// The ids of the stores it opens, or `None` for every store of the
    /// account.
    pub stores: Option<Vec<String>>,
    /// When it expires, as an RFC 3339 time, if it does.
    pub expires: Option<String>,
}

/// A store of the account.
#[cfg(feature = "hosted")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct StoreInfo {
    /// `st_…`.
    pub id: String,
    /// The store's name in the account.
    pub name: String,
    /// Whether it is the account's default store.
    pub default: bool,
}

/// The body of a refusal: `{"error": {"category", "message"}}`.
#[derive(Deserialize)]
struct Refusal {
    error: RefusalBody,
}

#[derive(Deserialize)]
struct RefusalBody {
    category: String,
    #[serde(default)]
    message: String,
}

impl Client {
    /// A client of `server`, which sends `token`. Nothing is sent until a
    /// request is made.
    #[must_use]
    pub fn new(server: Server, token: Token) -> Client {
        Client::with_limits(server, token, Limits::default())
    }

    pub(crate) fn with_limits(server: Server, token: Token, limits: Limits) -> Client {
        Client::agent(server, Some(token), limits)
    }

    /// A client that sends no token, for the sign-in routes.
    #[cfg(feature = "hosted")]
    pub(crate) fn unauthenticated(server: Server, limits: Limits) -> Client {
        Client::agent(server, None, limits)
    }

    fn agent(server: Server, token: Option<Token>, limits: Limits) -> Client {
        Client::agent_by(server, token, limits, Agent::config_builder())
    }

    /// A client whose agent `config` starts, with the settings every
    /// client has.
    fn agent_by(
        server: Server,
        token: Option<Token>,
        limits: Limits,
        config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
    ) -> Client {
        let agent = config
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(limits.connect))
            .user_agent("")
            .build()
            .new_agent();
        Client {
            agent,
            server,
            token,
            user_agent: concat!("cases/", env!("CARGO_PKG_VERSION")).to_owned(),
            limits,
            deadline: None,
            dial: None,
            forwarded: false,
        }
    }

    /// A client of the forwarded store that a `cases serve` at `server`
    /// answers the store routes of, over HTTP, such as
    /// `http://devbox.example.ts.net:7338` behind `tailscale serve
    /// --http=7338` (see [`Server::forward`]). It sends `token` when there
    /// is one; without one, the far side lets in only what its proxy says
    /// comes from one of its Tailscale logins. Its one store is
    /// [`DIAL_STORE`](crate::DIAL_STORE), as over a pipe, and a pickup
    /// wait's locks are held on a lease the hold renews by its id, since a
    /// proxy between keeps no connection for it.
    #[must_use]
    pub fn forward(server: Server, token: Option<Token>) -> Client {
        Client::forward_with_limits(server, token, Limits::default())
    }

    pub(crate) fn forward_with_limits(
        server: Server,
        token: Option<Token>,
        limits: Limits,
    ) -> Client {
        // Straight to the host, never through a proxy of the environment
        // (`HTTP_PROXY` and the like): over plain http the token would go
        // to it in the clear, and a host on a tailnet is reached over the
        // tailnet.
        let config = Agent::config_builder().proxy(None);
        let mut client = Client::agent_by(server, token, limits, config);
        client.forwarded = true;
        client
    }

    /// A client of the forwarded store that `dial` reaches: each connection
    /// is a child that `dial` starts, which speaks HTTP/1.1 over its stdin
    /// and stdout, such as `ssh host cases dial-stdio`. It sends no token,
    /// since whoever can start the child can open the store. Its one store
    /// is [`DIAL_STORE`](crate::DIAL_STORE).
    #[must_use]
    pub fn dial(dial: Dial) -> Client {
        Client::dial_with_limits(dial, Limits::default())
    }

    pub(crate) fn dial_with_limits(dial: Dial, limits: Limits) -> Client {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(limits.connect))
            .user_agent("")
            .build();
        let agent = Agent::with_parts(config, PipeConnector::new(dial.clone()), NoAddress);
        Client {
            agent,
            server: DIAL_SERVER.parse().unwrap_or_else(|_| unreachable!()),
            token: None,
            user_agent: concat!("cases/", env!("CARGO_PKG_VERSION")).to_owned(),
            limits,
            deadline: None,
            dial: Some(dial),
            forwarded: true,
        }
    }

    /// The forwarded store's dial, for a client over a pipe.
    #[must_use]
    pub fn dialled(&self) -> Option<&Dial> {
        self.dial.as_ref()
    }

    /// Whether it reaches a forwarded store, over a pipe ([`Client::dial`])
    /// or over HTTP ([`Client::forward`]).
    #[must_use]
    pub fn is_forwarded(&self) -> bool {
        self.forwarded
    }

    /// The client, sending `User-Agent: cases/<version>`, such as the
    /// version of the CLI that holds it. It is this crate's version
    /// otherwise.
    #[must_use]
    pub fn with_version(mut self, version: &str) -> Client {
        self.user_agent = format!("cases/{version}");
        self
    }

    /// The client, kept within `deadline`, such as a command's timeout:
    /// a read is not sent again, and a 429 is not waited out, when the wait
    /// would end past it, and a request is given no longer than the time
    /// left, or [`DEADLINE_FLOOR`] when less is left. The request fails
    /// with its error instead. `None` takes the deadline away.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Option<Instant>) -> Client {
        self.deadline = deadline;
        self
    }

    /// Whether a wait of `wait` from now would end past the deadline.
    fn past_deadline(&self, wait: Duration) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() + wait >= deadline)
    }

    /// How long a request of `timeout` may take, within the deadline.
    fn within_deadline(&self, timeout: Duration) -> Duration {
        match self.deadline {
            Some(deadline) => timeout.min(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(DEADLINE_FLOOR),
            ),
            None => timeout,
        }
    }

    /// The server.
    #[must_use]
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// How much longer than its hold a held poll may take.
    pub(crate) fn grace(&self) -> Duration {
        self.limits.grace
    }

    /// The limits the client works within.
    #[cfg(feature = "hosted")]
    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The account the token belongs to, with the stores it opens: `GET
    /// /v1/cli/whoami`.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Auth`] for a token the server refuses, and the
    /// client's other errors when the request fails.
    #[cfg(feature = "hosted")]
    pub fn whoami(&self) -> Result<Whoami, Error> {
        let call = Call {
            method: "GET",
            path: "/v1/cli/whoami",
            body: None,
            timeout: None,
            retry: Retry::Read,
        };
        self.json(&call).map_err(|failure| match failure {
            Failure::Error(error) => error,
            other => protocol_of(&other),
        })
    }

    /// Revokes the token this client sends: `DELETE
    /// /v1/accounts/{account}/cli/token`, where `account` is the id of the
    /// token's account, `ac_…`. It is safe to send twice, so it is retried
    /// as a read is.
    ///
    /// For logging out, [`Error::Auth`] (the token is already revoked or
    /// expired, or is not the account's) and [`Error::NoToken`] (the server
    /// does not know the account) mean the token is no use either way.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Invalid`] when `account` is not an account's id, with
    /// nothing sent; [`Error::Auth`] for a 401; [`Error::NoToken`] for a
    /// 404; and the client's other errors when the request fails.
    #[cfg(feature = "hosted")]
    pub fn revoke(&self, account: &str) -> Result<(), Error> {
        if !account_id(account) {
            return Err(Error::Invalid(CoreError::new(
                Category::Invalid,
                format!("{account:?} is not an account's id: it is ac_ and letters and digits"),
            )));
        }
        let path = format!("/v1/accounts/{account}/cli/token");
        let call = Call {
            method: "DELETE",
            path: &path,
            body: None,
            timeout: Some(self.limits.signin),
            retry: Retry::Read,
        };
        match self.send(&call) {
            Ok(_) => Ok(()),
            Err(
                Failure::NoCase
                | Failure::Error(
                    Error::NoStore | Error::NotListening | Error::Protocol { status: 404, .. },
                ),
            ) => Err(Error::NoToken),
            Err(other) => Err(protocol_of(&other)),
        }
    }

    /// The store `id` on this server. Nothing is sent: a store that is not
    /// there is found at its first request.
    ///
    /// # Errors
    ///
    /// Gives [`Invalid`] when `id` is not a store's id, `st_` and 16
    /// letters and digits.
    pub fn store(&self, id: &str) -> Result<RemoteStore, Invalid> {
        if !store_id(id) {
            return Err(Invalid::new(format!(
                "{id:?} is not a store's id: it is st_ and 16 letters and digits"
            )));
        }
        Ok(RemoteStore::new(self.clone(), id.to_owned()))
    }

    /// Sends `call` and reads its answer as `T`.
    pub(crate) fn json<T: DeserializeOwned>(&self, call: &Call<'_>) -> Result<T, Failure> {
        let (status, body) = self.send(call)?;
        serde_json::from_slice(&body).map_err(|error| {
            Failure::Error(Error::Protocol {
                status,
                message: error.to_string(),
            })
        })
    }

    /// Sends `call`, again where its retry allows, and gives the status and
    /// body of an answer of 200, or why there was none.
    pub(crate) fn send(&self, call: &Call<'_>) -> Result<(u16, Vec<u8>), Failure> {
        if let Some(body) = &call.body
            && body.len() > REQUEST_LIMIT
        {
            return Err(Error::TooLarge { size: body.len() }.into());
        }
        let mut tries = 0;
        let mut rated = false;
        loop {
            let failure = match self.once(call) {
                Ok(answer) => return Ok(answer),
                Err(failure) => failure,
            };
            let wait = match (&failure, call.retry) {
                (_, Retry::Never) => return Err(failure),
                (Failure::Error(Error::Rate { retry_after }), _) if !rated => {
                    let wait = retry_after.unwrap_or(self.limits.rate_default);
                    if wait > self.limits.rate_wait {
                        return Err(failure);
                    }
                    rated = true;
                    wait
                }
                (Failure::Error(error), Retry::Read) if error.is_transient() => {
                    match self.limits.retries.get(tries) {
                        Some(wait) => {
                            tries += 1;
                            *wait
                        }
                        None => return Err(failure),
                    }
                }
                _ => return Err(failure),
            };
            if self.past_deadline(wait) {
                return Err(failure);
            }
            thread::sleep(wait);
        }
    }

    /// Sends `call` once.
    fn once(&self, call: &Call<'_>) -> Result<(u16, Vec<u8>), Failure> {
        let url = format!("{}{}", self.server, call.path);
        let timeout = Some(self.within_deadline(call.timeout.unwrap_or(self.limits.request)));
        let bearer = self
            .token
            .as_ref()
            .map(|token| format!("Bearer {}", token.expose()));
        let sent = if let Some(body) = &call.body {
            let mut request = match call.method {
                "PUT" => self.agent.put(&url),
                _ => self.agent.post(&url),
            };
            if let Some(bearer) = &bearer {
                request = request.header("Authorization", bearer);
            }
            request
                .header("User-Agent", &self.user_agent)
                .header("Content-Type", "application/json")
                .config()
                .timeout_global(timeout)
                .build()
                .send(&body[..])
        } else {
            let mut request = match call.method {
                "DELETE" => self.agent.delete(&url),
                _ => self.agent.get(&url),
            };
            if let Some(bearer) = &bearer {
                request = request.header("Authorization", bearer);
            }
            request
                .header("User-Agent", &self.user_agent)
                .config()
                .timeout_global(timeout)
                .build()
                .call()
        };
        let mut response = sent.map_err(|error| self.transport(&error))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(self.limits.answer)
            .read_to_vec()
            .map_err(|error| self.transport(&error))?;
        if status == 200 {
            return Ok((status, body));
        }
        Err(classify(status, retry_after(&response), &body))
    }

    /// Sends a `POST` of `body` to `path` on a connection of its own, over a
    /// pipe, and keeps that connection open while the [`Hold`] is: what the
    /// far side holds for it, such as a pickup wait's locks, it holds until
    /// the connection ends. It is never sent again. An answer that gives a
    /// lease, `{lease_ms}`, has the hold renew it by a `POST` to
    /// `<path>/renew` over the same connection, and let go by a `POST` to
    /// `<path>/release`. One shorter than [`LEASE_MIN`](crate::LEASE_MIN)
    /// is refused.
    ///
    /// Over HTTP, to a `cases serve`, there is no connection of its own to
    /// keep: a proxy between, such as `tailscale serve`, may send each
    /// request on another. The answer must give a lease and the hold's id,
    /// `{lease_ms, hold}`, and the hold renews it by a `POST` to
    /// `<path>/<hold>/renew` and lets go by one to `<path>/<hold>/release`,
    /// each a request of its own.
    pub(crate) fn hold(&self, path: &str, body: &[u8]) -> Result<Hold, Error> {
        let Some(dial) = &self.dial else {
            if self.forwarded {
                return self.hold_over_http(path, body);
            }
            return Err(Error::Request {
                message: "only a forwarded store keeps a connection held".to_owned(),
            });
        };
        let no_answer = |error: &ureq::Error| match self.transport(error) {
            Failure::Error(error) => error,
            failure => protocol_of(&failure),
        };
        let mut pipe =
            Pipe::start(&Arc::new(dial.clone()), 1024, 1024).map_err(|e| no_answer(&e))?;
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nUser-Agent: {}\r\nContent-Type: \
             application/json\r\nContent-Length: {}\r\n\r\n",
            self.user_agent,
            body.len()
        )
        .into_bytes();
        request.extend_from_slice(body);
        let timeout = self.within_deadline(self.limits.request);
        let sent = hold::now();
        let (status, answer) = pipe
            .exchange(&request, timeout, self.limits.answer)
            .map_err(|e| no_answer(&e))?;
        if status == 200 {
            let lease =
                hold::lease_of(&answer).map_err(|message| Error::Protocol { status, message })?;
            let Some(lease) = lease else {
                return Ok(Hold::open(pipe));
            };
            let post = |route: &str, close: &str| {
                format!(
                    "POST {path}/{route} HTTP/1.1\r\nHost: localhost\r\nUser-Agent: {}\r\n\
                     {close}Content-Length: 0\r\n\r\n",
                    self.user_agent
                )
                .into_bytes()
            };
            let link = Link::Pipe {
                pipe,
                renew: post("renew", ""),
                release: post("release", "Connection: close\r\n"),
            };
            return Ok(Hold::leased(link, lease, sent));
        }
        Err(protocol_of(&classify(status, None, &answer)))
    }

    /// [`Client::hold`] over HTTP: the locks on a lease, renewed and let go
    /// by the hold's id.
    fn hold_over_http(&self, path: &str, body: &[u8]) -> Result<Hold, Error> {
        let call = Call {
            method: "POST",
            path,
            body: Some(body.to_vec()),
            timeout: None,
            retry: Retry::Never,
        };
        let sent = hold::now();
        let (status, answer) = self.send(&call).map_err(|failure| protocol_of(&failure))?;
        let bad = |message: &str| Error::Protocol {
            status,
            message: message.to_owned(),
        };
        let lease = hold::lease_of(&answer)
            .map_err(|message| bad(&message))?
            .ok_or_else(|| bad("the far side gave the locks on no lease"))?;
        let id = hold::id_of(&answer).ok_or_else(|| bad("the far side gave the locks no id"))?;
        let link = Link::Http {
            client: self.clone(),
            renew: format!("{path}/{id}/renew"),
            release: format!("{path}/{id}/release"),
        };
        Ok(Hold::leased(link, lease, sent))
    }

    /// Sends a `POST` with no body to `path` once, within `timeout`, and
    /// gives the status and body of its answer, of at most `limit` bytes,
    /// whatever the status: a hold's renewal or release over HTTP.
    pub(crate) fn post_empty(
        &self,
        path: &str,
        timeout: Duration,
        limit: u64,
    ) -> Result<(u16, Vec<u8>), ureq::Error> {
        let url = format!("{}{path}", self.server);
        let mut request = self.agent.post(&url);
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {}", token.expose()));
        }
        let mut response = request
            .header("User-Agent", &self.user_agent)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send_empty()?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()?;
        Ok((status, body))
    }

    /// The error for a request that got no answer. Over a pipe, it names
    /// the dial, and an error of the pipe's own says why without the HTTP
    /// client's prefix.
    fn transport(&self, error: &ureq::Error) -> Failure {
        Failure::Error(match (error, &self.dial) {
            (ureq::Error::BodyExceedsLimit(_), _) => Error::AnswerTooLarge,
            (ureq::Error::Io(io), Some(dial))
                if matches!(
                    io.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::Unsupported
                ) =>
            {
                Error::Dial {
                    dial: dial.to_string(),
                    reason: io.to_string(),
                }
            }
            (ureq::Error::Io(io), Some(dial)) => Error::Transport {
                server: dial.to_string(),
                reason: io.to_string(),
            },
            (error, dial) => Error::Transport {
                server: dial
                    .as_ref()
                    .map_or_else(|| self.server.to_string(), ToString::to_string),
                reason: error.to_string(),
            },
        })
    }
}

/// The `Retry-After` of an answer, when it is a number of seconds.
fn retry_after<B>(response: &Response<B>) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    value.trim().parse().ok().map(Duration::from_secs)
}

/// What an answer of `status`, not 200, with `body`, means: the error
/// table of the design. The status is read first, and then the body's
/// category, never its message.
pub(crate) fn classify(status: u16, retry_after: Option<Duration>, body: &[u8]) -> Failure {
    let refusal = serde_json::from_slice::<Refusal>(body)
        .ok()
        .map(|r| r.error);
    let message = refusal
        .as_ref()
        .map(|r| r.message.clone())
        .unwrap_or_default();
    let category = refusal.as_ref().map(|r| r.category.as_str());
    let protocol = |message: &str| {
        Failure::Error(Error::Protocol {
            status,
            message: message.to_owned(),
        })
    };
    let refused = |category| Failure::Refused(CoreError::new(category, message.clone()));
    match (status, category) {
        (300..=399, _) => Failure::Error(Error::Redirect { status }),
        (400, Some("malformed")) => refused(Category::Malformed),
        (400, Some(name)) => Failure::Error(match SigninError::from_name(name) {
            Some(kind) => Error::Signin { kind, message },
            None => Error::Request { message },
        }),
        (401, _) => Failure::Error(Error::Auth { message }),
        (404, Some("nocase")) => Failure::NoCase,
        (404, Some("missing")) => Failure::Error(Error::Missing),
        (404, Some("not-listening")) => Failure::Error(Error::NotListening),
        (404, Some("request")) => Failure::Error(Error::NoStore),
        (409, Some("stale")) => refused(Category::Stale),
        (409, Some("held")) => Failure::Error(Error::Held { message }),
        (409, Some("lapsed")) => Failure::Error(Error::Lapsed { reason: message }),
        (410, _) => Failure::Error(Error::Gone),
        (422, Some(name @ ("unknown" | "author" | "invalid" | "transition" | "no-open"))) => {
            match Category::from_name(name) {
                Some(category) => refused(category),
                None => protocol("an unknown category"),
            }
        }
        (429, _) => Failure::Error(Error::Rate { retry_after }),
        (500, Some("newer")) => Failure::Error(Error::Newer { message }),
        (500..=599, _) => Failure::Error(Error::Server { status, message }),
        (_, None) => protocol("the body is not a refusal"),
        (_, Some(category)) => protocol(&format!("an unexpected category {category:?}")),
    }
}

/// The error for a refusal or a missing case where the request can give
/// neither, such as a read.
pub(crate) fn protocol_of(failure: &Failure) -> Error {
    match failure {
        Failure::Error(error) => error.clone(),
        Failure::Refused(error) => Error::Protocol {
            status: 0,
            message: format!("an unexpected refusal, {}: {error}", error.category()),
        },
        Failure::NoCase => Error::Protocol {
            status: 404,
            message: "an unexpected nocase".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(category: &str) -> Vec<u8> {
        format!(r#"{{"error": {{"category": "{category}", "message": "why"}}}}"#).into_bytes()
    }

    fn error(status: u16, body: &[u8]) -> Error {
        match classify(status, None, body) {
            Failure::Error(error) => error,
            other => panic!("{status}: {other:?}"),
        }
    }

    fn refused(status: u16, category: &str) -> Category {
        match classify(status, None, &refusal(category)) {
            Failure::Refused(error) => {
                assert_eq!(error.to_string(), "why");
                error.category()
            }
            other => panic!("{status} {category}: {other:?}"),
        }
    }

    #[test]
    fn each_row_of_the_error_table() {
        assert_eq!(refused(400, "malformed"), Category::Malformed);
        assert_eq!(
            error(400, &refusal("request")),
            Error::Request {
                message: "why".into()
            }
        );
        assert_eq!(
            error(400, &refusal("other")),
            Error::Request {
                message: "why".into()
            }
        );
        for category in ["auth", "anything"] {
            assert_eq!(
                error(401, &refusal(category)),
                Error::Auth {
                    message: "why".into()
                }
            );
        }
        assert_eq!(
            error(401, b""),
            Error::Auth {
                message: String::new()
            }
        );
        assert_eq!(classify(404, None, &refusal("nocase")), Failure::NoCase);
        assert_eq!(error(404, &refusal("missing")), Error::Missing);
        assert_eq!(
            error(500, &refusal("newer")),
            Error::Newer {
                message: "why".into()
            }
        );
        assert_eq!(error(404, &refusal("not-listening")), Error::NotListening);
        assert_eq!(error(404, &refusal("request")), Error::NoStore);
        assert_eq!(refused(409, "stale"), Category::Stale);
        assert_eq!(error(410, &refusal("gone")), Error::Gone);
        assert_eq!(error(410, b"gone"), Error::Gone);
        for (name, category) in [
            ("unknown", Category::Unknown),
            ("author", Category::Author),
            ("invalid", Category::Invalid),
            ("transition", Category::Transition),
            ("no-open", Category::NoOpen),
        ] {
            assert_eq!(refused(422, name), category);
        }
        let after = Some(Duration::from_secs(3));
        assert_eq!(
            classify(429, after, &refusal("rate")),
            Failure::Error(Error::Rate { retry_after: after })
        );
        assert_eq!(
            classify(429, None, b"slow down"),
            Failure::Error(Error::Rate { retry_after: None })
        );
        // 429 is read by status, whatever the category (N26).
        assert_eq!(
            classify(429, None, &refusal("slow_down")),
            Failure::Error(Error::Rate { retry_after: None })
        );
        for (name, kind) in [
            ("invalid_grant", SigninError::InvalidGrant),
            ("authorization_pending", SigninError::AuthorizationPending),
            ("slow_down", SigninError::SlowDown),
            ("access_denied", SigninError::AccessDenied),
            ("expired_token", SigninError::ExpiredToken),
        ] {
            assert_eq!(
                error(400, &refusal(name)),
                Error::Signin {
                    kind,
                    message: "why".into()
                }
            );
            // Only a 400 is a sign-in refusal.
            assert!(matches!(error(401, &refusal(name)), Error::Auth { .. }));
            assert!(matches!(error(403, &refusal(name)), Error::Protocol { .. }));
        }
        assert_eq!(
            error(503, &refusal("unavailable")),
            Error::Server {
                status: 503,
                message: "why".into()
            }
        );
        for status in [301, 302, 307, 308, 399] {
            assert_eq!(error(status, b""), Error::Redirect { status });
        }
        for status in [500, 502, 503, 504, 599] {
            assert_eq!(
                error(status, &refusal("store")),
                Error::Server {
                    status,
                    message: "why".into()
                }
            );
        }
    }

    #[test]
    fn what_is_not_the_api_s_is_a_protocol_error() {
        for (status, body) in [
            (201, refusal("request")),
            (204, Vec::new()),
            (400, b"not json".to_vec()),
            (404, b"".to_vec()),
            (404, refusal("other")),
            (409, refusal("transition")),
            (422, refusal("stale")),
            (422, refusal("malformed")),
            (422, refusal("rate")),
            (418, refusal("request")),
            (403, b"{}".to_vec()),
            (100, Vec::new()),
            (600, Vec::new()),
        ] {
            assert!(
                matches!(error(status, &body), Error::Protocol { status: s, .. } if s == status),
                "{status} {}",
                String::from_utf8_lossy(&body)
            );
        }
        // Text that does not name a category is not a refusal.
        assert!(matches!(
            error(422, br#"{"error": {"message": "why"}}"#),
            Error::Protocol { .. }
        ));
    }

    #[test]
    fn a_refusal_where_none_is_expected_is_a_protocol_error() {
        let refused = Failure::Refused(CoreError::new(Category::Stale, "why"));
        assert!(matches!(
            protocol_of(&refused),
            Error::Protocol { message, .. } if message == "an unexpected refusal, stale: why"
        ));
        assert!(matches!(
            protocol_of(&Failure::NoCase),
            Error::Protocol { status: 404, .. }
        ));
        assert_eq!(protocol_of(&Failure::Error(Error::Gone)), Error::Gone);
    }

    #[test]
    fn the_limits_are_the_design_s() {
        assert_eq!(REQUEST_LIMIT, 1_048_576);
        assert_eq!(ANSWER_LIMIT, 268_435_456);
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(30));
        assert_eq!(POLL_GRACE, Duration::from_secs(15));
        assert_eq!(
            READ_RETRIES,
            [Duration::from_millis(500), Duration::from_secs(2)]
        );
        assert_eq!(RATE_WAIT, Duration::from_secs(10));
        let limits = Limits::default();
        assert_eq!(limits.connect, CONNECT_TIMEOUT);
        assert_eq!(limits.request, REQUEST_TIMEOUT);
        assert_eq!(limits.grace, POLL_GRACE);
        assert_eq!(limits.answer, ANSWER_LIMIT);
        assert_eq!(limits.retries, READ_RETRIES);
        assert_eq!(limits.rate_wait, RATE_WAIT);
        assert_eq!(limits.rate_default, Duration::from_secs(1));
        assert_eq!(SIGNIN_TIMEOUT, Duration::from_secs(30));
        assert_eq!(limits.signin, SIGNIN_TIMEOUT);
        assert_eq!(limits.exchange_retries, EXCHANGE_RETRIES);
    }

    #[test]
    fn the_agent_follows_no_redirect_and_keeps_the_environment_s_proxy() {
        let server = "https://cases.sh".parse().unwrap();
        let token = format!("csu_{}", "a".repeat(36)).parse().unwrap();
        let client = Client::new(server, token);
        let config = client.agent.config();
        assert_eq!(config.max_redirects(), 0);
        assert!(!config.http_status_as_error());
        assert_eq!(config.proxy(), ureq::Proxy::try_from_env().as_ref());
        assert_eq!(config.timeouts().connect, Some(CONNECT_TIMEOUT));
        let debug = format!("{client:?}");
        assert!(!debug.contains(&"a".repeat(36)), "{debug}");
        assert!(
            debug.contains("https://cases.sh") && debug.contains("csu_…"),
            "{debug}"
        );
        assert_eq!(
            client.user_agent,
            concat!("cases/", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(client.with_version("1.2.3").user_agent, "cases/1.2.3");
    }
}
