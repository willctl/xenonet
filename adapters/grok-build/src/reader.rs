//! Grok Build's sessions for the live tailer: which files under its
//! sessions folder are its sessions' (each session's updates.jsonl, in a
//! folder named for it), how a line is sampled for the drift check, and
//! what each line means on the telemetry. Its lines have Claude Code's
//! shapes, read through Claude Code's `claude_line`, with the run of
//! chunks each session's mapper keeps between lines; its tokens and cost
//! come from the usage.json beside its updates, read again when it was
//! written since. The map's listing of its sessions and a page of a conversation are
//! `listing` (over `entries`).

pub mod entries;
pub mod listing;

use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use xenonet_adapter::reader::{Call, Classified, Emit, Tail, Tailed, TypeError, VendorCheck, Where};
use xenonet_claude_code::reader::claude_line;
use xenonet_model::usage::{Speed, TokenUsage};
use xenonet_sources::grok::{usage_subagent, usage_turns};
use xenonet_sources::line_events::{self, LineFile, OpenRun};
use xenonet_sources::lines::{grok_kind_of, GrokLine};
use xenonet_sources::live::telemetry::refusals;

pub const SOURCE: &str = "spacexai/grok-build/update-line";
const USAGE: &str = "spacexai/grok-build/usage";

/// What a turn's call counted so far (tokens by kind, and cost), so a
/// turn written again adds only what's new.
#[derive(Clone, Copy, Debug, Default)]
struct Counted {
    input: f64,
    cache_read: f64,
    cache_write: f64,
    output: f64,
    cost: f64,
}

/// What the reader keeps of a session as its lines come: the run of
/// chunks its mapper has open, its usage.json as last read (its stamp),
/// and what each of its turns' calls counted so far, by turn and model.
#[derive(Default)]
struct Session {
    run: Option<OpenRun>,
    usage_stamp: Option<String>,
    counted: HashMap<String, Counted>,
}

/// The reader.
#[derive(Default)]
pub struct Reader {
    sessions: HashMap<String, Session>,
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    /// A session's usage.json and summary.json beside its updates, read
    /// when the file was written since it was last read: each of its
    /// turns' calls counted once, when the turn ended, as Claude Code's
    /// calls are, at what Grok said it cost, or without a price when it
    /// didn't say; and what a turn written again adds (Grok folds what
    /// you send during a turn into it). A subagent's are its parent's,
    /// whose turns count them, and a fork's turns from before it are the
    /// session's it came from (grok.rs's usage_turns). Grok writes the
    /// file as each turn ends, before the update that ends the turn, so
    /// it's asked after each of a session's lines.
    fn usage(&mut self, t: &mut dyn Tail, vendor: &dyn VendorCheck, at: &Where, live: bool, out: &mut Vec<Emit>) {
        let Some(dir) = at.file.as_str().and_then(|f| Path::new(f).parent()) else { return };
        let Ok(m) = std::fs::metadata(dir.join("usage.json")) else { return };
        let stamp = format!("{}:{}", m.len(), xenonet_os::fs::mtime_ms_of(&m).unwrap_or(f64::NAN));
        let Some(i) = t.find(&at.session_id) else { return };
        let s = self.sessions.entry(at.session_id.clone()).or_default();
        if s.usage_stamp.as_deref() == Some(stamp.as_str()) {
            return;
        }
        s.usage_stamp = Some(stamp);
        let json = |name: &str| std::fs::read_to_string(dir.join(name)).ok().and_then(|t| xenonet_text::json::parse(&t).ok());
        let usage = json("usage.json").and_then(|raw| vendor.sample(USAGE, raw, None, None, 16));
        let summary = json("summary.json");
        if usage_subagent(summary.as_ref()) {
            return;
        }
        for c in usage_turns(usage.as_ref(), summary.as_ref()) {
            let was = s.counted.get(&c.key).copied().unwrap_or_default();
            s.counted.insert(c.key.clone(), Counted { input: c.input, cache_read: c.cache_read, cache_write: c.cache_write, output: c.output, cost: c.cost.unwrap_or(0.0) });
            let (input, cache_read, cache_write, output) =
                ((c.input - was.input).max(0.0), (c.cache_read - was.cache_read).max(0.0), (c.cache_write - was.cache_write).max(0.0), (c.output - was.output).max(0.0));
            let n = input + cache_read + cache_write + output;
            if n.is_nan() || n <= 0.0 {
                continue;
            }
            let model = Value::String(c.model.clone());
            t.set_model(i, model.clone());
            t.add_output_tokens(i, output);
            t.add_tokens(i, n, c.t, live);
            let call = Call {
                tokens: TokenUsage {
                    input: Some(input),
                    cache_read: Some(cache_read),
                    cache_write_5m: Some(cache_write),
                    cache_write_1h: Some(0.0),
                    output: Some(output),
                    speed: Speed::default(),
                },
            };
            t.add_reported(i, &model, &call, c.t, c.cost.map(|cost| (cost - was.cost).max(0.0)));
            if live {
                out.push(Emit::tokens(t.session_id(i), "grok", n, c.t));
            }
        }
    }
}

impl Tailed for Reader {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn source(&self) -> &'static str {
        SOURCE
    }

    fn block(&self) -> &'static str {
        "grok-lines"
    }

    /// Grok Build's lines say no version.
    fn version_of(&self, _raw: &Value, current: Option<String>) -> Option<String> {
        current
    }

    fn kind_of(&self, raw: &Value) -> String {
        grok_kind_of(Some(raw))
    }

    /// A session's updates, in the session's folder, which is named for
    /// it: `<root>/<encoded cwd>/<session id>/updates.jsonl`. Its other
    /// files (chat history, rewind points) aren't read.
    fn classify(&self, norm: &str, root: Option<&str>) -> Option<Classified> {
        let rest = &norm[root?.len()..];
        let parts: Vec<&str> = rest.split('/').collect();
        (parts.len() == 4 && parts[0].is_empty() && !parts[1].is_empty() && !parts[2].is_empty() && parts[3] == "updates.jsonl")
            .then(|| Classified { id: parts[2].to_string(), agent: None })
    }

    fn line(&mut self, t: &mut dyn Tail, vendor: &dyn VendorCheck, at: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        // What the mapper keeps of the session's file, as the lines come
        // (its runs of chunks).
        t.session(&at.session_id, "grok");
        t.observe_servers(&raw);
        let o = GrokLine::new(raw);
        let s = self.sessions.entry(at.session_id.clone()).or_default();
        let file = LineFile { run: RefCell::new(s.run.take()), ..LineFile::new(Some(at.agent_id.clone())) };
        let events = line_events::grok(&o, &file);
        s.run = file.run.into_inner();
        refusals(t, "grok", at, &o.raw, &events, live, out);
        let read = claude_line(t, at, "grok", o.ts(), &events, live, now, out);
        self.usage(t, vendor, at, live, out);
        read
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sessions_updates_are_its_file() {
        let r = Reader::new();
        let root = "/h/.grok/sessions";
        assert_eq!(r.classify("/h/.grok/sessions/%2Fw/abc-1234567/updates.jsonl", Some(root)), Some(Classified { id: "abc-1234567".into(), agent: None }));
        assert_eq!(r.classify("/h/.grok/sessions/%2Fw/abc-1234567/history.jsonl", Some(root)), None);
        assert_eq!(r.classify("/h/.grok/sessions/%2Fw/abc-1234567/updates.jsonl", None), None);
    }
}
