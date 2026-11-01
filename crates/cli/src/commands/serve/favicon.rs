//! `GET /favicon.svg?n=N&blocking=1`: the tab icon for a count (IN-01).
//!
//! The server draws it from its query alone, so the policy needs no `data:`
//! or `blob:` images and the page no canvas. With nothing waiting it is the
//! mark, Tray (`design-pack/brand/tray/favicon.svg`): a blue tile with a tray
//! line cut through it and a sun above, in the dark scheme's blue when the
//! system is dark. With cases waiting a badge with the count, `9+` above
//! nine, is cut into the mark's lower right: ink, or paper on a dark system,
//! and red, the one hue the page gives blocking, with a blocking case among
//! them.

use axum::extract::RawQuery;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use super::app::Refusal;

const OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">"#;

/// The colours, light and then dark: the tile's blue, the sun, and the
/// badge's ink and the paper its count is written in.
const STYLE: &str = concat!(
    "<style>.accent{color:#1c62d4}.sun{color:#f8c73a}.badge{fill:#1d1b17}.count{fill:#fffdf8}",
    "@media (prefers-color-scheme:dark){.accent{color:#6ea8ff}.badge{fill:#ecebe8}.count{fill:#232428}}</style>",
);

/// The mark: the tile, with the tray line cut out of it, and the sun.
const MARK: &str = concat!(
    r##"<mask id="tray" maskUnits="userSpaceOnUse" x="0" y="0" width="32" height="32"><rect width="32" height="32" fill="#fff"/><path d="M1 18.5h8.2l2.3 4h9l2.3-4H31" fill="none" stroke="#000" stroke-width="3.2" stroke-linejoin="round"/></mask>"##,
    r#"<g mask="url(#tray)"><rect class="accent" x="2" y="2" width="28" height="28" rx="8" fill="currentColor"/></g>"#,
    r#"<circle class="sun" cx="20.5" cy="10.5" r="4" fill="currentColor"/>"#,
);

/// The ring cut out of the mark around the badge, so the badge stands clear
/// of what is behind it.
const RING: &str = r##"<mask id="ring" maskUnits="userSpaceOnUse" x="0" y="0" width="32" height="32"><rect width="32" height="32" fill="#fff"/><circle cx="22.5" cy="22.5" r="11" fill="#000"/></mask>"##;

/// A blocking badge and its count: red, with light text in either scheme.
const BLOCKING_BADGE: &str = r##"fill="#a11b2b""##;
const BLOCKING_COUNT: &str = r##"fill="#fbfbfa""##;

/// Draws the icon for the count the query gives. Each of `n` and `blocking`
/// is read from its first value, and an empty one is none, as the Go inbox
/// read them.
pub(crate) async fn icon(RawQuery(query): RawQuery) -> Result<Response, Refusal> {
    let query = query.unwrap_or_default();
    let pairs: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
    let first = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_ref())
            .filter(|value| !value.is_empty())
    };
    let n: u64 = match first("n") {
        Some(value) => value
            .parse()
            .map_err(|_| Refusal::bad("n is not a count"))?,
        None => 0,
    };
    let blocking = match first("blocking") {
        Some("1") => true,
        Some(_) => return Err(Refusal::bad("blocking is not 1")),
        None => false,
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            // The icon depends only on its query, so a browser may keep it.
            (header::CACHE_CONTROL, "max-age=3600"),
        ],
        draw(n, blocking),
    )
        .into_response())
}

/// The icon for `n` open cases, red when one of them is `blocking`.
fn draw(n: u64, blocking: bool) -> String {
    if n == 0 {
        return format!("{OPEN}{STYLE}{MARK}</svg>");
    }
    let (text, size, y) = if n > 9 {
        ("9+".to_owned(), 11, "26.4")
    } else {
        (n.to_string(), 14, "27.5")
    };
    let (badge, count) = if blocking {
        (BLOCKING_BADGE, BLOCKING_COUNT)
    } else {
        (r#"class="badge""#, r#"class="count""#)
    };
    format!(
        concat!(
            "{open}{style}{ring}",
            r#"<g mask="url(#ring)">{mark}</g>"#,
            r#"<circle cx="22.5" cy="22.5" r="9" {badge}/>"#,
            r#"<text x="22.5" y="{y}" text-anchor="middle" font-family="system-ui, -apple-system, 'Segoe UI', sans-serif" font-size="{size}" font-weight="700" {count}>{text}</text>"#,
            "</svg>",
        ),
        open = OPEN,
        style = STYLE,
        ring = RING,
        mark = MARK,
        badge = badge,
        y = y,
        size = size,
        count = count,
        text = text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_draws_the_count() {
        let mark = draw(0, false);
        assert!(
            mark.contains(r#"mask="url(#tray)""#)
                && mark.contains(r#"class="sun""#)
                && !mark.contains("<text")
                && !mark.contains("url(#ring)"),
            "{mark}"
        );
        assert_eq!(draw(0, true), mark, "no count, no red");
        let three = draw(3, false);
        assert!(
            three.contains(r#"mask="url(#ring)""#)
                && three.contains(r#"mask="url(#tray)""#)
                && three.contains(r#"r="9" class="badge""#)
                && three.contains(r#"font-size="14""#)
                && three.contains(r#"class="count">3</text>"#)
                && !three.contains(BLOCKING_BADGE),
            "{three}"
        );
        let many = draw(12, false);
        assert!(
            many.contains(">9+</text>")
                && many.contains(r#"font-size="11""#)
                && !many.contains("12"),
            "{many}"
        );
        let red = draw(1, true);
        assert!(
            red.contains(r##"r="9" fill="#a11b2b""##) && red.contains(">1</text>"),
            "{red}"
        );
    }
}
