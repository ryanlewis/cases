//! The fake server takes the token's `Bearer` scheme in any case, as
//! RFC 7235 has it and as `cases serve` takes it, so the vectors over HTTP
//! meet the same check as a served store.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    reason = "a test panics to fail, and uses only part of the shared fake"
)]

mod fake;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use fake::{Fake, TOKEN};

/// The status code of a whoami sent with `authorization`.
fn whoami(fake: &Fake, authorization: &str) -> u16 {
    let addr = fake.url().trim_start_matches("http://").to_owned();
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "GET /v1/cli/whoami HTTP/1.1\r\nhost: fake\r\nauthorization: {authorization}\r\nconnection: close\r\n\r\n"
    )
    .unwrap();
    let mut status = String::new();
    BufReader::new(stream).read_line(&mut status).unwrap();
    status.split(' ').nth(1).unwrap().parse().unwrap()
}

#[test]
fn the_bearer_scheme_is_taken_in_any_case() {
    let fake = Fake::start();
    for scheme in ["Bearer", "bearer", "BEARER", "bEaReR"] {
        assert_eq!(whoami(&fake, &format!("{scheme} {TOKEN}")), 200, "{scheme}");
    }
    for authorization in [
        format!("Basic {TOKEN}"),
        format!("Bearer{TOKEN}"),
        format!("Bearers {TOKEN}"),
        format!("Bearer  {TOKEN}"),
        format!("bearer {TOKEN}x"),
        TOKEN.to_owned(),
        "bearer ".to_owned(),
        "Bear".to_owned(),
    ] {
        assert_eq!(whoami(&fake, &authorization), 401, "{authorization}");
    }
}
