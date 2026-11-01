//! Word wrapping with a hanging indent: a bullet's or an option's later
//! lines start under its text, not under the bullet. Text is cleaned of
//! control characters on the way (TU-01).

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::app::Hit;
use super::safe;

/// One logical line: what goes before its first line, what goes before each
/// later one, and its text.
#[derive(Debug, Default)]
pub(crate) struct Para {
    pub(crate) first: Vec<Span<'static>>,
    pub(crate) rest: Vec<Span<'static>>,
    pub(crate) spans: Vec<Span<'static>>,
    /// What a click on any of its lines does.
    pub(crate) hit: Option<Hit>,
}

impl Para {
    pub(crate) fn plain(spans: Vec<Span<'static>>) -> Para {
        Para {
            hit: None,
            spans,
            ..Para::default()
        }
    }

    /// A paragraph whose later lines are indented as far as `first` is wide.
    pub(crate) fn hanging(first: Vec<Span<'static>>, spans: Vec<Span<'static>>) -> Para {
        let width: usize = first.iter().map(Span::width).sum();
        Para {
            hit: None,
            first,
            rest: vec![Span::raw(" ".repeat(width))],
            spans,
        }
    }

    pub(crate) fn blank() -> Para {
        Para::default()
    }

    /// The paragraph, with a click on it doing `hit`.
    pub(crate) fn on(self, hit: Hit) -> Para {
        Para {
            hit: Some(hit),
            ..self
        }
    }
}

/// The paragraphs as lines no wider than `width`.
pub(crate) fn wrap(paras: Vec<Para>, width: usize) -> Vec<Line<'static>> {
    wrap_hits(paras, width).0
}

/// The paragraphs as lines no wider than `width`, and what a click on each
/// line does.
pub(crate) fn wrap_hits(paras: Vec<Para>, width: usize) -> (Vec<Line<'static>>, Vec<Option<Hit>>) {
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    for para in paras {
        wrap_one(&para, width, &mut lines);
        hits.resize(lines.len(), para.hit);
    }
    (lines, hits)
}

fn wrap_one(para: &Para, width: usize, out: &mut Vec<Line<'static>>) {
    // Words with their style; a word keeps the white space before it.
    let mut words: Vec<(String, Style)> = Vec::new();
    for span in &para.spans {
        let mut word = String::new();
        let mut in_space = false;
        for c in safe::clean(&span.content).chars() {
            if c == ' ' && !in_space && !word.is_empty() {
                words.push((std::mem::take(&mut word), span.style));
            }
            in_space = c == ' ';
            word.push(c);
        }
        if !word.is_empty() {
            words.push((word, span.style));
        }
    }

    let mut line: Vec<Span<'static>> = para.first.clone();
    let mut used: usize = line.iter().map(Span::width).sum();
    let start = used;
    for (word, style) in words {
        let w = Span::raw(word.as_str()).width();
        if used + w > width && used > start {
            out.push(Line::from(std::mem::take(&mut line)));
            line.clone_from(&para.rest);
            used = line.iter().map(Span::width).sum();
            let trimmed = word.trim_start().to_string();
            used += Span::raw(trimmed.as_str()).width();
            push(&mut line, trimmed, style);
            continue;
        }
        used += w;
        push(&mut line, word, style);
    }
    out.push(Line::from(line));
}

/// Adds `text` to the line, joined to the last span when they share a style.
fn push(line: &mut Vec<Span<'static>>, text: String, style: Style) {
    if let Some(last) = line.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(&text);
        return;
    }
    line.push(Span::styled(text, style));
}
