//! The inbox's page (IN-01, IN-02, IN-07, IN-11, IN-12), over HTTP to a
//! running `cases serve`: the page for every page path, the SPA's files,
//! the policy that names the page's one inline script, and the build a page
//! compares with its own. What the page draws runs in a browser, which no
//! test here opens.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::Env;
use common::serve::Request;

/// Every page path is the one page, which the SPA draws from the path: an
/// unknown case too, which the SPA says is not found. Anything else is 404.
#[test]
fn every_page_path_is_the_page() {
    let env = Env::new();
    let id = env.open_fyi("Heads up");
    let serve = env.serve(&[]);
    let page = serve.get("/");
    assert_eq!(page.status, 200, "{page:?}");
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(page.header("cache-control"), Some("no-cache"));
    assert!(
        page.header("etag").is_some_and(|tag| tag.starts_with('"')),
        "{page:?}"
    );
    assert!(
        page.body
            .contains(r#"<script type="module" src="/static/app.js"></script>"#)
    );
    for path in [
        "/?show=done".to_owned(),
        "/done".to_owned(),
        "/done?show=inflight".to_owned(),
        "/message".to_owned(),
        format!("/message?to=bun-pins&re={id}"),
        format!("/cases/{id}"),
        format!("/cases/{id}?show=all"),
        format!("/cases/{id}-heads-up"),
        "/cases/zzzzz".to_owned(),
    ] {
        let other = serve.get(&path);
        assert_eq!((other.status, &other.body), (200, &page.body), "{path}");
    }
    for path in [
        "/inbox",
        "/cases",
        "/cases/",
        "/cases/UkLWZ/view",
        "/done/x",
        "/message/x",
        "/static",
    ] {
        let missing = serve.get(path);
        assert_eq!(missing.status, 404, "{path}: {missing:?}");
        assert_eq!(missing.category(), "request", "{path}");
    }
}

/// The page's policy allows scripts from the server's files and its one
/// inline script, the import map, by its hash; nothing is fetched from
/// elsewhere (IN-11).
#[test]
fn the_page_s_policy_names_its_one_inline_script() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let page = serve.get("/");
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.starts_with("default-src 'self'; "), "{csp}");
    assert!(csp.contains("script-src 'self' 'sha256-"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(!csp.contains("unsafe"), "{csp}");
    assert_eq!(page.body.matches("<script type=\"importmap\">").count(), 1);
    assert!(
        !page.body.contains("http://") && !page.body.contains("https://"),
        "{}",
        page.body
    );
}

/// Each of the SPA's files is served with its type, a tag, and no-cache,
/// so a reload after an upgrade gets the new build; a request that has the
/// file already is told so (IN-12).
#[test]
fn the_spa_s_files_are_served_with_a_tag() {
    let env = Env::new();
    let serve = env.serve(&[]);
    for (path, kind) in [
        ("/static/style.css", "text/css; charset=utf-8"),
        ("/static/theme.js", "text/javascript; charset=utf-8"),
        ("/static/app.js", "text/javascript; charset=utf-8"),
        (
            "/static/vendor/preact.module.js",
            "text/javascript; charset=utf-8",
        ),
        (
            "/static/vendor/hooks.module.js",
            "text/javascript; charset=utf-8",
        ),
        (
            "/static/vendor/htm.module.js",
            "text/javascript; charset=utf-8",
        ),
        (
            "/static/vendor/marked.esm.js",
            "text/javascript; charset=utf-8",
        ),
    ] {
        let file = serve.get(path);
        assert_eq!(file.status, 200, "{path}: {file:?}");
        assert_eq!(file.header("content-type"), Some(kind), "{path}");
        if path == "/static/style.css" {
            // A hidden element must stay hidden at every width and look
            // (regression: a phone-width `button { display: block }` once
            // overrode it).
            assert!(
                file.body.contains("[hidden] { display: none !important; }"),
                "{}",
                file.body
            );
        }
        assert_eq!(file.header("cache-control"), Some("no-cache"), "{path}");
        assert_eq!(
            file.header("x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
        let tag = file.header("etag").unwrap();
        let again = serve.send(&Request::new("GET", path).header("If-None-Match", tag));
        assert_eq!(again.status, 304, "{path}: {again:?}");
        assert_eq!(again.body, "", "{path}");
        let other = serve.send(&Request::new("GET", path).header("If-None-Match", "\"0\""));
        assert_eq!(other.status, 200, "{path}");
    }
    // A path that is not text, such as `%FF`, is no file either: the same
    // 404 as any other path, not axum's own refusal.
    for path in [
        "/static/nothing.js",
        "/static/vendor/README.md",
        "/static/vendor/preact.LICENSE.txt",
        "/static/../index.html",
        "/static/%FF",
    ] {
        let missing = serve.get(path);
        assert_eq!(missing.status, 404, "{path}: {missing:?}");
        assert_eq!(missing.category(), "request", "{path}");
    }
}

/// `GET /v1` gives the build: the version, and a hash of the page's files,
/// which a page compares with the build it names when its stream opens
/// (IN-12).
#[test]
fn v1_gives_the_build_of_the_page() {
    let env = Env::new();
    let serve = env.serve(&[]);
    let index = serve.get("/v1").json();
    let build = index["version"].as_str().unwrap();
    let (version, hash) = build.split_once('+').unwrap();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert!(
        hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "{build}"
    );
    let page = serve.get("/").body;
    assert!(
        page.contains(&format!(r#"<meta name="cases-build" content="{build}">"#)),
        "{page}"
    );
    assert_eq!(serve.stop().code, 0);
    let again = env.serve(&[]).get("/v1").json();
    assert_eq!(again["version"], build, "the same files are the same build");
}
