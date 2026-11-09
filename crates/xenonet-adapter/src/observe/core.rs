//! What every observer of an actor product shares
//! (docs/design/actors/interface.md, sections 1 and 7). An observer keeps
//! its product's actors in the registry the way section 1 has it:
//!
//!   keep(id, …)       the actor itself, made once and kept current; a
//!                     registered actor's name, heartbeat and capabilities
//!                     are what you allowed, so the observer only adds how
//!                     it knows the actor; and whether its product is
//!                     installed
//!   say(id, …)        what it sees of it (presence and status), said when
//!                     it changes, and every 30 s besides while it's there;
//!                     a registered actor says its own status
//!   gather(id, …)     what it did, gathered per minute, each minute
//!                     recorded once (happened), even across a restart
//!   touch(id, …)      the places it works in, at most once a minute

use super::attribution::Ordered;
use super::{text, text_or_empty, ObserverHost, Result};
use crate::products::Product;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use xenonet_base::json::{number, number_string, stringify};
use xenonet_base::jsstr::hex;
use xenonet_base::sync::Locked;
use xenonet_base::text::truthy;

pub const LOOK_MS: f64 = 5000.0;
pub const KEEPALIVE_MS: f64 = 30e3;
const TOUCH_MS: f64 = 60e3;
const MINUTE_MS: f64 = 60e3;
// A minute's events wait this long past its end for rows written late.
const GRACE_MS: f64 = 15e3;
const WEEK_MS: f64 = 7.0 * 24.0 * 3600e3;
const RECORDED_MAX: usize = 4096;

/// What a registered actor may be allowed to do, in the order the
/// registry lists them; an observer's actor gets the ones its product has.
pub const CAPABILITIES: [&str; 8] = ["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents", "schedule", "browse", "use-computer"];

// The presences an actor is heard from in; offline and unknown are said
// once.
fn there(p: &str) -> bool {
    matches!(p, "working" | "idle" | "away")
}

// A short, stable name for a place in an event's ref.
fn tag(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))[..8].to_string()
}

/// The actor an observer keeps: its name, its product's version when
/// read, how the observer knows it, its heartbeat if any, what it can do,
/// whether its product is installed (when the observer can say), and when.
#[derive(Clone, Debug)]
pub struct Keep {
    pub name: String,
    pub version: Option<String>,
    pub via: Vec<&'static str>,
    pub heartbeat_ms: Option<f64>,
    pub capabilities: Vec<&'static str>,
    pub installed: Option<bool>,
    pub at: f64,
}

/// What an observer sees of an actor: its presence, a status in XenoNet's
/// words (None for none), the usage its own records report, and when.
#[derive(Clone, Debug)]
pub struct Say {
    pub presence: &'static str,
    pub status: Option<String>,
    pub usage: Option<Value>,
    pub at: f64,
}

/// Something an actor did, to be gathered with the rest of its minute:
/// the kind, the place (null for none), when, how many (one if unsaid),
/// the files, the messaging platform and the subject when they apply.
#[derive(Clone, Debug)]
pub struct Gather {
    pub kind: &'static str,
    pub target: Value,
    pub at: f64,
    pub count: Option<f64>,
    pub paths: Vec<String>,
    pub platform: Option<String>,
    pub subject: Option<&'static str>,
}

impl Gather {
    pub fn new(kind: &'static str, target: Value, at: f64) -> Self {
        Gather { kind, target, at, count: None, paths: Vec::new(), platform: None, subject: None }
    }
}

#[derive(Clone, Debug)]
struct Said {
    presence: String,
    /// The status said, None for a registered actor (which says its own).
    status: Option<Value>,
    /// The usage it last said, as JSON, to tell a change.
    usage: Option<String>,
    at: f64,
}

#[derive(Clone, Debug)]
struct Bucket {
    id: String,
    kind: &'static str,
    target: Value,
    platform: Option<String>,
    subject: Option<&'static str>,
    minute: f64,
    count: f64,
    paths: Vec<String>,
    at: f64,
}

#[derive(Default)]
struct State {
    said: Vec<(String, Said)>,
    touched: Vec<(String, f64)>,
    recorded: Ordered,
    buckets: Vec<(String, Bucket)>,
}

/// An observer's shared state, never held across a call on the host.
pub struct Core {
    product: &'static Product,
    state: Mutex<State>,
}

impl Core {
    pub fn new(product: &'static Product) -> Self {
        Core { product, state: Mutex::new(State::default()) }
    }

    pub fn product(&self) -> &'static Product {
        self.product
    }

    fn s(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.locked()
    }

    /// The actor itself, made once and kept current.
    pub fn keep(&self, h: &dyn ObserverHost, id: &str, k: &Keep) -> Result<()> {
        let caps: Vec<Value> = CAPABILITIES.iter().filter(|c| k.capabilities.contains(c)).map(|c| json!(c)).collect();
        let version = k.version.clone().map_or(Value::Null, Value::String);
        let product = |version: Value| json!({ "id": self.product.id, "vendor": self.product.vendor, "version": version });
        let actors = h.actors();
        let info = actors.info(id);
        if !truthy(Some(&info)) {
            // Made again (you removed it, say): nothing said of the old one
            // counts.
            {
                let mut s = self.s();
                s.said.retain(|(x, _)| x != id);
                let prefix = format!("{id}|");
                s.touched.retain(|(x, _)| !x.starts_with(&prefix));
            }
            let def = json!({
                "id": id,
                "kind": "agent",
                "name": k.name,
                "product": product(version),
                "via": k.via,
                "heartbeatMs": k.heartbeat_ms.map_or(Value::Null, number),
                "capabilities": caps,
                "identity": null,
                "enabled": h.observing(),
                "at": number(k.at),
            });
            actors.upsert(&def)?;
            if let Some(installed) = k.installed {
                actors.set_installed(id, installed)?;
            }
            return Ok(());
        }
        if let Some(installed) = k.installed {
            actors.set_installed(id, installed)?;
        }
        let registered = actors.is_registered(id);
        let f = |k: &str| info.get(k).cloned();
        let fields = ["name", "product", "via", "heartbeatMs", "capabilities", "identity"];
        let was: Map<String, Value> = fields.iter().filter_map(|k| Some(((*k).to_string(), f(k)?))).collect();
        // A version XenoNet can't read just now (an update under way) isn't
        // a version gone.
        let version = if version.is_null() { f("product").and_then(|p| p.get("version").cloned()).unwrap_or(Value::Null) } else { version };
        let had: Vec<Value> = f("via").and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let mut via = had.clone();
        for v in &k.via {
            let v = json!(v);
            if !had.contains(&v) {
                via.push(v);
            }
        }
        let mut next = Map::new();
        let mut put = |key: &str, v: Option<Value>| {
            if let Some(v) = v {
                next.insert(key.into(), v);
            }
        };
        put("name", if registered { f("name") } else { Some(json!(k.name)) });
        put("product", Some(product(version)));
        put("via", Some(Value::Array(via)));
        put("heartbeatMs", if registered { f("heartbeatMs") } else { Some(k.heartbeat_ms.map_or(Value::Null, number)) });
        put("capabilities", if registered { f("capabilities") } else { Some(Value::Array(caps)) });
        put("identity", f("identity"));
        if stringify(&Value::Object(next.clone()), 0) != stringify(&Value::Object(was), 0) {
            let mut def = Map::new();
            def.insert("id".into(), json!(id));
            def.insert("kind".into(), json!("agent"));
            def.extend(next);
            def.insert("at".into(), number(k.at));
            actors.upsert(&Value::Object(def))?;
        }
        Ok(())
    }

    /// Presence, a status in XenoNet's words and the usage its own records
    /// report. Whether it said it.
    pub fn say(&self, h: &dyn ObserverHost, id: &str, s: &Say) -> Result<bool> {
        let actors = h.actors();
        let usage_key = s.usage.as_ref().map(|u| stringify(u, 0));
        if !actors.has(id) {
            return Ok(false);
        }
        if !there(s.presence) && !actors.presence(id).as_deref().is_some_and(there) && actors.installed(id) == Some(false) {
            return Ok(false);
        }
        let line: Option<Value> = if actors.is_registered(id) { None } else { Some(s.status.clone().map_or(Value::Null, Value::String)) };
        if let Some((_, last)) = self.s().said.iter().find(|(k, _)| k == id) {
            if last.presence == s.presence && last.status == line && last.usage == usage_key && (!there(s.presence) || s.at - last.at < KEEPALIVE_MS) {
                return Ok(false);
            }
        }
        let mut r = Map::new();
        r.insert("instance".into(), json!("observed"));
        r.insert("presence".into(), json!(s.presence));
        if let Some(l) = &line {
            r.insert("status".into(), l.clone());
        }
        if let Some(u) = &s.usage {
            r.insert("usage".into(), u.clone());
        }
        r.insert("via".into(), json!("observed"));
        r.insert("at".into(), number(s.at));
        actors.report(id, &Value::Object(r))?;
        let said = Said { presence: s.presence.into(), status: line, usage: usage_key, at: s.at };
        let mut st = self.s();
        match st.said.iter_mut().find(|(k, _)| k == id) {
            Some(slot) => slot.1 = said,
            None => st.said.push((id.into(), said)),
        }
        Ok(true)
    }

    /// An event, recorded once by its `ref` (`e`: ref, kind, target, via,
    /// at, and its facts). The event, or null.
    pub fn happened(&self, h: &dyn ObserverHost, id: &str, e: &Value) -> Result<Value> {
        let Some(fields) = e.as_object() else { return Ok(Value::Null) };
        let get = |k: &str| fields.get(k).cloned();
        let target = get("target").unwrap_or(Value::Null);
        let via = get("via").unwrap_or_else(|| json!("observed"));
        let at = get("at").and_then(|v| v.as_f64()).unwrap_or_else(|| h.now());
        let actors = h.actors();
        // One removed meanwhile is left alone until the observer makes it
        // again.
        if !actors.has(id) {
            return Ok(Value::Null);
        }
        if via.as_str() != Some("attributed") && actors.is_registered(id) {
            return Ok(Value::Null);
        }
        let r = get("ref").unwrap_or(Value::Null);
        let r_text = text(&r);
        let key = format!("{id}|{r_text}");
        {
            let mut s = self.s();
            if s.recorded.has(&key) {
                return Ok(Value::Null);
            }
            s.recorded.add(key);
            s.recorded.trim_to(RECORDED_MAX);
        }
        let week = h.now() - WEEK_MS;
        if actors.has_ref(id, &r_text, week)? {
            return Ok(Value::Null);
        }
        let mut out = Map::new();
        out.insert("kind".into(), get("kind").unwrap_or(Value::Null));
        out.insert("target".into(), target);
        out.insert("via".into(), via);
        out.insert("at".into(), number(at));
        out.insert("ref".into(), r);
        for (k, v) in fields {
            if !matches!(k.as_str(), "ref" | "kind" | "target" | "via" | "at") {
                out.insert(k.clone(), v.clone());
            }
        }
        actors.record(id, &Value::Object(out))
    }

    /// Something it did, gathered with the rest of its minute of the same
    /// kind in the same place.
    pub fn gather(&self, id: &str, g: &Gather) {
        let minute = (g.at / MINUTE_MS).floor();
        let key = format!("{id}|{}|{}|{}|{}", g.kind, text_or_empty(&g.target), g.platform.as_deref().unwrap_or(""), number_string(minute));
        let count = g.count.unwrap_or(1.0);
        let mut s = self.s();
        let i = match s.buckets.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                s.buckets.push((
                    key,
                    Bucket { id: id.into(), kind: g.kind, target: g.target.clone(), platform: g.platform.clone(), subject: g.subject, minute, count: 0.0, paths: vec![], at: g.at },
                ));
                s.buckets.len() - 1
            }
        };
        let b = &mut s.buckets[i].1;
        b.count += count;
        b.at = if b.at.is_nan() || g.at.is_nan() { f64::NAN } else { b.at.max(g.at) };
        for p in &g.paths {
            if b.paths.len() < 20 && !b.paths.contains(p) {
                b.paths.push(p.clone());
            }
        }
    }

    /// The minutes that are over, recorded.
    pub fn flush(&self, h: &dyn ObserverHost, now: f64) -> Result<()> {
        let keys: Vec<String> = self.s().buckets.iter().map(|(k, _)| k.clone()).collect();
        for key in keys {
            let b = {
                let mut s = self.s();
                let Some(i) = s.buckets.iter().position(|(k, _)| *k == key) else { continue };
                if now < (s.buckets[i].1.minute + 1.0) * MINUTE_MS + GRACE_MS {
                    continue;
                }
                s.buckets.remove(i).1
            };
            let r = format!("o:{}:{}:{}", b.kind, number_string(b.minute), tag(&format!("{}|{}", text_or_empty(&b.target), b.platform.as_deref().unwrap_or(""))));
            let mut o = Map::new();
            o.insert("ref".into(), json!(r));
            o.insert("kind".into(), json!(b.kind));
            o.insert("target".into(), b.target);
            o.insert("at".into(), number(b.at));
            o.insert("count".into(), number(b.count));
            if !b.paths.is_empty() {
                o.insert("paths".into(), json!(b.paths));
            }
            if let Some(p) = b.platform.filter(|p| !p.is_empty()) {
                o.insert("platform".into(), json!(p));
            }
            if let Some(s) = b.subject.filter(|s| !s.is_empty()) {
                o.insert("subject".into(), json!(s));
            }
            self.happened(h, &b.id, &Value::Object(o))?;
        }
        Ok(())
    }

    /// A place or session it acts on, touched at most once a minute.
    pub fn touch(&self, h: &dyn ObserverHost, id: &str, node: &Value, relation: &str, at: f64) -> Result<()> {
        if !truthy(Some(node)) || !h.actors().has(id) {
            return Ok(());
        }
        let node = text(node);
        let key = format!("{id}|{relation}|{node}");
        {
            let mut s = self.s();
            let last = s.touched.iter().find(|(k, _)| *k == key).map_or(f64::NEG_INFINITY, |(_, t)| *t);
            if at - last < TOUCH_MS {
                return Ok(());
            }
            match s.touched.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = at,
                None => s.touched.push((key, at)),
            }
            if s.touched.len() > RECORDED_MAX {
                s.touched.clear();
            }
        }
        h.actors().target(id, &node, relation, at)
    }
}
