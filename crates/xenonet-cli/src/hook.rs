//! xenonet hook <app>: what Claude Code's and Codex's hooks run (XenoNet
//! adds them when you connect your agents). It passes the event on to the
//! running XenoNet and prints what XenoNet wants the agent to hear, in the
//! hook output format both read. If XenoNet isn't running, or doesn't
//! answer within 4 seconds, it prints nothing and the agent carries on: it
//! always exits 0. XENONET_PORT (or --port) says which XenoNet.

use std::io::{Read, Write};
use std::time::Duration;
use xenonet_base::http;
use xenonet_base::json::Value;
use xenonet_base::text::js_number;

pub const TIMEOUT: Duration = Duration::from_secs(4);

/// The port a flag or XENONET_PORT names, as JS's `Number(x) ||` reads
/// them: the first that's a number other than 0, else 47840.
pub fn port(flag: Option<&str>, env: Option<&str>) -> Option<u16> {
    let n = [flag, env].into_iter().flatten().map(|s| js_number(Some(&Value::String(s.to_string())))).find(|n| *n != 0.0 && !n.is_nan()).unwrap_or(47840.0);
    // A port that isn't one makes no request at all, as fetch refuses its URL.
    (n.fract() == 0.0 && (1.0..=65535.0).contains(&n)).then_some(n as u16)
}

pub fn run(app: Option<&str>, port_flag: Option<&str>) -> i32 {
    let app = if app == Some("claude") { "claude" } else { "codex" };
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    // As text, the way the agent wrote it; nothing at all is an empty event.
    let text = String::from_utf8_lossy(&input).into_owned();
    let body = if text.is_empty() { "{}".to_string() } else { text };
    let env = std::env::var("XENONET_PORT").ok();
    if let Some(port) = port(port_flag, env.as_deref()) {
        if let Ok(http::Answer { status: 200, body: Ok(body), .. }) = http::post_json(port, &format!("/api/hook/{app}"), body.as_bytes(), TIMEOUT) {
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
