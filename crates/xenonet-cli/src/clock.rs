//! Times as the JS command shows them with Date's toLocale…String, in this
//! machine's time zone. The JS runs in Node's default locale, which is
//! en-US where none is set; these are en-US's forms, the ones the command
//! prints:
//!
//! ```text
//!   toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })   9:04 AM
//!   toLocaleTimeString()                                             9:04:05 AM
//!   toLocaleString()                                                 10/7/2026, 9:04:05 AM
//!   { month: "short", day: "numeric", hour, minute, second "2-digit" }   Oct 7, 09:04:05 AM
//!   { year: "numeric", month: "short", day: "numeric", hour, minute "2-digit" }   Oct 7, 2026, 09:04 AM
//! ```
//!
//! A time that isn't one is "Invalid Date", as JS writes it.

/// A moment in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "localtime_r(3), which std doesn't wrap")]
fn breakdown(secs: i64) -> Option<Local> {
    let t = secs as libc::time_t;
    // SAFETY: tm is a plain C struct, for which all zeros is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r only reads `t` and fills `tm`, both alive for the
    // call.
    let got = unsafe { libc::localtime_r(&raw const t, &raw mut tm) };
    (!got.is_null()).then(|| Local {
        year: i64::from(tm.tm_year) + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    })
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "_localtime64_s, the C runtime's localtime, which std doesn't wrap")]
fn breakdown(secs: i64) -> Option<Local> {
    // The C runtime's struct tm, nine ints.
    #[repr(C)]
    #[derive(Default)]
    struct Tm([i32; 9]);
    extern "C" {
        fn _localtime64_s(tm: *mut Tm, time: *const i64) -> i32;
    }
    let mut tm = Tm::default();
    // SAFETY: _localtime64_s reads the time given and fills the tm given;
    // both are locals that outlive the call, and Tm is laid out as the C
    // runtime's struct tm, nine ints.
    let ok = unsafe { _localtime64_s(&raw mut tm, &raw const secs) } == 0;
    let [sec, min, hour, mday, mon, year, ..] = tm.0;
    ok.then(|| Local { year: i64::from(year) + 1900, month: (mon + 1) as u32, day: mday as u32, hour: hour as u32, minute: min as u32, second: sec as u32 })
}

/// A time in ms since 1970 in local time, or None when it isn't one.
pub fn local(ms: f64) -> Option<Local> {
    // Date's own range: ±8.64e15 ms.
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    breakdown((ms / 1000.0).floor() as i64)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl Local {
    fn hour12(&self) -> u32 {
        match self.hour % 12 {
            0 => 12,
            h => h,
        }
    }
    fn half(&self) -> &'static str {
        if self.hour < 12 {
            "AM"
        } else {
            "PM"
        }
    }
    fn year_text(&self) -> String {
        self.year.to_string()
    }
}

fn shown(ms: f64, f: impl Fn(&Local) -> String) -> String {
    local(ms).map_or_else(|| "Invalid Date".to_string(), |t| f(&t))
}

/// `toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })`: 9:04 AM.
pub fn hour_minute(ms: f64) -> String {
    shown(ms, |t| format!("{}:{:02} {}", t.hour12(), t.minute, t.half()))
}

/// `toLocaleTimeString()`: 9:04:05 AM.
pub fn time(ms: f64) -> String {
    shown(ms, |t| format!("{}:{:02}:{:02} {}", t.hour12(), t.minute, t.second, t.half()))
}

/// `toLocaleString()`: 10/7/2026, 9:04:05 AM.
pub fn date_time(ms: f64) -> String {
    shown(ms, |t| format!("{}/{}/{}, {}:{:02}:{:02} {}", t.month, t.day, t.year_text(), t.hour12(), t.minute, t.second, t.half()))
}

/// `{ month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
/// second: "2-digit" }`: Oct 7, 09:04:05 AM.
pub fn month_day_time(ms: f64) -> String {
    shown(ms, |t| format!("{} {}, {:02}:{:02}:{:02} {}", MONTHS[(t.month - 1) as usize], t.day, t.hour12(), t.minute, t.second, t.half()))
}

/// `{ year: "numeric", month: "short", day: "numeric", hour: "2-digit",
/// minute: "2-digit" }`: Oct 7, 2026, 09:04 AM.
pub fn date_hour_minute(ms: f64) -> String {
    shown(ms, |t| format!("{} {}, {}, {:02}:{:02} {}", MONTHS[(t.month - 1) as usize], t.day, t.year_text(), t.hour12(), t.minute, t.half()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The forms, from a moment's parts: the time zone is the machine's, so
    // these build the moment in it.
    fn at(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> f64 {
        // Near enough to find the local moment: step from UTC by the offset
        // localtime gives, which settles in two steps away from a change.
        let utc = xenonet_base::time::day_ms(year, month, day) + f64::from(hour * 3600 + minute * 60 + second) * 1000.0;
        let mut ms = utc;
        for _ in 0..3 {
            let l = local(ms).unwrap();
            let shown = xenonet_base::time::day_ms(l.year, l.month, l.day) + f64::from(l.hour * 3600 + l.minute * 60 + l.second) * 1000.0;
            ms += utc - shown;
        }
        ms
    }

    #[test]
    fn en_us_forms() {
        let morning = at(2026, 10, 7, 9, 4, 5);
        assert_eq!(local(morning), Some(Local { year: 2026, month: 10, day: 7, hour: 9, minute: 4, second: 5 }));
        assert_eq!(hour_minute(morning), "9:04 AM");
        assert_eq!(time(morning), "9:04:05 AM");
        assert_eq!(date_time(morning), "10/7/2026, 9:04:05 AM");
        assert_eq!(month_day_time(morning), "Oct 7, 09:04:05 AM");
        assert_eq!(date_hour_minute(morning), "Oct 7, 2026, 09:04 AM");
        let midnight = at(2026, 1, 2, 0, 4, 5);
        assert_eq!((hour_minute(midnight), month_day_time(midnight)), ("12:04 AM".to_string(), "Jan 2, 12:04:05 AM".to_string()));
        let noon = at(2026, 12, 31, 12, 30, 0);
        assert_eq!((time(noon), date_hour_minute(noon)), ("12:30:00 PM".to_string(), "Dec 31, 2026, 12:30 PM".to_string()));
        assert_eq!(hour_minute(f64::NAN), "Invalid Date");
    }
}
