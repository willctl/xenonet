//! Times as the transcripts and XenoNet's files write them: ISO 8601, read
//! the way JS's `Date.parse` reads it, in milliseconds since 1970 (UTC).
//!
//! `Date.parse` reads ECMAScript's date time string format and, in V8, a
//! little more: a day past the end of its month rolls into the next one
//! ("2026-02-30" is March 2), the separator may be a space or a lowercase
//! `t`, the zone a lowercase `z` or an offset without its colon, and any
//! number of fraction digits counts to the millisecond. All of that reads
//! the same here. Two things don't: a date and time without a zone is
//! local time in JS and UTC here (Claude Code and Codex both write UTC with
//! a `Z`), and V8's fallback for other formats ("Oct 3 2026") isn't kept,
//! so those read as no time at all.

/// Days since 1970-01-01 of a day in the proleptic Gregorian calendar. A
/// day past the end of its month counts on into the next one.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year, month and day of a count of days since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Milliseconds since 1970 of a UTC day.
pub fn day_ms(year: i64, month: u32, day: u32) -> f64 {
    days_from_civil(year, month, day) as f64 * 86_400_000.0
}

/// JS's largest time: 100,000,000 days either side of 1970.
const MAX_MS: f64 = 8.64e15;

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn eat_any(&mut self, cs: &[u8]) -> Option<u8> {
        let c = self.peek()?;
        if cs.contains(&c) {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }
    // Exactly n digits.
    fn digits(&mut self, n: usize) -> Option<i64> {
        let end = self.i + n;
        if end > self.s.len() || !self.s[self.i..end].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let v = self.s[self.i..end].iter().fold(0i64, |v, d| v * 10 + i64::from(d - b'0'));
        self.i = end;
        Some(v)
    }
    fn done(&self) -> bool {
        self.i == self.s.len()
    }
}

/// When an ISO 8601 time is, as `Date.parse` reads it (see the module's
/// note): None where JS gives NaN.
pub fn parse_iso(text: &str) -> Option<f64> {
    let mut c = Cursor { s: text.as_bytes(), i: 0 };
    let year = match c.eat_any(b"+-") {
        Some(sign) => {
            let y = c.digits(6)?;
            // -000000 isn't a year (ECMAScript, 21.4.1.32.1).
            if sign == b'-' && y == 0 {
                return None;
            }
            if sign == b'-' {
                -y
            } else {
                y
            }
        }
        None => c.digits(4)?,
    };
    let (mut month, mut day) = (1, 1);
    if c.eat(b'-') {
        month = c.digits(2)?;
        if c.eat(b'-') {
            day = c.digits(2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut ms = 0i64;
    if c.eat_any(b"Tt ").is_some() {
        let h = c.digits(2)?;
        if !c.eat(b':') {
            return None;
        }
        let m = c.digits(2)?;
        let (mut s, mut frac) = (0, 0);
        if c.eat(b':') {
            s = c.digits(2)?;
            if c.eat(b'.') {
                let start = c.i;
                while c.peek().is_some_and(|d| d.is_ascii_digit()) {
                    c.i += 1;
                }
                if c.i == start {
                    return None;
                }
                // To the millisecond: the first three digits, padded.
                let digits = &c.s[start..c.i.min(start + 3)];
                frac = digits.iter().fold(0i64, |v, d| v * 10 + i64::from(d - b'0')) * 10i64.pow(3 - digits.len() as u32);
            }
        }
        if h > 24 || m > 59 || s > 59 || (h == 24 && (m, s, frac) != (0, 0, 0)) {
            return None;
        }
        ms = ((h * 60 + m) * 60 + s) * 1000 + frac;
        // The zone: Z, an offset, or none (UTC here; see the module's note).
        if c.eat_any(b"Zz").is_none() {
            if let Some(sign) = c.eat_any(b"+-") {
                let oh = c.digits(2)?;
                c.eat(b':');
                let om = c.digits(2)?;
                if oh > 23 || om > 59 {
                    return None;
                }
                let off = (oh * 60 + om) * 60_000;
                ms -= if sign == b'+' { off } else { -off };
            }
        }
    }
    if !c.done() {
        return None;
    }
    let t = days_from_civil(year, month as u32, day as u32) as f64 * 86_400_000.0 + ms as f64;
    (t.abs() <= MAX_MS).then_some(t)
}

/// A time as `Date.prototype.toISOString` writes it: 2026-10-05T12:00:00.000Z.
pub fn iso(ms: f64) -> String {
    let t = ms as i64;
    let (days, rest) = (t.div_euclid(86_400_000), t.rem_euclid(86_400_000));
    let (y, mo, d) = civil_from_days(days);
    let year = if (0..=9999).contains(&y) {
        format!("{y:04}")
    } else if y < 0 {
        format!("-{:06}", -y)
    } else {
        format!("+{y:06}")
    };
    format!("{year}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each as V8 (Node 22) reads it.
    #[test]
    fn reads_iso_times_as_date_parse_does() {
        let cases: &[(&str, Option<f64>)] = &[
            ("2026-10-03T12:00:00.000Z", Some(1791028800000.0)),
            ("2026-10-03T12:00Z", Some(1791028800000.0)),
            ("2026-10-03", Some(1790985600000.0)),
            ("2026-10", Some(1790812800000.0)),
            ("2026", Some(1767225600000.0)),
            ("2026-02-30", Some(1772409600000.0)),
            ("2024-02-29T00:00:00Z", Some(1709164800000.0)),
            ("2026-10-03T24:00:00Z", Some(1791072000000.0)),
            ("2026-10-03T24:00:01Z", None),
            ("2026-10-03T12:00:00.1Z", Some(1791028800100.0)),
            ("2026-10-03T12:00:00.1234567Z", Some(1791028800123.0)),
            ("2026-10-03T12:00:00+02:00", Some(1791021600000.0)),
            ("2026-10-03T12:00:00+0200", Some(1791021600000.0)),
            ("2026-10-03t12:00:00z", Some(1791028800000.0)),
            ("2026-10-03 12:00:00Z", Some(1791028800000.0)),
            ("+002026-10-03T00:00:00Z", Some(1790985600000.0)),
            ("-000001-01-01T00:00:00Z", Some(-62198755200000.0)),
            ("2026-13-01", None),
            ("2026-10-03T25:00Z", None),
            ("2026-10-03T12:60Z", None),
            ("2026-10-03T12:00:60Z", None),
            (" 2026-10-03T12:00:00Z", None),
            ("2026-10-03T12:00:00.Z", None),
            ("2026-10-03T12Z", None),
            ("not a date", None),
        ];
        for (text, want) in cases {
            assert_eq!(parse_iso(text), *want, "{text}");
        }
    }

    #[test]
    fn writes_iso_times_as_to_iso_string_does() {
        assert_eq!(iso(1791028800123.0), "2026-10-03T12:00:00.123Z");
        assert_eq!(iso(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(-62198755200000.0), "-000001-01-01T00:00:00.000Z");
        for d in [-800_000, -1, 0, 1, 20_000, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }
}
