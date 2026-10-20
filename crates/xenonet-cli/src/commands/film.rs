//! xenonet film (cli/commands/film.js): a film of the day (src/film.js),
//! rendered in a hidden XenoNet Dev window and recorded to an MP4
//! (electron/recorder.js), through the control channel (core/control.js).
//! Needs a checkout of XenoNet and ffmpeg.

use crate::args::Flags;
use crate::cores;
use crate::instances::instance;
use crate::js::{have, on, t};
use crate::out::{c, json, now, print, Done, Fail};
use crate::reach::find;
use crate::sys;
use serde_json::{json, Map, Value};
use std::time::Duration;
use xenonet_base::units::{js_round, to_fixed};

pub const HELP: &str = include_str!("help/film.txt");

// A film's frames and length (core/frames.js): a story by default, 15 to
// 30 seconds with its cards, which take 8 of them.
const STORY: (u32, u32) = (1080, 1920);
const SECONDS: (f64, f64, f64) = (15.0, 30.0, 20.0);
const CARDS_MS: f64 = 8000.0;

/// filmSeconds(total): how long the film is, cards included.
pub fn film_seconds(total: Option<&Value>) -> f64 {
    let n = js_round(xenonet_base::text::js_number(total));
    let n = if n == 0.0 || n.is_nan() { SECONDS.2 } else { n };
    n.clamp(SECONDS.0, SECONDS.1)
}

/// The frame --size names, WIDTHxHEIGHT, or None when it isn't one.
pub fn frame(size: Option<&Value>) -> Option<(f64, f64)> {
    let text = size.map_or_else(|| format!("{}x{}", STORY.0, STORY.1), xenonet_base::text::js_string);
    let parts: Vec<f64> = text.split('x').map(|p| xenonet_base::text::js_number(Some(&Value::String(p.into())))).collect();
    let (w, h) = (parts.first().copied().unwrap_or(f64::NAN), parts.get(1).copied().unwrap_or(f64::NAN));
    (w >= 320.0 && h >= 240.0).then_some((w, h))
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let id = flags.text("instance").unwrap_or_else(|| "film".into());
    let total = film_seconds(flags.get("seconds"));
    let seconds = total - CARDS_MS / 1000.0;
    let reveal = json!({ "names": flags.on("names"), "tokens": flags.on("tokens") });
    let Some((w, h)) = frame(flags.get("size")) else { return Err(Fail::new("--size is WIDTHxHEIGHT, such as 1080x1920", 2)) };
    let name = instance(&id).map_or("undefined", |i| i.name);
    print(c::dim(format!("Starting {name}, hidden…")));
    cores::control_launch(&id, &json!({ "show": "hidden" }))?;

    // The look: the one asked for, or XenoNet Dev's.
    let mut dev_flags = Flags::default();
    dev_flags.set("dev", Value::Bool(true));
    let theme = match flags.get("theme") {
        Some(th) => Some(th.clone()),
        None => find(&dev_flags).and_then(|dev| dev.get("/api/theme").ok()).and_then(|th| th.get("id").cloned()),
    }
    .filter(|th| xenonet_base::text::truthy(Some(th)));
    if let Some(theme) = &theme {
        // Drawn in place, without a reload: the driver returns once it's drawn.
        let port = instance(&id).map_or(f64::NAN, |i| f64::from(i.port));
        let shown = xenonet_base::client::request(port, "XenoNet", "GET", "/api/theme", None, &[])?;
        if shown.get("id") != Some(theme) {
            cores::control_driver(&id, "theme", theme)?;
        }
    }

    let mut start = Map::new();
    start.insert("action".into(), json!("start"));
    start.insert("size".into(), json!([w, h]));
    start.insert("width".into(), json!(w));
    if let Some(out) = flags.get("out").filter(|o| xenonet_base::text::truthy(Some(o))) {
        start.insert("file".into(), out.clone());
    }
    let started = cores::control_record(&id, &Value::Object(start))?;
    print(c::dim(format!("Recording {} seconds…", xenonet_base::json::number_string(total))));
    let filmed = (|| -> Result<(), Fail> {
        cores::control_driver(&id, "film", &json!({ "seconds": seconds, "reveal": reveal }))?;
        // Until the end card has come and gone.
        let deadline = now() + (total + 40.0) * 1000.0;
        let mut seen = false;
        loop {
            let st = cores::control_driver(&id, "state", &Value::Null)?;
            if st.get("overlays").is_some_and(|o| on(o, "film")) {
                seen = true;
            } else if seen {
                return Ok(());
            }
            if now() > deadline {
                return Err(Fail::error("the film didn't finish in time"));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();
    let result = cores::control_record(&id, &json!({ "action": "stop" })).unwrap_or_else(|e| json!({ "error": e.message }));
    if !flags.on("keep") {
        let _ = cores::control_quit(&id);
    }
    filmed?;
    if on(&result, "error") {
        return Err(Fail::new(format!("the film didn't save: {}", t(&result, "error")), 1));
    }
    if flags.on("json") {
        let mut out = result.as_object().cloned().unwrap_or_default();
        out.insert("theme".into(), theme.unwrap_or(Value::Null));
        out.insert("instance".into(), json!(id));
        out.insert("reveal".into(), reveal);
        json(&Value::Object(out));
        return Ok(0);
    }
    let wide = if on(&started, "width") { format!(", {} wide", t(&started, "width")) } else { String::new() };
    let mb = to_fixed(have(&result, "bytes").map_or(f64::NAN, |b| xenonet_base::text::js_number(Some(b))) / 1e6, 1);
    print(format!("{} {} {}", c::green("✓"), sys::tilde(&t(&result, "file")), c::dim(format!("({} s, {mb} MB{wide})", t(&result, "seconds")))));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_frames() {
        assert_eq!(film_seconds(None), 20.0);
        assert_eq!(film_seconds(Some(&json!("100"))), 30.0);
        assert_eq!(film_seconds(Some(&json!("16.4"))), 16.0);
        assert_eq!(frame(None), Some((1080.0, 1920.0)));
        assert_eq!(frame(Some(&json!("1920x1080"))), Some((1920.0, 1080.0)));
        assert_eq!(frame(Some(&json!(true))), None);
    }
}
