//! Drawing the screen: a header, the filters, the list beside the case, and
//! a footer of keys (TU-02, TU-03).

use cases_core::{Answer, Case, Kind, Record, State, Urgency};
use cases_store_sqlite::Key;
use ratatui::Frame;
use ratatui::buffer::CellWidth as _;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Clear, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph,
};

use super::actions::{self, Action};
use super::app::{App, Filter, Focus, Group, Hit, Newline, Verb};
use super::compose::{Compose, Part};
use super::field::Field;
use super::form::{Act, Confirm, Form, Press, Spot, Which, verdict_word};
use super::wrap::{self, Para};
use super::{markdown, safe, stats};
use crate::describe::{self, Previous};
use crate::output;
use cases_core::Listening;

/// The accent: selection, keys, workers, links.
pub(crate) const ACCENT: Color = Color::Cyan;
/// Detail that should recede: ids, ages, rules.
pub(crate) const DIM: Color = Color::DarkGray;
const SELECTED_BG: Color = Color::Indexed(236);
/// A button's face.
const BUTTON_BG: Color = Color::Indexed(238);
/// What the pointer is over: a button's face, or the cells of anything
/// else a click acts on.
pub(crate) const HOVER_BG: Color = Color::Indexed(241);
/// A dialog's and a toast's face, over the screen.
const PANEL_BG: Color = Color::Indexed(235);
/// The part of the answer the keys are on: a teal, apart from the grey of
/// the selection, the panel and the pointer.
pub(crate) const FOCUS_BG: Color = Color::Indexed(23);

/// Below this many columns, one pane at a time (TU-02).
pub(crate) const SIDE_BY_SIDE: u16 = 96;

pub(crate) fn draw(frame: &mut Frame, app: &mut App) {
    let width = frame.area().width;
    let side_by_side = width >= SIDE_BY_SIDE;
    // The filters and the footer wrap onto more lines on a narrow terminal
    // rather than lose a tab or a key off the end.
    let message = app.only.is_none().then(|| message_button(app));
    // The palette's button is at the right of the tabs, beside the
    // message's, where that takes the tabs no more rows than they take
    // without it; otherwise, and on one case's screen, in the header,
    // when it has room for it. On a screen narrower than that, the
    // footer's ^k says how to open the palette.
    let commands = commands_button();
    let tabs_in = |room: u16| {
        message
            .as_ref()
            .map(|message| filter_lines(app, room.saturating_sub(message.width() + 1)))
    };
    let alone = tabs_in(width);
    let beside = tabs_in(width.saturating_sub(commands.width() + 1));
    let rows_of = |tabs: &Option<Flow>| tabs.as_ref().map_or(0, |(lines, _)| lines.len());
    let fits_beside = message.is_some() && rows_of(&beside) <= rows_of(&alone);
    let in_header = !fits_beside && width >= HEADER_ROOM;
    let tabs = if fits_beside { beside } else { alone };
    app.strip.room = frame.area().height >= stats::MIN_ROWS;
    let rows = |lines: &[Line]| u16::try_from(lines.len()).unwrap_or(u16::MAX);
    // The stats strip (TU-13), under the filters: not on one case's screen,
    // and not on a screen too short to spare it.
    // An image that would not fit whole beside the labels is text.
    let image = app
        .strip
        .image
        .then(|| stats::image_width(width))
        .filter(|wide| width >= stats::LABEL + wide);
    let strip = app
        .strip
        .stats
        .as_ref()
        .filter(|_| app.strip.shown && app.strip.room && app.only.is_none())
        .map(|stats| stats::lines(stats, image));
    let layout = |keys: &Flow| -> [Rect; 6] {
        Layout::vertical([
            Constraint::Length(1),
            // One case's screen (TU-11) has no filters: no row for them.
            Constraint::Length(tabs.as_ref().map_or(0, |(lines, _)| rows(lines))),
            Constraint::Length(1),
            Constraint::Length(if strip.is_some() { stats::HEIGHT } else { 0 }),
            Constraint::Min(1),
            Constraint::Length(rows(&keys.0)),
        ])
        .areas(frame.area())
    };
    // The footer leaves to the case's bar the keys it has buttons for, when
    // the bar is drawn: worked out with the footer that leaves them, as
    // one that has them is no shorter, and leaves the bar no more room.
    // Worked out once a frame: the acts the case's bar offers, and the
    // answer's panel's buttons, as wide as the case pane.
    let verbs = app.verbs();
    let trimmed = footer_lines(app, &verbs, width, side_by_side, true);
    let pane = case_pane(app, layout(&trimmed)[4], side_by_side).map(|pane| inset(pane, 2, 1));
    let mut buttons = pane.and_then(|inner| panel_buttons(app, inner.width));
    let bar = pane.is_some_and(|inner| {
        case_fit(app, &verbs, scroll_area(inner, buttons.as_ref()))
            .0
            .is_some()
    });
    let keys = if bar {
        trimmed
    } else {
        footer_lines(app, &verbs, width, side_by_side, false)
    };
    let [header, filters, rule, band, body, footer] = layout(&keys);
    app.hits.clear();
    draw_header(frame, app, header, in_header.then_some(&commands));
    if let Some(tabs) = tabs {
        let hits = draw_flow(frame, tabs, filters);
        app.hits.extend(hits);
    }
    if let Some(message) = message {
        let x = filters.right().saturating_sub(message.width());
        message.draw(frame, app, x, filters.y, filters);
        if fits_beside {
            let x = x.saturating_sub(commands.width() + 1);
            commands.draw(frame, app, x, filters.y, filters);
        }
    }
    draw_rule(frame, app, rule);
    let drawn = strip.is_some();
    if let Some(strip) = strip {
        frame.render_widget(Paragraph::new(strip), band);
    }
    // The image goes over the cells the lines leave it, below the caption.
    app.strip.area = image
        .filter(|_| drawn && !app.help)
        .map(|wide| stats::image_area(band, wide));
    if app.only.is_some() {
        // One case (TU-11): no list beside it.
        draw_case(frame, app, body, &verbs, buttons.take());
    } else if side_by_side {
        let [list, gap, case] = side_by_side_panes(body);
        draw_list(frame, app, list, true);
        let bar: Vec<Line> = (0..gap.height).map(|_| Line::from("│")).collect();
        frame.render_widget(Paragraph::new(bar).style(Style::new().fg(DIM)), gap);
        draw_case(frame, app, case, &verbs, buttons.take());
    } else if app.focus == Focus::List {
        draw_list(frame, app, body, false);
    } else {
        draw_case(frame, app, body, &verbs, buttons.take());
    }
    // Under the toasts, the palette and a row's menu.
    draw_focus(frame, app);
    let hits = draw_flow(frame, keys, footer);
    app.hits.extend(hits);
    draw_toasts(frame, app, body);
    if app.palette.is_some() {
        draw_palette(frame, app);
    }
    if let Some(at) = app.menu.as_ref().map(|menu| menu.at) {
        draw_menu(frame, app, at);
    }
    // Over a dialog or help, what lies under them is not lit: a click
    // there takes them away.
    if !app.help && !app.asking() {
        light(frame, app);
    }
    if app.help {
        draw_help(frame, app.only.is_some());
    }
    if let Some(dialog) = question(app) {
        draw_dialog(frame, app, &dialog);
    }
    // The layout may have moved under a pointer that has not: what it is
    // over now, for the next frame.
    app.hover = app.hovered();
}

/// Lights the cells of what the pointer is over, unless it is a button or
/// a tab, which light themselves (TU-12).
fn light(frame: &mut Frame, app: &App) {
    let Some(hover) = app
        .hover
        .filter(|hit| !matches!(hit, Hit::Key(_) | Hit::Tab(_)))
    else {
        return;
    };
    let area = frame.area();
    // A toast over what is lit stays as it is.
    let toasts: Vec<Rect> = app
        .hits
        .iter()
        .filter(|(_, hit)| matches!(hit, Hit::Toast(_)) && *hit != hover)
        .map(|(rect, _)| *rect)
        .collect();
    let buffer = frame.buffer_mut();
    for (rect, _) in app.hits.iter().filter(|(_, hit)| *hit == hover) {
        let rect = rect.intersection(area);
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let at = ratatui::layout::Position::new(x, y);
                if toasts.iter().any(|toast| toast.contains(at)) {
                    continue;
                }
                if let Some(cell) = buffer.cell_mut((x, y)) {
                    cell.set_bg(HOVER_BG);
                }
            }
        }
    }
}

/// How a button looks: the one to press, one that loses something, or
/// any other (TU-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Look {
    Plain,
    Primary,
    Danger,
}

/// A button: its label on a filled run of cells, with its key before it
/// or a letter of it underlined, lit while the pointer is over it. A click
/// on it does what its hit says, as a key's does what pressing it does
/// (TU-12). Without a hit it is not a button: its text is drawn as a
/// footer's key that cannot be clicked, with no face.
#[derive(Debug, Clone)]
pub(crate) struct Button {
    /// The key that does the same, shown before the label.
    pub(crate) key: Option<&'static str>,
    pub(crate) label: &'static str,
    /// The letter of the label to underline, as a menu shows the key that
    /// picks an item.
    pub(crate) accel: Option<char>,
    pub(crate) look: Look,
    pub(crate) hit: Option<Hit>,
}

impl Button {
    /// A plain button that does `hit`.
    pub(crate) fn new(label: &'static str, hit: Option<Hit>) -> Button {
        Button {
            key: None,
            label,
            accel: None,
            look: Look::Plain,
            hit,
        }
    }

    /// The button with `key` shown before its label.
    pub(crate) fn key(self, key: &'static str) -> Button {
        Button {
            key: Some(key),
            ..self
        }
    }

    /// The button with the first `letter` of its label underlined, in
    /// either case.
    pub(crate) fn accel(self, letter: char) -> Button {
        Button {
            accel: Some(letter),
            ..self
        }
    }

    pub(crate) fn look(self, look: Look) -> Button {
        Button { look, ..self }
    }

    /// How it is drawn: lit, while the pointer is over it.
    pub(crate) fn spans(&self, lit: bool) -> Vec<Span<'static>> {
        let face = match (self.hit, self.look, lit) {
            (None, ..) => Style::new(),
            (Some(_), Look::Plain, false) => Style::new().bg(BUTTON_BG),
            (Some(_), Look::Plain, true) => Style::new().bg(HOVER_BG).fg(Color::White),
            (Some(_), Look::Primary, false) => Style::new().bg(Color::Green),
            (Some(_), Look::Primary, true) => Style::new().bg(Color::LightGreen),
            (Some(_), Look::Danger, false) => Style::new().bg(Color::Red),
            (Some(_), Look::Danger, true) => Style::new().bg(Color::LightRed),
        };
        let filled = self.hit.is_some() && self.look != Look::Plain;
        let face = if filled {
            face.fg(Color::Black).add_modifier(Modifier::BOLD)
        } else {
            face
        };
        let label = if self.hit.is_none() {
            face.fg(DIM)
        } else {
            face
        };
        let key = if filled {
            face
        } else {
            face.fg(ACCENT).add_modifier(Modifier::BOLD)
        };
        let mut spans = vec![Span::styled(" ", face)];
        if let Some(text) = self.key {
            spans.push(Span::styled(text, key));
            spans.push(Span::styled(" ", face));
        }
        let at = self.accel.and_then(|letter| {
            self.label
                .char_indices()
                .find(|(_, c)| c.eq_ignore_ascii_case(&letter))
        });
        match at {
            Some((at, c)) => {
                let after = at + c.len_utf8();
                spans.push(Span::styled(&self.label[..at], label));
                spans.push(Span::styled(
                    &self.label[at..after],
                    label.add_modifier(Modifier::UNDERLINED),
                ));
                spans.push(Span::styled(&self.label[after..], label));
            }
            None => spans.push(Span::styled(self.label, label)),
        }
        spans.push(Span::styled(" ", face));
        spans
    }

    /// How it is drawn while the keys are on it, when `focused`: pointed
    /// at, its label underlined, apart from the pointer's light (TU-12).
    pub(crate) fn spans_at(&self, lit: bool, focused: bool) -> Vec<Span<'static>> {
        let mut spans = self.spans(lit);
        if focused {
            let last = spans.len().saturating_sub(1);
            for span in spans.iter_mut().take(last).skip(1) {
                span.style = span
                    .style
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
            }
            spans.insert(
                0,
                Span::styled("▸", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            );
        }
        spans
    }

    /// How many columns it takes.
    pub(crate) fn width(&self) -> u16 {
        let wide: usize = self.spans(false).iter().map(Span::width).sum();
        u16::try_from(wide).unwrap_or(u16::MAX)
    }

    /// Whether the pointer is over it.
    fn lit(&self, app: &App) -> bool {
        self.hit.is_some() && app.hover == self.hit
    }

    /// Draws it at `x`, `y`, as far as `area` lets it, and says where a
    /// click on it acts. Gives the column after it.
    pub(crate) fn draw(&self, frame: &mut Frame, app: &mut App, x: u16, y: u16, area: Rect) -> u16 {
        let wide = self.width().min(area.right().saturating_sub(x));
        let rect = Rect::new(x, y, wide, 1).intersection(area);
        frame.render_widget(Line::from(self.spans(self.lit(app))), rect);
        if let Some(hit) = self.hit {
            app.hits.push((rect, hit));
        }
        x.saturating_add(wide)
    }
}

/// A question in a box over the screen, with a button for each answer
/// (TU-08, TU-12).
#[derive(Debug)]
pub(crate) struct Dialog {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    /// The safe answer first.
    pub(crate) buttons: Vec<Button>,
}

/// The question the screen asks, if any: whether to quit and lose what
/// was typed, or the form's or the message's. Esc, the first button, is
/// the safe answer, as is any key but y, or a click off the dialog.
fn question(app: &App) -> Option<Dialog> {
    let keep = |label| {
        Button::new(label, Some(Hit::Key(KeyEvent::from(KeyCode::Esc))))
            .key("esc")
            .look(Look::Primary)
    };
    let yes = |label| {
        Button::new(label, Some(Hit::Key(KeyEvent::from(KeyCode::Char('y')))))
            .key("y")
            .look(Look::Danger)
    };
    let typed = || "Throw away what you typed?".to_string();
    let (title, body, buttons) = if app.confirm_quit {
        (
            "Quit and throw away what you typed?".to_string(),
            None,
            vec![keep("Keep typing"), yes("Quit")],
        )
    } else if let Some((form, confirm)) = app
        .form
        .as_ref()
        .and_then(|form| form.confirm.map(|confirm| (form, confirm)))
    {
        match confirm {
            Confirm::Drop => (
                format!("Drop {}?", form.key),
                Some("The agent stops the work.".to_string()),
                vec![keep("Keep it"), yes("Drop")],
            ),
            Confirm::Withdraw => (
                format!("Withdraw {} from its agent?", form.key),
                None,
                vec![keep("Keep it"), yes("Withdraw")],
            ),
            Confirm::Discard => (typed(), None, vec![keep("Keep typing"), yes("Discard")]),
        }
    } else if app.compose.as_ref().is_some_and(|c| c.confirm) {
        (typed(), None, vec![keep("Keep typing"), yes("Discard")])
    } else {
        return None;
    };
    Some(Dialog {
        title,
        body,
        buttons,
    })
}

/// Draws `dialog` in the middle of the screen, its buttons along its foot.
/// A click on its face does nothing; one off it lands on what is under it.
fn draw_dialog(frame: &mut Frame, app: &mut App, dialog: &Dialog) {
    const GAP: u16 = 2;
    let area = frame.area();
    let row: u16 = dialog
        .buttons
        .iter()
        .map(Button::width)
        .sum::<u16>()
        .saturating_add(GAP * u16::try_from(dialog.buttons.len().saturating_sub(1)).unwrap_or(0));
    let text = |s: &str| u16::try_from(s.chars().count()).unwrap_or(u16::MAX);
    let wanted = text(&dialog.title)
        .max(dialog.body.as_deref().map_or(0, text))
        .max(row)
        .saturating_add(6);
    let width = wanted.min(area.width.saturating_sub(2)).min(72);
    let inner = usize::from(width.saturating_sub(6)).max(1);
    let mut paras = vec![Para::plain(vec![Span::styled(
        safe::clean(&dialog.title).into_owned(),
        Style::new().add_modifier(Modifier::BOLD),
    )])];
    if let Some(body) = &dialog.body {
        paras.push(Para::plain(vec![Span::styled(
            safe::clean(body).into_owned(),
            Style::new().fg(DIM),
        )]));
    }
    let lines = wrap::wrap(paras, inner);
    let tall = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    // The border, a blank row above and below the text, then the buttons.
    let height = tall.saturating_add(5).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Yellow))
        .style(Style::new().bg(PANEL_BG));
    let face = block.inner(rect);
    frame.render_widget(block, rect);
    // On a short screen the text is cut, not the buttons under it.
    let text_area = Rect::new(
        face.x.saturating_add(2),
        face.y.saturating_add(1),
        face.width.saturating_sub(4),
        tall.min(face.height.saturating_sub(2)),
    )
    .intersection(face);
    frame.render_widget(Paragraph::new(lines), text_area);
    app.hits.push((rect, Hit::Dialog));
    let y = face.bottom().saturating_sub(1);
    let mut x = face.x.saturating_add(2);
    for button in &dialog.buttons {
        x = button.draw(frame, app, x, y, face).saturating_add(GAP);
    }
}

/// Draws the toasts in the top right corner of `area`, the newest on top,
/// as many as fit: out of the way of the form at the foot of the case
/// (TU-12).
fn draw_toasts(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(2).min(56);
    if width < 8 {
        return;
    }
    let mut top = area.y;
    for toast in app.toasts.iter().rev() {
        let (mark, colour) = if toast.done {
            ("✓ ", Color::Green)
        } else {
            ("! ", Color::Yellow)
        };
        let para = Para::hanging(
            vec![
                Span::raw(" "),
                Span::styled(mark, Style::new().fg(colour).add_modifier(Modifier::BOLD)),
            ],
            vec![Span::raw(toast.text.clone())],
        );
        let lines = wrap::wrap(vec![para], usize::from(width - 1));
        let wide = lines.iter().map(Line::width).max().unwrap_or(0) + 1;
        let wide = u16::try_from(wide).unwrap_or(width).min(width);
        let tall = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        if tall > area.bottom().saturating_sub(top) {
            break;
        }
        let rect = Rect::new(area.right() - wide - 1, top, wide, tall);
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(lines).style(Style::new().bg(PANEL_BG)), rect);
        app.hits.push((rect, Hit::Toast(toast.id)));
        // A row between toasts, so that they read as two.
        top = top.saturating_add(tall + 1);
    }
}

/// The header: the store, what waits on the human, and the palette's
/// button `commands` before that when it is not beside the tabs.
fn draw_header(frame: &mut Frame, app: &mut App, area: Rect, commands: Option<&Button>) {
    let (waiting, blocking) = app.waiting();
    let mut right = Vec::new();
    if app.only.is_some() {
        // One case's screen (TU-11) is about that case, which it shows in
        // full: what else waits in the store is not its to count.
    } else if waiting == 0 {
        right.push(Span::styled("nothing needs you ", Style::new().fg(DIM)));
    } else {
        let colour = if blocking > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        right.push(Span::styled("● ", Style::new().fg(colour)));
        right.push(Span::styled(
            format!("{waiting} waiting"),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        if blocking > 0 {
            right.push(Span::styled(" · ", Style::new().fg(DIM)));
            right.push(Span::styled(
                format!("{blocking} blocking"),
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        right.push(Span::raw(" "));
    }
    let right = Line::from(right).right_aligned();
    let button = commands.map_or(0, |button| usize::from(button.width()) + 1);
    let room = usize::from(area.width).saturating_sub(right.width() + button + 10);
    let left = Line::from(vec![
        Span::styled(
            " cases ",
            Style::new()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", truncate(&safe::clean(&app.store), room)),
            Style::new().fg(DIM),
        ),
    ]);
    let right_width = u16::try_from(right.width()).unwrap_or(u16::MAX);
    frame.render_widget(left, area);
    frame.render_widget(right, area);
    if let Some(button) = commands {
        let x = area
            .right()
            .saturating_sub(right_width + button.width() + 1);
        button.draw(frame, app, x, area.y, area);
    }
}

/// Lines drawn over several rows, and where on them each part a click
/// acts on is: its row, and its first and last column.
type Flow = (Vec<Line<'static>>, Vec<(usize, usize, usize, Hit)>);

/// One part of a [`Flow`]: what it shows, whether a bar sets it apart from
/// the parts before it on its line, and what a click on it does.
struct Chip {
    spans: Vec<Span<'static>>,
    bar: bool,
    hit: Option<Hit>,
}

/// The chips in `width` columns, each kept whole: on one line with `gap`
/// between them where they fit, and otherwise with `tight` between them,
/// over as many lines as they need. A bar is drawn only between chips on
/// the same line.
fn flow(chips: Vec<Chip>, width: u16, gap: &str, tight: &str) -> Flow {
    let width = usize::from(width);
    let between = |gap: &str, bar: bool| {
        if bar {
            // The bar takes the place of the gap's first space after it.
            format!("{gap}│{}", gap.get(1..).unwrap_or(""))
        } else {
            gap.to_owned()
        }
    };
    let wide: usize = 1 + chips
        .iter()
        .enumerate()
        .map(|(n, chip)| {
            let gap = if n == 0 {
                0
            } else {
                between(gap, chip.bar).chars().count()
            };
            gap + chip.spans.iter().map(Span::width).sum::<usize>()
        })
        .sum::<usize>();
    let gap = if wide <= width { gap } else { tight };
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut used = 0;
    let mut hits = Vec::new();
    for chip in chips {
        let size: usize = chip.spans.iter().map(Span::width).sum();
        let lead = between(gap, chip.bar);
        let lead_width = lead.chars().count();
        if used > 0 && used + lead_width + size > width {
            lines.push(Line::from(std::mem::take(&mut line)));
            used = 0;
        }
        if used > 0 {
            line.push(Span::styled(lead, Style::new().fg(DIM)));
            used += lead_width;
        }
        if let Some(hit) = chip.hit {
            hits.push((lines.len(), used, used + size, hit));
        }
        line.extend(chip.spans);
        used += size;
    }
    lines.push(Line::from(line));
    (lines, hits)
}

/// Draws a [`Flow`] in `area`, and gives where each part a click acts on
/// was drawn (TU-12).
fn draw_flow(frame: &mut Frame, (lines, hits): Flow, area: Rect) -> Vec<(Rect, Hit)> {
    let hits = hits
        .into_iter()
        .filter_map(|(row, from, to, hit)| {
            let row = u16::try_from(row).ok().filter(|row| *row < area.height)?;
            let x = u16::try_from(from).unwrap_or(u16::MAX).min(area.width);
            let end = u16::try_from(to).unwrap_or(u16::MAX).min(area.width);
            Some((Rect::new(area.x + x, area.y + row, end - x, 1), hit))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    hits
}

/// The filters' tabs, over as many lines as `width` needs (TU-12).
fn filter_lines(app: &App, width: u16) -> Flow {
    let mut chips = Vec::new();
    for filter in Filter::ALL {
        // The briefings' count is of the open ones (IN-14).
        let count = match filter {
            Filter::Briefings => app.briefings().0.len(),
            _ => app.shown(filter).len(),
        };
        // A pill: the chosen one filled, any other lit under the pointer.
        let chosen = filter == app.filter;
        let hit = Hit::Tab(filter);
        let (face, quiet) = if chosen {
            let face = Style::new().fg(Color::Black).bg(if app.hover == Some(hit) {
                Color::LightCyan
            } else {
                ACCENT
            });
            (face.add_modifier(Modifier::BOLD), face)
        } else if app.hover == Some(hit) {
            let face = Style::new().bg(HOVER_BG).fg(Color::White);
            (face, face)
        } else {
            (Style::new(), Style::new().fg(DIM))
        };
        chips.push(Chip {
            spans: vec![
                Span::styled(" ", quiet),
                Span::styled(filter.name(), face),
                Span::styled(format!(" {count} "), quiet),
            ],
            bar: filter == Filter::Briefings,
            hit: Some(hit),
        });
    }
    flow(chips, width, "  ", " ")
}

/// The button at the right of the filters' row that starts a message
/// (TU-07, TU-12). While a form or a message is open, which takes the
/// keys, it is drawn but does nothing, as m does nothing then.
fn message_button(app: &App) -> Button {
    let open = app.form.is_some() || app.compose.is_some();
    let hit = (!open).then(|| Hit::Key(KeyEvent::from(KeyCode::Char('m'))));
    Button::new("+ Message", hit).accel('m')
}

/// How wide the screen is before the header has room for the palette's
/// button beside the store and the count of what waits.
const HEADER_ROOM: u16 = 72;

/// The button that opens the palette (TU-12).
fn commands_button() -> Button {
    let ctrl_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
    Button::new("Commands", Some(Hit::Key(ctrl_k))).key("^k")
}

/// The palette (TU-12): what is typed, the case it acts on, and the
/// actions that fit, best first, each with why and its key; the one the
/// keys are on marked. A click on one runs it.
fn draw_palette(frame: &mut Frame, app: &mut App) {
    let Some(palette) = &app.palette else {
        return;
    };
    let found = actions::ranked(app, &palette.query);
    let at = palette.at.min(found.len().saturating_sub(1));
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(64);
    let shown = found
        .len()
        .min(usize::from(area.height.saturating_sub(8)).max(1));
    let on = app
        .selected_case()
        .map(|(_, case)| safe::clean(case.title()).into_owned());
    let mut lines = vec![Line::from(vec![
        Span::styled("› ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(safe::clean(&palette.query).into_owned()),
        Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
    ])];
    lines.push(Line::styled(
        on.map_or_else(|| "on: no case".to_string(), |title| format!("on: {title}")),
        Style::new().fg(DIM),
    ));
    let inner = usize::from(width.saturating_sub(4));
    // The window of what was found that keeps the chosen one in sight.
    let skip = (at + 1).saturating_sub(shown);
    for (n, action) in found.iter().enumerate().skip(skip).take(shown) {
        let key = action.key();
        let name = action.name();
        let room = inner.saturating_sub(name.chars().count() + key.chars().count() + 3);
        let why = truncate(action.why(), room);
        let pad = room.saturating_sub(why.chars().count());
        let chosen = n == at;
        let face = if chosen {
            Style::new().bg(SELECTED_BG)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{name} "),
                face.add_modifier(if chosen {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            Span::styled(format!(" {why}"), face.fg(DIM)),
            Span::styled(" ".repeat(pad), face),
            Span::styled(
                format!(" {key}"),
                face.fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if found.is_empty() {
        lines.push(Line::styled("nothing fits", Style::new().fg(DIM)));
    }
    let height = u16::try_from(lines.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 3,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(" commands ")
        .style(Style::new().bg(PANEL_BG))
        .padding(Padding::horizontal(1));
    let face = block.inner(rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
    app.hits.push((rect, Hit::Dialog));
    for row in 0..u16::try_from(shown.min(found.len())).unwrap_or(0) {
        let y = face.y + 2 + row;
        if y < face.bottom()
            && let Some(action) = found.get(skip + usize::from(row))
        {
            let rect = Rect::new(face.x, y, face.width, 1);
            app.hits.push((rect, Hit::Palette(*action)));
        }
    }
}

/// The menu a right-click opens on a row (TU-12): the actions that fit
/// its case, each with its key, at the pointer `at`, kept inside the
/// screen.
fn draw_menu(frame: &mut Frame, app: &mut App, at: ratatui::layout::Position) {
    let found = Action::on_case(app);
    let item = app.menu.as_ref().map_or(0, |menu| menu.item);
    let wide = found
        .iter()
        .map(|action| action.name().chars().count() + action.key().chars().count() + 3)
        .max()
        .unwrap_or(0)
        .max("nothing to do on it".len());
    let area = frame.area();
    let width = u16::try_from(wide + 4).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(found.len().max(1) + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let rect = menu_rect(area, at, width, height);
    let inner = usize::from(width.saturating_sub(4));
    let mut lines = Vec::new();
    for (n, action) in found.iter().enumerate() {
        let face = if n == item {
            Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let name = action.name();
        let pad = inner.saturating_sub(name.chars().count() + action.key().chars().count());
        lines.push(Line::from(vec![
            Span::styled(format!("{name}{}", " ".repeat(pad)), face),
            Span::styled(action.key(), face.fg(ACCENT)),
        ]));
    }
    if found.is_empty() {
        lines.push(Line::styled("nothing to do on it", Style::new().fg(DIM)));
    }
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .style(Style::new().bg(PANEL_BG))
        .padding(Padding::horizontal(1));
    let face = block.inner(rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
    app.hits.push((rect, Hit::Dialog));
    for (n, action) in found.iter().enumerate() {
        let y = face.y + u16::try_from(n).unwrap_or(u16::MAX);
        if y < face.bottom() {
            let rect = Rect::new(face.x, y, face.width, 1);
            app.hits.push((rect, Hit::Menu(*action)));
        }
    }
}

/// Where a menu `width` by `height` opens for a right-click at `at`:
/// below and right of the pointer, moved up and left as far as it must
/// be to stay inside `area`.
pub(crate) fn menu_rect(
    area: Rect,
    at: ratatui::layout::Position,
    width: u16,
    height: u16,
) -> Rect {
    let x = at.x.min(area.right().saturating_sub(width)).max(area.x);
    let y = (at.y + 1)
        .min(area.bottom().saturating_sub(height))
        .max(area.y);
    Rect::new(x, y, width, height)
}

/// The case's bar: a button for each act its keys would do on it (TU-12).
fn verb_chips(app: &App, verbs: &[Verb]) -> Vec<Chip> {
    verbs
        .iter()
        .copied()
        .map(|verb| {
            let look = if verb == Verb::Answer {
                Look::Primary
            } else {
                Look::Plain
            };
            let button = Button::new(verb.label(), Some(Hit::Verb(verb)))
                .accel(verb.key())
                .look(look);
            Chip {
                spans: button.spans_at(button.lit(app), app.focus == Focus::Bar(verb)),
                bar: false,
                hit: button.hit,
            }
        })
        .collect()
}

/// The fewest rows of the case, under its bar, that the bar leaves.
const MIN_BODY: u16 = 3;

/// Where the case pane is drawn in `body`: beside the list, or alone; or
/// nowhere, on a narrow screen showing the list.
fn case_pane(app: &App, body: Rect, side_by_side: bool) -> Option<Rect> {
    if app.only.is_some() || (!side_by_side && app.focus != Focus::List) {
        Some(body)
    } else if side_by_side {
        let [_, _, case] = side_by_side_panes(body);
        Some(case)
    } else {
        None
    }
}

/// The list, the bar between, and the case, side by side.
fn side_by_side_panes(body: Rect) -> [Rect; 3] {
    let list_width = (body.width * 38 / 100).clamp(36, 58);
    Layout::horizontal([
        Constraint::Length(list_width),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(body)
}

/// How the case's bar fits the case pane's `inner` area: the bar, when
/// there is room for it and [`MIN_BODY`] rows of the case under it; and
/// whether the title and the meta are held above it too, when there is
/// room for them as well. Otherwise they scroll with the rest.
fn case_fit(app: &App, verbs: &[Verb], inner: Rect) -> (Option<Flow>, bool) {
    let chips = verb_chips(app, verbs);
    let Some((key, case)) = app.selected_case().filter(|_| !chips.is_empty()) else {
        return (None, false);
    };
    let bar = flow(chips, inner.width, " ", " ");
    let tall = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
    let bar_rows = tall(bar.0.len() + 1);
    if inner.height < bar_rows.saturating_add(MIN_BODY) {
        return (None, false);
    }
    let mut paras = case_paras(app, *key, case);
    paras.truncate(2);
    let head = tall(wrap::wrap(paras, usize::from(inner.width)).len());
    let fixed = inner.height >= head.saturating_add(bar_rows).saturating_add(MIN_BODY);
    (Some(bar), fixed)
}

/// Says that a click on each line drawn in `area`, from line `skip` of
/// `hits`, does what that line's paragraph does (TU-12).
fn line_hits(app: &mut App, area: Rect, hits: &[Option<Hit>], skip: usize) {
    for row in 0..area.height {
        match hits.get(skip + usize::from(row)) {
            // A row's verdicts: a click on each picks it.
            Some(Some(Hit::Verdicts(at, human))) => {
                // The line, under the cells: what the keys being on the
                // row fills, and what a click between them does nothing on.
                app.hits.push((
                    Rect::new(area.x, area.y + row, area.width, 1),
                    Hit::Verdicts(*at, *human),
                ));
                for (x, wide, verdict) in verdict_cells(*human) {
                    let rect = Rect::new(area.x + x, area.y + row, wide, 1).intersection(area);
                    app.hits.push((rect, Hit::Verdict(*at, verdict)));
                }
            }
            Some(Some(hit)) => app
                .hits
                .push((Rect::new(area.x, area.y + row, area.width, 1), *hit)),
            _ => {}
        }
    }
}

/// The lines of the case that the part of its answer the keys are on is
/// drawn over, first and last, from what a click on each does.
fn reveal_lines(app: &App, hits: &[Option<Hit>]) -> Option<(usize, usize)> {
    let form = app.form.as_ref().filter(|form| form.act == Act::Answer)?;
    let spot = form.spot();
    let lines: Vec<usize> = hits
        .iter()
        .enumerate()
        .filter(|(_, hit)| hit.is_some_and(|hit| on_spot(form, spot, hit, false)))
        .map(|(at, _)| at)
        .collect();
    Some((*lines.first()?, *lines.last()?))
}

/// Whether `hit` is drawn as part of `spot` of `form`'s answer: all its
/// choices, or, when `pointed`, only the one pointed at; the row the keys
/// are on, with its verdicts and note; the item; the field.
fn on_spot(form: &Form, spot: Spot, hit: Hit, pointed: bool) -> bool {
    match (spot, hit) {
        (Spot::Field(which), Hit::Field(field)) => which == field,
        (Spot::Choices, Hit::Choice(n)) => !pointed || pointed_choice(form) == n,
        (Spot::Choices, Hit::Mark(mark)) => !pointed || pointed_mark(form) == mark,
        (
            Spot::Rows,
            Hit::ApprovalRow(at)
            | Hit::Verdicts(at, _)
            | Hit::Verdict(at, _)
            | Hit::Field(Which::Row(at)),
        ) => at == form.row,
        (Spot::Items, Hit::LeftOut(at)) => at == form.item,
        _ => false,
    }
}

/// The decision's option the keys point at, on its choices: the one
/// chosen, other, or else the first.
fn pointed_choice(form: &Form) -> i64 {
    form.choice
        .or(form.other.then_some(form.options() + 1))
        .unwrap_or(1)
}

/// The signoff's or the briefing's answer the keys point at: the one
/// chosen, or else accept.
fn pointed_mark(form: &Form) -> &'static str {
    match form.kind {
        Kind::Signoff => form.signoff,
        _ => form.briefing,
    }
    .unwrap_or("accept")
}

/// Fills the part of the answer the keys are on, while they are in it,
/// as a ring the pointer's light and a chosen answer's dot both differ
/// from (TU-12): drawn before the toasts and anything over the screen. A
/// button is pointed at by its own face.
fn draw_focus(frame: &mut Frame, app: &App) {
    let Some(form) = app.form.as_ref().filter(|form| form.act == Act::Answer) else {
        return;
    };
    let spot = form.spot();
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for (rect, _) in app
        .hits
        .iter()
        .filter(|(_, hit)| on_spot(form, spot, *hit, true))
    {
        let rect = rect.intersection(area);
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                if let Some(cell) = buffer.cell_mut((x, y)) {
                    cell.set_bg(FOCUS_BG);
                }
            }
        }
    }
}

/// The scroll nearest `scroll` that shows the lines `first` to `last` in
/// `height` rows: their end, when they are taller than that.
fn in_sight(scroll: u16, height: u16, first: usize, last: usize) -> u16 {
    let line = |at: usize| u16::try_from(at).unwrap_or(u16::MAX);
    let mut scroll = scroll.max(line(last + 1).saturating_sub(height));
    if line(last) - line(first) < height {
        scroll = scroll.min(line(first));
    }
    scroll
}

/// Where a line of a row's verdicts draws each: its column, its width,
/// and the verdict, after an indent, two columns apart.
fn verdict_cells(human: bool) -> Vec<(u16, u16, &'static str)> {
    let mut x = 5;
    let mut cells = Vec::new();
    for verdict in ["approve", "hold", "reject"] {
        let wide = 2 + verdict_word(verdict, human).chars().count();
        let wide = u16::try_from(wide).unwrap_or(u16::MAX);
        cells.push((x, wide, verdict));
        x += wide + 2;
    }
    cells
}

/// The rule under the filters, or what is wrong with the store on it.
fn draw_rule(frame: &mut Frame, app: &App, area: Rect) {
    let warning = match (&app.problem, app.unfolded) {
        (Some(why), _) => Some(format!("cannot read the store, trying again: {why}")),
        (None, 0) => None,
        (None, 1) => Some("1 case cannot be read; cases list names it".into()),
        (None, n) => Some(format!("{n} cases cannot be read; cases list names them")),
    };
    let line = match warning {
        Some(text) => Line::styled(
            format!(
                " ⚠ {}",
                truncate(
                    &safe::clean(&text),
                    usize::from(area.width).saturating_sub(4)
                )
            ),
            Style::new().fg(Color::Yellow),
        ),
        None => Line::styled("─".repeat(usize::from(area.width)), Style::new().fg(DIM)),
    };
    frame.render_widget(line, area);
}

/// A region's title (TU-12): the list's, the case's or the answer's, with
/// an accent edge and "keys here" on the one with the keys, and dim on
/// the others.
fn region_title(name: &str, keys: bool) -> Vec<Span<'static>> {
    if keys {
        let accent = Style::new().fg(ACCENT);
        vec![
            Span::styled("▍", accent),
            Span::styled(name.to_string(), accent.add_modifier(Modifier::BOLD)),
            Span::styled(" · keys here", accent),
        ]
    } else {
        vec![Span::styled(
            format!(" {name}"),
            Style::new().fg(DIM).add_modifier(Modifier::BOLD),
        )]
    }
}

/// Which region has the keys: the list, the case, or its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    List,
    Case,
    Answer,
}

/// The region with the keys (TU-12): the answer while its form has them;
/// the case while a park's or a withdraw's form, a message, or a button
/// of the case's bar has them; or else the pane that has them.
pub(crate) fn region(app: &App) -> Region {
    match &app.form {
        Some(form) if form.act == Act::Answer => Region::Answer,
        None if app.compose.is_none() && app.focus == Focus::List => Region::List,
        _ => Region::Case,
    }
}

/// Draws a pane's title on the top row of its `area`.
fn draw_title(frame: &mut Frame, area: Rect, name: &str, keys: bool) {
    let row = Rect {
        height: area.height.min(1),
        ..area
    };
    frame.render_widget(Line::from(region_title(name, keys)), row);
}

fn urgency_style(urgency: Urgency) -> Style {
    match urgency {
        Urgency::Blocking => Style::new().fg(Color::Red),
        Urgency::Today => Style::new().fg(Color::Yellow),
        _ => Style::new().fg(DIM),
    }
}

/// A word for a state other than open, and its colour.
fn state_tag(state: State) -> Option<(&'static str, Style)> {
    Some(match state {
        State::Open => return None,
        State::Parked => ("parked", Style::new().fg(Color::Magenta)),
        State::Answered => ("answered", Style::new().fg(Color::Green)),
        State::PickedUp => ("picked up", Style::new().fg(Color::Blue)),
        State::Sent => ("sent", Style::new().fg(Color::Blue)),
        State::Closed => ("closed", Style::new().fg(DIM)),
        State::Withdrawn => ("withdrawn", Style::new().fg(DIM)),
        _ => ("unknown", Style::new().fg(DIM)),
    })
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect, side_by_side: bool) {
    draw_title(frame, area, "List", region(app) == Region::List);
    let shown = app.shown(app.filter);
    // The briefings' tab heads its answered briefings with a line that
    // says how many there are, and how many no agent has picked up yet,
    // and folds them (IN-14).
    let group = match app.filter {
        Filter::Briefings => {
            let (open, answered) = app.briefings();
            let waiting = answered
                .iter()
                .filter(|(_, case)| case.state() == State::Answered)
                .count();
            (!answered.is_empty()).then_some((open.len(), answered.len(), waiting))
        }
        _ => None,
    };
    if shown.is_empty() && group.is_none() {
        let text = match app.filter {
            Filter::Inbox => "nothing is waiting on you.",
            Filter::Briefings => "no briefings are waiting on you.",
            _ => "no cases here.",
        };
        let mut paras = vec![Para::plain(vec![Span::styled(text, Style::new().fg(DIM))])];
        if !side_by_side && app.filter == Filter::Inbox {
            paras.push(Para::blank());
            zero_paras(app, &mut paras);
        }
        let inner = inset(area, 2, 1);
        frame.render_widget(
            Paragraph::new(wrap::wrap(paras, usize::from(inner.width))),
            inner,
        );
        return;
    }
    let width = usize::from(area.width.saturating_sub(2));
    let mut items: Vec<ListItem> = shown
        .iter()
        .map(|(key, case)| list_item(app, *key, case, width))
        .collect();
    let mut targets: Vec<Hit> = shown.iter().map(|(key, _)| Hit::Row(*key)).collect();
    let mut selected = app.selected_index();
    if let Some((open, answered, waiting)) = group {
        let (arrow, hint) = if app.answered == Group::Folded {
            ("▸", "f to show")
        } else {
            ("▾", "f to fold")
        };
        items.insert(
            open,
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(format!("{arrow} "), Style::new().fg(Color::Green)),
                    Span::styled(
                        format!("{answered} answered"),
                        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if waiting > 0 {
                            format!(" · {waiting} not picked up")
                        } else {
                            String::new()
                        },
                        Style::new().fg(Color::Green),
                    ),
                    Span::styled(format!("  {hint}"), Style::new().fg(DIM)),
                ]),
                Line::raw(""),
            ])),
        );
        selected = selected.map(|at| if at >= open { at + 1 } else { at });
        targets.insert(open, Hit::Fold);
    }
    let heights: Vec<usize> = items.iter().map(ListItem::height).collect();
    let list = List::new(items)
        .highlight_style(Style::new().bg(SELECTED_BG))
        .highlight_symbol(Line::styled("▌", Style::new().fg(ACCENT)))
        .highlight_spacing(HighlightSpacing::Always)
        .block(Block::new().padding(Padding::top(1)));
    let mut state = ListState::default()
        .with_offset(app.list_offset)
        .with_selected(selected);
    frame.render_stateful_widget(list, area, &mut state);
    app.list_offset = state.offset();
    // Each row as drawn, below the padding (TU-12).
    app.hits.push((area, Hit::List));
    let mut y = area.y.saturating_add(1);
    for (height, hit) in heights.iter().zip(targets).skip(app.list_offset) {
        if y >= area.bottom() {
            break;
        }
        let height = u16::try_from(*height)
            .unwrap_or(u16::MAX)
            .min(area.bottom() - y);
        app.hits
            .push((Rect::new(area.x, y, area.width, height), hit));
        y += height;
    }
}

fn list_item(app: &App, key: Key, case: &Case, width: usize) -> ListItem<'static> {
    let done = matches!(case.state(), State::Closed | State::Withdrawn);
    let title_style = if done {
        Style::new().fg(DIM)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let (glyph, glyph_style) = match case.state() {
        State::Parked => ("◌", urgency_style(case.urgency())),
        State::Closed => ("✓", Style::new().fg(DIM)),
        State::Withdrawn => ("✕", Style::new().fg(DIM)),
        State::Answered | State::PickedUp | State::Sent => ("○", Style::new().fg(Color::Blue)),
        _ => ("●", urgency_style(case.urgency())),
    };
    let first = Line::from(vec![
        Span::styled(format!("{glyph} "), glyph_style),
        Span::styled(
            truncate(&safe::clean(case.title()), width.saturating_sub(4)),
            title_style,
        ),
    ]);

    let dim = Style::new().fg(DIM);
    let age = output::age(case.updated_at(), app.now);
    let mut meta = vec![
        Span::raw("  "),
        Span::styled(key.to_string(), dim),
        Span::styled(" · ", dim),
        Span::styled(case.kind().as_str(), dim),
    ];
    if let Some((tag, style)) = state_tag(case.state())
        && !done
    {
        meta.push(Span::styled(" · ", dim));
        meta.push(Span::styled(tag, style));
    }
    if case.kind() == Kind::Briefing && !done {
        // Only where it leaves room for the age.
        let (mark, style) = landed_mark(case);
        let used: usize = meta.iter().map(Span::width).sum();
        if used + 3 + mark.len() + age.len() + 2 <= width {
            meta.push(Span::styled(" · ", dim));
            meta.push(Span::styled(mark, style));
        }
    }
    if !case.worker().is_empty() {
        let used: usize = meta.iter().map(Span::width).sum();
        let room = width.saturating_sub(used + age.len() + 5);
        if room > 3 {
            meta.push(Span::styled(" · ", dim));
            meta.push(Span::styled(
                truncate(&safe::clean(case.worker()), room),
                dim,
            ));
        }
    }
    let used: usize = meta.iter().map(Span::width).sum();
    meta.push(Span::raw(
        " ".repeat(width.saturating_sub(used + age.len() + 1)),
    ));
    meta.push(Span::styled(age, dim));
    ListItem::new(Text::from(vec![first, Line::from(meta), Line::raw("")]))
}

/// A briefing's mark: whether its work has landed, or waits on the answer
/// to land (IN-14).
fn landed_mark(case: &Case) -> (&'static str, Style) {
    if case.landed() {
        ("landed", Style::new().fg(Color::Green))
    } else {
        ("waiting to land", Style::new().fg(Color::Yellow))
    }
}

/// `text` cut to `width` terminal columns, ending in `…` when cut, or to
/// nothing when there is no room even for that. A wide glyph that would
/// cross the edge is left out, not split.
fn truncate(text: &str, width: usize) -> String {
    // Each grapheme as the buffer draws it: where it ends in `text`, and
    // the columns it takes.
    let span = Span::raw(text);
    let mut at = 0;
    let graphemes: Vec<(usize, usize)> = span
        .styled_graphemes(Style::new())
        .map(|g| {
            at += text[at..].find(g.symbol).unwrap_or_default() + g.symbol.len();
            (at, usize::from(g.symbol.cell_width()))
        })
        .collect();
    if graphemes.iter().map(|(_, w)| w).sum::<usize>() <= width {
        return text.to_string();
    }
    let Some(room) = width.checked_sub(1) else {
        return String::new();
    };
    let mut used = 0;
    let mut end = 0;
    for (at, w) in graphemes {
        used += w;
        if used > room {
            break;
        }
        end = at;
    }
    let mut cut = text[..end].to_string();
    cut.push('…');
    cut
}

fn inset(area: Rect, x: u16, top: u16) -> Rect {
    Rect {
        x: area.x + x.min(area.width),
        y: area.y + top.min(area.height),
        width: area.width.saturating_sub(x * 2),
        height: area.height.saturating_sub(top),
    }
}

fn section(paras: &mut Vec<Para>, name: &str) {
    paras.push(Para::blank());
    paras.push(Para::plain(vec![Span::styled(
        name.to_uppercase(),
        Style::new().fg(DIM).add_modifier(Modifier::BOLD),
    )]));
}

fn dim_inline(text: &str) -> Vec<Span<'static>> {
    markdown::inline(text)
        .into_iter()
        .map(|s| s.patch_style(Style::new().fg(DIM)))
        .collect()
}

fn ago(age: &str) -> String {
    if age == "now" {
        "just now".into()
    } else {
        format!("{age} ago")
    }
}

/// Who an actor is, for people (IN-02): the name, the id when unnamed, and
/// "deleted user" when the actors table no longer has it.
fn who(app: &App, actor: Option<&cases_core::Actor>) -> Option<String> {
    let actor = actor?;
    Some(app.actors.name(actor).unwrap_or(&actor.id).to_string())
}

/// What an answer said: its response, each row's verdict, then its text and
/// note.
fn answer_paras(case: &Case, answer: &Answer, paras: &mut Vec<Para>) {
    let good = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
    let bad = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
    let held = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let bar = || vec![Span::styled("▍ ", Style::new().fg(Color::Green))];
    let mut said: Vec<Span<'static>> = Vec::new();
    if answer.drop {
        said.push(Span::styled("dropped", bad));
    } else if answer.ack {
        said.push(Span::styled("acknowledged", good));
    } else if answer.choice > 0 {
        let option = usize::try_from(answer.choice - 1)
            .ok()
            .and_then(|at| case.options().get(at))
            .cloned()
            .unwrap_or_default();
        said.push(Span::styled(format!("chose {}  ", answer.choice), good));
        said.extend(markdown::inline(&option));
    } else if answer.other {
        said.push(Span::styled("other", good));
    } else if answer.signoff == "accept" {
        said.push(Span::styled("accepted", good));
    } else if answer.signoff == "changes" {
        said.push(Span::styled("asked for changes", held));
    } else if answer.briefing == "accept" {
        said.push(Span::styled("accepted", good));
    } else if answer.briefing == "send_back" {
        said.push(Span::styled("sent back", held));
    }
    if !said.is_empty() {
        paras.push(Para::hanging(bar(), said));
    }
    for verdict in &answer.rows {
        let style = match verdict.verdict.as_str() {
            "approve" => good,
            "reject" => bad,
            _ => held,
        };
        let human = case
            .rows()
            .iter()
            .any(|row| row.id == verdict.id && row.by == "human");
        let mut spans = vec![Span::styled(
            format!("{} {}", verdict.id, verdict_word(&verdict.verdict, human)),
            style,
        )];
        if !verdict.note.is_empty() {
            spans.push(Span::raw("  "));
            spans.extend(markdown::inline(&verdict.note));
        }
        paras.push(Para::hanging(bar(), spans));
    }
    for n in &answer.file {
        let item = usize::try_from(n - 1)
            .ok()
            .and_then(|at| case.left_out().get(at))
            .cloned()
            .unwrap_or_default();
        let mut spans = vec![Span::styled(format!("file {n}  "), good)];
        spans.extend(markdown::inline(&item));
        paras.push(Para::hanging(bar(), spans));
    }
    for text in [&answer.text, &answer.note] {
        for line in text.trim().lines() {
            paras.push(Para::hanging(bar(), markdown::inline(line)));
        }
    }
}

/// For a case the agent's note reopened: the last answer and the notes
/// since it, unless an amend since has changed the question (IN-02).
fn follow_up(case: &Case) -> Option<(&Answer, Vec<&str>)> {
    if case.state() != State::Open {
        return None;
    }
    let events = case.events();
    let at = events
        .iter()
        .rposition(|e| matches!(e.record, Record::Answer(_)))?;
    let Record::Answer(answer) = &events[at].record else {
        return None;
    };
    let mut notes = Vec::new();
    for event in &events[at + 1..] {
        match &event.record {
            Record::Note(note) => notes.push(note.body.as_str()),
            Record::Amend(_) => return None,
            _ => {}
        }
    }
    (!notes.is_empty()).then_some((answer, notes))
}

fn draw_case(frame: &mut Frame, app: &mut App, area: Rect, verbs: &[Verb], buttons: Option<Flow>) {
    app.hits.push((area, Hit::Case));
    draw_title(
        frame,
        Rect {
            x: area.x.saturating_add(1),
            width: area.width.saturating_sub(1),
            ..area
        },
        "Case",
        region(app) == Region::Case,
    );
    let inner = inset(area, 2, 1);
    // The form or the message, fixed at the bottom of the pane.
    let selected = app.selected_case().map(|(key, _)| *key);
    let panel = if let Some(compose) = &app.compose {
        Some(compose_paras(compose, &app.listeners))
    } else {
        app.form
            .as_ref()
            .filter(|form| Some(form.key) == selected && form.act != Act::Answer)
            .map(form_paras)
    };
    let buttons = buttons.filter(|_| panel.is_none());
    let inner = match buttons {
        Some(buttons) => {
            let tall = u16::try_from(buttons.0.len()).unwrap_or(u16::MAX);
            let [case_area, _, buttons_area] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(tall),
            ])
            .areas(inner);
            let hits = draw_flow(frame, buttons, buttons_area);
            app.hits.extend(hits);
            case_area
        }
        None => inner,
    };
    let inner = match panel {
        Some(paras) => {
            let (lines, hits) = wrap::wrap_hits(paras, usize::from(inner.width));
            let height = u16::try_from(lines.len() + 1)
                .unwrap_or(u16::MAX)
                .min(inner.height * 2 / 3);
            let [case_area, panel_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(height)]).areas(inner);
            let rule = Line::styled(
                "─".repeat(usize::from(panel_area.width)),
                Style::new().fg(ACCENT),
            );
            frame.render_widget(
                Paragraph::new(std::iter::once(rule).chain(lines).collect::<Vec<_>>()),
                panel_area,
            );
            // Below the rule.
            let below = Rect {
                y: panel_area.y.saturating_add(1),
                height: panel_area.height.saturating_sub(1),
                ..panel_area
            };
            line_hits(app, below, &hits, 0);
            case_area
        }
        None => inner,
    };
    let Some((key, case)) = app.selected_case() else {
        let mut paras = Vec::new();
        if app.filter == Filter::Inbox {
            zero_paras(app, &mut paras);
        }
        frame.render_widget(
            Paragraph::new(wrap::wrap(paras, usize::from(inner.width))),
            inner,
        );
        return;
    };
    let mut paras = case_paras_in(app, *key, case, usize::from(inner.width));
    // The title and the meta stay at the top, with the bar under them, and
    // the rest scrolls; on a pane with no room for that and some of the
    // rest, the bar and then the title give way, as [`case_fit`] says.
    let (bar, fixed) = case_fit(app, verbs, inner);
    let rest = if fixed {
        paras.split_off(2.min(paras.len()))
    } else {
        std::mem::take(&mut paras)
    };
    let head = wrap::wrap(paras, usize::from(inner.width));
    let tall = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
    let [head_area, bar_area, inner] = Layout::vertical([
        Constraint::Length(tall(head.len())),
        // The bar, and a row under it.
        Constraint::Length(bar.as_ref().map_or(0, |(lines, _)| tall(lines.len() + 1))),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(head), head_area);
    if let Some(bar) = bar {
        // A row between the title and the bar, or under the bar.
        let bar_area = Rect {
            y: bar_area
                .y
                .saturating_add(u16::from(fixed))
                .min(bar_area.bottom()),
            ..bar_area
        };
        let hits = draw_flow(frame, bar, bar_area);
        app.hits.extend(hits);
    }
    let (lines, hits) = wrap::wrap_hits(rest, usize::from(inner.width));
    let max =
        u16::try_from(lines.len().saturating_sub(usize::from(inner.height))).unwrap_or(u16::MAX);
    app.scroll = app.scroll.min(max);
    let reveal = app
        .form
        .as_mut()
        .is_some_and(|form| std::mem::take(&mut form.reveal));
    if reveal && let Some((first, last)) = reveal_lines(app, &hits) {
        app.scroll = in_sight(app.scroll, inner.height, first, last).min(max);
    }
    line_hits(app, inner, &hits, usize::from(app.scroll));
    let more = app.scroll < max;
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), inner);
    if more && inner.height > 0 {
        let last = Rect {
            x: inner.right().saturating_sub(8),
            y: inner.bottom() - 1,
            width: 8.min(inner.width),
            height: 1,
        };
        frame.render_widget(Clear, last);
        frame.render_widget(
            Line::styled(" more ↓", Style::new().fg(DIM)).right_aligned(),
            last,
        );
    }
}

/// The case as paragraphs (TU-03).
fn case_paras(app: &App, key: Key, case: &Case) -> Vec<Para> {
    case_paras_in(app, key, case, usize::MAX)
}

/// The case as paragraphs, to be drawn `width` columns wide: its answer
/// lays out to fit.
fn case_paras_in(app: &App, key: Key, case: &Case, width: usize) -> Vec<Para> {
    let dim = Style::new().fg(DIM);
    let mut paras: Vec<Para> = vec![Para::plain(vec![Span::styled(
        case.title().to_string(),
        Style::new().add_modifier(Modifier::BOLD),
    )])];

    let sep = || Span::styled(" · ", dim);
    let mut meta = vec![
        Span::styled(
            format!("● {}", case.urgency().as_str()),
            urgency_style(case.urgency()),
        ),
        sep(),
        Span::raw(case.kind().as_str()),
    ];
    if let Some((tag, style)) = state_tag(case.state()) {
        meta.push(sep());
        meta.push(Span::styled(tag, style.add_modifier(Modifier::BOLD)));
    }
    if case.kind() == Kind::Briefing {
        let (mark, style) = landed_mark(case);
        meta.push(sep());
        meta.push(Span::styled(mark, style.add_modifier(Modifier::BOLD)));
    }
    if case.amend_seq() > 0 {
        meta.push(sep());
        meta.push(Span::styled("amended", Style::new().fg(Color::Yellow)));
    }
    if !case.worker().is_empty() {
        meta.push(sep());
        let to = if case.kind() == Kind::Message {
            "to "
        } else {
            ""
        };
        meta.push(Span::styled(
            format!("{to}{}", case.worker()),
            Style::new().fg(ACCENT),
        ));
    }
    for label in case.labels() {
        meta.push(Span::styled(
            format!(" #{label}"),
            Style::new().fg(Color::Magenta),
        ));
    }
    meta.push(sep());
    meta.push(Span::styled(
        ago(&output::age(case.opened_at(), app.now)),
        dim,
    ));
    meta.push(sep());
    meta.push(Span::styled(key.to_string(), dim));
    paras.push(Para::plain(meta));
    for (name, value) in [
        ("for", case.for_()),
        ("re", case.re()),
        ("brief", case.brief()),
        ("context", case.context()),
    ] {
        if !value.is_empty() {
            let value = if name == "re" {
                format!("case:{value}")
            } else {
                value.to_string()
            };
            paras.push(Para::hanging(
                vec![Span::styled(format!("{name:<8} "), dim)],
                vec![Span::styled(value, dim)],
            ));
        }
    }

    let follow = follow_up(case);
    if let Some((answer, notes)) = &follow {
        section(&mut paras, "Follow-up, new since your answer");
        for note in notes {
            paras.extend(markdown::render(note.trim()));
        }
        section(&mut paras, "Your last answer");
        answer_paras(case, answer, &mut paras);
    }

    let body = case.body().trim();
    if !body.is_empty() {
        if follow.is_some() {
            section(&mut paras, "The question");
        } else {
            paras.push(Para::blank());
        }
        paras.extend(markdown::render(body));
    }

    // An open case's options and left-out items are in its answer's
    // panel, at its foot, to choose and tick.
    let answerable = Act::Answer.fits(case);
    if !answerable {
        options_paras(case, &mut paras);
    }
    rows_paras(case, &mut paras);
    if !answerable {
        left_out_paras(case, &mut paras);
    }
    if !case.links().is_empty() {
        section(&mut paras, "Links");
        for link in case.links() {
            paras.push(Para::plain(vec![Span::styled(
                link.clone(),
                Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED),
            )]));
        }
    }
    if let Some(answer) = case.answer() {
        section(&mut paras, "Answer");
        answer_paras(case, answer, &mut paras);
        let by = who(app, answer.actor.as_ref()).unwrap_or_else(|| "the human".into());
        paras.push(Para::plain(vec![Span::styled(
            format!("  {by}, {}", ago(&output::age(answer.answered_at, app.now))),
            dim,
        )]));
    }
    if let Some(close) = case.close()
        && !close.outcome.trim().is_empty()
    {
        section(&mut paras, "Outcome");
        paras.extend(markdown::render(close.outcome.trim()));
    }
    thread_paras(app, case, &mut paras);
    if answerable {
        let fresh;
        let form = if let Some(form) = app.answer_of(key) {
            form
        } else {
            fresh = Form::new(key, case, Act::Answer);
            &fresh
        };
        let focused = app
            .form
            .as_ref()
            .is_some_and(|f| f.key == key && f.act == Act::Answer);
        let last = follow.map(|(answer, _)| answer.choice);
        paras.extend(panel_paras(form, case, focused, last, width));
    }
    paras
}

/// The answer's panel at the foot of an open case (TU-05, TU-12): what
/// its kind asks for, as radio rows to click, then the note. Its buttons
/// are kept in sight under the case, by [`panel_chips`]. The part the
/// keys are on, while they are in the panel, is pointed at.
fn panel_paras(
    form: &Form,
    case: &Case,
    focused: bool,
    last: Option<i64>,
    width: usize,
) -> Vec<Para> {
    let dim = Style::new().fg(DIM);
    let spot = focused.then(|| form.spot());
    let mut paras = vec![Para::blank()];
    paras.push(Para::plain(region_title("YOUR ANSWER", focused)));
    // A radio row: pointed at while the keys are on it, its dot filled
    // while it is chosen.
    let radio = |here: bool, chosen: bool, label: Vec<Span<'static>>| {
        let dot = if chosen {
            Span::styled(
                "● ",
                Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("○ ", dim)
        };
        let pointer = Span::styled(if here { "▸ " } else { "  " }, Style::new().fg(ACCENT));
        Para::hanging(vec![pointer, dot], label)
    };
    let on_choices = spot == Some(Spot::Choices);
    match form.kind {
        Kind::Decision => {
            let at = form.choice.or(form.other.then_some(form.options() + 1));
            for (n, option) in (1i64..).zip(case.options()) {
                let mut spans = vec![Span::styled(
                    format!("{n}  "),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                )];
                spans.extend(markdown::inline(option));
                if n == case.recommend() {
                    spans.push(Span::styled(
                        "  ★ recommended",
                        Style::new().fg(Color::Yellow),
                    ));
                }
                if last == Some(n) {
                    spans.push(Span::styled(
                        "  your last pick",
                        Style::new().fg(Color::Green),
                    ));
                }
                let here = on_choices && at.unwrap_or(1) == n;
                paras.push(radio(here, form.choice == Some(n), spans).on(Hit::Choice(n)));
            }
            let other = form.options() + 1;
            let spans = vec![
                Span::styled(
                    format!("{other}  "),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw("Other, see note"),
            ];
            let here = on_choices && at == Some(other);
            paras.push(radio(here, form.other, spans).on(Hit::Choice(other)));
        }
        Kind::Signoff | Kind::Briefing => {
            let (picked, marks) = if form.kind == Kind::Signoff {
                (
                    form.signoff,
                    [
                        ("accept", "accept"),
                        ("changes", "request changes (note required)"),
                    ],
                )
            } else {
                (
                    form.briefing,
                    [
                        ("accept", "accept"),
                        ("send_back", "send back (reason required)"),
                    ],
                )
            };
            for (n, (mark, label)) in marks.into_iter().enumerate() {
                let here = on_choices && picked.map_or(n == 0, |p| p == mark);
                paras.push(
                    radio(here, picked == Some(mark), vec![Span::raw(label)]).on(Hit::Mark(mark)),
                );
            }
            if form.kind == Kind::Briefing && !form.file.is_empty() {
                paras.push(Para::plain(vec![Span::styled(
                    "  file what was left out",
                    dim,
                )]));
                for (at, item) in case.left_out().iter().enumerate() {
                    let ticked = form.file.get(at).copied().unwrap_or(false);
                    let here = spot == Some(Spot::Items) && form.item == at;
                    let tick = if ticked {
                        Span::styled(
                            "[x] ",
                            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::styled("[ ] ", dim)
                    };
                    let pointer =
                        Span::styled(if here { "▸ " } else { "  " }, Style::new().fg(ACCENT));
                    let mut spans = vec![Span::styled(
                        format!("{}  ", at + 1),
                        Style::new().fg(ACCENT),
                    )];
                    spans.extend(markdown::inline(item));
                    paras.push(Para::hanging(vec![pointer, tick], spans).on(Hit::LeftOut(at)));
                }
            }
        }
        Kind::Approval => {
            for (at, (row, case_row)) in form.rows.iter().zip(case.rows()).enumerate() {
                let here = spot == Some(Spot::Rows) && at == form.row;
                let pointer = Span::styled(if here { "▸ " } else { "  " }, Style::new().fg(ACCENT));
                paras.push(
                    Para::hanging(
                        vec![
                            pointer,
                            Span::styled(
                                format!("{} ", row.id),
                                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                            ),
                        ],
                        vec![Span::raw(case_row.label.clone())],
                    )
                    .on(Hit::ApprovalRow(at)),
                );
                let cells = verdict_cells(row.human);
                // On a pane too narrow for the verdicts on one line, one
                // to a line, so that each is where a click picks it.
                let one_line = cells
                    .last()
                    .is_some_and(|(x, wide, _)| usize::from(x + wide) <= width);
                let mut spans = vec![Span::raw("     ")];
                for (n, (_, _, verdict)) in cells.into_iter().enumerate() {
                    if n > 0 && one_line {
                        spans.push(Span::raw("  "));
                    }
                    let chosen = row.verdict == Some(verdict);
                    spans.push(if chosen {
                        Span::styled(
                            "● ",
                            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::styled("○ ", dim)
                    });
                    let word = verdict_word(verdict, row.human).to_string();
                    spans.push(if chosen {
                        Span::styled(word, Style::new().add_modifier(Modifier::BOLD))
                    } else {
                        Span::raw(word)
                    });
                    if !one_line {
                        let line = std::mem::replace(&mut spans, vec![Span::raw("     ")]);
                        paras.push(Para::plain(line).on(Hit::Verdict(at, verdict)));
                    }
                }
                if one_line {
                    paras.push(Para::plain(spans).on(Hit::Verdicts(at, row.human)));
                }
                let editing = focused && form.being_edited() == Some(Which::Row(at));
                if editing || !row.note.text().is_empty() {
                    paras.extend(on(
                        field_paras("  note", &row.note, editing),
                        Hit::Field(Which::Row(at)),
                    ));
                }
            }
        }
        Kind::Question | Kind::Stuck => {
            let label = if form.kind == Kind::Question {
                "reply"
            } else {
                "guidance"
            };
            let editing = focused && form.being_edited() == Some(Which::Text);
            let here = spot == Some(Spot::Field(Which::Text));
            paras.extend(on(
                field_paras_at(label, &form.text, editing, Some(here)),
                Hit::Field(Which::Text),
            ));
        }
        _ => {}
    }
    let note = match (form.kind, form.briefing) {
        (Kind::Briefing, Some("send_back")) => "reason",
        (Kind::Signoff, _) => "comment",
        _ => "note",
    };
    let editing = focused && form.being_edited() == Some(Which::Note);
    let here = spot == Some(Spot::Field(Which::Note));
    paras.extend(on(
        field_paras_at(note, &form.note, editing, Some(here)),
        Hit::Field(Which::Note),
    ));
    paras
}

/// The answer's panel's buttons, kept in sight at the foot of the case
/// pane under an open case, `width` wide; none while a park's or a
/// withdraw's form or a message is there.
fn panel_buttons(app: &App, width: u16) -> Option<Flow> {
    if app.modal() {
        return None;
    }
    let (key, case) = app
        .selected_case()
        .filter(|(_, case)| Act::Answer.fits(case))?;
    let key = *key;
    let focused = app
        .form
        .as_ref()
        .filter(|f| f.key == key && f.act == Act::Answer);
    let touched = app.answer_of(key).is_some_and(Form::touched);
    let presses = Press::of(case.kind(), touched);
    Some(flow(
        panel_chips(app, case.kind(), &presses, focused),
        width,
        " ",
        " ",
    ))
}

/// The part of the case pane's `inner` area the case scrolls in, above
/// the answer's panel's `buttons`, when they are drawn.
fn scroll_area(inner: Rect, buttons: Option<&Flow>) -> Rect {
    match buttons {
        Some((lines, _)) => {
            let tall = u16::try_from(lines.len() + 1).unwrap_or(u16::MAX);
            Rect {
                height: inner.height.saturating_sub(tall),
                ..inner
            }
        }
        None => inner,
    }
}

/// The answer's panel's buttons for a case of `kind`, each lit while the
/// pointer is on it, or the keys of the `focused` form: send, which an fyi
/// calls acknowledge, park on a stuck case, drop, and discard.
fn panel_chips(app: &App, kind: Kind, presses: &[Press], focused: Option<&Form>) -> Vec<Chip> {
    presses
        .iter()
        .copied()
        .map(|press| {
            let button = match press {
                Press::Send if kind == Kind::Fyi => {
                    Button::new("Acknowledge", Some(Hit::Press(press)))
                        .key("⏎")
                        .look(Look::Primary)
                }
                Press::Send => Button::new("Send", Some(Hit::Press(press)))
                    .key("⏎")
                    .look(Look::Primary),
                Press::Park => Button::new("Park", Some(Hit::Press(press))),
                Press::Drop => Button::new("Drop", Some(Hit::Press(press))).key("d"),
                Press::Discard => Button::new("Discard", Some(Hit::Press(press))),
            };
            let here = focused.is_some_and(|f| f.spot() == Spot::Button(press));
            Chip {
                spans: button.spans_at(button.lit(app), here),
                bar: false,
                hit: button.hit,
            }
        })
        .collect()
}

/// A decision's options, once it is no longer open: the one chosen
/// marked. An open case has them in its answer, to choose.
fn options_paras(case: &Case, paras: &mut Vec<Para>) {
    if case.options().is_empty() {
        return;
    }
    section(paras, "Options");
    let chosen = case.answer().map(|a| a.choice);
    for (n, option) in (1i64..).zip(case.options()) {
        let number = if chosen == Some(n) {
            Style::new()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        };
        let mut spans = markdown::inline(option);
        if n == case.recommend() {
            spans.push(Span::styled(
                "  ★ recommended",
                Style::new().fg(Color::Yellow),
            ));
        }
        paras.push(Para::hanging(
            vec![Span::styled(format!(" {n} "), number), Span::raw(" ")],
            spans,
        ));
    }
}

/// A briefing's left-out items, numbered as an answer files them, once it
/// is no longer open (IN-14). An open one has them in its answer, to tick.
fn left_out_paras(case: &Case, paras: &mut Vec<Para>) {
    if case.left_out().is_empty() {
        return;
    }
    section(paras, "Left out");
    for (at, item) in case.left_out().iter().enumerate() {
        paras.push(Para::hanging(
            vec![
                Span::styled(
                    format!(" {} ", at + 1),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
            ],
            markdown::inline(item),
        ));
    }
}

fn rows_paras(case: &Case, paras: &mut Vec<Para>) {
    if case.rows().is_empty() {
        return;
    }
    section(paras, "Rows");
    for row in case.rows() {
        let human = row.by == "human";
        let mut label = markdown::inline(&row.label);
        if human {
            // A step the human carries out (case i2lDf).
            label.push(Span::styled(
                "  you",
                Style::new().fg(DIM).add_modifier(Modifier::BOLD),
            ));
        }
        paras.push(Para::hanging(
            vec![
                Span::styled(
                    format!(" {} ", row.id),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
            ],
            label,
        ));
        let indent = || vec![Span::raw("    ")];
        if !row.note.is_empty() {
            paras.push(Para::hanging(indent(), dim_inline(&row.note)));
        }
        if !row.link.is_empty() {
            paras.push(Para::hanging(
                indent(),
                vec![Span::styled(
                    row.link.clone(),
                    Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED),
                )],
            ));
        }
        // The human's row says the step in words, so it has no prompt.
        let prompt = if human { "      " } else { "    $ " };
        for script in row.script.trim_end().lines() {
            paras.push(Para {
                hit: None,
                first: vec![Span::styled(prompt, Style::new().fg(DIM))],
                rest: vec![Span::raw("      ")],
                spans: vec![Span::styled(
                    script.to_string(),
                    Style::new().fg(Color::Yellow),
                )],
            });
        }
    }
}

fn thread_paras(app: &App, case: &Case, paras: &mut Vec<Para>) {
    let dim = Style::new().fg(DIM);
    section(paras, "Thread");
    let indent = || vec![Span::raw(" ".repeat(12))];
    for (event, said) in describe::thread(case, &app.actors) {
        let author = event.author.as_str();
        let author_style = if author == "human" {
            Style::new().fg(Color::Green)
        } else {
            Style::new().fg(ACCENT)
        };
        // describe names the actor only when the table has a name; the
        // thread names it by id otherwise (IN-02).
        let by = who(app, event.actor()).map(|name| format!("  by {name}"));
        paras.push(Para::plain(vec![
            Span::styled(format!("{:>4}  ", output::age(event.at(), app.now)), dim),
            Span::styled(format!("{author:<6}"), author_style),
            Span::styled(
                event.event().as_str().to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled(by.unwrap_or_default(), dim),
        ]));
        for line in said.into_iter().filter(|l| !l.text.starts_with("by ")) {
            for text in line.text.lines() {
                paras.push(Para::hanging(indent(), dim_inline(text)));
            }
            let (field, text) = match &line.previous {
                Some(Previous::Body(text)) => ("body", text),
                Some(Previous::Context(text)) => ("context", text),
                None => continue,
            };
            paras.push(Para::hanging(
                indent(),
                vec![Span::styled(format!("previous {field}:"), dim)],
            ));
            let quote = || vec![Span::raw(" ".repeat(12)), Span::styled("▎ ", dim)];
            for text in text.trim().lines() {
                paras.push(Para {
                    hit: None,
                    first: quote(),
                    rest: quote(),
                    spans: vec![Span::styled(text.to_string(), dim)],
                });
            }
        }
    }
    let warn = Style::new().fg(Color::Yellow);
    for problem in case.problems() {
        paras.push(Para::hanging(
            vec![Span::styled("   ?  ", warn)],
            vec![Span::styled(
                format!(
                    "an event this cases cannot read, perhaps from a newer version: {}",
                    output::problem(problem)
                ),
                warn,
            )],
        ));
    }
}

/// Inbox zero (IN-01): what is still out, with the parts that are empty
/// left out.
fn zero_paras(app: &App, paras: &mut Vec<Para>) {
    paras.push(Para::plain(vec![Span::styled(
        "nothing needs you.",
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
    )]));
    if !app.listeners.is_empty() {
        section(paras, "Listening now");
        for listening in &app.listeners {
            let mut spans = vec![Span::raw(listening.worker.clone())];
            for label in &listening.labels {
                spans.push(Span::styled(
                    format!(" #{label}"),
                    Style::new().fg(Color::Magenta),
                ));
            }
            if let Some(doing) = &listening.doing {
                spans.push(Span::styled(format!("  {doing}"), Style::new().fg(DIM)));
            }
            paras.push(Para::hanging(vec![Span::raw("  ")], spans));
        }
    }
    let (sent, with_agent): (Vec<_>, Vec<_>) = app
        .shown(Filter::Flight)
        .into_iter()
        .partition(|(_, case)| case.state() == State::Sent);
    for (name, cases) in [
        ("Messages not picked up", sent),
        ("Still with an agent", with_agent),
    ] {
        if cases.is_empty() {
            continue;
        }
        section(paras, name);
        for (key, case) in cases {
            let (tag, style) = state_tag(case.state()).unwrap_or(("", Style::new()));
            paras.push(Para::hanging(
                vec![Span::styled(format!("{tag:<10} "), style)],
                vec![
                    Span::raw(case.title().to_string()),
                    Span::styled(format!("  {key}"), Style::new().fg(DIM)),
                ],
            ));
        }
    }
}

/// The form under the case (TU-05).
fn form_paras(form: &Form) -> Vec<Para> {
    let heading = match form.act {
        Act::Answer => format!("ANSWER {}", form.key),
        Act::Park => format!("PARK {}", form.key),
        Act::Withdraw => format!("WITHDRAW {}", form.key),
    };
    let mut paras = vec![Para::plain(vec![Span::styled(
        heading,
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    )])];
    let note = match form.act {
        // Sending a briefing back gives the reason in the note.
        Act::Answer if form.briefing == Some("send_back") => "reason",
        Act::Answer => "note",
        Act::Park => "why",
        Act::Withdraw => "reason",
    };
    paras.extend(on(
        field_paras(note, &form.note, form.being_edited() == Some(Which::Note)),
        Hit::Field(Which::Note),
    ));
    paras
}

/// The message being written (TU-07): who it is to, with the sessions
/// listening now to pick from, its title and its body.
fn compose_paras(compose: &Compose, listeners: &[Listening]) -> Vec<Para> {
    let dim = Style::new().fg(DIM);
    let heading = match compose.editing {
        Some((key, _)) => format!("EDIT MESSAGE {key}"),
        None => "MESSAGE".into(),
    };
    let mut paras = vec![Para::plain(vec![
        Span::styled(
            heading,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", compose.urgency.as_str()),
            urgency_style(compose.urgency),
        ),
    ])];
    paras.extend(field_paras(
        "to",
        &compose.to,
        compose.part == Some(Part::To),
    ));
    if compose.editing.is_none() {
        if listeners.is_empty() {
            paras.push(Para::hanging(
                vec![Span::raw(" ".repeat(9))],
                vec![Span::styled(
                    "no session is listening now: a worker name, or #label",
                    dim,
                )],
            ));
        }
        for (n, listening) in (1..).zip(listeners.iter().take(9)) {
            let mut spans = vec![
                Span::styled(
                    format!("{n} "),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(listening.worker.clone()),
            ];
            for label in &listening.labels {
                spans.push(Span::styled(
                    format!(" #{label}"),
                    Style::new().fg(Color::Magenta),
                ));
            }
            if let Some(doing) = &listening.doing {
                spans.push(Span::styled(format!("  {doing}"), dim));
            }
            paras.push(Para::hanging(vec![Span::raw(" ".repeat(9))], spans));
        }
    }
    if compose.editing.is_none()
        && compose.title.text().is_empty()
        && compose.part != Some(Part::Title)
    {
        let from_body = compose
            .title_to_send()
            .map_or("from the body's first line".to_string(), |t| {
                format!("{t}  (from the body)")
            });
        paras.push(Para::hanging(
            vec![Span::styled(format!("{:<9}", "title"), dim)],
            vec![Span::styled(from_body, dim)],
        ));
    } else {
        paras.extend(field_paras(
            "title",
            &compose.title,
            compose.part == Some(Part::Title),
        ));
    }
    paras.extend(field_paras(
        "body",
        &compose.body,
        compose.part == Some(Part::Body),
    ));
    paras
}

/// `paras`, a click on any of them doing `hit`.
fn on(paras: Vec<Para>, hit: Hit) -> Vec<Para> {
    paras.into_iter().map(|para| para.on(hit)).collect()
}

/// A field under its label, with the cursor drawn while it has the keys.
fn field_paras(label: &str, field: &Field, editing: bool) -> Vec<Para> {
    field_paras_at(label, field, editing, None)
}

/// [`field_paras`], in the answer's panel, after a column that points at
/// it while the keys are on it, when `pointed` says so, as at an option.
fn field_paras_at(label: &str, field: &Field, editing: bool, pointed: Option<bool>) -> Vec<Para> {
    let pointer = |first: bool| {
        pointed.map(|here| {
            Span::styled(
                if here && first { "▸ " } else { "  " },
                Style::new().fg(ACCENT),
            )
        })
    };
    let dim = Style::new().fg(DIM);
    let cursor = Style::new().add_modifier(Modifier::REVERSED);
    let label_style = if editing {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        dim
    };
    let text = field.text();
    if text.is_empty() && !editing {
        return vec![Para::hanging(
            pointer(true)
                .into_iter()
                .chain([Span::styled(format!("{label:<9}"), label_style)])
                .collect(),
            vec![Span::styled("empty", dim)],
        )];
    }
    let mut paras = Vec::new();
    let mut at = 0;
    for (n, line) in text.split('\n').enumerate() {
        let len = line.chars().count();
        let mut spans = Vec::new();
        if editing && (at..=at + len).contains(&field.cursor()) {
            let split = field.cursor() - at;
            let before: String = line.chars().take(split).collect();
            let mut rest = line.chars().skip(split);
            let under = rest.next().map_or(" ".to_string(), String::from);
            spans.push(Span::raw(before));
            spans.push(Span::styled(under, cursor));
            spans.push(Span::raw(rest.collect::<String>()));
        } else {
            spans.push(Span::raw(line.to_string()));
        }
        let first = if n == 0 {
            format!("{label:<9}")
        } else {
            " ".repeat(9)
        };
        paras.push(Para::hanging(
            pointer(n == 0)
                .into_iter()
                .chain([Span::styled(first, label_style)])
                .collect(),
            spans,
        ));
        at += len + 1;
    }
    paras
}

/// The footer: the keys, over as many lines as `width` needs, each a
/// button a click presses (TU-12); or a line the screen says, wrapped.
fn footer_lines(app: &App, verbs: &[Verb], width: u16, side_by_side: bool, bar: bool) -> Flow {
    let said = |text: String, style: Style| {
        let para = Para::hanging(vec![Span::raw(" ")], vec![Span::styled(text, style)]);
        (wrap::wrap(vec![para], usize::from(width)), Vec::new())
    };
    // The question is in a dialog, with its buttons: the keys under it
    // would only answer no.
    if app.asking() {
        return said("y yes, any other key no".into(), Style::new().fg(DIM));
    }
    if let Some(line) = app.sending_line() {
        return said(line, Style::new().fg(Color::Yellow));
    }
    // The keys that have a button on the screen are left to it: an answer
    // and a park in the case's bar, a message at the right of the tabs. An
    // answer is still offered when no button for it is drawn, as on a
    // narrow screen showing the list.
    let answer_shown = bar && verbs.contains(&Verb::Answer);
    let park_shown = bar && verbs.contains(&Verb::Park);
    let buttoned = |chip: &(&str, &str)| {
        (chip.0 == "a" && answer_shown)
            || (chip.0 == "p" && park_shown)
            || (chip.0 == "m" && app.only.is_none())
    };
    let open = app.form.is_some() || app.compose.is_some();
    let mut keys: Vec<(&str, &str)> = if let Some(form) = &app.form {
        keys_for(form, app.newline == Newline::ShiftEnter)
    } else if let Some(compose) = &app.compose {
        compose_keys(compose)
    } else {
        // An answer to give comes first: its keys, from anywhere (TU-12).
        let mut keys = app
            .selected_case()
            .filter(|(_, case)| Act::Answer.fits(case))
            .map(|(_, case)| answer_keys(case))
            .unwrap_or_default();
        let rest: &[(&str, &str)] = match (app.focus, side_by_side) {
            // A stuck case can be parked too, which ends the screen
            // (TU-11).
            _ if app.only.is_some() => &[("j/k", "scroll"), ("p", "park")],
            (Focus::List, true) => &[("j/k", "move"), ("m", "message"), ("[ ]", "filter")],
            (Focus::List, false) => &[
                ("j/k", "move"),
                ("→", "read"),
                ("m", "message"),
                ("[ ]", "filter"),
            ],
            (Focus::Case, _) => &[("j/k", "scroll"), ("esc", "back")],
            (Focus::Bar(_), _) => &[("⏎", "press"), ("esc", "back")],
        };
        keys.extend(rest);
        let parks = app
            .selected_case()
            .is_some_and(|(_, case)| Act::Park.fits(case));
        keys.retain(|key| *key != ("p", "park") || parks);
        keys.extend([
            ("s", "stats"),
            ("^k", "commands"),
            ("?", "keys"),
            ("q", "quit"),
        ]);
        keys
    };
    // One case's screen has no strip.
    if app.only.is_some() {
        keys.retain(|key| *key != ("s", "stats"));
    }
    let keys = &keys;
    // s does nothing on a screen with no room for the strip: not offered.
    let lead = focus_line(app, verbs).map(|text| Chip {
        spans: vec![Span::styled(
            text,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )],
        bar: false,
        hit: None,
    });
    let after_lead = lead.is_some();
    let chips = keys
        .iter()
        .filter(|chip| app.strip.room || **chip != ("s", "stats"))
        .filter(|chip| open || !buttoned(chip))
        .enumerate()
        .map(|(n, (key, what))| {
            let look = if *what == "send" {
                Look::Primary
            } else {
                Look::Plain
            };
            let button = Button::new(what, pressed(key).map(Hit::Key))
                .key(key)
                .look(look);
            Chip {
                spans: button.spans(button.lit(app)),
                bar: n == 0 && after_lead,
                hit: button.hit,
            }
        });
    flow(lead.into_iter().chain(chips).collect(), width, " ", " ")
}

/// The keys that answer `case` from the list or the case, before the
/// rest (TU-12): a number for an option, an answer or a row, n for the
/// note, and enter into the answer.
fn answer_keys(case: &Case) -> Vec<(&'static str, &'static str)> {
    let mut keys = match case.kind() {
        // Other is the number after the last option.
        Kind::Decision => vec![(numbers(case.options().len() + 1), "choose")],
        Kind::Signoff => vec![("1", "accept"), ("2", "changes")],
        Kind::Briefing => vec![("1", "accept"), ("2", "send back")],
        Kind::Approval if !case.rows().is_empty() => vec![(numbers(case.rows().len()), "row")],
        _ => Vec::new(),
    };
    keys.extend([("n", "note"), ("⏎", "answer")]);
    keys
}

/// The numbers from 1 to `n`, as far as 9, as the footer names them.
fn numbers(n: usize) -> &'static str {
    const NUMBERS: [&str; 9] = ["1", "1-2", "1-3", "1-4", "1-5", "1-6", "1-7", "1-8", "1-9"];
    NUMBERS[n.clamp(1, 9) - 1]
}

/// Where the keys are, and where tab takes them next, as the footer leads
/// with it (TU-12): "on: option 2 · tab → note". None while a park's or a
/// withdraw's form, or a message, has them.
fn focus_line(app: &App, verbs: &[Verb]) -> Option<String> {
    let home = if app.only.is_some() { "case" } else { "list" };
    let (here, next) = match &app.form {
        Some(form) if form.act == Act::Answer => {
            let spot = form.spot();
            let parts = form.parts();
            let next = parts
                .iter()
                .position(|part| *part == spot)
                .and_then(|at| parts.get(at + 1))
                .map(|part| part_name(form, *part));
            (
                spot_name(form, spot),
                Some(next.unwrap_or_else(|| home.into())),
            )
        }
        Some(_) => return None,
        None if app.compose.is_some() => return None,
        None => match app.focus {
            Focus::Bar(verb) => {
                let at = verbs.iter().position(|v| *v == verb).unwrap_or(0);
                let next = verbs.get(at + 1).map_or(home, |v| v.label());
                (verb.label().into(), Some(next.into()))
            }
            focus => {
                let here = if focus == Focus::List { "list" } else { "case" };
                let first = app.selected_case().and_then(|(key, case)| {
                    if !Act::Answer.fits(case) {
                        return verbs.first().map(|verb| verb.label().into());
                    }
                    let fresh;
                    let form = if let Some(form) = app.answer_of(*key) {
                        form
                    } else {
                        fresh = Form::new(*key, case, Act::Answer);
                        &fresh
                    };
                    form.parts().first().map(|part| part_name(form, *part))
                });
                (here.into(), first)
            }
        },
    };
    Some(match next {
        Some(next) => format!("on: {here} · tab → {next}"),
        None => format!("on: {here}"),
    })
}

/// A part of the answer as tab reaches it: its choices, rows or items as
/// a whole, or else as [`spot_name`] names it.
fn part_name(form: &Form, spot: Spot) -> String {
    match spot {
        Spot::Choices if form.kind == Kind::Decision => "options".into(),
        Spot::Choices => "answers".into(),
        Spot::Rows => "rows".into(),
        Spot::Items => "items".into(),
        _ => spot_name(form, spot),
    }
}

/// The part of the answer the keys are on, as the footer names it: the
/// option, answer, row or item pointed at, the field, or the button.
fn spot_name(form: &Form, spot: Spot) -> String {
    let row = |at: usize| form.rows.get(at).map_or("", |row| row.id.as_str());
    match spot {
        Spot::Choices if form.kind == Kind::Decision => {
            let n = pointed_choice(form);
            if n > form.options() {
                "other".into()
            } else {
                format!("option {n}")
            }
        }
        Spot::Choices => match pointed_mark(form) {
            "send_back" => "send back".into(),
            mark => mark.into(),
        },
        Spot::Rows => match form.being_edited() {
            Some(Which::Row(at)) => format!("row {}'s note", row(at)),
            _ => format!("row {}", row(form.row)),
        },
        Spot::Items => format!("item {}", form.item + 1),
        Spot::Field(Which::Text) if form.kind == Kind::Stuck => "guidance".into(),
        Spot::Field(Which::Text) => "reply".into(),
        Spot::Field(Which::Row(at)) => format!("row {}'s note", row(at)),
        Spot::Field(Which::Note) => match (form.kind, form.briefing) {
            (Kind::Briefing, Some("send_back")) => "reason".into(),
            (Kind::Signoff, _) => "comment".into(),
            _ => "note".into(),
        },
        Spot::Button(Press::Send) if form.kind == Kind::Fyi => "Acknowledge".into(),
        Spot::Button(Press::Send) => "Send".into(),
        Spot::Button(Press::Park) => "Park".into(),
        Spot::Button(Press::Drop) => "Drop".into(),
        Spot::Button(Press::Discard) => "Discard".into(),
    }
}

/// The key a footer's label names, for a click on it to press; none for a
/// label of several keys, such as j/k (TU-12).
fn pressed(label: &str) -> Option<KeyEvent> {
    let (code, modifiers) = match label {
        "⏎" => (KeyCode::Enter, KeyModifiers::NONE),
        "→" => (KeyCode::Right, KeyModifiers::NONE),
        "esc" => (KeyCode::Esc, KeyModifiers::NONE),
        "tab" => (KeyCode::Tab, KeyModifiers::NONE),
        "^s" => (KeyCode::Char('s'), KeyModifiers::CONTROL),
        "^e" => (KeyCode::Char('e'), KeyModifiers::CONTROL),
        "^k" => (KeyCode::Char('k'), KeyModifiers::CONTROL),
        _ => {
            let mut chars = label.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => (KeyCode::Char(c), KeyModifiers::NONE),
                _ => return None,
            }
        }
    };
    Some(KeyEvent::new(code, modifiers))
}

/// The keys the footer shows while a message is being written.
fn compose_keys(compose: &Compose) -> Vec<(&'static str, &'static str)> {
    match compose.part {
        Some(Part::Body) => vec![
            ("esc", "leave the field"),
            ("^s", "send"),
            ("^e", "editor"),
            ("⏎", "new line"),
        ],
        Some(_) => vec![
            ("⏎ tab", "next field"),
            ("esc", "leave the field"),
            ("^s", "send"),
        ],
        None if compose.editing.is_some() => {
            vec![("b", "body"), ("⏎", "send"), ("esc", "cancel")]
        }
        None => vec![
            ("1-9", "to a listener"),
            ("t", "to"),
            ("i", "title"),
            ("b", "body"),
            ("u", "urgency"),
            ("⏎", "send"),
            ("esc", "cancel"),
        ],
    }
}

/// The keys the footer shows while a form is open, with the key for a
/// new line in a field: shift+enter where the terminal tells it from
/// enter, and alt+enter where it does not.
fn keys_for(form: &Form, shift_enter: bool) -> Vec<(&'static str, &'static str)> {
    let line = if shift_enter { "⇧⏎" } else { "alt+⏎" };
    if form.editing.is_some() {
        return vec![
            ("⏎", "send"),
            (line, "new line"),
            ("^e", "editor"),
            ("esc", "leave the field"),
        ];
    }
    if form.act == Act::Answer {
        return panel_keys(form);
    }
    vec![("n", "edit"), ("⏎", "send"), ("esc", "cancel")]
}

/// The keys the footer shows while the answer's panel has them, out of a
/// field (TU-05): its kind's, enter to send or press the button the keys
/// are on, n for the note, those of the part the keys are on, and esc to
/// leave it. Where tab goes leads the footer.
fn panel_keys(form: &Form) -> Vec<(&'static str, &'static str)> {
    let spot = form.spot();
    let mut keys = Vec::new();
    match form.kind {
        Kind::Decision => keys.extend([
            (
                numbers(usize::try_from(form.options()).unwrap_or(9) + 1),
                "choose",
            ),
            ("o", "other"),
        ]),
        Kind::Signoff => keys.extend([("1", "accept"), ("2", "changes")]),
        Kind::Briefing if spot == Spot::Items => {
            keys.extend([("j/k", "move"), ("space", "file")]);
            match form.file.len() {
                0 => {}
                1..=9 => keys.push((numbers(form.file.len()), "file")),
                _ => keys.push(("1-9 0", "file")),
            }
        }
        Kind::Briefing => keys.extend([("1", "accept"), ("2", "send back")]),
        // With no rows, the row keys have nothing to act on.
        Kind::Approval if !form.rows.is_empty() => {
            let human = form.rows.get(form.row).is_some_and(|row| row.human);
            keys.extend([
                (numbers(form.rows.len()), "row"),
                ("y", verdict_word("approve", human)),
                ("h", "hold"),
                ("x", verdict_word("reject", human)),
                ("r", "row note"),
            ]);
        }
        Kind::Question | Kind::Stuck => keys.push(("i", "write")),
        _ => {}
    }
    // Enter sends from any part but a button, which it presses.
    if matches!(spot, Spot::Button(_)) {
        keys.push(("⏎", "press"));
    } else {
        keys.push(("⏎", "send"));
    }
    keys.push(("n", "note"));
    match spot {
        Spot::Choices if form.kind == Kind::Decision => keys.push(("j/k", "move")),
        Spot::Choices => keys.push(("j/k", "choose")),
        Spot::Rows => keys.push(("j/k", "row")),
        _ => {}
    }
    keys.push(("esc", "leave"));
    keys
}

/// The keys (TU-08).
pub(crate) const KEYS: [(&str, &str); 28] = [
    (
        "1-9",
        "answer: that option, answer (1 accept, 2 not) or row",
    ),
    ("n", "write the answer's note"),
    ("⏎", "into the answer; in it, send, or press the button"),
    ("⇧⏎ alt+⏎", "in a field: a new line (alt+⏎ in any terminal)"),
    (
        "tab ⇧tab",
        "list, then the answer's parts or the case's buttons",
    ),
    ("esc", "leave the field, then back to the list"),
    ("a", "into the answer; esc keeps it, Discard empties it"),
    ("j k ↑ ↓", "move in the list, or scroll the case"),
    ("g G", "first or last case"),
    ("l →", "read the case"),
    ("h ←", "back to the list"),
    ("space J K", "scroll the case a page"),
    ("^d ^u", "scroll the case a page"),
    ("[ ]", "previous or next filter"),
    ("f", "fold the answered briefings"),
    ("p r", "park a stuck case, or resume a parked one"),
    ("w", "withdraw an answered case, or a message"),
    ("m e", "write a message, or edit one not picked up"),
    ("^e ^s", "in a field: open $EDITOR, or send"),
    ("s", "show or hide the stats of the last 14 days"),
    ("^k", "commands: what fits now, found by name"),
    ("?", "these keys"),
    ("q ^c", "quit"),
    ("click", "a tab, a case, a button, an option or a field"),
    ("right-click", "a case: what can be done on it"),
    ("point", "lights what a click acts on"),
    ("wheel", "move in the list, or scroll the case"),
    ("⇧ drag", "select text, in most terminals"),
];

/// The keys one case's screen takes (TU-11): what [`KEYS`] lists less
/// the list's, the filters', a withdraw's, a resume's and messages'.
pub(crate) const ONE_KEYS: [(&str, &str); 19] = [
    (
        "1-9",
        "answer: that option, answer (1 accept, 2 not) or row",
    ),
    ("n", "write the answer's note"),
    ("⏎", "into the answer; in it, send, or press the button"),
    ("⇧⏎ alt+⏎", "in a field: a new line (alt+⏎ in any terminal)"),
    ("tab ⇧tab", "the case, then the answer's parts"),
    ("esc", "leave the field, then the answer"),
    ("a", "into the answer; esc keeps it, Discard empties it"),
    ("j k ↑ ↓", "scroll the case"),
    ("space J K", "scroll the case a page"),
    ("^d ^u", "scroll the case a page"),
    ("p", "park it, when it is stuck"),
    ("^e ^s", "in a field: open $EDITOR, or send"),
    ("^k", "commands: what fits now, found by name"),
    ("?", "these keys"),
    ("q ^c", "quit"),
    ("click", "a button, an option or a field"),
    ("point", "lights what a click acts on"),
    ("wheel", "scroll the case"),
    ("⇧ drag", "select text, in most terminals"),
];

fn draw_help(frame: &mut Frame, one: bool) {
    let keys: &[(&str, &str)] = if one { &ONE_KEYS } else { &KEYS };
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(
                    format!("{k:>10}  "),
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(*v),
            ])
        })
        .collect();
    let area = frame.area();
    let width = 60.min(area.width);
    let height = (u16::try_from(lines.len()).unwrap_or(0) + 4).min(area.height);
    let rect = Rect::new(
        (area.width - width) / 2,
        (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(ACCENT))
                .title(" keys ")
                .padding(Padding::new(1, 1, 1, 0)),
        ),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use cases_core::{
        Actor, ActorRow, Amend, Answer, Append, ApprovalRow, Author, Case, Close, Listening, Note,
        Open, Pickup, Record, Row, RowVerdict, State, Time, Urgency,
    };
    use cases_store_sqlite::Key;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

    use super::{
        ACCENT, DIM, Look, case_paras, compose_keys, compose_paras, field_paras, form_paras, inset,
        keys_for, panel_chips, panel_paras, question, state_tag, thread_paras, truncate,
        urgency_style, verdict_cells, verdict_word, zero_paras,
    };
    use crate::commands::tui::app::{App, Filter, Focus, Hit};
    use crate::commands::tui::compose::{Compose, Part};
    use crate::commands::tui::field::Field;
    use crate::commands::tui::form::{Act, Confirm, Form, Which};
    use crate::commands::tui::wrap::Para;
    use crate::output::Actors;

    const HOUR: i64 = 3_600;

    fn at(secs: i64) -> Time {
        Time::from_unix(secs, 0).unwrap()
    }

    fn key() -> Key {
        Key::new(7).unwrap()
    }

    /// A case the agent opened at the epoch, and its first row.
    fn open(kind: &str, with: impl FnOnce(&mut Open)) -> (Case, Vec<Row>) {
        let mut open = Open::default();
        kind.clone_into(&mut open.kind);
        "today".clone_into(&mut open.urgency);
        "The title".clone_into(&mut open.title);
        "worker".clone_into(&mut open.worker);
        with(&mut open);
        let author = if kind == "message" {
            Author::Human
        } else {
            Author::Agent
        };
        let (case, row) =
            Case::open(&Append::new(author, &Record::Open(open)), at(0), None).unwrap();
        (case, vec![row])
    }

    fn add(case: &mut (Case, Vec<Row>), author: Author, record: &Record, secs: i64) {
        let row = case
            .0
            .append(&Append::new(author, record), at(secs), None, None)
            .unwrap();
        case.1.push(row);
    }

    fn app(secs: i64) -> App {
        let mut app = App::new("cases.db".into());
        app.now = at(secs);
        app
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

    /// The style of the span that reads `content`, wherever it is.
    fn style_of(paras: &[Para], content: &str) -> Style {
        paras
            .iter()
            .flat_map(|p| p.first.iter().chain(&p.spans))
            .find(|s| s.content == content)
            .unwrap_or_else(|| panic!("no span reads {content:?}"))
            .style
    }

    fn green() -> Style {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    }

    fn dim() -> Style {
        Style::new().fg(DIM)
    }

    #[test]
    fn a_case_shows_its_meta_fields_links_and_outcome() {
        let mut case = open("decision", |open| {
            open.options = vec!["Ship".into(), "Wait".into()];
            open.recommend = 1;
            open.labels = vec!["ops".into(), "web".into()];
            "agent-1".clone_into(&mut open.for_);
            "abc12".clone_into(&mut open.re);
            "the brief".clone_into(&mut open.brief);
            "the context".clone_into(&mut open.context);
            open.links = vec!["https://x.test/pr/1".into()];
        });
        let mut amend = Amend::default();
        amend.body = "A body".into();
        amend.links = vec!["https://x.test/pr/2".into()];
        add(&mut case, Author::Agent, &Record::Amend(amend), HOUR);
        let mut answer = Answer::default();
        answer.choice = 2;
        answer.amend_seq = 2;
        add(&mut case, Author::Human, &Record::Answer(answer), 2 * HOUR);
        let mut pickup = Pickup::default();
        pickup.by = "worker".into();
        add(
            &mut case,
            Author::Agent,
            &Record::Pickup(pickup),
            2 * HOUR + 60,
        );
        let mut close = Close::default();
        close.outcome = "  Waited.  ".into();
        add(&mut case, Author::Agent, &Record::Close(close), 3 * HOUR);
        let app = app(4 * HOUR);
        let paras = case_paras(&app, key(), &case.0);
        assert_eq!(
            lines(&paras),
            [
                "The title".to_string(),
                format!(
                    "● today · decision · closed · amended · worker #ops #web · 4h ago · {}",
                    key()
                ),
                "for      agent-1".into(),
                "re       case:abc12".into(),
                "brief    the brief".into(),
                "context  the context".into(),
                String::new(),
                "A body".into(),
                String::new(),
                "OPTIONS".into(),
                " 1  Ship  ★ recommended".into(),
                " 2  Wait".into(),
                String::new(),
                "LINKS".into(),
                "https://x.test/pr/1".into(),
                "https://x.test/pr/2".into(),
                String::new(),
                "ANSWER".into(),
                "▍ chose 2  Wait".into(),
                "  the human, 2h ago".into(),
                String::new(),
                "OUTCOME".into(),
                "Waited.".into(),
                String::new(),
                "THREAD".into(),
                "  4h  agent open".into(),
                "  3h  agent amend".into(),
                "            replaced the body".into(),
                "            added link: https://x.test/pr/2".into(),
                "  2h  human answer".into(),
                "            chose 2. Wait".into(),
                "  1h  agent pickup".into(),
                "  1h  agent close".into(),
                "            Waited.".into(),
            ]
        );
        assert_eq!(style_of(&paras, "amended"), Style::new().fg(Color::Yellow));
        assert_eq!(
            style_of(&paras, "closed"),
            dim().add_modifier(Modifier::BOLD)
        );
        assert_eq!(style_of(&paras, "worker"), Style::new().fg(ACCENT));
        assert_eq!(style_of(&paras, " #ops"), Style::new().fg(Color::Magenta));
        assert_eq!(style_of(&paras, "re       "), dim());
        assert_eq!(style_of(&paras, "case:abc12"), dim());
        assert_eq!(
            style_of(&paras, "https://x.test/pr/2"),
            Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED)
        );
        // The answer's pick is marked among the options.
        assert_eq!(
            style_of(&paras, " 2 "),
            Style::new()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            style_of(&paras, " 1 "),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn a_message_says_who_it_is_to() {
        let case = open("message", |open| {
            "blocking".clone_into(&mut open.urgency);
            "Do the thing".clone_into(&mut open.body);
        });
        let paras = case_paras(&app(90), key(), &case.0);
        assert_eq!(
            lines(&paras)[..4],
            [
                "The title".to_string(),
                format!(
                    "● blocking · message · sent · to worker · 1m ago · {}",
                    key()
                ),
                String::new(),
                "Do the thing".into(),
            ]
        );
        assert_eq!(style_of(&paras, "to worker"), Style::new().fg(ACCENT));
        assert_eq!(style_of(&paras, "● blocking"), Style::new().fg(Color::Red));
    }

    #[test]
    fn a_follow_up_puts_the_notes_and_last_answer_before_the_question() {
        let mut case = open("decision", |open| {
            "The body".clone_into(&mut open.body);
            open.options = vec!["Ship".into(), "Wait".into()];
        });
        let mut answer = Answer::default();
        answer.choice = 1;
        answer.note = "go".into();
        add(&mut case, Author::Human, &Record::Answer(answer), 60);
        let mut note = Note::default();
        note.body = "  It broke.\n\n- twice  ".into();
        note.reopen = true;
        add(&mut case, Author::Agent, &Record::Note(note), 120);
        let paras = case_paras(&app(180), key(), &case.0);
        let all = lines(&paras);
        assert_eq!(
            all[2..14],
            [
                "",
                "FOLLOW-UP, NEW SINCE YOUR ANSWER",
                "It broke.",
                "",
                "• twice",
                "",
                "YOUR LAST ANSWER",
                "▍ chose 1  Ship",
                "▍ go",
                "",
                "THE QUESTION",
                "The body",
            ]
        );
        // The options are the answer's, at the foot, the last pick marked.
        let at = all
            .iter()
            .position(|line| [" YOUR ANSWER", "▍YOUR ANSWER · keys here"].contains(&line.as_str()))
            .unwrap();
        assert_eq!(
            all[at + 1..at + 3],
            ["  ○ 1  Ship  your last pick", "  ○ 2  Wait"]
        );
        assert_eq!(
            style_of(&paras, "  your last pick"),
            Style::new().fg(Color::Green)
        );
    }

    #[test]
    fn the_thread_quotes_what_an_amend_replaced() {
        let mut case = open("question", |open| {
            "Old body\nsecond line".clone_into(&mut open.body);
            "old context".clone_into(&mut open.context);
        });
        let mut amend = Amend::default();
        amend.body = "New body".into();
        amend.context = "new context".into();
        add(&mut case, Author::Agent, &Record::Amend(amend), 60);
        let mut paras = Vec::new();
        thread_paras(&app(120), &case.0, &mut paras);
        assert_eq!(
            lines(&paras),
            [
                "",
                "THREAD",
                "  2m  agent open",
                "  1m  agent amend",
                "            replaced the body",
                "            previous body:",
                "            ▎ Old body",
                "            ▎ second line",
                "            replaced the context: new context",
                "            previous context:",
                "            ▎ old context",
            ]
        );
        let quote = &paras[6];
        assert_eq!(text(&quote.rest), "            ▎ ");
        assert_eq!(quote.spans[0].style, dim());
        assert_eq!(style_of(&paras, "previous body:"), dim());
        assert_eq!(style_of(&paras, "agent "), Style::new().fg(ACCENT));
        assert_eq!(
            style_of(&paras, "amend"),
            Style::new().add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn the_thread_names_the_human_in_green_and_warns_of_a_row_it_cannot_read() {
        let mut case = open("fyi", |_| {});
        let mut answer = Answer::default();
        answer.ack = true;
        add(&mut case, Author::Human, &Record::Answer(answer), 60);
        let mut rows = case.1;
        rows.push(Row {
            seq: 3,
            author: "agent".into(),
            event: "frobnicate".into(),
            data: "{}".into(),
        });
        let case = Case::fold(&rows).unwrap();
        let mut paras = Vec::new();
        thread_paras(&app(60), &case, &mut paras);
        let lines = lines(&paras);
        assert_eq!(
            lines[..5],
            [
                "",
                "THREAD",
                "  1m  agent open",
                " now  human answer",
                "            acknowledged"
            ]
        );
        assert_eq!(lines.len(), 6);
        assert!(
            lines[5].starts_with(
                "   ?  an event this cases cannot read, perhaps from a newer version: \
                 0003-agent-frobnicate.json: "
            ),
            "{}",
            lines[5]
        );
        assert_eq!(style_of(&paras, "human "), Style::new().fg(Color::Green));
        let warn = Style::new().fg(Color::Yellow);
        assert_eq!(paras[5].first[0].style, warn);
        assert_eq!(paras[5].spans[0].style, warn);
    }

    fn form(kind: &str, act: Act, with: impl FnOnce(&mut Open)) -> Form {
        let case = open(kind, with);
        Form::new(key(), &case.0, act)
    }

    fn decision(open: &mut Open) {
        open.options = vec!["Ship".into(), "Wait".into()];
    }

    /// The answer's panel of a case of `kind`, as `edit` leaves its form,
    /// with the keys in it or not.
    fn panel(
        kind: &str,
        with: impl FnOnce(&mut Open),
        edit: impl FnOnce(&mut Form),
        focused: bool,
    ) -> Vec<Para> {
        let case = open(kind, with);
        let mut form = Form::new(key(), &case.0, Act::Answer);
        edit(&mut form);
        panel_paras(&form, &case.0, focused, None, usize::MAX)
    }

    #[test]
    fn a_decision_panel_has_its_options_as_radio_rows() {
        let paras = panel("decision", decision, |_| {}, false);
        assert_eq!(
            lines(&paras),
            [
                "",
                " YOUR ANSWER",
                "  ○ 1  Ship",
                "  ○ 2  Wait",
                "  ○ 3  Other, see note",
                "  note     empty"
            ]
        );
        // Out of the panel, its heading recedes; in it, it is accented.
        assert_eq!(
            style_of(&paras, " YOUR ANSWER"),
            dim().add_modifier(Modifier::BOLD)
        );
        assert_eq!(paras[2].hit, Some(Hit::Choice(1)));
        assert_eq!(paras[4].hit, Some(Hit::Choice(3)));
        assert_eq!(paras[5].hit, Some(Hit::Field(Which::Note)));
        let paras = panel("decision", decision, |f| f.other = true, true);
        assert_eq!(
            style_of(&paras, "YOUR ANSWER"),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        );
        // With the keys, the heading has an edge and says so.
        assert_eq!(lines(&paras)[1], "▍YOUR ANSWER · keys here");
        assert_eq!(lines(&paras)[4], "▸ ● 3  Other, see note");
        assert_eq!(style_of(&paras, "● "), green());
    }

    #[test]
    fn a_signoff_or_a_briefing_panel_has_its_two_answers() {
        let paras = panel("signoff", |_| {}, |f| f.signoff = Some("changes"), false);
        assert_eq!(
            lines(&paras)[2..],
            [
                "  ○ accept",
                "  ● request changes (note required)",
                "  comment  empty"
            ]
        );
        assert_eq!(paras[3].hit, Some(Hit::Mark("changes")));
        let paras = panel(
            "briefing",
            |open| {
                "The work.".clone_into(&mut open.body);
                open.left_out = vec!["docs".into(), "bench".into()];
            },
            |f| {
                f.briefing = Some("send_back");
                f.file[1] = true;
            },
            false,
        );
        assert_eq!(
            lines(&paras)[2..],
            [
                "  ○ accept",
                "  ● send back (reason required)",
                "  file what was left out",
                "  [ ] 1  docs",
                "  [x] 2  bench",
                "  reason   empty"
            ]
        );
        assert_eq!(paras[6].hit, Some(Hit::LeftOut(1)));
    }

    #[test]
    fn an_fyi_panel_is_its_note_and_acknowledge() {
        let case = open("fyi", |_| {});
        let form = Form::new(key(), &case.0, Act::Answer);
        let paras = panel_paras(&form, &case.0, false, None, usize::MAX);
        assert_eq!(lines(&paras), ["", " YOUR ANSWER", "  note     empty"]);
        let chips = panel_chips(&app(0), form.kind, &form.presses(), None);
        let labels: Vec<String> = chips.iter().map(|chip| text(&chip.spans)).collect();
        assert_eq!(labels, [" ⏎ Acknowledge ", " d Drop "]);
        // With the keys on a button, it is pointed at and underlined.
        let mut form = form;
        form.go(super::Spot::Button(super::Press::Drop));
        let chips = panel_chips(&app(0), form.kind, &form.presses(), Some(&form));
        let labels: Vec<String> = chips.iter().map(|chip| text(&chip.spans)).collect();
        assert_eq!(labels, [" ⏎ Acknowledge ", "▸ d Drop "]);
        assert!(
            chips[1].spans[2]
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn an_approval_panel_has_each_rows_verdicts_where_a_click_picks_them() {
        let paras = panel(
            "approval",
            |open| open.rows = vec![row("a", "human", "x"), row("b", "", "y")],
            |f| {
                f.rows[0].verdict = Some("approve");
                f.rows[1].verdict = Some("reject");
                f.rows[1].note.set("not now".into());
                f.row = 1;
            },
            true,
        );
        let all = lines(&paras);
        assert_eq!(
            all[2..],
            [
                "  a Step a",
                "     ● done  ○ hold  ○ won't do",
                "▸ b Step b",
                "     ○ approve  ○ hold  ● reject",
                "  note   not now",
                "  note     empty"
            ]
        );
        assert_eq!(paras[3].hit, Some(Hit::Verdicts(0, true)));
        // Each verdict's cells are where it is drawn.
        for (line, human) in [(&all[3], true), (&all[5], false)] {
            for (x, wide, verdict) in verdict_cells(human) {
                let drawn: String = line
                    .chars()
                    .skip(usize::from(x))
                    .take(usize::from(wide))
                    .collect();
                assert_eq!(
                    &drawn[drawn.len() - verdict_word(verdict, human).len()..],
                    verdict_word(verdict, human)
                );
            }
        }
    }

    #[test]
    fn a_question_or_stuck_panel_takes_text_first() {
        let paras = panel("question", |_| {}, |f| f.text.set("yes\nno".into()), true);
        assert_eq!(
            lines(&paras)[2..],
            ["▸ reply    yes", "           no ", "  note     empty"]
        );
        let editing = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
        assert_eq!(style_of(&paras, "reply    "), editing);
        assert_eq!(style_of(&paras, "note     "), dim());
        // Out of the panel, no field has the keys.
        let paras = panel("stuck", |_| {}, |_| {}, false);
        assert_eq!(lines(&paras)[2..], ["  guidance empty", "  note     empty"]);
        assert_eq!(style_of(&paras, "guidance "), dim());
    }

    #[test]
    fn a_park_and_a_withdraw_have_their_own_heading_and_note() {
        let paras = form_paras(&form("stuck", Act::Park, |_| {}));
        assert_eq!(
            lines(&paras),
            [format!("PARK {}", key()), "why       ".into()]
        );
        let mut withdraw = form("message", Act::Withdraw, |_| {});
        withdraw.editing = None;
        let paras = form_paras(&withdraw);
        assert_eq!(
            lines(&paras),
            [format!("WITHDRAW {}", key()), "reason   empty".into()]
        );
    }

    /// A button is its label on a face, its key before it or a letter of
    /// it underlined, lit under the pointer; without a hit, no face.
    #[test]
    fn a_button_draws_its_key_its_label_and_its_face() {
        use super::{BUTTON_BG, Button, HOVER_BG};
        let hit = Some(Hit::Key(KeyEvent::from(KeyCode::Char('p'))));
        let park = Button::new("Park", hit).accel('p');
        let spans = park.spans(false);
        assert_eq!(text(&spans), " Park ");
        assert_eq!(park.width(), 6);
        let underlined: String = spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(underlined, "P");
        assert!(spans.iter().all(|s| s.style.bg == Some(BUTTON_BG)));
        assert!(
            park.spans(true)
                .iter()
                .all(|s| s.style.bg == Some(HOVER_BG))
        );
        let send = Button::new("send", hit).key("⏎").look(Look::Danger);
        assert_eq!(text(&send.spans(false)), " ⏎ send ");
        assert!(
            send.spans(false)
                .iter()
                .all(|s| s.style.bg == Some(Color::Red))
        );
        let moves = Button::new("move", None).key("j/k");
        assert!(moves.spans(true).iter().all(|s| s.style.bg.is_none()));
    }

    #[test]
    fn a_form_asks_before_it_drops_withdraws_or_discards() {
        let mut app = App::new("s".into());
        app.form = Some(form("fyi", Act::Answer, |_| {}));
        let ask = |app: &mut App, confirm| {
            if let Some(form) = &mut app.form {
                form.confirm = Some(confirm);
            }
            let dialog = question(app).unwrap();
            let buttons: Vec<_> = dialog
                .buttons
                .iter()
                .map(|b| (b.key.unwrap(), b.label, b.look))
                .collect();
            (dialog.title, dialog.body, buttons)
        };
        // The safe answer first, on esc; the other on y.
        let keep = |label| ("esc", label, Look::Primary);
        let yes = |label| ("y", label, Look::Danger);
        assert_eq!(
            ask(&mut app, Confirm::Drop),
            (
                format!("Drop {}?", key()),
                Some("The agent stops the work.".into()),
                vec![keep("Keep it"), yes("Drop")]
            )
        );
        assert_eq!(
            ask(&mut app, Confirm::Withdraw),
            (
                format!("Withdraw {} from its agent?", key()),
                None,
                vec![keep("Keep it"), yes("Withdraw")]
            )
        );
        assert_eq!(
            ask(&mut app, Confirm::Discard),
            (
                "Throw away what you typed?".into(),
                None,
                vec![keep("Keep typing"), yes("Discard")]
            )
        );
        // No question, no dialog; and the form draws none of its own.
        app.form.as_mut().unwrap().confirm = None;
        assert!(question(&app).is_none());
        app.form.as_mut().unwrap().confirm = Some(Confirm::Drop);
        assert!(
            !lines(&form_paras(app.form.as_ref().unwrap()))
                .iter()
                .any(|l| l.contains("Drop"))
        );
    }

    #[test]
    fn the_option_being_picked_in_the_form_on_this_case_is_marked() {
        let case = open("decision", decision);
        let mut app = app(0);
        let mut form = Form::new(key(), &case.0, Act::Answer);
        form.choice = Some(2);
        app.form = Some(form);
        let paras = case_paras(&app, key(), &case.0);
        let all = lines(&paras);
        let at = all
            .iter()
            .position(|line| [" YOUR ANSWER", "▍YOUR ANSWER · keys here"].contains(&line.as_str()))
            .unwrap();
        // The keys are on the choices: the one chosen is pointed at.
        assert_eq!(
            all[at + 1..at + 5],
            [
                "  ○ 1  Ship",
                "▸ ● 2  Wait",
                "  ○ 3  Other, see note",
                "  note     empty"
            ]
        );
        assert_eq!(style_of(&paras, "● "), green());
        // A form on another case marks nothing here; nor is the panel's
        // part pointed at with the keys out of it.
        let other = Key::new(8).unwrap();
        let all = lines(&case_paras(&app, other, &case.0));
        let at = all
            .iter()
            .position(|line| [" YOUR ANSWER", "▍YOUR ANSWER · keys here"].contains(&line.as_str()))
            .unwrap();
        assert!(
            all[at..]
                .iter()
                .all(|line| !line.contains('●') && !line.contains('▸'))
        );
    }

    fn row(id: &str, by: &str, script: &str) -> ApprovalRow {
        let mut row = ApprovalRow::default();
        id.clone_into(&mut row.id);
        row.label = format!("Step {id}");
        by.clone_into(&mut row.by);
        script.clone_into(&mut row.script);
        row.link = format!("https://x.test/{id}");
        row
    }

    fn verdict(id: &str, verdict: &str, note: &str) -> RowVerdict {
        let mut row = RowVerdict::default();
        id.clone_into(&mut row.id);
        verdict.clone_into(&mut row.verdict);
        note.clone_into(&mut row.note);
        row
    }

    #[test]
    fn an_approval_shows_its_rows_and_each_verdict() {
        let mut case = open("approval", |open| {
            let mut a = row("a", "", "make\nmake install\n");
            "careful".clone_into(&mut a.note);
            open.rows = vec![a, row("b", "human", "Press go"), row("c", "human", "Wait")];
        });
        let mut answer = Answer::default();
        answer.rows = vec![
            verdict("a", "reject", "not yet"),
            verdict("b", "approve", ""),
            verdict("c", "hold", ""),
        ];
        add(&mut case, Author::Human, &Record::Answer(answer), 60);
        let paras = case_paras(&app(60), key(), &case.0);
        let lines = lines(&paras);
        let at = lines.iter().position(|l| l == "ROWS").unwrap();
        assert_eq!(
            lines[at..at + 17],
            [
                "ROWS",
                " a  Step a",
                "    careful",
                "    https://x.test/a",
                "    $ make",
                "    $ make install",
                " b  Step b  you",
                "    https://x.test/b",
                "      Press go",
                " c  Step c  you",
                "    https://x.test/c",
                "      Wait",
                "",
                "ANSWER",
                "▍ a reject  not yet",
                "▍ b done",
                "▍ c hold",
            ]
        );
        assert_eq!(style_of(&paras, "careful"), dim());
        assert_eq!(
            style_of(&paras, "  you"),
            dim().add_modifier(Modifier::BOLD)
        );
        assert_eq!(text(&paras[at + 5].rest), "      ");
        assert_eq!(
            style_of(&paras, "make install"),
            Style::new().fg(Color::Yellow)
        );
        assert_eq!(
            style_of(&paras, "a reject"),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
        );
        assert_eq!(style_of(&paras, "b done"), green());
        assert_eq!(
            style_of(&paras, "c hold"),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn a_follow_up_has_only_the_notes_since_the_answer_and_none_after_an_amend() {
        let mut case = open("decision", decision);
        let mut before = Note::default();
        before.body = "Before the answer.".into();
        add(&mut case, Author::Agent, &Record::Note(before), 10);
        let mut answer = Answer::default();
        answer.choice = 1;
        add(&mut case, Author::Human, &Record::Answer(answer), 20);
        let mut after = Note::default();
        after.body = "After it.".into();
        after.reopen = true;
        add(&mut case, Author::Agent, &Record::Note(after), 30);
        let paras = case_paras(&app(60), key(), &case.0);
        assert_eq!(
            lines(&paras)[2..7],
            [
                "",
                "FOLLOW-UP, NEW SINCE YOUR ANSWER",
                "After it.",
                "",
                "YOUR LAST ANSWER"
            ]
        );
        let mut amend = Amend::default();
        amend.body = "A new question.".into();
        add(&mut case, Author::Agent, &Record::Amend(amend), 40);
        let paras = case_paras(&app(60), key(), &case.0);
        assert_eq!(lines(&paras)[2..4], ["", "A new question."]);
    }

    fn actor(id: &str) -> Actor {
        let mut actor = Actor::default();
        id.clone_into(&mut actor.id);
        actor
    }

    #[test]
    fn the_answer_and_thread_name_the_actor() {
        let named = "hV3kQ9mZr2pX7wLc0tBnYA";
        let unnamed = "aaaaaaaaaaaaaaaaaaaaaa";
        let mut case = open("fyi", |_| {});
        let mut answer = Answer::default();
        answer.ack = true;
        let row = case
            .0
            .append(
                &Append::new(Author::Human, &Record::Answer(answer)),
                at(60),
                Some(&actor(named)),
                None,
            )
            .unwrap();
        case.1.push(row);
        let mut pickup = Pickup::default();
        pickup.by = "worker".into();
        let row = case
            .0
            .append(
                &Append::new(Author::Agent, &Record::Pickup(pickup)),
                at(120),
                Some(&actor(unnamed)),
                None,
            )
            .unwrap();
        case.1.push(row);
        let mut app = app(180);
        app.actors = Actors::new(vec![
            ActorRow::new(
                named.into(),
                "human".into(),
                Some("Ryan".into()),
                String::new(),
            ),
            ActorRow::new(unnamed.into(), "agent".into(), None, String::new()),
        ]);
        let paras = case_paras(&app, key(), &case.0);
        let all = lines(&paras);
        assert!(all.contains(&"  Ryan, 2m ago".to_string()), "{all:?}");
        let at = all.iter().position(|l| l == "THREAD").unwrap();
        assert_eq!(
            all[at + 1..],
            [
                "  3m  agent open",
                "  2m  human answer  by Ryan",
                "            acknowledged",
                "  1m  agent pickup  by aaaaaaaaaaaaaaaaaaaaaa",
            ]
        );
        assert_eq!(style_of(&paras, "  by Ryan"), dim());
        // An actor the table no longer has is a deleted user.
        app.actors = Actors::default();
        let all = lines(&case_paras(&app, key(), &case.0));
        assert!(
            all.contains(&"  deleted user, 2m ago".to_string()),
            "{all:?}"
        );
    }

    fn listening(worker: &str, labels: &[&str], doing: Option<&str>) -> Listening {
        Listening::new(
            worker.into(),
            labels.iter().map(|l| (*l).to_string()).collect(),
            at(0),
            at(0),
            doing.map(String::from),
            None,
            Vec::new(),
        )
    }

    #[test]
    fn inbox_zero_lists_the_listeners_and_what_is_out() {
        let mut app = app(60);
        let mut paras = Vec::new();
        zero_paras(&app, &mut paras);
        assert_eq!(lines(&paras), ["nothing needs you."]);
        assert_eq!(style_of(&paras, "nothing needs you."), green());

        app.listeners = vec![
            listening("lead", &["ops", "web"], Some("reviewing")),
            listening("w2", &[], None),
        ];
        let sent = open("message", |_| {}).0;
        let mut answered = open("fyi", |open| "Answered one".clone_into(&mut open.title));
        let mut answer = Answer::default();
        answer.ack = true;
        add(&mut answered, Author::Human, &Record::Answer(answer), 10);
        app.cases = vec![(key(), sent), (Key::new(8).unwrap(), answered.0)];
        let mut paras = Vec::new();
        zero_paras(&app, &mut paras);
        assert_eq!(
            lines(&paras),
            [
                "nothing needs you.".to_string(),
                String::new(),
                "LISTENING NOW".into(),
                "  lead #ops #web  reviewing".into(),
                "  w2".into(),
                String::new(),
                "MESSAGES NOT PICKED UP".into(),
                format!("sent       The title  {}", key()),
                String::new(),
                "STILL WITH AN AGENT".into(),
                format!("answered   Answered one  {}", Key::new(8).unwrap()),
            ]
        );
        assert_eq!(style_of(&paras, " #ops"), Style::new().fg(Color::Magenta));
        assert_eq!(style_of(&paras, "  reviewing"), dim());
        assert_eq!(
            style_of(&paras, "sent       "),
            Style::new().fg(Color::Blue)
        );
    }

    #[test]
    fn a_new_message_offers_the_listeners_and_a_title_from_the_body() {
        let editing = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
        let mut compose = Compose::new();
        let paras = compose_paras(&compose, &[]);
        assert_eq!(
            lines(&paras),
            [
                "MESSAGE  today",
                "to        ",
                "         no session is listening now: a worker name, or #label",
                "title    from the body's first line",
                "body     empty",
            ]
        );
        assert_eq!(style_of(&paras, "to       "), editing);
        assert_eq!(style_of(&paras, "  today"), Style::new().fg(Color::Yellow));

        compose.part = Some(Part::Body);
        compose.body.set("Deploy it\nnow".into());
        compose.urgency = Urgency::Blocking;
        let mut listeners = vec![listening("lead", &["ops"], Some("busy"))];
        listeners.extend((2..=10).map(|n| listening(&format!("w{n}"), &[], None)));
        let paras = compose_paras(&compose, &listeners);
        let all = lines(&paras);
        assert_eq!(all[0], "MESSAGE  blocking");
        assert_eq!(all[1], "to       empty");
        assert_eq!(all[2], "         1 lead #ops  busy");
        assert_eq!(all[10], "         9 w9");
        assert_eq!(
            all[11..],
            [
                "title    Deploy it  (from the body)",
                "body     Deploy it",
                "         now "
            ]
        );
        assert_eq!(style_of(&paras, "to       "), dim());
        assert_eq!(style_of(&paras, "body     "), editing);
        assert_eq!(style_of(&paras, "1 "), editing);
        assert_eq!(style_of(&paras, "  blocking"), Style::new().fg(Color::Red));

        // The title being typed is a field.
        compose.part = Some(Part::Title);
        let paras = compose_paras(&compose, &[]);
        assert_eq!(lines(&paras)[3], "title     ");
        assert_eq!(style_of(&paras, "title    "), editing);
        assert_eq!(style_of(&paras, "body     "), dim());
        // A title typed is shown as typed.
        compose.part = None;
        compose.title.set("Mine".into());
        let paras = compose_paras(&compose, &[]);
        assert_eq!(lines(&paras)[3], "title    Mine");
        assert_eq!(style_of(&paras, "title    "), dim());
    }

    #[test]
    fn an_edit_of_a_message_shows_its_fields_and_no_listeners() {
        let case = open("message", |open| "Do it".clone_into(&mut open.body));
        let mut compose = Compose::edit(key(), &case.0);
        compose.confirm = true;
        let paras = compose_paras(&compose, &[listening("lead", &[], None)]);
        assert_eq!(
            lines(&paras),
            [
                format!("EDIT MESSAGE {}  today", key()),
                "to       worker".into(),
                "title    The title".into(),
                "body     Do it ".into(),
            ]
        );
        // An edit with no title shows the empty field, not the body's line.
        compose.title.set(String::new());
        compose.part = None;
        let paras = compose_paras(&compose, &[]);
        assert_eq!(lines(&paras)[2], "title    empty");
    }

    #[test]
    fn the_cursor_is_drawn_on_the_line_it_is_in() {
        let mut field = Field::default();
        field.set("ab\ncd".into());
        assert!(field.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        let paras = field_paras("note", &field, true);
        let spans: Vec<Vec<String>> = paras
            .iter()
            .map(|p| p.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert_eq!(spans, [vec!["ab"], vec!["c", "d", ""]]);
        assert_eq!(
            paras[1].spans[1].style,
            Style::new().add_modifier(Modifier::REVERSED)
        );
        assert_eq!(text(&paras[1].first), "         ");
        // Not editing, there is no cursor.
        let paras = field_paras("note", &field, false);
        assert_eq!(lines(&paras), ["note     ab", "         cd"]);
    }

    fn pairs(keys: &[(&str, &str)]) -> String {
        keys.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    #[test]
    fn the_footer_keys_follow_the_form() {
        let mut decision = form("decision", Act::Answer, self::decision);
        let tail = "⏎ send, n note";
        // Other is the number after the last option.
        assert_eq!(
            pairs(&keys_for(&decision, false)),
            format!("1-3 choose, o other, {tail}, j/k move, esc leave")
        );
        // In a field, enter sends; the new line is shift+enter where the
        // terminal tells it from enter, and alt+enter where it does not.
        decision.editing = Some(Which::Note);
        assert_eq!(
            pairs(&keys_for(&decision, false)),
            "⏎ send, alt+⏎ new line, ^e editor, esc leave the field"
        );
        assert_eq!(
            pairs(&keys_for(&decision, true)),
            "⏎ send, ⇧⏎ new line, ^e editor, esc leave the field"
        );
        // On a button, enter presses it.
        decision.editing = None;
        decision.tab(1);
        decision.tab(1);
        assert_eq!(
            pairs(&keys_for(&decision, false)),
            "1-3 choose, o other, ⏎ press, n note, esc leave"
        );
        assert_eq!(
            pairs(&keys_for(&form("signoff", Act::Answer, |_| {}), false)),
            format!("1 accept, 2 changes, {tail}, j/k choose, esc leave")
        );
        let mut approval = form("approval", Act::Answer, |open| {
            open.rows = vec![row("a", "", "x"), row("b", "human", "y")];
        });
        assert_eq!(
            pairs(&keys_for(&approval, false)),
            format!("1-2 row, y approve, h hold, x reject, r row note, {tail}, j/k row, esc leave")
        );
        approval.row = 1;
        assert_eq!(
            pairs(&keys_for(&approval, false)),
            format!("1-2 row, y done, h hold, x won't do, r row note, {tail}, j/k row, esc leave")
        );
        for kind in ["question", "stuck"] {
            let mut text = form(kind, Act::Answer, |_| {});
            text.editing = None;
            assert_eq!(
                pairs(&keys_for(&text, false)),
                format!("i write, {tail}, esc leave")
            );
        }
        let park = form("stuck", Act::Park, |_| {});
        let mut park = park;
        park.editing = None;
        assert_eq!(pairs(&keys_for(&park, false)), "n edit, ⏎ send, esc cancel");
        assert_eq!(
            pairs(&keys_for(&form("fyi", Act::Answer, |_| {}), false)),
            format!("{tail}, esc leave")
        );
        // A briefing answers by number on its answers, and on its items
        // offers the number keys only for the items it has.
        for (items, keys) in [
            (0, None),
            (9, Some("j/k move, space file, 1-9 file")),
            (10, Some("j/k move, space file, 1-9 0 file")),
        ] {
            let mut briefing = form("briefing", Act::Answer, |open| {
                "The work.".clone_into(&mut open.body);
                open.left_out = (0..items).map(|n| format!("item {n}")).collect();
            });
            assert_eq!(
                pairs(&keys_for(&briefing, false)),
                format!("1 accept, 2 send back, {tail}, j/k choose, esc leave")
            );
            briefing.tab(1);
            if let Some(keys) = keys {
                assert_eq!(
                    pairs(&keys_for(&briefing, false)),
                    format!("{keys}, {tail}, esc leave")
                );
            }
        }
    }

    #[test]
    fn the_footer_keys_follow_the_message() {
        let mut compose = Compose::new();
        assert_eq!(
            pairs(&compose_keys(&compose)),
            "⏎ tab next field, esc leave the field, ^s send"
        );
        compose.part = Some(Part::Body);
        assert_eq!(
            pairs(&compose_keys(&compose)),
            "esc leave the field, ^s send, ^e editor, ⏎ new line"
        );
        compose.part = None;
        assert_eq!(
            pairs(&compose_keys(&compose)),
            "1-9 to a listener, t to, i title, b body, u urgency, ⏎ send, esc cancel"
        );
        compose.editing = Some((key(), 1));
        assert_eq!(pairs(&compose_keys(&compose)), "b body, ⏎ send, esc cancel");
    }

    #[test]
    fn truncate_cuts_with_an_ellipsis_only_when_too_long() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("", 0), "");
        // At no width there is no room for the ellipsis either.
        assert_eq!(truncate("ab", 0), "");
        assert_eq!(truncate("ab", 1), "…");
    }

    /// The columns `text` takes as the buffer draws it.
    fn drawn(text: &str) -> usize {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 1));
        usize::from(buffer.set_stringn(0, 0, text, 80, Style::new()).0)
    }

    #[test]
    fn truncate_counts_the_columns_a_wide_glyph_takes() {
        let columns = drawn;
        let cases = [
            ("用户项目", 8, "用户项目"),
            ("用户项目", 5, "用户…"),
            // A wide glyph that would straddle the edge is dropped.
            ("用户项目", 4, "用…"),
            ("a用户", 4, "a用…"),
            ("🙂🙂🙂", 6, "🙂🙂🙂"),
            ("🙂🙂🙂", 4, "🙂…"),
            ("🙂🙂🙂", 2, "…"),
            ("用户", 1, "…"),
            // A flag is one glyph, and is not split.
            ("🇬🇧🇬🇧", 4, "🇬🇧🇬🇧"),
            ("🇬🇧🇬🇧", 3, "🇬🇧…"),
            ("🇬🇧🇬🇧", 2, "…"),
            // A line break takes no column.
            ("ab\ncd", 4, "ab\ncd"),
        ];
        for (text, width, cut) in cases {
            assert_eq!(truncate(text, width), cut, "{text} at {width}");
            assert!(
                columns(&truncate(text, width)) <= width,
                "{text} at {width}"
            );
        }
        // Joined glyphs the buffer draws apart are measured as it draws them.
        for text in [
            "deploy 1\u{fe0f}\u{20e3}\u{200d}🔥 done",
            "🏽\u{200d}🙂",
            "🇬🇧\u{200d}🙂",
            "e\u{301}t\u{e9} ❤\u{fe0f} 👨\u{200d}👩\u{200d}👧",
        ] {
            for width in 0..=drawn(text) {
                let cut = truncate(text, width);
                assert!(drawn(&cut) <= width, "{text:?} at {width}: {cut:?}");
                assert!(width < drawn(text) || cut == text, "{text:?} at {width}");
            }
        }
    }

    #[test]
    fn inset_keeps_inside_the_area() {
        assert_eq!(inset(Rect::new(3, 4, 20, 10), 2, 1), Rect::new(5, 5, 16, 9));
        assert_eq!(inset(Rect::new(3, 4, 1, 0), 2, 1), Rect::new(4, 4, 0, 0));
        assert_eq!(inset(Rect::new(0, 0, 20, 10), 3, 2), Rect::new(3, 2, 14, 8));
    }

    #[test]
    fn each_state_and_urgency_has_its_colour() {
        assert_eq!(state_tag(State::Open), None);
        let tags = [
            (State::Parked, "parked", Color::Magenta),
            (State::Answered, "answered", Color::Green),
            (State::PickedUp, "picked up", Color::Blue),
            (State::Sent, "sent", Color::Blue),
            (State::Closed, "closed", DIM),
            (State::Withdrawn, "withdrawn", DIM),
        ];
        for (state, word, colour) in tags {
            assert_eq!(state_tag(state), Some((word, Style::new().fg(colour))));
        }
        assert_eq!(
            urgency_style(Urgency::Blocking),
            Style::new().fg(Color::Red)
        );
        assert_eq!(
            urgency_style(Urgency::Today),
            Style::new().fg(Color::Yellow)
        );
        assert_eq!(urgency_style(Urgency::Whenever), Style::new().fg(DIM));
    }

    fn screen(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| super::draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// The column where `needle` starts in row `y`, counted in cells, so
    /// that a wide glyph before it counts as the two it takes.
    fn col(buffer: &Buffer, y: u16, needle: &str) -> u16 {
        let at = rows(buffer)[usize::from(y)].find(needle).unwrap();
        let mut seen = 0;
        (0..buffer.area.width)
            .find(|&x| {
                let here = seen;
                seen += buffer[(x, y)].symbol().len();
                here == at
            })
            .unwrap()
    }

    /// An inbox of five cases, one in each state the list draws its own way.
    fn many() -> App {
        let mut app = app(2 * HOUR);
        let blocking = open("decision", |open| {
            decision(open);
            "blocking".clone_into(&mut open.urgency);
            "Ship it".clone_into(&mut open.title);
        });
        let mut parked = open("stuck", |open| "Stuck here".clone_into(&mut open.title));
        let mut park = cases_core::Park::default();
        park.note = "later".into();
        add(&mut parked, Author::Human, &Record::Park(park), 60);
        let mut answered = open("fyi", |open| {
            "For you".clone_into(&mut open.title);
            open.worker.clear();
            open.labels = vec!["ops".into()];
        });
        let mut answer = Answer::default();
        answer.ack = true;
        add(&mut answered, Author::Human, &Record::Answer(answer), 120);
        let mut closed = open("fyi", |open| {
            "Done one with a long title that is cut to the width of the list"
                .clone_into(&mut open.title);
        });
        let mut answer = Answer::default();
        answer.ack = true;
        add(&mut closed, Author::Human, &Record::Answer(answer), 120);
        let mut pickup = Pickup::default();
        pickup.by = "worker".into();
        add(&mut closed, Author::Agent, &Record::Pickup(pickup), 130);
        let mut close = Close::default();
        close.outcome = "done".into();
        add(&mut closed, Author::Agent, &Record::Close(close), 140);
        let mut withdrawn = open("message", |open| "Never mind".clone_into(&mut open.title));
        add(
            &mut withdrawn,
            Author::Human,
            &Record::Withdraw(cases_core::Withdraw::default()),
            150,
        );
        let cases = vec![blocking, parked, answered, closed, withdrawn]
            .into_iter()
            .zip(1..)
            .map(|(case, n)| (Key::new(n).unwrap(), case.0))
            .collect();
        app.update(cases, 0, Actors::default(), at(2 * HOUR));
        app.filter = Filter::All;
        app
    }

    #[test]
    fn a_narrow_screen_draws_each_state_of_case_in_the_list() {
        let mut app = many();
        let keys: Vec<String> = (1..=5).map(|n| Key::new(n).unwrap().to_string()).collect();
        let buffer = screen(&mut app, 50, 24);
        assert_eq!(
            rows(&buffer),
            [
                " cases   cases.db        ● 1 waiting · 1 blocking ".to_string(),
                // The tabs wrap rather than lose Briefings off the end,
                // beside the message button.
                " Inbox 2   In flight 1   Done 2         + Message ".into(),
                " All 5  │ Briefings 0                             ".into(),
                "─".repeat(50),
                // The list has the keys, and says so.
                format!("▍List · keys here{}", " ".repeat(33)),
                "▌● Ship it                                        ".into(),
                format!("   {} · decision · worker                  2h  ", keys[0]),
                " ".repeat(50),
                " ◌ Stuck here                                     ".into(),
                format!("   {} · stuck · parked · worker            1h  ", keys[1]),
                " ".repeat(50),
                " ○ For you                                        ".into(),
                format!("   {} · fyi · answered                     1h  ", keys[2]),
                " ".repeat(50),
                " ✕ Never mind                                     ".into(),
                format!("   {} · message · worker                   1h  ", keys[4]),
                " ".repeat(50),
                " ✓ Done one with a long title that is cut to t…   ".into(),
                format!("   {} · fyi · worker                       1h  ", keys[3]),
                " ".repeat(50),
                " ".repeat(50),
                // And so do the keys: where they are and where tab takes
                // them, the selected case's answer's first, then the rest,
                // but not m, with its button.
                "on: list · tab → options │ 1-3 choose   n note    ".into(),
                " ⏎ answer   j/k move   → read   [ ] filter        ".into(),
                " s stats   ^k commands   ? keys   q quit          ".into(),
            ]
        );
        // The blocking count is red, and so is the dot before it.
        assert_eq!(buffer[(col(&buffer, 0, "●"), 0)].fg, Color::Red);
        assert_eq!(buffer[(col(&buffer, 0, "1 blocking"), 0)].fg, Color::Red);
        // The chosen filter is a filled pill; the others are not filled.
        assert_eq!(buffer[(col(&buffer, 2, "All"), 2)].bg, ACCENT);
        assert_eq!(buffer[(col(&buffer, 1, "Inbox"), 1)].bg, Color::Reset);
        // A done case's title recedes; the others are bold.
        assert_eq!(buffer[(col(&buffer, 17, "Done"), 17)].fg, DIM);
        let ship = col(&buffer, 5, "Ship");
        assert_eq!(buffer[(ship, 5)].modifier, Modifier::BOLD);
        // A parked case's glyph keeps its urgency; one with an agent is blue.
        assert_eq!(buffer[(col(&buffer, 8, "◌"), 8)].fg, Color::Yellow);
        assert_eq!(buffer[(col(&buffer, 11, "○"), 11)].fg, Color::Blue);
    }

    #[test]
    fn a_wide_screen_draws_the_list_beside_the_case() {
        let mut app = many();
        let key = Key::new(1).unwrap();
        let buffer = screen(&mut app, 100, 13);
        let rows = rows(&buffer);
        assert_eq!(
            rows[3..],
            [
                // The list has the keys: its title says so, and the case's
                // is dim.
                format!(
                    "▍List · keys here{}│  Case{}",
                    " ".repeat(21),
                    " ".repeat(55)
                ),
                // The bar, its answer the footer's no more; on a pane this
                // short, the title scrolls under it.
                format!(
                    "▌● Ship it                            │   Answer {}",
                    " ".repeat(51)
                ),
                format!("   {key} · decision · worker      2h  │{}", " ".repeat(61)),
                format!("{}│  Ship it{}", " ".repeat(38), " ".repeat(52)),
                format!(
                    " ◌ Stuck here                         │  ● blocking · decision · worker · 2h ago · {key}            "
                ),
                format!(
                    "   {} · stuck · parked · work… 1h  │{}more ↓  ",
                    Key::new(2).unwrap(),
                    " ".repeat(53)
                ),
                format!("{}│{}", " ".repeat(38), " ".repeat(61)),
                // The answer's buttons, kept in sight.
                format!("{}│   ⏎ Send   d Drop{}", " ".repeat(38), " ".repeat(43)),
                "on: list · tab → options │ 1-3 choose   n note   ⏎ answer   j/k move   [ ] filter   ^k commands     "
                    .to_string(),
                format!(" ? keys   q quit{}", " ".repeat(84)),
            ]
        );
        // Scrolled past the end, the scroll stops at the last line, and
        // there is no more to show.
        app.scroll = u16::MAX;
        let scrolled = super::tests::rows(&screen(&mut app, 100, 12));
        assert!(app.scroll > 0 && app.scroll < u16::MAX);
        assert!(!scrolled.concat().contains("more"));
    }

    #[test]
    fn the_header_cuts_the_store_and_counts_what_waits() {
        let mut app = app(0);
        app.store = "/home/someone/projects/cases.db".into();
        let today = open("fyi", |_| {}).0;
        app.update(vec![(key(), today)], 0, Actors::default(), at(60));
        let buffer = screen(&mut app, 50, 6);
        assert_eq!(
            rows(&buffer)[0],
            " cases   /home/someone/projects/case… ● 1 waiting "
        );
        assert_eq!(buffer[(col(&buffer, 0, "●"), 0)].fg, Color::Yellow);
        // Nothing waiting says so.
        let mut app = self::app(0);
        assert_eq!(
            rows(&screen(&mut app, 50, 6))[0],
            " cases   cases.db               nothing needs you "
        );
    }

    #[test]
    fn a_briefings_mark_is_left_out_of_a_list_row_with_no_room_for_it() {
        let mut app = app(0);
        let mut briefing = open("briefing", |open| "The work.".clone_into(&mut open.body));
        let mut answer = Answer::default();
        answer.briefing = "accept".into();
        add(&mut briefing, Author::Human, &Record::Answer(answer), 60);
        app.update(
            vec![(key(), briefing.0)],
            0,
            Actors::default(),
            at(2 * HOUR),
        );
        app.filter = Filter::Briefings;
        app.answered = super::super::app::Group::Shown;
        let rows = rows(&screen(&mut app, 100, 10));
        // "waiting to land" would push the age off the row.
        assert_eq!(
            rows[7].split('│').next().unwrap(),
            format!("   {} · briefing · answered    1h  ", key())
        );
    }

    #[test]
    fn a_worker_is_left_out_of_a_list_row_with_no_room_for_it() {
        let mut app = app(0);
        let mut stuck = open("stuck", |_| {});
        let mut answer = Answer::default();
        answer.text = "go on".into();
        add(&mut stuck, Author::Human, &Record::Answer(answer), 60);
        app.update(vec![(key(), stuck.0)], 0, Actors::default(), at(2 * HOUR));
        app.filter = Filter::All;
        let rows = rows(&screen(&mut app, 100, 8));
        // Three columns are left, and a worker needs four.
        assert_eq!(
            rows[5].split('│').next().unwrap(),
            format!("   {} · stuck · answered       1h  ", key())
        );
    }

    #[test]
    fn an_empty_inbox_says_what_is_out_in_the_list_only_when_narrow() {
        let count = |rows: &[String]| rows.concat().matches("nothing needs you.").count();
        let mut app = app(0);
        // At 50 columns the tabs and the keys take two lines each.
        let drawn = rows(&screen(&mut app, 50, 10));
        assert_eq!(
            drawn[5..8],
            [
                format!("  nothing is waiting on you.{}", " ".repeat(22)),
                " ".repeat(50),
                format!("  nothing needs you.{}", " ".repeat(30)),
            ]
        );
        // Side by side, the case pane says it, and the list does not.
        let wide = self::rows(&screen(&mut app, 100, 8));
        assert_eq!(count(&wide), 1);
        // Another filter has no inbox zero.
        app.filter = Filter::Done;
        let narrow = self::rows(&screen(&mut app, 50, 10));
        assert_eq!(narrow[5], format!("  no cases here.{}", " ".repeat(34)));
        assert_eq!(count(&narrow), 0);
    }

    #[test]
    fn an_answer_scrolls_with_the_case_and_its_buttons_stay_in_sight() {
        let case = open("approval", |open| {
            open.rows = (0..12).map(|n| row(&format!("r{n}"), "", "x")).collect();
        });
        let mut app = app(0);
        app.update(vec![(key(), case.0.clone())], 0, Actors::default(), at(60));
        app.focus = Focus::Case;
        app.form = Some(Form::new(key(), &case.0, Act::Answer));
        for scroll in [0, u16::MAX] {
            app.scroll = scroll;
            let rows = rows(&screen(&mut app, 100, 20));
            // The pane's last row, above the footer, has the buttons.
            assert!(rows[17].contains("⏎ Send   d Drop"), "{rows:#?}");
        }
        let all = rows(&screen(&mut app, 100, 20)).concat();
        assert!(all.contains("note     "), "the foot of the answer: {all}");
    }

    #[test]
    fn a_pane_with_no_room_draws_no_more_marker() {
        let mut app = many();
        let rows = rows(&screen(&mut app, 100, 5));
        assert_eq!(
            rows[3],
            format!(
                "▍List · keys here{}│  Case{}",
                " ".repeat(21),
                " ".repeat(55)
            )
        );
    }

    #[test]
    fn help_is_a_box_in_the_middle() {
        let mut app = app(0);
        app.help = true;
        let buffer = screen(&mut app, 80, 40);
        assert_eq!(buffer[(10, 4)].symbol(), "╭");
        assert_eq!(buffer[(69, 35)].symbol(), "╯");
        assert_eq!(buffer[(10, 4)].fg, ACCENT);
        let rows = rows(&buffer);
        // The box is drawn over the empty inbox's line, from column 10.
        assert_eq!(
            rows[4].chars().skip(10).collect::<String>(),
            format!("╭ keys ─{}╮{}", "─".repeat(51), " ".repeat(10))
        );
        // The answer's keys first.
        assert!(
            rows[6].contains("       1-9  answer: that option"),
            "{}",
            rows[6]
        );
    }

    #[test]
    fn the_rule_says_what_cannot_be_read() {
        let mut app = app(0);
        app.unfolded = 1;
        let buffer = screen(&mut app, 60, 6);
        assert_eq!(
            rows(&buffer)[2].trim_end(),
            " ⚠ 1 case cannot be read; cases list names it"
        );
        assert_eq!(buffer[(1, 2)].fg, Color::Yellow);
        app.unfolded = 3;
        assert_eq!(
            rows(&screen(&mut app, 60, 6))[2].trim_end(),
            " ⚠ 3 cases cannot be read; cases list names them"
        );
        // A store that cannot be read says why, cut to the width.
        app.problem = Some("the disk is gone".into());
        assert_eq!(
            rows(&screen(&mut app, 30, 6))[2],
            " ⚠ cannot read the store, tr… "
        );
    }

    #[test]
    fn ninety_six_columns_is_wide_enough_for_two_panes() {
        let mut app = many();
        let wide = rows(&screen(&mut app, 96, 10));
        assert_eq!(wide[3].chars().nth(36), Some('│'));
        assert!(!wide[8..].concat().contains("→ read"), "{}", wide[8]);
        let narrow = rows(&screen(&mut app, 95, 10));
        assert!(!narrow[3].contains('│'), "{}", narrow[3]);
        assert!(narrow[8..].concat().contains("→ read"), "{}", narrow[8]);
    }

    #[test]
    fn a_header_with_no_room_for_the_store_leaves_it_out() {
        let mut app = app(0);
        let buffer = screen(&mut app, 28, 6);
        assert_eq!(rows(&buffer)[0], " cases    nothing needs you ");
    }

    #[test]
    fn a_wide_store_path_is_cut_to_the_columns_left() {
        let mut app = app(0);
        app.store = "/home/用户/项目/cases.db".into();
        let buffer = screen(&mut app, 50, 6);
        // 22 columns are left for the path, which takes 24.
        assert_eq!(col(&buffer, 0, "cases…"), 25);
        assert_eq!(col(&buffer, 0, "nothing"), 32);
        assert_eq!(buffer[(31, 0)].symbol(), " ");
    }

    #[test]
    fn an_approval_with_no_rows_offers_no_row_keys() {
        let mut form = form("approval", Act::Answer, |open| {
            open.rows = vec![row("a", "", "x")];
        });
        // An approval is opened with rows; a form can still hold none.
        form.rows.clear();
        assert_eq!(pairs(&keys_for(&form, false)), "⏎ send, n note, esc leave");
    }
}
