//! The machine's clock as people read it: a time in its own time zone.

/// A time in ms as people read it in this machine's time zone: its
/// weekday, hour and minute, as in Sat 8:48 PM.
#[cfg_attr(unix, expect(unsafe_code, reason = "localtime_r(3), which std doesn't wrap"))]
pub fn local_time(ms: f64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return "Invalid Date".into();
    }
    #[cfg(unix)]
    {
        let t = (ms / 1000.0).floor() as libc::time_t;
        // SAFETY: tm is a plain C struct, for which all zeros is a valid
        // value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r only reads `t` and fills `tm`, both alive for
        // the call.
        if !unsafe { libc::localtime_r(&raw const t, &raw mut tm) }.is_null() {
            let h = match tm.tm_hour % 12 {
                0 => 12,
                h => h,
            };
            return format!("{} {h}:{:02} {}", DAYS[tm.tm_wday.clamp(0, 6) as usize], tm.tm_min, if tm.tm_hour < 12 { "AM" } else { "PM" });
        }
    }
    let secs = (ms / 1000.0).floor() as i64;
    let day = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{} {h}:{minute:02} {}", DAYS[((day + 4).rem_euclid(7)) as usize], if hour < 12 { "AM" } else { "PM" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weekday_and_a_time() {
        let t = local_time(0.0);
        assert!(t.ends_with("AM") || t.ends_with("PM"), "{t}");
        assert_eq!(local_time(f64::NAN), "Invalid Date");
    }
}
