//! IMAP `date-time`, as INTERNALDATE carries it: `14-Oct-2026 10:19:00 +0900`, the day possibly
//! space-padded. Month names are the fixed English abbreviations.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MONTHS: [&[u8; 3]; 12] = [
    b"JAN", b"FEB", b"MAR", b"APR", b"MAY", b"JUN", b"JUL", b"AUG", b"SEP", b"OCT", b"NOV", b"DEC",
];

pub(super) fn parse(text: &str) -> Option<SystemTime> {
    let mut s = Scanner {
        bytes: text.as_bytes(),
        index: 0,
    };
    s.skip_spaces();
    let day = s.number(1, 2)?;
    s.expect(b'-')?;
    let month = s.month()?;
    s.expect(b'-')?;
    let year = s.number(4, 4)?;
    s.require_spaces()?;
    let hour = s.number(2, 2)?;
    s.expect(b':')?;
    let minute = s.number(2, 2)?;
    s.expect(b':')?;
    let second = s.number(2, 2)?;
    s.require_spaces()?;
    let sign = s.sign()?;
    let zone = s.number(4, 4)?;
    s.skip_spaces();
    if s.index != s.bytes.len() {
        return None;
    }

    let (zone_hours, zone_minutes) = (zone / 100, zone % 100);
    let valid = (1..=days_in_month(month, year)).contains(&day)
        && (0..=23).contains(&hour)
        && (0..=59).contains(&minute)
        && (0..=60).contains(&second)
        && (0..=23).contains(&zone_hours)
        && (0..=59).contains(&zone_minutes);
    if !valid {
        return None;
    }
    let offset = sign * (zone_hours * 3600 + zone_minutes * 60);
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    let magnitude = Duration::from_secs(seconds.unsigned_abs());
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(magnitude)
    } else {
        UNIX_EPOCH.checked_sub(magnitude)
    }
}

fn days_in_month(month: i64, year: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (H. Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

struct Scanner<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Scanner<'_> {
    fn skip_spaces(&mut self) -> bool {
        let start = self.index;
        while self.bytes.get(self.index) == Some(&b' ') {
            self.index += 1;
        }
        self.index > start
    }

    fn require_spaces(&mut self) -> Option<()> {
        self.skip_spaces().then_some(())
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.bytes.get(self.index) == Some(&byte)).then(|| self.index += 1)
    }

    /// Up to `most` digits, which must be at least `least`.
    fn number(&mut self, least: usize, most: usize) -> Option<i64> {
        let digits = self.bytes[self.index..]
            .iter()
            .take(most)
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits < least {
            return None;
        }
        let value = self.bytes[self.index..self.index + digits]
            .iter()
            .fold(0, |value, digit| value * 10 + i64::from(digit - b'0'));
        self.index += digits;
        Some(value)
    }

    fn month(&mut self) -> Option<i64> {
        let name = self.bytes.get(self.index..self.index + 3)?;
        let position = MONTHS
            .iter()
            .position(|month| name.eq_ignore_ascii_case(month.as_slice()))?;
        self.index += 3;
        Some(i64::try_from(position).expect("twelve months") + 1)
    }

    fn sign(&mut self) -> Option<i64> {
        match self.bytes.get(self.index)? {
            b'+' => {
                self.index += 1;
                Some(1)
            }
            b'-' => {
                self.index += 1;
                Some(-1)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Option<SystemTime> {
        let magnitude = Duration::from_secs(seconds.unsigned_abs());
        if seconds >= 0 {
            UNIX_EPOCH.checked_add(magnitude)
        } else {
            UNIX_EPOCH.checked_sub(magnitude)
        }
    }

    #[test]
    fn valid_dates() {
        for (text, seconds) in [
            ("14-Oct-2026 10:19:00 +0900", 1_791_940_740),
            (" 4-Oct-2026 00:00:00 +0000", 1_791_072_000),
            ("4-Oct-2026 00:00:00 +0000", 1_791_072_000),
            ("04-oct-2026 00:00:00 +0000", 1_791_072_000),
            ("29-Feb-2024 23:59:59 -0700", 1_709_276_399),
            ("01-Jan-1970 00:00:00 +0000", 0),
            ("31-Dec-1969 23:00:00 -0130", 1800),
            ("15-OCT-2026 09:30:05 +0530", 1_792_036_805),
            ("  15-Oct-2026 09:30:05 +0530  ", 1_792_036_805),
            ("31-Dec-1969 23:59:59 +0000", -1),
        ] {
            assert_eq!(parse(text), at(seconds), "{text:?}");
        }
    }

    #[test]
    fn invalid_dates() {
        for text in [
            "",
            "yesterday",
            "29-Feb-2026 00:00:00 +0000",
            "31-Apr-2026 00:00:00 +0000",
            "00-Oct-2026 00:00:00 +0000",
            "14-Okt-2026 10:19:00 +0900",
            "14-Oct-26 10:19:00 +0900",
            "14-Oct-2026 24:00:00 +0900",
            "14-Oct-2026 10:60:00 +0900",
            "14-Oct-2026 10:19 +0900",
            "14-Oct-2026 10:19:00",
            "14-Oct-2026 10:19:00 0900",
            "14-Oct-2026 10:19:00 +09:00",
            "14-Oct-2026 10:19:00 +0960",
            "14-Oct-2026 10:19:00 +0900 KST",
            "Wed, 14 Oct 2026 10:19:00 +0900",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
