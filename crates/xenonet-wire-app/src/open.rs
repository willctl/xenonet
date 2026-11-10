//! Opening things (core/open.js) and the xenonet command on your PATH
//! (core/command.js), xenonet-adapter's open.rs and xenonet-components' command.rs: the
//! URLs and the script are the Rust's, the programs they start run here.

use crate::fx::{self, platform};
use serde_json::{json, Value};
use xenonet_api::cores::{Command, CoreError, Opener, R};

pub struct Open;

// The fields of a node the actions read.
fn pick(n: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in ["kind", "observed", "reader", "cwd", "worktree", "app", "rawId", "desktopId", "path"] {
        if let Some(v) = n.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    Value::Object(out)
}

fn start((cmd, args): (String, Vec<String>)) {
    fx::start_detached(&cmd, &args);
}

impl Opener for Open {
    fn url_for(&self, node: &Value, action: &str) -> R<Option<String>> {
        Ok(xenonet_adapter::open::url_for(&pick(node), action).and_then(|v| v.as_str().map(str::to_string)))
    }

    fn handoff_url(&self, target: &str, folder: &Value, brief: &str) -> R<String> {
        let folder = xenonet_text::text::truthy(Some(folder)).then(|| xenonet_text::text::display(folder));
        xenonet_adapter::open::handoff_url(target, folder.as_deref(), brief).ok_or_else(|| CoreError::status(400, "bad handoff"))
    }

    fn launch(&self, target: &str) -> R<()> {
        start(xenonet_os::open::launch_command(target, platform()));
        Ok(())
    }

    fn reveal(&self, file: &str) -> R<()> {
        start(xenonet_os::open::reveal_command(file, platform()));
        Ok(())
    }
}

/// The xenonet command, for os.homedir()'s folder: your own home, which the
/// demo's world doesn't change.
pub struct XenonetCommand {
    home: String,
}

impl XenonetCommand {
    pub fn new() -> Self {
        XenonetCommand { home: xenonet_os::home_dir(&|k| std::env::var(k).ok()) }
    }

    fn dir(&self) -> String {
        xenonet_components::command::command_dir(&self.home, platform(), std::env::var("LOCALAPPDATA").ok().as_deref())
    }
}

impl Default for XenonetCommand {
    fn default() -> Self {
        Self::new()
    }
}

fn state_json(st: xenonet_components::command::State) -> Value {
    json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
}

impl Command for XenonetCommand {
    fn state(&self) -> R<Value> {
        Ok(state_json(xenonet_components::command::command_state(Some(&self.dir()), std::env::var("PATH").ok().as_deref(), platform())))
    }

    /// A script that runs the xenonet binary (fx::xenonet_bin: installed
    /// XenoNet's launcher, or this one). What the route gives, the JS's
    /// entry and runtime, has no part in it.
    fn install(&self, _entry: &str, _runtime: Option<&str>) -> R<Value> {
        let path = std::env::var("PATH").ok();
        xenonet_components::command::install_command(&self.dir(), &fx::xenonet_bin(&self.home), false, path.as_deref(), platform())
            .map(state_json)
            .map_err(|e| CoreError::new(e.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_for_what_the_node_offers() {
        // The apps' openers are their adapters'.
        xenonet_platforms::install();
        let o = Open;
        let node = json!({ "kind": "project", "path": "/w/atlas", "title": "never read" });
        assert_eq!(o.url_for(&node, "open-folder").unwrap().is_some(), xenonet_adapter::open::url_for(&pick(&node), "open-folder").is_some());
        assert_eq!(o.url_for(&node, "no-such-action").unwrap(), None);
        assert!(o.handoff_url("claude", &json!("/w/atlas"), "go on").unwrap().starts_with("claude://"));
    }

    #[cfg(unix)]
    #[test]
    fn the_command_goes_in_the_home_given() {
        let home = std::env::temp_dir().join(format!("xenonet-wire-command-{}", std::process::id()));
        let c = XenonetCommand { home: home.to_string_lossy().into_owned() };
        assert_eq!(c.state().unwrap()["installed"], false);
        let st = c.install("/x/cli/xenonet.js", Some("/x/node")).unwrap();
        assert_eq!((st["installed"].clone(), st["ours"].clone()), (json!(true), json!(true)));
        assert_eq!(st["runs"].as_array().map(Vec::len), Some(1), "the binary alone");
        assert!(st["file"].as_str().unwrap().starts_with(&*home.to_string_lossy()));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
