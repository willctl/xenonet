//! Money, tokens, durations, shares and long text as people read them
//! (core/units.js), as the command prints them. The console page keeps the
//! JS module; this is the same text, rounded the way JS rounds:
//!
//! - `to_fixed` as Number.prototype.toFixed: the exact binary value, a tie
//!   going to the larger magnitude (0.25 → 0.3, 1.05 → 1.1);
//! - `locale` as toLocaleString("en-US"): the shortest decimal that reads
//!   back as the number, rounded half away from zero (1.005 → 1.01), with
//!   thousands grouped;
//! - `js_round` as Math.round: a tie goes up (-2.5 → -2).

use crate::json::{number_string, shortest};

/// Math.round.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() || x.fract() == 0.0 {
        return x;
    }
    let f = x.floor();
    if x - f >= 0.5 {
        f + 1.0
    } else {
        f
    }
}

// Digits and a point, rounded half up at `digits` places after the point.
fn round_decimal(int: &str, frac: &str, digits: usize, half_up: impl Fn(&str) -> bool) -> (String, String) {
    let mut frac: Vec<u8> = frac.bytes().collect();
    while frac.len() < digits {
        frac.push(b'0');
    }
    let rest: String = frac[digits..].iter().map(|&b| b as char).collect();
    frac.truncate(digits);
    let mut all: Vec<u8> = int.bytes().chain(frac.iter().copied()).collect();
    if half_up(&rest) {
        let mut i = all.len();
        loop {
            if i == 0 {
                all.insert(0, b'1');
                break;
            }
            i -= 1;
            if all[i] == b'9' {
                all[i] = b'0';
            } else {
                all[i] += 1;
                break;
            }
        }
    }
    let split = all.len() - digits;
    let int: String = all[..split].iter().map(|&b| b as char).collect();
    let frac: String = all[split..].iter().map(|&b| b as char).collect();
    (if int.is_empty() { "0".into() } else { int }, frac)
}

/// Number.prototype.toFixed(digits).
pub fn to_fixed(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.abs() >= 1e21 || x.is_infinite() {
        return number_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // The double's decimal expansion to 80 places: one that isn't a tie at
    // the few places asked for is further from it than 1e-80, so the
    // first digit past them says which way it goes.
    let exact = format!("{:.80}", x.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let (i, f) = round_decimal(int, frac, digits, |rest| rest.as_bytes().first().is_some_and(|d| *d >= b'5'));
    let body = if digits == 0 { i } else { format!("{i}.{f}") };
    // -0.0001 toFixed(2) is "-0.00", as JS writes it.
    format!("{sign}{body}")
}

fn group(int: &str) -> String {
    let b = int.as_bytes();
    let mut out = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c as char);
    }
    out
}

/// toLocaleString("en-US", { minimumFractionDigits, maximumFractionDigits }).
pub fn locale(x: f64, min_frac: usize, max_frac: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "∞".into() } else { "-∞".into() };
    }
    // -0 is "-0" in Intl, as is anything that rounds to it.
    let sign = if x.is_sign_negative() { "-" } else { "" };
    // The shortest digits, written out in full: 1.005, 1e21 → 1000…0.
    let (digits, exp) = shortest(x.abs());
    let point = i64::from(exp) + 1;
    let (int, frac) = if point <= 0 {
        ("0".to_string(), format!("{}{digits}", "0".repeat((-point) as usize)))
    } else if (point as usize) >= digits.len() {
        (format!("{digits}{}", "0".repeat(point as usize - digits.len())), String::new())
    } else {
        (digits[..point as usize].to_string(), digits[point as usize..].to_string())
    };
    let (i, mut f) = round_decimal(&int, &frac, max_frac, |rest| rest.as_bytes().first().is_some_and(|d| *d >= b'5'));
    while f.len() > min_frac && f.ends_with('0') {
        f.pop();
    }
    let body = if f.is_empty() { group(&i) } else { format!("{}.{f}", group(&i)) };
    format!("{sign}{body}")
}

/// Dollars: "$1,234.50", "<$0.01" for less than a cent, "no price" for none.
pub fn money(n: Option<f64>) -> String {
    match n {
        None => "no price".into(),
        Some(n) if n > 0.0 && n < 0.01 => "<$0.01".into(),
        Some(n) => format!("${}", locale(n, 2, 2)),
    }
}

/// Tokens: 950, 1.2k, 3M, 4.5B.
pub fn tokens(n: Option<f64>) -> String {
    let n = n.unwrap_or(0.0);
    let a = n.abs();
    let one = |v: f64, unit: &str| {
        let s = to_fixed(v, 1);
        format!("{}{unit}", s.strip_suffix(".0").unwrap_or(&s))
    };
    if a >= 1e9 {
        one(n / 1e9, "B")
    } else if a >= 1e6 {
        one(n / 1e6, "M")
    } else if a >= 1e3 {
        one(n / 1e3, "k")
    } else {
        number_string(js_round(n))
    }
}

/// A duration: "12 s", "5 min", "2 h 5 min", "—" for none.
pub fn duration(ms: Option<f64>) -> String {
    let Some(ms) = ms else { return "—".into() };
    let m = js_round(ms / 60000.0);
    if m < 1.0 {
        return format!("{} s", number_string(js_round(ms / 1000.0).max(1.0)));
    }
    if m < 60.0 {
        return format!("{} min", number_string(m));
    }
    let h = (m / 60.0).floor();
    let rest = m % 60.0;
    // NaN % 60 is falsy in JS, as 0 is.
    if rest != 0.0 && !rest.is_nan() {
        format!("{} h {} min", number_string(h), number_string(rest))
    } else {
        format!("{} h", number_string(h))
    }
}

/// "1 session", "1,234 sessions".
pub fn plural(n: f64, word: &str, many: Option<&str>) -> String {
    let many = many.map_or_else(|| format!("{word}s"), str::to_string);
    format!("{} {}", locale(n, 0, 3), if n == 1.0 { word } else { &many })
}

/// A share as a percentage: "37%", "<1%", nothing of nothing.
pub fn share(part: f64, whole: f64) -> String {
    if whole == 0.0 || whole.is_nan() {
        return String::new();
    }
    share_of(part / whole)
}

/// The share `p` (of 1) as a percentage.
pub fn share_of(p: f64) -> String {
    if p > 0.0 && p < 0.005 {
        "<1%".into()
    } else {
        format!("{}%", number_string(js_round(p * 100.0)))
    }
}

/// A long text cut at `n` UTF-16 units, saying how much more there was. A
/// character cut in two ends in U+FFFD, as JS's half of one prints.
pub fn excerpt(s: &str, n: usize) -> String {
    let len = s.encode_utf16().count();
    if len > n {
        let head = crate::log::js_slice(s, n);
        let half = if head.encode_utf16().count() < n { "\u{FFFD}" } else { "" };
        format!("{head}{half}\n… ({} more chars)", len - n)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each as Node 22 writes it.
    #[test]
    fn rounds_as_js_does() {
        for (x, f1, f2) in [
            (1.25, "1.3", "1.25"),
            (1.05, "1.1", "1.05"),
            (0.05, "0.1", "0.05"),
            (2.5, "2.5", "2.50"),
            (0.25, "0.3", "0.25"),
            (1234.5678, "1234.6", "1234.57"),
            (-1.25, "-1.3", "-1.25"),
            (999.95, "1000.0", "999.95"),
            (1e-7, "0.0", "0.00"),
        ] {
            assert_eq!((to_fixed(x, 1).as_str(), to_fixed(x, 2).as_str()), (f1, f2), "{x}");
        }
        for (x, l, l2) in
            [(1234.5678, "1,234.568", "1,234.57"), (1e-7, "0", "0.00"), (123456789.25, "123,456,789.25", "123,456,789.25"), (1.005, "1.005", "1.01"), (2.675, "2.675", "2.68")]
        {
            assert_eq!((locale(x, 0, 3).as_str(), locale(x, 2, 2).as_str()), (l, l2), "{x}");
        }
        assert_eq!((js_round(2.5), js_round(-2.5), js_round(0.49999999999999994)), (3.0, -2.0, 0.0));
    }

    #[test]
    fn units() {
        assert_eq!((money(Some(1234.5)).as_str(), money(Some(0.004)).as_str(), money(None).as_str()), ("$1,234.50", "<$0.01", "no price"));
        assert_eq!((tokens(Some(950.0)).as_str(), tokens(Some(1250.0)).as_str(), tokens(Some(3e6)).as_str()), ("950", "1.3k", "3M"));
        assert_eq!((duration(Some(12e3)).as_str(), duration(Some(125.0 * 60e3)).as_str(), duration(Some(120.0 * 60e3)).as_str()), ("12 s", "2 h 5 min", "2 h"));
        assert_eq!((plural(1.0, "session", None).as_str(), plural(1234.0, "session", None).as_str()), ("1 session", "1,234 sessions"));
        assert_eq!((share(1.0, 3.0).as_str(), share(1.0, 1000.0).as_str(), share(1.0, 0.0).as_str()), ("33%", "<1%", ""));
        assert_eq!((excerpt("abcdef", 4).as_str(), excerpt("a😀b", 2).as_str(), excerpt("ab", 4).as_str()), ("abcd\n… (2 more chars)", "a\u{FFFD}\n… (2 more chars)", "ab"));
    }
}
