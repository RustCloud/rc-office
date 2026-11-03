//! RFC 3339 times as the server writes them (`2026-11-03T10:00:00.123456Z`),
//! without a date library.

/// An instant: seconds since 1970 and nanoseconds.
pub type Instant = (i64, u32);

/// Parses `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`.
pub fn parse(text: &str) -> Option<Instant> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let part = text.get(from..to)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &text[19..];
    let mut nanos: u32 = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let mut scale = 100_000_000;
        for digit in fraction[..digits].bytes().take(9) {
            nanos += u32::from(digit - b'0') * scale;
            scale /= 10;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && matches!(rest.as_bytes()[0], b'+' | b'-') && rest.as_bytes()[3] == b':' => {
            let hours: i64 = rest[1..3].parse().ok()?;
            let minutes: i64 = rest[4..6].parse().ok()?;
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (hours * 3600 + minutes * 60)
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some((seconds, nanos))
}

/// Milliseconds since 1970.
pub fn parse_ms(text: &str) -> Option<i64> {
    parse(text).map(|(seconds, nanos)| seconds * 1000 + i64::from(nanos / 1_000_000))
}

/// Whether two texts name the same instant (formats may differ).
pub fn same(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.trim() == b.trim(),
    }
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = (month + 9) % 12;
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse("1970-01-01T00:00:00Z"), Some((0, 0)));
        assert_eq!(parse("2000-03-01T00:00:00Z"), Some((951_868_800, 0)));
        assert_eq!(parse("2026-11-03T10:00:00.5Z"), Some((1_793_700_000, 500_000_000)));
        assert_eq!(parse("2026-11-03T12:00:00+02:00"), parse("2026-11-03T10:00:00Z"));
        assert_eq!(parse_ms("2026-11-03T10:00:00.123456Z"), Some(1_793_700_000_123));
        assert_eq!(parse("2026-11-03"), None);
        assert_eq!(parse("2026-13-03T10:00:00Z"), None);
        assert_eq!(parse("2026-11-03T10:00:00"), None);
    }

    #[test]
    fn compares() {
        assert!(same("2026-11-03T10:00:00.120Z", "2026-11-03T10:00:00.12+00:00"));
        assert!(!same("2026-11-03T10:00:00.12Z", "2026-11-03T10:00:00.13Z"));
        assert!(same("weird", "weird"));
    }
}
