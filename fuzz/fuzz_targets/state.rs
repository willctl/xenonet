#![no_main]

// What XenoNet saves for you (the store's state domain, in place of
// state.json), as the page and the routes change it: a layout the page
// sends, then a value the routes keep for you (a rule for a folder, an
// app's picks, what XenoNet knows of an identity, an app's sign-ins, the
// folders the map found), each read back as the map reads it. A NUL
// splits the positions from the value. Nothing is read from disk: the
// store is in memory, and a fresh one each run.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::sync::Arc;
use xenonet_api::cores::Store;
use xenonet_saved::{layout_changes, rules_to_clear, Saved};
use xenonet_store::Db;

const NOW: f64 = 1_791_230_000_000.0;

fuzz_target!(|data: &[u8]| {
    let (positions, value) = xenonet_fuzz::split(data, 0);
    let positions = xenonet_fuzz::json(positions).unwrap_or(Value::Null);
    let value = xenonet_fuzz::json(value);
    xenonet_fuzz::run(data, || {
        let s = Saved::with_clock(Arc::new(Db::memory().expect("a store in memory")), Arc::new(|| NOW));
        let _ = layout_changes(&positions);
        let _ = s.set_layout(&positions);
        let _ = s.layout();
        if let Some(v) = &value {
            let _ = s.set_project_rule("/home/nova/projects/labs", Some(v));
            let _ = s.set_app_setting("codex", "picks", "/home/nova/projects", Some(v));
            let _ = s.set_identity_known("github:nova-dev", v);
            let _ = s.set_signins("claude", v);
            if let Some(name) = v.as_str() {
                let _ = (s.set_identity_name("github:nova-dev", Some(name)), s.set_identity_pin("project:atlas", Some(name)), s.set_theme(name));
            }
            // The folders a scan found: none is opened, since no version
            // control is installed here.
            let _ = s.reconcile_checkouts(v);
        }
        s.ack("claude:0190d3e0-0000-7000-8000-000000000001").expect("an ack");
        let _ = (s.acks(), s.marks(), s.project_rules(), s.identities(), s.theme(), s.checkouts());
        let _ = s.app_setting("codex", "picks", "/home/nova/projects");
        let keys: Vec<String> = s.project_rules().ok().and_then(|r| r.as_object().map(|m| m.keys().cloned().collect())).unwrap_or_default();
        let _ = rules_to_clear(&keys, Some("/home/nova/projects"));
        let _ = s.clear_project_rules(Some("/home/nova/projects"));
        let _ = s.clear_layout();
    });
});
