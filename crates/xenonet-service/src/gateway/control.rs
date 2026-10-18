//! The actor gateway's switch, its tunnel and how it stands
//! (server/gateway-control.js): whether it's on, which tunnel puts it on the
//! internet (none is assumed), the address that gives it, whether that
//! address reaches it, and how each actor's connection is doing. Kept in the
//! engine's own database, so all of it comes back as it was when XenoNet
//! starts again. Changing it is the owner's (the actors' routes check that).
//!
//! While it's on, XenoNet checks the address every ten minutes (every two
//! while it fails, and at once after the computer wakes), and sets the
//! tunnel up again when it went. The address can change under it; each
//! actor's last call says which address it used, so the window can say
//! which apps still have an old one.

use super::actor_gateway::{normal_host, ActorGateway, GatewayCores, GatewayHooks};
use super::reach::{check_reach, Network};
use super::tunnel::{kind, kind_ids, tunnel_name, Tunnel, TunnelHooks, TunnelHost};
use crate::cores::{Actors, CoreError, EngineStore, Kind, R};
use crate::window_key::random_hex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_base::json;

const KEY: &str = "gateway";
const SEEN: &str = "gateway.seen";
const TICK: Duration = Duration::from_secs(60);
const OK_EVERY_MS: f64 = 10.0 * 60e3;
const FAILING_EVERY_MS: f64 = 2.0 * 60e3;
// An actor's last call is written down when its address changes, or at most
// once an hour.
const SEEN_SAVE_MS: f64 = 3600e3;
// A gateway token that goes unused this much longer stops working: the
// window says so first.
pub const EXPIRING_MS: f64 = 7.0 * 24.0 * 3600e3;
// core/actors/registration.js's TOKEN_UNUSED_MS.
const TOKEN_UNUSED_MS: f64 = 30.0 * 24.0 * 3600e3;

fn bad(message: &str) -> CoreError {
    CoreError::of(Kind::Error, 400, "bad-request", message)
}

/// { on, host, tunnel: { kind, name, host } } as saved.
#[derive(Clone, Debug, Default)]
struct Saved {
    on: bool,
    host: Option<String>,
    tunnel: Option<(String, Option<String>, Option<String>)>,
}

impl Saved {
    fn to_json(&self) -> Value {
        json!({ "on": self.on, "host": self.host, "tunnel": self.tunnel.as_ref().map(|(k, n, h)| json!({ "kind": k, "name": n, "host": h })) })
    }
}

pub struct ControlParts {
    pub cores: GatewayCores,
    pub store: Arc<dyn EngineStore>,
    /// After a run an actor reported: the map refreshed, and the actor's
    /// beam to it.
    pub on_run: Arc<dyn Fn(&str, &str) + Send + Sync>,
    pub port: u16,
    pub version: String,
    pub dir: Option<String>,
    pub home: String,
    pub probe_local: bool,
    pub log: Arc<dyn Fn(&str, &str) + Send + Sync>,
    pub tunnel_host: Arc<dyn TunnelHost>,
    pub network: Arc<dyn Network>,
    pub now: Arc<dyn Fn() -> f64 + Send + Sync>,
}

pub struct GatewayControl {
    store: Arc<dyn EngineStore>,
    actors: Arc<dyn Actors>,
    port: u16,
    probe_local: bool,
    log: Arc<dyn Fn(&str, &str) + Send + Sync>,
    network: Arc<dyn Network>,
    now: Arc<dyn Fn() -> f64 + Send + Sync>,
    gateway: OnceLock<ActorGateway>,
    tunnel: OnceLock<Tunnel>,
    seen: Mutex<Map<String, Value>>,
    refused: Mutex<HashMap<String, Value>>,
    problem: Mutex<Option<String>>,
    reach: Mutex<Value>,
    checking: tokio::sync::Mutex<()>,
    soon: Mutex<Option<tokio::task::JoinHandle<()>>>,
    ticker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    last_tick: Mutex<f64>,
}

impl GatewayControl {
    pub fn new(parts: ControlParts) -> Arc<GatewayControl> {
        let seen = parts.store.meta(SEEN).ok().flatten().and_then(|s| json::parse(&s).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        let me = Arc::new(GatewayControl {
            store: parts.store.clone(),
            actors: parts.cores.actors.clone(),
            port: parts.port,
            probe_local: parts.probe_local,
            log: parts.log.clone(),
            network: parts.network.clone(),
            now: parts.now.clone(),
            gateway: OnceLock::new(),
            tunnel: OnceLock::new(),
            seen: Mutex::new(seen),
            refused: Mutex::new(HashMap::new()),
            problem: Mutex::new(None),
            reach: Mutex::new(json!({ "state": "unknown", "stage": null, "why": null, "at": null, "okAt": null })),
            checking: tokio::sync::Mutex::new(()),
            soon: Mutex::new(None),
            ticker: Mutex::new(None),
            last_tick: Mutex::new((parts.now)()),
        });
        let weak = Arc::downgrade(&me);
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
        let log = parts.log.clone();
        let hooks = GatewayHooks {
            on_call: Arc::new(move |id, slug, host, at| {
                if let Some(c) = w1.upgrade() {
                    c.called(slug, id, host, at);
                }
            }),
            on_refused: Arc::new(move |actor, code, reason, at| {
                if let (Some(c), Some(actor)) = (w2.upgrade(), actor) {
                    c.refused.lock().unwrap().insert(actor.to_string(), json!({ "code": code, "reason": reason, "at": json::number(at) }));
                }
            }),
            on_run: parts.on_run.clone(),
            log: Arc::new(move |code, detail| log(code, detail)),
        };
        let host = me.saved().host;
        let _ = me.gateway.set(ActorGateway::with_clock(parts.cores, hooks, parts.port, host.as_deref(), &parts.version, parts.now.clone()));
        let tlog = parts.log.clone();
        let w4 = weak.clone();
        let thooks = TunnelHooks {
            on_host: Arc::new(move |h| {
                if let Some(c) = w3.upgrade() {
                    c.learned(h);
                }
            }),
            on_ready: Arc::new(move || {
                if let Some(c) = w4.upgrade() {
                    c.soon(1500);
                }
            }),
            on_change: Arc::new(|| {}),
            log: Arc::new(move |code, detail| tlog(code, detail)),
        };
        let _ = me.tunnel.set(Tunnel::new(parts.port, parts.dir.as_deref(), &parts.home, parts.tunnel_host, thooks));
        let _ = weak;
        me
    }

    pub fn gateway(&self) -> &ActorGateway {
        self.gateway.get().expect("made in new")
    }

    pub fn tunnel(&self) -> &Tunnel {
        self.tunnel.get().expect("made in new")
    }

    // A setting from before tunnels were picked here, a host alone, is the
    // owner's own tunnel.
    fn saved(&self) -> Saved {
        let s = self.store.meta(KEY).ok().flatten().and_then(|t| json::parse(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
        let host = normal_host(s["host"].as_str());
        let k = s["tunnel"]["kind"].as_str().filter(|k| kind(k).is_some()).map(str::to_string).or_else(|| host.as_ref().map(|_| "own".to_string()));
        let tunnel = k.map(|k| {
            let name = s["tunnel"]["name"].as_str().filter(|n| tunnel_name(n)).map(str::to_string);
            let given = normal_host(s["tunnel"]["host"].as_str()).or_else(|| if k == "own" { host.clone() } else { None });
            (k, name, given)
        });
        Saved { on: s["on"] == json!(true), host, tunnel }
    }

    fn save(&self, s: &Saved) -> R<()> {
        self.store.set_meta(KEY, &json::stringify(&s.to_json(), 0))
    }

    fn called(&self, slug: &str, id: &str, host: Option<&str>, at: f64) {
        self.refused.lock().unwrap().remove(id);
        let mut seen = self.seen.lock().unwrap();
        let had = seen.get(slug).cloned();
        let host = host.map(|h| json!(h)).or_else(|| had.as_ref().map(|h| h["host"].clone())).unwrap_or(Value::Null);
        let saved_at = had.as_ref().and_then(|h| h["savedAt"].as_f64()).unwrap_or(0.0);
        let mut next = json!({ "host": host, "at": json::number(at), "savedAt": json::number(saved_at) });
        if had.is_none() || had.as_ref().map(|h| &h["host"]) != Some(&next["host"]) || at - saved_at >= SEEN_SAVE_MS {
            next["savedAt"] = json::number(at);
            seen.insert(slug.to_string(), next);
            let _ = self.store.set_meta(SEEN, &json::stringify(&Value::Object(seen.clone()), 0));
        } else {
            seen.insert(slug.to_string(), next);
        }
    }

    // The address a tunnel got (Tailscale's name, a quick tunnel's, ngrok's).
    fn learned(self: &Arc<Self>, name: &str) {
        let Some(h) = normal_host(Some(name)) else { return };
        let mut s = self.saved();
        if s.host.as_deref() == Some(h.as_str()) {
            return;
        }
        s.host = Some(h.clone());
        let _ = self.save(&s);
        self.gateway().set_host(Some(&h));
        (self.log)("gateway.address", &format!("the gateway's address is now {h}"));
        self.soon(3000);
    }

    pub async fn check(&self) -> Value {
        // One at a time: a check already under way is the answer.
        if let Ok(_held) = self.checking.try_lock() {
            self.check_now().await;
        } else {
            let _wait = self.checking.lock().await;
        }
        self.reach.lock().unwrap().clone()
    }

    async fn check_now(&self) {
        let s = self.saved();
        if !s.on || !self.gateway().listening() {
            let mut r = self.reach.lock().unwrap();
            r["state"] = json!("off");
            r["stage"] = Value::Null;
            r["why"] = Value::Null;
            return;
        }
        let before = self.reach.lock().unwrap()["state"].clone();
        self.reach.lock().unwrap()["state"] = json!("checking");
        let g = self.gateway().clone();
        let got = check_reach(s.host.as_deref(), &random_hex(16), |w| g.expect_probe(w), self.probe_local.then_some(self.port), self.network.as_ref()).await;
        let t = (self.now)();
        let ok = got["ok"] == json!(true);
        let mut r = self.reach.lock().unwrap();
        let ok_at = if ok { json::number(t) } else { r["okAt"].clone() };
        *r = json!({ "state": if ok { "ok" } else { "failing" }, "stage": got["stage"], "why": got["why"], "at": json::number(t), "okAt": ok_at, "local": self.probe_local });
        if before != r["state"] {
            let words = if ok { format!("the gateway is reachable at {}", s.host.unwrap_or_default()) } else { xenonet_base::text::js_string(&got["why"]) };
            (self.log)("gateway.reach", &words);
        }
    }

    fn soon(self: &Arc<Self>, ms: u64) {
        let me = self.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            me.check().await;
        });
        if let Some(old) = self.soon.lock().unwrap().replace(task) {
            old.abort();
        }
    }

    // Every minute: a check when one is due, or at once after the computer
    // slept (the clock jumped), with the tunnel set up again first.
    async fn tick(self: &Arc<Self>) {
        let t = (self.now)();
        let woke = {
            let mut last = self.last_tick.lock().unwrap();
            let woke = t - *last > 3.0 * TICK.as_millis() as f64;
            *last = t;
            woke
        };
        let s = self.saved();
        if !s.on || self.checking.try_lock().is_err() {
            return;
        }
        let (at, state) = {
            let r = self.reach.lock().unwrap();
            (r["at"].as_f64(), r["state"].clone())
        };
        let due = woke || at.is_none() || t - at.unwrap() >= if state == "ok" { OK_EVERY_MS } else { FAILING_EVERY_MS };
        if !due {
            return;
        }
        // Its port was taken: try it again, and the tunnel with it.
        if !self.gateway().listening() {
            self.reach.lock().unwrap()["at"] = json::number(t);
            return self.apply(&s, false).await;
        }
        if s.tunnel.is_none() {
            return;
        }
        self.tunnel().ensure().await;
        self.check().await;
    }

    async fn apply(self: &Arc<Self>, s: &Saved, take_over: bool) {
        self.gateway().set_host(s.host.as_deref());
        let had = self.problem.lock().unwrap().take();
        let result = if s.on {
            self.gateway().start().await.map(|_| ())
        } else {
            self.gateway().stop().await;
            Ok(())
        };
        if let Err(e) = result {
            let p = if e.kind() == std::io::ErrorKind::AddrInUse { format!("port {} is in use by another program", self.port) } else { e.to_string() };
            if had.as_deref() != Some(p.as_str()) {
                (self.log)("gateway.error", &p);
            }
            *self.problem.lock().unwrap() = Some(p.clone());
            // Its port taken: a tunnel XenoNet runs stops, but Funnel's
            // setting, which may be that one's, stays.
            self.tunnel().stop();
            let mut r = self.reach.lock().unwrap();
            for (k, v) in [("state", json!("failing")), ("stage", json!("port")), ("why", json!(p)), ("at", json::number((self.now)()))] {
                r[k] = v;
            }
            return;
        }
        {
            let mut r = self.reach.lock().unwrap();
            if r["stage"] == "port" {
                r["state"] = json!("unknown");
                r["stage"] = Value::Null;
                r["why"] = Value::Null;
            }
        }
        let Some((k, name, host)) = s.tunnel.clone().filter(|_| s.on) else {
            self.tunnel().down().await;
            let mut r = self.reach.lock().unwrap();
            r["state"] = json!(if s.on { "failing" } else { "off" });
            r["stage"] = if s.on { json!("address") } else { Value::Null };
            r["why"] = Value::Null;
            return;
        };
        if let Err(e) = self.tunnel().up(&k, name.as_deref(), host.as_deref(), take_over).await {
            (self.log)("gateway.tunnel", &e);
        }
        self.soon(if k == "own" || k == "tailscale" { 1500 } else { 10_000 });
    }

    // The actors with a gateway token, or refused one lately, each with how
    // its connection stands.
    fn actors_view(&self, host: Option<&str>) -> Value {
        let t = (self.now)();
        let mut out = Vec::new();
        let mut ids = Vec::new();
        let seen = self.seen.lock().unwrap().clone();
        let refused = self.refused.lock().unwrap().clone();
        let row = |id: &str, extra: Value| {
            let info = self.actors.info(id);
            let slug = id.strip_prefix("actor:").unwrap_or(id);
            let sn = seen.get(slug);
            let last_host = sn.and_then(|s| s["host"].as_str());
            let mut r = json!({
                "id": id,
                "name": info.as_ref().and_then(|i| i.get("name").cloned()).unwrap_or(json!(slug)),
                "product": info.as_ref().and_then(|i| i["product"].get("id").cloned()).unwrap_or(Value::Null),
                "lastAt": sn.map_or(Value::Null, |s| s["at"].clone()),
                "lastHost": last_host,
                "oldAddress": host.is_some() && last_host.is_some() && last_host != host,
                "refused": refused.get(id).cloned().unwrap_or(Value::Null),
            });
            for (k, v) in extra.as_object().unwrap() {
                r[k] = v.clone();
            }
            r
        };
        for r in self.store.actor_token_times().unwrap_or_default() {
            if r["hasGateway"] != json!(true) {
                continue;
            }
            let id = format!("actor:{}", xenonet_base::text::js_string(&r["slug"]));
            ids.push(id.clone());
            let expires = r["gatewayAt"].as_f64().map(|g| g + TOKEN_UNUSED_MS);
            out.push(row(&id, json!({ "token": true, "expiresAt": expires.map(json::number), "expiring": expires.is_some_and(|e| e - t < EXPIRING_MS) })));
        }
        for id in refused.keys() {
            if !ids.contains(id) {
                out.push(row(id, json!({ "token": false, "expiresAt": null, "expiring": false })));
            }
        }
        Value::Array(out)
    }

    /// At start, as it was left, with the clock started.
    pub async fn init(self: &Arc<Self>) {
        let s = self.saved();
        self.apply(&s, false).await;
        let me = self.clone();
        let task = tokio::spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
            loop {
                every.tick().await;
                me.tick().await;
            }
        });
        *self.ticker.lock().unwrap() = Some(task);
    }

    pub async fn stop(&self) {
        if let Some(t) = self.ticker.lock().unwrap().take() {
            t.abort();
        }
        if let Some(t) = self.soon.lock().unwrap().take() {
            t.abort();
        }
        self.tunnel().stop();
        self.gateway().stop().await;
    }

    /// `set({ on, tunnel, check, repair })`: `tunnel` None leaves it,
    /// Some(null) takes it away. Another tunnel stops the one before; repair
    /// sets it up again, letting Funnel replace what it forwarded to, and
    /// checks at once.
    pub async fn set(self: &Arc<Self>, on: Option<bool>, pick: Option<Value>, check_now: bool, repair: bool) -> R<Value> {
        let mut s = self.saved();
        if let Some(pick) = pick {
            if pick.is_null() {
                s.tunnel = None;
            } else {
                let k_id = match pick.get("kind") {
                    Some(Value::Null) | None => String::new(),
                    Some(v) => xenonet_base::text::js_string(v),
                };
                let Some(k) = kind(&k_id) else { return Err(bad(&format!("a tunnel is one of {}", kind_ids().join(", ")))) };
                let text = |key: &str| match pick.get(key) {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) if s.is_empty() => None,
                    Some(v) => Some(xenonet_base::text::js_trim(&xenonet_base::text::js_string(v)).to_string()),
                };
                let name = text("name");
                if k.asks_name.is_some() && name.as_deref().is_some_and(|n| !tunnel_name(n)) {
                    return Err(bad("a tunnel's name is letters, digits, dots, dashes and underscores, as cloudflared tunnel list shows it"));
                }
                if k.asks_name == Some("required") && name.is_none() {
                    return Err(bad("say the tunnel's name, as cloudflared tunnel list shows it"));
                }
                let raw_host = text("host");
                let given = raw_host.as_deref().and_then(|h| normal_host(Some(h)));
                if k.asks_host.is_some() && raw_host.is_some() && given.is_none() {
                    return Err(bad("that isn't a host name, such as xenonet.example.com"));
                }
                if k.asks_host == Some("required") && given.is_none() {
                    return Err(bad("say the address it answers for, such as xenonet.example.com"));
                }
                let before = s.tunnel.clone();
                let host = if k.asks_host.is_some() { given } else { None };
                s.tunnel = Some((k_id.clone(), if k.asks_name.is_some() { name } else { None }, host.clone()));
                // The address: the one given, else the one this kind got
                // before, else none until the tunnel says.
                if host.is_some() {
                    s.host = host;
                } else if before.as_ref().map(|b| b.0.as_str()) != Some(k_id.as_str()) || before.as_ref().is_some_and(|b| b.2.is_some()) {
                    s.host = None;
                }
            }
        }
        if let Some(on) = on {
            s.on = on;
        }
        // Kept first: a gateway turned on that wouldn't be on after a
        // restart isn't one to turn on.
        self.save(&s)?;
        self.apply(&s, repair).await;
        if repair || check_now {
            self.check().await;
        }
        self.status().await
    }

    pub async fn status(&self) -> R<Value> {
        let s = self.saved();
        let g = self.gateway().status();
        let tv = self.tunnel().view();
        let reach = self.reach.lock().unwrap().clone();
        let why = self
            .problem
            .lock()
            .unwrap()
            .clone()
            .map(Value::String)
            .or_else(|| (s.on && s.tunnel.is_none()).then(|| json!("pick how the gateway reaches the internet")))
            .or_else(|| (s.on && tv["state"] == "failed").then(|| tv["why"].clone()))
            .or_else(|| (s.on && reach["state"] == "failing").then(|| reach["why"].clone()))
            .unwrap_or(Value::Null);
        let mut tunnel = tv.clone();
        match &s.tunnel {
            Some((k, name, given)) => {
                let kd = kind(k).unwrap();
                for (key, v) in
                    [("kind", json!(k)), ("label", json!(kd.label)), ("through", json!(kd.through)), ("name", json!(name)), ("given", json!(given)), ("stable", json!(kd.stable))]
                {
                    tunnel[key] = v;
                }
            }
            None => {
                for key in ["kind", "label", "through", "name", "given", "stable"] {
                    tunnel[key] = Value::Null;
                }
            }
        }
        let mut out = json!({ "on": s.on });
        for (k, v) in g.as_object().unwrap() {
            out[k] = v.clone();
        }
        out["tunnel"] = tunnel;
        out["reach"] = reach;
        out["problem"] = why;
        out["kinds"] = self.tunnel().kinds().await;
        out["actors"] = self.actors_view(s.host.as_deref());
        Ok(out)
    }

    /// The gateway's public address, or null.
    pub fn url(&self) -> Value {
        self.gateway().status()["url"].clone()
    }
}
