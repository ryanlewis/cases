//! The stats strip (TU-13): the cases opened and answered on each of the
//! last 14 UTC days, as sparklines, and the median time to answer. The
//! reads' thread works the stats out with each read, by the core's
//! [`cases_core::stats`], so drawing them costs the screen nothing.

use std::time::Duration;

use cases_core::Stats;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::view::{ACCENT, DIM};
use crate::error::Error;

/// How many days the strip counts, today last.
pub(crate) const DAYS: u16 = 14;

/// How many rows the strip takes.
pub(crate) const HEIGHT: u16 = 3;

/// Below this many rows the strip is not drawn, so that the list keeps
/// its room on a small screen.
pub(crate) const MIN_ROWS: u16 = 24;

/// The config file's `tui-stats`: whether the strip shows at first. `s`
/// shows or hides it either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Shown, drawn as well as the terminal can. For now that is text.
    Auto,
    /// Shown, as text.
    Text,
    /// Hidden until `s`.
    Off,
}

impl Mode {
    /// The mode the config file gives as `text`, or `auto` when it gives
    /// none.
    ///
    /// # Errors
    ///
    /// When `text` is not `auto`, `text` or `off`.
    pub(crate) fn of(text: Option<&str>) -> Result<Mode, Error> {
        match text {
            None | Some("auto") => Ok(Mode::Auto),
            Some("text") => Ok(Mode::Text),
            Some("off") => Ok(Mode::Off),
            Some(text) => Err(Error::new(format!(
                "the config file's tui-stats: {text:?} is not auto, text or off"
            ))),
        }
    }

    /// Whether the strip shows at first.
    pub(crate) fn shown(self) -> bool {
        self != Mode::Off
    }

    /// Whether the terminal is asked if it draws images: only for `auto`,
    /// so `text` and `off` always draw text.
    pub(crate) fn asks(self) -> bool {
        self == Mode::Auto
    }
}

/// How many columns the image of the two series takes on a screen
/// `width` wide: two a day where there is room, one at a phone's width.
pub(crate) fn image_width(width: u16) -> u16 {
    if width >= 60 { 2 * DAYS } else { DAYS }
}

/// The cells of a strip drawn in `band` that the image of `width` columns
/// goes over: the two rows under the caption, past the labels.
pub(crate) fn image_area(band: Rect, width: u16) -> Rect {
    Rect::new(band.x + LABEL, band.y + 1, width, 2).intersection(band)
}

/// The columns before the series: a space and the longest label.
pub(crate) const LABEL: u16 = 10;

/// The strip's three lines: the days and the median time to answer, then
/// a sparkline each of the cases opened and the answers given, with their
/// totals. The sparklines share one scale, so that the two compare. With
/// `image`, the series are left as that many blank columns, for the image
/// to go over, and the labels take the colours of its bars.
pub(crate) fn lines(stats: &Stats, image: Option<u16>) -> Vec<Line<'static>> {
    let opened = stats.opened();
    let answered = stats.answered();
    let top = opened.iter().chain(answered).copied().max().unwrap_or(0);
    let median = stats.median().map_or_else(|| "-".to_owned(), short);
    let dim = Style::new().fg(DIM);
    let series = |label: &'static str, counts: &[u32], colour: Color| {
        let (label_style, drawn) = match image {
            Some(width) => (Style::new().fg(colour), " ".repeat(usize::from(width))),
            None => (dim, spark(counts, top)),
        };
        Line::from(vec![
            Span::styled(
                format!(" {label:<pad$}", pad = usize::from(LABEL - 1)),
                label_style,
            ),
            Span::styled(drawn, Style::new().fg(colour)),
            Span::styled(
                format!("  {}", counts.iter().map(|n| u64::from(*n)).sum::<u64>()),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ])
    };
    vec![
        Line::from(vec![
            Span::styled(format!(" {DAYS} days, UTC · median answer "), dim),
            Span::styled(median, Style::new().add_modifier(Modifier::BOLD)),
        ]),
        series("opened", opened, ACCENT),
        series("answered", answered, Color::Green),
    ]
}

/// One character a day: `·` for none, and otherwise one of eight blocks,
/// by its count against `top`, the largest count of the strip.
fn spark(counts: &[u32], top: u32) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    counts
        .iter()
        .map(|&n| {
            if n == 0 || top == 0 {
                return '·';
            }
            let level = (u64::from(n) * 8).div_ceil(u64::from(top));
            let level = usize::try_from(level).unwrap_or(8).clamp(1, 8);
            BLOCKS[level - 1]
        })
        .collect()
}

/// A time to answer, short: seconds under a minute, minutes under an hour,
/// hours and minutes under two days, then days.
fn short(time: Duration) -> String {
    let secs = time.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3_600 => format!("{}m", secs / 60),
        3_600..172_800 if (secs % 3_600) / 60 == 0 => format!("{}h", secs / 3_600),
        3_600..172_800 => format!("{}h{}m", secs / 3_600, (secs % 3_600) / 60),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_auto_text_or_off_and_auto_by_default() {
        assert_eq!(Mode::of(None).unwrap(), Mode::Auto);
        assert_eq!(Mode::of(Some("auto")).unwrap(), Mode::Auto);
        assert_eq!(Mode::of(Some("text")).unwrap(), Mode::Text);
        assert_eq!(Mode::of(Some("off")).unwrap(), Mode::Off);
        let error = Mode::of(Some("on")).unwrap_err().to_string();
        assert!(error.contains("\"on\" is not auto, text or off"), "{error}");
        assert!(Mode::Auto.shown() && Mode::Text.shown() && !Mode::Off.shown());
        assert!(Mode::Auto.asks() && !Mode::Text.asks() && !Mode::Off.asks());
    }

    #[test]
    fn the_image_goes_under_the_caption_past_the_labels() {
        assert_eq!(image_width(59), 14);
        assert_eq!(image_width(60), 28);
        assert_eq!(
            image_area(Rect::new(0, 3, 100, 3), 28),
            Rect::new(10, 4, 28, 2)
        );
        // Cut to the band on a narrow screen.
        assert_eq!(
            image_area(Rect::new(0, 3, 20, 3), 14),
            Rect::new(10, 4, 10, 2)
        );
    }

    #[test]
    fn a_sparkline_scales_each_day_against_the_largest() {
        assert_eq!(spark(&[0, 1, 2, 4, 8], 8), "·▁▂▄█");
        assert_eq!(spark(&[3, 0], 3), "█·");
        assert_eq!(spark(&[1], 100), "▁");
        assert_eq!(spark(&[0, 0], 0), "··");
    }

    #[test]
    fn a_time_to_answer_is_short() {
        let s = Duration::from_secs;
        assert_eq!(short(s(0)), "0s");
        assert_eq!(short(s(59)), "59s");
        assert_eq!(short(s(60)), "1m");
        assert_eq!(short(s(3_599)), "59m");
        assert_eq!(short(s(3_600)), "1h");
        assert_eq!(short(s(4_320)), "1h12m");
        assert_eq!(short(s(172_799)), "47h59m");
        assert_eq!(short(s(172_800)), "2d");
    }
}
