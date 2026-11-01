//! The inbox's page: `index.html` for every page path, and the SPA's files
//! under `/static/` (IN-01, IN-02, IN-07, IN-11). Everything is in the
//! binary, and nothing is fetched from the network. Each file is served with
//! an `ETag` and `Cache-Control: no-cache`, so a reload after an upgrade
//! always gets the new build, and the page names its build, so a page the
//! browser takes from its cache without asking knows it is an older one.
//! The vendored mermaid is in the binary gzipped, and served so to a
//! browser, which always takes gzip.

use std::io::Read as _;
use std::sync::LazyLock;

use axum::extract::Path;
use axum::extract::rejection::PathRejection;
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};

use super::app::Refusal;
use crate::instance;

/// The policy on every response: nothing from anywhere but this server,
/// scripts from files, and the page's one inline script, its import map,
/// by its hash; and no page may frame the inbox. A test checks the hash
/// against `index.html`.
pub(crate) const CSP: &str = "default-src 'self'; \
    script-src 'self' 'sha256-/6fO3A6JvnCxDv/qyHqkkelFqgZQxYll4HhP1qfVyhw='; \
    frame-ancestors 'none'; form-action 'self'";

/// The page, for every page path: the SPA picks the view from the path.
const INDEX: &[u8] = include_bytes!("web/index.html");

const JS: &str = "text/javascript; charset=utf-8";
const SVG: &str = "image/svg+xml";
const WOFF2: &str = "font/woff2";

/// A file under `/static/`.
#[derive(Debug)]
struct File {
    path: &'static str,
    kind: &'static str,
    bytes: &'static [u8],
}

/// The files under `/static/`: the stylesheet, the SPA's modules, which
/// build.rs strips of their types from `web/static/*.ts` into `$OUT_DIR`,
/// the glyphs a look draws for each kind of case, under `kinds/<look>/`,
/// the mark, Tray, under `brand/`, and the vendored libraries and fonts, as
/// they are in `web/static/vendor/` (see its README).
const FILES: &[File] = &[
    File {
        path: "style.css",
        kind: "text/css; charset=utf-8",
        bytes: include_bytes!("web/static/style.css"),
    },
    File {
        path: "theme.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/theme.js")),
    },
    File {
        path: "app.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/app.js")),
    },
    File {
        path: "api.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/api.js")),
    },
    File {
        path: "ask.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/ask.js")),
    },
    File {
        path: "case.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/case.js")),
    },
    File {
        path: "code.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/code.js")),
    },
    File {
        path: "compose.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/compose.js")),
    },
    File {
        path: "diagram.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/diagram.js")),
    },
    File {
        path: "followup.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/followup.js")),
    },
    File {
        path: "fresh.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/fresh.js")),
    },
    File {
        path: "html.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/html.js")),
    },
    File {
        path: "ids.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/ids.js")),
    },
    File {
        path: "kinds.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/kinds.js")),
    },
    File {
        path: "keys.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/keys.js")),
    },
    File {
        path: "lists.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/lists.js")),
    },
    File {
        path: "links.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/links.js")),
    },
    File {
        path: "markdown.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/markdown.js")),
    },
    File {
        path: "options.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/options.js")),
    },
    File {
        path: "page.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/page.js")),
    },
    File {
        path: "props.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/props.js")),
    },
    File {
        path: "routes.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/routes.js")),
    },
    File {
        path: "thread.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/thread.js")),
    },
    File {
        path: "time.js",
        kind: JS,
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/static/time.js")),
    },
    File {
        path: "kinds/daylight/decision.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/decision.svg"),
    },
    File {
        path: "kinds/daylight/approval.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/approval.svg"),
    },
    File {
        path: "kinds/daylight/signoff.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/signoff.svg"),
    },
    File {
        path: "kinds/daylight/question.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/question.svg"),
    },
    File {
        path: "kinds/daylight/stuck.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/stuck.svg"),
    },
    File {
        path: "kinds/daylight/fyi.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/fyi.svg"),
    },
    File {
        path: "kinds/daylight/message.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/message.svg"),
    },
    File {
        path: "kinds/daylight/briefing.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/daylight/briefing.svg"),
    },
    File {
        path: "kinds/keel/decision.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/decision.svg"),
    },
    File {
        path: "kinds/keel/approval.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/approval.svg"),
    },
    File {
        path: "kinds/keel/signoff.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/signoff.svg"),
    },
    File {
        path: "kinds/keel/question.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/question.svg"),
    },
    File {
        path: "kinds/keel/stuck.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/stuck.svg"),
    },
    File {
        path: "kinds/keel/fyi.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/fyi.svg"),
    },
    File {
        path: "kinds/keel/message.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/message.svg"),
    },
    File {
        path: "kinds/keel/briefing.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/kinds/keel/briefing.svg"),
    },
    File {
        path: "brand/mark.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/brand/mark.svg"),
    },
    File {
        path: "brand/mark-accent-light.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/brand/mark-accent-light.svg"),
    },
    File {
        path: "brand/mark-accent-dark.svg",
        kind: SVG,
        bytes: include_bytes!("web/static/brand/mark-accent-dark.svg"),
    },
    File {
        path: "vendor/preact.module.js",
        kind: JS,
        bytes: include_bytes!("web/static/vendor/preact.module.js"),
    },
    File {
        path: "vendor/hooks.module.js",
        kind: JS,
        bytes: include_bytes!("web/static/vendor/hooks.module.js"),
    },
    File {
        path: "vendor/htm.module.js",
        kind: JS,
        bytes: include_bytes!("web/static/vendor/htm.module.js"),
    },
    File {
        path: "vendor/marked.esm.js",
        kind: JS,
        bytes: include_bytes!("web/static/vendor/marked.esm.js"),
    },
    File {
        path: "vendor/inter-latin-wght-normal.woff2",
        kind: WOFF2,
        bytes: include_bytes!("web/static/vendor/inter-latin-wght-normal.woff2"),
    },
    File {
        path: "vendor/jetbrains-mono-latin-wght-normal.woff2",
        kind: WOFF2,
        bytes: include_bytes!("web/static/vendor/jetbrains-mono-latin-wght-normal.woff2"),
    },
];

/// The files under `/static/vendor/mermaid/`, which build.rs gzips from
/// `web/static/vendor/mermaid/` (see its README): each file's bytes are
/// gzip's. The page loads them only to draw a diagram.
const GZIPPED: &[File] = include!(concat!(env!("OUT_DIR"), "/mermaid.rs"));

/// Each file's `ETag`: a hash of its bytes, in quotes, gzipped or not as
/// they are in the binary, for the files then the gzipped files. The last
/// is that of `index.html` as written, which the build is made of with the
/// rest.
static TAGS: LazyLock<Vec<String>> = LazyLock::new(|| {
    FILES
        .iter()
        .chain(GZIPPED)
        .map(|file| file.bytes)
        .chain([INDEX])
        .map(|bytes| format!("\"{:016x}\"", instance::fnv1a(bytes)))
        .collect()
});

/// The build a page is: the version of cases, and a hash of every file the
/// page is made of, so that a page from another build, even of the same
/// version, knows it (IN-12).
pub(crate) static BUILD: LazyLock<String> = LazyLock::new(|| {
    let files: String = TAGS.concat();
    format!(
        "{}+{:016x}",
        env!("CARGO_PKG_VERSION"),
        instance::fnv1a(files.as_bytes())
    )
});

/// The page as served: `index.html` with the build it belongs to, which the
/// SPA compares with serve's. A page the browser takes from its cache, as
/// on a move back, so knows that it is from before an upgrade (IN-12).
static PAGE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    String::from_utf8_lossy(INDEX)
        .replace("{build}", &BUILD)
        .into_bytes()
});

/// The page's `ETag`: a hash of the page as served, which changes with the
/// build it names. A tag of `index.html` as written would let a browser keep
/// the page of an older build when only another file changed.
static PAGE_TAG: LazyLock<String> =
    LazyLock::new(|| format!("\"{:016x}\"", instance::fnv1a(PAGE.as_slice())));

/// `GET /`, `GET /done` and `GET /cases/{id}`: the page. An unknown case is
/// the page too; the SPA reads the case from the API and says it is not
/// found. `/done` is where the done list was: the SPA shows the list under
/// its done filter, at that filter's address, `/?show=done`.
pub(crate) async fn index(headers: HeaderMap) -> Response {
    file(
        &headers,
        &PAGE_TAG,
        "text/html; charset=utf-8",
        PAGE.as_slice(),
    )
    .into_response()
}

/// `GET /static/{path}`: a file of the page. A path that is not text, such
/// as `%FF`, is no file either, so it is the 404 of any other path rather
/// than axum's own refusal.
pub(crate) async fn static_file(
    headers: HeaderMap,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(path)) = path else {
        return Refusal::request(StatusCode::NOT_FOUND, "no such path").into_response();
    };
    if let Some(at) = FILES.iter().position(|file| file.path == path) {
        return file(&headers, &TAGS[at], FILES[at].kind, FILES[at].bytes).into_response();
    }
    match GZIPPED.iter().position(|file| file.path == path) {
        Some(at) => gzipped(&headers, &TAGS[FILES.len() + at], &GZIPPED[at]),
        None => Refusal::request(StatusCode::NOT_FOUND, "no such path").into_response(),
    }
}

/// A gzipped file: as it is, with `Content-Encoding: gzip`, when the
/// request takes gzip, and unzipped when it does not, under a tag of its
/// own. `Vary` says so to a cache between.
fn gzipped(headers: &HeaderMap, tag: &str, gz: &File) -> Response {
    let vary = [(header::VARY, HeaderValue::from_static("accept-encoding"))];
    if takes_gzip(headers) {
        let encoding = [(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"))];
        return (vary, encoding, file(headers, tag, gz.kind, gz.bytes)).into_response();
    }
    let tag = format!("{}-plain\"", tag.trim_end_matches('"'));
    // A request that has the file already needs no unzipping.
    if fresh(headers, &tag) {
        return (vary, file(headers, &tag, gz.kind, &[][..])).into_response();
    }
    let Ok(plain) = gunzip(gz.bytes) else {
        return Refusal::request(StatusCode::INTERNAL_SERVER_ERROR, "a file did not unzip")
            .into_response();
    };
    (vary, file(headers, &tag, gz.kind, plain)).into_response()
}

/// `gz` unzipped.
fn gunzip(gz: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut plain)?;
    Ok(plain)
}

/// Whether the request's `Accept-Encoding` takes gzip: it names gzip with a
/// weight above zero, or, when it does not name gzip, `*` with a weight
/// above zero.
fn takes_gzip(headers: &HeaderMap) -> bool {
    let mut gzip = None;
    let mut any = None;
    for coding in headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
    {
        let mut parts = coding.split(';');
        let name = parts.next().unwrap_or_default().trim();
        let takes = !parts.any(|part| {
            let part = part.trim();
            part.get(..2)
                .is_some_and(|q| q.eq_ignore_ascii_case("q="))
                .then(|| part[2..].trim().parse::<f32>().ok())
                .flatten()
                .is_some_and(|q| q <= 0.0)
        });
        if name.eq_ignore_ascii_case("gzip") {
            gzip = Some(gzip.unwrap_or(false) || takes);
        } else if name == "*" {
            any = Some(any.unwrap_or(false) || takes);
        }
    }
    gzip.or(any).unwrap_or(false)
}

/// Whether the request says it has the file tagged `tag`.
fn fresh(headers: &HeaderMap, tag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|have| have.trim() == tag))
}

/// `bytes` as a response with its type and `ETag`, or 304 when the request
/// says it has them.
fn file(
    headers: &HeaderMap,
    tag: &str,
    kind: &'static str,
    bytes: impl Into<axum::body::Body>,
) -> impl IntoResponse {
    let fresh = fresh(headers, tag);
    let tag = HeaderValue::from_str(tag).unwrap_or(HeaderValue::from_static("\"\""));
    let cache = [
        (header::ETAG, tag),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    if fresh {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (
        cache,
        [(header::CONTENT_TYPE, HeaderValue::from_static(kind))],
        bytes.into(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};

    use super::*;

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let word = chunk.iter().enumerate().fold(0u32, |word, (i, byte)| {
                word | u32::from(*byte) << (16 - 8 * i)
            });
            for i in 0..4 {
                if i <= chunk.len() {
                    text.push(char::from(ALPHABET[(word >> (18 - 6 * i) & 63) as usize]));
                } else {
                    text.push('=');
                }
            }
        }
        text
    }

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).unwrap()
    }

    /// The file under `/static/` at `path`.
    fn served(path: &str) -> &'static File {
        FILES
            .iter()
            .chain(GZIPPED)
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} is not served"))
    }

    /// The bytes of the file at `path` as written: a gzipped file unzipped.
    fn plain(path: &str) -> Vec<u8> {
        let file = served(path);
        if !GZIPPED.iter().any(|gz| gz.path == path) {
            return file.bytes.to_vec();
        }
        gunzip(file.bytes).unwrap()
    }

    #[test]
    fn base64_is_the_standard_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    /// Each vendored file is the release its README names, byte for byte
    /// (IN-11): a file that changes fails here until the README and this
    /// table change with it. A gzipped file is checked as it unzips, and
    /// each is in the table.
    #[test]
    fn each_vendored_file_is_the_recorded_release() {
        let readme = include_str!("web/static/vendor/README.md");
        let files = [
            (
                "vendor/preact.module.js",
                "c30e721ebfdc6e2ad4c18c14d2dfb82667829c8aec27de1207774e3fc16858a8",
            ),
            (
                "vendor/hooks.module.js",
                "a6ee626f2d01570592dd569a792e3f050154aa02890eead8c223fa3ed5aa3d5a",
            ),
            (
                "vendor/htm.module.js",
                "ab33dd3f38059b9be4d5f5350128eefb2356639c4e0bbe9d9e8b3ba75847e9e4",
            ),
            (
                "vendor/marked.esm.js",
                "528a1b88bef88fc27277e06036ce7f4afb6a220b18110ba57c09e292b24a7ce0",
            ),
            (
                "vendor/inter-latin-wght-normal.woff2",
                "3100e775e8616cd2611beecfa23a4263d7037586789b43f035236a2e6fbd4c62",
            ),
            (
                "vendor/jetbrains-mono-latin-wght-normal.woff2",
                "18be452724bfdc236c074ca94a249a7f41a86752c7d04ab258ce9ed5651f6a7e",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-3KU6CYTB.mjs",
                "00edcfae1102c086d7d7c0f69e03bb9e66d341c2d7bbd4a60a60ec6127e40007",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-4GHFHL4L.mjs",
                "396c335a6630a82ef77635b7014b65d6b3095e72da420e7946a6a5c88f6d3c7e",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-674HQBI4.mjs",
                "78a14415742410cd85c530442d1fbc121550f42ae99e0618150e0920314f5ff3",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-7P5HNUOW.mjs",
                "d0aa9958356deea20cadbccb3e2d1283ddbc955aa7ccf8a707f673f2dbda8d5e",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-BOZ75642.mjs",
                "258c1f8a5e8fc1cc5edfd343779c9c50f0bf8932a6e7fa67aa2d174283b617fa",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-D36F7FYG.mjs",
                "6fa5cda64a6227731eb422b312aaff0070f0986097e8d94354ac6101a25c398b",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-DXAG4NAJ.mjs",
                "c907572175ed48fd9471e38ed333fc3b2edaadf482839998b2f57079d318902a",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-F6PIGZHG.mjs",
                "7a600c611d09d8a05cd2e6f96f13901d9ba27576fc21c77e63668af1005cd163",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-FNR2RGIF.mjs",
                "8d7c420d9526ab350be333dbf743e91354691a6481275e64f1db7549db4dba05",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-FWWDAFD2.mjs",
                "6599aa9fff9fe86e90eb3cc860483ee0910e5fa029e6336895aeb38e5b58bbe8",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-GDOATIN3.mjs",
                "0613e167b74c7f000549b95541b971a1ec378792a71beed86196c60a1fce0700",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-GHI2JMQX.mjs",
                "46d67e0fa2bf1579a00d9565db7e0cadcfe7ee3a4449aad4cec62d9cd52f3f28",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-IER4RZHT.mjs",
                "068959b1dfcb8d26bcbfd1e34ad922467d0590eae06cec62f29cb5a285f8748a",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-KJ5T2YZX.mjs",
                "88af19a0945746df13d4c25164c83d47cca25f24485412c982d70122b16a5283",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-KTS5TP6K.mjs",
                "0ae23b8e2fb9297d068a8b1f745bc2b08a55320dfb0db90711863b7d74ba2f86",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-L2B47AAZ.mjs",
                "57b417baaff19ed1a3383bf327d39247c0c4aa876dc433e09f708d2e5950b71a",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-LGKD3V67.mjs",
                "4bfc3262830f618555cfa3403a7773939d4ad11af2e36db796ac11f8ee726a92",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-MFYRHTYG.mjs",
                "907abf3f3c54b58e7577c4bd6a3cfa18a9978110765843cd0d46748d6db4e748",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-MIE3LDYP.mjs",
                "5d4f7c6773b99ffca185401cf164d111ab304d05709d933c10ce47a1e06d5188",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-MMC4O45A.mjs",
                "9a375b48e696b59371d6535c26cf4ce73a759911d6af71d0ca60ef4526c944c5",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-QVIEFLGF.mjs",
                "5ad0557882607c7b100c33722714c62832a84feb26d79a7151fdbe4680f2165b",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-SDIBIOXT.mjs",
                "c927a4a7d111b376cddf6c30a73bc2366a6fa1be145ca69abdead4c55d24fb4f",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-XGFVAD2T.mjs",
                "e9826d66fac551f54b354d1499ad41b94136845c3d833ebdd9da1c597b077204",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-XUG7IRDC.mjs",
                "1d906b46af61082cb43f6d3bc1a903b7108975c7a66606b6cf52d46d2670647e",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-YX6CVSZ3.mjs",
                "940039393be1f982148fd4dfc998b4512fba3ae556b7d75d86b3d831992e9315",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/chunk-ZZ52EIBP.mjs",
                "05866f9f6191141f83fd225d0ae8071873cdd24f94e8b99023cd3401ad4a12a8",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/classDiagram-v2-Y5FV6ONJ.mjs",
                "c36c276b44e7807b119af2434346d4a4a6e53846ed196dd819bfae8a717f922d",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/dagre-JKPHJXCU.mjs",
                "c9dbd1a82b6d86ed7f7e4979c5f9ec94406c727be4a9928e99cc142deff1683c",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/erDiagram-Y4LBXKHG.mjs",
                "10d0cd14992ac9ae0f432399b43ee2802752b812a68438770ceb142dc367c0e0",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/flowDiagram-KMDKLPNL.mjs",
                "0130c02c9cbea5f095c5318affd4a840a17f35652beaa0ff1ff085532d0eef32",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/sequenceDiagram-OV7DSCPC.mjs",
                "7eb32f7cac87fe60c149f14824e4b31dbe6c0898f152dd30fcd382823e0d46ff",
            ),
            (
                "vendor/mermaid/chunks/mermaid.esm.min/stateDiagram-v2-PVXPCO6V.mjs",
                "fc40c5b8600e2cc3462be10f72aaa997bdda89568d5b3d82d84fc1cf3d19933c",
            ),
            (
                "vendor/mermaid/mermaid.esm.min.mjs",
                "dae3440dd5a4adecf76cfd2951c7a69523df3237f110dd186bebbd55f6038c9d",
            ),
        ];
        for (path, sha256) in files {
            assert_eq!(hex(&plain(path)), sha256, "{path}");
            assert!(readme.contains(sha256), "the README does not record {path}");
        }
        for gz in GZIPPED {
            assert!(
                files.iter().any(|(path, _)| *path == gz.path),
                "{} is not recorded",
                gz.path
            );
        }
    }

    /// Each file the vendored mermaid imports as it loads is served, so
    /// what is left out of it is only what a kind of diagram not vendored
    /// imports when it is drawn (vendor/README.md).
    #[test]
    fn every_file_mermaid_imports_is_served() {
        for gz in GZIPPED {
            let source = String::from_utf8(plain(gz.path)).unwrap();
            let dir = &gz.path[..gz.path.rfind('/').unwrap()];
            for part in source
                .split("from\"")
                .skip(1)
                .chain(source.split("import\"").skip(1))
            {
                let name = &part[..part.find('"').unwrap()];
                let Some(name) = name.strip_prefix("./") else {
                    continue;
                };
                served(&format!("{dir}/{name}"));
            }
        }
    }

    /// A gzipped file goes as it is to a request that takes gzip, and
    /// unzipped, under a tag of its own, to one that does not; either way
    /// the response varies with Accept-Encoding.
    #[test]
    fn a_gzipped_file_is_unzipped_for_a_request_without_gzip() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let gz = served("vendor/mermaid/mermaid.esm.min.mjs");
        let get = |accept: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(accept) = accept {
                headers.insert(
                    header::ACCEPT_ENCODING,
                    HeaderValue::from_str(accept).unwrap(),
                );
            }
            let response = gzipped(&headers, "\"tag\"", gz);
            let head = response.headers().clone();
            let body = runtime
                .block_on(axum::body::to_bytes(response.into_body(), usize::MAX))
                .unwrap();
            (head, body)
        };
        for accept in [
            "gzip, deflate, br, zstd",
            "br;q=1.0, GZIP;q=0.5",
            "*",
            "br, *;q=0.5",
        ] {
            let (head, body) = get(Some(accept));
            assert_eq!(head[header::CONTENT_ENCODING], "gzip", "{accept}");
            assert_eq!(head[header::VARY], "accept-encoding", "{accept}");
            assert_eq!(head[header::ETAG], "\"tag\"", "{accept}");
            assert_eq!(&body[..], gz.bytes, "{accept}");
        }
        for accept in [
            None,
            Some("identity"),
            Some("br"),
            Some("gzip;q=0"),
            Some("gzip; q=0.0, br"),
            Some("gzip;q=0, *"),
            Some("*;Q=0"),
        ] {
            let (head, body) = get(accept);
            assert!(head.get(header::CONTENT_ENCODING).is_none(), "{accept:?}");
            assert_eq!(head[header::VARY], "accept-encoding", "{accept:?}");
            assert_eq!(head[header::ETAG], "\"tag-plain\"", "{accept:?}");
            assert_eq!(&body[..], plain(gz.path), "{accept:?}");
        }
    }

    /// The types for tsc are the vendored releases: package.json pins each
    /// library at the version the vendor README records, so the page is
    /// checked against the code it runs.
    #[test]
    fn the_types_are_the_vendored_releases() {
        let readme = include_str!("web/static/vendor/README.md");
        let package: serde_json::Value =
            serde_json::from_str(include_str!("web/package.json")).unwrap();
        assert!(package.get("dependencies").is_none(), "{package}");
        let dev = package["devDependencies"].as_object().unwrap();
        for name in ["preact", "htm", "marked", "mermaid"] {
            let version = dev[name].as_str().unwrap();
            assert!(
                readme.contains(&format!("| {name} {version}, ")),
                "package.json pins {name} {version}, which the vendor README does not record"
            );
        }
        for version in dev.values() {
            let version = version.as_str().unwrap();
            assert!(
                version.chars().all(|c| c.is_ascii_digit() || c == '.'),
                "not an exact version: {version}"
            );
        }
    }

    /// What the page serves as a script is JavaScript: build.rs stripped
    /// every type, and left nothing of TypeScript's own. theme.js is a
    /// plain script, the rest modules.
    #[test]
    fn every_script_served_is_javascript() {
        use oxc_allocator::Allocator;
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        for file in FILES.iter().filter(|file| file.kind == JS) {
            let kind = if file.path == "theme.js" {
                SourceType::script()
            } else {
                SourceType::mjs()
            };
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, text(file.bytes), kind).parse();
            assert!(
                parsed.diagnostics.is_empty(),
                "{}: {:?}",
                file.path,
                parsed.diagnostics
            );
        }
    }

    /// Each of the page's TypeScript modules is served, as its `.js`.
    #[test]
    fn every_module_written_is_served() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands/serve/web/static");
        let mut written = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if let Some(module) = name.strip_suffix(".ts")
                && !name.ends_with(".d.ts")
            {
                served(&format!("{module}.js"));
                written += 1;
            }
        }
        let own = FILES
            .iter()
            .filter(|file| file.kind == JS && !file.path.starts_with("vendor/"))
            .count();
        assert_eq!(written, own, "a module served has no .ts");
    }

    /// The page has one inline script, its import map, and the policy names
    /// it by its hash; every other script is a file, and none runs code
    /// from an attribute (IN-11).
    #[test]
    fn the_policy_names_the_import_map_by_its_hash() {
        let page = text(INDEX);
        let open = r#"<script type="importmap">"#;
        let start = page.find(open).expect("an import map") + open.len();
        let map = &page[start..start + page[start..].find("</script>").unwrap()];
        let hash = format!("'sha256-{}'", base64(&Sha256::digest(map.as_bytes())));
        assert!(CSP.contains(&hash), "the policy lacks {hash}: {CSP}");
        assert!(CSP.starts_with("default-src 'self'; "), "{CSP}");
        assert!(!CSP.contains("unsafe"), "{CSP}");
        for script in page.split("<script").skip(1) {
            let tag = script.split('>').next().unwrap_or_default();
            assert!(
                tag.contains(" src=") || tag == r#" type="importmap""#,
                "an inline script: <script{tag}>"
            );
        }
        for tag in page.split('<').skip(1) {
            let attributes = tag.split('>').next().unwrap_or_default();
            assert!(
                !attributes.contains(" on"),
                "an event handler in <{attributes}>"
            );
        }
    }

    /// The import map resolves each bare name the modules import to a file
    /// served, and every module one imports is served (IN-11).
    #[test]
    fn every_module_imported_is_served() {
        let page = text(INDEX);
        let map: serde_json::Value = {
            let open = r#"<script type="importmap">"#;
            let start = page.find(open).unwrap() + open.len();
            serde_json::from_str(&page[start..start + page[start..].find("</script>").unwrap()])
                .unwrap()
        };
        let imports = map["imports"].as_object().unwrap();
        for target in imports.values() {
            served(target.as_str().unwrap().strip_prefix("/static/").unwrap());
        }
        for file in FILES.iter().filter(|file| file.kind == JS) {
            let source = text(file.bytes);
            // Each `from` of an import, as minified code writes it too
            // (`from"preact"`); not one that is a string's own text, such as
            // a property named "from", which a quote comes right before.
            let parts = source
                .match_indices("from")
                .filter(|(at, _)| !source[..*at].ends_with(['"', '\'']))
                .map(|(at, word)| &source[at + word.len()..]);
            for part in parts {
                let Some(name) = part
                    .trim_start()
                    .strip_prefix('"')
                    .and_then(|rest| rest.split('"').next())
                else {
                    continue;
                };
                if let Some(module) = name.strip_prefix("./") {
                    let dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
                    let path = if dir.is_empty() {
                        module.to_owned()
                    } else {
                        format!("{dir}/{module}")
                    };
                    served(&path);
                } else if !name.contains('/') || name.starts_with("preact") {
                    assert!(
                        imports.contains_key(name),
                        "{}: {name} is not mapped",
                        file.path
                    );
                }
            }
        }
        assert!(page.contains(r#"<script type="module" src="/static/app.js"></script>"#));
    }

    /// The preferences apply before the page paints: theme.js is a plain
    /// script, loaded before the stylesheet (IN-10).
    #[test]
    fn the_theme_applies_before_the_stylesheet() {
        let page = text(INDEX);
        let script = page
            .find(r#"<script src="/static/theme.js"></script>"#)
            .expect("theme.js");
        let sheet = page
            .find(r#"<link rel="stylesheet" href="/static/style.css">"#)
            .expect("the stylesheet");
        assert!(script < sheet, "theme.js loads after the stylesheet");
        let theme = text(served("theme.js").bytes);
        // The first value of each option is its default, which the options
        // dialog marks as chosen when nothing is stored.
        for want in [
            r#"{ name: "look", values: ["daylight", "mono", "keel"] }"#,
            r#"{ name: "theme", values: ["system", "light", "dark"] }"#,
            r#"{ name: "face", values: ["look", "mono", "sans", "serif"] }"#,
            r#"{ name: "size", values: ["medium", "small", "large"] }"#,
            r#"{ name: "links", values: ["new", "same"] }"#,
            r#"{ name: "notify", values: ["off", "blocking", "all"] }"#,
            r#"{ name: "hints", values: ["auto", "show", "hide"] }"#,
            r#"{ name: "approve", values: ["off", "on"] }"#,
            r#"{ name: "wrap", values: ["scroll", "wrap"] }"#,
        ] {
            assert!(theme.contains(want), "theme.js lacks {want}");
        }
    }

    /// The stylesheet keeps the rules the page relies on: page changes that
    /// cross-fade unless motion is reduced, inbox zero in the case pane, the
    /// split across the window, bold figures that stay readable on hover, the
    /// recorded box that floats over the page and goes by itself, fields that
    /// grow, and one column on a narrow window (IN-01, IN-03).
    #[test]
    fn the_stylesheet_keeps_its_rules() {
        let css = text(served("style.css").bytes);
        for want in [
            "header.top { view-transition-name: masthead; }",
            "::view-transition-group(masthead) { animation: none; }",
            ".split > .list { view-transition-name: inbox-list; }",
            ".split.zero .work { display: block; }",
            "main:has(> .split) { max-width: none; padding: 0; margin-inline: 0; }",
            "a:hover strong { color: inherit; }",
            ".recorded { animation: recorded-in .25s ease-out both, recorded-out .6s ease-in 6s forwards; }",
            ".recorded:hover, .recorded:focus-within { animation-play-state: paused; }",
            "position: fixed; z-index: 10; right: max(1rem, env(safe-area-inset-right));",
            "field-sizing: content;",
            r#"textarea[rows="1"] { --rows: 1; }"#,
            ".split.home .work, .split:not(.home) .list { display: none; }",
            ".case-main:has(> .answer > form) > .answer {",
            "main:has(> .new-build-live > .new-build) > .split { flex: 1; min-height: 0; height: auto; }",
            ".body th, .body td { padding: .25rem .75rem .25rem 0; text-align: left; vertical-align: top; overflow-wrap: normal; }",
            ".body th, .body td.short, .body td code { white-space: nowrap; }",
            "grid-column: 2; grid-row: 1 / span 2; position: sticky; top: calc(var(--bar) + 1rem);",
            "position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 1rem;",
            "grid-area: props; align-self: start; position: sticky; top: 0;",
            ".work { container: work / inline-size; --pane: 100cqh; }",
            ".split > .work { container-type: size; }",
            ".split:has(> .waiting-live.float > .waiting-notice) > .work { --pane: calc(100cqh - 4.5rem); }",
            "position: fixed; z-index: 9; padding: 0; bottom: max(1rem, env(safe-area-inset-bottom));",
            "max-height: max(min(12rem, var(--pane)), var(--pane) - var(--lift, 0rem)); overflow-y: auto;",
            "--cap: calc(var(--pane) - var(--bar) - 1rem);",
            "max-height: max(min(12rem, var(--cap)), var(--cap) - var(--lift, 0rem)); overflow-y: auto;",
        ] {
            assert!(css.contains(want), "style.css lacks {want}");
        }
        // The caps of the boxes held in view take off their --lift, which
        // case.js keeps on them as the case scrolls.
        let case = text(served("case.js").bytes);
        for want in [
            r##"const HELD = "#case > .props, .case-main > .answer";"##,
            r#"box.style.setProperty("--lift", "#,
            r#"document.addEventListener("scroll", scrolled, { capture: true, passive: true });"#,
            "const grown = new ResizeObserver(all);",
        ] {
            assert!(case.contains(want), "case.js lacks {want}");
        }
        // The case's id is shown at every width, as agents write it in chat.
        assert!(
            !css.contains(".prop-id { display: none; }"),
            "style.css hides the case's id"
        );
        // The boxes held in view are sized by the case's pane, not the
        // window, which a new build's line or a wrapped masthead takes from.
        assert!(
            !css.contains("100dvh - 5rem"),
            "a box held in view is sized by the window"
        );
        let on = css.find("@view-transition { navigation: auto; }").unwrap();
        let off = css
            .find("@media (prefers-reduced-motion: reduce) {\n  @view-transition { navigation: none; }\n}")
            .unwrap();
        assert!(
            on < off,
            "reduced motion does not follow the view transition"
        );
        for gone in ["main:has(> .recorded)", "--recorded-h"] {
            assert!(
                !css.contains(gone),
                "the recorded box is in the page's flow: {gone}"
            );
        }
        let moving = css.find("@keyframes recorded-out {").unwrap();
        let fading = css
            .find("@media (prefers-reduced-motion: reduce) {\n  @keyframes recorded-in { from { opacity: 0; } }\n  @keyframes recorded-out { to { opacity: 0; } }\n}")
            .unwrap();
        assert!(
            moving < fading,
            "the recorded line moves under reduced motion"
        );
    }

    /// Every file the stylesheet names, a font or a kind's glyph, is served,
    /// as the type the browser takes it for; and a look that draws the
    /// glyphs of kinds (kinds.js) draws one for every kind.
    #[test]
    fn the_stylesheet_names_only_files_served() {
        let css = text(served("style.css").bytes);
        let mut named = Vec::new();
        for (at, _) in css.match_indices("url(") {
            let rest = &css[at + 4..];
            let url = rest[..rest.find(')').unwrap()].trim_matches('"');
            if url.starts_with('#') {
                continue;
            }
            let path = url
                .strip_prefix("/static/")
                .unwrap_or_else(|| panic!("style.css names {url}, which is not under /static/"));
            let file = served(path);
            let want = match path.rsplit('.').next() {
                Some("woff2") => WOFF2,
                Some("svg") => SVG,
                _ => panic!("style.css names {url}, which is no font or image"),
            };
            assert_eq!(file.kind, want, "{path}");
            named.push(path);
        }
        // Every kind the core knows, so that a kind added later needs its
        // glyph in each look.
        let kinds = cases_core::Kind::ALL.iter().map(|kind| kind.as_str());
        let looks: Vec<&str> = named
            .iter()
            .filter_map(|path| path.strip_prefix("kinds/")?.split('/').next())
            .collect();
        assert!(!looks.is_empty(), "no look draws the glyphs of kinds");
        for look in looks {
            for kind in kinds.clone() {
                let path = format!("kinds/{look}/{kind}.svg");
                assert!(
                    named.contains(&path.as_str()),
                    "{look} draws no glyph for {kind}"
                );
                assert!(
                    text(served(&path).bytes)
                        .starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
                    "{path} is not an SVG image"
                );
            }
        }
        // The files served are those in the tree, so none is left unnamed.
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/serve/web/static/kinds"
        );
        for look in std::fs::read_dir(dir).unwrap() {
            let look = look.unwrap();
            for glyph in std::fs::read_dir(look.path()).unwrap() {
                let path = format!(
                    "kinds/{}/{}",
                    look.file_name().to_string_lossy(),
                    glyph.unwrap().file_name().to_string_lossy()
                );
                assert!(
                    named.contains(&path.as_str()),
                    "style.css does not name {path}"
                );
            }
        }
    }

    /// Lengths are in rem, so the text size option, which sets the root
    /// size, scales the whole page (IN-10). Hairline rules, offsets and a
    /// phone's least text size stay in px, as do media queries, which the
    /// root size does not move.
    #[test]
    fn the_stylesheet_scales_with_the_text_size() {
        let css = text(served("style.css").bytes);
        for want in [
            r#":root[data-size="small"] { font-size: 87.5%; }"#,
            r#":root[data-size="large"] { font-size: 125%; }"#,
        ] {
            assert!(css.contains(want), "style.css lacks {want}");
        }
        // Comments blanked, their line breaks kept, so line numbers hold.
        let mut plain = String::new();
        for (i, part) in css.split("/*").enumerate() {
            let rest = if i == 0 {
                part
            } else {
                let (comment, rest) = part.split_once("*/").unwrap();
                plain.extend(comment.chars().filter(|c| *c == '\n'));
                rest
            };
            plain.push_str(rest);
        }
        for (number, line) in plain.lines().enumerate() {
            if line.contains("@media") {
                continue;
            }
            for (at, _) in line.match_indices("px") {
                let start = line[..at]
                    .rfind(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                    .map_or(0, |i| i + 1);
                let length = &line[start..at + 2];
                let allowed = matches!(length, "1px" | "2px" | "-1px" | "-3px" | "-5px")
                    || length == "16px" && line.contains("max(1rem, 16px)");
                assert!(
                    allowed,
                    "style.css line {} uses {length}: {}",
                    number + 1,
                    line.trim()
                );
            }
        }
    }

    /// The page's slug is the CLI's, run in Node on ids.js as it is served,
    /// so a case's address is the same from the CLI and the page (ST-10); and
    /// the page reads the id from an address with or without a slug.
    #[test]
    fn the_page_s_slug_is_the_cli_s() {
        let titles = [
            "Pin bun or float?",
            "Ünïcode ✓ and all",
            "!!!",
            "",
            "a--b  c",
            "\u{212A}elvin is a K",
            "\u{130}stanbul",
            "ΑΣ final sigma",
            &"word ".repeat(20),
            &format!("{} tail", "x".repeat(70)),
        ];
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("ids.mjs");
        std::fs::write(&module, served("ids.js").bytes).unwrap();
        let script = format!(
            "import {{ slug, idOf, page }} from {:?};\nconst titles = {};\n\
             console.log(JSON.stringify({{ slugs: titles.map(slug), ids: [idOf('UkLWZ'), idOf('UkLWZ-pin-bun'), idOf('UkLWZ-a-b')], page: page({{ id: 'UkLWZ', title: 'Pin bun?' }}) }}));",
            format!("file://{}", module.display()),
            serde_json::to_string(&titles).unwrap()
        );
        let out = std::process::Command::new("node")
            .args(["--input-type=module", "-e", &script])
            .output()
            .unwrap_or_else(|error| {
                panic!("this test runs ids.js in node, which is not on $PATH: {error}")
            });
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        for (title, slug) in titles.iter().zip(got["slugs"].as_array().unwrap()) {
            assert_eq!(
                slug.as_str().unwrap(),
                crate::output::slug(title),
                "{title:?}"
            );
        }
        assert_eq!(got["ids"], serde_json::json!(["UkLWZ", "UkLWZ", "UkLWZ"]));
        assert_eq!(got["page"], "/cases/UkLWZ-pin-bun");
    }

    /// The keys find what the views draw: the case's answer form, its send
    /// button, an approval's rows, the inbox's cards and the list of keys
    /// (IN-09). The keys themselves run in a browser.
    #[test]
    fn the_keys_find_what_the_page_draws() {
        let keys = text(served("keys.js").bytes);
        for want in [
            r#"document.querySelector('#case form.respond[data-write="answer"]')"#,
            "return form && form.getClientRects().length ? form : null;",
            r##"form.requestSubmit(form.querySelector("#respond-send"))"##,
            r#"form.querySelector("fieldset.row")"#,
            r##"document.querySelectorAll("#inbox a.card")"##,
        ] {
            assert!(keys.contains(want), "keys.js lacks {want}");
        }
        let case = text(served("case.js").bytes);
        for want in [
            r#"<div id="case">"#,
            r#"<form class="respond" data-write="answer""#,
            r#"id="respond-send""#,
            r#"<fieldset class="row""#,
        ] {
            assert!(case.contains(want), "case.js lacks {want}");
        }
        let lists = text(served("lists.js").bytes);
        assert!(
            lists.contains(r#"<div id="inbox">"#) && lists.contains(r#"["card", "#),
            "lists.js"
        );
        assert!(text(served("options.js").bytes).contains(r#"<dialog id="keys-dialog""#));
    }

    /// The options dialog is two columns, appearance then behaviour, when it
    /// is wide and one when it is not, never taller than the window, with only
    /// its groups scrolling between the header and the actions (IN-10). The
    /// groups are served in the order drawn, so tabbing follows it.
    #[test]
    fn the_options_dialog_is_in_columns_capped_to_the_window() {
        let options = text(served("options.js").bytes);
        let mut at = 0;
        for want in [
            r#"heading: "appearance""#,
            r#"name: "look""#,
            r#"name: "theme""#,
            r#"name: "face""#,
            r#"name: "size""#,
            r#"heading: "behaviour""#,
            r#"name: "links""#,
            r#"name: "notify""#,
        ] {
            let found = options
                .find(want)
                .unwrap_or_else(|| panic!("options.js lacks {want}"));
            assert!(found > at, "{want} is out of order");
            at = found;
        }
        let body = options.find(r#"<div class="options-body">"#).unwrap();
        assert!(options[body..].contains(
            r#"</div>
  <div class="actions">"#
        ));
        let css = text(served("style.css").bytes);
        for want in [
            "max-height: calc(100dvh - 2rem);",
            "container: options / inline-size;",
            "flex: 1 1 auto; min-height: 0; overflow-y: auto;",
            "@container options (width >= 40rem) { .options-body { grid-template-columns: 1fr 1fr; } }",
        ] {
            assert!(css.contains(want), "style.css lacks {want}");
        }
    }

    /// The page's one rule for links, run in Node on links.js as it is served
    /// (IN-02): an absolute address is a link unless its scheme is blocked,
    /// read as a browser reads it; a relative one is text.
    #[test]
    fn the_link_rule_links_absolute_addresses_but_blocked_schemes() {
        let cases = [
            ("https://example.com/a", Some("https://example.com/a")),
            ("http://example.com", Some("http://example.com/")),
            ("mailto:ryan@example.com", Some("mailto:ryan@example.com")),
            ("obsidian://open?vault=x", Some("obsidian://open?vault=x")),
            ("javascript:alert(1)", None),
            ("JaVa\tscript:alert(1)", None),
            (" \u{1}javascript:alert(1)", None),
            ("VBScript:msgbox", None),
            ("data:text/html,<b>x</b>", None),
            ("file:///etc/passwd", None),
            ("blob:https://example.com/1", None),
            ("filesystem:https://example.com/temporary/x", None),
            ("about:blank", None),
            ("src/main.rs", None),
            ("/cases/UkLWZ", None),
            ("#top", None),
            ("", None),
        ];
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("links.mjs");
        std::fs::write(&module, served("links.js").bytes).unwrap();
        let hrefs: Vec<&str> = cases.iter().map(|(href, _)| *href).collect();
        let script = format!(
            "import {{ link }} from {:?};\nconst hrefs = {};\n\
             console.log(JSON.stringify(hrefs.map((h) => {{ const u = link(h); return u ? u.href : null; }})));",
            format!("file://{}", module.display()),
            serde_json::to_string(&hrefs).unwrap()
        );
        let out = std::process::Command::new("node")
            .args(["--input-type=module", "-e", &script])
            .output()
            .unwrap_or_else(|error| {
                panic!("this test runs links.js in node, which is not on $PATH: {error}")
            });
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Vec<Option<String>> = serde_json::from_slice(&out.stdout).unwrap();
        for ((href, want), got) in cases.iter().zip(got) {
            assert_eq!(got.as_deref(), *want, "{href:?}");
        }
    }

    /// The case's properties, drawn in Node from props.js and the modules it
    /// imports as they are served (IN-02): a case shows each property it has
    /// and the keys its form takes, and leaves out each one it lacks.
    #[test]
    fn the_properties_show_what_a_case_has_and_omit_what_it_lacks() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "props.js",
            "html.js",
            "ids.js",
            "kinds.js",
            "thread.js",
            "time.js",
            "vendor/preact.module.js",
            "vendor/htm.module.js",
        ] {
            // Node has no import map: the libraries are named by their path.
            let js = text(served(path).bytes)
                .replace(r#"from "preact""#, r#"from "./vendor/preact.module.js""#)
                .replace(r#"from "htm""#, r#"from "./vendor/htm.module.js""#);
            let to = dir.path().join(path);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, js).unwrap();
        }
        let full = serde_json::json!({
            "id": "UkLWZ", "state": "open", "kind": "decision", "urgency": "blocking",
            "title": "Pin bun or float?", "options": ["pin", "float"],
            "worker": "search-api", "for": "sam", "labels": ["search", "deps"],
            "links": ["https://example.com/pr/1", "src/main.rs"],
            "opened_at": "2026-09-26T10:00:00Z", "updated_at": "2026-09-26T11:00:00Z",
            "events": [
                { "seq": 1, "author": "agent", "event": "open", "file": "1", "at": "2026-09-26T10:00:00Z", "data": {} },
                { "seq": 2, "author": "agent", "event": "amend", "file": "2", "at": "2026-09-26T11:00:00Z", "data": {} }
            ],
            "revision": 2, "amend_seq": 2, "last_seq": 2
        });
        let bare = serde_json::json!({
            "id": "xY3", "state": "parked", "kind": "fyi", "urgency": "whenever",
            "title": "Heads up", "labels": [], "links": [], "events": [], "revision": 1, "amend_seq": 0, "last_seq": 1
        });
        let closed = serde_json::json!({
            "id": "Qr7", "state": "closed", "kind": "stuck", "urgency": "today",
            "title": "Stuck", "opened_at": "2026-09-26T10:00:00Z", "updated_at": "2026-09-26T10:00:00Z",
            "events": [], "revision": 1, "amend_seq": 0, "last_seq": 1
        });
        let script = format!(
            "import {{ Properties }} from {:?};\nimport {{ html }} from {:?};\n\
             const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/\"/g, '&quot;');\n\
             const draw = (n) => n == null || typeof n === 'boolean' ? '' : Array.isArray(n) ? n.map(draw).join('')\n\
               : typeof n !== 'object' ? esc(n) : typeof n.type === 'function' ? draw(n.type(n.props))\n\
               : `<${{n.type}}${{Object.entries(n.props).filter(([k, v]) => k !== 'children' && v != null && v !== false).map(([k, v]) => ` ${{k}}=\"${{esc(v)}}\"`).join('')}}>${{draw(n.props.children)}}</${{n.type}}>`;\n\
             const now = Date.parse('2026-09-26T12:00:00Z');\n\
             console.log(JSON.stringify([{full}, {bare}, {closed}].map((c) => draw(html`<${{Properties}} c=${{c}} now=${{now}} />`))));",
            format!("file://{}", dir.path().join("props.js").display()),
            format!("file://{}", dir.path().join("html.js").display()),
        );
        let out = std::process::Command::new("node")
            .args(["--input-type=module", "-e", &script])
            .output()
            .unwrap_or_else(|error| {
                panic!("this test runs props.js in node, which is not on $PATH: {error}")
            });
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
        let [full, bare, closed] = got.as_slice() else {
            panic!("{got:?}")
        };

        for want in [
            r#"<aside class="props urgency-blocking" aria-label="properties">"#,
            r#"<dt>kind</dt><dd><span class="kind-icon kind-decision" aria-hidden="true"></span>decision</dd>"#,
            r#"<dd><span class="badge urgency">blocking</span></dd>"#,
            r#"<dd><span class="badge state">open</span></dd>"#,
            "<dt>worker</dt><dd>search-api</dd>",
            r#"<div class="prop-for named"><dt>for</dt><dd>sam</dd>"#,
            r#"<span class="tag">search</span><span class="tag">deps</span>"#,
            r#"<dt>opened</dt><dd><time datetime="2026-09-26T10:00:00Z""#,
            ">2h ago</time>",
            r#"<dt>updated</dt><dd><time datetime="2026-09-26T11:00:00Z""#,
            r#"<dt>amended</dt><dd><time datetime="2026-09-26T11:00:00Z""#,
            r#"<dt>id</dt><dd><code title="UkLWZ-pin-bun-or-float">case:UkLWZ</code></dd>"#,
            "<dt>choose</dt><dd><kbd>1</kbd>–<kbd>3</kbd></dd>",
            "<dt>send</dt>",
            "<dt>next, previous</dt>",
        ] {
            assert!(full.contains(want), "the full case lacks {want}: {full}");
        }
        // The case's links are under its body (case.js), not among its
        // properties.
        for absent in ["props-links", "https://example.com/pr/1", "src/main.rs"] {
            assert!(!full.contains(absent), "the full case has {absent}: {full}");
        }

        for want in [
            r#"<dt>kind</dt><dd><span class="kind-icon kind-fyi" aria-hidden="true"></span>fyi</dd>"#,
            r#"<span class="badge state parked">parked</span>"#,
            r#"<dt>id</dt><dd><code title="xY3-heads-up">case:xY3</code></dd>"#,
            "<dt>next, previous</dt>",
        ] {
            assert!(bare.contains(want), "the bare case lacks {want}: {bare}");
        }
        for absent in [
            "<dt>worker</dt>",
            "<dt>for</dt>",
            "<dt>labels</dt>",
            "<dt>opened</dt>",
            "<dt>updated</dt>",
            "<dt>amended</dt>",
            "<dt>choose</dt>",
            "<dt>send</dt>",
        ] {
            assert!(!bare.contains(absent), "the bare case has {absent}: {bare}");
        }
        // Updated the moment it was opened says nothing more; a closed case
        // takes no keys but the list's.
        assert!(closed.contains("<dt>opened</dt>"), "{closed}");
        for absent in ["<dt>updated</dt>", "<dt>choose</dt>", "<dt>send</dt>"] {
            assert!(
                !closed.contains(absent),
                "the closed case has {absent}: {closed}"
            );
        }
    }

    /// The page follows the store with one event stream per browser, held
    /// by the tab that holds the "cases-stream" lock, shown or hidden, or by
    /// a visible tab where there are no locks, passing each version to the
    /// other tabs (IN-06); it
    /// asks for notifications every five seconds (IN-08); and markdown
    /// drops raw HTML, and the text after a raw tag is text, and it checks
    /// a link where the browser would follow it (IN-02).
    #[test]
    fn the_page_follows_the_store_and_keeps_markdown_safe() {
        let app = text(served("app.js").bytes);
        for want in [
            "new EventSource(`/v1/changes${after}`)",
            r#"new BroadcastChannel("cases-store")"#,
            r#".request("cases-stream","#,
            "(channel && !locks && document.hidden)",
            r#""visibilitychange""#,
            "checkBuild();",
            r#"<div class="new-build-live" aria-live="polite">"#,
            r#"!input.closest("[data-preset]")"#,
        ] {
            assert!(app.contains(want), "app.js lacks {want}");
        }
        let options = text(served("options.js").bytes);
        for want in [
            "const POLL = 5000;",
            "api.notifications(",
            "new Notification(item.title, { body: item.body, tag: item.tag })",
        ] {
            assert!(options.contains(want), "options.js lacks {want}");
        }
        let markdown = text(served("markdown.js").bytes);
        for want in [
            "html() {\n      return \"\";",
            r#"if (token.type === "text") token.escaped = false;"#,
            "return link(attribute(href));",
            "if (url.origin !== location.origin)",
            r#"' target="_blank" rel="noopener noreferrer"'"#,
        ] {
            assert!(markdown.contains(want), "markdown.js lacks {want}");
        }
    }

    /// The page names the build it is, which the SPA compares with serve's,
    /// and its tag is of the page as served, so it changes with the build
    /// (IN-12).
    #[test]
    fn the_page_names_its_build() {
        let page = text(PAGE.as_slice());
        let meta = format!(r#"<meta name="cases-build" content="{}">"#, *BUILD);
        assert!(page.contains(&meta), "{page}");
        assert!(!page.contains("{build}"), "{page}");
        assert_eq!(
            *PAGE_TAG,
            format!("\"{:016x}\"", instance::fnv1a(PAGE.as_slice()))
        );
        assert_ne!(PAGE_TAG.as_str(), TAGS[FILES.len() + GZIPPED.len()]);
        let app = text(served("app.js").bytes);
        assert!(app.contains(r#"document.querySelector('meta[name="cases-build"]')"#));
    }

    /// The build changes with any file of the page, and the tags are each
    /// file's.
    #[test]
    fn the_build_names_the_version_and_the_files() {
        assert!(
            BUILD.starts_with(concat!(env!("CARGO_PKG_VERSION"), "+")),
            "{}",
            *BUILD
        );
        assert_eq!(TAGS.len(), FILES.len() + GZIPPED.len() + 1);
        let mut unique = TAGS.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), TAGS.len(), "two files share a tag");
    }
}
