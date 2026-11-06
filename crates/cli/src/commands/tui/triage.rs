//! SPIKE (triage-spike#1), not for merging: the one-case screen (TU-11)
//! drawn as a triage card. The case fills the screen and its choices are
//! large tiles: a click or the tile's number picks it. A tile either sends
//! the answer at once, through the form, or opens the form for what it
//! needs typed. On only when `CASES_TUI_TRIAGE=1`.

use cases_core::{Case, Kind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::app::{App, Hit};
use super::form::Act;
use super::markdown;
use super::view::{ACCENT, DIM};
use super::wrap::{self, Para};

/// What a tile does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tile {
    /// A decision's option, by its number: sends it.
    Option(i64),
    /// A decision's "something else": opens the note.
    Other,
    /// A signoff's or a briefing's accept: sends it.
    Accept,
    /// A signoff's ask for changes: opens the note.
    Changes,
    /// A briefing's send back: opens the note.
    SendBack,
    /// A briefing's accept with left-out items to file: opens the form.
    File,
    /// An fyi's acknowledgement: sends it.
    Ack,
    /// An approval with every row approved: sends it.
    ApproveAll,
    /// An approval row by row: opens the form.
    Rows,
    /// A question's or a stuck case's reply: opens the text.
    Reply,
    /// A stuck case's park: opens its note.
    Park,
}

/// A tile as drawn: what it does, its label, and whether it is the
/// recommended one.
pub(crate) struct Face {
    pub(crate) tile: Tile,
    pub(crate) label: String,
    pub(crate) star: bool,
}

/// Whether the triage screen is on: `CASES_TUI_TRIAGE=1`.
pub(crate) fn enabled() -> bool {
    std::env::var_os("CASES_TUI_TRIAGE").is_some_and(|v| v == "1")
}

/// The case's tiles, numbered from 1 in this order. A decision's
/// "something else" is not among them: it is a line under them, on `o`.
pub(crate) fn tiles(case: &Case) -> Vec<Face> {
    let face = |tile, label: &str| Face {
        tile,
        label: label.to_string(),
        star: false,
    };
    match case.kind() {
        Kind::Decision => (1i64..)
            .zip(case.options())
            .map(|(n, option)| Face {
                tile: Tile::Option(n),
                label: option.clone(),
                star: n == case.recommend(),
            })
            .collect(),
        Kind::Signoff => vec![
            face(Tile::Accept, "Accept"),
            face(Tile::Changes, "Ask for changes, with a note…"),
        ],
        Kind::Briefing => {
            let mut faces = vec![
                face(Tile::Accept, "Accept"),
                face(Tile::SendBack, "Send it back, with a reason…"),
            ];
            if !case.left_out().is_empty() {
                faces.push(face(Tile::File, "Accept, and pick what to file…"));
            }
            faces
        }
        Kind::Fyi => vec![face(Tile::Ack, "Got it")],
        Kind::Approval => {
            let n = case.rows().len();
            vec![
                face(
                    Tile::ApproveAll,
                    &format!("Approve all {n} row{}", if n == 1 { "" } else { "s" }),
                ),
                face(Tile::Rows, "Go row by row…"),
            ]
        }
        Kind::Question => vec![face(Tile::Reply, "Reply…")],
        Kind::Stuck => vec![
            face(Tile::Reply, "Unblock it: write the guidance…"),
            face(Tile::Park, "Park it, with a note…"),
        ],
        _ => Vec::new(),
    }
}

/// The tile a key picks: its number, or `o` for "something else".
pub(crate) fn by_key(case: &Case, c: char) -> Option<Tile> {
    if c == 'o' && case.kind() == Kind::Decision {
        return Some(Tile::Other);
    }
    let n = usize::try_from(c.to_digit(10)?).ok()?;
    tiles(case).get(n.checked_sub(1)?).map(|face| face.tile)
}

/// The narrowest a tile is in a row; below it the tiles stack.
const MIN_TILE: u16 = 20;
/// The widest a tile in a row is.
const MAX_TILE: u16 = 34;
/// The widest the card is.
const MAX_CARD: u16 = 100;
/// The most lines of a label a tile in a row shows.
const LABEL_LINES: usize = 4;

/// How the tiles are laid out.
enum Shape {
    /// Boxes side by side, each `width` wide and `height` tall.
    Row { width: u16, height: u16 },
    /// Boxes one above the other, each as tall as its label.
    Column,
    /// One line each, no box: when boxes would leave the body no room.
    Compact,
}

/// Draws the case as a triage card: title, meta, body, and the tiles at
/// the bottom. The body scrolls; the tiles never do.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push((area, Hit::Case));
    let Some((key, case)) = app.selected_case() else {
        return;
    };
    let key = *key;
    let width = area.width.saturating_sub(4).min(MAX_CARD);
    let card = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y.saturating_add(1),
        width,
        height: area.height.saturating_sub(1),
    };
    let faces = tiles(case);
    let other = case.kind() == Kind::Decision;
    let gap = 2u16;
    let n = u16::try_from(faces.len()).unwrap_or(u16::MAX).max(1);
    let row_width = (width.saturating_sub(gap * (n - 1)) / n).min(MAX_TILE);
    let label_lines = |face: &Face, w: u16| {
        wrap::wrap(
            vec![Para::plain(markdown::inline(&face.label))],
            usize::from(w.saturating_sub(4)),
        )
    };
    // The tiles' height in each shape, and the one that leaves the body at
    // least a third of the card.
    let row_height = faces
        .iter()
        .map(|f| label_lines(f, row_width).len().min(LABEL_LINES) + 1 + usize::from(f.star))
        .max()
        .unwrap_or(0)
        + 2;
    let column_height: usize = faces
        .iter()
        .map(|f| label_lines(f, width).len().min(2) + 2)
        .sum();
    let extra = if other { 2 } else { 0 };
    let room = usize::from(card.height) * 2 / 3;
    let shape = if row_width >= MIN_TILE && row_height + extra <= room {
        Shape::Row {
            width: row_width,
            height: u16::try_from(row_height).unwrap_or(u16::MAX),
        }
    } else if column_height + extra <= room {
        Shape::Column
    } else {
        Shape::Compact
    };
    let tiles_height = match shape {
        Shape::Row { height, .. } => usize::from(height),
        Shape::Column => column_height,
        Shape::Compact => (1..)
            .zip(&faces)
            .map(|(n, f)| compact_lines(n, f, width).len())
            .sum(),
    } + extra;
    let tiles_height = u16::try_from(tiles_height)
        .unwrap_or(u16::MAX)
        .min(card.height);

    // The body: title, meta, then the case's text, scrolled.
    let dim = Style::new().fg(DIM);
    let mut paras = vec![
        Para::plain(vec![Span::styled(
            case.title().to_string(),
            Style::new().add_modifier(Modifier::BOLD),
        )]),
        Para::plain(vec![
            Span::raw(case.kind().as_str()),
            Span::styled(" · ", dim),
            Span::raw(case.urgency().as_str()),
            Span::styled(" · ", dim),
            Span::styled(case.worker().to_string(), Style::new().fg(ACCENT)),
            Span::styled(format!(" · {key}"), dim),
        ]),
        Para::blank(),
    ];
    paras.extend(markdown::render(case.body().trim()));
    if case.kind() == Kind::Approval {
        paras.push(Para::blank());
        for row in case.rows() {
            paras.push(Para::hanging(
                vec![Span::styled(
                    format!(" {} ", row.id),
                    Style::new().fg(ACCENT),
                )],
                markdown::inline(&row.label),
            ));
        }
    }
    if case.kind() == Kind::Briefing && !case.left_out().is_empty() {
        paras.push(Para::blank());
        paras.push(Para::plain(vec![Span::styled("Left out", dim)]));
        for item in case.left_out() {
            paras.push(Para::hanging(
                vec![Span::raw(" · ")],
                markdown::inline(item),
            ));
        }
    }
    let lines = wrap::wrap(paras, usize::from(card.width));
    // The body takes what it needs, up to what the tiles leave; the tiles
    // follow it, a line below.
    let body = Rect {
        height: card
            .height
            .saturating_sub(tiles_height + 1)
            .min(u16::try_from(lines.len()).unwrap_or(u16::MAX)),
        ..card
    };
    let mut y = body.bottom() + 1;
    let max =
        u16::try_from(lines.len().saturating_sub(usize::from(body.height))).unwrap_or(u16::MAX);
    app.scroll = app.scroll.min(max);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), body);
    if app.scroll < max && body.height > 0 {
        let last = Rect {
            x: body.right().saturating_sub(8),
            y: body.bottom() - 1,
            width: 8.min(body.width),
            height: 1,
        };
        frame.render_widget(Clear, last);
        frame.render_widget(Line::styled(" more ↓", dim).right_aligned(), last);
    }

    // The tiles.
    let bottom = card.bottom();
    let mut x = card.x;
    for (n, face) in (1..).zip(&faces) {
        let edge = if face.star {
            Style::new().fg(Color::Green)
        } else {
            Style::new().fg(DIM)
        };
        let number = Span::styled(
            format!("{n}"),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
        let star = Span::styled("★ recommended", Style::new().fg(Color::Green));
        let rect = match shape {
            Shape::Row { width: w, height } => {
                let rect = Rect::new(x, y, w, height).intersection(card);
                x += w + gap;
                let mut text = vec![Line::from(number)];
                text.extend(label_lines(face, w).into_iter().take(LABEL_LINES));
                if face.star {
                    text.push(Line::from(star));
                }
                frame.render_widget(
                    Paragraph::new(text).block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(edge)
                            .padding(ratatui::widgets::Padding::horizontal(1)),
                    ),
                    rect,
                );
                rect
            }
            Shape::Column => {
                let mut label = label_lines(face, width.saturating_sub(4));
                label.truncate(2);
                let h = u16::try_from(label.len()).unwrap_or(2) + 2;
                let rect = Rect::new(card.x, y, width, h).intersection(card);
                y += h;
                let text: Vec<Line> = label
                    .into_iter()
                    .enumerate()
                    .map(|(at, line)| {
                        let mut spans = if at == 0 {
                            vec![number.clone(), Span::raw("  ")]
                        } else {
                            vec![Span::raw("   ")]
                        };
                        spans.extend(line.spans);
                        if at == 0 && face.star {
                            spans.push(Span::raw("  "));
                            spans.push(star.clone());
                        }
                        Line::from(spans)
                    })
                    .collect();
                frame.render_widget(
                    Paragraph::new(text).block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(edge)
                            .padding(ratatui::widgets::Padding::horizontal(1)),
                    ),
                    rect,
                );
                rect
            }
            Shape::Compact => {
                let lines = compact_lines(n, face, width);
                let h = u16::try_from(lines.len()).unwrap_or(1);
                let rect = Rect::new(card.x, y, width, h).intersection(card);
                y += h;
                frame.render_widget(Paragraph::new(lines), rect);
                rect
            }
        };
        if rect.y < bottom {
            app.hits.push((rect, Hit::Tile(face.tile)));
        }
    }
    if other {
        let y = match shape {
            Shape::Row { height, .. } => y + height + 1,
            _ => y + 1,
        };
        if y < bottom {
            let rect = Rect::new(card.x, y, width.min(40), 1);
            frame.render_widget(
                Line::from(vec![
                    Span::styled("o ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    Span::styled("+ Something else, with a note…", Style::new().fg(ACCENT)),
                ]),
                rect,
            );
            app.hits.push((rect, Hit::Tile(Tile::Other)));
        }
    }
}

/// A tile as a numbered line, its label wrapped under itself: no box.
fn compact_lines(n: usize, face: &Face, width: u16) -> Vec<Line<'static>> {
    let mut spans = markdown::inline(&face.label);
    if face.star {
        spans.push(Span::styled(
            "  ★ recommended",
            Style::new().fg(Color::Green),
        ));
    }
    let number = Span::styled(
        format!(" {n} "),
        Style::new()
            .fg(Color::Black)
            .bg(if face.star { Color::Green } else { ACCENT })
            .add_modifier(Modifier::BOLD),
    );
    wrap::wrap(
        vec![Para::hanging(vec![number, Span::raw(" ")], spans)],
        usize::from(width),
    )
}

impl App {
    /// Does what a tile asks: opens the form it needs, sets the answer the
    /// tile names, and sends it unless something is still to be typed.
    pub(crate) fn tile(&mut self, tile: Tile) {
        use super::form::Which;
        if tile == Tile::Park {
            self.start(Act::Park);
            return;
        }
        if self.form.is_none() {
            self.start(Act::Answer);
        }
        let Some(form) = &mut self.form else {
            return;
        };
        let send = match tile {
            Tile::Option(n) => {
                form.choice = Some(n);
                form.other = false;
                true
            }
            Tile::Other => {
                form.choice = None;
                form.other = true;
                form.editing = Some(Which::Note);
                false
            }
            Tile::Accept if form.kind == Kind::Signoff => {
                form.signoff = Some("accept");
                true
            }
            Tile::Accept => {
                form.briefing = Some("accept");
                true
            }
            Tile::Changes => {
                form.signoff = Some("changes");
                form.editing = Some(Which::Note);
                false
            }
            Tile::SendBack => {
                form.briefing = Some("send_back");
                form.editing = Some(Which::Note);
                false
            }
            Tile::File => {
                form.briefing = Some("accept");
                false
            }
            Tile::Ack => true,
            Tile::ApproveAll => {
                for row in &mut form.rows {
                    row.verdict = Some("approve");
                }
                true
            }
            Tile::Rows | Tile::Park => false,
            Tile::Reply => {
                form.editing = Some(Which::Text);
                false
            }
        };
        if send {
            let outcome = form.send();
            self.form_outcome(outcome);
        }
    }
}
