//! What live delivery needs on this machine, for xenonet doctor --live:
//! core/delivery/setup.js. Claude Code new enough for channels and the
//! command that starts a session listening, the settings that keep a
//! session listening after its turns, tmux for typing, and the OpenCode
//! servers XenoNet can find.

use super::opencode::Server;
use super::{number_text, Files, Result};
use serde_json::{json, Value};

/// Commands and the OpenCode servers, as the doctor's live check reads
/// them.
pub trait Host {
    /// A command's output, trimmed, or None when it isn't there or failed.
    fn exec(&mut self, cmd: &str, args: &[&str]) -> Option<String>;
    /// The OpenCode servers running now.
    fn servers(&mut self) -> Vec<Server>;
}

/// The first Claude Code with channels: its changelog adds --channels in
/// 2.1.80.
pub const CHANNELS_FROM: [f64; 3] = [2.0, 1.0, 80.0];
pub const LISTEN_SECONDS: f64 = 300.0;

/// Whether version text `v` is `min` or newer.
pub fn newer(v: Option<&str>, min: &[f64]) -> bool {
    let Some(m) = crate::re!(r"([0-9]+)\.([0-9]+)\.([0-9]+)").captures(v.unwrap_or("")) else { return false };
    for i in 0..3 {
        let p = xenonet_base::text::js_number(Some(&Value::String(m[i + 1].to_string())));
        let want = min.get(i).copied().unwrap_or(f64::NAN);
        if p != want {
            return p > want;
        }
    }
    true
}

/// The MCP server's name in Claude Code's settings, as connect gives it:
/// xenonet on the usual port, xenonet-dev on XenoNet Dev's, else
/// `xenonet-<port>`.
pub fn mcp_server_name(port: f64) -> String {
    if port == 0.0 || port.is_nan() || port == 47840.0 {
        "xenonet".into()
    } else if port == 47842.0 {
        "xenonet-dev".into()
    } else {
        format!("xenonet-{}", number_text(port))
    }
}

/// Listening after each turn: a Claude Code session started with these
/// settings has a Stop hook that waits for a message for up to `seconds`
/// once a turn ends, and hands it over.
pub fn listen_settings(port: f64, seconds: f64) -> Value {
    let url = format!("http://127.0.0.1:{}/api/hook/claude/listen?seconds={}", number_text(port), number_text(seconds));
    json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": url, "timeout": xenonet_base::json::number(seconds + 30.0) }] }] } })
}

/// The settings file for this XenoNet, written into `dir` when it's
/// missing or out of date. Its path.
pub fn write_listen_settings(files: &mut dyn Files, dir: &str, port: f64, seconds: f64) -> Result<String> {
    let file = xenonet_base::paths::join_all(&[dir, &format!("listen-{}.json", number_text(port))]);
    let text = format!("{}\n", xenonet_base::json::stringify(&listen_settings(port, seconds), 2));
    if files.read_file(&file).ok().as_deref() != Some(text.as_str()) {
        files.mkdir_all(dir)?;
        files.write_file(&file, &text, None)?;
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

/// What live delivery needs here: Claude Code's version and whether it has
/// channels, the commands to start a session with them and listening, tmux,
/// and the OpenCode servers found.
pub fn live_setup(h: &mut dyn Host, port: f64, listen_file: Option<&str>, home: &str) -> Value {
    let name = mcp_server_name(port);
    let claude = h.exec("claude", &["--version"]);
    let tmux = h.exec("tmux", &["-V"]);
    let servers = h.servers();
    let command = format!("claude --dangerously-load-development-channels server:{name}");
    let version = match &claude {
        None => Value::Null,
        Some(text) => Value::String(crate::re!(r"[0-9]+\.[0-9]+\.[0-9]+").find(text).map_or_else(|| text.clone(), |m| m.as_str().to_string())),
    };
    let listen = match listen_file.filter(|f| !f.is_empty()) {
        Some(f) => Value::String(format!("claude --settings {}", shell_path(f, home))),
        None => Value::Null,
    };
    let tmux = tmux.map_or(Value::Null, |t| Value::String(crate::re!(r"^tmux[\s\x{FEFF}--\x{85}]+").replace(&t, "").into_owned()));
    json!({
        "claude": { "version": version, "channels": newer(claude.as_deref(), &CHANNELS_FROM) },
        "command": command,
        "alias": format!("alias claude='{command}'"),
        "listen": listen,
        "opencodeAlias": "alias opencode='opencode --hostname 127.0.0.1'",
        "tmux": tmux,
        "opencode": servers.iter().map(|s| s.base.clone()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer(Some("2.1.302 (Claude Code)"), &CHANNELS_FROM));
        assert!(newer(Some("2.1.80"), &CHANNELS_FROM));
        assert!(!newer(Some("2.0.99"), &CHANNELS_FROM));
        assert!(!newer(None, &CHANNELS_FROM));
        assert_eq!(mcp_server_name(47843.0), "xenonet-47843");
        assert_eq!(mcp_server_name(0.0), "xenonet");
        assert_eq!(mcp_server_name(47842.0), "xenonet-dev");
    }

    /// A machine with Claude Code and tmux, and one OpenCode server.
    struct Machine;

    impl Host for Machine {
        fn exec(&mut self, cmd: &str, _: &[&str]) -> Option<String> {
            match cmd {
                "claude" => Some("2.1.90 (Claude Code)".into()),
                "tmux" => Some("tmux 3.4".into()),
                _ => None,
            }
        }
        fn servers(&mut self) -> Vec<Server> {
            vec![Server { pid: 10.0, base: "http://127.0.0.1:4096".into(), dir: None }]
        }
    }

    #[test]
    fn the_live_setup_says_what_is_here() {
        let v = live_setup(&mut Machine, 47842.0, Some("/home/me/.xenonet/listen-47842.json"), "/home/me");
        assert_eq!(v["claude"], json!({ "version": "2.1.90", "channels": true }));
        assert_eq!(v["command"], "claude --dangerously-load-development-channels server:xenonet-dev");
        assert_eq!(v["tmux"], "3.4");
        assert_eq!(v["opencode"], json!(["http://127.0.0.1:4096"]));
        if !cfg!(windows) {
            assert_eq!(v["listen"], "claude --settings \"$HOME/.xenonet/listen-47842.json\"");
        }
        assert_eq!(listen_settings(47840.0, 300.0)["hooks"]["Stop"][0]["hooks"][0]["timeout"], json!(330));
    }
}
