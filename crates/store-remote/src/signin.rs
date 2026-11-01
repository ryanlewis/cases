//! Signing in to an account: PKCE (RFC 7636), the loopback sign-in's
//! approval address and exchange, the device code (RFC 8628), and the
//! token answer both give.
//!
//! The sign-in routes take no token. What needs the system beyond random
//! bytes, the loopback listener, the browser and keeping the token, is the
//! caller's.

use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::client::{Account, Call, Client, Failure, Limits, Retry, Whoami, protocol_of};
use crate::error::{Error, SigninError};
use crate::shape::{Server, Token};

/// How much a device's poll interval grows by on each `slow_down` (RFC
/// 8628, 3.5).
pub const SLOW_DOWN: Duration = Duration::from_secs(5);

/// The least interval between a device's polls, whatever the server says.
pub const POLL_FLOOR: Duration = Duration::from_secs(1);

/// The secrets of one loopback sign-in: the code verifier, its S256
/// challenge, and the state the browser must bring back.
///
/// The verifier and the state are 32 random bytes each, as base64url (43
/// characters). Neither is shown by [`Debug`](fmt::Debug): the verifier
/// leaves the process only in the exchange's body.
pub struct Pkce {
    verifier: String,
    state: String,
    challenge: String,
}

impl Pkce {
    /// New secrets, from the operating system's random bytes.
    ///
    /// # Errors
    ///
    /// Gives the error of the system's random source, when it has none.
    pub fn new() -> std::io::Result<Pkce> {
        let mut bytes = [0u8; 64];
        getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
        let (verifier, state) = bytes.split_at(32);
        Ok(Pkce::from_bytes(verifier, state))
    }

    /// The secrets made from these bytes.
    pub(crate) fn from_bytes(verifier: &[u8], state: &[u8]) -> Pkce {
        let verifier = base64url(verifier);
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            state: base64url(state),
            challenge,
        }
    }

    /// The challenge: the verifier's SHA-256, as base64url (S256). It is no
    /// secret.
    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    /// Whether `given` is this sign-in's state, compared in constant time
    /// for its length.
    #[must_use]
    pub fn state_is(&self, given: &str) -> bool {
        let (want, given) = (self.state.as_bytes(), given.as_bytes());
        want.len() == given.len()
            && want
                .iter()
                .zip(given)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

/// `bytes` as base64url, without padding (RFC 4648, 5).
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// Percent-encodes `text` onto `out`: all but RFC 3986's unreserved
/// characters.
fn escape(out: &mut String, text: &str) {
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
}

/// The loopback redirect for a listener on `port`:
/// `http://127.0.0.1:<port>/callback`, the only shape the server takes.
#[must_use]
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// A store, by its id and its name in the account.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct StoreRef {
    /// `st_…`.
    pub id: String,
    /// The store's name in the account.
    pub name: String,
}

/// What a sign-in ends with: the new token, and what it opens.
///
/// The token never shows itself, so its [`Debug`](fmt::Debug) is safe.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenAnswer {
    /// The new token.
    pub token: Token,
    /// The name it was given.
    pub name: String,
    /// The account it belongs to.
    pub account: Account,
    /// The stores it opens, in the order the account made them.
    pub stores: Vec<StoreRef>,
    /// The account's default store, when the token opens it.
    pub default_store: Option<StoreRef>,
}

/// The token answer as it comes.
#[derive(Deserialize)]
struct AnswerWire {
    token: String,
    name: String,
    account: Account,
    stores: Vec<StoreRef>,
    default_store: Option<StoreRef>,
}

impl TokenAnswer {
    /// Reads the token answer in `body`. An answer whose token is not one
    /// is a protocol error, and the error never holds it.
    fn read(status: u16, body: &[u8]) -> Result<TokenAnswer, Error> {
        let protocol = |message: String| Error::Protocol { status, message };
        let wire: AnswerWire = serde_json::from_slice(body)
            .map_err(|error| protocol(format!("the token answer does not read: {error}")))?;
        let token = wire
            .token
            .parse()
            .map_err(|_| protocol("the token in the answer is not one".to_owned()))?;
        Ok(TokenAnswer {
            token,
            name: wire.name,
            account: wire.account,
            stores: wire.stores,
            default_store: wire.default_store,
        })
    }

    /// Checks `whoami`, asked with the new token, against this answer: the
    /// same account and name, the same stores for a token that opens some
    /// (`token.stores` is `None` for one that opens every store), and every
    /// store this answer names listed.
    ///
    /// # Errors
    ///
    /// Gives what does not agree.
    pub fn check(&self, whoami: &Whoami) -> Result<(), String> {
        if whoami.account.id != self.account.id {
            return Err(format!(
                "the token is for the account {}, not {}",
                whoami.account.id, self.account.id
            ));
        }
        if whoami.token.name != self.name {
            return Err(format!(
                "the token is named {:?}, not {:?}",
                whoami.token.name, self.name
            ));
        }
        let named: BTreeSet<&str> = self.stores.iter().map(|s| s.id.as_str()).collect();
        if let Some(scope) = &whoami.token.stores {
            let scope: BTreeSet<&str> = scope.iter().map(String::as_str).collect();
            if scope != named {
                return Err("the token opens other stores than the sign-in said".to_owned());
            }
        }
        let listed: BTreeSet<&str> = whoami.stores.iter().map(|s| s.id.as_str()).collect();
        match self.stores.iter().find(|s| !listed.contains(s.id.as_str())) {
            Some(missing) => Err(format!(
                "the store {} ({}) is not among the token's stores",
                missing.name, missing.id
            )),
            None => Ok(()),
        }
    }
}

/// A device sign-in, started: the code to show and where to type it, and
/// how often to poll. The device code itself, a secret, is never shown by
/// [`Debug`](fmt::Debug).
pub struct DeviceCode {
    code: String,
    user_code: String,
    verification_uri: String,
    interval: Duration,
    expires_in: Duration,
}

/// The device start's answer as it comes.
#[derive(Deserialize)]
struct DeviceWire {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

impl DeviceCode {
    /// The code the person types, such as `BCDF-GHJK`.
    #[must_use]
    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    /// The page to type it on, such as `https://cases.sh/device`.
    #[must_use]
    pub fn verification_uri(&self) -> &str {
        &self.verification_uri
    }

    /// How long to wait between polls now: the server's interval, at least
    /// [`POLL_FLOOR`], and [`SLOW_DOWN`] more for each `slow_down`.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// How long the code lasts, from the start.
    #[must_use]
    pub fn expires_in(&self) -> Duration {
        self.expires_in
    }

    /// What to do after a poll that failed with `error`: how long to wait
    /// before the next poll, or `None` to stop with the error.
    ///
    /// - `authorization_pending`: the interval.
    /// - `slow_down`: the interval grows by [`SLOW_DOWN`], for good.
    /// - 429: its `Retry-After`, at most [`RATE_WAIT`](crate::RATE_WAIT)
    ///   and 1 s when none is sent, and never less than the interval.
    /// - No answer, or a 5xx (503 `unavailable`: a store could not take
    ///   the token, and the code stays approved): the interval.
    /// - Anything else, `access_denied`, `expired_token` and
    ///   `invalid_grant` among them: stop.
    ///
    /// The caller keeps to [`expires_in`](DeviceCode::expires_in).
    pub fn wait_after(&mut self, error: &Error) -> Option<Duration> {
        self.wait_within(error, &Limits::default())
    }

    fn wait_within(&mut self, error: &Error, limits: &Limits) -> Option<Duration> {
        match error {
            Error::Signin {
                kind: SigninError::AuthorizationPending,
                ..
            }
            | Error::Transport { .. }
            | Error::Server { .. } => Some(self.interval),
            Error::Signin {
                kind: SigninError::SlowDown,
                ..
            } => {
                self.interval += SLOW_DOWN;
                Some(self.interval)
            }
            Error::Rate { retry_after } => Some(
                retry_after
                    .unwrap_or(limits.rate_default)
                    .min(limits.rate_wait)
                    .max(self.interval),
            ),
            _ => None,
        }
    }
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("interval", &self.interval)
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// The sign-in routes of one server, which take no token: the loopback
/// sign-in's approval address and exchange, and the device code.
///
/// Its requests are sent as a [`Client`]'s are: `User-Agent:
/// cases/<version>`, no redirect followed, the environment's proxy, and
/// each given [`SIGNIN_TIMEOUT`](crate::SIGNIN_TIMEOUT). No error holds a
/// code, a verifier or a token.
#[derive(Clone, Debug)]
pub struct Signin {
    client: Client,
}

impl Signin {
    /// The sign-in routes of `server`. Nothing is sent until a request is
    /// made.
    #[must_use]
    pub fn new(server: Server) -> Signin {
        Signin::with_limits(server, Limits::default())
    }

    pub(crate) fn with_limits(server: Server, limits: Limits) -> Signin {
        Signin {
            client: Client::unauthenticated(server, limits),
        }
    }

    /// The sign-in, sending `User-Agent: cases/<version>`.
    #[must_use]
    pub fn with_version(mut self, version: &str) -> Signin {
        self.client = self.client.with_version(version);
        self
    }

    /// The server.
    #[must_use]
    pub fn server(&self) -> &Server {
        self.client.server()
    }

    /// The approval page for a loopback sign-in, to open in a signed-in
    /// browser: `{server}/cli/authorize?…` with the challenge, the state,
    /// `redirect` (see [`redirect_uri`]), the token's `name`, and the names
    /// of the `stores` it is to open, joined by commas, or none for every
    /// store. Each value is percent-encoded.
    #[must_use]
    pub fn authorize_url(
        &self,
        pkce: &Pkce,
        redirect: &str,
        name: &str,
        stores: Option<&[String]>,
    ) -> String {
        let mut url = format!("{}/cli/authorize?", self.server());
        let joined = stores.map(|names| names.join(","));
        let pairs = [
            ("code_challenge", Some(pkce.challenge.as_str())),
            ("code_challenge_method", Some("S256")),
            ("state", Some(pkce.state.as_str())),
            ("redirect_uri", Some(redirect)),
            ("name", Some(name)),
            ("stores", joined.as_deref()),
        ];
        for (i, (key, value)) in pairs.iter().enumerate() {
            let Some(value) = value else { continue };
            if i > 0 {
                url.push('&');
            }
            url.push_str(key);
            url.push('=');
            escape(&mut url, value);
        }
        url
    }

    /// Trades the `code` the browser brought back for a token: `POST
    /// /v1/cli/token` `{code, code_verifier, redirect_uri}`, with the same
    /// `redirect` the approval had.
    ///
    /// A 503 (a store could not take the token; the server kept the code)
    /// is sent again after each of [`EXCHANGE_RETRIES`](crate::EXCHANGE_RETRIES).
    /// One that gets no answer is sent once more, since the server may have
    /// made the token and lost the answer.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Signin`] with [`SigninError::InvalidGrant`] for a code
    /// that is unknown, spent or expired, or does not match; [`Error::Lost`]
    /// when that comes after an exchange that got no answer;
    /// [`Error::Server`] when the 503s outlast the retries; and the client's
    /// other errors.
    pub fn exchange(&self, code: &str, pkce: &Pkce, redirect: &str) -> Result<TokenAnswer, Error> {
        let body = json!({
            "code": code,
            "code_verifier": pkce.verifier,
            "redirect_uri": redirect,
        });
        let call = self.post("/v1/cli/token", &body, Retry::Write);
        let mut waits = self.client.limits().exchange_retries.iter();
        let mut resent = false;
        loop {
            let error = match self.client.send(&call) {
                Ok((status, body)) => return TokenAnswer::read(status, &body),
                Err(failure) => signin_error(&failure),
            };
            match error {
                Error::Server { status: 503, .. } => match waits.next() {
                    Some(wait) => thread::sleep(*wait),
                    None => return Err(error),
                },
                Error::Transport { .. } if !resent => resent = true,
                Error::Signin {
                    kind: SigninError::InvalidGrant,
                    ..
                } if resent => {
                    return Err(Error::Lost {
                        server: self.server().to_string(),
                    });
                }
                error => return Err(error),
            }
        }
    }

    /// Starts a device sign-in: `POST /v1/cli/device` `{name, stores?}`,
    /// with the names of the `stores` the token is to open, or none for
    /// every store. It is not sent again, even after a 429.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Rate`] when too many sign-ins wait on the server,
    /// [`Error::Request`] for a name or store name it refuses, and the
    /// client's other errors.
    pub fn device_start(&self, name: &str, stores: Option<&[String]>) -> Result<DeviceCode, Error> {
        let mut body = json!({ "name": name });
        if let Some(stores) = stores {
            body["stores"] = json!(stores);
        }
        let call = self.post("/v1/cli/device", &body, Retry::Never);
        let (status, body) = self
            .client
            .send(&call)
            .map_err(|failure| signin_error(&failure))?;
        let wire: DeviceWire = serde_json::from_slice(&body).map_err(|error| Error::Protocol {
            status,
            message: format!("the device start's answer does not read: {error}"),
        })?;
        Ok(DeviceCode {
            code: wire.device_code,
            user_code: wire.user_code,
            verification_uri: wire.verification_uri,
            interval: Duration::from_secs(wire.interval).max(POLL_FLOOR),
            expires_in: Duration::from_secs(wire.expires_in),
        })
    }

    /// Polls a device sign-in once: `POST /v1/cli/device/token`
    /// `{device_code}`. It is not sent again: what to do after an error is
    /// [`DeviceCode::wait_after`]'s.
    ///
    /// # Errors
    ///
    /// Gives [`Error::Signin`] with the kind RFC 8628 names
    /// (`authorization_pending`, `slow_down`, `access_denied`,
    /// `expired_token`) or `invalid_grant`, [`Error::Rate`] for a 429
    /// whatever its body, and the client's other errors.
    pub fn device_poll(&self, device: &DeviceCode) -> Result<TokenAnswer, Error> {
        let body = json!({ "device_code": device.code });
        let call = self.post("/v1/cli/device/token", &body, Retry::Never);
        let (status, body) = self
            .client
            .send(&call)
            .map_err(|failure| signin_error(&failure))?;
        TokenAnswer::read(status, &body)
    }

    /// A `POST` of `body` to `path`.
    fn post<'a>(&self, path: &'a str, body: &serde_json::Value, retry: Retry) -> Call<'a> {
        Call {
            method: "POST",
            path,
            body: Some(body.to_string().into_bytes()),
            timeout: Some(self.client.limits().signin),
            retry,
        }
    }
}

/// The error of a sign-in request that failed. A 404 means the server has
/// no such route, not that a store is missing.
fn signin_error(failure: &Failure) -> Error {
    match failure {
        Failure::Error(Error::NoStore) => Error::Protocol {
            status: 404,
            message: "the server has no sign-in route here".to_owned(),
        },
        other => protocol_of(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_follows_rfc_7636_appendix_b() {
        let octets = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let pkce = Pkce::from_bytes(&octets, &octets);
        assert_eq!(pkce.verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            pkce.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn base64url_is_rfc_4648_without_padding() {
        for (bytes, want) in [
            (&b""[..], ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff, 0xfe], "-__-"),
            (&[0, 0x10, 0x83], "ABCD"),
        ] {
            assert_eq!(base64url(bytes), want, "{bytes:?}");
        }
    }

    #[test]
    fn new_secrets_are_43_base64url_characters_and_differ() {
        let one = Pkce::new().unwrap();
        let two = Pkce::new().unwrap();
        let url = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        for text in [&one.verifier, &one.state, &one.challenge] {
            assert_eq!(text.len(), 43, "{text}");
            assert!(text.chars().all(url), "{text}");
        }
        assert_ne!(one.verifier, one.state);
        assert_ne!(one.verifier, two.verifier);
        assert_ne!(one.state, two.state);
        assert_eq!(
            one.challenge,
            base64url(&Sha256::digest(one.verifier.as_bytes()))
        );
    }

    #[test]
    fn the_state_is_compared_whole() {
        let pkce = Pkce::from_bytes(&[1; 32], &[2; 32]);
        let state = pkce.state.clone();
        assert!(pkce.state_is(&state));
        assert!(!pkce.state_is(""));
        assert!(!pkce.state_is(&state[1..]));
        assert!(!pkce.state_is(&format!("{state}A")));
        let mut last = state.clone();
        last.pop();
        last.push('x');
        assert!(!pkce.state_is(&last));
        let mut first = state.clone();
        first.replace_range(..1, "x");
        assert!(!pkce.state_is(&first));
        // Two differences that are the same bits do not cancel out.
        let twice: String = state
            .bytes()
            .enumerate()
            .map(|(i, b)| char::from(if i < 2 { b ^ 1 } else { b }))
            .collect();
        assert!(!pkce.state_is(&twice));
    }

    #[test]
    fn debug_hides_the_verifier_the_state_and_the_device_code() {
        let pkce = Pkce::from_bytes(&[7; 32], &[9; 32]);
        let debug = format!("{pkce:?} {pkce:#?}");
        assert!(!debug.contains(&pkce.verifier), "{debug}");
        assert!(!debug.contains(&pkce.state), "{debug}");
        assert!(debug.contains(&pkce.challenge), "{debug}");
        let device = DeviceCode {
            code: "ac_fake.SECRETdevicecode".into(),
            user_code: "BCDF-GHJK".into(),
            verification_uri: "https://cases.sh/device".into(),
            interval: Duration::from_secs(5),
            expires_in: Duration::from_secs(600),
        };
        let debug = format!("{device:?} {device:#?}");
        assert!(!debug.contains("SECRET"), "{debug}");
        assert!(debug.contains("BCDF-GHJK"), "{debug}");
    }

    fn signin() -> Signin {
        Signin::new("https://cases.sh".parse().unwrap())
    }

    #[test]
    fn the_approval_address_escapes_each_value() {
        let pkce = Pkce::from_bytes(&[0; 32], &[255; 32]);
        let url = signin().authorize_url(
            &pkce,
            &redirect_uri(49152),
            "cases CLI on mbp & co",
            Some(&["team-x".to_owned(), "ops".to_owned()]),
        );
        assert_eq!(
            url,
            format!(
                "https://cases.sh/cli/authorize?code_challenge={}&code_challenge_method=S256\
                 &state={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback\
                 &name=cases%20CLI%20on%20mbp%20%26%20co&stores=team-x%2Cops",
                pkce.challenge, pkce.state
            )
        );
        let every = signin().authorize_url(&pkce, &redirect_uri(1), "a=b~c_d.e", None);
        assert!(!every.contains("stores"), "{every}");
        assert!(every.ends_with("&name=a%3Db~c_d.e"), "{every}");
        let mut text = String::new();
        escape(&mut text, "é/?#+");
        assert_eq!(text, "%C3%A9%2F%3F%23%2B");
    }

    #[test]
    fn a_sign_in_keeps_its_limits() {
        let limits = Limits {
            signin: Duration::from_secs(7),
            exchange_retries: [Duration::from_millis(1); 3],
            ..Limits::default()
        };
        let signin = Signin::with_limits("https://cases.sh".parse().unwrap(), limits);
        assert_eq!(signin.client.limits().signin, Duration::from_secs(7));
        assert_eq!(
            signin.client.limits().exchange_retries,
            [Duration::from_millis(1); 3]
        );
    }

    #[test]
    fn the_redirect_is_the_loopback_callback() {
        assert_eq!(redirect_uri(8123), "http://127.0.0.1:8123/callback");
    }

    #[test]
    fn the_exchange_waits_longer_than_the_server_does() {
        assert!(crate::SIGNIN_TIMEOUT >= Duration::from_secs(20));
        assert_eq!(Limits::default().signin, crate::SIGNIN_TIMEOUT);
        assert_eq!(
            crate::EXCHANGE_RETRIES,
            [
                Duration::from_secs(2),
                Duration::from_secs(5),
                Duration::from_secs(10)
            ]
        );
        assert_eq!(Limits::default().exchange_retries, crate::EXCHANGE_RETRIES);
        assert_eq!(SLOW_DOWN, Duration::from_secs(5));
        assert_eq!(POLL_FLOOR, Duration::from_secs(1));
    }

    const TOKEN: &str = "csu_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn answer(token: &str) -> String {
        format!(
            r#"{{"token": "{token}", "name": "cases CLI on mbp",
                "account": {{"id": "ac_fake", "handle": "ryan"}},
                "stores": [{{"id": "st_0000000000000001", "name": "me"}},
                           {{"id": "st_0000000000000002", "name": "team-x"}}],
                "default_store": {{"id": "st_0000000000000001", "name": "me"}}}}"#
        )
    }

    #[test]
    fn the_token_answer_reads() {
        let read = TokenAnswer::read(200, answer(TOKEN).as_bytes()).unwrap();
        assert_eq!(read.token.expose(), TOKEN);
        assert_eq!(read.name, "cases CLI on mbp");
        assert_eq!(read.account.id, "ac_fake");
        assert_eq!(read.account.handle, "ryan");
        assert_eq!(read.stores.len(), 2);
        assert_eq!(read.stores[1].name, "team-x");
        assert_eq!(
            read.default_store.map(|s| s.id).as_deref(),
            Some("st_0000000000000001")
        );
        let debug = format!("{:?}", TokenAnswer::read(200, answer(TOKEN).as_bytes()));
        assert!(!debug.contains(TOKEN), "{debug}");
        let none =
            answer(TOKEN).replace(r#"{"id": "st_0000000000000001", "name": "me"}}"#, "null}");
        assert_eq!(
            TokenAnswer::read(200, none.as_bytes())
                .unwrap()
                .default_store,
            None
        );
    }

    #[test]
    fn a_bad_token_answer_is_a_protocol_error_that_never_shows_the_token() {
        let bad_token = answer("csu_short");
        let error = TokenAnswer::read(200, bad_token.as_bytes()).unwrap_err();
        assert_eq!(
            error,
            Error::Protocol {
                status: 200,
                message: "the token in the answer is not one".into()
            }
        );
        let no_account =
            answer(TOKEN).replace(r#""account": {"id": "ac_fake", "handle": "ryan"},"#, "");
        for body in [
            no_account,
            "{}".to_owned(),
            "not json".to_owned(),
            String::new(),
        ] {
            let error = TokenAnswer::read(200, body.as_bytes()).unwrap_err();
            assert!(
                matches!(&error, Error::Protocol { status: 200, message } if message.starts_with("the token answer does not read")),
                "{error:?}"
            );
            assert!(!error.to_string().contains(TOKEN), "{error}");
        }
    }

    fn whoami(account: &str, name: &str, scope: Option<&[&str]>, listed: &[&str]) -> Whoami {
        let value = json!({
            "account": {"id": account, "handle": "ryan"},
            "token": {"name": name, "stores": scope, "expires": null},
            "stores": listed.iter().map(|id| json!({"id": id, "name": id, "default": false})).collect::<Vec<_>>(),
        });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_check_agrees_with_the_answer() {
        let read = TokenAnswer::read(200, answer(TOKEN).as_bytes()).unwrap();
        let (one, two, three) = (
            "st_0000000000000001",
            "st_0000000000000002",
            "st_0000000000000003",
        );
        let name = "cases CLI on mbp";
        // Every store: whoami may list more.
        assert_eq!(
            read.check(&whoami("ac_fake", name, None, &[one, two, three])),
            Ok(())
        );
        // Some stores, in any order.
        assert_eq!(
            read.check(&whoami("ac_fake", name, Some(&[two, one]), &[two, one])),
            Ok(())
        );
        for (who, want) in [
            (
                whoami("ac_other", name, None, &[one, two]),
                "the token is for the account ac_other, not ac_fake",
            ),
            (
                whoami("ac_fake", "tests", None, &[one, two]),
                "the token is named \"tests\", not \"cases CLI on mbp\"",
            ),
            (
                whoami("ac_fake", name, Some(&[one]), &[one, two]),
                "the token opens other stores than the sign-in said",
            ),
            (
                whoami(
                    "ac_fake",
                    name,
                    Some(&[one, two, three]),
                    &[one, two, three],
                ),
                "the token opens other stores than the sign-in said",
            ),
            (
                whoami("ac_fake", name, None, &[one]),
                "the store team-x (st_0000000000000002) is not among the token's stores",
            ),
            (
                whoami("ac_fake", name, Some(&[one, two]), &[two]),
                "the store me (st_0000000000000001) is not among the token's stores",
            ),
        ] {
            assert_eq!(read.check(&who), Err(want.to_owned()));
        }
    }

    fn device(interval: u64) -> DeviceCode {
        DeviceCode {
            code: "ac_fake.x".into(),
            user_code: "BCDF-GHJK".into(),
            verification_uri: "https://cases.sh/device".into(),
            interval: Duration::from_secs(interval),
            expires_in: Duration::from_secs(600),
        }
    }

    fn signin_error_of(kind: SigninError) -> Error {
        Error::Signin {
            kind,
            message: "why".into(),
        }
    }

    #[test]
    fn after_a_poll_the_device_waits_as_rfc_8628_says() {
        let secs = Duration::from_secs;
        let mut code = device(2);
        assert_eq!(
            code.wait_after(&signin_error_of(SigninError::AuthorizationPending)),
            Some(secs(2))
        );
        assert_eq!(
            code.wait_after(&signin_error_of(SigninError::SlowDown)),
            Some(secs(7))
        );
        assert_eq!(code.interval(), secs(7));
        assert_eq!(
            code.wait_after(&signin_error_of(SigninError::AuthorizationPending)),
            Some(secs(7))
        );
        let mut code = device(2);
        for error in [
            Error::Transport {
                server: String::new(),
                reason: String::new(),
            },
            Error::Server {
                status: 503,
                message: "a store could not be reached; try again".into(),
            },
            Error::Server {
                status: 500,
                message: String::new(),
            },
        ] {
            assert_eq!(code.wait_after(&error), Some(secs(2)), "{error:?}");
        }
        for (after, want) in [
            (None, 2),
            (Some(0), 2),
            (Some(4), 4),
            (Some(10), 10),
            (Some(60), 10),
        ] {
            let error = Error::Rate {
                retry_after: after.map(secs),
            };
            assert_eq!(code.wait_after(&error), Some(secs(want)), "{after:?}");
        }
        assert_eq!(code.interval(), secs(2));
        let mut fast = device(0);
        fast.interval = Duration::ZERO;
        assert_eq!(
            fast.wait_after(&Error::Rate { retry_after: None }),
            Some(secs(1))
        );
        for error in [
            signin_error_of(SigninError::AccessDenied),
            signin_error_of(SigninError::ExpiredToken),
            signin_error_of(SigninError::InvalidGrant),
            Error::Auth {
                message: String::new(),
            },
            Error::Request {
                message: String::new(),
            },
            Error::Redirect { status: 302 },
            Error::Protocol {
                status: 200,
                message: String::new(),
            },
        ] {
            assert_eq!(code.wait_after(&error), None, "{error:?}");
        }
        assert_eq!(code.interval(), secs(2));
    }

    #[test]
    fn a_missing_route_is_not_a_missing_store() {
        assert!(matches!(
            signin_error(&Failure::Error(Error::NoStore)),
            Error::Protocol { status: 404, .. }
        ));
        assert_eq!(signin_error(&Failure::Error(Error::Gone)), Error::Gone);
    }
}
