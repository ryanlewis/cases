//! Signing in against the fake server: the loopback exchange, the device
//! code and the revoke, with their faults, and what goes on the wire.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

use std::thread;
use std::time::Duration;

use cases_core::Store as _;

use crate::client::Limits;
use crate::fake::{ACCOUNT, DEVICE_INTERVAL, Fake, Fault, TOKEN};
use crate::signin::{Pkce, redirect_uri};
use crate::tests::quick;
use crate::{Client, Error, Signin, SigninError, Token, TokenAnswer};

fn signin(fake: &Fake) -> Signin {
    Signin::with_limits(fake.url().parse().unwrap(), quick())
}

fn client(fake: &Fake, token: &Token) -> Client {
    Client::with_limits(fake.url().parse().unwrap(), token.clone(), quick())
}

/// The value of `name` in the query of `url`.
fn param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

const REDIRECT_PORT: u16 = 49152;

/// A loopback sign-in up to the exchange: the secrets, and the code the
/// browser brought back.
fn approved(fake: &Fake, stores: Option<&[String]>) -> (Pkce, String) {
    let pkce = Pkce::new().unwrap();
    let url = signin(fake).authorize_url(
        &pkce,
        &redirect_uri(REDIRECT_PORT),
        "cases CLI on mbp",
        stores,
    );
    let back = fake.approve(&url).unwrap();
    assert!(
        back.starts_with(&format!("{}?code=", redirect_uri(REDIRECT_PORT))),
        "{back}"
    );
    assert!(pkce.state_is(&param(&back, "state").unwrap()));
    (pkce, param(&back, "code").unwrap())
}

fn exchange(fake: &Fake, pkce: &Pkce, code: &str) -> Result<TokenAnswer, Error> {
    signin(fake).exchange(code, pkce, &redirect_uri(REDIRECT_PORT))
}

#[test]
fn a_loopback_sign_in_gives_a_token_that_works() {
    let fake = Fake::start();
    let me = fake.add_store("me");
    let team = fake.add_store("team-x");
    let (pkce, code) = approved(&fake, None);
    let answer = exchange(&fake, &pkce, &code).unwrap();
    assert_eq!(answer.name, "cases CLI on mbp");
    assert_eq!(
        (answer.account.id.as_str(), answer.account.handle.as_str()),
        (ACCOUNT, "ryan")
    );
    let stores: Vec<_> = answer.stores.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(stores, [me.as_str(), team.as_str()]);
    assert_eq!(
        answer.default_store.as_ref().map(|s| s.name.as_str()),
        Some("me")
    );
    let who = client(&fake, &answer.token).whoami().unwrap();
    assert_eq!(answer.check(&who), Ok(()));
    assert_eq!(who.token.stores, None);
    let minted = &fake.tokens()[1];
    assert_eq!(minted.token, answer.token.expose());
    assert_eq!(minted.stores, None);

    // What went on the wire: the exchange with no token, and the verifier
    // only in its body; the new token only in a later Authorization.
    let requests = fake.requests();
    assert_eq!(
        fake.lines(),
        ["POST /v1/cli/token HTTP/1.1", "GET /v1/cli/whoami HTTP/1.1"]
    );
    let sent = &requests[0];
    assert!(
        !sent.to_ascii_lowercase().contains("authorization"),
        "{sent}"
    );
    let lower = sent.to_ascii_lowercase();
    assert!(lower.contains("content-type: application/json"), "{sent}");
    assert!(
        lower.contains(&format!("user-agent: cases/{}", env!("CARGO_PKG_VERSION"))),
        "{sent}"
    );
    let body: serde_json::Value =
        serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["code"], code.as_str());
    assert_eq!(body["redirect_uri"], redirect_uri(REDIRECT_PORT).as_str());
    let verifier = body["code_verifier"].as_str().unwrap().to_owned();
    assert_eq!(verifier.len(), 43);
    assert!(!requests[1].contains(&verifier));
    let token = answer.token.expose();
    for line in requests[1].lines() {
        assert_eq!(
            line.contains(token),
            line.to_ascii_lowercase()
                .starts_with("authorization: bearer ")
                && line.ends_with(&format!(" {token}")),
            "{line}"
        );
    }
}

#[test]
fn a_scoped_sign_in_opens_only_its_stores() {
    let fake = Fake::start();
    let me = fake.add_store("me");
    let team = fake.add_store("team-x");
    let (pkce, code) = approved(&fake, Some(&["team-x".to_owned()]));
    let answer = exchange(&fake, &pkce, &code).unwrap();
    let stores: Vec<_> = answer.stores.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(stores, [team.as_str()]);
    assert_eq!(answer.default_store, None);
    let scoped = client(&fake, &answer.token);
    let who = scoped.whoami().unwrap();
    assert_eq!(who.token.stores, Some(vec![team.clone()]));
    assert_eq!(answer.check(&who), Ok(()));
    let error = scoped.store(&me).unwrap().rows_since(None).unwrap_err();
    assert_eq!(
        error,
        Error::Auth {
            message: "the token is not for this store".into()
        }
    );
    assert!(scoped.store(&team).unwrap().rows_since(None).is_ok());
}

#[test]
fn a_code_is_spent_by_a_bad_verifier_and_by_another_redirect() {
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    let other = Pkce::new().unwrap();
    let error = exchange(&fake, &other, &code).unwrap_err();
    assert!(
        matches!(&error, Error::Signin { kind: SigninError::InvalidGrant, message } if message == "the verifier or the redirect does not match"),
        "{error:?}"
    );
    assert!(!error.to_string().contains(&code), "{error}");
    // Spent: the right verifier is too late.
    assert!(matches!(
        exchange(&fake, &pkce, &code),
        Err(Error::Signin {
            kind: SigninError::InvalidGrant,
            ..
        })
    ));
    let (pkce, code) = approved(&fake, None);
    let error = signin(&fake)
        .exchange(&code, &pkce, &redirect_uri(REDIRECT_PORT + 1))
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Signin {
                kind: SigninError::InvalidGrant,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(fake.tokens().len(), 1);
}

#[test]
fn a_denial_goes_back_with_access_denied_and_the_state() {
    let fake = Fake::start();
    fake.add_store("me");
    let pkce = Pkce::new().unwrap();
    let url = signin(&fake).authorize_url(&pkce, &redirect_uri(REDIRECT_PORT), "cli", None);
    let back = fake.deny(&url).unwrap();
    assert_eq!(param(&back, "error").as_deref(), Some("access_denied"));
    assert!(pkce.state_is(&param(&back, "state").unwrap()));
    assert_eq!(param(&back, "code"), None);
}

#[test]
fn the_fake_s_approval_checks_the_ask_as_the_worker_does() {
    let fake = Fake::start();
    fake.add_store("me");
    let pkce = Pkce::new().unwrap();
    let good = signin(&fake).authorize_url(&pkce, &redirect_uri(REDIRECT_PORT), "cli", None);
    assert!(fake.approve(&good).is_ok());
    for (bad, want) in [
        (
            good.replace("S256", "plain"),
            "The sign-in must use PKCE with S256.",
        ),
        (
            good.replace("127.0.0.1%3A", "localhost%3A"),
            "The sign-in may only go back to http://127.0.0.1:<port>/callback.",
        ),
        (
            good.replace("&name=cli", "&name="),
            "The token's name is 1 to 100 characters.",
        ),
        (format!("{good}&stores=Team"), "A store name is not one."),
        (
            format!("{good}&stores=team-x"),
            "There is no store named team-x.",
        ),
    ] {
        assert_eq!(fake.approve(&bad), Err(want.to_owned()), "{bad}");
    }
}

#[test]
fn an_exchange_that_gets_503_is_sent_again_after_each_wait() {
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    for _ in 0..3 {
        fake.fault(Fault::Unavailable);
    }
    assert!(exchange(&fake, &pkce, &code).is_ok());
    assert_eq!(fake.lines().len(), 4);

    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    for _ in 0..4 {
        fake.fault(Fault::Unavailable);
    }
    assert_eq!(
        exchange(&fake, &pkce, &code).unwrap_err(),
        Error::Server {
            status: 503,
            message: "a store could not be reached; try again".into()
        }
    );
    assert_eq!(fake.lines().len(), 4);
    // The code was kept.
    assert!(exchange(&fake, &pkce, &code).is_ok());
}

#[test]
fn an_exchange_with_no_answer_is_sent_once_more() {
    // The answer was lost before the server did anything: the second try
    // gets the token.
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Hangup);
    assert!(exchange(&fake, &pkce, &code).is_ok());
    assert_eq!(fake.lines().len(), 2);

    // The server made the token and the answer was lost: the code is
    // spent, and the error says to look at /tokens.
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Lost);
    let error = exchange(&fake, &pkce, &code).unwrap_err();
    assert_eq!(error, Error::Lost { server: fake.url() });
    assert_eq!(fake.lines().len(), 2);
    assert_eq!(fake.tokens().len(), 2);

    // Only once more.
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Hangup);
    fake.fault(Fault::Hangup);
    assert!(matches!(
        exchange(&fake, &pkce, &code),
        Err(Error::Transport { .. })
    ));
    assert_eq!(fake.lines().len(), 2);

    // invalid_grant with no lost answer before it is the server's refusal.
    let fake = Fake::start();
    fake.fault(Fault::Refuse(400, "invalid_grant"));
    let error = exchange(&fake, &Pkce::new().unwrap(), "ac_fake.x").unwrap_err();
    assert!(
        matches!(
            error,
            Error::Signin {
                kind: SigninError::InvalidGrant,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn an_exchange_waits_out_one_429() {
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Rate(Some(0)));
    assert!(exchange(&fake, &pkce, &code).is_ok());
    assert_eq!(fake.lines().len(), 2);

    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Rate(Some(0)));
    fake.fault(Fault::Rate(Some(0)));
    assert_eq!(
        exchange(&fake, &pkce, &code).unwrap_err(),
        Error::Rate {
            retry_after: Some(Duration::ZERO)
        }
    );
}

#[test]
fn a_sign_in_request_is_given_the_sign_in_timeout() {
    let fake = Fake::start();
    fake.add_store("me");
    let limits = Limits {
        request: Duration::from_millis(100),
        signin: Duration::from_secs(5),
        ..quick()
    };
    let slow = Signin::with_limits(fake.url().parse().unwrap(), limits.clone());
    let (pkce, code) = approved(&fake, None);
    fake.fault(Fault::Slow(Duration::from_millis(400)));
    assert!(
        slow.exchange(&code, &pkce, &redirect_uri(REDIRECT_PORT))
            .is_ok()
    );
    fake.fault(Fault::Slow(Duration::from_millis(400)));
    assert!(slow.device_start("cli", None).is_ok());
    let device = slow.device_start("cli", None).unwrap();
    fake.fault(Fault::Slow(Duration::from_millis(400)));
    assert!(matches!(
        slow.device_poll(&device),
        Err(Error::Signin {
            kind: SigninError::AuthorizationPending,
            ..
        })
    ));
    let answer = {
        let (pkce, code) = approved(&fake, None);
        slow.exchange(&code, &pkce, &redirect_uri(REDIRECT_PORT))
            .unwrap()
    };
    let client = Client::with_limits(fake.url().parse().unwrap(), answer.token, limits);
    fake.fault(Fault::Slow(Duration::from_millis(400)));
    assert_eq!(client.revoke(ACCOUNT), Ok(()));
}

#[test]
fn a_device_sign_in_waits_then_gets_its_token() {
    let fake = Fake::start();
    let me = fake.add_store("me");
    fake.add_store("team-x");
    let signin = signin(&fake);
    let mut device = signin.device_start("cases CLI on ci", None).unwrap();
    assert_eq!(fake.user_codes(), [device.user_code().to_owned()]);
    assert_eq!(device.verification_uri(), format!("{}/device", fake.url()));
    assert_eq!(device.interval(), DEVICE_INTERVAL);
    assert_eq!(device.expires_in(), Duration::from_secs(600));
    let pending = signin.device_poll(&device).unwrap_err();
    assert!(
        matches!(
            pending,
            Error::Signin {
                kind: SigninError::AuthorizationPending,
                ..
            }
        ),
        "{pending:?}"
    );
    assert_eq!(device.wait_after(&pending), Some(DEVICE_INTERVAL));
    // Too soon.
    let slow = signin.device_poll(&device).unwrap_err();
    assert!(
        matches!(
            slow,
            Error::Signin {
                kind: SigninError::SlowDown,
                ..
            }
        ),
        "{slow:?}"
    );
    assert_eq!(
        device.wait_after(&slow),
        Some(DEVICE_INTERVAL + Duration::from_secs(5))
    );
    fake.approve_device(device.user_code()).unwrap();
    thread::sleep(DEVICE_INTERVAL);
    let answer = signin.device_poll(&device).unwrap();
    assert_eq!(answer.name, "cases CLI on ci");
    assert_eq!(answer.default_store.map(|s| s.id), Some(me));
    let who = client(&fake, &answer.token).whoami().unwrap();
    assert_eq!(answer.token.expose(), fake.tokens()[1].token);
    assert!(fake.user_codes().is_empty());
    let _ = who;
    // The code is gone once used.
    assert!(matches!(
        signin.device_poll(&device),
        Err(Error::Signin {
            kind: SigninError::InvalidGrant,
            ..
        })
    ));
    // The device code went only in the poll's body, never in a URL.
    for request in fake.requests() {
        let first = request.lines().next().unwrap();
        assert!(!first.contains("device1"), "{first}");
        assert!(
            !request.to_ascii_lowercase().contains("authorization") || first.contains("whoami")
        );
    }
}

#[test]
fn a_device_sign_in_for_some_stores_names_them() {
    let fake = Fake::start();
    fake.add_store("me");
    let team = fake.add_store("team-x");
    let signin = signin(&fake);
    let device = signin
        .device_start("ci", Some(&["team-x".to_owned()]))
        .unwrap();
    let every = signin.device_start("ci", None).unwrap();
    let requests = fake.requests();
    assert!(
        requests[0].ends_with(r#"{"name":"ci","stores":["team-x"]}"#),
        "{}",
        requests[0]
    );
    assert!(requests[1].ends_with(r#"{"name":"ci"}"#), "{}", requests[1]);
    fake.approve_device(device.user_code()).unwrap();
    let answer = signin.device_poll(&device).unwrap();
    let stores: Vec<_> = answer.stores.iter().map(|s| s.id.clone()).collect();
    assert_eq!(stores, [team]);
    let who = client(&fake, &answer.token).whoami().unwrap();
    assert_eq!(answer.check(&who), Ok(()));
    fake.deny_device(every.user_code()).unwrap();
}

#[test]
fn a_device_sign_in_ends_when_denied_or_run_out() {
    let fake = Fake::start();
    fake.add_store("me");
    let signin = signin(&fake);
    for (end, kind) in [
        (
            Fake::deny_device as fn(&Fake, &str) -> Result<(), String>,
            SigninError::AccessDenied,
        ),
        (Fake::expire_device, SigninError::ExpiredToken),
    ] {
        let mut device = signin.device_start("ci", None).unwrap();
        end(&fake, &device.user_code().replace('-', "")).unwrap();
        let error = signin.device_poll(&device).unwrap_err();
        assert!(
            matches!(&error, Error::Signin { kind: k, .. } if *k == kind),
            "{error:?}"
        );
        assert_eq!(device.wait_after(&error), None);
    }
    assert_eq!(fake.tokens().len(), 1);
}

#[test]
fn a_device_start_is_not_sent_again() {
    let fake = Fake::start();
    let signin = signin(&fake);
    fake.fault(Fault::Rate(Some(0)));
    assert_eq!(
        signin.device_start("ci", None).unwrap_err(),
        Error::Rate {
            retry_after: Some(Duration::ZERO)
        }
    );
    fake.fault(Fault::Hangup);
    assert!(matches!(
        signin.device_start("ci", None),
        Err(Error::Transport { .. })
    ));
    assert_eq!(fake.lines().len(), 2);
    assert!(matches!(
        signin.device_start("", None),
        Err(Error::Request { .. })
    ));
    assert!(matches!(
        signin.device_start("ci", Some(&[])),
        Err(Error::Request { .. })
    ));
    fake.fault(Fault::Body("{}".into()));
    assert!(matches!(
        signin.device_start("ci", None),
        Err(Error::Protocol { status: 200, .. })
    ));
}

#[test]
fn a_device_poll_is_read_by_status_and_not_sent_again() {
    let fake = Fake::start();
    fake.add_store("me");
    let signin = signin(&fake);
    let mut device = signin.device_start("ci", None).unwrap();
    fake.approve_device(device.user_code()).unwrap();
    // 429 is Rate, whatever its body's category.
    fake.fault(Fault::Refuse(429, "slow_down"));
    let error = signin.device_poll(&device).unwrap_err();
    assert_eq!(error, Error::Rate { retry_after: None });
    assert_eq!(device.wait_after(&error), Some(DEVICE_INTERVAL));
    // 503: a store could not take the token; the code stays approved.
    fake.fault(Fault::Unavailable);
    let error = signin.device_poll(&device).unwrap_err();
    assert!(
        matches!(error, Error::Server { status: 503, .. }),
        "{error:?}"
    );
    assert_eq!(device.wait_after(&error), Some(DEVICE_INTERVAL));
    fake.fault(Fault::Hangup);
    let error = signin.device_poll(&device).unwrap_err();
    assert!(matches!(error, Error::Transport { .. }), "{error:?}");
    assert_eq!(device.wait_after(&error), Some(DEVICE_INTERVAL));
    assert_eq!(fake.lines().len(), 4);
    assert!(signin.device_poll(&device).is_ok());
}

#[test]
fn a_sign_in_route_the_server_lacks_is_not_a_missing_store() {
    let fake = Fake::start();
    fake.fault(Fault::Refuse(404, "request"));
    assert!(matches!(
        signin(&fake).device_start("ci", None),
        Err(Error::Protocol { status: 404, .. })
    ));
}

#[test]
fn a_revoke_ends_the_token() {
    let fake = Fake::start();
    fake.add_store("me");
    let (pkce, code) = approved(&fake, None);
    let answer = exchange(&fake, &pkce, &code).unwrap();
    let client = client(&fake, &answer.token);
    fake.fault(Fault::Status(503));
    assert_eq!(client.revoke(ACCOUNT), Ok(()));
    assert!(fake.tokens()[1].revoked);
    assert!(!fake.tokens()[0].revoked);
    let lines = fake.lines();
    assert_eq!(
        &lines[lines.len() - 2..],
        [
            "DELETE /v1/accounts/ac_fake/cli/token HTTP/1.1",
            "DELETE /v1/accounts/ac_fake/cli/token HTTP/1.1"
        ]
    );
    assert_eq!(
        client.whoami().unwrap_err(),
        Error::Auth {
            message: "the token is revoked".into()
        }
    );
    // Again: the Worker refuses a revoked token with 401.
    assert_eq!(
        client.revoke(ACCOUNT).unwrap_err(),
        Error::Auth {
            message: "the token is revoked".into()
        }
    );
    assert_eq!(client.revoke("ac_other").unwrap_err(), Error::NoToken);
    let before = fake.lines().len();
    assert!(matches!(
        client.revoke("ac_../x"),
        Err(Error::Invalid(error)) if error.to_string().starts_with("\"ac_../x\" is not an account's id")
    ));
    assert_eq!(fake.lines().len(), before);
}

#[test]
fn a_revoke_of_a_token_nobody_knows_is_no_token() {
    let fake = Fake::start();
    let client = client(&fake, &TOKEN.parse().unwrap());
    for fault in [
        Fault::Refuse(404, "request"),
        Fault::Refuse(404, "nocase"),
        Fault::Refuse(404, "not-listening"),
        Fault::Refuse(404, "other"),
    ] {
        fake.fault(fault);
        assert_eq!(client.revoke(ACCOUNT).unwrap_err(), Error::NoToken);
    }
    fake.fault(Fault::Refuse(410, "gone"));
    assert_eq!(client.revoke(ACCOUNT).unwrap_err(), Error::Gone);
}

#[test]
fn an_expired_token_is_refused_as_expired() {
    let fake = Fake::start();
    fake.expire(TOKEN);
    let client = client(&fake, &TOKEN.parse().unwrap());
    assert_eq!(
        client.whoami().unwrap_err(),
        Error::Auth {
            message: "the token is expired".into()
        }
    );
    fake.revoke(TOKEN);
    assert_eq!(
        client.whoami().unwrap_err(),
        Error::Auth {
            message: "the token is revoked".into()
        }
    );
}
