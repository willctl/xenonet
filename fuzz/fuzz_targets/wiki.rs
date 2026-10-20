#![no_main]

// The shared context's folder, as other agents write it: a page with its
// frontmatter and wiki links, the projects it names, the day's log, and
// the whole folder read as the map reads it. NULs split the page from the
// log and the inbox note.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use xenonet_sources::wiki::{self, Out};

const NOW: f64 = 1_791_230_000_000.0;

fn dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("xenonet-fuzz-wiki-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["pages", "log/2026", "inbox"] {
            std::fs::create_dir_all(d.join(sub)).expect("a temp folder");
        }
        d
    })
}

fuzz_target!(|data: &[u8]| {
    let (page, rest) = xenonet_fuzz::split(data, 0);
    let (log, note) = xenonet_fuzz::split(rest, 0);
    let text = xenonet_fuzz::text(page);
    let log_text = xenonet_fuzz::text(log);
    let projects = json!([
        { "rel": "atlas-api", "name": "atlas-api", "key": "project:/home/nova/projects/atlas-api" },
        { "rel": "labs/geofence", "name": "geofence", "key": "project:/home/nova/projects/labs/geofence" },
    ]);
    let root = Value::String("/home/nova/Projects".into());
    let d = dir();
    std::fs::write(d.join("pages").join("refunds.md"), page).expect("a page");
    std::fs::write(d.join("log/2026").join("2026-10-05.md"), log).expect("a log");
    std::fs::write(d.join("inbox").join("note.md"), note).expect("a note");
    xenonet_fuzz::run(data, || {
        let _ = wiki::frontmatter_at(&text);
        let (meta, body) = wiki::parse_frontmatter(&text);
        let _ = wiki::wiki_links(&body);
        let _ = wiki::mentioned_projects(&body, meta.get("tags"), Some(&projects), Some(&root));
        let _ = wiki::parse_log(&log_text, Some(&json!("2026-10-05")));
        for v in meta.values() {
            let _ = (wiki::to_str(Some(v)), wiki::to_str_or_empty(Some(v)));
        }
        let _ = wiki::read_wiki(&d.to_string_lossy(), Some(&projects), Some(&root), NOW, Out::Json(json!("agent-wiki")));
    });
});
