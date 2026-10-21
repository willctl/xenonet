#![no_main]

// A transcript file as the service reads it from disk: Claude Code's
// session listing (again after the file grew, from what the first listing
// kept) and its messages, whole and a page at a time (the Agent SDK's, in
// Rust), every line through its app's lenient normalizer, a Codex
// rollout's pages, the Zstandard decoder of archived rollouts, and a
// session's stats; and the tailer, which reads a
// transcript in pieces that can end anywhere and must find the same lines
// as one read of it. The first byte picks Claude Code or Codex and where
// the tailer's pieces end; a NUL splits the transcript from its
// custom-title.json sidecar.

use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_blocks::tailer::Tailer;
use xenonet_schema::drift::DriftLog;
use xenonet_state::stats::{session_stats, File};
use xenonet_wire_map::codex::CodexClient;
use xenonet_wire_map::vendor::Vendor;
use xenonet_wire_map::{sdk, stats};

const ID: &str = "0190d3e0-0000-7000-8000-000000000001";

fn dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("xenonet-fuzz-transcripts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("demo-atlas").join(ID)).expect("a temp folder");
        d
    })
}

// The vendor, with its schemas read once, made again every few thousand
// runs so its drift log (bounded, but with a set of what it let go that
// isn't) doesn't grow across a long run.
fn vendor() -> Arc<Vendor> {
    static V: Mutex<Option<(Arc<Vendor>, u32)>> = Mutex::new(None);
    let mut v = V.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match v.as_mut() {
        Some((vendor, runs)) if *runs < 5000 => {
            *runs += 1;
            vendor.clone()
        }
        _ => {
            let vendor = Vendor::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas"), Arc::new(Mutex::new(DriftLog::default())));
            *v = Some((vendor.clone(), 0));
            vendor
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&how, body)) = data.split_first() else { return };
    let codex = how & 1 != 0;
    let (transcript, sidecar) = xenonet_fuzz::split(body, 0);
    let root = dir();
    let file = root.join("demo-atlas").join(format!("{ID}.jsonl"));
    let title = root.join("demo-atlas").join(ID).join("custom-title.json");
    std::fs::write(&file, transcript).expect("a transcript");
    if sidecar.is_empty() {
        let _ = std::fs::remove_file(&title);
    } else {
        std::fs::write(&title, sidecar).expect("a sidecar");
    }
    let root = root.to_string_lossy().into_owned();
    let file = file.to_string_lossy().into_owned();
    xenonet_fuzz::run(data, || {
        let mut listings = sdk::Listings::default();
        let _ = sdk::list_sessions_with(&root, &mut listings);
        let m = sdk::messages(&root, ID);
        let n = m.len();
        let _ = (m.page(0, n / 2), m.page(usize::from(how) % (n + 1), n), m.page(n, n + 5), m.all());
        let _ = sdk::session_messages(&root, ID);
        // The tailer, in pieces of 1 to 64 bytes, finds what one read finds.
        let mut whole = Tailer::new("fuzz", 0.0, 0.0);
        whole.add_file(&file);
        let at_once = whole.took(&file, transcript, transcript.len() as u64);
        let mut pieces = Tailer::new("fuzz", 0.0, 0.0);
        pieces.add_file(&file);
        let mut found = Vec::new();
        let step = usize::from(how >> 2) % 64 + 1;
        for (i, piece) in transcript.chunks(step).enumerate() {
            found.extend(pieces.took(&file, piece, ((i + 1) * step).min(transcript.len()) as u64));
        }
        assert_eq!(found, at_once, "the same lines, however the reads were cut");
        let vendor = vendor();
        if let Ok(lines) = stats::read_lines(&vendor, &file, codex) {
            let id = if codex { format!("codex:{ID}") } else { format!("claude:{ID}") };
            let _ = session_stats(&id, vec![File { agent: None, lines }]);
        }
        if codex {
            let client = CodexClient::new("codex".into(), "0.0.0".into(), vendor);
            if let Ok(page) = client.rollout_page(&file, None, 20.0) {
                if let Some(cursor) = page.get("nextCursor").and_then(|c| c.as_str()) {
                    let _ = client.rollout_page(&file, Some(cursor), 20.0);
                }
            }
            let _ = client.rollout_page(&file, Some("rollout:3"), 1.0);
            // The same bytes as an archived rollout's (.jsonl.zst): the
            // decoder says what's wrong with them rather than panic.
            let _ = xenonet_wire_map::archive::unzstd(transcript, 1 << 24);
        }
        // The file grows by a line, and is listed again from what the
        // first listing kept.
        let mut grown = transcript.to_vec();
        grown.extend_from_slice(b"\n{\"type\":\"user\",\"message\":{\"content\":\"and more\"}}\n");
        if std::fs::write(&file, &grown).is_ok() {
            let _ = sdk::list_sessions_with(&root, &mut listings);
            let _ = std::fs::write(&file, transcript);
        }
    });
});
