//! `xenonet hook <app>` (cli/commands/hook.js): what Claude Code's and
//! Codex's hooks run (XenoNet adds them when you connect your agents). It
//! passes the event on to the running XenoNet and prints what XenoNet
//! wants the agent to hear, in the hook output format both read. If XenoNet isn't running, or doesn't
//! answer within 4 seconds, it prints nothing and the agent carries on: it
//! always exits 0. XENONET_PORT (or --port) says which XenoNet, and
//! --via=plugin marks the event as the copy XenoNet's plugin for Claude
//! Code sends, which XenoNet ignores while connect's own hook is there. An
//! app XenoNet doesn't tail is refused, quietly: the event goes nowhere, a
//! line in XenoNet's log says why, and the agent carries on.

use crate::args::Flags;
use crate::out::Done;
use crate::sys;
use std::io::{Read, Write};
use std::time::Duration;
use xenonet_base::http;
use xenonet_base::json::Value;
use xenonet_base::log::{log_dir, Log, MAX_BYTES};
use xenonet_base::text::js_number;

pub const HELP: &str = include_str!("help/hook.txt");

pub const TIMEOUT: Duration = Duration::from_secs(4);

/// The port a flag or XENONET_PORT names, as JS's `Number(x) ||` reads
/// them: the first that's a number other than 0, else 47840.
pub fn port(flag: Option<&str>, env: Option<&str>) -> Option<u16> {
    let n = [flag, env].into_iter().flatten().map(|s| js_number(Some(&Value::String(s.to_string())))).find(|n| *n != 0.0 && !n.is_nan()).unwrap_or(47840.0);
    // A port that isn't one makes no request at all, as fetch refuses its URL.
    (n.fract() == 0.0 && (1.0..=65535.0).contains(&n)).then_some(n as u16)
}

#[expect(clippy::unnecessary_wraps, reason = "every command's run has the dispatcher's signature, and a hook never fails")]
pub fn run(args: &[String], flags: &Flags) -> Done {
    // --port as JS's Number() reads the flag: on its own, it's 1.
    let port_flag = flags.get("port").map(|v| if *v == Value::Bool(true) { "1".to_string() } else { xenonet_base::text::js_string(v) });
    let plugin = flags.get("via").and_then(Value::as_str) == Some("plugin");
    Ok(hook(args.first().map(String::as_str), port_flag.as_deref(), plugin))
}

pub fn hook(given: Option<&str>, port_flag: Option<&str>, plugin: bool) -> i32 {
    // Codex when no app is named; another app is refused.
    let app = xenonet_base::apps::tailed_or_text(given, Some("codex"));
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let Some(app) = app else {
        let why = xenonet_base::apps::app_refusal("xenonet hook", &Value::String(given.unwrap_or_default().to_string()));
        Log::new(Some(&log_dir(&sys::state_dir(false))), "cli", None, None, &sys::home(), MAX_BYTES).write("warn", "cli.hook", Some(&why), None, None, None, crate::out::now());
        return 0;
    };
    // As text, the way the agent wrote it; nothing at all is an empty event.
    let text = String::from_utf8_lossy(&input).into_owned();
    let body = if text.is_empty() { "{}".to_string() } else { text };
    let env = std::env::var("XENONET_PORT").ok();
    if let Some(port) = port(port_flag, env.as_deref()) {
        let path = format!("/api/hook/{app}{}", if plugin { "?via=plugin" } else { "" });
        if let Ok(http::Answer { status: 200, body: Ok(body), .. }) = http::post_json(port, &path, body.as_bytes(), TIMEOUT) {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&body);
            let _ = out.flush();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::port;

    #[test]
    fn ports_as_js_reads_them() {
        assert_eq!(port(Some("47842"), Some("47841")), Some(47842));
        assert_eq!(port(Some("x"), Some("47841")), Some(47841));
        assert_eq!(port(Some("0"), None), Some(47840));
        assert_eq!(port(None, None), Some(47840));
        assert_eq!(port(Some("70000"), None), None);
    }
}
