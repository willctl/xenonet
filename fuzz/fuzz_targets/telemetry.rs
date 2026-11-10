#![no_main]

// The live reader: a transcript's lines, one after another, into one
// Telemetry, as the tailer hands them over (each app's reader comes from
// its adapter, as in the service), then everything the map asks of it.
// The first byte picks Claude Code or Codex, a subagent's file or the
// session's own, and whether each line also comes as a hook's body, a
// plan usage answer and Claude desktop's samples.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_adapter::reader::{Tail, Unchecked, Where};
use xenonet_sources::lines;
use xenonet_sources::live::telemetry::Telemetry;
use xenonet_sources::refusals::{detect, store};
use xenonet_text::json;

const NOW: f64 = 1_791_230_000_000.0;

fuzz_target!(|data: &[u8]| {
    let Some((&how, body)) = data.split_first() else { return };
    xenonet_fuzz::run(data, || {
        let codex = how & 1 != 0;
        let mut t = Telemetry::new(Some("/home/nova".into()), vec![("/home/nova/.codex/sessions".into(), "codex".into())]);
        let id = if codex { "codex:0190d3e0-0000-7000-8000-000000000002" } else { "claude:0190d3e0-0000-7000-8000-000000000001" };
        let f = Where {
            session_id: id.into(),
            agent_id: if how & 2 != 0 { Value::String("agent-f8851cd6".into()) } else { Value::Null },
            file: Value::String("/home/nova/.claude/projects/demo/0190d3e0-0000-7000-8000-000000000001.jsonl".into()),
        };
        let taken = lines::take(body);
        let mut all = Vec::new();
        let mut now = NOW;
        for (n, l) in taken.lines.iter().enumerate() {
            let Ok(v) = json::parse(l) else { continue };
            let ts = lines::ClaudeLine { raw: v.clone() }.ts().unwrap_or(f64::NAN);
            if ts.is_finite() {
                now = now.max(ts);
            }
            let mut out = Vec::new();
            if how & 4 != 0 {
                let _ = t.note_hook(id, Some(&v), now, &mut out);
                let _ = t.usage_reading("claude", Some(&v), now);
                t.usage_samples("claude", &v);
                let _ = t.usage_reading("codex", Some(&v), now);
            }
            let _ = t.line(if codex { "codex" } else { "claude" }, &Unchecked, &f, v.clone(), n % 3 != 0, now, &mut out);
            all.push(v);
        }
        for v in &all {
            if let Some(sig) = detect::claude_signal(Some(v)) {
                let _ = detect::key_of(&sig, id);
                if let Some(c) = store::claude_context(&all, &sig) {
                    let _ = c.to_json();
                }
            }
            if let Some(sig) = detect::codex_signal(Some(v)) {
                let _ = detect::key_of(&sig, id);
                if let Ok(c) = store::codex_context(&all, &sig) {
                    let _ = c.to_json();
                }
            }
        }
        if let Some(l) = taken.lines.first() {
            let _ = t.classify(l);
        }
        t.settle_usage("claude");
        t.settle_usage("codex");
        t.settle(now + 3_600_000.0);
        for later in [now, now + 60_000.0, now + 8.0 * 86_400_000.0] {
            let snap = t.snapshot(later);
            let _ = Telemetry::collab(&[snap.clone(), snap]);
            let _ = t.collisions(later);
            let _ = t.costs(later, &all);
            let _ = t.session_costs(later);
            let _ = t.session_notes(id, later);
        }
        let _ = t.collision_diffs(id, "claude:0190d3e0-0000-7000-8000-000000000003");
        let _ = t.pending_label();
    });
});
