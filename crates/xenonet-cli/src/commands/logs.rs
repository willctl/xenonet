//! xenonet logs (cli/commands/logs.js): what went wrong in XenoNet lately,
//! from its error log (core/log.js), read from the files themselves, so it
//! works with XenoNet stopped or failing to start.

use crate::args::Flags;
use crate::clock::month_day_time as when;
use crate::js::{num, on, t};
use crate::out::{c, now, print, Done, Fail};
use crate::sys;
use serde_json::Value;
use std::path::Path;
use xenonet_base::json::stringify;
use xenonet_base::log::{log_dir, read_logs, summarize, COMPONENTS, LEVELS};
use xenonet_base::text::{is_js_space, js_string, js_trim};

pub const HELP: &str = include_str!("help/logs.txt");

/// sinceOf(text, now): how far back --since says, as a time: 30m, 6h, 2d
/// (24h when it's not given).
pub fn since_of(text: Option<&Value>, now: f64) -> Result<f64, Fail> {
    let said = text.filter(|v| !v.is_null()).map_or_else(|| "24h".to_string(), js_string);
    let s = js_trim(&said);
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let unit = s[digits..].trim_start_matches(is_js_space);
    let ms = match unit {
        "m" => 60e3,
        "h" => 3600e3,
        "d" => 86400e3,
        _ => f64::NAN,
    };
    if digits == 0 || ms.is_nan() {
        return Err(Fail::error(format!("--since takes a time such as 30m, 6h or 2d, not {}", crate::js::text(text))));
    }
    Ok(now - s[..digits].parse::<f64>().unwrap_or(f64::NAN) * ms)
}

fn paint(level: &str, s: String) -> String {
    match level {
        "error" => c::red(s),
        "warn" => c::yellow(s),
        _ => c::dim(s),
    }
}

fn fix(e: &Value) -> String {
    if on(e, "fix") {
        format!("\n  {} {}", c::cyan("fix:"), t(e, "fix"))
    } else {
        String::new()
    }
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let dir = log_dir(&sys::state_dir(flags.on("dev")));
    let component = flags.text("component");
    if component.as_deref().is_some_and(|cm| !COMPONENTS.contains(&cm)) {
        return Err(Fail::error(format!("--component is {}", COMPONENTS.join(", "))));
    }
    let level = if flags.on("errors") {
        "error"
    } else if flags.on("all") {
        "info"
    } else {
        "warn"
    };
    let since = since_of(flags.get("since"), now())?;
    let entries = read_logs(Path::new(&dir), since, level, component.as_deref(), flags.text("code").as_deref(), 2000);
    let pad = |v: &Value| format!("{:<5}", t(v, "level"));
    if flags.on("summary") {
        let rows = summarize(&entries);
        if flags.on("json") {
            print(rows.iter().map(|r| stringify(r, 0)).collect::<Vec<_>>().join("\n"));
            return Ok(0);
        }
        if rows.is_empty() {
            print(c::dim(format!("Nothing at {level} or worse since {}.", when(since_of(flags.get("since"), now())?))));
            return Ok(0);
        }
        for r in &rows {
            let line = format!("{} · {}× · last {}", t(r, "component"), t(r, "count"), when(num(r, "last")));
            print(format!("{} {} {}\n  {}{}", paint(&t(r, "level"), pad(r)), c::bold(t(r, "code")), c::dim(line), t(r, "message"), fix(r)));
        }
        return Ok(0);
    }
    if flags.on("json") {
        print(entries.iter().map(|e| stringify(e, 0)).collect::<Vec<_>>().join("\n"));
        return Ok(0);
    }
    if entries.is_empty() {
        print(c::dim(format!("Nothing at {level} or worse since {}. ({})", when(since_of(flags.get("since"), now())?), LEVELS.join(", "))));
        return Ok(0);
    }
    for e in entries.iter().rev() {
        let repeats = if on(e, "repeats") { c::dim(format!(" (+{})", t(e, "repeats"))) } else { String::new() };
        print(format!(
            "{} {} {} {}{repeats}\n  {}{}",
            c::dim(when(num(e, "at"))),
            paint(&t(e, "level"), pad(e)),
            c::dim(t(e, "component")),
            c::bold(t(e, "code")),
            t(e, "message"),
            fix(e)
        ));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn how_far_back() {
        assert_eq!(since_of(None, 1e9).unwrap(), 1e9 - 24.0 * 3600e3);
        assert_eq!(since_of(Some(&json!(" 30 m ")), 1e9).unwrap(), 1e9 - 30.0 * 60e3);
        assert_eq!(since_of(Some(&json!("2d")), 1e9).unwrap(), 1e9 - 2.0 * 86400e3);
        assert_eq!(since_of(Some(&json!(true)), 1e9).unwrap_err().message, "--since takes a time such as 30m, 6h or 2d, not true");
        assert!(since_of(Some(&json!("6x")), 1e9).is_err());
    }
}
