//! The runs XenoNet observes without a reader of its own (state.js's
//! `observed`): the experimental native readers of other harnesses' own
//! records (core/native/), one for each app the registry gives `native`
//! that XENONET_READERS names, and numbat's records (core/numbat.js, when
//! XENONET_NUMBAT_FILE names them). Each reads with xenonet-sources' reader,
//! through its NativeReader, on every refresh, and is a block of its own;
//! the runs actors report come with them in the state.

use crate::blocks::{health_of, Block};
use crate::config::MapConfig;
use crate::util::{lock, native_out, now_ms, obj};
use crate::vendor::Vendor;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_base::apps::App;
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_sources::native::files::StdFiles;
use xenonet_sources::native::{self, copilot_cli, cursor, gemini_cli, opencode, Checked, NativeReader, Threw};
use xenonet_sources::numbat::Numbat;

/// What a reader is: its name as a source of the map, its block's, what
/// it reads in words (for its status), its vendor schema, and whether its
/// runs are observed (numbat's) rather than read from the harness's own
/// records.
#[derive(Clone, Copy)]
struct Kind {
    source: &'static str,
    block: &'static str,
    what: &'static str,
    schema: &'static str,
    observed: bool,
}

/// A native reader the map can make (core/native/index.js's row): the app
/// it reads, by its id in the registry; what it is; the file or folder it
/// reads, from the map's settings; and a new reader.
struct Row {
    app: &'static str,
    kind: Kind,
    at: fn(&MapConfig) -> String,
    make: fn() -> Box<dyn NativeReader>,
}

/// The native readers, a row for each app the registry gives `native`, in
/// the order the state lists them.
const NATIVE: [Row; 4] = [
    Row {
        app: "opencode",
        kind: Kind { source: "opencode", block: "opencode-sessions", what: "OpenCode's database", schema: opencode::SOURCE, observed: false },
        at: opencode_db,
        make: || Box::new(opencode::OpenCode::new()),
    },
    Row {
        app: "gemini-cli",
        kind: Kind { source: "gemini-cli", block: "gemini-cli-sessions", what: "Gemini CLI's folder", schema: gemini_cli::SOURCE, observed: false },
        at: |cfg| gemini_cli::gemini_root(var(cfg, "GEMINI_CLI_HOME").as_deref(), &cfg.home_dir),
        make: || Box::new(gemini_cli::GeminiCli::new()),
    },
    Row {
        app: "copilot",
        kind: Kind { source: "copilot", block: "copilot-sessions", what: "Copilot CLI's folder", schema: copilot_cli::SOURCE, observed: false },
        at: |cfg| copilot_cli::copilot_root(&cfg.home_dir),
        make: || Box::new(copilot_cli::CopilotCli::new()),
    },
    Row {
        app: "cursor",
        kind: Kind { source: "cursor", block: "cursor-sessions", what: "the Cursor hook's file", schema: cursor::SOURCE, observed: false },
        at: cursor_events,
        make: || Box::new(cursor::Cursor::new()),
    },
];

const NUMBAT: Kind = Kind { source: "numbat", block: "numbat-sessions", what: "numbat's file", schema: "", observed: true };

// A variable of the service's environment, when it's set.
fn var(cfg: &MapConfig, k: &str) -> Option<String> {
    cfg.env.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string)
}

fn opencode_db(cfg: &MapConfig) -> String {
    let named = var(cfg, "OPENCODE_DB").filter(|n| n != ":memory:");
    opencode::opencode_db_path(&cfg.home_dir, var(cfg, "XDG_DATA_HOME").as_deref(), named.as_deref())
}

fn cursor_events(cfg: &MapConfig) -> String {
    let named = var(cfg, "XENONET_CURSOR_EVENTS");
    cursor::cursor_events_file(named.as_deref(), if named.is_some() { "" } else { &cfg.state_dir })
}

pub struct Observed {
    kind: Kind,
    /// Its name as a source of the map.
    pub source: &'static str,
    /// The file or folder it reads.
    path: String,
    vendor: Arc<Vendor>,
    inner: Mutex<(Box<dyn NativeReader>, Lifecycle)>,
}

// A record checked against its vendor's schema, as the native readers ask.
fn checked(vendor: &Vendor, source: &str, rec: &Value, version: Option<&str>) -> Checked {
    match vendor.check(source, rec.clone(), version, None) {
        None => Checked::Null,
        Some(v) if &v == rec => Checked::Same,
        Some(v) => Checked::Other(v),
    }
}

/// The map's vendor readers, as one reader's records are checked: against
/// its schema.
struct Schema<'a> {
    vendor: &'a Vendor,
    schema: &'static str,
}

impl native::Vendor for Schema<'_> {
    fn missing(&mut self, path: &str, version: Option<&str>) -> Result<(), Threw> {
        self.vendor.missing(self.schema, path, version);
        Ok(())
    }
    fn check(&mut self, record: &Value, version: Option<&str>) -> Result<Checked, Threw> {
        Ok(checked(self.vendor, self.schema, record, version))
    }
}

impl Observed {
    fn make(kind: Kind, path: String, reader: Box<dyn NativeReader>, vendor: Arc<Vendor>) -> Arc<Observed> {
        Arc::new(Observed { kind, source: kind.source, path, vendor, inner: Mutex::new((reader, Lifecycle::new(now_ms()))) })
    }

    /// The native readers that are on, and numbat's when its file is named,
    /// in the order the state lists them: the native readers first, so a
    /// run they read wins over numbat's.
    pub fn all(cfg: &MapConfig, vendor: &Arc<Vendor>) -> Vec<Arc<Observed>> {
        let mut out = Self::native(cfg, vendor, xenonet_base::apps::apps(), &NATIVE);
        if let Some(file) = cfg.numbat_file.clone() {
            out.push(Observed::make(NUMBAT, file, Box::new(Numbat::new()), vendor.clone()));
        }
        out
    }

    /// The native readers that are on, made now: for each app `apps` (the
    /// registry, or a test's) gives `native` that XENONET_READERS names, its
    /// row's reader, in the table's order.
    fn native(cfg: &MapConfig, vendor: &Arc<Vendor>, apps: &[App], table: &[Row]) -> Vec<Arc<Observed>> {
        let native = |id: &str| apps.iter().any(|a| a.id == id && a.has("native"));
        table.iter().filter(|row| native(row.app) && cfg.readers.contains(row.app)).map(|row| Observed::make(row.kind, (row.at)(cfg), (row.make)(), vendor.clone())).collect()
    }

    /// Reads what's new; a problem is the block's to say, never thrown.
    pub fn read(&self) {
        let now = now_ms();
        let mut g = lock(&self.inner);
        let (reader, life) = &mut *g;
        let mut vendor = Schema { vendor: &self.vendor, schema: self.kind.schema };
        match reader.read(&self.path, now, &mut StdFiles, &mut vendor) {
            Ok(()) => life.ok(now),
            Err(m) => life.problem(&m, now),
        }
    }

    fn runs(reader: &dyn NativeReader) -> Vec<Value> {
        reader.runs_out().iter().filter_map(native_out).collect()
    }

    /// The runs as sessions, for the map.
    pub fn sessions(&self) -> Vec<Value> {
        lock(&self.inner).0.sessions().iter().filter_map(native_out).collect()
    }

    /// What the engine reads of a run, in the shape of a live session's:
    /// when it was last at work, its edits, and no turn XenoNet can see.
    pub fn live(&self, id: &str) -> Option<Value> {
        let g = lock(&self.inner);
        let r = Self::runs(&*g.0).into_iter().find(|r| r.get("id").and_then(Value::as_str) == Some(id))?;
        let edits: Vec<Value> = crate::util::items(r.get("edits"))
            .iter()
            .filter_map(|kv| kv.get(1).and_then(Value::as_array).and_then(|l| l.last()).cloned())
            .map(|e| obj(vec![("path", e.get("path").cloned().unwrap_or(Value::Null)), ("ts", e.get("ts").cloned().unwrap_or(Value::Null))]))
            .collect();
        Some(obj(vec![
            ("observed", json!(self.kind.observed)),
            ("lastEventAt", r.get("updatedAt").cloned().unwrap_or(Value::Null)),
            ("endedAt", r.get("endedAt").cloned().unwrap_or(Value::Null)),
            ("status", json!("idle")),
            ("edits", Value::Array(edits)),
        ]))
    }

    /// Its status as a source of the map.
    pub fn status(&self) -> Value {
        let g = lock(&self.inner);
        xenonet_state::state::reader_status(self.kind.source, g.0.source_state(), Self::runs(&*g.0).len(), self.kind.what)
    }
}

impl Block for Observed {
    fn name(&self) -> String {
        self.kind.block.into()
    }
    fn health(&self) -> Value {
        let g = lock(&self.inner);
        health_of(self.kind.block, "gatherer", &g.1, &[], Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use xenonet_schema::drift::DriftLog;
    use xenonet_sources::native::files::Files;
    use xenonet_sources::native::{Edits, Out};
    use xenonet_sources::session::{RunSession, Via};

    fn vendor() -> Arc<Vendor> {
        Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())))
    }

    fn config(readers: &str) -> MapConfig {
        let mut env = Map::new();
        env.insert("XENONET_READERS".into(), json!(readers));
        MapConfig::new(false, false, env, "/home/nova", None)
    }

    #[test]
    fn numbats_records_are_runs_on_the_map() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-numbat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("events.ndjson");
        let now = xenonet_base::time::iso(now_ms());
        let ev = |kind: &str, file: Option<&str>| json!({ "schema_version": "0.5.0", "record_type": "event", "source_agent": "gemini-cli", "event_type": kind, "session_id": "g-0000001", "project_path": "/w/atlas", "file_path": file, "timestamp": now });
        let text: String = [ev("session.start", None), ev("file.write", Some("/w/atlas/src/a.js"))].iter().map(|e| xenonet_base::json::stringify(e, 0) + "\n").collect();
        std::fs::write(&file, text).expect("its records");
        let o = Observed::make(NUMBAT, file.to_string_lossy().into_owned(), Box::new(Numbat::new()), vendor());
        o.read();
        assert_eq!((o.status()["name"].as_str(), o.status()["ok"].as_bool(), o.status()["count"].as_u64()), (Some("numbat"), Some(true), Some(1)));
        let sessions = o.sessions();
        assert_eq!(sessions.len(), 1);
        let id = sessions[0]["id"].as_str().unwrap_or_default().to_string();
        let live = o.live(&id).expect("its run");
        assert_eq!((live["observed"].as_bool(), live["edits"][0]["path"].as_str()), (Some(true), Some("/w/atlas/src/a.js")));
        assert_eq!(o.health()["name"], "numbat-sessions");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }

    #[test]
    fn a_reader_for_each_app_the_registry_gives_one_that_xenonet_readers_names() {
        let mut apps: Vec<&str> = NATIVE.iter().map(|r| r.app).collect();
        let mut native: Vec<&str> = xenonet_base::apps::with("native").map(|a| a.id.as_str()).collect();
        apps.sort_unstable();
        native.sort_unstable();
        assert_eq!(apps, native, "a row for each app with `native`, and none else");
        for r in &NATIVE {
            assert_eq!((r.kind.source, r.kind.observed), (r.app, false));
        }
        let on =
            |readers: &str| Observed::all(&config(readers), &vendor()).iter().map(|o| (o.source, o.health()["name"].as_str().unwrap_or_default().to_string())).collect::<Vec<_>>();
        assert_eq!(
            on("cursor, copilot,gemini-cli,opencode"),
            [
                ("opencode", "opencode-sessions".to_string()),
                ("gemini-cli", "gemini-cli-sessions".into()),
                ("copilot", "copilot-sessions".into()),
                ("cursor", "cursor-sessions".into())
            ]
        );
        assert_eq!(on("vscode,claude,nope"), [], "only apps with a native reader");
        // An app the registry doesn't give `native` has no reader, even with a row.
        let apps: Vec<App> = xenonet_base::apps::apps().iter().map(|a| if a.id == "cursor" { App { has: Vec::new(), ..a.clone() } } else { a.clone() }).collect();
        let made = Observed::native(&config("cursor,opencode"), &vendor(), &apps, &NATIVE);
        assert_eq!(made.iter().map(|o| o.source).collect::<Vec<_>>(), ["opencode"]);
    }

    /// A made-up app's reader, Kite's: one run, read from nothing.
    #[derive(Default)]
    struct Kite {
        state: &'static str,
        updated_at: Option<f64>,
        edits: Edits,
    }

    impl NativeReader for Kite {
        fn read(&mut self, at: &str, now: f64, _: &mut dyn Files, _: &mut dyn native::Vendor) -> Result<(), String> {
            if at.is_empty() {
                self.state = "missing";
                return Err("no file".into());
            }
            self.state = "ok";
            self.updated_at = Some(now);
            self.edits = Edits::new();
            self.edits.insert("/w/atlas/notes.txt", vec![native::Edit { ts: now, path: "/w/atlas/notes.txt".into() }]);
            Ok(())
        }
        fn source_state(&self) -> &'static str {
            if self.state.is_empty() {
                "unread"
            } else {
                self.state
            }
        }
        fn ids(&self) -> Vec<String> {
            self.updated_at.map(|_| "kite:k-0000001".to_string()).into_iter().collect()
        }
        fn sessions(&self) -> Vec<Out> {
            let Some(at) = self.updated_at else { return Vec::new() };
            vec![RunSession {
                id: "kite:k-0000001",
                app: "kite",
                raw_id: "k-0000001",
                title: "Kite in atlas".into(),
                cwd: Some("/w/atlas"),
                created_at: Out::Num(at),
                updated_at: Out::Num(at),
                archived: false,
                model: None,
                branch: None,
                subagent_of: None,
                via: Via::Reader,
            }
            .record()]
        }
        fn runs_out(&self) -> Vec<Out> {
            let Some(at) = self.updated_at else { return Vec::new() };
            vec![Out::Obj(vec![("id", Out::str("kite:k-0000001")), ("updatedAt", Out::Num(at)), ("edits", native::edits_out(&self.edits))])]
        }
    }

    #[test]
    fn a_made_up_reader_for_a_made_up_app_is_on_the_map() {
        let mut apps = xenonet_base::apps::apps().to_vec();
        apps.push(App { id: "kite".into(), name: "Kite".into(), numbat: vec!["kite".into()], has: vec!["native".into()], ..App::default() });
        let kite = |at: fn(&MapConfig) -> String| Row {
            app: "kite",
            kind: Kind { source: "kite", block: "kite-sessions", what: "Kite's log", schema: "", observed: false },
            at,
            make: || Box::<Kite>::default(),
        };
        let table = [kite(|cfg| format!("{}/kite.log", cfg.state_dir))];
        let made = Observed::native(&config("kite"), &vendor(), &apps, &table);
        assert_eq!(made.len(), 1);
        let o = &made[0];
        assert_eq!((o.source, o.health()["name"].as_str()), ("kite", Some("kite-sessions")));
        assert_eq!(o.status(), json!({ "name": "kite", "ok": false, "count": 0, "error": "Kite's log is unread" }));
        o.read();
        assert_eq!(o.status(), json!({ "name": "kite", "ok": true, "count": 1 }));
        let sessions = o.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!((sessions[0]["id"].as_str(), sessions[0]["app"].as_str(), sessions[0]["reader"].as_str()), (Some("kite:k-0000001"), Some("kite"), Some("experimental")));
        let live = o.live("kite:k-0000001").expect("its run");
        assert_eq!((live["observed"].as_bool(), live["edits"][0]["path"].as_str()), (Some(false), Some("/w/atlas/notes.txt")));
        assert_eq!(o.live("kite:other"), None);
        // Off unless XENONET_READERS names it, and off for an app the
        // registry doesn't give `native`.
        assert!(Observed::native(&config("opencode"), &vendor(), &apps, &table).is_empty());
        assert!(Observed::native(&config("kite"), &vendor(), xenonet_base::apps::apps(), &table).is_empty());
        // A read that fails is its block's problem.
        let broken = Observed::native(&config("kite"), &vendor(), &apps, &[kite(|_| String::new())]);
        broken[0].read();
        assert_eq!(broken[0].status(), json!({ "name": "kite", "ok": false, "count": 0, "error": "Kite's log is missing" }));
        assert_eq!(broken[0].health()["state"], "degraded");
    }
}
