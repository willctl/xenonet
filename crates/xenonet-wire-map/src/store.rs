//! core/store.js and its JSON store (core/blocks/store.js): the small state
//! XenoNet keeps in state.json, read once through xenonet-blocks' store
//! (versions, migrations, files moved aside) and saved a moment after each
//! change, through a temporary file. What a document must hold and what
//! each change does are xenonet-state's store.rs; the debounced save is
//! tokio's here, where the twin had a timer.

use crate::blocks::{health_of, Block};
use crate::util::{lock, now_ms, num};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_blocks::store::{read_versioned, write_atomic, DocHooks, Halt, Report};
use xenonet_service::cores::{Store, R};
use xenonet_state::{statefile, store};

const DEBOUNCE_MS: u64 = 200;

type Doc = Map<String, Value>;

struct Hooks {
    root_key: Option<String>,
}

impl DocHooks<Doc> for Hooks {
    fn fresh(&mut self) -> Result<Doc, Halt> {
        let mut m = Map::new();
        m.insert("version".into(), json!(statefile::STATE_VERSION as u64));
        Ok(m)
    }
    fn parsed(&mut self, v: Value) -> Result<Doc, Halt> {
        Ok(match v {
            Value::Object(m) => m,
            _ => Map::new(),
        })
    }
    // A file without a version is version 1, from before there were
    // versions.
    fn version_of(&mut self, doc: &Doc) -> Result<Option<f64>, Halt> {
        Ok(match doc.get("version") {
            None => Some(1.0),
            Some(v) => v.as_f64(),
        })
    }
    fn migrate(&mut self, doc: Doc, from: f64, to: f64) -> Result<Result<Doc, String>, Halt> {
        let steps = [(1.0, statefile::STATE_VERSION)];
        let plan = match xenonet_state::migrate::plan(&steps, Some(&num(from)), Some(&num(to))) {
            Ok(p) => p,
            Err(why) => return Ok(Err(why)),
        };
        let mut v = Value::Object(doc);
        for _ in plan {
            v = statefile::up_1_to_2(&v, self.root_key.as_deref());
        }
        Ok(Ok(v.as_object().cloned().unwrap_or_default()))
    }
    fn prepare(&mut self, doc: &mut Doc) -> Result<Result<Vec<String>, ()>, Halt> {
        Ok(Ok(store::prepare(doc)))
    }
    fn prepare_fresh(&mut self, doc: &mut Doc) -> Result<(), Halt> {
        store::prepare(doc);
        Ok(())
    }
}

fn report_json(r: &Report) -> Value {
    let opt = |s: &Option<String>| s.clone().map_or(Value::Null, Value::String);
    let mut m = Map::new();
    m.insert("file".into(), Value::String(r.file.clone()));
    m.insert("version".into(), r.version.map_or(Value::Null, num));
    m.insert("migrated".into(), r.migrated.map_or(Value::Null, |(f, t)| json!({ "from": num(f), "to": num(t) })));
    m.insert("backup".into(), opt(&r.backup));
    m.insert("broken".into(), opt(&r.broken));
    m.insert("readOnly".into(), Value::Bool(r.read_only));
    m.insert("problem".into(), opt(&r.problem));
    if !r.repaired.is_empty() {
        m.insert("repaired".into(), json!(r.repaired));
    }
    Value::Object(m)
}

struct Inner {
    doc: Option<Doc>,
    report: Option<Report>,
    unsaved: bool,
    life: Lifecycle,
}

/// state.json, as the map and the service's routes share it.
pub struct StateStore {
    me: Weak<StateStore>,
    file: String,
    /// The projects folder's key, for v1's Codex picks.
    root_key: Option<String>,
    inner: Mutex<Inner>,
    /// Each save moves this on; a debounced flush runs only for the last.
    saves: AtomicU64,
}

impl StateStore {
    pub fn new(state_dir: &str, projects_dir: &str) -> Arc<StateStore> {
        let file = std::path::Path::new(state_dir).join("state.json").to_string_lossy().into_owned();
        Arc::new_cyclic(|me| StateStore {
            me: me.clone(),
            file,
            root_key: xenonet_base::paths::path_key(projects_dir),
            inner: Mutex::new(Inner { doc: None, report: None, unsaved: false, life: Lifecycle::new(now_ms()) }),
            saves: AtomicU64::new(0),
        })
    }

    // The document, read the first time it's wanted. Says whether it was
    // moved up from an older version, to be saved once the lock is let go.
    fn loaded(&self, g: &mut Inner) -> bool {
        let mut migrated = false;
        if g.doc.is_none() {
            let now = now_ms();
            let read = read_versioned(&self.file, statefile::STATE_VERSION, now, &mut Hooks { root_key: self.root_key.clone() });
            let (doc, report) = read.unwrap_or_else(|_| {
                let mut d = Map::new();
                store::prepare(&mut d);
                (d, Report { file: self.file.clone(), ..Default::default() })
            });
            if !report.repaired.is_empty() {
                let n = report.repaired.len();
                crate::util::log(
                    "warn",
                    "store.repaired",
                    &format!("state.json: {} {} so XenoNet started afresh there", report.repaired.join(", "), if n == 1 { "wasn't an object," } else { "weren't objects," }),
                );
            }
            g.life.count("loads", 1.0);
            match &report.problem {
                Some(p) => g.life.problem(p, now),
                None => g.life.ok(now),
            }
            // Moved up from an older version: saved as this one.
            migrated = report.migrated.is_some() && !report.read_only;
            g.doc = Some(doc);
            g.report = Some(report);
            if migrated {
                g.unsaved = true;
            }
        }
        migrated
    }

    /// Reads, or changes and saves, the document.
    pub fn with<T>(&self, f: impl FnOnce(&mut Doc) -> T) -> T {
        let (out, migrated) = {
            let mut g = lock(&self.inner);
            let migrated = self.loaded(&mut g);
            (f(g.doc.get_or_insert_with(Map::new)), migrated)
        };
        if migrated {
            self.schedule();
        }
        out
    }

    pub fn change<T>(&self, f: impl FnOnce(&mut Doc) -> T) -> T {
        let out = self.with(f);
        self.save();
        out
    }

    /// A part of the document, as it is now.
    pub fn part(&self, path: &[&str]) -> Value {
        self.with(|d| {
            let mut v = d.get(path[0]).cloned().unwrap_or(Value::Null);
            for k in &path[1..] {
                v = v.get(*k).cloned().unwrap_or(Value::Null);
            }
            v
        })
    }

    /// Saves a moment later, with whatever else changes meanwhile.
    pub fn save(&self) {
        lock(&self.inner).unsaved = true;
        self.schedule();
    }

    fn schedule(&self) {
        let n = self.saves.fetch_add(1, Ordering::SeqCst) + 1;
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                let me = self.me.clone();
                rt.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
                    if let Some(s) = me.upgrade() {
                        if s.saves.load(Ordering::SeqCst) == n {
                            s.flush();
                        }
                    }
                });
            }
            // No runtime (a test): saved now.
            Err(_) => self.flush(),
        }
    }

    /// Writes what's unsaved now. A write that fails is reported and tried
    /// again with the next change, or at exit.
    pub fn flush(&self) {
        let mut g = lock(&self.inner);
        if !g.unsaved || g.report.as_ref().is_some_and(|r| r.read_only) {
            return;
        }
        let Some(doc) = g.doc.clone() else { return };
        let text = xenonet_base::json::stringify(&Value::Object(doc), 2);
        match write_atomic(&self.file, &text) {
            Ok(()) => {
                g.unsaved = false;
                g.life.count("saves", 1.0);
                g.life.ok(now_ms());
            }
            Err(e) => {
                let message = e.message.clone();
                crate::util::log("error", "store.save", &format!("couldn't save its state to {}: {message}", self.file));
                g.life.problem(&message, now_ms());
            }
        }
    }

    pub fn load_report(&self) -> Value {
        self.with(|_| ());
        lock(&self.inner).report.as_ref().map_or(Value::Null, report_json)
    }

    // ----- what store.js exports -----

    pub fn layout(&self) -> Value {
        self.part(&["layout"])
    }

    pub fn acks(&self) -> Value {
        self.part(&["acks"])
    }

    pub fn ack(&self, session: &str, ts: f64) {
        self.change(|d| obj_at(d, &["acks"]).insert(session.into(), num(ts)));
    }

    pub fn project_rules(&self) -> Value {
        self.part(&["projects", "rules"])
    }

    pub fn set_project_rule(&self, key: &str, rule: Option<Value>) {
        self.change(|d| {
            let rules = obj_at(d, &["projects", "rules"]);
            match rule {
                Some(r) => {
                    rules.insert(key.into(), r);
                }
                None => {
                    rules.shift_remove(key);
                }
            }
        });
    }

    pub fn clear_project_rules(&self, under_key: Option<&str>) {
        self.change(|d| {
            let rules = obj_at(d, &["projects", "rules"]);
            let keys: Vec<String> = rules.keys().cloned().collect();
            for k in store::rules_to_clear(&keys, under_key) {
                rules.shift_remove(&k);
            }
        });
    }

    pub fn codex_picks(&self, root_key: Option<&str>) -> Option<Value> {
        let key = store::folder_key(root_key.map(|k| Value::String(k.into())).as_ref()).ok()?;
        Some(self.part(&["projects", "codexByRoot"]).get(&key).cloned().unwrap_or(Value::Null)).filter(|v| !v.is_null())
    }

    pub fn set_codex_picks(&self, root_key: Option<&str>, picks: Option<Value>) -> Result<(), String> {
        let key = store::folder_key(root_key.map(|k| Value::String(k.into())).as_ref())?;
        self.change(|d| {
            let all = obj_at(d, &["projects", "codexByRoot"]);
            match picks {
                Some(p) => {
                    all.insert(key, p);
                }
                None => {
                    all.shift_remove(&key);
                }
            }
        });
        Ok(())
    }

    pub fn workspace_root(&self) -> Value {
        self.part(&["workspace", "root"])
    }

    pub fn set_workspace_root(&self, dir: Option<String>) {
        self.change(|d| obj_at(d, &["workspace"]).insert("root".into(), dir.map_or(Value::Null, Value::String)));
    }

    pub fn identities(&self) -> Value {
        self.part(&["identities"])
    }

    pub fn set_identity_name(&self, id: &str, name: Option<String>) {
        self.change(|d| {
            let names = obj_at(d, &["identities", "names"]);
            match name {
                Some(n) => {
                    names.insert(id.into(), Value::String(n));
                }
                None => {
                    names.shift_remove(id);
                }
            }
        });
    }

    pub fn set_identity_hidden(&self, id: &str, hidden: bool) {
        self.change(|d| {
            let h = obj_at(d, &["identities", "hidden"]);
            if hidden {
                h.insert(id.into(), Value::Bool(true));
            } else {
                h.shift_remove(id);
            }
        });
    }

    pub fn set_identity_pin(&self, node: &str, identity: Option<String>) {
        self.change(|d| {
            let pins = obj_at(d, &["identities", "pins"]);
            match identity {
                Some(i) => {
                    pins.insert(node.into(), json!({ "identity": i, "at": num(now_ms()) }));
                }
                None => {
                    pins.shift_remove(node);
                }
            }
        });
    }
}

/// The object at a path of the document, made where it's missing.
pub fn obj_at<'a>(d: &'a mut Doc, path: &[&str]) -> &'a mut Doc {
    let mut cur = d;
    for k in path {
        let slot = cur.entry(k.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        cur = slot.as_object_mut().expect("made an object just above");
    }
    cur
}

impl Block for StateStore {
    fn name(&self) -> String {
        "state".into()
    }
    fn health(&self) -> Value {
        health_of("state", "store", &lock(&self.inner).life, &[], Value::Null)
    }
}

/// The Store core the service's routes call.
pub struct StoreCore(pub Arc<StateStore>);

impl Store for StoreCore {
    fn ack(&self, session: &str) -> R<()> {
        self.0.ack(session, now_ms());
        Ok(())
    }
    fn layout(&self) -> R<Value> {
        Ok(self.0.layout())
    }
    fn set_layout(&self, positions: &Value) -> R<()> {
        self.0.change(|d| {
            let layout = obj_at(d, &["layout"]);
            for (id, p) in store::layout_changes(Some(positions)) {
                match p {
                    Some(p) => {
                        layout.insert(id, p);
                    }
                    None => {
                        layout.shift_remove(&id);
                    }
                }
            }
        });
        Ok(())
    }
    fn clear_layout(&self) -> R<()> {
        self.0.change(|d| d.insert("layout".into(), Value::Object(Map::new())));
        Ok(())
    }
    fn theme(&self) -> R<Value> {
        Ok(self.0.part(&["theme"]))
    }
    fn set_theme(&self, id: &str) -> R<()> {
        self.0.change(|d| d.insert("theme".into(), Value::String(id.into())));
        Ok(())
    }
    fn environment(&self) -> R<Value> {
        Ok(self.0.part(&["environment"]))
    }
    fn set_environment(&self, id: &str) -> R<()> {
        self.0.change(|d| d.insert("environment".into(), Value::String(id.into())));
        Ok(())
    }
    fn load_report(&self) -> R<Value> {
        Ok(self.0.load_report())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-wire-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_v1_file_moves_up_and_keeps_its_parts() {
        let d = dir("v1");
        std::fs::write(d.join("state.json"), r#"{"layout":{"core":{"x":1,"y":2,"z":3}},"projects":{"codex":{"picks":[]}}}"#).unwrap();
        let s = StateStore::new(&d.to_string_lossy(), "/p");
        assert_eq!(s.layout()["core"]["x"], 1);
        assert_eq!(s.codex_picks(Some("/p")), Some(json!({ "picks": [] })), "v1's picks under the folder of the day");
        let r = s.load_report();
        assert_eq!(r["migrated"], json!({ "from": 1, "to": 2 }));
        assert!(r["backup"].as_str().unwrap().ends_with("state.v1.json"));
        // Saved as v2, with no runtime to wait on.
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.join("state.json")).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn changes_as_store_js_makes_them() {
        let d = dir("changes");
        let s = StateStore::new(&d.to_string_lossy(), "/p");
        let core = StoreCore(s.clone());
        core.set_layout(&json!({ "a": { "x": 1.005, "y": 0, "z": 2 }, "b": { "x": 1, "y": 1, "z": 1 } })).unwrap();
        core.set_layout(&json!({ "b": null })).unwrap();
        assert_eq!(core.layout().unwrap(), json!({ "a": { "x": 1, "y": 0, "z": 2 } }));
        core.clear_layout().unwrap();
        assert_eq!(core.layout().unwrap(), json!({}));
        s.set_project_rule("/p/labs", Some(json!({ "path": "/p/labs", "as": "project" })));
        s.set_project_rule("/q/x", Some(json!({ "path": "/q/x", "as": "group" })));
        s.clear_project_rules(Some("/p"));
        assert_eq!(s.project_rules().as_object().unwrap().keys().collect::<Vec<_>>(), ["/q/x"]);
        s.set_identity_pin("claude:a", Some("github:x/y".into()));
        assert_eq!(s.identities()["pins"]["claude:a"]["identity"], "github:x/y");
        s.set_identity_pin("claude:a", None);
        assert_eq!(s.identities()["pins"], json!({}));
        assert!(s.set_codex_picks(None, None).is_err(), "picks are for a folder");
        assert_eq!(core.theme().unwrap(), Value::Null);
        core.set_theme("rime").unwrap();
        assert_eq!(core.theme().unwrap(), "rime");
        let h = s.health();
        assert_eq!((h["name"].as_str(), h["kind"].as_str()), (Some("state"), Some("store")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
