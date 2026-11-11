#![no_main]

// What another machine of the network sends a console: its relay's
// frames, read into the host's link (hello, snapshots, events, answers,
// times, byes), checked against the schemas the binary checks them with,
// and merged into the map with this machine's own. Also what a console
// sends a relay. The first byte picks where the stream is cut, and which
// side reads it.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use xenonet_runtime::remote::frames::{self, FROM_CONSOLE, FROM_RELAY};
use xenonet_runtime::remote::link::{Asks, Ended, Link};
use xenonet_runtime::remote::merge::{self, HostIn};
use xenonet_runtime::remote::relay::{self, Scrubber};
use xenonet_runtime::remote::runtime::schemas::{HostSchemas, MergeSchemas};

const NOW: f64 = 1_791_230_000_000.0;

fn schemas() -> &'static HostSchemas {
    static S: OnceLock<HostSchemas> = OnceLock::new();
    S.get_or_init(|| HostSchemas::new(xenonet_state::graph::activity_of))
}

struct Ask;

impl Asks for Ask {
    fn random(&mut self) -> f64 {
        0.5
    }
    fn check_hello(&mut self, frame: &Value) -> Option<String> {
        schemas().check_hello(frame)
    }
}

fn local() -> Value {
    json!({ "nodes": [{ "id": "core", "kind": "core", "label": "XenoNet" }, { "id": "project:/home/nova/projects/atlas-api", "kind": "project", "label": "atlas-api", "git": { "origin": "git@github.com:atlas-co/atlas-api.git" } }], "links": [] })
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, stream)) = data.split_first() else { return };
    xenonet_fuzz::run(stream, || {
        // The frames as lines, the stream cut where the first byte says.
        let cut = usize::from(pick) * stream.len() / 256;
        let text = [String::from_utf8_lossy(&stream[..cut]), String::from_utf8_lossy(&stream[cut..])].concat();
        let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty() && l.len() <= 1 << 16).collect();
        let windows = json!({ "activeWindowMs": 600000, "recentWindowMs": 86400000, "projectActiveMs": 3600000 });
        if pick & 1 == 0 {
            // The console's side: a link to vm1 that reads what came.
            let entry = json!({ "id": "vm1", "transport": "network" });
            let mut link = Link::new(entry, NOW, 1e4, 3e4, (1e3, 6e4), 2e4);
            let mut a = Ask;
            let _ = link.start(NOW, &mut a);
            let mut snapshot = Value::Null;
            for (n, line) in lines.iter().enumerate() {
                let now = NOW + n as f64 * 10.0;
                let _ = link.line(1, line, now, &mut a);
                let Ok(frame) = frames::parse_frame(line, Some(&FROM_RELAY)) else { continue };
                let _ = frames::shallow(&frame, frames::MAX_DEPTH);
                let info = link.info();
                let mut ms = MergeSchemas(schemas());
                match frame.get("t").and_then(Value::as_str) {
                    Some("snapshot") => snapshot = frame.clone(),
                    Some("event") => {
                        let name = frame.get("name").and_then(Value::as_str).unwrap_or("");
                        let _ = merge::qualify_event(name, frame.get("data").unwrap_or(&Value::Null), &info, &mut ms);
                    }
                    _ => {}
                }
            }
            let info = link.info();
            let mut ms = MergeSchemas(schemas());
            let mut ins = vec![HostIn { info: info.clone(), snapshot: snapshot.clone(), clean: None }];
            let _ = merge::merge_graph(&local(), &mut ins, &json!({}), NOW, &windows, &json!("core"), &json!({}), &json!({}), &HashMap::new(), &mut ms);
            let pairs = [(info.clone(), snapshot.clone())];
            let _ = merge::merge_live_with(&json!({ "sessions": [] }), &pairs, &mut ms);
            let _ = merge::merge_summary(&json!({}), &pairs, &mut ms);
            if let Some(e) = snapshot.get("engine") {
                let _ = merge::qualify_engine(e, &info, &mut ms);
            }
            let _ = (link.time_tick(1, NOW + 5e4), link.check_stale(NOW + 1e5));
            let bye = lines.last().and_then(|l| frames::parse_frame(l, None).ok());
            let _ = link.end(1, &Ended { bye: bye.as_ref(), spawn_error: None, network: None }, NOW + 2e5, &mut a);
            let _ = link.info();
        } else {
            // The relay's side: what a console asks of it, and its folders
            // taken out of what it sends back.
            let scrub = Scrubber::new(Some("/home/nova"), &["/srv/work".to_string()]);
            for line in &lines {
                let _ = scrub.text(line);
                let Ok(frame) = frames::parse_frame(line, Some(&FROM_CONSOLE)) else { continue };
                let _ = scrub.walk(&frame);
                if let Ok((shape, path)) = relay::allowed(frame.get("path")) {
                    let _ = relay::route_to_read(shape, &path);
                }
                let _ = relay::user_folder_of(frame.get("path"), Some("/home/nova"));
            }
        }
    });
});
