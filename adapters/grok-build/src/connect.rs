//! Grok Build's connect: the xenonet MCP server as tables in config.toml,
//! which XenoNet writes itself (`grok mcp add` rewrites the file and drops
//! its comments; see `toml`), and command hooks in Claude Code's format in
//! a file of XenoNet's own, hooks/xenonet.json, which Grok runs without
//! asking, as it does every file in its hooks folder. GROK_HOME moves
//! both. Grok gives a session's MCP servers its id (GROK_SESSION_ID), so
//! the server knows whose it is.

pub mod toml;

use serde_json::{json, Map, Value};
use xenonet_adapter::apps::App;
use xenonet_adapter::connect::{cli_json, event_list, events_of, hook_set, join, list_strings, Format, Found, Machine, Problem, Result, Shared};
use xenonet_base::nodepath::NATIVE;
use xenonet_base::paths::tilde;

/// The app's config format, for a connect.
pub fn format(app: &App, shared: &Shared) -> Box<dyn Format> {
    Box::new(GrokHooks::new(app, shared))
}

/// Grok Build's config.toml and hooks file.
pub struct GrokHooks {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    grok_home: String,
    config_file: String,
    hooks_file: String,
    hooks: Value,
}

impl GrokHooks {
    pub fn new(app: &App, k: &Shared) -> Self {
        let o = k.options();
        let win = o.platform == "win32";
        let events = events_of(app);
        // Grok runs a hook's command with sh -c, and on Windows under the
        // shell it finds there, so the Windows form is one command line. Its
        // SessionEnd hooks get little time.
        let hooks = hook_set(&events, |event| {
            let command = if win { k.windows_hook(&app.id) } else { k.hook_command(&app.id) };
            json!({ "type": "command", "command": command, "timeout": if event == "SessionEnd" { 3 } else { 10 } })
        });
        let grok_home = o.grok_home.clone().unwrap_or_else(|| join(&o.home, ".grok"));
        GrokHooks {
            id: app.id.clone(),
            name: app.name.clone(),
            events,
            config_file: join(&grok_home, "config.toml"),
            hooks_file: join(&join(&grok_home, "hooks"), "xenonet.json"),
            grok_home,
            hooks,
        }
    }

    fn is_xenonet(&self, h: &Value) -> bool {
        Shared::xenonet_command(&self.id, h)
    }

    fn foreign(&self, k: &Shared, name: &str) -> String {
        format!(
            "{} names mcp_servers.{name} in a form XenoNet doesn't edit. Take it out, or write it as a [mcp_servers.{name}] table, and connect again.",
            tilde(&self.config_file, &k.options().home)
        )
    }

    // Writes config.toml as `edit` makes it: through a copy of the file (so
    // it keeps its mode), with a copy of it as it was the first time
    // XenoNet changes it.
    fn write_config(&self, k: &Shared, c: &mut dyn Machine, edit: &dyn Fn(&str) -> std::result::Result<String, String>) -> Result<()> {
        let before = k.read_text(c, &self.config_file)?;
        let after = edit(&before).map_err(Problem::new)?;
        if after == before {
            return Ok(());
        }
        c.make_dir(&NATIVE.dirname(&self.config_file))?;
        let tmp = format!("{}.tmp-xenonet", self.config_file);
        if c.exists(&self.config_file) {
            let first = format!("{}.before-xenonet", self.config_file);
            if !c.exists(&first) {
                c.copy(&self.config_file, &first)?;
            }
            c.copy(&self.config_file, &tmp)?;
        }
        c.write_text(&tmp, &after)?;
        c.rename(&tmp, &self.config_file)?;
        Ok(())
    }

    // XenoNet's hooks file: written as the settings files are, and taken
    // out once no XenoNet's hooks are left in it.
    fn write_hooks(&self, k: &Shared, c: &mut dyn Machine, add: Option<&Value>) -> Result<()> {
        let hooks = k.read_json(c, &self.hooks_file)?;
        let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), add)?;
        if add.is_none() && merged.as_object().is_some_and(Map::is_empty) {
            return c.remove(&self.hooks_file);
        }
        Shared::write_json(c, &self.hooks_file, &merged)
    }

    fn command_args(k: &Shared, extra: Option<&str>) -> (String, Vec<String>) {
        let mut args = list_strings(&k.mcp()["args"]);
        if let Some(e) = extra {
            args.push(e.into());
        }
        (k.options().bin.clone(), args)
    }
}

impl Format for GrokHooks {
    fn cli(&self) -> &'static str {
        "grok"
    }

    fn installed(&self, _k: &Shared, c: &mut dyn Machine) -> Result<bool> {
        Ok(c.cli("grok").is_some() || c.exists(&self.grok_home))
    }

    fn server(&self, k: &Shared, c: &mut dyn Machine, _cli: Option<&str>, name: &str) -> Result<Option<Value>> {
        Ok(toml::read_server(&k.read_text(c, &self.config_file)?, name))
    }

    fn entry(&self, k: &Shared, c: &mut dyn Machine, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let home = &k.options().home;
        let command = k.command();
        let text = k.read_text(c, &self.config_file)?;
        let hooks = k.read_json(c, &self.hooks_file)?;
        let installed = self.installed(k, c)?;
        let foreign = toml::foreign_server(&text, xenonet);
        let mut changes = k.console_change(&self.id, found.console);
        changes.push(json!({
            "id": "mcp",
            "what": format!("The {xenonet} MCP server, for every project"),
            "where": tilde(&self.config_file, home),
            "state": if foreign { "different" } else { k.server_state(found.server, None) },
            "detail": format!("{command} mcp (XENONET_PORT={}), as [mcp_servers.{xenonet}]; the rest of the file stays as it is", k.port()),
        }));
        let state = k.hooks_state(c, &hooks, &is, &self.hooks)?;
        changes.push(json!({ "id": "hooks", "what": format!("Hooks: {}", event_list(&self.events)), "where": tilde(&self.hooks_file, home), "state": state, "detail": format!("running {command} hook {}", self.id) }));
        let mut notes: Vec<Value> = if found.cli.is_some() {
            Vec::new()
        } else {
            vec![json!("XenoNet can't find the grok command. Connecting writes Grok Build's settings all the same, for when you install it.")]
        };
        if foreign {
            notes.push(json!(self.foreign(k, xenonet)));
        }
        notes.push(json!("Grok Build runs the hooks in its hooks folder without asking."));
        let others = k.others(c, hooks.get("hooks"), &is);
        notes.extend(k.besides(&others));
        let broken = k.broken(c, found.server, hooks.get("hooks"), &is);
        Ok(json!({ "app": self.id, "name": self.name, "cli": cli_json(found.cli), "installed": installed, "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        self.write_hooks(k, c, Some(&self.hooks))?;
        let (xenonet, console) = k.names().clone();
        let console_mcp = k.options().console_mcp;
        let env = k.server_env().to_vec();
        let (bin, args) = Self::command_args(k, None);
        let (_, console_args) = Self::command_args(k, Some("console"));
        self.write_config(k, c, &|text| {
            let out = toml::with_server(text, &xenonet, &bin, &args, &env)?;
            if console_mcp {
                toml::with_server(&out, &console, &bin, &console_args, &env)
            } else {
                Ok(out)
            }
        })
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
        if c.exists(&self.hooks_file) {
            self.write_hooks(k, c, None)?;
        }
        Ok(())
    }

    // Only this XenoNet's servers, by their names: another's stay.
    fn remove_server(&self, k: &Shared, c: &mut dyn Machine, _cli: Option<&str>, name: &str) {
        if c.exists(&self.config_file) {
            let _ = self.write_config(k, c, &|text| Ok(toml::without_server(text, name)));
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
    fn the_server_goes_in_config_toml_and_the_hooks_in_a_file_of_xenonets_own() {
        let c = Connect::with(options(), std::slice::from_ref(app("grok").unwrap()), &|app, k| Some(format(app, k)));
        let mut m = FakeMachine::default();
        // Grok Build isn't here: connecting every app leaves it out, but
        // the preview still says what connecting would do.
        assert_eq!(c.installed_ids(&mut m).unwrap(), Vec::<String>::new());
        let before = c.preview(&mut m).unwrap();
        let entry = &before["apps"][0];
        assert_eq!((entry["installed"].as_bool(), entry["cli"].as_str()), (Some(false), None));
        assert_eq!(entry["changes"][0]["where"], json!(xenonet_base::nodepath::join(&["~", ".grok", "config.toml"])));
        assert_eq!(entry["notes"][0], json!("XenoNet can't find the grok command. Connecting writes Grok Build's settings all the same, for when you install it."));

        // Its folder is here (the fake machine knows paths, folders too).
        m.files.insert("/h/.grok".into(), String::new());
        m.files.insert("/h/.grok/config.toml".into(), "# mine\n[cli]\ninstaller = \"internal\"\n".into());
        assert_eq!(c.installed_ids(&mut m).unwrap(), ["grok"]);
        c.apply(&mut m, &["grok".to_string()]).unwrap();
        let config = &m.files["/h/.grok/config.toml"];
        assert!(config.starts_with("# mine\n[cli]\ninstaller = \"internal\"\n\n[mcp_servers.xenonet]\n"), "{config}");
        assert!(m.files.contains_key("/h/.grok/config.toml.before-xenonet"));
        let hooks: Value = serde_json::from_str(&m.files["/h/.grok/hooks/xenonet.json"]).unwrap();
        assert_eq!(hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"], json!("XENONET_PORT=47840 '/x/xenonet' hook grok"));
        let after = c.preview(&mut m).unwrap();
        assert_eq!(after["apps"][0]["changes"][0]["state"], json!("present"));
        assert_eq!(after["apps"][0]["changes"][1]["state"], json!("present"));

        // Disconnecting takes the tables out and the hooks file with them.
        c.remove(&mut m).unwrap();
        assert_eq!(m.files["/h/.grok/config.toml"].trim_end(), "# mine\n[cli]\ninstaller = \"internal\"");
        assert!(!m.files.contains_key("/h/.grok/hooks/xenonet.json"));

        // A server written another way is left alone, and the preview
        // says so.
        m.files.insert("/h/.grok/config.toml".into(), "[mcp_servers]\nxenonet = { command = \"x\" }\n".into());
        let foreign = c.preview(&mut m).unwrap();
        assert_eq!(foreign["apps"][0]["changes"][0]["state"], json!("different"));
        assert_eq!(
            foreign["apps"][0]["notes"][1],
            json!("~/.grok/config.toml names mcp_servers.xenonet in a form XenoNet doesn't edit. Take it out, or write it as a [mcp_servers.xenonet] table, and connect again.")
        );
        assert!(c.apply(&mut m, &["grok".to_string()]).unwrap_err().message.contains("doesn't edit"));
    }
}
