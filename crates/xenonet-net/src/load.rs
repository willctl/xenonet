//! Each machine's load (CPU, GPU, memory and what the parts are) for the
//! window hosts' Machines panel (docs/design/network.md, section 7): a
//! live envelope of kind `load` on link/2, every few seconds, to each
//! connected window host of a network that shares it ("Share load with
//! the network"). Never kept, never passed on, never to a machine that
//! isn't a window host.
//!
//! What crosses is system/v1's own fields and nothing else: no history, no
//! cores, no other machine's readings, no names, serials or OS version. The
//! receiver shapes what it's sent again, since it's untrusted, and keeps
//! the latest of each member in memory, under its host id.

use serde_json::{Map, Value};

/// The envelope's kind.
pub const KIND: &str = "load";
/// How often a machine sends it while a window host is connected.
pub const EVERY_MS: u64 = 5_000;

fn list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The fields of `v` named, in that order, those it has; nothing for a
/// value that isn't an object.
fn pick(v: Option<&Value>, keys: &[&str]) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(Value::Object(o)) = v {
        for k in keys {
            if let Some(x) = o.get(*k) {
                m.insert((*k).into(), x.clone());
            }
        }
    }
    m
}

/// The GPUs of a list, each with the fields named, at most 16.
fn pick_gpus(v: Option<&Value>, keys: &[&str]) -> Value {
    Value::Array(list(v).iter().take(16).map(|g| Value::Object(pick(Some(g), keys))).collect())
}

/// A system/v1 reading as it crosses: what the schema declares of it,
/// with no history, no cores and no other machine's readings.
pub fn shape(data: &Value) -> Value {
    let mut out = pick(Some(data), &["at", "platform", "arch", "uptime"]);
    out.insert("cpu".into(), Value::Object(pick(data.get("cpu"), &["model", "threads", "ghz", "usage"])));
    out.insert("memory".into(), Value::Object(pick(data.get("memory"), &["total", "used"])));
    out.insert("gpus".into(), pick_gpus(data.get("gpus"), &["index", "name", "usage", "memUsed", "memTotal", "temp", "shared"]));
    out.insert("gpu".into(), data.get("gpu").cloned().unwrap_or(Value::Null));
    let hw = data.get("hardware");
    let mut h = pick(hw, &["platform", "arch"]);
    h.insert("cpu".into(), Value::Object(pick(hw.and_then(|x| x.get("cpu")), &["model", "threads", "ghz"])));
    h.insert("memory".into(), Value::Object(pick(hw.and_then(|x| x.get("memory")), &["total"])));
    h.insert("gpus".into(), pick_gpus(hw.and_then(|x| x.get("gpus")), &["name", "memTotal", "shared"]));
    out.insert("hardware".into(), Value::Object(h));
    Value::Object(out)
}

/// A load another machine sent, as this one keeps it: shaped, under its
/// host id.
pub fn kept(host: &str, body: &Value) -> Value {
    let mut v = shape(body);
    v["host"] = Value::String(host.to_string());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_declared_fields_cross() {
        let reading = json!({
            "at": 5, "platform": "darwin", "arch": "arm64", "uptime": 9, "os": "macOS 26.1", "hostname": "Novas-Mac",
            "cpu": { "model": "M3", "threads": 12, "ghz": 4, "usage": 0.2, "cores": [0.1, 0.3] },
            "memory": { "total": 16, "used": 8, "swap": 1 },
            "gpus": [{ "index": 0, "name": "M3", "usage": 0.1, "serial": "X1" }],
            "gpu": 0.1,
            "history": [1, 2, 3],
            "hosts": [{ "host": "rack" }],
            "hardware": { "platform": "darwin", "arch": "arm64", "serial": "C02", "cpu": { "model": "M3", "threads": 12, "ghz": 4 }, "memory": { "total": 16 }, "gpus": [{ "name": "M3", "memTotal": 0, "shared": true, "vendor": "Apple" }] },
        });
        let k = kept("laptop", &reading);
        assert_eq!(k["host"], "laptop");
        assert_eq!(k["cpu"], json!({ "model": "M3", "threads": 12, "ghz": 4, "usage": 0.2 }));
        assert_eq!(k["gpus"], json!([{ "index": 0, "name": "M3", "usage": 0.1 }]));
        assert_eq!(k["hardware"]["gpus"], json!([{ "name": "M3", "memTotal": 0, "shared": true }]));
        for gone in ["os", "hostname", "history", "hosts"] {
            assert!(k.get(gone).is_none(), "{gone}");
        }
        assert!(k["hardware"].get("serial").is_none());
        assert_eq!(kept("rack", &json!("nonsense"))["cpu"], json!({}));
    }
}
