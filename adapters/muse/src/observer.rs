//! Muse's observer (docs/design/actors/muse.md): presence, nothing more.
//! Muse is Meta's personal agent, and it works in a cloud VM of its own
//! whether or not its Mac app is open. So:
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

use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_adapter::observe::attribution::census;
use xenonet_adapter::observe::{known, AppBundle, Core, Keep, Observer, ObserverHost, Result, Say};
use xenonet_adapter::products::Product;
use xenonet_os::sync::Locked;
use xenonet_text::text::display;

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

pub struct Muse {
    core: Core,
    app: AppBundle,
    state: Mutex<State>,
}

impl Default for Muse {
    fn default() -> Self {
        Self::new()
    }
}

impl Muse {
    pub fn new() -> Self {
        Muse { core: Core::new(crate::product()), app: AppBundle::new("Muse.app", r"^com\.meta\.endo(\.|$)"), state: Mutex::new(State::default()) }
    }
}

impl Observer for Muse {
    fn name(&self) -> &'static str {
        "muse-observer"
    }

    fn product(&self) -> &'static Product {
        self.core.product()
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn look(&self, h: &dyn ObserverHost, at: f64) -> Result<()> {
        let known = known(h, ID);
        // Whether the app where apps go is Meta's, from its Info.plist.
        self.app.read(h);
        // Not installed where apps go and not seen before: the process table
        // is read only once a minute, for an app run from somewhere else.
        if !known && !self.app.installed(h) {
            let mut s = self.state.locked();
            if at - s.looked_at.unwrap_or(f64::NEG_INFINITY) < STRAY_MS {
                return Ok(());
            }
            s.looked_at = Some(at);
        }
        let mut table_state = None;
        let census = census(h, &mut table_state);
        self.state.locked().table_status = table_state;
        let census = census?;
        let open = census.table.iter().enumerate().any(|(i, p)| {
            let (product, role) = census.product_role(i);
            product == Some("muse") && role == Some("app") && p.row.get("program").filter(|v| !v.is_null()).map_or(String::new(), display).ends_with(MAIN)
        });
        let installed = open || self.app.installed(h);
        self.state.locked().was_installed = installed;
        if !installed && !known {
            return Ok(());
        }
        let version = if self.app.ours() { self.app.plist().and_then(|p| p.version) } else { None };
        let keep = Keep { name: "Muse".into(), version, via: vec!["observed"], heartbeat_ms: None, capabilities: CAPABILITIES.to_vec(), installed: Some(installed), at };
        self.core.keep(h, ID, &keep)?;
        if !h.actors().enabled(ID) {
            return Ok(());
        }
        let (presence, status) =
            if open { ("idle", "Its app is open on this Mac; it works in Meta's cloud") } else { ("unknown", "Its app on this Mac is closed; it may still work in Meta's cloud") };
        self.core.say(h, ID, &Say { presence, status: Some(status.into()), usage: None, at })?;
        Ok(())
    }

    /// What the doctor shows: whether the app is installed, and what this
    /// observer reads.
    fn status(&self, h: &dyn ObserverHost) -> Result<Value> {
        let (was, table) = {
            let s = self.state.locked();
            (s.was_installed, s.table_status.clone().unwrap_or_else(|| "ok".into()))
        };
        let installed = self.app.installed(h) || was;
        let mut sources = vec![json!({ "name": "processes", "file": null, "state": table })];
        sources.extend(self.app.sources(h));
        Ok(json!({ "product": "muse", "name": "Muse", "installed": installed, "sources": sources, "recognized": null }))
    }
}
