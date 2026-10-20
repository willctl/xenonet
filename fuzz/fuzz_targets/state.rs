#![no_main]

// state.json as the service loads it: read from disk, its version read,
// migrated (v1 to v2), prepared, and what the map asks of it, then a
// layout from the page put in it and the file saved again. The file can be
// one an older or newer XenoNet wrote, or one cut short or edited by hand.
// A NUL splits the file from the positions the page sends.

use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use std::sync::OnceLock;
use xenonet_state::{migrate, statefile, store};
use xenonet_wire_map::store::StateStore;

fn dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| std::env::temp_dir().join(format!("xenonet-fuzz-state-{}", std::process::id())))
}

fuzz_target!(|data: &[u8]| {
    let (file, positions) = xenonet_fuzz::split(data, 0);
    let d = dir();
    // A fresh folder each time: a load can leave a backup of a file it
    // couldn't read.
    let _ = std::fs::remove_dir_all(d);
    std::fs::create_dir_all(d).expect("a temp folder");
    std::fs::write(d.join("state.json"), file).expect("a state file");
    let positions = xenonet_fuzz::json(positions);
    xenonet_fuzz::run(data, || {
        if let Some(v) = xenonet_fuzz::json(file) {
            let _ = statefile::up_1_to_2(&v, Some("/home/nova/projects"));
            let _ = migrate::plan(&[(1.0, 2.0)], v.get("version"), Some(&serde_json::json!(2)));
            if let serde_json::Value::Object(mut m) = v {
                let _ = store::prepare(&mut m);
            }
            let _ = store::folder_key(positions.as_ref());
        }
        let s = StateStore::new(&d.to_string_lossy(), "/home/nova/Projects");
        let _ = (s.load_report(), s.layout(), s.acks(), s.project_rules(), s.workspace_root(), s.identities());
        let _ = s.codex_picks(Some("/home/nova/projects"));
        let _ = s.part(&["layout", "positions"]);
        let _ = store::layout_changes(positions.as_ref());
        s.change(|doc| {
            for (k, v) in store::layout_changes(positions.as_ref()) {
                let layout = doc.entry("layout").or_insert_with(|| serde_json::json!({}));
                if let Some(l) = layout.as_object_mut() {
                    match v {
                        Some(v) => l.insert(k, v),
                        None => l.shift_remove(&k),
                    };
                }
            }
        });
        s.ack("claude:0190d3e0-0000-7000-8000-000000000001", 1_791_230_000_000.0);
        s.set_identity_name("github:nova-dev", Some("Nova".into()));
        s.flush();
    });
});
