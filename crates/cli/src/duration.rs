//! Durations as flags and the config file take them: Go's spelling, such
//! as `30m`, `72h` or `1h30m`, and days, such as `7d`.

use std::time::Duration;

/// Reads a duration: `0`, or one or more numbers, each with a unit, such as
/// `1.5h`, `1h30m` or `7d`. The units are `ns`, `us` (or `µs`), `ms`, `s`,
/// `m`, `h` and `d`, a day of 24 hours. A duration is never negative.
///
/// # Errors
///
/// The reason, for clap to report, when `text` is not a duration.
pub(crate) fn parse(text: &str) -> Result<Duration, String> {
    let invalid = || format!("{text:?} is not a duration, such as 30m, 72h, 1h30m or 7d");
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    let mut rest = text.strip_prefix('+').unwrap_or(text);
    if rest.is_empty() {
        return Err(invalid());
    }
    let mut nanos: u128 = 0;
    while !rest.is_empty() {
        let number_len = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or_else(invalid)?;
        let (number, after) = rest.split_at(number_len);
        let unit_len = after
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(unit_len);
        let scale: u128 = match unit {
            "ns" => 1,
            "us" | "µs" | "μs" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60 * 1_000_000_000,
            "h" => 3_600 * 1_000_000_000,
            "d" => 86_400 * 1_000_000_000,
            _ => return Err(invalid()),
        };
        nanos = nanos
            .checked_add(scaled(number, scale).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        rest = after;
    }
    let secs = u64::try_from(nanos / 1_000_000_000).map_err(|_| invalid())?;
    let subsec = u32::try_from(nanos % 1_000_000_000).map_err(|_| invalid())?;
    Ok(Duration::new(secs, subsec))
}

/// `number`, digits with at most one `.` among them, times `scale`
/// nanoseconds, with any part of a nanosecond dropped.
fn scaled(number: &str, scale: u128) -> Option<u128> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() || fraction.contains('.') {
        return None;
    }
    let whole: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut total = whole.checked_mul(scale)?;
    let mut place = scale;
    for digit in fraction.bytes() {
        place /= 10;
        total = total.checked_add(u128::from(digit - b'0') * place)?;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_go_writes_them() {
        for (text, want) in [
            ("0", Duration::ZERO),
            ("0s", Duration::ZERO),
            ("30m", Duration::from_mins(30)),
            ("72h", Duration::from_hours(72)),
            ("1h30m", Duration::from_mins(90)),
            ("1.5h", Duration::from_mins(90)),
            ("7d", Duration::from_hours(168)),
            ("14d", Duration::from_hours(336)),
            ("1.5d", Duration::from_hours(36)),
            ("1d12h", Duration::from_hours(36)),
            (".5s", Duration::from_millis(500)),
            ("10ms", Duration::from_millis(10)),
            ("3us", Duration::from_micros(3)),
            ("3µs", Duration::from_micros(3)),
            ("7ns", Duration::from_nanos(7)),
            ("+2s", Duration::from_secs(2)),
            ("1.0000000001s", Duration::from_secs(1)),
        ] {
            assert_eq!(parse(text), Ok(want), "{text}");
        }
    }

    #[test]
    fn what_is_not_a_duration_is_refused() {
        for text in [
            "",
            "soon",
            "30",
            "1w",
            "d",
            "h",
            ".s",
            "1..5s",
            "-1h",
            "1h-2m",
            "+",
            "99999999999999999999999h",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        assert_eq!(
            parse("soon").unwrap_err(),
            "\"soon\" is not a duration, such as 30m, 72h, 1h30m or 7d"
        );
    }
}
