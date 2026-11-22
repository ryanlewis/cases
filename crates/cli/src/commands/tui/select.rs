//! Copying text off the screen with the mouse (TU-12). A press of the left
//! button in the list or the case pane, then a drag, selects what the pane
//! shows, drawn reversed; the release copies it to the system clipboard
//! with OSC 52. The press is still a click, and acts as one at once, so
//! that a click feels as it did; a press and a release on one cell selects
//! nothing.
//!
//! The selection keeps to the pane the press began in, and to what it
//! shows: no scrolling while dragging. Esc, another click, or any change
//! to the pane's text clears it. What is copied is the pane's text as last
//! drawn, which [`super::safe`] has already cleaned, so the only escape
//! this sends carries text the person chose, as base64.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::Span;

use super::app::{App, Hit};

/// A run of a pane's cells, from where the press began to where the
/// pointer is.
#[derive(Debug)]
pub(crate) struct Selection {
    /// The pane the press began in, as the frame drew it, and which pane
    /// it is: on a narrow terminal the case takes the list's place.
    pane: Rect,
    kind: Hit,
    anchor: Position,
    at: Position,
    /// The pane's cells as last drawn, a symbol each, row by row: none
    /// before the first frame after the press.
    cells: Option<Vec<String>>,
    /// Whether the button is still down.
    held: bool,
}

impl Selection {
    /// Whether it is more than a click: the pointer has gone another row,
    /// or two columns or more, so that a hand that wobbles a column as it
    /// clicks copies nothing.
    fn shown(&self) -> bool {
        self.anchor.y != self.at.y || self.anchor.x.abs_diff(self.at.x) >= 2
    }

    /// Its first and last cells, in reading order.
    fn ends(&self) -> (Position, Position) {
        let key = |p: Position| (p.y, p.x);
        if key(self.anchor) <= key(self.at) {
            (self.anchor, self.at)
        } else {
            (self.at, self.anchor)
        }
    }

    /// The columns it takes on row `y` of the pane, ends included.
    fn columns(&self, y: u16) -> Option<(u16, u16)> {
        let (from, to) = self.ends();
        if y < from.y || y > to.y {
            return None;
        }
        let left = if y == from.y { from.x } else { self.pane.x };
        let right = if y == to.y {
            to.x
        } else {
            self.pane.right().saturating_sub(1)
        };
        Some((left, right))
    }

    /// The text it covers, as drawn: each row's trailing spaces dropped,
    /// the pane's margin taken off the rows that start at its left edge,
    /// and a wide character at either end whole.
    fn text(&self) -> Option<String> {
        let cells = self.cells.as_ref()?;
        let width = usize::from(self.pane.width);
        let (from, to) = self.ends();
        let mut rows = Vec::new();
        for y in from.y..=to.y {
            let (left, right) = self.columns(y)?;
            let start = usize::from(y - self.pane.y) * width;
            let row = cells.get(start..start + width)?;
            let mut x = usize::from(left - self.pane.x);
            let end = usize::from(right - self.pane.x);
            // Begun on the second half of a wide character: from its first.
            if x > 0 && cell_width(&row[x - 1]) > 1 {
                x -= 1;
            }
            let mut line = String::new();
            while x <= end && x < width {
                line.push_str(&row[x]);
                x += cell_width(&row[x]).max(1);
            }
            let whole = left == self.pane.x;
            rows.push((whole, line.trim_end_matches(' ').to_string()));
        }
        let margin = rows
            .iter()
            .filter(|(whole, line)| *whole && !line.is_empty())
            .map(|(_, line)| line.len() - line.trim_start_matches(' ').len())
            .min()
            .unwrap_or(0);
        let lines: Vec<&str> = rows
            .iter()
            .map(|(whole, line)| {
                if *whole {
                    line.get(margin..).unwrap_or("")
                } else {
                    line
                }
            })
            .collect();
        Some(lines.join("\n"))
    }
}

/// How many columns a cell's symbol takes.
fn cell_width(symbol: &str) -> usize {
    Span::raw(symbol).width()
}

/// Whether something is up over the panes that takes a click: help, the
/// palette, a row's menu or a question.
fn overlaid(app: &App) -> bool {
    app.help || app.palette.is_some() || app.menu.is_some() || app.asking()
}

/// The pane the last frame drew at `at`, if any, and which it is.
fn pane_at(app: &App, at: Position) -> Option<(Rect, Hit)> {
    app.hits
        .iter()
        .filter(|(_, hit)| matches!(hit, Hit::List | Hit::Case))
        .find(|(rect, _)| rect.contains(at))
        .copied()
}

/// Takes the mouse before the click does: a press of the left button in a
/// pane begins a selection, a drag carries it, and the release copies it,
/// once it is more than a click.
/// Any press clears the selection shown.
///
/// Gives whether the selection drawn changed.
pub(crate) fn mouse(app: &mut App, event: MouseEvent) -> bool {
    let at = Position::new(event.column, event.row);
    match event.kind {
        MouseEventKind::Down(button) => {
            let was = app.selection.take().is_some_and(|s| s.shown());
            if button == MouseButton::Left
                && !overlaid(app)
                && let Some((pane, kind)) = pane_at(app, at)
            {
                app.selection = Some(Selection {
                    pane,
                    kind,
                    anchor: at,
                    at,
                    cells: None,
                    held: true,
                });
            }
            was
        }
        MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
            let Some(selection) = app.selection.as_mut().filter(|s| s.held) else {
                return false;
            };
            // Kept inside its pane: the pointer past an edge is at it.
            let pane = selection.pane;
            let to = Position::new(
                at.x.clamp(pane.x, pane.right().saturating_sub(1)),
                at.y.clamp(pane.y, pane.bottom().saturating_sub(1)),
            );
            let moved = to != selection.at;
            selection.at = to;
            if matches!(event.kind, MouseEventKind::Up(_)) {
                selection.held = false;
                if !selection.shown() {
                    app.selection = None;
                    return moved;
                }
                if let Some(text) = selection.text().filter(|text| !text.is_empty()) {
                    let count = text.chars().count();
                    app.copied = Some(text);
                    let s = if count == 1 { "" } else { "s" };
                    app.done(&format!("copied {count} character{s}"));
                }
            }
            moved
        }
        _ => false,
    }
}

/// Clears the selection; gives whether one was shown, which Esc then puts
/// away and does nothing else.
pub(crate) fn clear(app: &mut App) -> bool {
    app.selection.take().is_some_and(|s| s.shown())
}

/// Keeps the pane's text as the frame drew it, and draws the selection
/// over it reversed: under the toasts, the palette and a row's menu. A
/// pane that moved, or whose text changed, or one covered by something
/// that takes a click, clears it.
pub(crate) fn draw(frame: &mut Frame, app: &mut App) {
    let covered = overlaid(app);
    let Some(selection) = &app.selection else {
        return;
    };
    let (pane, kind) = (selection.pane, selection.kind);
    let still = app.hits.contains(&(pane, kind));
    let buffer = frame.buffer_mut();
    let cells: Vec<String> = pane
        .rows()
        .flat_map(Rect::columns)
        .map(|at| {
            buffer
                .cell(at)
                .map_or(" ", |cell| cell.symbol())
                .to_string()
        })
        .collect();
    if covered || !still || selection.cells.as_ref().is_some_and(|was| *was != cells) {
        app.selection = None;
        return;
    }
    let Some(selection) = app.selection.as_mut() else {
        return;
    };
    selection.cells = Some(cells);
    if !selection.shown() {
        return;
    }
    for y in pane.top()..pane.bottom() {
        let Some((mut left, right)) = selection.columns(y) else {
            continue;
        };
        // Begun on the second half of a wide character: lit from its
        // first, which holds its look, as it is copied whole.
        if left > pane.x
            && buffer
                .cell((left - 1, y))
                .is_some_and(|cell| cell_width(cell.symbol()) > 1)
        {
            left -= 1;
        }
        for x in left..=right {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.modifier.toggle(Modifier::REVERSED);
            }
        }
    }
}

/// The OSC 52 sequence that puts `text` on the system clipboard.
pub(crate) fn osc52(text: &str) -> Vec<u8> {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text)).into_bytes()
}
