//! Codex's connect: the xenonet MCP server through `codex mcp`, and
//! command hooks in hooks.json, which Codex asks you to trust once.

use serde_json::{json, Map, Value};
use xenonet_adapter::apps::App;
use xenonet_adapter::connect::{cli_json, event_list, events_of, hook_set, join, list_strings, Format, Found, Machine, Result, Shared, EARLIER};
use xenonet_base::paths::tilde;

/// The app's config format, for a connect.
pub fn format(app: &App, shared: &Shared) -> Box<dyn Format> {
    Box::new(CodexHooks::new(app, shared))
}

/// Codex's hooks file and MCP settings.
pub struct CodexHooks {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    hooks_file: String,
    hooks: Value,
}

impl CodexHooks {
    pub fn new(app: &App, k: &Shared) -> Self {
        let o = k.options();
        let win = o.platform == "win32";
        let events = events_of(app);
        // Codex allows SessionEnd hooks 3 seconds at most.
        let hooks = hook_set(&events, |event| {
            let mut h = Map::new();
            h.insert("type".into(), json!("command"));
            h.insert("command".into(), json!(k.hook_command(&app.id)));
            if win {
                h.insert("commandWindows".into(), json!(k.windows_hook(&app.id)));
            }
            h.insert("timeout".into(), json!(if event == "SessionEnd" { 3 } else { 10 }));
            Value::Object(h)
        });
        CodexHooks { id: app.id.clone(), name: app.name.clone(), events, hooks_file: join(&o.codex_home.clone().unwrap_or_else(|| join(&o.home, ".codex")), "hooks.json"), hooks }
    }

    fn is_xenonet(&self, h: &Value) -> bool {
        Shared::xenonet_command(&self.id, h)
    }
}

impl Format for CodexHooks {
    fn cli(&self) -> &'static str {
        "codex"
    }

    fn server(&self, _k: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) -> Result<Option<Value>> {
        let Some(cli) = cli else { return Ok(None) };
        let r = c.run(cli, &["mcp".into(), "get".into(), name.into(), "--json".into()]).unwrap_or_default();
        if r.is_empty() {
            return Ok(None);
        }
        Ok(Some(match xenonet_base::json::parse(&r) {
            Ok(Value::Null) | Err(_) => json!({ "command": null }),
            Ok(s) => match s.get("transport").filter(|t| !t.is_null()) {
                Some(t) => t.clone(),
                None => s,
            },
        }))
    }

    fn entry(&self, k: &Shared, c: &mut dyn Machine, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let home = &k.options().home;
        let command = k.command();
        let mut changes = k.console_change(&self.id, found.console);
        changes.push(json!({ "id": "mcp", "what": format!("The {xenonet} MCP server, for every project"), "where": "Codex's config, through codex mcp add", "state": k.server_state(found.server, None), "detail": format!("{command} mcp (XENONET_PORT={})", k.port()) }));
        let hooks_file = k.read_json(c, &self.hooks_file)?;
        let state = k.hooks_state(c, &hooks_file, &is, &self.hooks)?;
        changes.push(json!({ "id": "hooks", "what": format!("Hooks: {}", event_list(&self.events)), "where": tilde(&self.hooks_file, home), "state": state, "detail": format!("running {command} hook {}", self.id) }));
        let mut notes: Vec<Value> =
            if found.cli.is_some() { Vec::new() } else { vec![json!("XenoNet can't find the codex command, so it can add the hooks but not the MCP server.")] };
        notes.push(json!("Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's."));
        if k.earlier(hooks_file.get("hooks"), &is, found.server) {
            notes.push(json!(EARLIER));
        }
        let others = k.others(c, hooks_file.get("hooks"), &is);
        notes.extend(k.besides(&others));
        let broken = k.broken(c, found.server, hooks_file.get("hooks"), &is);
        Ok(json!({ "app": self.id, "name": self.name, "cli": cli_json(found.cli), "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        let (xenonet, console) = k.names();
        let hooks = k.read_json(c, &self.hooks_file)?;
        let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), Some(&self.hooks))?;
        Shared::write_json(c, &self.hooks_file, &merged)?;
        if let Some(bin) = c.cli("codex") {
            let env: Vec<String> = k.server_env().iter().flat_map(|(key, v)| ["--env".to_string(), format!("{key}={v}")]).collect();
            let tail = |extra: Option<&str>| {
                let mut t = vec!["--".to_string(), k.options().bin.clone()];
                t.extend(list_strings(&k.mcp()["args"]));
                if let Some(e) = extra {
                    t.push(e.into());
                }
                t
            };
            let _ = c.run(&bin, &["mcp".into(), "remove".into(), xenonet.clone()]);
            let mut args = vec!["mcp".to_string(), "add".into(), xenonet.clone()];
            args.extend(env.clone());
            args.extend(tail(None));
            c.run(&bin, &args)?;
            if k.options().console_mcp {
                let _ = c.run(&bin, &["mcp".into(), "remove".into(), console.clone()]);
                let mut args = vec!["mcp".to_string(), "add".into(), console.clone()];
                args.extend(env);
                args.extend(tail(Some("console")));
                c.run(&bin, &args)?;
            }
        }
        Ok(())
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        if c.exists(&self.hooks_file) {
            let hooks = k.read_json(c, &self.hooks_file)?;
            let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), None)?;
            Shared::write_json(c, &self.hooks_file, &merged)?;
        }
        Ok(())
    }

    fn remove_server(&self, _k: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) {
        if let Some(cli) = cli {
            let _ = c.run(cli, &["mcp".into(), "remove".into(), name.into()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::apps::app;
    use xenonet_adapter::connect::{Connect, FakeMachine, Instance, Options};

    fn options() -> Options {
        Options {
            port: 47840.0,
            demo: false,
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            grok_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![Instance { id: "installed".into(), name: "XenoNet".into(), port: 47840.0 }],
        }
    }

    #[test]
    fn hooks_go_in_hooks_json_and_the_server_through_codex_mcp() {
        let c = Connect::with(options(), std::slice::from_ref(app("codex").unwrap()), &|app, k| Some(format(app, k)));
        let k = Shared::new(options());
        let f = CodexHooks::new(app("codex").unwrap(), &k);
        assert_eq!(f.hooks_file, "/h/.codex/hooks.json");
        assert_eq!(f.hooks["SessionEnd"]["hooks"][0]["timeout"], json!(3));
        let mut m = FakeMachine { clis: vec!["codex".into()], ..FakeMachine::default() };
        let before = c.preview(&mut m).unwrap();
        assert_eq!(before["apps"][0]["changes"][1]["state"], json!("missing"));
        assert_eq!(before["apps"][0]["notes"], json!(["Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's."]));
        assert_eq!(m.asked, ["cli codex", "/bin/codex mcp get xenonet --json"]);
        m.asked.clear();
        c.apply(&mut m, &["codex".to_string()]).unwrap();
        assert_eq!(m.asked[1], "/bin/codex mcp remove xenonet");
        assert_eq!(m.asked[2], "/bin/codex mcp add xenonet --env XENONET_PORT=47840 -- /x/xenonet mcp");
        let written: Value = serde_json::from_str(&m.files["/h/.codex/hooks.json"]).unwrap();
        assert_eq!(written["hooks"]["SessionStart"][0]["hooks"][0]["command"], json!("XENONET_PORT=47840 '/x/xenonet' hook codex"));
        assert_eq!(c.preview(&mut m).unwrap()["apps"][0]["changes"][1]["state"], json!("present"));
        c.remove(&mut m).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&m.files["/h/.codex/hooks.json"]).unwrap(), json!({}));
        assert!(m.asked.iter().any(|a| a == "/bin/codex mcp remove xenonet"));
    }
}
