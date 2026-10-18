//! Muse's observer (core/actors/muse.js; docs/design/actors/muse.md):
//! presence, nothing more. Muse is Meta's personal agent, and it works in a
//! cloud VM of its own whether or not its Mac app is open. So:
//!
//!   idle      its app runs
//!   unknown   its app isn't running: Muse goes on in the cloud with it
//!             closed, and nothing here can say it stopped, so it's never
//!             offline, and it stays listed for as long as it's installed
//!
//! Never working, since XenoNet can't see its work, and no events or
//! targets beyond presence. The app is Meta's only when its bundle id says
//! so (com.meta.endo). Of its files, XenoNet reads only whether the app is
//! there, and two keys of its Info.plist.
//!
//! The look calls the observer's own fields and methods ("actors", "app",
//! "procs", keep, say, and __census, the census with what each process is).

use crate::attribution::Census;
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_base::sync::Locked;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;

pub const ID: &str = "actor:muse";
const MAIN: &str = "/Muse.app/Contents/MacOS/Muse";
const STRAY_MS: f64 = 60e3;
// What Muse can do, as Meta's docs have it.
const CAPABILITIES: [&str; 4] = ["browse", "message-people", "schedule", "use-computer"];

#[derive(Default)]
struct State {
    looked_at: Option<f64>,
    was_installed: bool,
    table_status: Option<String>,
}

#[derive(Default)]
pub struct Muse {
    state: Mutex<State>,
}

/// Whether the actor is known, and not the demo's.
pub fn known(c: &mut dyn Calls, id: &str) -> Result<bool> {
    if !c.call("actors", "has", vec![Arg::text(id)])?.truthy() {
        return Ok(false);
    }
    let info = c.call("actors", "info", vec![Arg::text(id)])?;
    Ok(!xenonet_engine::peer::list(info.field("via")).iter().any(|v| v.as_str() == Some("demo")))
}

/// The census, read for an observer: the table and what each process is,
/// and whether procs could look ("procs.supported").
pub fn census(c: &mut dyn Calls, table_state: &mut Option<String>) -> Result<Census> {
    match c.call("", "__census", vec![]) {
        Ok(a) => {
            *table_state = Some(if c.get("procs.supported")? == Arg::Bool(false) { "unsupported" } else { "ok" }.into());
            Ok(Census::of(&a))
        }
        Err(e) => {
            *table_state = Some("unreadable".into());
            Err(e)
        }
    }
}

impl Muse {
    pub fn look(&self, c: &mut dyn Calls, at: f64) -> Result<()> {
        let known = known(c, ID)?;
        // Whether the app where apps go is Meta's, from its Info.plist.
        c.call("app", "read", vec![])?;
        // Not installed where apps go and not seen before: the process table
        // is read only once a minute, for an app run from somewhere else.
        if !known && !c.call("app", "installed", vec![])?.truthy() {
            let mut s = self.state.locked();
            if at - s.looked_at.unwrap_or(f64::NEG_INFINITY) < STRAY_MS {
                return Ok(());
            }
            s.looked_at = Some(at);
        }
        let mut table_state = None;
        let census = census(c, &mut table_state);
        self.state.locked().table_status = table_state;
        let census = census?;
        let open = census.table.iter().enumerate().any(|(i, p)| {
            let pr = &census.products[i];
            pr.get("product").and_then(Value::as_str) == Some("muse")
                && pr.get("role").and_then(Value::as_str) == Some("app")
                && xenonet_engine::js::template(p.row.get("program").filter(|v| !v.is_null()).or(Some(&json!("")))).ends_with(MAIN)
        });
        let installed = open || c.call("app", "installed", vec![])?.truthy();
        self.state.locked().was_installed = installed;
        if !installed && !known {
            return Ok(());
        }
        let version = if c.call("app", "ours", vec![])?.truthy() { c.get("app.plist")?.field("version") } else { Arg::Null };
        let version = if version.is_nullish() { Arg::Null } else { version };
        let keep = Arg::Obj(vec![
            ("name".into(), Arg::text("Muse")),
            ("version".into(), version),
            ("via".into(), Arg::List(vec![Arg::text("observed")])),
            ("heartbeatMs".into(), Arg::Null),
            ("capabilities".into(), Arg::List(CAPABILITIES.iter().map(|c| Arg::text(*c)).collect())),
            ("installed".into(), Arg::Bool(installed)),
            ("at".into(), Arg::Num(at)),
        ]);
        c.call("", "keep", vec![Arg::text(ID), keep])?;
        if !c.call("actors", "enabled", vec![Arg::text(ID)])?.truthy() {
            return Ok(());
        }
        let (presence, status) =
            if open { ("idle", "Its app is open on this Mac; it works in Meta's cloud") } else { ("unknown", "Its app on this Mac is closed; it may still work in Meta's cloud") };
        c.call("", "say", vec![Arg::text(ID), Arg::Obj(vec![("presence".into(), Arg::text(presence)), ("status".into(), Arg::text(status)), ("at".into(), Arg::Num(at))])])?;
        Ok(())
    }

    /// What the doctor shows: whether the app is installed, and what this
    /// observer reads.
    pub fn status(&self, c: &mut dyn Calls, home: &str) -> Result<Value> {
        let (was, table) = {
            let s = self.state.locked();
            (s.was_installed, s.table_status.clone().unwrap_or_else(|| "ok".into()))
        };
        let installed = c.call("app", "installed", vec![])?.truthy() || was;
        let mut sources = vec![json!({ "name": "processes", "file": null, "state": table })];
        sources.extend(xenonet_engine::peer::list(c.call("app", "sources", vec![Arg::text(home)])?).iter().map(Arg::to_json));
        Ok(json!({ "product": "muse", "name": "Muse", "installed": installed, "sources": sources, "recognized": null }))
    }
}
