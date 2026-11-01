//! Times: RFC 3339, read strictly and written in UTC.

use alloc::format;
use core::fmt;
use core::str::FromStr;

use serde::{Serialize, Serializer};

use crate::error::Error;

/// An instant, as a record gives one: an RFC 3339 `date-time`.
///
/// A time reads when it is `YYYY-MM-DD`, then `T` or `t`, then `hh:mm:ss`,
/// then optionally `.` and one or more digits, then `Z`, `z` or an offset
/// `+hh:mm` or `-hh:mm`. The date must be a day of the calendar, the hour 00
/// to 23, the minute 00 to 59 and the second 00 to 59, and an offset's hour
/// 00 to 23 and minute 00 to 59. Anything else does not read, such as a space
/// in place of the `T`, or a leap second (`23:59:60`).
///
/// A time is written in UTC, ending in `Z`, with only as many fractional
/// digits as it needs, such as `2026-09-16T22:15:41.605108Z`.
///
/// It holds nanoseconds: fractional digits past the ninth are dropped when a
/// time is read. It holds the instants whose date in UTC is in the years 0000
/// to 9999, which are the ones RFC 3339 can write in UTC, so
/// `0000-01-01T00:00:00+01:00` does not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time {
    /// Seconds since 1970-01-01T00:00:00Z.
    secs: i64,
    nanos: u32,
}

/// 0000-01-01T00:00:00Z, in seconds since 1970.
const MIN_SECS: i64 = -62_167_219_200;
/// 9999-12-31T23:59:59Z, in seconds since 1970.
const MAX_SECS: i64 = 253_402_300_799;
const DAY: i64 = 86_400;

impl Time {
    /// The time `secs` seconds and `nanos` nanoseconds after
    /// 1970-01-01T00:00:00Z. `None` when `nanos` is a second or more, or the
    /// time is outside the years 0000 to 9999 in UTC.
    #[must_use]
    pub fn from_unix(secs: i64, nanos: u32) -> Option<Time> {
        ((MIN_SECS..=MAX_SECS).contains(&secs) && nanos < 1_000_000_000)
            .then_some(Time { secs, nanos })
    }

    /// Whole seconds since 1970-01-01T00:00:00Z.
    #[must_use]
    pub fn unix_secs(self) -> i64 {
        self.secs
    }

    /// The nanoseconds after [`unix_secs`](Time::unix_secs).
    #[must_use]
    pub fn subsec_nanos(self) -> u32 {
        self.nanos
    }
}

impl FromStr for Time {
    type Err = Error;

    /// Reads an RFC 3339 `date-time`. A time that does not read is
    /// [`Malformed`](crate::Category::Malformed).
    fn from_str(s: &str) -> Result<Time, Error> {
        read(s.as_bytes()).ok_or_else(|| Error::malformed(format!("{s:?} is not an RFC 3339 time")))
    }
}

fn read(text: &[u8]) -> Option<Time> {
    let (head, rest) = text.split_at_checked(19)?;
    let (date, clock) = head.split_at(10);
    let [b'T' | b't', clock @ ..] = clock else {
        return None;
    };
    let (year, month, day) = read_date(date)?;
    let (hour, minute, second) = read_clock(clock)?;
    let (nanos, zone) = read_fraction(rest)?;
    let offset = read_offset(zone)?;

    if !is_date(year, month, day) || !is_clock(hour, minute, second) {
        return None;
    }
    let secs = days_from_civil(year, month, day) * DAY + hour * 3600 + minute * 60 + second;
    Time::from_unix(secs - offset, nanos)
}

/// The year, month and day of a `YYYY-MM-DD`, not yet checked against the
/// calendar.
fn read_date(date: &[u8]) -> Option<(i64, i64, i64)> {
    let [y1, y2, y3, y4, b'-', mo1, mo2, b'-', d1, d2] = *date else {
        return None;
    };
    Some((
        number(&[y1, y2, y3, y4])?,
        number(&[mo1, mo2])?,
        number(&[d1, d2])?,
    ))
}

/// The hour, minute and second of an `hh:mm:ss`, not yet checked against
/// the clock.
fn read_clock(clock: &[u8]) -> Option<(i64, i64, i64)> {
    let [h1, h2, b':', mi1, mi2, b':', s1, s2] = *clock else {
        return None;
    };
    Some((number(&[h1, h2])?, number(&[mi1, mi2])?, number(&[s1, s2])?))
}

/// The nanoseconds of an optional `.` and digits, and the text after them.
/// Digits past the ninth are dropped.
fn read_fraction(rest: &[u8]) -> Option<(u32, &[u8])> {
    let [b'.', fraction @ ..] = rest else {
        return Some((0, rest));
    };
    let digits = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let nanos = (0..9).fold(0, |nanos, i| {
        let digit = if i < digits { fraction[i] - b'0' } else { 0 };
        nanos * 10 + u32::from(digit)
    });
    Some((nanos, &fraction[digits..]))
}

/// The seconds east of UTC of a `Z`, `z`, `+hh:mm` or `-hh:mm`.
fn read_offset(zone: &[u8]) -> Option<i64> {
    let [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] = *zone else {
        return matches!(zone, [b'Z' | b'z']).then_some(0);
    };
    let (hours, minutes) = (number(&[h1, h2])?, number(&[m1, m2])?);
    if hours > 23 || minutes > 59 {
        return None;
    }
    let offset = hours * 3600 + minutes * 60;
    Some(if sign == b'-' { -offset } else { offset })
}

/// Whether the date is a day of the calendar.
fn is_date(year: i64, month: i64, day: i64) -> bool {
    (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)
}

/// Whether the time of day is 00:00:00 to 23:59:59.
fn is_clock(hour: i64, minute: i64, second: i64) -> bool {
    hour <= 23 && minute <= 59 && second <= 59
}

/// The number the ASCII digits spell, or `None` if any is not a digit.
fn number(digits: &[u8]) -> Option<i64> {
    digits.iter().try_fold(0, |n, &c| {
        c.is_ascii_digit().then(|| n * 10 + i64::from(c - b'0'))
    })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar, from
/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date of a day counted from 1970-01-01, from Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

impl fmt::Display for Time {
    /// Writes the time in UTC, with only the fractional digits it needs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = civil_from_days(self.secs.div_euclid(DAY));
        let secs = self.secs.rem_euclid(DAY);
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        )?;
        if self.nanos != 0 {
            let (mut fraction, mut width) = (self.nanos, 9);
            while fraction % 10 == 0 {
                fraction /= 10;
                width -= 1;
            }
            write!(f, ".{fraction:0width$}")?;
        }
        f.write_str("Z")
    }
}

impl Serialize for Time {
    /// Serializes the time as it is written: in UTC, as a string.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    fn written(s: &str) -> Option<alloc::string::String> {
        s.parse::<Time>().ok().map(|t| t.to_string())
    }

    #[test]
    fn a_time_is_written_in_utc_with_the_digits_it_needs() {
        for (read, write) in [
            ("2026-09-15T09:02:00Z", "2026-09-15T09:02:00Z"),
            ("2026-09-15t09:02:00z", "2026-09-15T09:02:00Z"),
            ("2026-09-15T11:45:00+02:00", "2026-09-15T09:45:00Z"),
            ("2026-09-15T00:15:00+00:30", "2026-09-14T23:45:00Z"),
            ("2026-12-31T23:30:00-01:00", "2027-01-01T00:30:00Z"),
            ("2026-09-15T09:02:00-00:00", "2026-09-15T09:02:00Z"),
            ("2026-09-15T09:02:00.500Z", "2026-09-15T09:02:00.5Z"),
            ("2026-09-15T09:02:00.000Z", "2026-09-15T09:02:00Z"),
            ("2026-09-16T22:15:41.605108Z", "2026-09-16T22:15:41.605108Z"),
            (
                "2026-09-15T09:02:00.123456789Z",
                "2026-09-15T09:02:00.123456789Z",
            ),
            (
                "2026-09-15T09:02:00.1234567891Z",
                "2026-09-15T09:02:00.123456789Z",
            ),
            ("0001-01-01T00:00:00Z", "0001-01-01T00:00:00Z"),
            ("0000-01-01T00:00:00Z", "0000-01-01T00:00:00Z"),
            (
                "9999-12-31T23:59:59.999999999Z",
                "9999-12-31T23:59:59.999999999Z",
            ),
            ("1969-12-31T23:59:59.5Z", "1969-12-31T23:59:59.5Z"),
            ("2024-02-29T12:00:00Z", "2024-02-29T12:00:00Z"),
            ("2000-02-29T12:00:00Z", "2000-02-29T12:00:00Z"),
            ("0000-02-29T12:00:00Z", "0000-02-29T12:00:00Z"),
        ] {
            assert_eq!(written(read).as_deref(), Some(write), "{read}");
        }
    }

    #[test]
    fn what_rfc_3339_does_not_allow_does_not_read() {
        for time in [
            "",
            "2026-09-15",
            "2026-09-15T09:02:00",
            "2026-09-15 09:02:00Z",
            "2026-09-15T9:02:00Z",
            "2026-09-15T09:02:00,5Z",
            "2026-09-15T09:02:00.Z",
            "2026-09-15T09:02:00+24:00",
            "2026-09-15T09:02:00+01:60",
            "2026-09-15T09:02:00+0100",
            "2026-09-15T09:02:00+01",
            "2016-12-31T23:59:60Z",
            "2026-09-15T24:00:00Z",
            "2026-09-15T09:60:00Z",
            "2026-13-01T00:00:00Z",
            "2026-00-01T00:00:00Z",
            "2026-09-00T00:00:00Z",
            "2026-09-31T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "+2026-09-15T09:02:00Z",
            "2026-09-15T09:02:00Z ",
            " 2026-09-15T09:02:00Z",
            "2026-09-15T09:02:00ZZ",
            "２026-09-15T09:02:00Z",
            "0000-01-01T00:30:00+01:00",
            "9999-12-31T23:30:00-01:00",
        ] {
            let err = time.parse::<Time>().expect_err(time);
            assert_eq!(err.category(), crate::Category::Malformed, "{time}");
        }
    }

    #[test]
    fn a_time_whose_only_fault_is_a_separator_does_not_read() {
        for time in [
            "2026/09/16T00:00:00Z",
            "2026-09/16T00:00:00Z",
            "2026/09-16T00:00:00Z",
            "2026.09-16T00:00:00Z",
            "2026-09.16T00:00:00Z",
            "2026-09-16T00.00:00Z",
            "2026-09-16T00:00.00Z",
            "2026-09-16T00-00:00Z",
            "2026-09-16T00:00-00Z",
        ] {
            let err = time.parse::<Time>().expect_err(time);
            assert_eq!(err.category(), crate::Category::Malformed, "{time}");
        }
    }

    #[test]
    fn times_order_by_instant_whatever_their_offset() {
        let earlier: Time = "2026-09-15T11:44:59.9+02:00".parse().unwrap();
        let later: Time = "2026-09-15T09:45:00Z".parse().unwrap();
        assert!(earlier < later);
    }

    #[test]
    fn unix_seconds_go_both_ways() {
        let t: Time = "2026-09-15T09:02:00.25Z".parse().unwrap();
        assert_eq!(
            (t.unix_secs(), t.subsec_nanos()),
            (1_789_462_920, 250_000_000)
        );
        assert_eq!(Time::from_unix(1_789_462_920, 250_000_000), Some(t));
        assert_eq!(Time::from_unix(0, 1_000_000_000), None);
        assert_eq!(Time::from_unix(MIN_SECS - 1, 0), None);
        assert_eq!(Time::from_unix(MAX_SECS + 1, 0), None);
        assert_eq!(
            Time::from_unix(MIN_SECS, 0).unwrap().to_string(),
            "0000-01-01T00:00:00Z"
        );
        assert_eq!(
            Time::from_unix(MAX_SECS, 0).unwrap().to_string(),
            "9999-12-31T23:59:59Z"
        );
    }

    #[test]
    fn every_day_of_four_centuries_reads_back_as_itself() {
        for days in days_from_civil(1600, 1, 1)..days_from_civil(2000, 1, 1) {
            let (year, month, day) = civil_from_days(days);
            assert!((1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day));
            assert_eq!(days_from_civil(year, month, day), days);
        }
    }

    #[test]
    fn an_offset_reads_up_to_23_59() {
        assert_eq!(
            written("2026-09-15T23:59:00+23:59"),
            Some("2026-09-15T00:00:00Z".into())
        );
        assert_eq!(
            written("2026-09-15T00:00:00-23:59"),
            Some("2026-09-15T23:59:00Z".into())
        );
    }
}
