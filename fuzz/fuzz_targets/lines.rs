#![no_main]

// A transcript's bytes as a read hands them over (a read can end anywhere,
// inside a line or a character), then every line through the views Claude
// Code's and Codex's readers take of it, and through each app's mapper to
// its events.

use libfuzzer_sys::fuzz_target;
use xenonet_base::json;
use xenonet_sources::line_events::{self, LineFile};
use xenonet_sources::lines::{self, ClaudeLine, CodexLine};

fn view(raw: serde_json::Value) {
    let c = ClaudeLine { raw: raw.clone() };
    let _ = (c.kind(), c.ts(), c.meta(), c.sidechain(), c.compact_summary(), c.effort());
    let _ = (c.content(), c.has_results(), c.results(), c.text(), c.interruption());
    let _ = (c.message(), c.message_id(), c.model(), c.usage(), c.blocks(), c.end_turn());
    if let Ok(k) = c.kind() {
        let _ = lines::claude_kind_of(k);
    }
    let _ = line_events::claude(&c, &LineFile::default());
    let x = CodexLine { raw };
    for file in [LineFile::default(), LineFile { items: true, ..LineFile::default() }, LineFile { follow: true, ..LineFile::default() }] {
        let _ = line_events::codex(&x, &file);
    }
    if let Ok(p) = x.payload() {
        let _ = line_events::codex_item(p, 0.0, &LineFile { follow: true, ..LineFile::default() });
    }
    let _ = (x.kind(), x.ts(), x.payload(), x.user_message(), x.user_event());
    if let (Ok(k), Ok(p)) = (x.kind(), x.payload()) {
        let pt = lines::opt_prop(Some(p), "type");
        let it = lines::opt_prop(lines::opt_prop(Some(p), "item"), "type");
        let _ = lines::codex_kind_of(k, pt, it);
        let _ = lines::codex_version_of(k, lines::opt_prop(Some(p), "cli_version"), None);
        let _ = lines::is_codex_prompt(lines::opt_prop(Some(p), "message"));
    }
    let _ = lines::has_user_messages(&[x]);
}

fuzz_target!(|data: &[u8]| {
    xenonet_fuzz::run(data, || {
        let taken = lines::take(data);
        for l in &taken.lines {
            if let Ok(v) = json::parse(l) {
                view(v);
            }
        }
        if let Some(v) = taken.last {
            view(v);
        }
        // The rest waits for the next read, which starts with it.
        let mut next = taken.rest.clone();
        next.extend_from_slice(b"\n");
        let _ = lines::take(&next);
    });
});
