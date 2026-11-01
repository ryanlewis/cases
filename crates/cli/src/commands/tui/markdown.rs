//! Markdown drawn for a terminal (TU-03): headings, lists with a hanging
//! indent, quotes, fenced code (a `diff` block coloured by line), tables as
//! aligned text, and bold, emphasis, code and links within a line, a link
//! showing its URL. Enough to read a body the way an agent writes one, not a
//! full `CommonMark` renderer. Raw HTML is left as text.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::view::{ACCENT, DIM};
use super::wrap::Para;

/// The lines of `text`, styled.
pub(crate) fn render(text: &str) -> Vec<Para> {
    let mut lines = Vec::new();
    let mut fence: Option<String> = None;
    let mut table: Vec<&str> = Vec::new();
    for raw in text.trim_end().lines() {
        let trimmed = raw.trim_start();
        if fence.is_none() && trimmed.starts_with('|') {
            table.push(trimmed);
            continue;
        }
        if !table.is_empty() {
            lines.extend(render_table(&table));
            table.clear();
        }
        if let Some(rest) = trimmed.strip_prefix("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(rest.trim().to_string()),
            };
            continue;
        }
        if let Some(lang) = &fence {
            lines.push(code_line(raw, lang));
            continue;
        }
        lines.push(block_line(raw));
    }
    lines.extend(render_table(&table));
    lines
}

/// A table's rows as aligned text: the header bold, the row of dashes under
/// it drawn as a rule.
fn render_table(rows: &[&str]) -> Vec<Para> {
    let cells: Vec<Vec<&str>> = rows
        .iter()
        .map(|row| {
            let row = row.trim().trim_start_matches('|');
            let row = row.strip_suffix('|').unwrap_or(row);
            row.split('|').map(str::trim).collect()
        })
        .collect();
    let rule = |row: &[&str]| {
        row.iter()
            .all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':')))
    };
    let mut widths: Vec<usize> = Vec::new();
    for row in cells.iter().filter(|row| !rule(row)) {
        for (at, cell) in row.iter().enumerate() {
            let width = inline(cell).iter().map(Span::width).sum();
            match widths.get_mut(at) {
                Some(w) => *w = (*w).max(width),
                None => widths.push(width),
            }
        }
    }
    let mut out = Vec::new();
    for (n, row) in cells.iter().enumerate() {
        if rule(row) {
            let line = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─");
            out.push(Para::plain(vec![Span::styled(line, Style::new().fg(DIM))]));
            continue;
        }
        let mut spans = Vec::new();
        for (at, width) in widths.iter().enumerate() {
            if at > 0 {
                spans.push(Span::styled(" │ ", Style::new().fg(DIM)));
            }
            let cell = inline(row.get(at).copied().unwrap_or(""));
            let used: usize = cell.iter().map(Span::width).sum();
            if n == 0 {
                spans.extend(
                    cell.into_iter()
                        .map(|s| s.patch_style(Style::new().add_modifier(Modifier::BOLD))),
                );
            } else {
                spans.extend(cell);
            }
            spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
        }
        out.push(Para::plain(spans));
    }
    out
}

fn code_line(raw: &str, lang: &str) -> Para {
    let style = if lang == "diff" {
        match raw.chars().next() {
            Some('+') => Style::new().fg(Color::Green),
            Some('-') => Style::new().fg(Color::Red),
            Some('@') => Style::new().fg(Color::Cyan),
            _ => Style::new().fg(DIM),
        }
    } else {
        Style::new().fg(Color::Yellow)
    };
    let bar = || vec![Span::styled("  │ ", Style::new().fg(DIM))];
    Para {
        hit: None,
        first: bar(),
        rest: bar(),
        spans: vec![Span::styled(raw.to_string(), style)],
    }
}

fn block_line(raw: &str) -> Para {
    let trimmed = raw.trim_start();
    let indent = columns(&raw[..raw.len() - trimmed.len()]);
    if let Some((level, heading)) = heading(trimmed) {
        let style = if level <= 2 {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        };
        return Para::plain(vec![Span::styled(heading.to_string(), style)]);
    }
    if let Some(rest) = trimmed.strip_prefix("> ") {
        let bar = || vec![Span::styled("▎ ", Style::new().fg(DIM))];
        return Para {
            hit: None,
            first: bar(),
            rest: bar(),
            spans: inline(rest)
                .into_iter()
                .map(|s| s.patch_style(Style::new().fg(DIM).add_modifier(Modifier::ITALIC)))
                .collect(),
        };
    }
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return Para::hanging(
            vec![
                Span::raw(" ".repeat(indent)),
                Span::styled("• ", Style::new().fg(ACCENT)),
            ],
            inline(rest),
        );
    }
    if let Some(dot) = trimmed.find(". ")
        && dot > 0
        && dot <= 3
        && trimmed[..dot].chars().all(|c| c.is_ascii_digit())
    {
        return Para::hanging(
            vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(trimmed[..=dot].to_string() + " ", Style::new().fg(ACCENT)),
            ],
            inline(&trimmed[dot + 2..]),
        );
    }
    Para::plain(inline(raw))
}

/// How many columns leading white space takes: a tab runs to the next
/// multiple of four, as in `CommonMark`.
fn columns(space: &str) -> usize {
    space.chars().fold(0, |at, c| match c {
        '\t' => at / 4 * 4 + 4,
        _ => at + 1,
    })
}

/// Where the `)` that ends a link's URL is: the first one outside a pair of
/// brackets in it, or, when they do not balance before white space, the
/// first one, as the web inbox has it.
fn url_end(rest: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(at),
            ')' => depth -= 1,
            c if c.is_whitespace() => break,
            _ => {}
        }
    }
    rest.find(')')
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) {
        line[level..].strip_prefix(' ').map(|rest| (level, rest))
    } else {
        None
    }
}

/// Spans for bold (`**x**`), emphasis (`*x*`), code (`` `x` ``) and links
/// (`[x](url)`, shown as the text and then the URL).
pub(crate) fn inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>| {
        if !plain.is_empty() {
            spans.push(Span::raw(std::mem::take(plain)));
        }
    };
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut plain, &mut spans);
            spans.push(Span::styled(
                after[..end].to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            flush(&mut plain, &mut spans);
            spans.push(Span::styled(
                after[..end].to_string(),
                Style::new().fg(Color::Yellow),
            ));
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = url_end(&after[close + 2..])
        {
            flush(&mut plain, &mut spans);
            let text = &after[..close];
            let url = &after[close + 2..close + 2 + end];
            spans.push(Span::styled(
                text.to_string(),
                Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED),
            ));
            if url != text {
                spans.push(Span::styled(format!(" ({url})"), Style::new().fg(DIM)));
            }
            rest = &after[close + 2 + end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('*')
            && after.starts_with(|c: char| !c.is_whitespace() && c != '*')
            && let Some(end) = after.find('*')
        {
            flush(&mut plain, &mut spans);
            spans.push(Span::styled(
                after[..end].to_string(),
                Style::new().add_modifier(Modifier::ITALIC),
            ));
            rest = &after[end + 1..];
            continue;
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut plain, &mut spans);
    spans
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

    use super::{inline, render};
    use crate::commands::tui::view::{ACCENT, DIM};
    use crate::commands::tui::wrap::Para;

    fn styled(spans: &[Span<'static>]) -> Vec<(String, Style)> {
        spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    fn text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Each paragraph as the text of its first line: what goes before it,
    /// then its text.
    fn lines(paras: &[Para]) -> Vec<String> {
        paras
            .iter()
            .map(|p| text(&p.first) + &text(&p.spans))
            .collect()
    }

    fn s(text: &str, style: Style) -> (String, Style) {
        (text.to_string(), style)
    }

    #[test]
    fn headings_one_and_two_are_accented_and_the_rest_bold() {
        let paras = render("# One\n## Two\n### Three\n###### Six\n####### Seven\n#None");
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let accent = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
        assert_eq!(styled(&paras[0].spans), [s("One", accent)]);
        assert_eq!(styled(&paras[1].spans), [s("Two", accent)]);
        assert_eq!(styled(&paras[2].spans), [s("Three", bold)]);
        assert_eq!(styled(&paras[3].spans), [s("Six", bold)]);
        // Seven hashes, or none of them followed by a space, is not a heading.
        assert_eq!(styled(&paras[4].spans), [s("####### Seven", Style::new())]);
        assert_eq!(styled(&paras[5].spans), [s("#None", Style::new())]);
        assert!(
            paras
                .iter()
                .all(|p| p.first.is_empty() && p.rest.is_empty())
        );
    }

    #[test]
    fn a_quote_has_a_bar_and_is_dim_and_italic() {
        let paras = render("> said **so**");
        let bar = Style::new().fg(DIM);
        let quote = Style::new().fg(DIM).add_modifier(Modifier::ITALIC);
        assert_eq!(styled(&paras[0].first), [s("▎ ", bar)]);
        assert_eq!(styled(&paras[0].rest), [s("▎ ", bar)]);
        assert_eq!(
            styled(&paras[0].spans),
            [
                s("said ", quote),
                s("so", quote.add_modifier(Modifier::BOLD)),
            ]
        );
    }

    #[test]
    fn lists_hang_under_their_bullet_or_number() {
        let paras = render("- dash\n  * star\n12. twelve\n1234. long\na. letter\n. dot");
        let marker = Style::new().fg(ACCENT);
        assert_eq!(
            styled(&paras[0].first),
            [s("", Style::new()), s("• ", marker)]
        );
        assert_eq!(text(&paras[0].rest), "  ");
        assert_eq!(text(&paras[0].spans), "dash");
        assert_eq!(
            styled(&paras[1].first),
            [s("  ", Style::new()), s("• ", marker)]
        );
        assert_eq!(text(&paras[1].rest), "    ");
        assert_eq!(text(&paras[1].spans), "star");
        assert_eq!(
            styled(&paras[2].first),
            [s("", Style::new()), s("12. ", marker)]
        );
        assert_eq!(text(&paras[2].rest), "    ");
        assert_eq!(text(&paras[2].spans), "twelve");
        assert_eq!(lines(&render("1. one")), ["1. one"]);
        // More than three digits, a letter, or no number is not a list.
        assert_eq!(lines(&paras[3..]), ["1234. long", "a. letter", ". dot"]);
        assert!(paras[3..].iter().all(|p| p.first.is_empty()));
    }

    #[test]
    fn a_diff_block_is_coloured_by_line() {
        let paras = render("```diff\n+added\n-removed\n@@ hunk\n same\n\n```\nafter");
        let bar = [s("  │ ", Style::new().fg(DIM))];
        for para in &paras[..5] {
            assert_eq!(styled(&para.first), bar);
            assert_eq!(styled(&para.rest), bar);
        }
        let spans: Vec<_> = paras[..5].iter().map(|p| styled(&p.spans)).collect();
        assert_eq!(
            spans,
            [
                vec![s("+added", Style::new().fg(Color::Green))],
                vec![s("-removed", Style::new().fg(Color::Red))],
                vec![s("@@ hunk", Style::new().fg(Color::Cyan))],
                vec![s(" same", Style::new().fg(DIM))],
                vec![s("", Style::new().fg(DIM))],
            ]
        );
        assert_eq!(paras.len(), 6);
        assert_eq!(styled(&paras[5].spans), [s("after", Style::new())]);
        assert!(paras[5].first.is_empty());
    }

    #[test]
    fn other_code_is_yellow_and_kept_as_written() {
        // A table row, a heading and markup inside a fence are code, and the
        // indented fence closes it.
        let paras = render("```rust\n  let x = **1**;\n| a |\n# no\n+1\n  ```\n**b**");
        let code = Style::new().fg(Color::Yellow);
        let spans: Vec<_> = paras[..4].iter().map(|p| styled(&p.spans)).collect();
        assert_eq!(
            spans,
            [
                vec![s("  let x = **1**;", code)],
                vec![s("| a |", code)],
                vec![s("# no", code)],
                vec![s("+1", code)],
            ]
        );
        assert_eq!(paras.len(), 5);
        assert_eq!(
            styled(&paras[4].spans),
            [s("b", Style::new().add_modifier(Modifier::BOLD))]
        );
    }

    #[test]
    fn a_fence_with_no_language_is_code_and_an_open_fence_runs_to_the_end() {
        let paras = render("```\n-x\n");
        assert_eq!(paras.len(), 1);
        assert_eq!(
            styled(&paras[0].spans),
            [s("-x", Style::new().fg(Color::Yellow))]
        );
    }

    #[test]
    fn a_table_ends_at_the_first_line_that_is_not_a_row() {
        let paras = render("| a | b |\n|---|:-:|\n| 1 | 22 |\n| x |\nafter");
        assert_eq!(
            lines(&paras),
            ["a │ b ", "──┼───", "1 │ 22", "x │   ", "after"]
        );
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let sep = Style::new().fg(DIM);
        assert_eq!(
            styled(&paras[0].spans),
            [
                s("a", bold),
                s("", Style::new()),
                s(" │ ", sep),
                s("b", bold),
                s(" ", Style::new()),
            ]
        );
        assert_eq!(styled(&paras[1].spans), [s("──┼───", sep)]);
        // A missing cell is padded to its column.
        assert_eq!(
            styled(&paras[3].spans),
            [
                s("x", Style::new()),
                s("", Style::new()),
                s(" │ ", sep),
                s("  ", Style::new()),
            ]
        );
    }

    #[test]
    fn a_table_cell_is_as_wide_as_its_text_not_its_markup() {
        let paras = render("| **bold** | `c` |\n| x | yy |");
        assert_eq!(lines(&paras), ["bold │ c ", "x    │ yy"]);
    }

    #[test]
    fn inline_bold_code_and_emphasis() {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let italic = Style::new().add_modifier(Modifier::ITALIC);
        let code = Style::new().fg(Color::Yellow);
        assert_eq!(
            styled(&inline("a **b** `c` *d* é")),
            [
                s("a ", Style::new()),
                s("b", bold),
                s(" ", Style::new()),
                s("c", code),
                s(" ", Style::new()),
                s("d", italic),
                s(" é", Style::new()),
            ]
        );
    }

    #[test]
    fn markup_that_does_not_close_is_text() {
        for text in [
            "**open",
            "`open",
            "*open",
            "* not emphasis*",
            "**",
            "[a](b",
            "[a] (b)",
        ] {
            assert_eq!(styled(&inline(text)), [s(text, Style::new())], "{text}");
        }
        assert!(inline("").is_empty());
    }

    #[test]
    fn a_link_shows_its_url_unless_the_text_is_the_url() {
        let link = Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED);
        let url = Style::new().fg(DIM);
        assert_eq!(
            styled(&inline("see [the docs](https://x.test/d) now")),
            [
                s("see ", Style::new()),
                s("the docs", link),
                s(" (https://x.test/d)", url),
                s(" now", Style::new()),
            ]
        );
        assert_eq!(
            styled(&inline("[https://x.test](https://x.test)")),
            [s("https://x.test", link)]
        );
    }

    #[test]
    fn a_link_url_keeps_its_balanced_brackets() {
        let link = Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED);
        let url = Style::new().fg(DIM);
        assert_eq!(
            styled(&inline("[Rust](https://w.test/Rust_(language)) ok")),
            [
                s("Rust", link),
                s(" (https://w.test/Rust_(language))", url),
                s(" ok", Style::new()),
            ]
        );
        assert_eq!(
            styled(&inline("[a](b(c(d))e)")),
            [s("a", link), s(" (b(c(d))e)", url)]
        );
        // A link in brackets ends at its own.
        assert_eq!(
            styled(&inline("([a](b))")),
            [
                s("(", Style::new()),
                s("a", link),
                s(" (b)", url),
                s(")", Style::new()),
            ]
        );
        // Unbalanced, it ends at the first, as the web inbox has it; a URL
        // has no white space, so the scan stops there.
        assert_eq!(
            styled(&inline("[a](b(c) d")),
            [s("a", link), s(" (b(c)", url), s(" d", Style::new())]
        );
        assert_eq!(
            styled(&inline("(see [a](b(c) d)")),
            [
                s("(see ", Style::new()),
                s("a", link),
                s(" (b(c)", url),
                s(" d)", Style::new()),
            ]
        );
    }

    #[test]
    fn a_tab_in_a_list_indent_is_four_columns() {
        let paras = render("\t- tab\n  \t- two and tab\n \t\t1. deep");
        assert_eq!(
            lines(&paras),
            ["    • tab", "    • two and tab", "        1. deep"]
        );
        assert_eq!(text(&paras[0].rest), "      ");
        assert_eq!(text(&paras[2].rest), "           ");
    }
}
