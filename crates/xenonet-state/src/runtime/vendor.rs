//! The vendor readers and the line normalizers, as the map's
//! readers share them: one lenient reader for each vendor source, which
//! records what differs from XenoNet's schemas in the service's one drift
//! log, and the normalizers every transcript line goes
//! through (claude-lines, codex-lines, grok-lines: one for each tailed app
//! the adapters bring), with the transcript hooks that sample each file's
//! lines as its app's reader says. The readers are the service's one set
//! (WireEnv's), which the actors' observers read through too, so the drift
//! report reads what all of them have read.

use crate::runtime::blocks::{health_of, Block};
use crate::runtime::util::{lock, now_ms};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use xenonet_adapter::reader::{Tailed, VendorCheck};
use xenonet_blocks::normalizer::{Hooks, Normalizer};
use xenonet_schema::drift::{DriftLog, Piece};
use xenonet_schema::vendor::{Checked, Readers, Sampling, VendorReader};

pub const CLAUDE_LINES: &str = "anthropic/claude-code/transcript-line";
pub const CODEX_LINES: &str = "openai/codex/rollout-line";

/// A tailed app's lines: the adapter's reader of them (which says how a
/// line is sampled) and their normalizer.
struct Lines {
    tailed: Box<dyn Tailed>,
    normalizer: Mutex<Normalizer>,
}

/// Every reader, by its source, the log they record in, and the
/// transcripts' normalizers, one for each tailed app the adapters bring.
pub struct Vendor {
    readers: Arc<Readers>,
    pub drift: Arc<Mutex<DriftLog>>,
    apps: Vec<Lines>,
}

/// What a check made of a record: the record to use, or None to drop it.
fn out(c: Checked, record: Value) -> Option<Value> {
    match c {
        Checked::Same => Some(record),
        Checked::Stripped(v) => Some(v),
        Checked::Dropped => None,
    }
}

impl Vendor {
    /// Readers of its own, over `dir` (XenoNet's schemas/ folder).
    pub fn new(dir: PathBuf, drift: Arc<Mutex<DriftLog>>) -> Arc<Vendor> {
        Vendor::with_readers(Arc::new(Readers::new(dir, drift)))
    }

    /// The map's vendor over the service's readers.
    pub fn with_readers(readers: Arc<Readers>) -> Arc<Vendor> {
        let now = now_ms();
        let apps = xenonet_adapter::platforms()
            .tailed()
            .into_iter()
            .map(|tailed| Lines { normalizer: Mutex::new(Normalizer::new(tailed.block(), Some(tailed.source()), now)), tailed })
            .collect();
        Arc::new(Vendor { drift: readers.drift().clone(), readers, apps })
    }

    /// The lines of the tailed app `app` names: any
    /// app that isn't tailed reads as Claude Code's.
    fn app(&self, app: &str) -> Option<&Lines> {
        let id = xenonet_adapter::apps::tailed_app(app, "claude");
        self.apps.iter().find(|l| l.tailed.id() == id)
    }

    pub fn dir(&self) -> &std::path::Path {
        self.readers.dir()
    }

    fn reader<T>(&self, source: &str, f: impl FnOnce(&mut VendorReader, &mut DriftLog) -> T) -> T {
        self.readers.with(source, f)
    }

    /// reader(source).check(record, { version, kind }): counted and checked
    /// whole.
    pub fn check(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>) -> Option<Value> {
        let c = self.readers.check(source, &record, version, kind, now_ms());
        out(c, record)
    }

    /// reader(source).sample(record, { version, kind, every }).
    pub fn sample(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>, every: u64) -> Option<Value> {
        let c = self.reader(source, |r, log| r.sample(&record, version, kind.map(str::to_string), every, log, now_ms()));
        out(c, record)
    }

    /// reader(source).pick(version), for the drift a version none of its
    /// schemas has is.
    pub fn pick(&self, source: &str, version: Option<&str>) {
        self.reader(source, |r, log| {
            r.pick(version, log, now_ms());
        });
    }

    /// A record that wasn't JSON.
    pub fn unparsable(&self, source: &str, version: Option<&str>) {
        let mut log = lock(&self.drift);
        log.record(
            &Piece { source: source.into(), kind: "unparsable".into(), vendor_version: version.map_or(Value::Null, |v| Value::String(v.into())), ..Default::default() },
            now_ms(),
        );
    }

    /// A field a reader looks for and didn't find.
    pub fn missing(&self, source: &str, path: &str, version: Option<&str>) {
        let mut log = lock(&self.drift);
        log.record(
            &Piece {
                source: source.into(),
                kind: "missing-field".into(),
                path: Value::String(path.into()),
                vendor_version: version.map_or(Value::Null, |v| Value::String(v.into())),
                ..Default::default()
            },
            now_ms(),
        );
    }

    /// What a source's reader has read, as the drift report says: { records,
    /// checked, dropped, stripped, versions }, None when it has read nothing.
    /// The observers' reads count too.
    pub fn read_of(&self, source: &str) -> Option<Value> {
        self.readers.read_of(source)
    }

    /// A transcript line through its app's normalizer: the record to read, or None for a
    /// line that isn't JSON, is dropped, or breaks a hook. `sampling` is the
    /// file's: which of its lines are checked whole. `app` is the
    /// transcript's: any app that isn't tailed is read as Claude Code's.
    pub fn line(&self, app: &str, text: &str, sampling: &mut Sampling) -> Option<Value> {
        let l = self.app(app)?;
        let mut hooks = Transcript { vendor: self, app: &*l.tailed, sampling };
        let mut n = lock(&l.normalizer);
        n.line(text, &(), &mut hooks, now_ms)
    }

    /// A record already parsed, through the normalizer, as the last line of a file without a newline is read.
    pub fn record(&self, app: &str, raw: Value, sampling: &mut Sampling) -> Option<Value> {
        let l = self.app(app)?;
        let mut hooks = Transcript { vendor: self, app: &*l.tailed, sampling };
        let mut n = lock(&l.normalizer);
        n.record(raw, &(), &mut hooks, now_ms)
    }

    /// The normalizers, as blocks.
    pub fn blocks(self: &Arc<Self>) -> Vec<Arc<dyn Block>> {
        (0..self.apps.len()).map(|k| -> Arc<dyn Block> { Arc::new(LinesBlock { vendor: self.clone(), k }) }).collect()
    }
}

/// The map's vendor, as a reader checks records through it.
impl VendorCheck for Vendor {
    fn check(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>) -> Option<Value> {
        Vendor::check(self, source, record, version, kind)
    }
    fn sample(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>, every: u64) -> Option<Value> {
        Vendor::sample(self, source, record, version, kind, every)
    }
    fn line(&self, app: &str, text: &str, sampling: &mut Sampling) -> Option<Value> {
        Vendor::line(self, app, text, sampling)
    }
    fn record(&self, app: &str, raw: Value, sampling: &mut Sampling) -> Option<Value> {
        Vendor::record(self, app, raw, sampling)
    }
    fn pick(&self, source: &str, version: Option<&str>) {
        Vendor::pick(self, source, version);
    }
    fn missing(&self, source: &str, path: &str, version: Option<&str>) {
        Vendor::missing(self, source, path, version);
    }
}

// The transcript hooks: each line counted,
// the version each file's lines carry followed, a sample of each kind of
// line in each file checked whole (the 1st, 2nd, 4th, 8th…), the rest
// guarded.
struct Transcript<'a> {
    vendor: &'a Vendor,
    app: &'a dyn Tailed,
    sampling: &'a mut Sampling,
}

impl Hooks<Value, Value, ()> for Transcript<'_> {
    fn parse(&mut self, text: &str) -> Result<Value, String> {
        xenonet_text::json::parse(text).map_err(|e| e.to_string())
    }

    fn inspect(&mut self, raw: Value, (): &()) -> Result<Option<Value>, String> {
        let source = self.app.source();
        let app = self.app;
        let sampling = &mut *self.sampling;
        let c = self.vendor.reader(source, |r, log| {
            r.stats.records += 1;
            let now = now_ms();
            if !raw.is_object() {
                return r.inspect(&raw, sampling.version.as_deref(), None, log, now);
            }
            let version = app.version_of(&raw, sampling.version.clone());
            if version != sampling.version {
                sampling.version.clone_from(&version);
                r.pick(version.as_deref(), log, now);
            }
            let kind = app.kind_of(&raw);
            if sampling.whole(&kind) {
                return r.inspect(&raw, version.as_deref(), Some(kind), log, now);
            }
            if r.any_careful() {
                r.guard(&raw, version.as_deref(), &Some(kind), log, now)
            } else {
                Checked::Same
            }
        });
        Ok(out(c, raw))
    }

    fn unparsable(&mut self, _text: &str, (): &(), _why: &str) {
        let source = self.app.source();
        let version = self.sampling.version.clone();
        self.vendor.reader(source, |r, _| r.stats.records += 1);
        self.vendor.unparsable(source, version.as_deref());
    }

    fn normalize(&mut self, raw: Value, (): &()) -> Result<Value, String> {
        Ok(raw)
    }
}

struct LinesBlock {
    vendor: Arc<Vendor>,
    k: usize,
}

impl Block for LinesBlock {
    fn name(&self) -> String {
        self.vendor.apps[self.k].tailed.block().into()
    }
    fn health(&self) -> Value {
        let mut n = lock(&self.vendor.apps[self.k].normalizer);
        n.look(now_ms());
        health_of(&self.name(), "normalizer", &n.life, &[], Value::Null)
    }
}

/// XenoNet's schemas/ folder: under its root (XENONET_ROOT, or the checkout
/// or package the binary is in), else beside the binary.
pub fn schemas_dir(root: &std::path::Path) -> PathBuf {
    let at = root.join("schemas");
    if at.join("vendor").is_dir() {
        return at;
    }
    std::env::current_exe().ok().and_then(|exe| exe.ancestors().skip(1).take(5).map(|d| d.join("schemas")).find(|d| d.join("vendor").is_dir())).unwrap_or(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout_schemas() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
    }

    #[test]
    fn lines_are_read_counted_and_sampled() {
        xenonet_platforms::install();
        let v = Vendor::new(checkout_schemas(), Arc::new(Mutex::new(DriftLog::default())));
        let mut s = Sampling::default();
        let line = r#"{"type":"user","version":"2.0.1","timestamp":"2026-10-17T10:00:00.000Z","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(v.line("claude", line, &mut s).unwrap()["type"], "user");
        assert!(v.line("claude", "{ not json", &mut s).is_none());
        let read = v.read_of(CLAUDE_LINES).unwrap();
        assert_eq!((read["records"].as_u64(), read["checked"].as_u64()), (Some(2), Some(1)));
        assert_eq!(read["versions"], serde_json::json!(["2.0.1"]));
        assert!(v.read_of(CODEX_LINES).is_none());
        assert_eq!(v.blocks().len(), 3);
        let drift = lock(&v.drift).list();
        assert!(drift.iter().any(|r| r["kind"] == "unparsable"));
        let h = v.blocks()[0].health();
        assert_eq!((h["name"].as_str(), h["counters"]["records"].as_u64(), h["counters"]["unparsable"].as_u64()), (Some("claude-lines"), Some(1), Some(1)));
    }
}
