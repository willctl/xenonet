//! What the hosts take from another machine, checked and copied: a host
//! entry and a relay's hello read as host/list/v1's entry and
//! host/hello/v1, and what another host sends read as its document's type
//! (crates/xenonet-model) and copied with only what that type declares
//! (core/hosts/merge.js's clean). Everything a host sends is untrusted, so
//! the copy is what the type keeps of it: a key the type doesn't name is
//! dropped at every level, and a value that isn't its type is left out.

use serde_json::{json, Value};
use xenonet_hosts::merge;
use xenonet_model::{engine, graph, host, identity, live, system};
use xenonet_schema::typed::{self, clean};

/// The checks on what hosts send, by the document each frame is.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostSchemas;

// `<path>: <message>`, with "(the value)" for the root.
fn explain(issue: typed::Issue) -> String {
    format!("{}: {}", if issue.path.is_empty() { "(the value)" } else { &issue.path }, issue.message)
}

impl HostSchemas {
    pub fn new() -> Self {
        HostSchemas
    }

    /// A host entry (host/list/v1's hosts' items), or why it isn't one.
    pub fn check_entry(&self, v: &Value) -> Option<String> {
        typed::read::<host::Host>(v).err().map(explain)
    }

    /// A relay's hello, or why it isn't one.
    pub fn check_hello(&self, v: &Value) -> Option<String> {
        typed::read::<host::Hello>(v).err().map(explain)
    }

    /// clean(kind, value): checked, then copied with what its type
    /// declares. The core and host nodes are this machine's to make, so
    /// another host's are refused.
    pub fn clean(&self, kind: merge::Kind<'_>, v: &Value) -> Option<Value> {
        match kind {
            merge::Kind::Node("session") => clean::<graph::SessionNode>(v),
            merge::Kind::Node("project") => clean::<graph::ProjectNode>(v),
            merge::Kind::Node("native") => clean::<graph::NativeNode>(v),
            merge::Kind::Node("zone") => clean::<graph::ZoneNode>(v),
            merge::Kind::Node("wiki") => clean::<graph::WikiNode>(v),
            merge::Kind::Node("page") => clean::<graph::PageNode>(v),
            merge::Kind::Node("resource") => clean::<graph::ResourceNode>(v),
            merge::Kind::Node(_) => None,
            merge::Kind::Identity => clean::<identity::Identity>(v),
            merge::Kind::LiveSession => clean::<live::Session>(v),
            merge::Kind::LiveActivity => clean::<live::Activity>(v),
            merge::Kind::EngineAgent => clean::<engine::Agent>(v),
            merge::Kind::EngineMessage => clean::<engine::Message>(v),
            merge::Kind::System => clean::<system::System>(v),
        }
    }
}

/// The merge's asks, answered: the schemas, and graph.js's activityOf.
pub struct MergeSchemas<'a>(pub &'a HostSchemas);

impl merge::Schemas for MergeSchemas<'_> {
    fn clean(&mut self, kind: merge::Kind<'_>, value: &Value) -> Option<Value> {
        self.0.clean(kind, value)
    }

    fn activity(&mut self, updated_at: &Value, now: f64, windows: &Value) -> Value {
        json!(xenonet_state::graph::activity_of(Some(updated_at), now, Some(windows)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hosts_entry_and_hello_as_their_types_check_them() {
        let s = HostSchemas::new();
        let entry =
            json!({ "id": "lab", "transport": "local", "ssh": null, "port": 47840, "command": null, "since": "7d", "detail": false, "key": null, "name": null, "addedAt": 1 });
        assert_eq!(s.check_entry(&entry), None);
        let mut bad = entry.clone();
        bad["port"] = json!("47840");
        assert!(s.check_entry(&bad).unwrap().starts_with("port: "));
        let mut bad = entry;
        bad["id"] = json!("Not A Name");
        assert!(s.check_entry(&bad).unwrap().starts_with("id: "));
        assert!(s.check_hello(&json!({ "t": "hello" })).unwrap().starts_with("(the value): missing field"));
    }

    #[test]
    fn what_a_host_sends_keeps_only_what_its_type_declares() {
        let s = HostSchemas::new();
        let mut m = MergeSchemas(&s);
        use merge::Schemas;
        let wiki = json!({ "id": "wiki", "kind": "wiki", "name": "Agent Wiki", "path": "~/wiki", "pages": 3, "script": "<b>x</b>" });
        if let Some(out) = m.clean(merge::Kind::Node("wiki"), &wiki) {
            assert!(out.get("script").is_none(), "{out}");
            assert_eq!(out["kind"], "wiki");
        }
        assert_eq!(m.clean(merge::Kind::Node("core"), &json!({ "id": "core", "kind": "core" })), None, "the core is this machine's");
        assert_eq!(m.clean(merge::Kind::Node("session"), &json!({ "kind": "session" })), None, "one that doesn't pass is left out");
    }

    #[test]
    fn a_hosts_load_fits_system_v1_and_keeps_only_what_it_declares() {
        let s = HostSchemas::new();
        let mut m = MergeSchemas(&s);
        use merge::Schemas;
        // The demo's reading, as this machine's own /api/system answers it.
        let demo = xenonet_os::stats::demo_reading(1_800_000_000_000.0);
        assert_eq!(m.clean(merge::Kind::System, &demo).as_ref(), Some(&demo), "it fits, with nothing it doesn't declare");
        let mut sent = demo.clone();
        sent["cpu"]["script"] = json!("<b>x</b>");
        sent["hardware"]["serial"] = json!("C02XYZ");
        let kept = m.clean(merge::Kind::System, &sent).unwrap();
        assert!(kept["cpu"].get("script").is_none() && kept["hardware"].get("serial").is_none(), "{kept}");
        sent["cpu"]["usage"] = json!("2");
        assert_eq!(m.clean(merge::Kind::System, &sent), None, "a share that isn't a number isn't one");
    }
}
