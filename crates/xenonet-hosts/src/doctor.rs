//! core/hosts/doctor.js: the doctor's check of the hosts as a whole:
//! whether two hosts answer with the same key, as one XenoNet added twice
//! does, or two machines when one's state folder was copied from the
//! other's. They'd keep one snapshot in the cache between them.

use serde_json::{json, Value};

fn and(ids: &[String]) -> String {
    if ids.len() < 3 {
        ids.join(" and ")
    } else {
        format!("{} and {}", ids[..ids.len() - 1].join(", "), ids[ids.len() - 1])
    }
}

/// The hosts that count: answered with a key, and not simulated.
pub fn keyed(hosts: &[Value]) -> Vec<Value> {
    hosts.iter().filter(|h| xenonet_app::jsv::truthy(h.get("key")) && !xenonet_app::jsv::truthy(h.get("simulated"))).cloned().collect()
}

pub fn key_check(hosts: &[Value]) -> Value {
    let mut by_key: Vec<(String, Vec<String>)> = Vec::new();
    for h in keyed(hosts) {
        let key = xenonet_base::text::js_string(&h["key"]);
        let id = h.get("id").map_or_else(|| "undefined".into(), xenonet_base::text::js_string);
        match by_key.iter_mut().find(|(k, _)| *k == key) {
            Some((_, ids)) => ids.push(id),
            None => by_key.push((key, vec![id])),
        }
    }
    let sort16 = |v: &mut Vec<String>| v.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut shared: Vec<Vec<String>> = by_key.iter().filter(|(_, ids)| ids.len() > 1).map(|(_, ids)| ids.clone()).collect();
    for s in &mut shared {
        sort16(s);
    }
    if shared.is_empty() {
        let mut all: Vec<String> = by_key.into_iter().flat_map(|(_, ids)| ids).collect();
        sort16(&mut all);
        return json!({ "status": "ok", "detail": format!("{} each answer with a key of their own", and(&all)) });
    }
    let first = &shared[0];
    let other = first.last().cloned().unwrap_or_default();
    let detail = shared.iter().map(|ids| format!("{} answer with the same key", and(ids))).collect::<Vec<_>>().join("; ");
    json!({
        "status": "warn",
        "detail": format!("{detail}, so they keep one snapshot in the cache between them: each writes over the other's, and after a restart they show the same map until they answer"),
        "fix": format!("If {} are one XenoNet reached two ways, remove one: xenonet hosts remove {other}. If they're two machines, one's state folder was copied from the other's: on that one, quit XenoNet, take \"self\" out of hosts.json in its state folder so it makes a key of its own, and start it again; then accept its new key here: xenonet hosts add {other} … --replace.", and(first)),
    })
}
