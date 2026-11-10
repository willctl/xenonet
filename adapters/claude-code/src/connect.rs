//! Claude Code's connect: the xenonet MCP server at user scope, through
//! `claude mcp`, kept in .claude.json; HTTP hooks in settings.json; and
//! XenoNet's plugin for Claude Code, which carries both for the installed
//! XenoNet.
//!
//! The plugin and connect are kept to one: while the plugin is installed
//! and turned on, connecting leaves Claude Code to it and takes connect's
//! own copies out; and while connect's own hook for an event is in Claude
//! Code's settings, the service ignores the plugin's copy of it
//! (`hook_connected`), so an event never arrives twice.

use serde_json::{json, Map, Value};
use xenonet_adapter::apps::App;
use xenonet_adapter::connect::{cli_json, event_list, events_of, hook_set, hooks_of, join, json_text, text_of, Format, Found, Machine, Result, Shared, EARLIER};
use xenonet_os::paths::tilde;

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($p).expect("a pattern that compiles"))
    }};
}

/// XenoNet's plugin for Claude Code, which Claude Code knows as
/// `xenonet@<marketplace>` once it's installed.
const PLUGIN: &str = "xenonet";

/// What the preview says while connect's own entries are in Claude Code's
/// settings beside the plugin's.
const TWICE: &str = "xenonet connect's own entries are in Claude Code's settings beside the plugin's. XenoNet ignores the plugin's copy of any hook connect added, so each event still arrives once. Connecting takes connect's entries out.";

/// What the preview says when the plugin can't find the command it runs.
const NO_COMMAND: &str = "XenoNet's plugin runs the xenonet command, which isn't on your PATH";

/// The app's config format, for a connect.
pub fn format(app: &App, shared: &Shared) -> Box<dyn Format> {
    Box::new(ClaudeSettings::new(app, shared))
}

/// Claude Code's settings.
pub struct ClaudeSettings {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    settings_file: String,
    state_file: String,
    /// Claude Code's list of the plugins it has installed.
    plugins_file: String,
    /// Whether XenoNet's plugin, which reaches the installed XenoNet, is
    /// for this XenoNet.
    plugin_port: bool,
    hook_url: String,
    hooks: Value,
}

impl ClaudeSettings {
    pub fn new(app: &App, k: &Shared) -> Self {
        let o = k.options();
        let dir = o.claude_config_dir.clone().unwrap_or_else(|| join(&o.home, ".claude"));
        // Claude Code keeps user-scope MCP servers in .claude.json, in
        // CLAUDE_CONFIG_DIR when that's set.
        let state_file = match o.claude_config_dir.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => join(d, ".claude.json"),
            None => join(&o.home, ".claude.json"),
        };
        let hook_url = format!("http://127.0.0.1:{}/api/hook/{}", k.port(), app.id);
        let win = o.platform == "win32";
        let events = events_of(app);
        // Claude Code takes HTTP hooks, which cost nothing to run, except at
        // SessionStart, which takes only commands.
        let hooks = hook_set(&events, |event| {
            if event == "SessionStart" {
                let mut h = Map::new();
                h.insert("type".into(), json!("command"));
                h.insert("command".into(), json!(if win { k.windows_hook(&app.id) } else { k.hook_command(&app.id) }));
                if win {
                    h.insert("shell".into(), json!("powershell"));
                }
                h.insert("timeout".into(), json!(10));
                Value::Object(h)
            } else {
                json!({ "type": "http", "url": hook_url, "timeout": 5 })
            }
        });
        ClaudeSettings {
            id: app.id.clone(),
            name: app.name.clone(),
            events,
            settings_file: join(&dir, "settings.json"),
            state_file,
            plugins_file: join(&join(&dir, "plugins"), "installed_plugins.json"),
            plugin_port: o.instances.iter().any(|i| i.id == "installed" && i.port == o.port),
            hook_url,
            hooks,
        }
    }

    /// Any XenoNet's entry for this app: an HTTP hook posting to a
    /// XenoNet's hook route for it, or a command entry.
    fn is_xenonet(&self, h: &Value) -> bool {
        let http = h.get("type").and_then(Value::as_str) == Some("http") && {
            let url = text_of(h.get("url"));
            re!(r"^http://(127\.0\.0\.1|localhost):[0-9]+/api/hook/").find(&url).is_some_and(|m| url[m.end()..] == self.id)
        };
        http || Shared::xenonet_command(&self.id, h)
    }

    /// XenoNet's plugin for Claude Code, as Claude Code knows it
    /// (`xenonet@<marketplace>`), when it's installed and turned on in
    /// Claude Code's user settings. The plugin reaches the installed
    /// XenoNet, so only that XenoNet counts it. A list of plugins that
    /// can't be read counts as none.
    fn plugin(&self, k: &Shared, c: &mut dyn Machine, settings: &Value) -> Option<String> {
        if !self.plugin_port {
            return None;
        }
        let enabled = settings.get("enabledPlugins").and_then(Value::as_object)?;
        let installed = k.read_json(c, &self.plugins_file).ok()?;
        let installed = installed.get("plugins").and_then(Value::as_object)?;
        enabled
            .iter()
            .find(|(key, on)| {
                key.split_once('@').is_some_and(|(name, _)| name == PLUGIN)
                    && **on == Value::Bool(true)
                    && installed.get(*key).and_then(Value::as_array).is_some_and(|a| !a.is_empty())
            })
            .map(|(key, _)| key.clone())
    }
}

impl Format for ClaudeSettings {
    fn cli(&self) -> &'static str {
        "claude"
    }

    fn server(&self, k: &Shared, c: &mut dyn Machine, _cli: Option<&str>, name: &str) -> Result<Option<Value>> {
        let state = k.read_json(c, &self.state_file)?;
        Ok(state.get("mcpServers").filter(|m| !m.is_null()).and_then(|m| m.get(name)).cloned())
    }

    fn entry(&self, k: &Shared, c: &mut dyn Machine, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let port = k.port();
        let home = &k.options().home;
        let command = k.command();
        let mut changes = k.console_change(&self.id, found.console);
        let settings = k.read_json(c, &self.settings_file)?;
        let plugin = self.plugin(k, c, &settings);
        let mcp_what = format!("The {xenonet} MCP server, for every project");
        let mcp_detail = format!("{command} mcp (XENONET_PORT={port})");
        let hooks_what = format!("Hooks: {}", event_list(&self.events));
        let hooks_detail = format!("posting to {}; SessionStart runs {command} hook {}", self.hook_url, self.id);
        let own_hooks = k.has_own(settings.get("hooks"), &is);
        let mut twice = false;
        if let Some(key) = &plugin {
            // The plugin carries the server and the hooks; connect's own
            // copies beside it are to come out.
            let carried = format!("XenoNet's plugin for Claude Code ({key})");
            if k.server_state(found.server, None) == "missing" {
                changes.push(json!({ "id": "mcp", "what": mcp_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("xenonet mcp, from your PATH (XENONET_PORT={port})") }));
            } else {
                twice = true;
                changes.push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, beside the plugin's copy", "state": "remove", "detail": mcp_detail }));
            }
            if own_hooks {
                twice = true;
                changes.push(json!({ "id": "hooks", "what": hooks_what, "where": format!("{}, beside the plugin's copies", tilde(&self.settings_file, home)), "state": "remove", "detail": hooks_detail }));
            } else {
                changes.push(json!({ "id": "hooks", "what": hooks_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("posting to {}?via=plugin; SessionStart runs xenonet hook {}", self.hook_url, self.id) }));
            }
        } else {
            changes.push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, through claude mcp add-json --scope user", "state": k.server_state(found.server, None), "detail": mcp_detail }));
            let state = k.hooks_state(c, &settings, &is, &self.hooks)?;
            changes.push(json!({ "id": "hooks", "what": hooks_what, "where": tilde(&self.settings_file, home), "state": state, "detail": hooks_detail }));
        }
        // With the plugin, connecting adds nothing through the claude
        // command, so whether it's there doesn't matter.
        let mut notes: Vec<Value> = if found.cli.is_some() || plugin.is_some() {
            Vec::new()
        } else {
            vec![json!("XenoNet can't find the claude command, so it can add the hooks but not the MCP server. Install Claude Code's CLI, or add the server by hand.")]
        };
        if let Some(key) = &plugin {
            notes.push(json!(format!("Claude Code has XenoNet's plugin ({key}), which carries the {xenonet} MCP server and the hooks, so connecting leaves them to it.")));
        }
        if twice {
            notes.push(json!(TWICE));
        }
        if k.earlier(settings.get("hooks"), &is, found.server) {
            notes.push(json!(EARLIER));
        }
        let others = k.others(c, settings.get("hooks"), &is);
        notes.extend(k.besides(&others));
        let mut broken = k.broken(c, found.server, settings.get("hooks"), &is);
        if plugin.is_some() && c.cli("xenonet").is_none() {
            broken.push(json!(NO_COMMAND));
        }
        Ok(json!({ "app": self.id, "name": self.name, "cli": cli_json(found.cli), "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, console) = k.names();
        let settings = k.read_json(c, &self.settings_file)?;
        // With XenoNet's plugin there, connect's own copies of the server
        // and the hooks come out, and none go in.
        let plugin = self.plugin(k, c, &settings).is_some();
        if !plugin {
            let merged = k.merge(c, &settings, &is, Some(&self.hooks))?;
            Shared::write_json(c, &self.settings_file, &merged)?;
        } else if k.has_own(settings.get("hooks"), &is) {
            let merged = k.merge(c, &settings, &is, None)?;
            Shared::write_json(c, &self.settings_file, &merged)?;
        }
        if let Some(bin) = c.cli("claude") {
            let _ = c.run(&bin, &["mcp".into(), "remove".into(), xenonet.clone(), "--scope".into(), "user".into()]);
            if !plugin {
                c.run(&bin, &["mcp".into(), "add-json".into(), xenonet.clone(), json_text(k.mcp()), "--scope".into(), "user".into()])?;
            }
            if k.options().console_mcp {
                let mut console_mcp = k.mcp().clone();
                if let Some(Value::Array(a)) = console_mcp.get_mut("args") {
                    a.push(json!("console"));
                }
                let _ = c.run(&bin, &["mcp".into(), "remove".into(), console.clone(), "--scope".into(), "user".into()]);
                c.run(&bin, &["mcp".into(), "add-json".into(), console.clone(), json_text(&console_mcp), "--scope".into(), "user".into()])?;
            }
        }
        Ok(())
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        let settings = k.read_json(c, &self.settings_file)?;
        let merged = k.merge(c, &settings, &|h| self.is_xenonet(h), None)?;
        Shared::write_json(c, &self.settings_file, &merged)
    }

    fn remove_server(&self, _k: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) {
        if let Some(cli) = cli {
            let _ = c.run(cli, &["mcp".into(), "remove".into(), name.into(), "--scope".into(), "user".into()]);
        }
    }

    /// Whether this XenoNet's own Claude Code hook for `event`, as connect
    /// adds it, is in Claude Code's settings and can run (a command's files
    /// are all there). While it is, the copy XenoNet's plugin posts stands
    /// by, so the event reaches XenoNet once. Settings that can't be read
    /// hold no hook Claude Code runs.
    fn hook_connected(&self, k: &Shared, c: &mut dyn Machine, event: &str) -> Option<Result<bool>> {
        let Ok(settings) = k.read_json(c, &self.settings_file) else {
            return Some(Ok(false));
        };
        let groups = match settings.get("hooks").and_then(|h| h.get(event)) {
            Some(Value::Array(a)) => a.clone(),
            _ => return Some(Ok(false)),
        };
        for h in groups.iter().flat_map(hooks_of) {
            if !self.is_xenonet(&h) || !k.this_port(&h) {
                continue;
            }
            let one = json!({ "x": [{ "hooks": [h] }] });
            if Shared::hook_files(Some(&one), &mut |_| true).iter().all(|f| c.exists(f)) {
                return Some(Ok(true));
            }
        }
        Some(Ok(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::apps::app;
    use xenonet_adapter::connect::{Connect, FakeMachine, Instance, Options};

    fn options(port: f64) -> Options {
        Options {
            port,
            demo: false,
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            grok_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![Instance { id: "installed".into(), name: "XenoNet".into(), port: 47840.0 }, Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: 47842.0 }],
        }
    }

    /// A connect through Claude Code's settings alone.
    fn at(port: f64) -> Connect {
        Connect::with(options(port), std::slice::from_ref(app("claude").unwrap()), &|app, k| Some(format(app, k)))
    }

    /// Claude Code's settings format as connect makes it for that port.
    fn claude_at(port: f64) -> (Shared, ClaudeSettings) {
        let k = Shared::new(options(port));
        let f = ClaudeSettings::new(app("claude").unwrap(), &k);
        (k, f)
    }

    fn files(m: &[(String, String)]) -> FakeMachine {
        FakeMachine { files: m.iter().cloned().collect(), ..FakeMachine::default() }
    }

    #[test]
    fn the_plugin_counts_when_installed_and_turned_on_and_only_for_the_installed_xenonet() {
        let installed = r#"{ "version": 2, "plugins": { "xenonet@xenonet": [{ "scope": "user" }] } }"#;
        // The paths as this machine joins them.
        let (settings_file, list_file) = {
            let (_, f) = claude_at(47840.0);
            (f.settings_file, f.plugins_file)
        };
        let machine = |enabled: &str, list: Option<&str>| {
            let mut m = vec![(settings_file.clone(), format!(r#"{{ "enabledPlugins": {enabled} }}"#))];
            if let Some(l) = list {
                m.push((list_file.clone(), l.to_string()));
            }
            files(&m)
        };
        let plugin = |(k, c): (Shared, ClaudeSettings), mut m: FakeMachine| {
            let settings = k.read_json(&mut m, &c.settings_file).unwrap();
            c.plugin(&k, &mut m, &settings)
        };
        let on = r#"{ "xenonet@xenonet": true }"#;
        assert_eq!(plugin(claude_at(47840.0), machine(on, Some(installed))), Some("xenonet@xenonet".into()));
        assert_eq!(plugin(claude_at(47842.0), machine(on, Some(installed))), None, "XenoNet Dev's own connect");
        assert_eq!(plugin(claude_at(47840.0), machine(r#"{ "xenonet@xenonet": false }"#, Some(installed))), None, "turned off");
        assert_eq!(plugin(claude_at(47840.0), machine(on, None)), None, "not installed");
        assert_eq!(plugin(claude_at(47840.0), machine(on, Some("{ not json"))), None, "a list that can't be read");
        assert_eq!(plugin(claude_at(47840.0), machine(r#"{ "xenonet-tools@xenonet": true }"#, Some(installed))), None, "another plugin");
    }

    #[test]
    fn connects_own_hook_for_an_event_is_there_only_when_it_can_run() {
        let c = at(47840.0);
        let (_, cf) = claude_at(47840.0);
        let start = cf.hooks["SessionStart"].clone();
        let stop = cf.hooks["Stop"].clone();
        let settings = json!({ "hooks": { "SessionStart": [start], "Stop": [stop] } }).to_string();
        let mut m = files(&[(cf.settings_file.clone(), settings)]);
        assert!(c.hook_connected(&mut m, "Stop").unwrap());
        assert!(!c.hook_connected(&mut m, "SessionStart").unwrap(), "the binary it runs is gone");
        assert!(!c.hook_connected(&mut m, "Notification").unwrap());
        assert!(!at(47842.0).hook_connected(&mut m, "Stop").unwrap(), "another XenoNet's");
        m.files.insert("/x/xenonet".to_string(), String::new());
        assert!(c.hook_connected(&mut m, "SessionStart").unwrap());
        m.files.insert(cf.settings_file, "{ not json".into());
        assert!(!c.hook_connected(&mut m, "Stop").unwrap(), "settings that can't be read");
    }

    #[test]
    fn hooks_say_which_xenonet_theyre_for() {
        let c = at(47842.0);
        let (k, cf) = claude_at(47842.0);
        let claude = |h: &Value| cf.is_xenonet(h);
        assert_eq!(c.names(), &("xenonet-dev".to_string(), "xenonet-dev-console".to_string()));
        let start = &cf.hooks["SessionStart"]["hooks"][0];
        assert_eq!(start["command"], json!("XENONET_PORT=47842 '/x/xenonet' hook claude"));
        assert!(claude(start));
        assert_eq!(Shared::port_of(start), Some(47842.0));
        assert_eq!(cf.hooks["Stop"]["hooks"][0], json!({ "type": "http", "url": "http://127.0.0.1:47842/api/hook/claude", "timeout": 5 }));
        // An earlier XenoNet's hook, which ran Node and cli/xenonet.js, is
        // still a XenoNet's, and says so.
        let node = json!({ "type": "command", "command": "XENONET_PORT=47842 ELECTRON_RUN_AS_NODE=1 '/x/node' '/app/cli/xenonet.js' hook claude" });
        assert!(claude(&node));
        let hooks = json!({ "SessionStart": [{ "hooks": [node] }] });
        assert!(k.has_own(Some(&hooks), &claude) && k.earlier(Some(&hooks), &claude, None));
        assert!(!k.earlier(Some(&cf.hooks), &claude, Some(k.mcp())));
    }

    #[test]
    fn an_entry_is_one_apps_own_and_no_other_apps() {
        let (_, cf) = claude_at(47840.0);
        for (url, ours) in [
            ("http://127.0.0.1:47840/api/hook/claude", true),
            ("http://localhost:47842/api/hook/claude", true),
            ("http://127.0.0.1:47840/api/hook/codex", false),
            ("http://127.0.0.1:47840/api/hook/claude/listen", false),
            ("http://127.0.0.1:47840/api/hook/claudes", false),
            ("http://127.0.0.1:/api/hook/claude", false),
            ("https://127.0.0.1:47840/api/hook/claude", false),
            ("http://10.0.0.1:47840/api/hook/claude", false),
        ] {
            assert_eq!(cf.is_xenonet(&json!({ "type": "http", "url": url })), ours, "{url}");
        }
        assert!(cf.is_xenonet(&json!({ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook claude" })));
    }

    #[test]
    fn the_preview_then_connecting_then_disconnecting() {
        let c = at(47840.0);
        let (_, cf) = claude_at(47840.0);
        let mut m = FakeMachine { clis: vec!["claude".into()], ..FakeMachine::default() };
        let before = c.preview(&mut m).unwrap();
        let entry = &before["apps"][0];
        assert_eq!((entry["app"].as_str(), entry["cli"].as_str()), (Some("claude"), Some("/bin/claude")));
        assert_eq!(entry["changes"][0]["state"], json!("missing"));
        assert_eq!(entry["changes"][1]["state"], json!("missing"));
        assert_eq!(entry["notes"], json!([]));
        m.asked.clear();
        c.apply(&mut m, &["claude".to_string()]).unwrap();
        let written: Value = serde_json::from_str(&m.files[&cf.settings_file]).unwrap();
        assert_eq!(written["hooks"]["Stop"], json!([cf.hooks["Stop"]]));
        assert_eq!(m.asked[0], "cli claude");
        assert_eq!(m.asked[1], "/bin/claude mcp remove xenonet --scope user");
        assert!(
            m.asked[2].starts_with(
                "/bin/claude mcp add-json xenonet {\"type\":\"stdio\",\"command\":\"/x/xenonet\",\"args\":[\"mcp\"],\"env\":{\"XENONET_PORT\":\"47840\"}} --scope user"
            ),
            "{}",
            m.asked[2]
        );
        // The server is kept in .claude.json; with it there, the preview
        // says present.
        m.files.insert(
            cf.state_file.clone(),
            json!({ "mcpServers": { "xenonet": { "type": "stdio", "command": "/x/xenonet", "args": ["mcp"], "env": { "XENONET_PORT": "47840" } } } }).to_string(),
        );
        m.files.insert("/x/xenonet".into(), String::new());
        let after = c.preview(&mut m).unwrap();
        assert_eq!(after["apps"][0]["changes"][0]["state"], json!("present"));
        assert_eq!(after["apps"][0]["changes"][1]["state"], json!("present"));
        c.remove(&mut m).unwrap();
        let left: Value = serde_json::from_str(&m.files[&cf.settings_file]).unwrap();
        assert_eq!(left, json!({}));
        assert!(m.files.contains_key(format!("{}.before-xenonet", cf.settings_file)), "a copy of the file as it was");
    }
}
