//! The stats strip as an image (TU-13), through the kitty graphics
//! protocol, where the terminal speaks it: kitty, Ghostty, and Rex, which
//! is built on libghostty.
//!
//! The screen asks once, at start, before it reads keys: a query for the
//! protocol, then DA1, which every terminal answers. A kitty `OK` before
//! the DA1 reply means images; DA1 alone, or nothing within a second, means
//! text. tmux answers DA1 itself and drops the query, so it gets text.
//!
//! The image is a bitmap the strip draws, sent as zlib-compressed RGBA, and
//! put over the strip's cells with a placement at the cursor. Unicode
//! placeholders would keep it in ratatui's buffer, but libghostty does not
//! draw them for an embedder such as Rex (ghostty-org/ghostty#14698), while
//! it does draw a placement. So the placement is moved as the strip moves,
//! the image deleted while help is up, and sent again only as the strip
//! changes. The
//! image's id is the process's, so that two inboxes in one terminal keep to
//! their own; it is deleted when the strip is hidden and when the terminal
//! is put back, on a panic or a signal too.

use std::io::{self, Write as _};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use ratatui::layout::Rect;

/// The longest the start waits for the terminal's answers. Every terminal
/// answers DA1, which ends the wait, so this is reached only on a link
/// slower than a round trip of it, or a terminal that does not answer: an
/// answer later than this would be read as keys.
const WAIT: Duration = Duration::from_secs(1);

/// The most base64 one escape of the protocol carries.
const CHUNK: usize = 4096;

/// The opened series: a cyan that reads on dark and on light.
const OPENED: [u8; 4] = [0x2b, 0xa6, 0xc4, 0xff];
/// The answered series: a green that reads on dark and on light.
const ANSWERED: [u8; 4] = [0x4c, 0xaf, 0x50, 0xff];
/// The line under the bars, half seen.
const BASE: [u8; 4] = [0x80, 0x80, 0x80, 0x80];

/// A cell's size in pixels when the terminal does not say: the strip's
/// placement scales the image to its cells, so this only sets how sharp
/// it is.
const CELL: (u32, u32) = (10, 20);

/// The image's id: the process's, which is never 0.
pub(crate) fn id() -> u32 {
    std::process::id().max(1)
}

/// What the terminal's answers to the query and DA1 say, so far.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// The protocol's `OK`, then DA1: images.
    Kitty,
    /// DA1 with no `OK` before it, or an error for the query: text.
    Text,
    /// Not all there yet.
    Partial,
}

/// The query, of a 1x1 image never kept, then DA1.
pub(crate) const QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";

/// Reads the terminal's answers to [`QUERY`].
pub(crate) fn reply(read: &[u8]) -> Reply {
    let Some(da1) = find(read, b"\x1b[?").filter(|at| read[*at..].contains(&b'c')) else {
        return Reply::Partial;
    };
    let kitty = find(&read[..da1], b"\x1b_Gi=31;").is_some_and(|at| {
        let rest = &read[at + 8..da1];
        rest.starts_with(b"OK\x1b\\")
    });
    if kitty { Reply::Kitty } else { Reply::Text }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Whether the terminal draws kitty images: asked on the terminal in raw
/// mode, before anything reads keys. Text when it cannot be asked.
#[cfg(unix)]
pub(crate) fn supported() -> bool {
    use ratatui::crossterm::terminal;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    if terminal::enable_raw_mode().is_err() {
        return false;
    }
    let mut out = io::stdout();
    let asked = out.write_all(QUERY).and_then(|()| out.flush()).is_ok();
    let mut read = Vec::new();
    let end = Instant::now() + WAIT;
    let stdin = io::stdin();
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if !asked || left.is_zero() || reply(&read) != Reply::Partial {
            break;
        }
        let timeout = Timespec::try_from(left).ok();
        let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
        if !matches!(poll(&mut fds, timeout.as_ref()), Ok(n) if n > 0) {
            break;
        }
        let mut buf = [0u8; 256];
        match rustix::io::read(&stdin, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => read.extend_from_slice(&buf[..n]),
        }
    }
    let _ = terminal::disable_raw_mode();
    reply(&read) == Reply::Kitty
}

/// Not asked off unix: text.
#[cfg(not(unix))]
pub(crate) fn supported() -> bool {
    false
}

/// The bars of `opened` and `answered`, a day each, oldest left, in an
/// RGBA bitmap `width` by `height` pixels: in each day the opened bar on
/// the left and the answered on the right, on one scale, over a line half
/// seen. A day with none has no bar. The rest is clear, so the terminal's
/// background shows.
pub(crate) fn bitmap(opened: &[u32], answered: &[u32], width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let mut fill = |x0: u32, x1: u32, y0: u32, y1: u32, colour: [u8; 4]| {
        for y in y0..y1.min(height) {
            for x in x0..x1.min(width) {
                let at = ((y * width + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&colour);
            }
        }
    };
    if height == 0 || width == 0 {
        return rgba;
    }
    fill(0, width, height - 1, height, BASE);
    let days = u32::try_from(opened.len().max(answered.len())).unwrap_or(u32::MAX);
    let top = opened.iter().chain(answered).copied().max().unwrap_or(0);
    if days == 0 || top == 0 {
        return rgba;
    }
    // The bars stand on the line, and the tallest reaches the top.
    let room = height - 1;
    let tall = |n: u32| {
        u32::try_from((u64::from(n) * u64::from(room)).div_ceil(u64::from(top))).unwrap_or(room)
    };
    for day in 0..days {
        let x0 = day * width / days;
        let x1 = (day + 1) * width / days;
        let gap = ((x1 - x0) / 6).max(1).min((x1 - x0) / 2);
        let mid = x0 + (x1 - x0) / 2;
        for (n, left, right, colour) in [
            (opened.get(day as usize), x0 + gap, mid, OPENED),
            (
                answered.get(day as usize),
                mid,
                x1.saturating_sub(gap),
                ANSWERED,
            ),
        ] {
            let n = n.copied().unwrap_or(0);
            if n > 0 && left < right {
                fill(left, right, room - tall(n), room, colour);
            }
        }
    }
    rgba
}

/// The escapes that send `rgba`, `width` by `height`, as image `id`:
/// zlib-compressed, in base64, in chunks the protocol takes, and quiet, so
/// the terminal answers nothing.
pub(crate) fn transmit(id: u32, width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing to a Vec does not fail.
    let _ = zlib.write_all(rgba);
    let packed = zlib.finish().unwrap_or_default();
    let data = STANDARD.encode(packed);
    let mut out = Vec::new();
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(CHUNK).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i < last);
        if i == 0 {
            out.extend_from_slice(
                format!("\x1b_Ga=t,f=32,o=z,s={width},v={height},i={id},q=2,m={more};").as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// The escapes that put image `id` over `area`, from its top left cell:
/// its one placement, moved there if it was elsewhere, and scaled to the
/// cells. The cursor is put back where it was, as a form's field shows it.
pub(crate) fn place(id: u32, area: Rect) -> Vec<u8> {
    format!(
        "\x1b7\x1b[{};{}H\x1b_Ga=p,i={id},p=1,c={},r={},C=1,q=2\x1b\\\x1b8",
        area.y + 1,
        area.x + 1,
        area.width,
        area.height
    )
    .into_bytes()
}

/// The escape that deletes image `id`, and its placement with it.
pub(crate) fn delete(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\").into_bytes()
}

/// The image as the terminal has it: what was sent, and where it is put.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// The counts and the size in pixels of the image sent, if one was.
    sent: Option<(Vec<u32>, Vec<u32>, u32, u32)>,
    /// Where it is put, if it is.
    placed: Option<Rect>,
}

impl Shown {
    /// The escapes that bring the terminal from what it has to `want`: the
    /// counts opened and answered, over these cells, each `cell` pixels; or
    /// no image, which deletes the one it has. Empty when it has it
    /// already.
    pub(crate) fn sync(
        &mut self,
        id: u32,
        want: Option<(&[u32], &[u32], Rect)>,
        cell: (u32, u32),
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let Some((opened, answered, area)) = want else {
            if self.sent.is_some() {
                out.extend(delete(id));
                self.forget();
            }
            return out;
        };
        let (cw, ch) = if cell.0 == 0 || cell.1 == 0 {
            CELL
        } else {
            cell
        };
        let (width, height) = (u32::from(area.width) * cw, u32::from(area.height) * ch);
        let sent = (opened.to_vec(), answered.to_vec(), width, height);
        if self.sent.as_ref() != Some(&sent) {
            out.extend(transmit(
                id,
                width,
                height,
                &bitmap(opened, answered, width, height),
            ));
            self.sent = Some(sent);
            // A new image under the id takes its placement off.
            self.placed = None;
        }
        if self.placed != Some(area) {
            out.extend(place(id, area));
            self.placed = Some(area);
        }
        out
    }

    /// The escape that deletes what the terminal has, if anything, which
    /// is then forgotten: for a screen drawn afresh, such as after a
    /// resize, where a terminal may keep the old placement.
    pub(crate) fn reset(&mut self, id: u32) -> Vec<u8> {
        if self.sent.is_none() {
            return Vec::new();
        }
        self.forget();
        delete(id)
    }

    /// The terminal has none of it: deleted, or the screen taken again.
    pub(crate) fn forget(&mut self) {
        *self = Shown::default();
    }
}

/// The size of a cell in pixels, as the terminal says, or none.
pub(crate) fn cell() -> (u32, u32) {
    ratatui::crossterm::terminal::window_size()
        .ok()
        .filter(|size| size.columns > 0 && size.rows > 0)
        .map_or((0, 0), |size| {
            (
                u32::from(size.width / size.columns),
                u32::from(size.height / size.rows),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kitty_ok_before_da1_is_images() {
        assert_eq!(reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c"), Reply::Kitty);
    }

    #[test]
    fn da1_alone_or_an_error_is_text() {
        assert_eq!(reply(b"\x1b[?62;22c"), Reply::Text);
        assert_eq!(reply(b"\x1b[?1;2c"), Reply::Text);
        assert_eq!(
            reply(b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\\x1b[?62c"),
            Reply::Text
        );
    }

    #[test]
    fn no_reply_or_half_of_one_is_not_yet_an_answer() {
        assert_eq!(reply(b""), Reply::Partial);
        assert_eq!(reply(b"\x1b_Gi=31;OK\x1b\\"), Reply::Partial);
        assert_eq!(reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;2"), Reply::Partial);
    }

    /// The pixel at `x`, `y` of a bitmap `width` wide.
    fn at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_bitmap_draws_each_day_s_two_bars_on_one_scale() {
        // Two days, 12 pixels wide and 5 high: a line on the bottom row,
        // and 4 rows above it for the bars.
        let rgba = bitmap(&[2, 0], &[1, 4], 12, 5);
        assert_eq!(rgba.len(), 12 * 5 * 4);
        for x in 0..12 {
            assert_eq!(at(&rgba, 12, x, 4), BASE);
        }
        // Day one: 2 of 4 opened, half the room, in its left half past the
        // gap; 1 of 4 answered, a quarter, in its right half.
        assert_eq!(at(&rgba, 12, 0, 3), [0; 4], "the gap");
        assert_eq!(at(&rgba, 12, 1, 3), OPENED);
        assert_eq!(at(&rgba, 12, 2, 2), OPENED);
        assert_eq!(at(&rgba, 12, 2, 1), [0; 4]);
        assert_eq!(at(&rgba, 12, 3, 3), ANSWERED);
        assert_eq!(at(&rgba, 12, 3, 2), [0; 4]);
        assert_eq!(at(&rgba, 12, 5, 3), [0; 4], "the gap");
        // Day two: no opened bar; the answered one is the tallest.
        assert_eq!(at(&rgba, 12, 7, 3), [0; 4]);
        assert_eq!(at(&rgba, 12, 9, 0), ANSWERED);
        assert_eq!(at(&rgba, 12, 10, 0), ANSWERED);
        assert_eq!(at(&rgba, 12, 11, 0), [0; 4], "the gap");
    }

    #[test]
    fn a_bitmap_of_no_cases_is_only_the_line() {
        let rgba = bitmap(&[0, 0], &[0, 0], 4, 3);
        let set = rgba.chunks(4).filter(|p| p[3] != 0).count();
        assert_eq!(set, 4);
        assert!(bitmap(&[], &[], 0, 0).is_empty());
    }

    #[test]
    fn an_image_is_sent_compressed_in_chunks() {
        let small = transmit(7, 2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let small = String::from_utf8(small).unwrap();
        assert!(
            small.starts_with("\x1b_Ga=t,f=32,o=z,s=2,v=1,i=7,q=2,m=0;"),
            "{small}"
        );
        assert!(small.ends_with("\x1b\\"));
        assert_eq!(small.matches("\x1b_G").count(), 1);
        // The data reads back as the pixels.
        let data = &small["\x1b_Ga=t,f=32,o=z,s=2,v=1,i=7,q=2,m=0;".len()..small.len() - 2];
        let packed = STANDARD.decode(data).unwrap();
        let mut plain = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&packed[..]), &mut plain)
            .unwrap();
        assert_eq!(plain, [1, 2, 3, 4, 5, 6, 7, 8]);

        // Noise does not compress, so this takes three chunks.
        let mut x: u32 = 0x9e37_79b9;
        let noise: Vec<u8> = (0..7_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.to_le_bytes()[0]
            })
            .collect();
        let big = String::from_utf8(transmit(7, 1, 1, &noise)).unwrap();
        let chunks: Vec<&str> = big.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert_eq!(chunks.len(), 3, "{}", chunks.len());
        assert!(chunks[0].contains(",m=1;"));
        assert!(chunks[1].starts_with("\x1b_Gm=1;"));
        assert!(chunks[2].starts_with("\x1b_Gm=0;"));
        assert!(chunks.iter().all(|c| c.len() <= CHUNK + 40));
    }

    #[test]
    fn the_placement_and_the_deletes_are_quiet_escapes() {
        assert_eq!(
            place(7, Rect::new(10, 4, 28, 2)),
            b"\x1b7\x1b[5;11H\x1b_Ga=p,i=7,p=1,c=28,r=2,C=1,q=2\x1b\\\x1b8"
        );
        assert_eq!(delete(7), b"\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
    }

    #[test]
    fn the_image_is_sent_and_put_again_only_as_it_changes() {
        let mut shown = Shown::default();
        let area = Rect::new(10, 4, 14, 2);
        let first = shown.sync(7, Some((&[1, 2], &[0, 1], area)), (8, 16));
        let text = String::from_utf8_lossy(&first);
        assert!(
            text.starts_with("\x1b_Ga=t,f=32,o=z,s=112,v=32,i=7,"),
            "{text}"
        );
        assert!(text.ends_with("\x1b7\x1b[5;11H\x1b_Ga=p,i=7,p=1,c=14,r=2,C=1,q=2\x1b\\\x1b8"));
        // The same again: nothing.
        assert!(
            shown
                .sync(7, Some((&[1, 2], &[0, 1], area)), (8, 16))
                .is_empty()
        );
        // Moved: put again, not sent.
        let lower = Rect::new(10, 5, 14, 2);
        assert_eq!(
            shown.sync(7, Some((&[1, 2], &[0, 1], lower)), (8, 16)),
            place(7, lower)
        );
        // New counts: sent and put.
        let counts = shown.sync(7, Some((&[1, 3], &[0, 1], lower)), (8, 16));
        assert!(counts.starts_with(b"\x1b_Ga=t,"));
        assert!(counts.ends_with(&place(7, lower)));
        // Hidden: deleted once.
        assert_eq!(shown.sync(7, None, (8, 16)), delete(7));
        assert!(shown.sync(7, None, (8, 16)).is_empty());
        // Shown again: sent and put.
        let again = shown.sync(7, Some((&[1, 3], &[0, 1], lower)), (8, 16));
        assert!(again.starts_with(b"\x1b_Ga=t,") && again.ends_with(&place(7, lower)));
        // Forgotten, as after the editor: sent again.
        shown.forget();
        assert!(
            shown
                .sync(7, Some((&[1, 3], &[0, 1], lower)), (8, 16))
                .starts_with(b"\x1b_Ga=t,")
        );
    }

    #[test]
    fn a_reset_deletes_what_was_sent_so_none_is_left_behind() {
        let mut shown = Shown::default();
        shown.sync(7, Some((&[1], &[1], Rect::new(0, 0, 3, 2))), (8, 16));
        // As on a resize to a screen with no room for the strip.
        assert_eq!(shown.reset(7), delete(7));
        assert!(shown.sync(7, None, (8, 16)).is_empty());
        assert!(shown.reset(7).is_empty());
    }

    #[test]
    fn a_cell_of_no_size_draws_at_a_default_size() {
        let mut shown = Shown::default();
        let text = shown.sync(7, Some((&[1], &[1], Rect::new(0, 0, 3, 2))), (0, 0));
        assert!(String::from_utf8_lossy(&text).starts_with("\x1b_Ga=t,f=32,o=z,s=30,v=40,"));
    }
}
