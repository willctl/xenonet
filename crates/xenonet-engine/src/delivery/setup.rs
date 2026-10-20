//! What live delivery needs on this machine, for xenonet doctor --live:
//! core/delivery/setup.js. Claude Code new enough for channels and the
//! command that starts a session listening, the settings that keep a
//! session listening after its turns, tmux for typing, and the OpenCode
//! servers XenoNet can find. Commands and files are the twin's.

use crate::peer::{list, Calls, Result};
use crate::sql::Arg;
use crate::store::js_text;
use serde_json::{json, Value};

/// The first Claude Code with channels: its changelog adds --channels in
/// 2.1.80.
pub const CHANNELS_FROM: [f64; 3] = [2.0, 1.0, 80.0];
pub const LISTEN_SECONDS: f64 = 300.0;

/// Whether version text `v` is `min` or newer.
pub fn newer(v: &Arg, min: &[f64]) -> bool {
    let s = if v.is_nullish() { String::new() } else { js_text(v) };
    let Some(m) = crate::re!(r"(\d+)\.(\d+)\.(\d+)").captures(&s) else { return false };
    for i in 0..3 {
        let p = xenonet_base::text::js_number(Some(&Value::String(m[i + 1].to_string())));
        let want = min.get(i).copied().unwrap_or(f64::NAN);
        if p != want {
            return p > want;
        }
    }
    true
}

/// The MCP server's name in Claude Code's settings, as connect gives it.
pub fn server_name(port: &Arg) -> String {
    let n = match port {
        Arg::Num(n) => *n,
        Arg::Null | Arg::Undef => 0.0,
        other => xenonet_base::text::js_number(Some(&other.to_json())),
    };
    if !port.truthy() || n == 47840.0 {
        "xenonet".into()
    } else if n == 47842.0 {
        "xenonet-dev".into()
    } else {
        format!("xenonet-{}", js_text(port))
    }
}

/// `seconds + 30`, as JS adds.
fn plus_30(seconds: &Arg) -> Value {
    match seconds {
        Arg::Text(s) => Value::String(format!("{s}30")),
        Arg::Num(n) => xenonet_base::json::number(n + 30.0),
        other => xenonet_base::json::number(xenonet_base::text::js_number(Some(&other.to_json())) + 30.0),
    }
}

/// Listening after each turn: a Claude Code session started with these
/// settings has a Stop hook that waits for a message for up to `seconds`
/// once a turn ends, and hands it over.
pub fn listen_settings(port: &Arg, seconds: &Arg) -> Value {
    let url = format!("http://127.0.0.1:{}/api/hook/claude/listen?seconds={}", js_text(port), js_text(seconds));
    json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": url, "timeout": plus_30(seconds) }] }] } })
}

/// The settings file for this XenoNet, written into `dir` when it's
/// missing or out of date. Its path.
pub fn write_listen_settings(c: &mut dyn Calls, dir: &str, port: &Arg, seconds: &Arg) -> Result<String> {
    let file = xenonet_base::paths::join_all(&[dir, &format!("listen-{}.json", js_text(port))]);
    let text = format!("{}\n", xenonet_base::json::stringify(&listen_settings(port, seconds), 2));
    let old = c.call("fs", "readFileSync", vec![Arg::text(file.clone()), Arg::text("utf8")]).ok();
    if old.as_ref().and_then(Arg::as_str) != Some(text.as_str()) {
        c.call("fs", "mkdirSync", vec![Arg::text(dir), Arg::Obj(vec![("recursive".into(), Arg::Bool(true))])])?;
        c.call("fs", "writeFileSync", vec![Arg::text(file.clone()), Arg::text(text)])?;
    }
    Ok(file)
}

/// A path as a shell takes it: in double quotes, with your home folder as
/// $HOME (not on Windows, whose shells spell it otherwise).
fn shell_path(file: &str, home: &str) -> String {
    let sep = if cfg!(windows) { "\\" } else { "/" };
    if !home.is_empty() && !cfg!(windows) && file.starts_with(&format!("{home}{sep}")) {
        format!("\"$HOME{}\"", &file[home.len()..])
    } else {
        format!("\"{file}\"")
    }
}

/// What live delivery needs here. The twin's "exec" runs a command (its
/// output, or null), "servers" lists the OpenCode servers.
pub fn live_setup(c: &mut dyn Calls, port: &Arg, listen_file: &Arg, home: &Arg) -> Result<Value> {
    let name = server_name(port);
    let claude = c.call("", "exec", vec![Arg::text("claude"), Arg::List(vec![Arg::text("--version")])])?;
    let tmux = c.call("", "exec", vec![Arg::text("tmux"), Arg::List(vec![Arg::text("-V")])])?;
    let servers = c.call("", "servers", vec![]).map(list).unwrap_or_default();
    let command = format!("claude --dangerously-load-development-channels server:{name}");
    let version = if claude.is_nullish() {
        claude.to_json()
    } else {
        let text = js_text(&claude);
        match crate::re!(r"\d+\.\d+\.\d+").find(&text) {
            Some(m) => Value::String(m.as_str().to_string()),
            None => claude.to_json(),
        }
    };
    let home = if home.truthy() { js_text(home) } else { String::new() };
    let listen = if listen_file.truthy() { Value::String(format!("claude --settings {}", shell_path(&js_text(listen_file), &home))) } else { Value::Null };
    let tmux = if tmux.is_nullish() { Value::Null } else { Value::String(crate::re!(r"^tmux\s+").replace(&js_text(&tmux), "").into_owned()) };
    Ok(json!({
        "claude": { "version": version, "channels": newer(&claude, &CHANNELS_FROM) },
        "command": command,
        "alias": format!("alias claude='{command}'"),
        "listen": listen,
        "opencodeAlias": "alias opencode='opencode --hostname 127.0.0.1'",
        "tmux": tmux,
        "opencode": servers.iter().map(|s| s.field("base").to_json()).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer(&Arg::text("2.1.302 (Claude Code)"), &CHANNELS_FROM));
        assert!(newer(&Arg::text("2.1.80"), &CHANNELS_FROM));
        assert!(!newer(&Arg::text("2.0.99"), &CHANNELS_FROM));
        assert!(!newer(&Arg::Null, &CHANNELS_FROM));
        assert_eq!(server_name(&Arg::num(47843.0)), "xenonet-47843");
        assert_eq!(server_name(&Arg::Null), "xenonet");
    }
}
