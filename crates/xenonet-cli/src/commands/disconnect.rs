//! xenonet disconnect (cli/commands/disconnect.js): takes XenoNet back out
//! of the settings of the apps connect goes through (Claude Code, Codex and
//! Grok Build; core/engine/connect.js), leaving everything else as it was.
//! With no XenoNet running on the port it names, it takes that XenoNet's
//! entries out by itself, so uninstalling XenoNet leaves no hook behind
//! that runs files about to be deleted.

use super::connect::{connected, local_options, render};
use super::words;
use crate::args::Flags;
use crate::cores;
use crate::js::{list, str, t};
use crate::out::{c, print, Done};
use crate::prompt::confirm;
use crate::reach::{candidates, find};
use serde_json::{json, Map, Value};
use xenonet_base::client::name_of;

pub const HELP: &str = include_str!("help/disconnect.txt");

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let s = find(flags);
    // Nothing answers: this XenoNet's entries, by its port, taken out here.
    let opts = local_options(candidates(flags)[0]);
    let preview = match &s {
        Some(s) => s.get("/api/engine/connect")?,
        None => cores::connect_preview(&opts)?,
    };
    // What XenoNet's plugin for Claude Code carries is the plugin's, not
    // connect's, so it stays.
    let ours = |ch: &&Value| str(ch, "state") != Some("missing") && str(ch, "via") != Some("plugin");
    let plugin = list(&preview, "apps").iter().any(|a| list(a, "changes").iter().any(|ch| str(ch, "via") == Some("plugin")));
    let there: Vec<&Value> = list(&preview, "apps").iter().filter(|a| list(a, "changes").iter().any(|ch| ours(&ch))).collect();
    if there.is_empty() {
        let settings: Vec<String> = words::names("connect").iter().map(|n| format!("{n}'s")).collect();
        print(c::dim(format!("XenoNet isn't in {} settings; nothing to take out.", words::listed(&settings, "or"))));
        if plugin {
            print(c::dim(PLUGIN_STAYS));
        }
        return Ok(0);
    }
    let shown: Vec<Value> = there
        .iter()
        .map(|a| {
            let mut a = a.as_object().cloned().unwrap_or_default();
            a.insert("notes".into(), json!([]));
            a.insert("broken".into(), json!([]));
            let changes: Vec<Value> = list(&Value::Object(a.clone()), "changes")
                .iter()
                .filter(|ch| ours(ch))
                .map(|ch| {
                    let mut ch: Map<String, Value> = ch.as_object().cloned().unwrap_or_default();
                    ch.insert("state".into(), json!("remove"));
                    Value::Object(ch)
                })
                .collect();
            a.insert("changes".into(), Value::Array(changes));
            Value::Object(a)
        })
        .collect();
    render(&json!({ "apps": shown }), &connected());
    let names = words::listed(&there.iter().map(|a| t(a, "name")).collect::<Vec<_>>(), "and");
    if !confirm(&format!("Take XenoNet out of {names}?"), flags)? {
        print(c::dim("Nothing changed."));
        return Ok(0);
    }
    match &s {
        Some(s) => {
            s.del("/api/engine/connect")?;
        }
        None => {
            cores::connect_remove(&opts)?;
        }
    }
    let keeps = s.as_ref().map_or(String::new(), |s| format!(" ({} keeps running)", name_of(s)));
    print(format!("{} Took XenoNet out of {names}{keeps}. Sessions already open keep their tools until restarted.", c::green("✓")));
    if plugin {
        print(c::dim(PLUGIN_STAYS));
    }
    Ok(0)
}

const PLUGIN_STAYS: &str = "XenoNet's plugin for Claude Code stays. /plugin in Claude Code uninstalls it.";
