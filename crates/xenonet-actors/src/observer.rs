//! What every observer of an actor product shares (core/actors/observer.js;
//! docs/design/actors/interface.md, sections 1 and 7). An observer keeps
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
//!
//! Its collaborators are the observer's own fields, as the JS read them:
//! "actors", "product", "clock", and its observing() (`Calls` on the
//! observer).

use crate::attribution::Ordered;
use crate::registration::CAPABILITIES;
use crate::tokens::hex;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use xenonet_base::sync::Locked;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;

pub const LOOK_MS: f64 = 5000.0;
pub const KEEPALIVE_MS: f64 = 30e3;
const TOUCH_MS: f64 = 60e3;
const MINUTE_MS: f64 = 60e3;
// A minute's events wait this long past its end for rows written late.
const GRACE_MS: f64 = 15e3;
const WEEK_MS: f64 = 7.0 * 24.0 * 3600e3;
const RECORDED_MAX: usize = 4096;

// The presences an actor is heard from in; offline and unknown are said
// once.
fn there(p: &Arg) -> bool {
    matches!(p.as_str(), Some("working" | "idle" | "away"))
}

// A short, stable name for a place in an event's ref.
fn tag(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))[..8].to_string()
}

fn text(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

#[derive(Clone, Debug)]
struct Said {
    presence: Arg,
    status: Arg,
    at: f64,
}

#[derive(Clone, Debug)]
struct Bucket {
    id: String,
    kind: Arg,
    target: Arg,
    platform: Arg,
    subject: Arg,
    minute: f64,
    count: f64,
    paths: Vec<Arg>,
    at: f64,
}

#[derive(Default)]
pub struct State {
    said: Vec<(String, Said)>,
    touched: Vec<(String, f64)>,
    recorded: Ordered,
    buckets: Vec<(String, Bucket)>,
}

/// An observer's state, shared by its calls; never held across a call to
/// a collaborator.
#[derive(Default)]
pub struct Observer {
    state: Mutex<State>,
}

fn now(c: &mut dyn Calls, at: &Arg) -> Result<f64> {
    Ok(match at {
        Arg::Undef => c.now_at("clock")?,
        a => a.as_f64().unwrap_or(f64::NAN),
    })
}

trait ClockAt {
    fn now_at(&mut self, path: &str) -> Result<f64>;
}

impl<T: Calls + ?Sized> ClockAt for T {
    fn now_at(&mut self, path: &str) -> Result<f64> {
        Ok(self.call(path, "now", vec![])?.as_f64().unwrap_or(f64::NAN))
    }
}

fn eq(a: &Arg, b: &Arg) -> bool {
    match (a, b) {
        (Arg::Num(x), Arg::Num(y)) => x == y,
        _ => a == b,
    }
}

impl Observer {
    fn s(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.locked()
    }

    /// The actor itself, made once and kept current (`o`: name, version,
    /// via, heartbeatMs, capabilities, installed, at).
    pub fn keep(&self, c: &mut dyn Calls, id: &str, o: &Arg) -> Result<()> {
        let version = match o.field("version") {
            Arg::Undef => Arg::Null,
            v => v,
        };
        let asked: Vec<Arg> = xenonet_engine::peer::list(match o.field("capabilities") {
            Arg::Undef => Arg::List(vec![]),
            v => v,
        });
        let caps: Vec<Arg> = CAPABILITIES.iter().filter(|c| asked.iter().any(|a| a.as_str() == Some(*c))).map(|c| Arg::text(*c)).collect();
        let at = now(c, &o.field("at"))?;
        let installed = o.field("installed");
        let product_id = c.get("product.id")?;
        let vendor = c.get("product.vendor")?;
        let info = c.call("actors", "info", vec![Arg::text(id)])?;
        if !info.truthy() {
            // Made again (you removed it, say): nothing said of the old one
            // counts.
            {
                let mut s = self.s();
                s.said.retain(|(k, _)| k != id);
                let prefix = format!("{id}|");
                s.touched.retain(|(k, _)| !k.starts_with(&prefix));
            }
            let product = Arg::Obj(vec![("id".into(), product_id), ("vendor".into(), vendor), ("version".into(), version)]);
            let enabled = c.call("", "observing", vec![])?;
            let def = Arg::Obj(vec![
                ("id".into(), Arg::text(id)),
                ("kind".into(), Arg::text("agent")),
                ("name".into(), o.field("name")),
                ("product".into(), product),
                ("via".into(), o.field("via")),
                ("heartbeatMs".into(), o.field("heartbeatMs")),
                ("capabilities".into(), Arg::List(caps)),
                ("identity".into(), Arg::Null),
                ("enabled".into(), enabled),
                ("at".into(), Arg::Num(at)),
            ]);
            c.call("actors", "upsert", vec![def])?;
            if installed != Arg::Undef {
                c.call("actors", "setInstalled", vec![Arg::text(id), installed])?;
            }
            return Ok(());
        }
        if installed != Arg::Undef {
            c.call("actors", "setInstalled", vec![Arg::text(id), installed])?;
        }
        let registered = c.call("actors", "isRegistered", vec![Arg::text(id)])?.truthy();
        let f = |k: &str| info.field(k);
        let was = Arg::Obj(vec![
            ("name".into(), f("name")),
            ("product".into(), f("product")),
            ("via".into(), f("via")),
            ("heartbeatMs".into(), f("heartbeatMs")),
            ("capabilities".into(), f("capabilities")),
            ("identity".into(), f("identity")),
        ]);
        // A version XenoNet can't read just now (an update under way) isn't
        // a version gone.
        let version = if version.is_nullish() { f("product").field("version").or(Arg::Null) } else { version };
        let had: Vec<Arg> = xenonet_engine::peer::list(f("via"));
        let mut via = had.clone();
        for v in xenonet_engine::peer::list(o.field("via")) {
            if !had.contains(&v) {
                via.push(v);
            }
        }
        let next = Arg::Obj(vec![
            ("name".into(), if registered { f("name") } else { o.field("name") }),
            ("product".into(), Arg::Obj(vec![("id".into(), product_id), ("vendor".into(), vendor), ("version".into(), version)])),
            ("via".into(), Arg::List(via)),
            ("heartbeatMs".into(), if registered { f("heartbeatMs") } else { o.field("heartbeatMs") }),
            ("capabilities".into(), if registered { f("capabilities") } else { Arg::List(caps) }),
            ("identity".into(), f("identity")),
        ]);
        let json = |a: &Arg| xenonet_base::json::stringify(&a.to_json(), 0);
        if json(&next) != json(&was) {
            let Arg::Obj(mut fields) = next else { unreachable!() };
            fields.insert(0, ("kind".into(), Arg::text("agent")));
            fields.insert(0, ("id".into(), Arg::text(id)));
            fields.push(("at".into(), Arg::Num(at)));
            c.call("actors", "upsert", vec![Arg::Obj(fields)])?;
        }
        Ok(())
    }

    /// Presence and a status in XenoNet's words (`o`: presence, status,
    /// at). Whether it said it.
    pub fn say(&self, c: &mut dyn Calls, id: &str, o: &Arg) -> Result<bool> {
        let presence = o.field("presence");
        let status = match o.field("status") {
            Arg::Undef => Arg::Null,
            v => v,
        };
        let at = now(c, &o.field("at"))?;
        if !c.call("actors", "has", vec![Arg::text(id)])?.truthy() {
            return Ok(false);
        }
        if !there(&presence) && !there(&c.call("actors", "presence", vec![Arg::text(id)])?) && c.call("actors", "installed", vec![Arg::text(id)])? == Arg::Bool(false) {
            return Ok(false);
        }
        let line = if c.call("actors", "isRegistered", vec![Arg::text(id)])?.truthy() { Arg::Undef } else { status };
        if let Some((_, last)) = self.s().said.iter().find(|(k, _)| k == id) {
            if eq(&last.presence, &presence) && eq(&last.status, &line) && (!there(&presence) || at - last.at < KEEPALIVE_MS) {
                return Ok(false);
            }
        }
        let mut r = vec![("instance".to_string(), Arg::text("observed")), ("presence".to_string(), presence.clone())];
        if line != Arg::Undef {
            r.push(("status".into(), line.clone()));
        }
        r.push(("via".into(), Arg::text("observed")));
        r.push(("at".into(), Arg::Num(at)));
        c.call("actors", "report", vec![Arg::text(id), Arg::Obj(r)])?;
        let said = Said { presence, status: line, at };
        let mut s = self.s();
        match s.said.iter_mut().find(|(k, _)| k == id) {
            Some(slot) => slot.1 = said,
            None => s.said.push((id.into(), said)),
        }
        Ok(true)
    }

    /// An event, recorded once by its `ref` (`o`: ref, kind, target, via,
    /// at, and its facts). The event, or null.
    pub fn happened(&self, c: &mut dyn Calls, id: &str, o: &Arg) -> Result<Arg> {
        let Arg::Obj(fields) = o else { return Ok(Arg::Null) };
        let get = |k: &str| o.field(k);
        let target = match get("target") {
            Arg::Undef => Arg::Null,
            v => v,
        };
        let via = match get("via") {
            Arg::Undef => Arg::text("observed"),
            v => v,
        };
        let at = now(c, &get("at"))?;
        // One removed meanwhile is left alone until the observer makes it
        // again.
        if !c.call("actors", "has", vec![Arg::text(id)])?.truthy() {
            return Ok(Arg::Null);
        }
        if via.as_str() != Some("attributed") && c.call("actors", "isRegistered", vec![Arg::text(id)])?.truthy() {
            return Ok(Arg::Null);
        }
        let r = get("ref");
        let key = format!("{id}|{}", text(&r));
        {
            let mut s = self.s();
            if s.recorded.has(&key) {
                return Ok(Arg::Null);
            }
            s.recorded.add(key);
            s.recorded.trim_to(RECORDED_MAX);
        }
        let week = c.now_at("clock")? - WEEK_MS;
        if c.call("actors.store", "hasActorRef", vec![Arg::text(id), r.clone(), Arg::Num(week)])?.truthy() {
            return Ok(Arg::Null);
        }
        let mut out = vec![("kind".to_string(), get("kind")), ("target".to_string(), target), ("via".to_string(), via), ("at".to_string(), Arg::Num(at)), ("ref".to_string(), r)];
        for (k, v) in fields {
            if !matches!(k.as_str(), "ref" | "kind" | "target" | "via" | "at") {
                out.push((k.clone(), v.clone()));
            }
        }
        c.call("actors", "record", vec![Arg::text(id), Arg::Obj(out)])
    }

    /// Something it did, gathered with the rest of its minute of the same
    /// kind in the same place (`o`: kind, target, at, count, paths,
    /// platform, subject).
    pub fn gather(&self, id: &str, o: &Arg) {
        let at = o.field("at").as_f64().unwrap_or(f64::NAN);
        let minute = (at / MINUTE_MS).floor();
        let target = match o.field("target") {
            Arg::Undef => Arg::Null,
            v => v,
        };
        let opt = |a: &Arg| if a.is_nullish() { String::new() } else { text(a) };
        let key = format!("{id}|{}|{}|{}|{}", text(&o.field("kind")), opt(&target), opt(&o.field("platform")), xenonet_base::json::number_string(minute));
        let count = match o.field("count") {
            Arg::Undef => 1.0,
            v => v.as_f64().unwrap_or(f64::NAN),
        };
        let paths = xenonet_engine::peer::list(match o.field("paths") {
            Arg::Undef => Arg::List(vec![]),
            v => v,
        });
        let mut s = self.s();
        let i = match s.buckets.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                s.buckets.push((
                    key,
                    Bucket { id: id.into(), kind: o.field("kind"), target, platform: o.field("platform"), subject: o.field("subject"), minute, count: 0.0, paths: vec![], at },
                ));
                s.buckets.len() - 1
            }
        };
        let b = &mut s.buckets[i].1;
        b.count += count;
        b.at = if b.at.is_nan() || at.is_nan() { f64::NAN } else { b.at.max(at) };
        for p in paths {
            if b.paths.len() < 20 && !b.paths.contains(&p) {
                b.paths.push(p);
            }
        }
    }

    /// The minutes that are over, recorded.
    pub fn flush(&self, c: &mut dyn Calls, now_at: &Arg) -> Result<()> {
        let now = now(c, now_at)?;
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
            let opt = |a: &Arg| if a.is_nullish() { String::new() } else { text(a) };
            let r = format!("o:{}:{}:{}", text(&b.kind), xenonet_base::json::number_string(b.minute), tag(&format!("{}|{}", opt(&b.target), opt(&b.platform))));
            let mut o = vec![
                ("ref".to_string(), Arg::text(r)),
                ("kind".to_string(), b.kind),
                ("target".to_string(), b.target),
                ("at".to_string(), Arg::Num(b.at)),
                ("count".to_string(), Arg::Num(b.count)),
            ];
            if !b.paths.is_empty() {
                o.push(("paths".into(), Arg::List(b.paths)));
            }
            if b.platform.truthy() {
                o.push(("platform".into(), b.platform));
            }
            if b.subject.truthy() {
                o.push(("subject".into(), b.subject));
            }
            self.happened(c, &b.id, &Arg::Obj(o))?;
        }
        Ok(())
    }

    /// A place or session it acts on, touched at most once a minute.
    pub fn touch(&self, c: &mut dyn Calls, id: &str, node: &Arg, relation: &Arg, at: &Arg) -> Result<()> {
        let at = now(c, at)?;
        if !node.truthy() || !c.call("actors", "has", vec![Arg::text(id)])?.truthy() {
            return Ok(());
        }
        let key = format!("{id}|{}|{}", text(relation), text(node));
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
        c.call("actors", "target", vec![Arg::text(id), node.clone(), relation.clone(), Arg::Num(at)])?;
        Ok(())
    }

    /// Whether to look now: its observer is on, and the map is there (before
    /// XenoNet's first look at it, no place could be named).
    pub fn may_look(c: &mut dyn Calls) -> Result<bool> {
        if !c.call("", "observing", vec![])?.truthy() {
            return Ok(false);
        }
        Ok(xenonet_engine::peer::opt_call(c, "actors.map", "ready", vec![])? != Arg::Bool(false))
    }

    /// One look: the kind's own look(at), then the minutes over recorded,
    /// and the block ok; a look that throws leaves it degraded.
    pub fn tick(&self, c: &mut dyn Calls, at: f64) -> Result<bool> {
        match c.call("", "look", vec![Arg::Num(at)]) {
            Ok(_) => match self.flush(c, &Arg::Num(at)) {
                Ok(()) => {
                    c.call("", "ok", vec![Arg::Num(at)])?;
                }
                Err(e) => {
                    c.call("", "problem", vec![Arg::text(e.message), Arg::Num(at)])?;
                }
            },
            Err(e) => {
                c.call("", "problem", vec![Arg::text(e.message), Arg::Num(at)])?;
            }
        }
        Ok(true)
    }
}
