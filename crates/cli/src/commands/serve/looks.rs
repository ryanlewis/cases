//! The inbox's looks: the token blocks at the top of `style.css`, one set per
//! look in light and dark, which the structure below them draws with. These
//! tests read the blocks as the browser would pick them and check that every
//! look defines the same tokens, and that the pairs of tokens drawn together
//! can be read: 4.5:1 for text on its ground, 3:1 for the edges and marks a
//! control or state is known by (WCAG 2.2, 1.4.3 and 1.4.11).
//!
//! A new look is checked by adding its blocks to `style.css` and its name to
//! the look option in `theme.ts`; nothing here names a look. A look passes
//! every pair: where one does not, its colours change, not the test.

use std::collections::BTreeMap;

const CSS: &str = include_str!("web/static/style.css");
const THEME: &str = include_str!("web/static/theme.ts");

/// The tokens every look defines, as its light block gives them. A dark
/// block redefines each colour among them that the light block gives as hex.
const TOKENS: &[&str] = &[
    // The palette.
    "--bg",
    "--bg-1",
    "--bg-2",
    "--fg",
    "--fg-1",
    "--fg-2",
    "--fg-3",
    "--rule",
    "--rule-strong",
    "--blocking",
    "--parked",
    // The options dialog's backdrop: always a dim towards black, in every
    // theme, so it darkens the page rather than veiling it. It sits behind
    // the dialog's own opaque surface, never under text, so it draws no
    // contrast pair.
    "--scrim",
    // The roles, which a look may point at its palette.
    "--surface",
    "--control",
    "--focus",
    "--chosen-bg",
    "--chosen-fg",
    "--primary-bg",
    "--primary-fg",
    "--on-blocking",
    "--followup-bg",
    "--followup-edge",
    // The hue of each kind of case, which a look may draw its glyph in.
    "--kind-decision",
    "--kind-approval",
    "--kind-signoff",
    "--kind-question",
    "--kind-stuck",
    "--kind-fyi",
    "--kind-message",
    "--kind-briefing",
    // A mark drawn on a kind's hue, such as daylight's glyph on its tile.
    "--on-kind",
    // The needs-you banner: its ground, edge and text.
    "--sun-bg",
    "--sun-edge",
    "--sun-fg",
    // A diff's added, removed and @@ lines.
    "--diff-add",
    "--diff-add-bg",
    "--diff-del",
    "--diff-del-bg",
    "--diff-hunk",
    // The faces.
    "--ui",
    "--mono",
    "--sans",
    "--serif",
    "--prose",
    // The radii.
    "--radius",
    "--radius-sm",
];

/// Text and the ground it is drawn on, which need 4.5:1.
const TEXT: &[(&str, &str)] = &[
    // The page: bodies, context, micro-labels, ages.
    ("--fg", "--bg"),
    ("--fg-1", "--bg"),
    ("--fg-2", "--bg"),
    ("--fg-3", "--bg"),
    // A card under the pointer, a chip or choice under it, a field, code.
    ("--fg", "--bg-1"),
    ("--fg-1", "--bg-1"),
    ("--fg-2", "--bg-1"),
    ("--fg-3", "--bg-1"),
    // The selected card.
    ("--fg", "--bg-2"),
    ("--fg-2", "--bg-2"),
    ("--fg-3", "--bg-2"),
    // The options and keys dialogs.
    ("--fg", "--surface"),
    ("--fg-2", "--surface"),
    // The current view, the chosen option, the form's action.
    ("--chosen-fg", "--chosen-bg"),
    ("--primary-fg", "--primary-bg"),
    // An agent's follow-up note: its text and its header.
    ("--fg", "--followup-bg"),
    ("--fg-2", "--followup-bg"),
    // The needs-you banner, and its link.
    ("--sun-fg", "--sun-bg"),
    // A diff's lines, in a code block.
    ("--diff-add", "--diff-add-bg"),
    ("--diff-del", "--diff-del-bg"),
    ("--diff-hunk", "--bg-1"),
    // The filled blocking badge, and the outline badges and the tally on the
    // grounds a card can have.
    ("--on-blocking", "--blocking"),
    ("--blocking", "--bg"),
    ("--blocking", "--bg-1"),
    ("--blocking", "--bg-2"),
    ("--parked", "--bg"),
    ("--parked", "--bg-1"),
    ("--parked", "--bg-2"),
];

/// Marks a control or a state is known by, and what they sit on, which need
/// 3:1. The rails and radio rings are the colours of text pairs above.
const UI: &[(&str, &str)] = &[
    // The focus ring, on each ground it is drawn over.
    ("--focus", "--bg"),
    ("--focus", "--bg-1"),
    ("--focus", "--bg-2"),
    ("--focus", "--surface"),
    // The edge of a field, a chip group, a button on a phone.
    ("--control", "--bg"),
    ("--control", "--surface"),
    // The form's action, and the current view or chosen option.
    ("--primary-bg", "--bg"),
    ("--chosen-bg", "--bg"),
    ("--chosen-bg", "--surface"),
    // The edge of an agent's follow-up note.
    ("--followup-edge", "--bg"),
    // A kind's glyph, on the page, a card under the pointer and the
    // selected card.
    ("--kind-decision", "--bg"),
    ("--kind-decision", "--bg-1"),
    ("--kind-decision", "--bg-2"),
    ("--kind-approval", "--bg"),
    ("--kind-approval", "--bg-1"),
    ("--kind-approval", "--bg-2"),
    ("--kind-signoff", "--bg"),
    ("--kind-signoff", "--bg-1"),
    ("--kind-signoff", "--bg-2"),
    ("--kind-question", "--bg"),
    ("--kind-question", "--bg-1"),
    ("--kind-question", "--bg-2"),
    ("--kind-stuck", "--bg"),
    ("--kind-stuck", "--bg-1"),
    ("--kind-stuck", "--bg-2"),
    ("--kind-fyi", "--bg"),
    ("--kind-fyi", "--bg-1"),
    ("--kind-fyi", "--bg-2"),
    ("--kind-message", "--bg"),
    ("--kind-message", "--bg-1"),
    ("--kind-message", "--bg-2"),
    ("--kind-briefing", "--bg"),
    ("--kind-briefing", "--bg-1"),
    ("--kind-briefing", "--bg-2"),
    // A mark on a kind's hue.
    ("--on-kind", "--kind-decision"),
    ("--on-kind", "--kind-approval"),
    ("--on-kind", "--kind-signoff"),
    ("--on-kind", "--kind-question"),
    ("--on-kind", "--kind-stuck"),
    ("--on-kind", "--kind-fyi"),
    ("--on-kind", "--kind-message"),
    ("--on-kind", "--kind-briefing"),
];

/// Where a block of tokens applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Variant {
    /// The look with no theme stored.
    Light,
    /// The look in the system's dark scheme, unless light is stored.
    SystemDark,
    /// The look with dark stored.
    StoredDark,
}

/// The look a selector names: `Some(name)`, or `None` for the default, the
/// look drawn when no `data-look` is set.
type Look = Option<String>;

type Tokens = BTreeMap<String, String>;

/// `css` without its comments.
fn uncomment(css: &str) -> String {
    let mut out = String::new();
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find("*/").expect("an unclosed comment");
        rest = &rest[at + end + 2..];
    }
    out.push_str(rest);
    out
}

/// The rules of `css` as (prelude, block) pairs, each block without its
/// braces, at the top level only.
fn rules(css: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let mut depth = 0;
        let mut close = None;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.expect("an unclosed block");
        // A statement at-rule, such as @import, ends at a semicolon.
        let prelude = rest[..open].rsplit(';').next().unwrap().trim();
        out.push((prelude, &rest[open + 1..close]));
        rest = &rest[close + 1..];
    }
    out
}

/// The look and variant one selector of a token block applies to.
fn variant(selector: &str, dark_media: bool) -> (Look, Variant) {
    let look = if let Some(at) = selector.find(r#"[data-look=""#) {
        let name = &selector[at + 12..];
        Some(name[..name.find('"').unwrap()].to_string())
    } else {
        assert!(
            selector.contains(":not([data-look])"),
            "{selector} names no look"
        );
        None
    };
    let variant = match (
        dark_media,
        selector.contains(r#":not([data-theme="light"])"#),
        selector.contains(r#"[data-theme="dark"]"#),
    ) {
        (false, false, false) => Variant::Light,
        (true, true, false) => Variant::SystemDark,
        (false, false, true) => Variant::StoredDark,
        _ => panic!("{selector} is not the form of a light or dark block"),
    };
    (look, variant)
}

/// The selectors of the list `prelude`, split at its commas but not at those
/// inside parentheses, such as in `:is(a, b)`.
fn selectors(prelude: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in prelude.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&prelude[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&prelude[start..]);
    out
}

/// Whether the rule with selectors `prelude` is a block of a look's tokens:
/// each selector is the root alone, `:root` with the look and the theme it
/// applies to. A rule that names a look and then an element under the root,
/// or a pseudo-element, is the look's own CSS for that element, not tokens.
fn tokens_block(prelude: &str) -> bool {
    if !prelude.contains("data-look") {
        return false;
    }
    let bare = |selector: &str| {
        let selector = selector.trim();
        selector.starts_with(":root")
            && !selector.contains("::")
            && !selector.contains(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~'))
    };
    let selectors = selectors(prelude);
    let root = selectors.iter().filter(|s| bare(s)).count();
    assert!(
        root == 0 || root == selectors.len(),
        "{prelude} mixes the root with elements under it"
    );
    root > 0
}

/// Every token block in `style.css`, by look and variant, with the
/// `color-scheme` each one sets.
fn blocks() -> BTreeMap<(Look, Variant), (Tokens, String)> {
    let css = uncomment(CSS);
    let mut found = BTreeMap::new();
    let mut take = |prelude: &str, block: &str, dark_media: bool| {
        if !tokens_block(prelude) {
            return;
        }
        let mut tokens = Tokens::new();
        let mut scheme = String::new();
        for declaration in block.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                assert!(declaration.trim().is_empty(), "{declaration:?}");
                continue;
            };
            let (name, value) = (name.trim(), value.trim().to_string());
            if name == "color-scheme" {
                scheme = value;
            } else {
                assert!(
                    name.starts_with("--"),
                    "{prelude} sets {name}: a look sets only tokens"
                );
                assert!(
                    tokens.insert(name.to_string(), value).is_none(),
                    "{prelude} sets {name} twice"
                );
            }
        }
        for selector in selectors(prelude) {
            let key = variant(selector.trim(), dark_media);
            let had = found.insert(key.clone(), (tokens.clone(), scheme.clone()));
            assert!(had.is_none(), "two blocks for {key:?}");
        }
    };
    for (prelude, block) in rules(&css) {
        if prelude.starts_with("@media") {
            let dark = prelude == "@media (prefers-color-scheme: dark)";
            for (inner, block) in rules(block) {
                assert!(
                    dark || !tokens_block(inner),
                    "a token block under {prelude}"
                );
                take(inner, block, dark);
            }
        } else {
            take(prelude, block, false);
        }
    }
    found
}

/// The looks the look option offers, the default first, from `theme.ts`.
fn offered() -> Vec<String> {
    let line = THEME
        .lines()
        .find(|line| line.contains(r#"{ name: "look", values: ["#))
        .expect("theme.ts has no look option");
    let values = &line[line.find('[').unwrap() + 1..line.find(']').unwrap()];
    values
        .split(',')
        .map(|v| v.trim().trim_matches('"').to_string())
        .collect()
}

/// Each look's tokens in light and dark: its light block, and that with its
/// dark block over it, after checking the blocks are whole and agree.
fn looks() -> Vec<(String, Tokens, Tokens)> {
    let blocks = blocks();
    let offered = offered();
    let mut drawn: Vec<String> = blocks.keys().filter_map(|(look, _)| look.clone()).collect();
    drawn.dedup();
    let mut want = offered.clone();
    want.sort();
    assert_eq!(
        drawn, want,
        "the looks style.css draws are not those theme.ts offers"
    );

    let mut out = Vec::new();
    for look in &offered {
        let block = |variant| {
            blocks
                .get(&(Some(look.clone()), variant))
                .unwrap_or_else(|| panic!("{look} has no {variant:?} block"))
        };
        let (light, scheme) = block(Variant::Light);
        assert_eq!(
            scheme, "light",
            "{look}'s light block sets color-scheme {scheme:?}"
        );
        let (dark, scheme) = block(Variant::SystemDark);
        assert_eq!(
            scheme, "dark",
            "{look}'s dark block sets color-scheme {scheme:?}"
        );
        assert_eq!(
            block(Variant::StoredDark),
            block(Variant::SystemDark),
            "{look}'s two dark blocks differ"
        );
        let names: Vec<&str> = light.keys().map(String::as_str).collect();
        let mut want = TOKENS.to_vec();
        want.sort_unstable();
        assert_eq!(
            names, want,
            "{look}'s tokens are not those every look defines"
        );
        for (name, value) in light {
            if value.starts_with('#') {
                assert!(dark.contains_key(name), "{look} has no dark {name}");
            }
        }
        for name in dark.keys() {
            assert!(
                light.contains_key(name),
                "{look}'s dark block sets {name}, which its light block does not"
            );
        }
        let mut dark_tokens = light.clone();
        dark_tokens.extend(dark.clone());
        out.push((look.clone(), light.clone(), dark_tokens));
    }

    // With no look stored, data-look is not set, and the default is drawn.
    let default = &offered[0];
    for variant in [Variant::Light, Variant::SystemDark, Variant::StoredDark] {
        assert_eq!(
            blocks.get(&(None, variant)),
            blocks.get(&(Some(default.clone()), variant)),
            "with no data-look, the {variant:?} tokens are not {default}'s"
        );
    }
    out
}

/// The colour of token `name` as sRGB, following `var()`. Colours are hex,
/// which is all the looks use and all this reads.
fn colour(tokens: &Tokens, name: &str) -> [u8; 3] {
    let mut name = name;
    for _ in 0..8 {
        let value = tokens
            .get(name)
            .unwrap_or_else(|| panic!("no token {name}"));
        if let Some(inner) = value.strip_prefix("var(").and_then(|v| v.strip_suffix(')')) {
            name = inner.trim();
            continue;
        }
        let hex = value
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("{name} is {value}: a colour token is hex or var()"));
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| u8::from_str_radix(&c.to_string(), 16))
            .collect::<Result<_, _>>()
            .unwrap_or_else(|_| panic!("{name} is {value}: not hex"));
        return match digits[..] {
            [r, g, b] => [r * 17, g * 17, b * 17],
            [r1, r2, g1, g2, b1, b2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2],
            _ => panic!("{name} is {value}: a colour token is opaque, #rgb or #rrggbb"),
        };
    }
    panic!("{name} refers round in a circle");
}

/// WCAG's relative luminance of an sRGB colour.
fn luminance(rgb: [u8; 3]) -> f64 {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

/// WCAG's contrast ratio of two colours, 1 to 21.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn contrast_is_worked_out_as_wcag_does() {
    assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    assert!((contrast([255, 255, 255], [255, 255, 255]) - 1.0).abs() < 1e-9);
    // #767676 on white is the usual least grey for 4.5:1.
    let grey = contrast([0x76, 0x76, 0x76], [255, 255, 255]);
    assert!((4.5..4.6).contains(&grey), "{grey}");
    let tokens: Tokens = [("--a", "#abc"), ("--b", "var(--a)"), ("--c", "#aabbcc")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(colour(&tokens, "--b"), colour(&tokens, "--c"));
}

/// A token block is a rule on the root alone; a look's rules for elements
/// under the root, or its pseudo-elements, are not tokens.
#[test]
fn a_token_block_is_the_root_alone() {
    assert!(tokens_block(r#":root[data-look="keel"]"#));
    assert!(tokens_block(
        r#":root:not([data-look])[data-theme="dark"], :root[data-look="mono"][data-theme="dark"]"#
    ));
    assert!(tokens_block(
        r#":root[data-look="keel"]:not([data-theme="light"])"#
    ));
    assert!(!tokens_block(r#":root[data-face="sans"]"#));
    assert!(!tokens_block(r#":root[data-look="keel"] .card"#));
    assert!(!tokens_block(r#":root[data-look="keel"] :is(.a, .b)"#));
    assert!(!tokens_block(r#":root[data-look="keel"]>body"#));
    assert!(!tokens_block(
        r#":root:is(:not([data-look]), [data-look="keel"]) .card"#
    ));
    assert!(!tokens_block(
        r#":root[data-look="keel"]::view-transition-old(root)"#
    ));
}

#[test]
#[should_panic(expected = "mixes the root with elements under it")]
fn a_token_block_does_not_mix_in_elements() {
    tokens_block(r#":root[data-look="keel"], :root[data-look="keel"] .card"#);
}

/// Every look, light and dark, is whole, and each pair drawn together reads.
#[test]
fn every_look_can_be_read() {
    let mut failed = Vec::new();
    for (look, light, dark) in looks() {
        for (theme, tokens) in [("light", &light), ("dark", &dark)] {
            for (pairs, least) in [(TEXT, 4.5), (UI, 3.0)] {
                for &(fg, bg) in pairs {
                    let ratio = contrast(colour(tokens, fg), colour(tokens, bg));
                    if ratio < least {
                        failed.push(format!(
                            "{look} {theme}: {fg} on {bg} is {ratio:.2}:1, under {least}:1"
                        ));
                    }
                }
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
