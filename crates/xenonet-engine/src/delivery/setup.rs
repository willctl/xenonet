//! What live delivery needs on this machine, for xenonet doctor --live:
//! core/delivery/setup.js. Claude Code new enough for channels and the
//! command that starts a session listening, the settings that keep a
//! session listening after its turns, tmux for typing, and the OpenCode
//! servers XenoNet can find.

use super::opencode::Server;
use super::{number_text, Files, Result};
use serde_json::{json, Map, Value};

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
        let p = xenonet_text::text::to_number(Some(&Value::String(m[i + 1].to_string())));
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
    json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": url, "timeout": xenonet_text::json::number(seconds + 30.0) }] }] } })
}

/// The settings file for this XenoNet, written into `dir` when it's
/// missing or out of date. Its path.
pub fn write_listen_settings(files: &mut dyn Files, dir: &str, port: f64, seconds: f64) -> Result<String> {
    let file = xenonet_os::paths::join_all(&[dir, &format!("listen-{}.json", number_text(port))]);
    let text = format!("{}\n", xenonet_text::json::stringify(&listen_settings(port, seconds), 2));
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

/// What live delivery needs here: the version of the app with channels
/// (Claude Code) and whether it has them, the commands to start a session
/// with them and listening, tmux, the alias that starts the app whose own
/// server takes messages (OpenCode), and its servers found. Each app's
/// part is keyed by its id, and its commands are its adapter's.
pub fn live_setup(h: &mut dyn Host, port: f64, listen_file: Option<&str>, home: &str) -> Value {
    let name = mcp_server_name(port);
    let ways = super::apps::apps();
    let channeled = ways.iter().find(|(_, w)| w.channel_command.is_some());
    let served = ways.iter().find(|(_, w)| w.server_alias.is_some());
    // The commands run in the order they always have: the version, tmux,
    // then the servers.
    let asked = channeled.and_then(|(_, w)| w.version).and_then(|(program, args)| h.exec(program, args));
    let tmux = h.exec("tmux", &["-V"]);
    let mut out = Map::new();
    if let Some((app, w)) = channeled {
        let version = match &asked {
            None => Value::Null,
            Some(text) => Value::String(crate::re!(r"[0-9]+\.[0-9]+\.[0-9]+").find(text).map_or_else(|| text.clone(), |m| m.as_str().to_string())),
        };
        let command = w.channel_command.map(|c| c(&name)).unwrap_or_default();
        let listen = match (listen_file.filter(|f| !f.is_empty()), w.listen_command) {
            (Some(f), Some(l)) => Value::String(l(&shell_path(f, home))),
            _ => Value::Null,
        };
        out.insert(app.id.clone(), json!({ "version": version, "channels": newer(asked.as_deref(), &CHANNELS_FROM) }));
        out.insert("command".into(), json!(command));
        if let Some((program, _)) = w.version {
            out.insert("alias".into(), json!(format!("alias {program}='{command}'")));
        }
        out.insert("listen".into(), listen);
    }
    let servers = h.servers();
    if let Some((app, w)) = served {
        out.insert(format!("{}Alias", app.id), json!(w.server_alias));
    }
    let tmux = tmux.map_or(Value::Null, |t| Value::String(crate::re!(r"^tmux[\s\x{FEFF}--\x{85}]+").replace(&t, "").into_owned()));
    out.insert("tmux".into(), tmux);
    if let Some((app, _)) = served {
        out.insert(app.id.clone(), json!(servers.iter().map(|s| s.base.clone()).collect::<Vec<_>>()));
    }
    Value::Object(out)
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
        xenonet_platforms::install();
        let v = live_setup(&mut Machine, 47842.0, Some("/home/me/.xenonet/listen-47842.json"), "/home/me");
        assert_eq!(v["claude"], json!({ "version": "2.1.90", "channels": true }));
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["claude", "command", "alias", "listen", "opencodeAlias", "tmux", "opencode"]);
        assert_eq!(
            (&v["alias"], &v["opencodeAlias"]),
            (&json!("alias claude='claude --dangerously-load-development-channels server:xenonet-dev'"), &json!("alias opencode='opencode --hostname 127.0.0.1'"))
        );
        assert_eq!(v["command"], "claude --dangerously-load-development-channels server:xenonet-dev");
        assert_eq!(v["tmux"], "3.4");
        assert_eq!(v["opencode"], json!(["http://127.0.0.1:4096"]));
        if !cfg!(windows) {
            assert_eq!(v["listen"], "claude --settings \"$HOME/.xenonet/listen-47842.json\"");
        }
        assert_eq!(listen_settings(47840.0, 300.0)["hooks"]["Stop"][0]["hooks"][0]["timeout"], json!(330));
    }
}
