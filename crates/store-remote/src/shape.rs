//! What the client checks before it sends anything: the server's address,
//! the token and a store's id.

use std::fmt;
use std::str::FromStr;

/// Text that is not what it should be: a server, a token or a store id.
/// Nothing was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid(String);

impl Invalid {
    pub(crate) fn new(message: String) -> Invalid {
        Invalid(message)
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// A server's address, checked: `https://host` or `https://host:port`, with
/// no path, query, user or fragment. Plain `http` is taken only for a
/// loopback host (`127.0.0.1`, `localhost` and `[::1]`), for tests.
///
/// The scheme is matched in any case, so `HTTPS://cases.sh` is a server and
/// never a file. The scheme and host are kept in lower case, and one `/` at
/// the end is dropped.
///
/// ```
/// use cases_store_remote::Server;
///
/// let server: Server = "HTTPS://Staging.cases.sh/".parse().unwrap();
/// assert_eq!(server.as_str(), "https://staging.cases.sh");
/// assert!("http://cases.sh".parse::<Server>().is_err());
/// assert!("http://127.0.0.1:8787".parse::<Server>().is_ok());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Server(String);

/// The hosts plain `http` is taken for.
const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

impl Server {
    /// The server's address, such as `https://cases.sh`: the scheme, the
    /// host and any port, with no `/` at the end.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The server's host, with any port, such as `cases.sh` or
    /// `127.0.0.1:8787`.
    #[must_use]
    pub fn host(&self) -> &str {
        self.0.split_once("://").map_or("", |(_, host)| host)
    }
}

impl Server {
    /// The address of a `cases serve` that a forwarded store is reached at
    /// over HTTP, such as `http://devbox.example.ts.net:7338`: as a server's
    /// address, but with plain `http` taken for any host, as for a host on
    /// a tailnet, whose `WireGuard` encrypts it.
    ///
    /// ```
    /// use cases_store_remote::Server;
    ///
    /// let server = Server::forward("HTTP://Devbox.example.ts.net:7338/").unwrap();
    /// assert_eq!(server.as_str(), "http://devbox.example.ts.net:7338");
    /// assert!(Server::forward("http://devbox/v1").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// When `text` is not `http://host[:port]` or `https://host[:port]`.
    pub fn forward(text: &str) -> Result<Server, Invalid> {
        Server::parse(text, true)
    }

    /// Reads `text`, taking plain `http` for any host when `any_http`, and
    /// for a loopback host alone otherwise.
    fn parse(text: &str, any_http: bool) -> Result<Server, Invalid> {
        let bad = |why: &str| Invalid(format!("{text:?} is not a server's address: {why}"));
        let (scheme, rest) = text
            .split_once("://")
            .ok_or_else(|| bad("it is https://host[:port]"))?;
        let scheme = scheme.to_ascii_lowercase();
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['/', '?', '#', '@']) {
            return Err(bad("it may not have a path, a query, a user or a fragment"));
        }
        let host = rest.to_ascii_lowercase();
        let (name, port) = split_port(&host).ok_or_else(|| bad("the port is not one"))?;
        if !host_name(name) {
            return Err(bad("the host is not one"));
        }
        if port.is_some_and(|port| port.is_empty() || port.parse::<u16>().is_err()) {
            return Err(bad("the port is not one"));
        }
        match scheme.as_str() {
            "https" => {}
            "http" if any_http || LOOPBACK.contains(&name) => {}
            "http" => return Err(bad("plain http is only for 127.0.0.1, localhost and [::1]")),
            _ => return Err(bad("the scheme is not https")),
        }
        Ok(Server(format!("{scheme}://{host}")))
    }
}

impl FromStr for Server {
    type Err = Invalid;

    fn from_str(text: &str) -> Result<Server, Invalid> {
        Server::parse(text, false)
    }
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The host and the port of `host`, or `None` when the text after a
/// bracketed address is not a port.
fn split_port(host: &str) -> Option<(&str, Option<&str>)> {
    if host.starts_with('[') {
        let end = host.find(']')? + 1;
        let (name, rest) = host.split_at(end);
        return match rest {
            "" => Some((name, None)),
            _ => Some((name, Some(rest.strip_prefix(':')?))),
        };
    }
    Some(match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    })
}

/// Whether `name` is a host: a name of letters, digits, dots and dashes,
/// or an address in brackets.
fn host_name(name: &str) -> bool {
    if let Some(inner) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        return !inner.is_empty() && inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
    }
    !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A bearer token, checked for its shape: `csu_` or `csa_`, then 36 base62
/// characters (AC-05). The server checks the rest.
///
/// It never shows itself: [`Debug`] and [`Display`](fmt::Display) give only
/// its prefix, so a token held in an error or a log line is not printed.
///
/// ```
/// use cases_store_remote::Token;
///
/// let token: Token = "csu_000000000000000000000000000000000000".parse().unwrap();
/// assert_eq!(token.to_string(), "csu_…");
/// assert_eq!(format!("{token:?}"), "Token(csu_…)");
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Token(String);

/// The characters of a token after its prefix.
const TOKEN_BODY: usize = 36;

impl Token {
    /// The token itself, for what must send or hash it. Never print it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// A token for the store routes of a `cases serve` that a forwarded
    /// store is reached at over HTTP, one of its `store-tokens-file`: at
    /// least 32 characters, each an ASCII letter or
    /// digit or one of `-._~+/=`, as hex and base64 give. It shows no part
    /// of itself, not even a prefix.
    ///
    /// ```
    /// use cases_store_remote::Token;
    ///
    /// let token = Token::forward(&"0f".repeat(32)).unwrap();
    /// assert_eq!(token.to_string(), "…");
    /// assert!(Token::forward("short").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// When `text` is not one. The error never holds the text.
    pub fn forward(text: &str) -> Result<Token, Invalid> {
        if forward_token(text) {
            Ok(Token(text.to_owned()))
        } else {
            Err(Invalid(format!(
                "the token is not one: it is at least {FORWARD_TOKEN_MIN} letters, digits and \
                 -._~+/=, such as openssl rand -hex 32 gives"
            )))
        }
    }

    /// The token's prefix, `csu_` or `csa_`, or nothing for a forward's
    /// token, which has none to show.
    fn prefix(&self) -> &str {
        self.0
            .get(..4)
            .filter(|prefix| matches!(*prefix, "csu_" | "csa_"))
            .unwrap_or_default()
    }
}

/// The fewest characters a forward's token holds (see [`Token::forward`]).
pub(crate) const FORWARD_TOKEN_MIN: usize = 32;

/// Whether `text` has the shape of a forward's token (see
/// [`Token::forward`]).
#[must_use]
pub(crate) fn forward_token(text: &str) -> bool {
    text.len() >= FORWARD_TOKEN_MIN
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
}

impl FromStr for Token {
    type Err = Invalid;

    /// Reads a token. The error never holds the text.
    fn from_str(text: &str) -> Result<Token, Invalid> {
        let body = text
            .strip_prefix("csu_")
            .or_else(|| text.strip_prefix("csa_"));
        match body {
            Some(body) if body.len() == TOKEN_BODY && base62(body) => Ok(Token(text.to_owned())),
            _ => Err(Invalid(
                "the token is not one: it is csu_ and 36 letters and digits".to_owned(),
            )),
        }
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({}…)", self.prefix())
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}…", self.prefix())
    }
}

/// The characters of a store id after `st_`.
const STORE_BODY: usize = 16;

/// Whether `id` is a store's id: `st_`, then 16 base62 characters (AC-01).
#[must_use]
pub fn store_id(id: &str) -> bool {
    id.strip_prefix("st_")
        .is_some_and(|body| body.len() == STORE_BODY && base62(body))
}

/// The most characters an account's id holds after `ac_`.
const ACCOUNT_BODY: usize = 64;

/// Whether `id` is an account's id: `ac_`, then 1 to 64 base62 characters,
/// so it can go in a path as it is.
#[cfg_attr(
    not(feature = "hosted"),
    allow(dead_code, reason = "only a revoke checks an account's id")
)]
pub(crate) fn account_id(id: &str) -> bool {
    id.strip_prefix("ac_")
        .is_some_and(|body| (1..=ACCOUNT_BODY).contains(&body.len()) && base62(body))
}

/// Whether `text` is all base62: ASCII letters and digits.
fn base62(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(text: &str) -> Result<String, Invalid> {
        text.parse::<Server>().map(|s| s.as_str().to_owned())
    }

    #[test]
    fn a_server_is_https_and_a_host() {
        for (text, want) in [
            ("https://cases.sh", "https://cases.sh"),
            ("https://cases.sh/", "https://cases.sh"),
            ("HTTPS://Cases.SH", "https://cases.sh"),
            (
                "Https://staging.cases.sh:8443",
                "https://staging.cases.sh:8443",
            ),
            ("https://[2001:db8::1]:443", "https://[2001:db8::1]:443"),
            ("https://[2001:db8::1]", "https://[2001:db8::1]"),
            ("http://127.0.0.1:8787", "http://127.0.0.1:8787"),
            ("http://localhost", "http://localhost"),
            ("HTTP://LOCALHOST:1", "http://localhost:1"),
            ("http://[::1]:9", "http://[::1]:9"),
            ("https://a-b.c", "https://a-b.c"),
        ] {
            assert_eq!(server(text).as_deref(), Ok(want), "{text}");
        }
        let s: Server = "https://cases.sh:8443".parse().unwrap();
        assert_eq!(s.host(), "cases.sh:8443");
        assert_eq!(s.to_string(), "https://cases.sh:8443");
    }

    #[test]
    fn a_server_is_refused_with_more_than_a_host() {
        for text in [
            "",
            "cases.sh",
            "https:/cases.sh",
            "https://",
            "https:///",
            "https://cases.sh/v1",
            "https://cases.sh//",
            "https://cases.sh?a=b",
            "https://cases.sh#x",
            "https://user@cases.sh",
            "https://cases.sh:",
            "https://cases.sh:x",
            "https://cases.sh:65536",
            "https://ca ses.sh",
            "https://.cases.sh",
            "https://-cases.sh",
            "https://[]",
            "https://[::1",
            "https://[::1]x",
            "https://[g::1]",
            "http://cases.sh",
            "http://127.0.0.2",
            "http://localhost.cases.sh",
            "ftp://cases.sh",
            "file:///etc/passwd",
        ] {
            assert!(server(text).is_err(), "{text:?} was taken");
        }
        let error = server("http://cases.sh").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"http://cases.sh\" is not a server's address: plain http is only for 127.0.0.1, \
             localhost and [::1]"
        );
    }

    #[test]
    fn a_token_is_its_prefix_and_36_base62_characters() {
        let body = "aZ09".repeat(9);
        for prefix in ["csu_", "csa_"] {
            let text = format!("{prefix}{body}");
            let token: Token = text.parse().unwrap();
            assert_eq!(token.expose(), text);
        }
        for text in [
            String::new(),
            body.clone(),
            format!("csx_{body}"),
            format!("CSU_{body}"),
            format!("csu_{}", &body[1..]),
            format!("csu_{body}a"),
            format!("csu_{}-", &body[1..]),
            format!("csu_{}é", &body[2..]),
        ] {
            let error = text.parse::<Token>().unwrap_err();
            assert!(!error.to_string().contains(&body), "{error}");
        }
    }

    #[test]
    fn a_token_never_shows_itself() {
        let text = format!("csa_{}", "Q".repeat(36));
        let token: Token = text.parse().unwrap();
        assert_eq!(token.to_string(), "csa_…");
        assert_eq!(format!("{token:?}"), "Token(csa_…)");
        assert_eq!(format!("{token:#?}"), "Token(csa_…)");
    }

    #[test]
    fn a_forward_is_http_or_https_to_any_host() {
        for (text, want) in [
            (
                "http://devbox.example.ts.net:7338",
                "http://devbox.example.ts.net:7338",
            ),
            ("HTTP://Devbox/", "http://devbox"),
            (
                "https://devbox.example.ts.net",
                "https://devbox.example.ts.net",
            ),
            ("http://100.64.0.1:80", "http://100.64.0.1:80"),
            ("http://[fd7a::1]:7338", "http://[fd7a::1]:7338"),
        ] {
            assert_eq!(
                Server::forward(text)
                    .map(|s| s.as_str().to_owned())
                    .as_deref(),
                Ok(want),
                "{text}"
            );
        }
        for text in [
            "devbox",
            "ssh://devbox",
            "http://devbox/v1",
            "http://devbox?x",
            "http://user@devbox",
            "http://devbox:x",
            "ftp://devbox",
        ] {
            assert!(Server::forward(text).is_err(), "{text:?} was taken");
        }
        // A server's address still takes plain http for loopback alone.
        assert!("http://devbox".parse::<Server>().is_err());
    }

    #[test]
    fn a_forward_s_token_is_long_and_shows_nothing() {
        let hex = "0123456789abcdef".repeat(4);
        for text in [
            hex.as_str(),
            "A-Z_a.z~0+9/=AAAAAAAAAAAAAAAAAAAAAAAA",
            &format!("csu_{}", "a".repeat(36)),
        ] {
            let token = Token::forward(text).unwrap();
            assert_eq!(token.expose(), text);
        }
        let token = Token::forward(&hex).unwrap();
        assert_eq!(token.to_string(), "…");
        assert_eq!(format!("{token:?}"), "Token(…)");
        for text in [
            String::new(),
            "a".repeat(31),
            format!("{} ", "a".repeat(32)),
            format!("{}\n", "a".repeat(32)),
            format!("{}é", "a".repeat(32)),
            format!("{}:", "a".repeat(32)),
        ] {
            let error = Token::forward(&text).unwrap_err();
            assert!(!error.to_string().contains("aaaa"), "{error}");
        }
    }

    #[test]
    fn an_account_id_is_ac_and_base62_characters() {
        for id in [
            "ac_fake",
            "ac_bootstrap0000000",
            "ac_a",
            &format!("ac_{}", "Z".repeat(64)),
        ] {
            assert!(account_id(id), "{id:?}");
        }
        for id in [
            "",
            "ac_",
            "ac",
            "AC_fake",
            "st_fake",
            "ac_fa/ke",
            "ac_../x",
            "ac_fake?x=1",
            &format!("ac_{}", "Z".repeat(65)),
        ] {
            assert!(!account_id(id), "{id:?}");
        }
    }

    #[test]
    fn a_store_id_is_st_and_16_base62_characters() {
        assert!(store_id("st_4kQ9mZr2pX7wLc0t"));
        for id in [
            "",
            "st_",
            "st_4kQ9mZr2pX7wLc0",
            "st_4kQ9mZr2pX7wLc0tt",
            "ST_4kQ9mZr2pX7wLc0t",
            "xx_4kQ9mZr2pX7wLc0t",
            "st_4kQ9mZr2pX7wLc0/",
            "st_4kQ9mZr2pX7wL../",
        ] {
            assert!(!store_id(id), "{id:?}");
        }
    }
}
