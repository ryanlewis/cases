//! The inbox's routes (spec/api.md), the checks every request passes first
//! (IN-11), and the request log (SV-02).

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use cases_store_sqlite::{Db, Writer};
use serde::Serialize;
use tokio::sync::watch;

use super::api;
use super::favicon;
use super::http::Timeouts;
use super::log::Log;
use super::page;
use super::stream;
use super::watch::Watch;
use crate::dial;
use crate::output;
use crate::tokens::Tokens;

/// The header every write carries its CSRF token in.
pub(crate) const CSRF_HEADER: &str = "x-cases-csrf";

/// The header `tailscale serve` names the request's Tailscale user in. A
/// tagged device sends none.
const LOGIN_HEADER: &str = "tailscale-user-login";

/// The header `tailscale funnel` sets on a request from the internet, which
/// serve always refuses: the inbox is never public.
const FUNNEL_HEADER: &str = "tailscale-funnel-request";

/// The header `tailscale serve` sets on every request it passes on, over
/// any the client sent.
const FORWARDED_HOST_HEADER: &str = "x-forwarded-host";

/// The path every store route is under: the routes a forwarded store is
/// reached by over HTTP (`forward = "http://…"`), as over a pipe.
const STORE_ROUTES: &str = "/v1/stores/";

/// What every request is served with.
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) watch: Watch,
    pub(crate) db: Arc<Db>,
    /// Who the human is, for the answers, parks, resumes, withdraws,
    /// messages and edits the inbox writes (HU-06).
    pub(crate) writer: Writer,
    /// The `Host` values a request may carry, in lowercase: the listen
    /// address and its `localhost` spelling.
    pub(crate) hosts: Vec<String>,
    /// The config file's `allow-hosts`: other `Host` values, in lowercase,
    /// of a proxy that serves the inbox over HTTPS, such as `tailscale
    /// serve`.
    pub(crate) proxied: Vec<String>,
    /// The config file's `tailscale-logins`: when there are any, a request
    /// through a proxied host must carry one of them in
    /// `Tailscale-User-Login`.
    pub(crate) logins: Vec<String>,
    /// The tokens of the config file's `store-tokens-file`: a store route
    /// through a proxy is let in with one of them, or a login of
    /// `logins`.
    pub(crate) store_tokens: Tokens,
    /// The pickup waits' locks taken on the store routes, by id.
    pub(crate) leases: dial::Leases,
    /// The token every write carries, new each time serve starts.
    pub(crate) csrf: String,
    /// The inbox's address, such as `http://127.0.0.1:7337/`.
    pub(crate) url: String,
    /// The inbox's address in the links it gives, such as a case's `url`
    /// (`instance::link`).
    pub(crate) link: String,
    pub(crate) log: Arc<Log>,
    /// Set when serve stops: the event streams end, and new ones end at
    /// once.
    pub(crate) stopping: watch::Receiver<bool>,
    /// How many requests were logged since serve started.
    pub(crate) requests: AtomicU64,
    pub(crate) timeouts: Timeouts,
}

/// The `Host` values a request to a server listening on `host`, as the
/// listen address gives it, and `port` may carry: the host in its canonical
/// spelling, as a browser sends it, and `localhost`, with the port, and
/// without it when it is 80, which browsers leave out.
pub(crate) fn hosts(host: &str, port: u16) -> Vec<String> {
    let mut names = vec![host.to_ascii_lowercase(), "localhost".to_owned()];
    if let Ok(ip) = host.parse::<IpAddr>() {
        names[0] = ip.to_string();
    }
    if names[0] == "localhost" {
        names.splice(1.., ["127.0.0.1".to_owned(), "::1".to_owned()]);
    }
    let mut hosts = Vec::new();
    for name in names {
        let name = if name.contains(':') {
            format!("[{name}]")
        } else {
            name
        };
        hosts.push(format!("{name}:{port}"));
        if port == 80 {
            hosts.push(name);
        }
    }
    hosts
}

/// The inbox's routes, behind the request guard and the log.
pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page::index))
        .route("/done", get(page::index))
        .route("/cases/{id}", get(page::index))
        .route("/message", get(page::index))
        .route("/static/{*path}", get(page::static_file))
        .route("/v1", get(api::index))
        .route("/v1/cases", get(api::list).post(api::send))
        .route("/v1/cases/{id}", get(api::case))
        .route("/v1/cases/{id}/{write}", post(api::write))
        .route("/v1/changes", get(stream::changes))
        .route("/v1/listeners", get(api::listeners))
        .route("/v1/notifications", get(api::notifications))
        .route("/favicon.svg", get(favicon::icon))
        .route("/v1/stores/{*route}", any(store))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn_with_state(Arc::clone(&app), guard))
        .layer(middleware::from_fn_with_state(Arc::clone(&app), log))
        .with_state(app)
}

/// A store route, `/v1/stores/{id}/store/…`, answered as `cases dial-stdio`
/// answers it over a pipe (see [`dial::handle_http`]), on a blocking
/// thread, as every read and write of the store is: a held poll waits
/// there.
async fn store(State(app): State<Arc<App>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = axum::body::to_bytes(body, cases_store_remote::REQUEST_LIMIT).await else {
        return Refusal::bad("the body is over 1 MiB, or did not arrive").into_response();
    };
    let header = |name: header::HeaderName| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let request = dial::Request::new(
        parts.method.as_str(),
        parts.uri.path(),
        parts.uri.query().unwrap_or_default(),
        header(header::CONTENT_TYPE).as_deref(),
        header(header::USER_AGENT).as_deref(),
        body.to_vec(),
    );
    let answered = tokio::task::spawn_blocking(move || {
        let answer = dial::handle_http(&app.db, &request, &app.leases);
        (answer.status(), answer.body().to_string().into_bytes())
    })
    .await;
    match answered {
        Ok((status, body)) => json(
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        ),
        Err(error) => Refusal::store(error).into_response(),
    }
}

/// Any path the inbox does not have.
async fn not_found() -> Refusal {
    Refusal::request(StatusCode::NOT_FOUND, "no such path")
}

async fn method_not_allowed() -> Refusal {
    Refusal::request(
        StatusCode::METHOD_NOT_ALLOWED,
        "the path does not take this method",
    )
}

/// Refuses a request that could come from another site, and sets the
/// security headers on every response. A request must name this server as
/// its `Host`, reads included, so that a name rebound to loopback cannot
/// read the API, or one of `allow-hosts`, a proxy's, whose requests must
/// come from one of `tailscale-logins` when it is set, as must any request
/// the proxy passes on with another Host. A request `tailscale funnel`
/// passed on, from the internet, is refused whatever it carries. A write,
/// any method but GET and HEAD, is refused when `Sec-Fetch-Site` or `Origin` says it
/// came from elsewhere, or when it does not carry the CSRF token. The
/// origin of a proxied host is `https://`, as the proxy serves it over TLS.
async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let checked = if request.uri().path().starts_with(STORE_ROUTES) {
        check_store(&app, request.headers())
    } else {
        check(&app, request.method(), request.headers())
    };
    let mut response = match checked {
        Ok(()) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    };
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_SECURITY_POLICY, page::CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "same-origin"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn check(app: &App, method: &Method, headers: &HeaderMap) -> Result<(), Refusal> {
    // `tailscale funnel` serves a port to the internet, and marks each
    // request it passes on. The inbox is for the tailnet at most.
    if headers.contains_key(FUNNEL_HEADER) {
        return Err(Refusal::request(
            StatusCode::FORBIDDEN,
            "the inbox is not served through tailscale funnel, only to this machine and the \
             tailnet",
        ));
    }
    let host = text(headers, &header::HOST).to_ascii_lowercase();
    let proxied = app.proxied.contains(&host);
    // A request came through the proxy when its Host is the proxy's, or when
    // it carries the X-Forwarded-Host that `tailscale serve` always sets: the
    // proxy picks its site by the TLS server name and passes any Host on, the
    // loopback one included. The login header is only as good as serve's
    // listening on loopback alone: anything that reaches the port can send it.
    if !app.logins.is_empty() && (proxied || headers.contains_key(FORWARDED_HOST_HEADER)) {
        let login = text(headers, &HeaderName::from_static(LOGIN_HEADER));
        if !app.logins.iter().any(|allowed| allowed == login) {
            return Err(Refusal::request(
                StatusCode::FORBIDDEN,
                "the request is not from one of the config file's tailscale-logins",
            ));
        }
    }
    let scheme = if app.hosts.contains(&host) {
        "http"
    } else if proxied {
        "https"
    } else {
        return Err(Refusal::request(
            StatusCode::FORBIDDEN,
            "the Host is not this server; a proxy's host, such as tailscale serve's, must be \
             in the config file's allow-hosts",
        ));
    };
    if method == Method::GET || method == Method::HEAD {
        return Ok(());
    }
    let site = text(headers, &HeaderName::from_static("sec-fetch-site"));
    if !site.is_empty() && site != "same-origin" && site != "none" {
        return Err(Refusal::request(
            StatusCode::FORBIDDEN,
            "a write from another site",
        ));
    }
    if let Some(origin) = headers.get(header::ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|origin| origin.eq_ignore_ascii_case(&format!("{scheme}://{host}")))
    {
        return Err(Refusal::request(
            StatusCode::FORBIDDEN,
            "a write from another origin",
        ));
    }
    let token = headers
        .get(CSRF_HEADER)
        .map_or(&[][..], HeaderValue::as_bytes);
    if !same(token, app.csrf.as_bytes()) {
        return Err(Refusal {
            status: StatusCode::FORBIDDEN,
            category: "csrf",
            message: "a write needs the token GET /v1 gives; read it again and send once more"
                .into(),
            case: None,
        });
    }
    Ok(())
}

/// The check of a store route, in place of the inbox's: they are for a
/// forwarding cases, never a browser, and carry no CSRF token. A request
/// `tailscale funnel` passed on is refused, as is one a browser sent,
/// which says so in `Origin` or `Sec-Fetch-Site`, so that no page can
/// reach them. One straight to the loopback address, by a Host of this
/// server's and with no `X-Forwarded-Host`, is let in, as the inbox is: it
/// is from this machine. One through a proxy, such as `tailscale serve
/// --http=PORT`, which sets `X-Forwarded-Host`, or by a Host of
/// `allow-hosts`, must carry one of `store-tokens-file`'s tokens as its
/// bearer, or come from one of `tailscale-logins` by
/// `Tailscale-User-Login`: a tagged device has no login, so a token is its
/// way in. Plain `http` through the proxy is let in, unlike the inbox's
/// writes: those are https only for the browser's origin check, and a
/// store route has no browser, takes no cookie, and must name who it is
/// from. Any other Host is refused.
///
/// Each refusal is 401 `auth`, which the forwarding cases shows as the
/// far side's refusal, with the message.
fn check_store(app: &App, headers: &HeaderMap) -> Result<(), Refusal> {
    let refuse = |message: &str| {
        Err(Refusal {
            status: StatusCode::UNAUTHORIZED,
            category: "auth",
            message: message.to_owned(),
            case: None,
        })
    };
    if headers.contains_key(FUNNEL_HEADER) {
        return refuse(
            "the store is not served through tailscale funnel, only to this machine and the \
             tailnet",
        );
    }
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return refuse("the store routes are for a forwarding cases, not a browser");
    }
    let host = text(headers, &header::HOST).to_ascii_lowercase();
    let through_proxy = headers.contains_key(FORWARDED_HOST_HEADER);
    if app.hosts.contains(&host) && !through_proxy {
        return Ok(());
    }
    if !through_proxy && !app.proxied.contains(&host) {
        return refuse(
            "the Host is not this server; a proxy's host must be in the config file's \
             allow-hosts, or the proxy must set X-Forwarded-Host, as tailscale serve does",
        );
    }
    if let Some(bearer) = headers.get(header::AUTHORIZATION) {
        let sent = bearer
            .as_bytes()
            .strip_prefix(b"Bearer ")
            .unwrap_or_default();
        if !sent.is_empty() && app.store_tokens.has(sent) {
            return Ok(());
        }
        return refuse("the token is not one of the store-tokens-file of the cases serve there");
    }
    let login = text(headers, &HeaderName::from_static(LOGIN_HEADER));
    if !login.is_empty() && app.logins.iter().any(|allowed| allowed == login) {
        return Ok(());
    }
    refuse(
        "a store route through a proxy needs a token of the store-tokens-file of the cases \
         serve there (forward-token-file here), or a Tailscale login in its tailscale-logins",
    )
}

/// The value of the header `name`, or empty when it is missing or is not
/// text.
fn text<'a>(headers: &'a HeaderMap, name: &HeaderName) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

/// Whether `a` and `b` are the same, in a time that depends only on their
/// lengths.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Logs one line for each page load, write and failed request, with the
/// cause of a 500 on the line before it. The requests a page makes by
/// itself and that succeed, its event stream, API reads, notification
/// checks, icons and scripts, are left out: an open tab would bury
/// everything else.
///
/// The request runs on a task of its own, which the page going away does not
/// cancel. hyper drops a request whose page closes the connection, so a
/// write the page sent in full would otherwise be lost while it waits for
/// the store, or be made and never logged (IN-03, SV-02).
async fn log(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let handled = tokio::spawn(async move {
        let start = Instant::now();
        // The line is stamped with when the request came, as the log is read
        // in that order.
        let at = output::now()
            .map(|now| output::stamp(Some(now)))
            .unwrap_or_default();
        let method = request.method().clone();
        let uri = request.uri().clone();
        let response = next.run(request).await;
        let status = response.status();
        let read = method == Method::GET || method == Method::HEAD;
        // A pickup wait renews its locks every 10 s: one that is renewed is
        // left out, as a read is.
        let renewed = status.is_success() && is_renewal(uri.path());
        if !(read || renewed)
            || status.is_client_error()
            || status.is_server_error()
            || is_page(uri.path())
        {
            if let Some(Failure(error)) = response.extensions().get::<Failure>() {
                app.log.line(format!("error: {error}"));
            }
            app.requests.fetch_add(1, Ordering::Relaxed);
            app.log.line(format!(
                "{at} {method} {uri} {} {}ms",
                status.as_u16(),
                start.elapsed().as_millis()
            ));
        }
        response
    });
    // A request whose handler panicked.
    handled
        .await
        .unwrap_or_else(|error| Refusal::store(error).into_response())
}

/// Whether `path` renews a pickup wait's locks on the store routes.
fn is_renewal(path: &str) -> bool {
    path.starts_with(STORE_ROUTES) && path.contains("/store/lock/") && path.ends_with("/renew")
}

/// Whether `path` is one of the inbox's pages, which the SPA draws.
pub(crate) fn is_page(path: &str) -> bool {
    path == "/" || path == "/done" || path == "/message" || path.starts_with("/cases/")
}

/// A request the API refused, and why: the error body of spec/api.md.
#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) status: StatusCode,
    /// The core's category for a refused write, `csrf`, `request` for a
    /// request the API refuses itself, or `store` when the store failed.
    pub(crate) category: &'static str,
    pub(crate) message: String,
    /// The case as it is now, on a 409 or a 422.
    pub(crate) case: Option<serde_json::Value>,
}

impl Refusal {
    /// A request the API refuses itself.
    pub(crate) fn request(status: StatusCode, message: impl Into<String>) -> Refusal {
        Refusal {
            status,
            category: "request",
            message: message.into(),
            case: None,
        }
    }

    /// A request body or query the route does not take.
    pub(crate) fn bad(message: impl Into<String>) -> Refusal {
        Refusal::request(StatusCode::BAD_REQUEST, message)
    }

    /// No case with the id asked for.
    pub(crate) fn no_case() -> Refusal {
        Refusal::request(StatusCode::NOT_FOUND, "no such case")
    }

    /// The store failed.
    pub(crate) fn store(error: impl std::fmt::Display) -> Refusal {
        Refusal {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            category: "store",
            message: error.to_string(),
            case: None,
        }
    }
}

/// Why a request failed with a 500, for the log: the page reads it in the
/// body, and the log line would say only the status (SV-02).
#[derive(Clone, Debug)]
struct Failure(String);

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body<'a> {
            error: Error<'a>,
        }
        #[derive(Serialize)]
        struct Error<'a> {
            category: &'a str,
            message: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            case: Option<&'a serde_json::Value>,
        }
        let body = Body {
            error: Error {
                category: self.category,
                message: &self.message,
                case: self.case.as_ref(),
            },
        };
        let mut response = match serde_json::to_vec(&body) {
            Ok(body) => json(self.status, body),
            Err(_) => self.status.into_response(),
        };
        if self.status.is_server_error() {
            response.extensions_mut().insert(Failure(self.message));
        }
        response
    }
}

/// A JSON response of `body`. API responses are never cached.
pub(crate) fn json(status: StatusCode, mut body: Vec<u8>) -> Response {
    body.push(b'\n');
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

/// `value` as a JSON response.
pub(crate) fn ok(value: &impl Serialize) -> Result<Response, Refusal> {
    let body = serde_json::to_vec(value).map_err(Refusal::store)?;
    Ok(json(StatusCode::OK, body))
}

/// Runs `f` on a blocking thread, as every read and write of the store is.
/// The thread runs to the end even if the request goes away first, so a
/// write that has begun is made (IN-03).
pub(crate) async fn blocking<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&App) -> Result<T, Refusal> + Send + 'static,
) -> Result<T, Refusal> {
    let app = Arc::clone(app);
    tokio::task::spawn_blocking(move || f(&app))
        .await
        .map_err(Refusal::store)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_the_listen_address_or_localhost() {
        assert_eq!(
            hosts("127.0.0.1", 7337),
            ["127.0.0.1:7337", "localhost:7337"]
        );
        assert_eq!(
            hosts("127.0.0.1", 80),
            ["127.0.0.1:80", "127.0.0.1", "localhost:80", "localhost"]
        );
        assert_eq!(
            hosts("::1", 80),
            ["[::1]:80", "[::1]", "localhost:80", "localhost"]
        );
        assert_eq!(
            hosts("0:0:0:0:0:0:0:1", 7337),
            ["[::1]:7337", "localhost:7337"]
        );
        assert_eq!(
            hosts("LocalHost", 7337),
            ["localhost:7337", "127.0.0.1:7337", "[::1]:7337"]
        );
    }

    const TAILNET: &str = "devbox.example.ts.net:7337";

    /// A server on 127.0.0.1:7337 with `allow-hosts` and `tailscale-logins`
    /// set to `proxied` and `logins`, over a store it never opens.
    fn app(dir: &tempfile::TempDir, proxied: &[&str], logins: &[&str]) -> App {
        let db = Arc::new(Db::new(dir.path().join("cases.db")));
        let log = Arc::new(Log::new(false, 0));
        let url = "http://127.0.0.1:7337/".to_owned();
        let (_, stopping) = watch::channel(false);
        App {
            watch: Watch::new(
                Arc::clone(&db),
                "boot".into(),
                url.clone(),
                Arc::clone(&log),
            )
            .unwrap(),
            db,
            writer: Writer::new(None),
            hosts: hosts("127.0.0.1", 7337),
            proxied: proxied.iter().map(|&host| host.to_owned()).collect(),
            logins: logins.iter().map(|&login| login.to_owned()).collect(),
            store_tokens: Tokens::default(),
            leases: dial::Leases::new(dial::LEASE),
            csrf: "token".into(),
            link: url.clone(),
            url,
            log,
            stopping,
            requests: AtomicU64::new(0),
            timeouts: Timeouts::SERVE,
        }
    }

    /// Headers of `name: value` pairs.
    fn with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for &(name, value) in pairs {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    /// The message of the refusal `check` gives, or `None` when it lets the
    /// request through.
    fn refused(app: &App, method: &Method, pairs: &[(&str, &str)]) -> Option<String> {
        check(app, method, &with(pairs)).err().map(|refusal| {
            assert_eq!(refusal.status, StatusCode::FORBIDDEN);
            refusal.message
        })
    }

    /// With neither setting, a proxy's host is refused, and the refusal
    /// names the setting that would let it in.
    #[test]
    fn a_proxied_host_is_refused_unless_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir, &[], &[]);
        let message = refused(&app, &Method::GET, &[("host", TAILNET)]).unwrap();
        assert!(message.contains("allow-hosts"), "{message}");
        assert_eq!(
            refused(&app, &Method::GET, &[("host", "127.0.0.1:7337")]),
            None
        );
    }

    /// A read through an allowed host passes, and through any other is
    /// refused, the port included.
    #[test]
    fn a_read_through_an_allowed_host_passes() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir, &[TAILNET], &[]);
        for host in [TAILNET, "Devbox.Example.TS.net:7337"] {
            assert_eq!(
                refused(&app, &Method::GET, &[("host", host)]),
                None,
                "{host}"
            );
        }
        for host in [
            "devbox.example.ts.net",
            "devbox.example.ts.net:443",
            "evil.example:7337",
            "",
        ] {
            assert!(
                refused(&app, &Method::GET, &[("host", host)]).is_some(),
                "{host}"
            );
        }
    }

    /// A write through an allowed host must come from its https origin and
    /// carry the token; one from its http origin, or another, is refused.
    #[test]
    fn a_write_through_an_allowed_host_comes_from_https() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir, &[TAILNET], &[]);
        let write = |origin: &str, token: &str| {
            let mut pairs = vec![("host", TAILNET), ("sec-fetch-site", "same-origin")];
            if !origin.is_empty() {
                pairs.push(("origin", origin));
            }
            if !token.is_empty() {
                pairs.push((CSRF_HEADER, token));
            }
            refused(&app, &Method::POST, &pairs)
        };
        let https = format!("https://{TAILNET}");
        assert_eq!(write(&https, "token"), None);
        assert_eq!(write("", "token"), None);
        for origin in [
            format!("http://{TAILNET}"),
            "https://devbox.example.ts.net".to_owned(),
            "https://evil.example".to_owned(),
            "http://127.0.0.1:7337".to_owned(),
            "null".to_owned(),
        ] {
            let message = write(&origin, "token").unwrap();
            assert!(message.contains("another origin"), "{origin}: {message}");
        }
        assert!(write(&https, "").is_some());
        assert!(write(&https, "wrong").is_some());
        // A write from another site is refused whatever its origin.
        assert!(
            refused(
                &app,
                &Method::POST,
                &[
                    ("host", TAILNET),
                    ("sec-fetch-site", "cross-site"),
                    ("origin", &https),
                    (CSRF_HEADER, "token"),
                ],
            )
            .is_some()
        );
        // The loopback host keeps its http origin.
        let local = |origin: &str| {
            refused(
                &app,
                &Method::POST,
                &[
                    ("host", "127.0.0.1:7337"),
                    ("origin", origin),
                    (CSRF_HEADER, "token"),
                ],
            )
        };
        assert_eq!(local("http://127.0.0.1:7337"), None);
        assert!(local("https://127.0.0.1:7337").is_some());
    }

    /// With tailscale-logins set, a request through an allowed host must
    /// name one of them in Tailscale-User-Login; one to the loopback host
    /// needs none.
    #[test]
    fn a_proxied_request_must_come_from_a_named_login() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(&dir, &[TAILNET], &["ryan@github", "sam@github"]);
        let read = |login: Option<&str>| {
            let mut pairs = vec![("host", TAILNET)];
            if let Some(login) = login {
                pairs.push(("tailscale-user-login", login));
            }
            refused(&app, &Method::GET, &pairs)
        };
        for login in [None, Some(""), Some("eve@github"), Some("RYAN@github")] {
            let message = read(login).unwrap();
            assert!(message.contains("tailscale-logins"), "{login:?}: {message}");
        }
        assert_eq!(read(Some("ryan@github")), None);
        assert_eq!(read(Some("sam@github")), None);
        let https = format!("https://{TAILNET}");
        let write = |login: &str| {
            refused(
                &app,
                &Method::POST,
                &[
                    ("host", TAILNET),
                    ("origin", &https),
                    ("tailscale-user-login", login),
                    (CSRF_HEADER, "token"),
                ],
            )
        };
        assert_eq!(write("ryan@github"), None);
        assert!(write("eve@github").is_some());
        assert_eq!(
            refused(&app, &Method::GET, &[("host", "127.0.0.1:7337")]),
            None
        );
        assert_eq!(
            refused(
                &app,
                &Method::POST,
                &[("host", "localhost:7337"), (CSRF_HEADER, "token")],
            ),
            None
        );
    }

    /// `tailscale serve` picks its site by the TLS server name and passes the
    /// client's Host on as it is, so a client on the tailnet can send the
    /// loopback Host through it. With tailscale-logins set, such a request,
    /// marked by the X-Forwarded-Host the proxy always sets, still needs a
    /// named login.
    #[test]
    fn a_proxied_request_with_a_loopback_host_needs_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let guarded = app(&dir, &[TAILNET], &["ryan@github"]);
        let local = "127.0.0.1:7337";
        for pairs in [
            &[("host", local), ("x-forwarded-host", local)][..],
            &[
                ("host", local),
                ("x-forwarded-host", local),
                ("tailscale-user-login", "eve@github"),
            ],
        ] {
            let message = refused(&guarded, &Method::GET, pairs).unwrap();
            assert!(message.contains("tailscale-logins"), "{pairs:?}: {message}");
        }
        let write = [
            ("host", local),
            ("x-forwarded-host", local),
            (CSRF_HEADER, "token"),
        ];
        assert!(refused(&guarded, &Method::POST, &write).is_some());
        assert_eq!(
            refused(
                &guarded,
                &Method::GET,
                &[
                    ("host", local),
                    ("x-forwarded-host", local),
                    ("tailscale-user-login", "ryan@github"),
                ],
            ),
            None
        );
        // Without tailscale-logins, a proxied request is let in as before.
        let open = app(&dir, &[TAILNET], &[]);
        assert_eq!(
            refused(
                &open,
                &Method::GET,
                &[("host", local), ("x-forwarded-host", local)]
            ),
            None
        );
    }

    /// A request `tailscale funnel` passed on is refused, through an allowed
    /// host from a named login, or to the loopback address, as is a write
    /// that carries the token.
    #[test]
    fn a_request_through_tailscale_funnel_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let local = "127.0.0.1:7337";
        for app in [
            app(&dir, &[], &[]),
            app(&dir, &[TAILNET], &[]),
            app(&dir, &[TAILNET], &["ryan@github"]),
        ] {
            for host in [local, TAILNET] {
                let pairs = [
                    ("host", host),
                    ("tailscale-funnel-request", "?1"),
                    ("tailscale-user-login", "ryan@github"),
                ];
                let message = refused(&app, &Method::GET, &pairs).unwrap();
                assert!(message.contains("funnel"), "{host}: {message}");
                let write = [
                    ("host", host),
                    ("tailscale-funnel-request", "?1"),
                    (CSRF_HEADER, "token"),
                ];
                assert!(refused(&app, &Method::POST, &write).is_some(), "{host}");
            }
            assert_eq!(refused(&app, &Method::GET, &[("host", local)]), None);
        }
    }

    /// A store token, as `store-tokens-file` holds one.
    const STORE_TOKEN: &str = "0123456789abcdef0123456789abcdef";

    /// The app of [`app`], taking [`STORE_TOKEN`] on its store routes.
    fn store_app(dir: &tempfile::TempDir, proxied: &[&str], logins: &[&str]) -> App {
        let token = cases_store_remote::Token::forward(STORE_TOKEN).unwrap();
        App {
            store_tokens: Tokens::new(vec![token]),
            ..app(dir, proxied, logins)
        }
    }

    /// The message of the refusal `check_store` gives, always 401 `auth`,
    /// or `None` when it lets the request through.
    fn store_refused(app: &App, pairs: &[(&str, &str)]) -> Option<String> {
        check_store(app, &with(pairs)).err().map(|refusal| {
            assert_eq!(refusal.status, StatusCode::UNAUTHORIZED);
            assert_eq!(refusal.category, "auth");
            assert_eq!(refusal.case, None);
            refusal.message
        })
    }

    /// A store route straight to the loopback address needs nothing, by
    /// any of the server's Hosts in any case; through a proxy's
    /// X-Forwarded-Host it needs a token or a login like any other.
    #[test]
    fn a_store_route_to_loopback_needs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let app = store_app(&dir, &[], &[]);
        for host in ["127.0.0.1:7337", "localhost:7337", "LocalHost:7337"] {
            assert_eq!(store_refused(&app, &[("host", host)]), None, "{host}");
        }
        let message = store_refused(
            &app,
            &[("host", "127.0.0.1:7337"), ("x-forwarded-host", TAILNET)],
        )
        .unwrap();
        assert!(message.contains("needs a token"), "{message}");
        assert_eq!(
            store_refused(
                &app,
                &[
                    ("host", "127.0.0.1:7337"),
                    ("x-forwarded-host", TAILNET),
                    ("authorization", &format!("Bearer {STORE_TOKEN}")),
                ],
            ),
            None
        );
    }

    /// A Host that is neither the server's nor in allow-hosts, with no
    /// X-Forwarded-Host, is refused whatever it carries; with
    /// X-Forwarded-Host, any Host goes on to the token and login checks.
    #[test]
    fn a_store_route_by_another_host_is_refused_unless_proxied() {
        let dir = tempfile::tempdir().unwrap();
        let app = store_app(&dir, &[TAILNET], &["ryan@github"]);
        let bearer = format!("Bearer {STORE_TOKEN}");
        for host in [
            "evil.example:7337",
            "devbox.example.ts.net",
            "127.0.0.1",
            "",
        ] {
            let message = store_refused(
                &app,
                &[
                    ("host", host),
                    ("authorization", &bearer),
                    ("tailscale-user-login", "ryan@github"),
                ],
            )
            .unwrap();
            assert!(message.contains("allow-hosts"), "{host}: {message}");
            assert_eq!(
                store_refused(
                    &app,
                    &[
                        ("host", host),
                        ("x-forwarded-host", host),
                        ("authorization", &bearer)
                    ],
                ),
                None,
                "{host}"
            );
        }
        // An allowed host, in any case, goes on to the checks without
        // X-Forwarded-Host.
        for host in [TAILNET, "DevBox.Example.TS.net:7337"] {
            assert_eq!(
                store_refused(&app, &[("host", host), ("authorization", &bearer)]),
                None,
                "{host}"
            );
            let message = store_refused(&app, &[("host", host)]).unwrap();
            assert!(message.contains("needs a token"), "{host}: {message}");
        }
    }

    /// Through a proxy, the bearer must be one of the tokens, whole, after
    /// `Bearer ` as spelt. An Authorization header that is not one is
    /// refused even with a listed login.
    #[test]
    fn a_proxied_store_route_lets_in_only_a_whole_token() {
        let dir = tempfile::tempdir().unwrap();
        let app = store_app(&dir, &[TAILNET], &["ryan@github"]);
        let route = |authorization: &str, login: Option<&str>| {
            let mut pairs = vec![("host", TAILNET), ("authorization", authorization)];
            if let Some(login) = login {
                pairs.push(("tailscale-user-login", login));
            }
            store_refused(&app, &pairs)
        };
        assert_eq!(route(&format!("Bearer {STORE_TOKEN}"), None), None);
        let wrong = "f".repeat(32);
        for authorization in [
            format!("Bearer {wrong}"),
            format!("Bearer {}", &STORE_TOKEN[1..]),
            format!("Bearer {STORE_TOKEN}x"),
            // Pins current behaviour, possibly a bug: RFC 7235 makes the
            // scheme case-insensitive, but only `Bearer ` is taken.
            format!("bearer {STORE_TOKEN}"),
            format!("Bearer  {STORE_TOKEN}"),
            STORE_TOKEN.to_owned(),
            "Bearer ".to_owned(),
            "Bearer".to_owned(),
            String::new(),
        ] {
            for login in [None, Some("ryan@github")] {
                let message = route(&authorization, login).unwrap();
                assert!(
                    message.contains("the token is not one"),
                    "{authorization:?} {login:?}: {message}"
                );
            }
        }
    }

    /// Through a proxy with no Authorization, the request must name one of
    /// tailscale-logins, exactly; with none set, nothing gets in.
    #[test]
    fn a_proxied_store_route_lets_in_a_listed_login() {
        let dir = tempfile::tempdir().unwrap();
        let listed = store_app(&dir, &[TAILNET], &["ryan@github", "sam@github"]);
        let route = |login: Option<&str>| {
            let mut pairs = vec![("host", TAILNET)];
            if let Some(login) = login {
                pairs.push(("tailscale-user-login", login));
            }
            store_refused(&listed, &pairs)
        };
        assert_eq!(route(Some("ryan@github")), None);
        assert_eq!(route(Some("sam@github")), None);
        for login in [None, Some(""), Some("eve@github"), Some("RYAN@github")] {
            let message = route(login).unwrap();
            assert!(message.contains("tailscale-logins"), "{login:?}: {message}");
        }
        // An empty login is never one, even when the list holds one.
        let blank = store_app(&dir, &[TAILNET], &[""]);
        for pairs in [
            &[("host", TAILNET)][..],
            &[("host", TAILNET), ("tailscale-user-login", "")],
        ] {
            assert!(store_refused(&blank, pairs).is_some(), "{pairs:?}");
        }
        // With no tokens and no logins, a proxied store route is refused.
        let closed = app(&dir, &[TAILNET], &[]);
        assert!(store_refused(&closed, &[("host", TAILNET)]).is_some());
        assert!(
            store_refused(
                &closed,
                &[
                    ("host", TAILNET),
                    ("authorization", &format!("Bearer {STORE_TOKEN}"))
                ],
            )
            .is_some()
        );
    }

    /// A store route `tailscale funnel` passed on, or a browser sent, is
    /// refused even to the loopback address with a token and a login.
    #[test]
    fn a_store_route_from_funnel_or_a_browser_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let app = store_app(&dir, &[TAILNET], &["ryan@github"]);
        let bearer = format!("Bearer {STORE_TOKEN}");
        for host in ["127.0.0.1:7337", TAILNET] {
            let pass = [
                ("host", host),
                ("authorization", bearer.as_str()),
                ("tailscale-user-login", "ryan@github"),
            ];
            assert_eq!(store_refused(&app, &pass), None, "{host}");
            for (name, value, says) in [
                ("tailscale-funnel-request", "?1", "funnel"),
                ("origin", "http://127.0.0.1:7337", "not a browser"),
                ("sec-fetch-site", "same-origin", "not a browser"),
                ("sec-fetch-site", "none", "not a browser"),
            ] {
                let mut pairs = pass.to_vec();
                pairs.push((name, value));
                let message = store_refused(&app, &pairs).unwrap();
                assert!(message.contains(says), "{host} {name}: {message}");
            }
        }
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"ab", b"abc"));
        assert!(!same(b"", b"abc"));
    }

    /// The status and the JSON body of `method path` with `body` through
    /// the inbox's router, to the loopback address, as a forwarding cases
    /// sends it.
    async fn through(
        app: &Arc<App>,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, serde_json::Value) {
        use hyper::service::Service as _;
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7337")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_owned()))
            .unwrap();
        let service = hyper_util::service::TowerToHyperService::new(router(Arc::clone(app)));
        let response = service.call(request).await.unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{status}: {error}: {bytes:?}"));
        (status, body)
    }

    /// A lock renewal that went well is left out of the log, as a read is;
    /// one refused 409 `lapsed`, as a hold the far side no longer has is,
    /// is logged.
    #[tokio::test]
    async fn only_a_renewal_that_went_well_is_left_out_of_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(App {
            log: Arc::new(Log::new(false, 16)),
            ..app(&dir, &[], &[])
        });
        let lock = format!("/v1/stores/{}/store/lock", cases_store_remote::DIAL_STORE);
        let (status, taken) = through(
            &app,
            "POST",
            &lock,
            r#"{"locks": [{"on": "label", "text": "deploy"}], "lease": true, "release": true}"#,
        )
        .await;
        assert_eq!(status, 200, "{taken}");
        let hold = taken["hold"].as_str().unwrap();
        let (status, renewed) = through(&app, "POST", &format!("{lock}/{hold}/renew"), "").await;
        assert_eq!(status, 200, "{renewed}");
        let (status, lapsed) = through(&app, "POST", &format!("{lock}/0000/renew"), "").await;
        assert_eq!(status, 409, "{lapsed}");
        assert_eq!(lapsed["error"]["category"], "lapsed", "{lapsed}");
        let lines = app.log.last();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains(&format!("POST {lock} 200 ")), "{lines:?}");
        assert!(
            lines[1].contains(&format!("POST {lock}/0000/renew 409 ")),
            "{lines:?}"
        );
        assert_eq!(app.requests.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_pages_are_the_list_its_old_done_address_and_a_case() {
        for path in ["/", "/done", "/message", "/cases/UkLWZ"] {
            assert!(is_page(path), "{path}");
        }
        for path in ["/v1/cases", "/static/style.css", "/favicon.svg", "/donex"] {
            assert!(!is_page(path), "{path}");
        }
    }
}
