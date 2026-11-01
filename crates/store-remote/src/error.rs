//! What can go wrong with a remote store, beside the core's refusals.

use std::fmt;
use std::time::Duration;

/// What went wrong talking to a server, outside the core's refusals, which
/// come back as [`StoreError::Refused`](cases_core::StoreError::Refused).
///
/// No error holds the token. One that names where it went names the server
/// and the store, never a query.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Refused before anything was sent: a doing line that is not one
    /// (AG-17), or a blank worker. Its category is the core's `invalid`.
    Invalid(cases_core::Error),
    /// The request's body is over [`REQUEST_LIMIT`](crate::REQUEST_LIMIT),
    /// which the server would refuse. Nothing was sent.
    TooLarge {
        /// The body's size, in bytes.
        size: usize,
    },
    /// The server refused the request itself (400 `request`, or another
    /// category): a bug in the client or the server.
    Request {
        /// What the server said.
        message: String,
    },
    /// The server refused the token (401): it is malformed, expired,
    /// revoked, or not for this store.
    Auth {
        /// What the server said, such as `revoked`.
        message: String,
    },
    /// There is no store with this id (404 `request`).
    NoStore,
    /// The forwarded store is not made yet (404 `missing`): only opening a
    /// case makes it, which a read takes as no cases.
    Missing,
    /// The worker is not listening now, so it has no doing line to set
    /// (404 `not-listening`).
    NotListening,
    /// The store was removed (410).
    Gone,
    /// Another wait holds a pickup wait's lock on the forwarded store's
    /// machine (409 `held`): it is already picking up what this one would.
    Held {
        /// What the far side said, naming the wait that holds it.
        message: String,
    },
    /// What a forwarded store's far side held on a lease for a
    /// [`Hold`](crate::Hold), such as a pickup wait's locks, may have been
    /// let go: a renewal failed or was not answered in time, or the far side
    /// said the lease had run out (409 `lapsed`).
    Lapsed {
        /// Why.
        reason: String,
    },
    /// The server is limiting requests (429).
    Rate {
        /// How long the server asked to wait, when it said.
        retry_after: Option<Duration>,
    },
    /// The server answered with a redirect, which the client never
    /// follows: it would drop the token and turn a write into a read.
    Redirect {
        /// The status, such as 302.
        status: u16,
    },
    /// The server failed (5xx).
    Server {
        /// The status, such as 503.
        status: u16,
        /// What the server said, if anything.
        message: String,
    },
    /// The forwarded store was made by a newer cases than the one on the
    /// far side (500 `newer`): that cases must be updated before the store
    /// can be read.
    Newer {
        /// What the far side said.
        message: String,
    },
    /// The answer is over [`ANSWER_LIMIT`](crate::ANSWER_LIMIT).
    AnswerTooLarge,
    /// The answer is not the API's: a status the client does not know, or
    /// a body that does not read.
    Protocol {
        /// The status.
        status: u16,
        /// What was wrong with it.
        message: String,
    },
    /// No answer at all: the name did not resolve, the connection or TLS
    /// failed, the request timed out, or the answer was cut short.
    Transport {
        /// The server, as `https://host[:port]`.
        server: String,
        /// Why, as the HTTP client gives it.
        reason: String,
    },
    /// A forwarded store's far side could not be started, and will not be
    /// on another try: the program is not there, there is no cases on the
    /// far side's PATH, or that cases has no `dial-stdio`. Nothing was
    /// sent to a store.
    Dial {
        /// The dial, such as `ssh://devbox`.
        dial: String,
        /// Why.
        reason: String,
    },
    /// A sign-in route's refusal (400): the code is spent or unknown, or a
    /// device sign-in waits, was denied or ran out.
    Signin {
        /// Which refusal.
        kind: SigninError,
        /// What the server said.
        message: String,
    },
    /// The server does not know this token or its account (404 on a
    /// revoke).
    NoToken,
    /// The exchange got no answer, and sent once more, its code was spent:
    /// the server may have made a token whose answer was lost.
    Lost {
        /// The server, as `https://host[:port]`.
        server: String,
    },
}

/// What a sign-in route refused, by the category of its 400: the codes of
/// RFC 8628 and OAuth's `invalid_grant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SigninError {
    /// `invalid_grant`: the code is unknown, spent or expired, or the
    /// verifier or the redirect does not match it.
    InvalidGrant,
    /// `authorization_pending`: the device sign-in waits for approval.
    AuthorizationPending,
    /// `slow_down`: the device polled too soon; the interval grows by
    /// [`SLOW_DOWN`](crate::SLOW_DOWN).
    SlowDown,
    /// `access_denied`: the person denied the sign-in.
    AccessDenied,
    /// `expired_token`: the device code ran out before it was approved.
    ExpiredToken,
}

impl SigninError {
    /// The refusal of the category `name`, if it is a sign-in one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<SigninError> {
        Some(match name {
            "invalid_grant" => SigninError::InvalidGrant,
            "authorization_pending" => SigninError::AuthorizationPending,
            "slow_down" => SigninError::SlowDown,
            "access_denied" => SigninError::AccessDenied,
            "expired_token" => SigninError::ExpiredToken,
            _ => return None,
        })
    }

    /// The category's name, such as `slow_down`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SigninError::InvalidGrant => "invalid_grant",
            SigninError::AuthorizationPending => "authorization_pending",
            SigninError::SlowDown => "slow_down",
            SigninError::AccessDenied => "access_denied",
            SigninError::ExpiredToken => "expired_token",
        }
    }
}

impl Error {
    /// Whether the same request may work if it is sent again later: no
    /// answer at all, the server limiting requests, or a 502, 503 or 504.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Transport { .. } | Error::Rate { .. } => true,
            Error::Server { status, .. } => matches!(status, 502..=504),
            _ => false,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(error) => error.fmt(f),
            Error::TooLarge { size } => write!(
                f,
                "the request is {size} bytes, over the server's limit of {} bytes; nothing was sent",
                crate::REQUEST_LIMIT
            ),
            Error::Request { message } => {
                write!(f, "the server refused the request: {message}")
            }
            Error::Auth { message } => write!(f, "the server refused the token: {message}"),
            Error::NoStore => f.write_str("the server has no store with this id"),
            Error::Missing => f.write_str("the store is not made yet"),
            Error::NotListening => f.write_str("the worker is not listening now"),
            Error::Gone => f.write_str("the store was removed"),
            Error::Held { message } | Error::Newer { message } => f.write_str(message),
            Error::Lapsed { reason } => write!(
                f,
                "this wait's locks on the far side may have been let go ({reason}); it stops, \
                 as another wait could now pick up what it would"
            ),
            Error::Rate {
                retry_after: Some(after),
            } => write!(
                f,
                "the server is limiting requests; try again in {} s",
                after.as_secs().max(1)
            ),
            Error::Rate { retry_after: None } => {
                f.write_str("the server is limiting requests; try again later")
            }
            Error::Redirect { status } => write!(
                f,
                "the server answered {status} with a redirect, which cases does not follow: \
                 check the server's address"
            ),
            Error::Server { status, message } if message.is_empty() => {
                write!(f, "the server failed ({status})")
            }
            Error::Server { status, message } => {
                write!(f, "the server failed ({status}): {message}")
            }
            Error::AnswerTooLarge => write!(
                f,
                "the server's answer is over the limit of {} MiB",
                crate::ANSWER_LIMIT / (1024 * 1024)
            ),
            Error::Protocol { status, message } => {
                write!(
                    f,
                    "the server's answer ({status}) is not the API's: {message}"
                )
            }
            Error::Transport { server, reason } => write!(f, "could not reach {server}: {reason}"),
            Error::Dial { dial, reason } => write!(f, "could not reach {dial}: {reason}"),
            Error::Signin { kind, message } => match kind {
                SigninError::InvalidGrant => write!(f, "the server refused the sign-in: {message}"),
                SigninError::AuthorizationPending => f.write_str("the sign-in waits for approval"),
                SigninError::SlowDown => f.write_str("the server asked to be polled less often"),
                SigninError::AccessDenied => f.write_str("the sign-in was denied"),
                SigninError::ExpiredToken => {
                    f.write_str("the code ran out before it was approved: start again")
                }
            },
            Error::NoToken => f.write_str("the server does not know this token or its account"),
            Error::Lost { server } => write!(
                f,
                "the sign-in may have made a token that never arrived: revoke any you don't \
                 recognise at {server}/tokens, then sign in again"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_no_answer_a_limit_and_a_gateway() {
        let transport = Error::Transport {
            server: "https://cases.sh".into(),
            reason: "timeout".into(),
        };
        assert!(transport.is_transient());
        assert!(Error::Rate { retry_after: None }.is_transient());
        for status in [502, 503, 504] {
            let error = Error::Server {
                status,
                message: String::new(),
            };
            assert!(error.is_transient(), "{status}");
        }
        for status in [500, 501, 505] {
            let error = Error::Server {
                status,
                message: String::new(),
            };
            assert!(!error.is_transient(), "{status}");
        }
        for error in [
            Error::Gone,
            Error::NoStore,
            Error::NotListening,
            Error::AnswerTooLarge,
            Error::Redirect { status: 302 },
            Error::TooLarge { size: 1 },
            Error::Auth {
                message: "revoked".into(),
            },
            Error::Request {
                message: String::new(),
            },
            Error::Protocol {
                status: 200,
                message: String::new(),
            },
            Error::NoToken,
            Error::Held {
                message: String::new(),
            },
            Error::Lapsed {
                reason: String::new(),
            },
            Error::Lost {
                server: String::new(),
            },
            Error::Signin {
                kind: SigninError::AuthorizationPending,
                message: String::new(),
            },
        ] {
            assert!(!error.is_transient(), "{error:?}");
        }
    }

    #[test]
    fn errors_say_what_went_wrong() {
        for (error, want) in [
            (
                Error::TooLarge { size: 2_000_000 },
                "the request is 2000000 bytes, over the server's limit of 1048576 bytes; nothing \
                 was sent",
            ),
            (
                Error::Request {
                    message: "bad".into(),
                },
                "the server refused the request: bad",
            ),
            (
                Error::Auth {
                    message: "revoked".into(),
                },
                "the server refused the token: revoked",
            ),
            (Error::NoStore, "the server has no store with this id"),
            (Error::NotListening, "the worker is not listening now"),
            (Error::Gone, "the store was removed"),
            (
                Error::Held {
                    message: "another wait is already picking up worker \"lead\"".into(),
                },
                "another wait is already picking up worker \"lead\"",
            ),
            (
                Error::Rate {
                    retry_after: Some(Duration::from_secs(30)),
                },
                "the server is limiting requests; try again in 30 s",
            ),
            (
                Error::Rate {
                    retry_after: Some(Duration::ZERO),
                },
                "the server is limiting requests; try again in 1 s",
            ),
            (
                Error::Rate { retry_after: None },
                "the server is limiting requests; try again later",
            ),
            (
                Error::Redirect { status: 302 },
                "the server answered 302 with a redirect, which cases does not follow: check the \
                 server's address",
            ),
            (
                Error::Server {
                    status: 503,
                    message: String::new(),
                },
                "the server failed (503)",
            ),
            (
                Error::Server {
                    status: 500,
                    message: "boom".into(),
                },
                "the server failed (500): boom",
            ),
            (
                Error::AnswerTooLarge,
                "the server's answer is over the limit of 256 MiB",
            ),
            (
                Error::Protocol {
                    status: 418,
                    message: "no".into(),
                },
                "the server's answer (418) is not the API's: no",
            ),
            (
                Error::Transport {
                    server: "https://cases.sh".into(),
                    reason: "timeout".into(),
                },
                "could not reach https://cases.sh: timeout",
            ),
            (
                Error::Signin {
                    kind: SigninError::InvalidGrant,
                    message: "no such code".into(),
                },
                "the server refused the sign-in: no such code",
            ),
            (
                Error::Signin {
                    kind: SigninError::AuthorizationPending,
                    message: "why".into(),
                },
                "the sign-in waits for approval",
            ),
            (
                Error::Signin {
                    kind: SigninError::SlowDown,
                    message: "why".into(),
                },
                "the server asked to be polled less often",
            ),
            (
                Error::Signin {
                    kind: SigninError::AccessDenied,
                    message: "why".into(),
                },
                "the sign-in was denied",
            ),
            (
                Error::Signin {
                    kind: SigninError::ExpiredToken,
                    message: "why".into(),
                },
                "the code ran out before it was approved: start again",
            ),
            (
                Error::NoToken,
                "the server does not know this token or its account",
            ),
            (
                Error::Lost {
                    server: "https://cases.sh".into(),
                },
                "the sign-in may have made a token that never arrived: revoke any you don't \
                 recognise at https://cases.sh/tokens, then sign in again",
            ),
            (
                Error::Invalid(cases_core::Error::new(
                    cases_core::Category::Invalid,
                    "too long",
                )),
                "too long",
            ),
        ] {
            assert_eq!(error.to_string(), want);
        }
    }

    #[test]
    fn a_sign_in_refusal_is_named_by_its_category() {
        for (name, kind) in [
            ("invalid_grant", SigninError::InvalidGrant),
            ("authorization_pending", SigninError::AuthorizationPending),
            ("slow_down", SigninError::SlowDown),
            ("access_denied", SigninError::AccessDenied),
            ("expired_token", SigninError::ExpiredToken),
        ] {
            assert_eq!(SigninError::from_name(name), Some(kind));
            assert_eq!(kind.as_str(), name);
        }
        for name in [
            "",
            "rate",
            "request",
            "unavailable",
            "SLOW_DOWN",
            "slow-down",
        ] {
            assert_eq!(SigninError::from_name(name), None, "{name}");
        }
    }
}
