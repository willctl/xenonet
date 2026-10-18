//! Hosts as this XenoNet keeps them (core/hosts/config.js,
//! crates/xenonet-hosts/src/config.rs): hosts.json in the state folder, read
//! once as core/blocks/store.js reads a versioned file, and XenoNet's own
//! ssh config written from it, readable by you alone. What an entry may
//! hold, the ssh config's text and what's kept of a hand-edited file are
//! the Rust's; the store, the files and the key are here.

use super::schemas::HostSchemas;
use crate::fx::{now_ms, Locked};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_blocks::store::{read_versioned, write_atomic, DocHooks, Report};
use xenonet_blocks::Halt;
use xenonet_hosts::config::{self, Refused};
use xenonet_service::cores::{CoreError, HostsConfig, R};

fn refused(r: Refused) -> CoreError {
    CoreError::status(r.status, r.message)
}

struct Doc {
    doc: Value,
    report: Report,
}

pub struct HostsStore {
    state_dir: String,
    schemas: Arc<HostSchemas>,
    loaded: Mutex<Option<Doc>>,
    /// The store as a building block: its state, last success and error,
    /// and its loads and saves, for /api/health and the doctor.
    life: Mutex<Lifecycle>,
    /// The entries the last read left out, and why.
    pub dropped: Mutex<Vec<String>>,
}

struct Hooks<'a> {
    schemas: &'a HostSchemas,
    dropped: Vec<String>,
}

impl Hooks<'_> {
    fn fill(&mut self, doc: &mut Value) {
        let (self_v, hosts, dropped) = config::prepare(doc, now_ms(), &mut |h| self.schemas.check_entry(h).is_none());
        *doc = json!({ "schema": "host/list/v1", "self": self_v, "hosts": hosts });
        self.dropped = dropped;
    }
}

impl DocHooks<Value> for Hooks<'_> {
    fn fresh(&mut self) -> Result<Value, Halt> {
        Ok(json!({ "schema": "host/list/v1", "self": null, "hosts": [] }))
    }
    fn parsed(&mut self, v: Value) -> Result<Value, Halt> {
        Ok(v)
    }
    fn version_of(&mut self, doc: &Value) -> Result<Option<f64>, Halt> {
        Ok(config::version_of(doc))
    }
    // hosts.json has had one version.
    fn migrate(&mut self, _doc: Value, from: f64, to: f64) -> Result<Result<Value, String>, Halt> {
        Ok(Err(format!("no way from version {from} to {to}")))
    }
    fn prepare(&mut self, doc: &mut Value) -> Result<Result<Vec<String>, ()>, Halt> {
        self.fill(doc);
        Ok(Ok(Vec::new()))
    }
    fn prepare_fresh(&mut self, doc: &mut Value) -> Result<(), Halt> {
        self.fill(doc);
        Ok(())
    }
}

// The host's key: made once, at random.
fn new_key() -> String {
    xenonet_app::fsx::random_hex(16)
}

impl HostsStore {
    pub fn new(state_dir: &str, schemas: Arc<HostSchemas>) -> Self {
        HostsStore { state_dir: state_dir.into(), schemas, loaded: Mutex::new(None), life: Mutex::new(Lifecycle::new(now_ms())), dropped: Mutex::new(Vec::new()) }
    }

    pub fn file(&self) -> String {
        Path::new(&self.state_dir).join(config::HOSTS_FILE).to_string_lossy().into_owned()
    }

    pub fn ssh_config_file(&self) -> String {
        Path::new(&self.state_dir).join(config::SSH_CONFIG_FILE).to_string_lossy().into_owned()
    }

    // The document, read the first time it's asked for.
    fn with<T>(&self, f: impl FnOnce(&mut Doc) -> T) -> T {
        let mut slot = self.loaded.locked();
        if slot.is_none() {
            let mut hooks = Hooks { schemas: &self.schemas, dropped: Vec::new() };
            let (doc, report) =
                read_versioned(&self.file(), 1.0, now_ms(), &mut hooks).unwrap_or_else(|Halt| (json!({ "schema": "host/list/v1", "self": null, "hosts": [] }), Report::default()));
            *self.dropped.locked() = hooks.dropped;
            let mut life = self.life.locked();
            life.count("loads", 1.0);
            match &report.problem {
                Some(p) => life.problem(p, now_ms()),
                None => life.ok(now_ms()),
            }
            drop(life);
            *slot = Some(Doc { doc, report });
        }
        match slot.as_mut() {
            Some(d) => f(d),
            None => f(&mut Doc { doc: json!({ "schema": "host/list/v1", "self": null, "hosts": [] }), report: Report::default() }),
        }
    }

    fn save(&self, d: &Doc, file: &str) -> R<()> {
        if d.report.read_only {
            return Err(CoreError::status(409, format!("XenoNet can't change {file}: {}", d.report.problem.clone().unwrap_or_default())));
        }
        let wrote = write_atomic(file, &xenonet_base::json::stringify(&d.doc, 2));
        let mut life = self.life.locked();
        match wrote {
            Ok(()) => {
                life.count("saves", 1.0);
                life.ok(now_ms());
                Ok(())
            }
            Err(e) => {
                life.problem(&e.message, now_ms());
                Err(CoreError::new(format!("couldn't save its hosts to {file}: {}", e.message)))
            }
        }
    }

    /// The block's health, as core/blocks/lifecycle.js's health() with its
    /// name and kind.
    pub fn health(&self) -> Value {
        let life = self.life.locked();
        let counters: serde_json::Map<String, Value> = life.counters.iter().map(|(k, n)| (k.to_string(), xenonet_base::json::number(n))).collect();
        json!({
            "name": "hosts",
            "kind": "store",
            "state": life.state.as_str(),
            "since": xenonet_base::json::number(life.since),
            "lastOk": life.last_ok.map(xenonet_base::json::number),
            "lastError": life.last_error.as_ref().map(|e| json!({ "at": xenonet_base::json::number(e.at), "message": e.message })),
            "counters": counters,
            "budget": null,
        })
    }

    /// The store's load report, for the doctor and the hub's health.
    pub fn read_only(&self) -> bool {
        self.with(|d| d.report.read_only)
    }

    /// This XenoNet as a host: the name it gave itself and its key, made
    /// the first time anything asks.
    pub fn self_entry(&self) -> Value {
        let file = self.file();
        self.with(|d| {
            if d.doc.get("self").is_none_or(Value::is_null) {
                d.doc["self"] = json!({ "id": null, "key": new_key(), "createdAt": xenonet_base::json::number(now_ms()) });
                // A read-only file: the key lasts until XenoNet quits.
                let _ = self.save(d, &file);
            }
            d.doc["self"].clone()
        })
    }

    pub fn set_self_id(&self, id: &Value) -> R<Value> {
        config::check_self_id(Some(id)).map_err(refused)?;
        self.self_entry();
        let file = self.file();
        self.with(|d| {
            d.doc["self"]["id"] = id.clone();
            self.save(d, &file)
        })?;
        Ok(self.self_entry())
    }

    pub fn list(&self) -> Vec<Value> {
        self.with(|d| d.doc["hosts"].as_array().cloned().unwrap_or_default())
    }

    pub fn entry(&self, input: &Value) -> R<Value> {
        config::host_entry(input, now_ms(), &mut |e| self.schemas.check_entry(e).map_or(Ok(()), Err)).map_err(refused)
    }

    pub fn add(&self, input: &Value, replace: bool) -> R<Value> {
        let entry = self.entry(input)?;
        let file = self.file();
        self.with(|d| {
            let hosts = d.doc["hosts"].as_array().cloned().unwrap_or_default();
            d.doc["hosts"] = Value::Array(config::add(&hosts, &entry, replace).map_err(refused)?);
            self.save(d, &file)
        })?;
        self.write_ssh_config()?;
        Ok(entry)
    }

    pub fn remove(&self, id: &str) -> R<Option<Value>> {
        let file = self.file();
        let gone = self.with(|d| {
            let Some(hosts) = d.doc["hosts"].as_array_mut() else { return Ok(None) };
            let Some(at) = hosts.iter().position(|h| h["id"] == json!(id)) else { return Ok(None) };
            let gone = hosts.remove(at);
            self.save(d, &file).map(|()| Some(gone))
        })?;
        if gone.is_some() {
            self.write_ssh_config()?;
        }
        Ok(gone)
    }

    /// What a link learned of its host: its key, and what it calls itself.
    pub fn learn(&self, id: &str, key: Option<&Value>, name: Option<&Value>) -> R<Option<Value>> {
        let file = self.file();
        self.with(|d| {
            let Some(hosts) = d.doc["hosts"].as_array_mut() else { return Ok(None) };
            let Some(at) = hosts.iter().position(|h| h["id"] == json!(id)) else { return Ok(None) };
            hosts[at] = config::learn(&hosts[at], key, name);
            let out = hosts[at].clone();
            self.save(d, &file).map(|()| Some(out))
        })
    }

    /// Writes XenoNet's ssh config from the list, readable by you alone
    /// (ssh refuses a config others can write).
    pub fn write_ssh_config(&self) -> R<String> {
        let file = self.ssh_config_file();
        let hosts = self.list();
        let text = config::ssh_config_text(&hosts, &mut |h| self.schemas.check_entry(h).is_none());
        let fail = |e: std::io::Error, what: &str| CoreError::new(format!("{what} {file}: {e}"));
        if let Some(dir) = Path::new(&file).parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(e, "couldn't make the folder of"))?;
        }
        let tmp = format!("{file}.tmp");
        std::fs::write(&tmp, text).map_err(|e| fail(e, "couldn't write"))?;
        let mut run = |f: &str, a: &[&str]| xenonet_base::private_file::run_private(f, a);
        xenonet_base::private_file::protect_file(&tmp, cfg!(windows), &mut run).map_err(|e| CoreError::new(e.message().to_string()))?;
        std::fs::rename(&tmp, &file).map_err(|e| fail(e, "couldn't put in place"))?;
        Ok(file)
    }
}

/// The HostsConfig trait, for the routes.
pub struct Hosts(pub Arc<HostsStore>);

impl HostsConfig for Hosts {
    fn self_info(&self) -> R<Value> {
        Ok(self.0.self_entry())
    }

    fn set_self(&self, id: &Value) -> R<Value> {
        self.0.set_self_id(id)
    }

    fn ssh_config(&self) -> R<Value> {
        Ok(json!(self.0.ssh_config_file()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (HostsStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-hosts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (HostsStore::new(&dir.to_string_lossy(), Arc::new(HostSchemas::new())), dir)
    }

    // test/hosts-config.test.js's cases: the key made once, a name, an
    // entry added and its ssh config, and one taken away.
    #[test]
    fn a_host_and_its_hosts() {
        let (s, dir) = store("one");
        let me = s.self_entry();
        assert!(me["key"].as_str().is_some_and(|k| k.len() == 32));
        assert_eq!(s.self_entry()["key"], me["key"], "made once");
        assert_eq!(s.set_self_id(&json!("lab")).unwrap()["id"], "lab");
        assert_eq!(s.set_self_id(&json!("Not A Name")).unwrap_err().status, Some(400));
        let e = s.add(&json!({ "id": "studio", "target": "nova@studio.local", "port": 47840 }), false).unwrap();
        assert_eq!((e["id"].clone(), e["transport"].clone()), (json!("studio"), json!("ssh")));
        assert!(s.add(&json!({ "id": "studio", "target": "nova@studio.local" }), false).is_err(), "once, unless replaced");
        let text = std::fs::read_to_string(s.ssh_config_file()).unwrap();
        assert!(text.contains("Host studio"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(s.ssh_config_file()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(s.learn("studio", Some(&json!("0123456789abcdef0123456789abcdef")), None).unwrap().unwrap()["key"], "0123456789abcdef0123456789abcdef");
        // Read again from the file, as the next start would.
        let again = HostsStore::new(&dir.to_string_lossy(), Arc::new(HostSchemas::new()));
        assert_eq!(again.list().len(), 1);
        assert_eq!(again.self_entry()["id"], "lab");
        assert!(again.remove("studio").unwrap().is_some());
        assert!(again.list().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_it_cant_use_is_moved_aside() {
        let (s, dir) = store("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hosts.json"), "{ nope").unwrap();
        assert!(s.list().is_empty());
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("hosts.broken-")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
