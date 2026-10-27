//! The engine across machines (docs/design/network.md, sections 5, 6, 8,
//! 10 and 11): what this host tells the others through the seam
//! (`xenonet_base::network`), and what it does with what they tell it.
//!
//! - **Presence.** Every 2 seconds, if its agents changed, this host sends
//!   them to every other host; a full set when its link comes up or after
//!   a gap. It keeps the latest presence of each other host, and drops an
//!   older one. Unless the host shares titles only, each agent carries
//!   its detail (section 7): the text of what waits on you, its last
//!   message and why it seems stuck. That's for the other consoles: the
//!   engine's view of another host's agent, which agents read in `peers`,
//!   never carries it.
//! - **Messages.** A message to an agent on another host is stored here as
//!   sent, with its id across the network (its envelope's mid), and its
//!   frame waits in the outbox until a stored receipt comes back. One from
//!   another host is stored with its sender qualified (`@laptop:claude:…`)
//!   and an envelope that says how it came, then delivered as local mail.
//!   Receipts go back: stored at once, then delivered, seen or held. One
//!   stored there that no one read by the end of the outbox's week is
//!   unread here.
//! - **Claims across clones.** Claims and observed edits go out as a repo
//!   key and a path inside the repo; others' come back for the edit
//!   warning.
//! - **Handoffs.** Work left for another session, kept by the host that
//!   made it, which decides who takes it.
//!
//! Every path here runs only while `Net::me()` says this machine is in a
//! network, except handoffs, which work on one machine too.

use crate::engine::{told, EngineCalls};
use crate::js::{arg, at, json, list, now_ms, obj, text, Locked};
use crate::world::World;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use xenonet_base::ids::{is_session_id, parse_actor_id, qualify, YOU};
use xenonet_base::network::{Host, Incoming, Kind, Net, Outgoing, Sink, To};
use xenonet_engine::delivery::reach::{self, Action, Decision, Settings};
use xenonet_engine::engine::{is_actor_agent, is_agent_id, Fail, ACTIVE_MS};
use xenonet_engine::network::{host_of, is_remote, local_of, new_id};
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::sql::{Arg, Row};
use xenonet_service::glue::From;

/// How often presence and claims go out when they changed.
pub const PRESENCE_MS: f64 = 2000.0;
/// What waits in the outbox, at most: a week.
const OUTBOX_MS: f64 = 7.0 * 24.0 * 3600e3;
/// A handoff no one takes lapses after a week.
const HANDOFF_MS: f64 = 7.0 * 24.0 * 3600e3;
/// An edit counts as the agent's for half an hour, as the engine says.
const EDIT_HOLD_MS: f64 = 30.0 * 60e3;
/// Messages in one thread, across hosts.
const MAX_HOPS: f64 = 6.0;
/// What another host may keep here: its claims, and handoffs open.
const MAX_CLAIMS: f64 = 5000.0;
const MAX_HANDOFFS: f64 = 100.0;
/// An agent's detail, at most, as network/presence/v1 has it: a waiting
/// question's text, its last message (as the live state keeps it), and its
/// stuck reasons.
const QUESTION_MAX: usize = 320;
const LAST_MAX: usize = 280;
const STUCK_MAX: usize = 8;
const REASON_MAX: usize = 280;
/// When the person last worked at this console is kept to the half minute,
/// so presence doesn't change every tick while they type.
const YOU_STEP_MS: f64 = 30e3;

/// What this host last told the others: its presence (each agent as sent)
/// and its claims, and the timer that tells them again.
#[derive(Default)]
pub struct NetState {
    out: Mutex<Out>,
    ticker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Held while a body from another host is taken, one at a time, so a
    /// repeat that arrives beside its first copy is still seen as one.
    taking: Mutex<()>,
}

#[derive(Default)]
struct Out {
    counter: f64,
    agents: BTreeMap<String, Value>,
    you: bool,
    /// When the person last worked at this console, as presence said it.
    you_at: Option<f64>,
    /// The roster head the last presence carried.
    roster: String,
    claims: BTreeMap<String, Value>,
    /// The roster head when the claims last went: a new one sends them all.
    claims_roster: String,
}

impl Drop for NetState {
    fn drop(&mut self) {
        if let Some(t) = self.ticker.locked().take() {
            t.abort();
        }
    }
}

// ---------- wiring ----------

/// Puts the network behind the engine, with no timer: what the tests do,
/// calling `tick` themselves.
pub fn set_net(w: &World, net: Arc<dyn Net>) {
    w.engine.set_net(net);
}

/// Puts the network's link behind the engine and starts telling the other
/// hosts of this one (presence and claims every 2 s when they changed,
/// handoffs that lapse). The sink is what the link calls with what
/// arrives.
pub fn plug(w: &Arc<World>, net: Arc<dyn Net>) -> Arc<EngineSink> {
    set_net(w, net);
    let t = w.every(PRESENCE_MS, tick);
    if let Some(old) = w.net.ticker.locked().replace(t) {
        old.abort();
    }
    EngineSink::new(w)
}

/// A call from the link, for the sink's own thread.
type Job = Box<dyn FnOnce(&World) + Send + 'static>;

/// What the network calls in the engine. The calls run one at a time, in
/// the order the link made them, on a thread of the sink's own: the engine
/// reads git and its database, and two messages from one host must reach
/// its sessions in the order they were sent, which separate blocking
/// threads don't keep.
pub struct EngineSink {
    w: Weak<World>,
    jobs: Option<Mutex<std::sync::mpsc::Sender<Job>>>,
}

impl EngineSink {
    pub fn new(w: &Arc<World>) -> Arc<EngineSink> {
        let weak = Arc::downgrade(w);
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let worker = weak.clone();
        // The thread ends when the sink, and so the sender, goes.
        let spawned = std::thread::Builder::new().name("xenonet-net-sink".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                if let Some(w) = worker.upgrade() {
                    job(&w);
                }
            }
        });
        let jobs = match spawned {
            Ok(_) => Some(Mutex::new(tx)),
            Err(e) => {
                w.log("warn", "net.sink", &format!("the network's own thread didn't start ({e}); what arrives runs on the shared ones"));
                None
            }
        };
        Arc::new(EngineSink { w: weak, jobs })
    }

    fn run(&self, f: impl FnOnce(&World) + Send + 'static) {
        let mut job: Job = Box::new(f);
        if let Some(jobs) = &self.jobs {
            match jobs.locked().send(job) {
                Ok(()) => return,
                Err(e) => job = e.0,
            }
        }
        if let Some(w) = self.w.upgrade() {
            w.spawn_blocking(move |w| job(w));
        }
    }
}

impl Sink for EngineSink {
    fn receive(&self, incoming: Incoming) {
        self.run(move |w| receive(w, incoming));
    }
    fn linked(&self, up: bool) {
        self.run(move |w| linked(w, up));
    }
    fn removed(&self, host: &str) {
        let host = host.to_string();
        self.run(move |w| removed(w, &host));
    }
    fn gap(&self) {
        self.run(gap);
    }
}

fn net(w: &World) -> Arc<dyn Net> {
    w.engine.net.get()
}

fn send(w: &World, to: To, kind: Kind, body: Value) {
    if let Err(e) = net(w).send(Outgoing { to, kind, body }) {
        w.log("warn", "net.send", &format!("a {} for the network wasn't queued: {e}", kind.as_str()));
    }
}

fn store<T>(w: &World, f: impl FnOnce(&xenonet_engine::store::EngineStore) -> xenonet_engine::sql::Result<T>) -> Result<T> {
    w.store.with(|s| f(s)).map_err(Thrown::from)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn n(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

/// At most `max` characters, as a schema's limit asks.
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn opt(v: Option<String>, max: usize) -> Value {
    v.map_or(Value::Null, |s| Value::String(clip(&s, max)))
}

fn ms(x: f64) -> Value {
    if x.is_finite() && x >= 0.0 {
        json!(x.round() as i64)
    } else {
        Value::Null
    }
}

fn row_json(r: &Row) -> Value {
    r.to_json()
}

// ---------- from the link ----------

/// The link came up, or went down. On the way up this host sends its full
/// presence and claims, and again what has no stored receipt.
pub fn linked(w: &World, up: bool) {
    if up && net(w).me().is_some() {
        presence(w, true);
        claims(w, true);
        resend(w);
    }
}

/// The gateway lost frames: a full presence and claims, and again what has
/// no stored receipt.
pub fn gap(w: &World) {
    linked(w, true);
}

/// A host left the network: its agents, claims and waiting handoffs go.
pub fn removed(w: &World, host: &str) {
    if let Err(e) = store(w, |s| s.drop_net_host(host)) {
        w.log("warn", "net.removed", &e.message);
    }
}

/// What this host tells the others now, if it changed: presence and
/// claims; and handoffs made here that lapsed, and messages no one read.
pub fn tick(w: &World) {
    if net(w).me().is_none() {
        return;
    }
    presence(w, false);
    claims(w, false);
    lapse(w);
    unread(w);
}

/// A body from another host.
pub fn receive(w: &World, incoming: Incoming) {
    if net(w).me().is_none() {
        return;
    }
    let Incoming { from, kind, body, from_window_host, .. } = incoming;
    let _one = w.net.taking.locked();
    let done = match kind {
        Kind::Presence => presence_in(w, &from, &body),
        Kind::Claims => claims_in(w, &from, &body),
        Kind::Message => message_in(w, &from, &body, from_window_host),
        Kind::Receipt => receipt_in(w, &from, &body),
        Kind::Handoff => handoff_in(w, &from, &body),
        // The person's actions from another console are the network's
        // own (section 8); a message from them comes as a message.
        Kind::Operator => Ok(()),
        // The map's frames go to the relay and the hub, and services'
        // requests and answers to the network's services: never the engine.
        Kind::Relay | Kind::Svc => Ok(()),
    };
    if let Err(e) = done {
        w.log("warn", "net.receive", &format!("a {} from {from} wasn't taken: {}", kind.as_str(), e.message));
    }
}

// ---------- presence ----------

fn tilde(w: &World, p: &str) -> String {
    xenonet_base::paths::tilde(p, &w.setup.home)
}

/// This host's agents as presence lists them: with their detail, unless
/// the host shares titles only.
fn agents_now(w: &World, titles_only: bool) -> Result<BTreeMap<String, Value>> {
    let mut c = EngineCalls::new(w, None);
    let agents = w.engine.agents(&mut c, ACTIVE_MS).map_err(|e| match e {
        Fail::Error(m) | Fail::Bad(m) => Thrown::new(m),
        Fail::Thrown(t) => t,
    })?;
    // The relay's rule for text that leaves the machine: the home folder,
    // and any other folder named for a person that the agents work in, as ~.
    let scrub = (!titles_only).then(|| {
        let home = w.setup.home.as_str();
        let mut also: Vec<String> = Vec::new();
        for a in &agents {
            if let Some(f) = xenonet_hosts::relay::user_folder_of(Some(&json(&a.field("cwd"))), Some(home)) {
                if !also.contains(&f) {
                    also.push(f);
                }
            }
        }
        xenonet_hosts::relay::Scrubber::new(Some(home), &also)
    });
    let now = now_ms();
    let mut out = BTreeMap::new();
    for a in agents {
        let id = text(&a.field("id"));
        if is_remote(&a.field("id")) || id == YOU {
            continue;
        }
        let cwd = a.field("cwd");
        let key = w.repos.remote_key(&cwd);
        let waits = a.field("needsYou").truthy();
        let state = if waits { "waiting".to_string() } else { a.field("status").as_str().unwrap_or("idle").to_string() };
        out.insert(
            id.clone(),
            json!({
                "id": clip(&id, 200),
                "app": opt(a.field("app").as_str().map(str::to_string), 32),
                "title": opt(a.field("title").as_str().map(str::to_string), 200),
                "state": clip(&state, 32),
                "doing": opt(a.field("focus").as_str().map(str::to_string), 280),
                "repoKey": opt(key.as_str().map(str::to_string), 300),
                "branch": opt(a.field("branch").as_str().map(str::to_string), 200),
                "cwd": opt(cwd.as_str().map(|p| tilde(w, p)), 500),
                // To the half minute, so an agent at work isn't a change
                // every tick.
                "lastActive": ms((a.field("lastActive").as_f64().unwrap_or(f64::NAN) / 30e3).floor() * 30e3),
                "waitsOnYou": waits,
            }),
        );
        if let (Some(scrub), Some(v)) = (&scrub, out.get_mut(&id)) {
            let live = match w.state().live_info(&id, now) {
                Ok(Some(l)) => l,
                _ => Value::Null,
            };
            detail(v, &live, waits, scrub);
        }
    }
    Ok(out)
}

/// An agent's detail from its live state, as it leaves this host: the text
/// of what waits on you (only while it waits), its last message and its
/// stuck reasons, each scrubbed and clipped as the schema has them. What
/// it doesn't have is left out.
fn detail(v: &mut Value, live: &Value, waits: bool, scrub: &xenonet_hosts::relay::Scrubber) {
    let text = |k: &str, max: usize| live.get(k).and_then(Value::as_str).map(|t| clip(&scrub.text(t), max)).filter(|t| !t.trim().is_empty());
    if let Some(q) = text("question", QUESTION_MAX).filter(|_| waits) {
        v["question"] = json!(q);
    }
    if let Some(m) = text("lastMessage", LAST_MAX) {
        v["lastMessage"] = json!(m);
    }
    let reasons: Vec<Value> =
        live.get("stuck").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(STUCK_MAX).map(|r| json!(clip(&scrub.text(r), REASON_MAX))).collect();
    if !reasons.is_empty() {
        v["stuck"] = Value::Array(reasons);
    }
}

/// Another host's agent as this host keeps it: the fields
/// network/presence/v1 declares, each to its limit, and nothing else.
fn declared(a: &Value) -> Option<Value> {
    let id = s(a, "id").filter(|id| own_id(id) && id.chars().count() <= 200)?;
    let text = |k: &str, max: usize| s(a, k).map_or(Value::Null, |t| Value::String(clip(&t, max)));
    let mut out = json!({
        "id": id,
        "app": text("app", 32),
        "title": text("title", 200),
        "state": text("state", 32),
        "doing": text("doing", 280),
        "repoKey": text("repoKey", 300),
        "branch": text("branch", 200),
        "cwd": text("cwd", 500),
        "lastActive": n(a, "lastActive").map_or(Value::Null, ms),
        "waitsOnYou": a.get("waitsOnYou").and_then(Value::as_bool).unwrap_or(false),
    });
    for (k, max) in [("question", QUESTION_MAX), ("lastMessage", LAST_MAX)] {
        if let Some(t) = s(a, k) {
            out[k] = json!(clip(&t, max));
        }
    }
    let reasons: Vec<Value> =
        a.get("stuck").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(STUCK_MAX).map(|r| json!(clip(r, REASON_MAX))).collect();
    if !reasons.is_empty() {
        out["stuck"] = Value::Array(reasons);
    }
    Some(out)
}

/// How the person stands at this console, from actor:you's presence:
/// whether they're here (working or idle), and whether they're working
/// now (input in the last two minutes).
fn you_here(w: &World) -> (bool, bool) {
    let Some(a) = &w.actors else { return (false, false) };
    let p = a.call(w, "presence", &[Arg::text(YOU)]).unwrap_or(Arg::Null);
    (matches!(p.as_str(), Some("working" | "idle")), p.as_str() == Some("working"))
}

/// This host's presence, sent to every other host when it changed (or in
/// full). The body it sent, if it sent one.
pub fn presence(w: &World, full: bool) -> Option<Value> {
    net(w).me()?;
    let now = match agents_now(w, net(w).titles_only()) {
        Ok(a) => a,
        Err(e) => {
            w.log("warn", "net.presence", &e.message);
            return None;
        }
    };
    // Where the person is: the consoles chime for what waits only where
    // they last worked, or everywhere once they've been away from all of
    // them for five minutes (section 8), so presence says when that was.
    let (you, working) = you_here(w);
    let worked = working.then(|| (now_ms() / YOU_STEP_MS).floor() * YOU_STEP_MS);
    // The roster's head as this host has it: hosts that see different
    // rosters notice it from each other's presence. A new head may be a
    // host that joined and has heard nothing yet, so the set goes in full.
    let roster = net(w).roster_head().unwrap_or_default();
    let body = {
        let mut out = w.net.out.locked();
        let full = full || roster != out.roster;
        let (agents, removed): (Vec<Value>, Vec<Value>) = if full {
            (now.values().take(1000).cloned().collect(), vec![])
        } else {
            let changed = now.iter().filter(|(k, v)| out.agents.get(*k) != Some(*v)).map(|(_, v)| v.clone()).take(1000).collect::<Vec<_>>();
            let gone = out.agents.keys().filter(|k| !now.contains_key(*k)).take(1000).map(|k| json!(k)).collect::<Vec<_>>();
            (changed, gone)
        };
        let you_at = worked.or(out.you_at);
        if !full && agents.is_empty() && removed.is_empty() && you == out.you && you_at == out.you_at && roster == out.roster {
            return None;
        }
        out.counter = (out.counter + 1.0).max(now_ms().round());
        out.agents = now;
        out.you = you;
        out.you_at = you_at;
        out.roster.clone_from(&roster);
        json!({ "schema": "network/presence/v1", "counter": out.counter, "full": full, "agents": agents, "removed": removed, "you": you, "youAt": you_at.map_or(Value::Null, ms), "roster": roster })
    };
    send(w, To::All, Kind::Presence, body.clone());
    Some(body)
}

/// Whether an id another host lists is one of its own agents: never one
/// it qualified itself, nor you.
fn own_id(id: &str) -> bool {
    !id.starts_with('@') && id != YOU && (is_session_id(id) || parse_actor_id(id).is_some_and(|(h, _)| h.is_none()))
}

fn presence_in(w: &World, from: &str, body: &Value) -> Result<()> {
    let counter = n(body, "counter").ok_or_else(|| Thrown::new("a presence without a counter"))?;
    let had = store(w, |s| s.net_presence(from))?;
    if let Some(h) = &had {
        if counter <= h.num("counter").unwrap_or(f64::NEG_INFINITY) {
            return Ok(());
        }
    }
    let full = body.get("full").and_then(Value::as_bool).unwrap_or(false);
    let mut agents: Vec<Value> =
        if full { Vec::new() } else { had.as_ref().and_then(|h| h.text("agents")).and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok()).unwrap_or_default() };
    let removed: Vec<String> = body.get("removed").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    agents.retain(|a| s(a, "id").is_some_and(|id| !removed.contains(&id)));
    for a in body.get("agents").and_then(Value::as_array).into_iter().flatten() {
        let Some(kept) = declared(a) else { continue };
        let id = s(&kept, "id").unwrap_or_default();
        agents.retain(|x| s(x, "id").as_deref() != Some(&id));
        agents.push(kept);
    }
    agents.truncate(1000);
    let you = body.get("you").and_then(Value::as_bool).unwrap_or(false);
    let roster = s(body, "roster").unwrap_or_default();
    store(w, |s| s.set_net_presence(from, counter, now_ms(), you, &roster, &Value::Array(agents).to_string()))
}

/// Another host's agent as the engine sees it; with `detail`, as the
/// console sees it, with what waits on you, its last message and why it
/// seems stuck, which no agent's view carries.
fn view_of(host: &Host, a: &Value, detail: bool) -> Value {
    let local = s(a, "id").unwrap_or_default();
    let key = s(a, "repoKey");
    let cwd = s(a, "cwd");
    let project =
        key.as_deref().and_then(|k| k.rsplit('/').next()).or_else(|| cwd.as_deref().and_then(|c| c.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())).map(str::to_string);
    let state = s(a, "state").unwrap_or_else(|| "idle".into());
    let status = match state.as_str() {
        "working" => "working",
        "ended" => "ended",
        _ => "idle",
    };
    let title = s(a, "title").unwrap_or_else(|| local.clone());
    let mut v = json!({
        "id": format!("@{}:{local}", host.id),
        "app": s(a, "app").unwrap_or_else(|| local.split(':').next().unwrap_or("").to_string()),
        "title": clip(&title, 90),
        "project": project,
        "projectId": null,
        "projectKind": null,
        "cwd": cwd,
        "top": null,
        "repo": null,
        "branch": s(a, "branch"),
        "status": status,
        "ended": status == "ended",
        "needsYou": null,
        "lastActive": a.get("lastActive").cloned().unwrap_or(Value::Null),
        "focus": s(a, "doing"),
        "editing": [],
        "claims": [],
        "reachable": host.online,
        "host": host.id,
        "seen": host.seen.map_or(Value::Null, ms),
        "online": host.online,
        "repoKey": key,
        "waitsOnYou": a.get("waitsOnYou").and_then(Value::as_bool).unwrap_or(false),
    });
    if detail {
        for k in ["question", "lastMessage", "stuck"] {
            v[k] = a.get(k).cloned().unwrap_or(Value::Null);
        }
    }
    v
}

/// Every other host's agents, by the presence each last sent: only hosts
/// in the roster, each host's working agents first. With `detail`, for the
/// console's state, never for an agent.
pub fn remote_views(w: &World, detail: bool) -> Vec<Value> {
    let net = net(w);
    if net.me().is_none() {
        return Vec::new();
    }
    let hosts = net.hosts();
    let rows = store(w, xenonet_engine::store::EngineStore::net_presences).unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let Some(host) = r.text("host").and_then(|h| hosts.iter().find(|x| x.id == h)) else { continue };
        let agents: Vec<Value> = r.text("agents").and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        let mut views: Vec<Value> = agents.iter().map(|a| view_of(host, a, detail)).collect();
        views.sort_by(|a, b| {
            let w = |x: &Value| u8::from(x["status"] == "working");
            w(b).cmp(&w(a)).then_with(|| n(b, "lastActive").unwrap_or(0.0).partial_cmp(&n(a, "lastActive").unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
        });
        out.extend(views);
    }
    out
}

// ---------- the engine's calls ----------

/// `engine.net.<method>(...)`, as the engine calls it.
pub fn call(w: &World, c: &mut EngineCalls<'_>, method: &str, args: &[Arg]) -> Result<Arg> {
    let x = |i| at(args, i);
    match method {
        "agents" => Ok(Arg::List(remote_views(w, false).iter().map(arg).collect())),
        // The console's state: with each agent's detail.
        "console" => Ok(Arg::List(remote_views(w, true).iter().map(arg).collect())),
        "view" => {
            let id = text(&x(0));
            Ok(remote_views(w, false).into_iter().find(|v| v["id"] == id).map_or_else(|| arg(&stub(&id)), |v| arg(&v)))
        }
        "repoKey" => Ok(w.repos.remote_key(&x(0))),
        "sent" => sent(w, &x(0)).map(|()| Arg::Undef),
        "claimHits" => claim_hits(w, c, &x(0), &list(x(1))),
        "handoffs" => Ok(Arg::List(offers(w, &x(0))?.iter().map(arg).collect())),
        "receipt" => {
            let d = store(w, |s| s.delivery(crate::js::num(&x(0)), &text(&x(1))))?;
            Ok(d.map_or(Arg::Null, |d| obj(vec![("host", crate::js::cell(d.get("host"))), ("receipt", crate::js::cell(d.get("receipt")))])))
        }
        _ => Err(crate::js::no_method("engine.net", method)),
    }
}

/// An agent on a host whose presence doesn't list it (any more).
fn stub(id: &str) -> Value {
    let local = local_of(id);
    json!({ "id": id, "app": local.split(':').next().unwrap_or(""), "title": clip(local, 90), "project": null, "branch": null, "host": host_of(id) })
}

// ---------- messages out ----------

fn sender_kind(me: &str) -> &'static str {
    if me == YOU {
        "you"
    } else if parse_actor_id(me).is_some() {
        "actor"
    } else {
        "agent"
    }
}

/// A message stored here as sent goes to each agent on another host as a
/// frame of its own, which waits in the outbox for its stored receipt.
fn sent(w: &World, a: &Arg) -> Result<()> {
    let id = crate::js::num(&a.field("id"));
    let env = a.field("envelope");
    let mid = match env.field("mid") {
        Arg::Text(m) if m.starts_with("msg_") => m,
        _ => new_id("msg")?,
    };
    store(w, |s| s.set_message_net(id, None, &mid))?;
    let me = text(&a.field("sender"));
    let auth = env.field("auth");
    let method = auth.field("method").as_str().unwrap_or(if me == YOU { "window" } else { "session" }).to_string();
    let strength = if auth.field("strength").as_str() == Some("strong") { "strong" } else { "weak" };
    let reply_mid = match a.field("replyTo") {
        Arg::Num(r) => store(w, |s| s.message(Arg::Num(r)))?.and_then(|m| m.text("mid").map(str::to_string)),
        _ => None,
    };
    for r in list(a.field("recipients")) {
        let rid = text(&r.field("id"));
        let Some(host) = host_of(&rid).map(str::to_string) else { continue };
        let body = json!({
            "schema": "network/message/v1",
            "mid": mid,
            "sender": me,
            "senderKind": sender_kind(&me),
            "auth": { "method": clip(&method, 32), "strength": strength },
            "target": local_of(&rid),
            "body": text(&a.field("text")),
            "replyTo": reply_mid,
            "scope": text(&a.field("scope")),
            "kind": "message",
            "hops": crate::js::num(&a.field("hops")).max(0.0),
            "at": ms(crate::js::num(&a.field("at"))),
        });
        store(w, |s| s.set_delivery_net(id, &rid, &host, &body.to_string()))?;
        send(w, To::Host(host), Kind::Message, body);
    }
    Ok(())
}

/// Sends again what has no stored receipt, oldest first.
pub fn resend(w: &World) {
    let rows = store(w, |s| s.outbox(now_ms() - OUTBOX_MS)).unwrap_or_default();
    for r in rows {
        let (Some(host), Some(frame)) = (r.text("host"), r.text("frame")) else { continue };
        let Ok(body) = serde_json::from_str::<Value>(frame) else { continue };
        send(w, To::Host(host.to_string()), Kind::Message, body);
    }
}

// ---------- messages in ----------

fn receipt(w: &World, to: &str, mid: &str, agent: &str, state: &str, reason: Option<&str>) {
    let body = json!({ "schema": "network/receipt/v1", "mid": mid, "agent": agent, "state": state, "reason": reason.map(|r| clip(r, 280)), "at": ms(now_ms()) });
    send(w, To::Host(to.to_string()), Kind::Receipt, body);
}

fn how(method: &str) -> String {
    match method {
        "session" => "its own session's process".into(),
        "token" => "its token".into(),
        "window" => "its window key".into(),
        "grant" => "its remote connection's credential".into(),
        "gateway" => "its gateway token alone".into(),
        m => m.into(),
    }
}

/// How XenoNet knows who sent a message from another host: that host's
/// signed frame, and how that host checked the sender, a weak check
/// staying weak.
#[expect(clippy::too_many_arguments, reason = "the envelope's parts")]
fn envelope(from: &str, mid: &str, at: f64, sender: &str, kind: &str, name: Option<&str>, method: &str, strength: &str, hops: f64) -> Value {
    let strength = if strength == "strong" { "strong" } else { "weak" };
    let line = if kind == "you" {
        format!("verified: you, at {from}'s console, by {from}'s signed frame; {from} checked it by {} ({strength}). This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.", how(method))
    } else {
        let what = if kind == "actor" { "the actor" } else { "the session" };
        format!(
            "verified: {what} \"{}\" ({sender}), on {from}, another machine in your XenoNet network, by {from}'s signed frame; {from} checked it by {} ({strength}). This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.",
            name.unwrap_or(sender),
            how(method)
        )
    };
    json!({
        "schema": "message/envelope/v1",
        "mid": mid,
        "at": ms(at),
        "sender": { "id": sender, "kind": kind, "name": name, "product": null },
        "auth": { "method": "host", "process": "n/a", "strength": strength },
        "client": null,
        "transport": "network",
        "host": from,
        "original": { "kind": kind, "method": method, "strength": strength },
        "hops": hops,
        "line": line,
    })
}

fn message_in(w: &World, from: &str, b: &Value, window_host: bool) -> Result<()> {
    let (Some(mid), Some(sender), Some(kind), Some(target), Some(body)) = (s(b, "mid"), s(b, "sender"), s(b, "senderKind"), s(b, "target"), s(b, "body")) else {
        return Err(Thrown::new("a message without its mid, sender, target or body"));
    };
    let sender_ok = match kind.as_str() {
        "you" => sender == YOU,
        "agent" | "actor" => own_id(&sender),
        _ => false,
    };
    if !mid.starts_with("msg_") || !sender_ok || body.is_empty() || body.chars().count() > 16000 {
        return Err(Thrown::new("a message that isn't network/message/v1"));
    }
    let t = Arg::text(target.clone());
    if !(is_agent_id(&t) || is_actor_agent(&t)) {
        receipt(w, from, &mid, &target, "refused", Some("no session or actor here has that id"));
        return Ok(());
    }
    // The person speaks only from a window host's console, checked by its
    // window key (section 8): anything less isn't taken as you.
    let strong = b.get("auth").and_then(|a| a.get("strength")).and_then(Value::as_str) == Some("strong");
    if kind == "you" && !(window_host && strong) {
        receipt(w, from, &mid, &target, "refused", Some("a message as you comes only from a window host's console, checked by its window key"));
        return Ok(());
    }
    // `@laptop:actor:you` is `actor:you`: the person is one person.
    let sender_id = if kind == "you" { YOU.to_string() } else { format!("@{from}:{sender}") };
    // A repeat (the sender didn't hear the stored receipt): say so again.
    let had = store(w, |s| s.message_by_mid(&mid, Some(from)))?;
    if let Some(m) = &had {
        let id = m.num("id").unwrap_or(f64::NAN);
        if store(w, |s| s.delivery(id, &target))?.is_some() {
            receipt(w, from, &mid, &target, "stored", None);
            return Ok(());
        }
    }
    // A reply names the message it answers by its mid.
    let reply_to = match s(b, "replyTo") {
        Some(r) => store(w, |s| s.message_by_mid(&r, None))?.or(store(w, |s| s.message_by_mid(&r, Some(from)))?).and_then(|m| m.num("id")),
        None => None,
    };
    // How deep the thread is: as the sender says, or as this host's own
    // store has it, whichever is deeper, so no sender can reset the count.
    let hops = n(b, "hops").unwrap_or(0.0).max(local_depth(w, reply_to));
    if hops >= MAX_HOPS {
        receipt(w, from, &mid, &target, "refused", Some("this thread has gone back and forth too many times; XenoNet stops agent-to-agent threads at 6 messages"));
        return Ok(());
    }
    // What this machine lets the sender do here.
    if let Decision::Refuse(why) = reach::decide(Action::Message, &reach::sender_of(&Arg::text(sender_id.clone())), &target, &Settings::default()) {
        receipt(w, from, &mid, &target, "refused", Some(&why));
        return Ok(());
    }
    let mut c = EngineCalls::new(w, None);
    let view = c.view(&t)?;
    // An actor XenoNet only observes reads nothing: refused, as a message
    // sent on this machine is, rather than stored for no one.
    let unreached = xenonet_engine::engine::Engine::unreached(&mut c, &view).map_err(|e| match e {
        Fail::Error(m) | Fail::Bad(m) => Thrown::new(m),
        Fail::Thrown(t) => t,
    })?;
    if let Some(why) = unreached {
        receipt(w, from, &mid, &target, "refused", Some(&why));
        return Ok(());
    }
    // This host's own limits hold for it too: paused, too fast, a repeat.
    let ask = obj(vec![("sender", Arg::text(sender_id.clone())), ("recipients", Arg::List(vec![view.clone()])), ("text", Arg::text(body.clone())), ("depth", Arg::num(hops))]);
    if let Err(e) = xenonet_engine::peer::Calls::call(&mut c, "engine.policy", "checkSend", vec![ask]) {
        receipt(w, from, &mid, &target, "refused", Some(&e.message));
        return Ok(());
    }
    let name = remote_views(w, false).into_iter().find(|v| v["id"] == sender_id).and_then(|v| s(&v, "title"));
    let auth = b.get("auth").cloned().unwrap_or(Value::Null);
    let env =
        envelope(from, &mid, now_ms(), &sender_id, &kind, name.as_deref(), &s(&auth, "method").unwrap_or_else(|| "?".into()), &s(&auth, "strength").unwrap_or_default(), hops);
    let now = now_ms();
    let id = match &had {
        Some(m) => {
            let id = m.num("id").unwrap_or(f64::NAN);
            store(w, |s| s.deliver(Arg::Num(id), t.clone()))?;
            id
        }
        None => {
            let stored = obj(vec![
                ("sender", Arg::text(sender_id.clone())),
                ("target", t.clone()),
                ("scope", Arg::Null),
                ("body", Arg::text(body.clone())),
                ("replyTo", reply_to.map_or(Arg::Null, Arg::Num)),
                ("recipients", Arg::List(vec![t.clone()])),
                ("envelope", arg(&env)),
            ]);
            let id = store(w, |s| s.send(&stored, Arg::Num(now)))?;
            store(w, |s| s.set_message_net(id, Some(from), &mid))?;
            id
        }
    };
    // A session that bypasses its permission checks holds it until it asks.
    let held = w.engine.bypassed(&target);
    receipt(w, from, &mid, &target, if held { "held" } else { "stored" }, None);
    w.mail.wake(&target);
    // How it reads when it's pushed into the session: its framing first.
    let framing = if kind == "you" { xenonet_engine::network::from_you_line(from) } else { xenonet_engine::network::from_host_line(from) };
    let mut event = obj(vec![
        ("id", Arg::Num(id)),
        ("from", Arg::text(sender_id.clone())),
        ("to", Arg::List(vec![t.clone()])),
        ("target", t),
        ("text", Arg::text(format!("{framing}\n{body}"))),
        ("ts", Arg::Num(now)),
        ("verified", arg(&env["line"])),
        ("strength", arg(&env["auth"]["strength"])),
        ("host", Arg::text(from)),
        ("mid", Arg::text(mid)),
    ]);
    if let Some(name) = name {
        push(&mut event, "fromName", Arg::text(format!("{name} on {from}")));
    }
    if held {
        push(&mut event, "held", Arg::Bool(true));
    }
    told(w, "message", &event);
    Ok(())
}

/// The messages a reply chains back through in this host's store, the one
/// it answers included: how deep its thread is here.
fn local_depth(w: &World, reply_to: Option<f64>) -> f64 {
    let mut depth = 0.0;
    let mut next = reply_to;
    while let Some(id) = next {
        if depth >= 50.0 {
            break;
        }
        let Ok(Some(m)) = store(w, |s| s.message(Arg::Num(id))) else { break };
        depth += 1.0;
        next = m.num("reply_to");
    }
    depth
}

fn push(a: &mut Arg, k: &str, v: Arg) {
    if let Arg::Obj(f) = a {
        f.retain(|(n, _)| n != k);
        f.push((k.to_string(), v));
    }
}

fn receipt_in(w: &World, from: &str, b: &Value) -> Result<()> {
    let (Some(mid), Some(agent), Some(state)) = (s(b, "mid"), s(b, "agent"), s(b, "state")) else {
        return Err(Thrown::new("a receipt without its mid, agent or state"));
    };
    let Some(m) = store(w, |s| s.message_by_mid(&mid, None))? else { return Ok(()) };
    let id = m.num("id").unwrap_or(f64::NAN);
    let who = format!("@{from}:{agent}");
    let changed = store(w, |s| s.set_receipt(id, &who, &state, now_ms()))?;
    if changed {
        w.emit(From::Engine, "receipt", json!({ "id": id, "agent": who, "host": from, "state": state, "reason": b.get("reason").cloned().unwrap_or(Value::Null) }));
    }
    Ok(())
}

/// Messages a session took: those from other hosts get a seen receipt.
pub fn seen(w: &World, agent: &Arg, ids: &[Arg]) {
    if net(w).me().is_none() {
        return;
    }
    let me = text(agent);
    for id in ids {
        let Ok(Some(m)) = store(w, |s| s.message(id.clone())) else { continue };
        if let (Some(host), Some(mid)) = (m.text("host"), m.text("mid")) {
            receipt(w, host, mid, &me, "seen", None);
        }
    }
}

/// A message from another host that the router pushed straight into its
/// session: a delivered receipt.
pub fn delivered(w: &World, m: &Arg) {
    let (Some(host), Some(mid)) = (m.field("host").as_str().map(str::to_string), m.field("mid").as_str().map(str::to_string)) else { return };
    let id = m.field("id");
    for agent in list(m.field("to")) {
        let live = w.router.list(200).iter().any(|r| r.field("message") == id && r.field("agent") == agent && r.field("state").as_str() == Some("live"));
        if live {
            receipt(w, &host, &mid, &text(&agent), "delivered", None);
        }
    }
}

// ---------- claims across clones ----------

/// A path inside a working tree, with / between its parts, or None for one
/// outside it.
fn inside(top: &str, p: &str) -> Option<String> {
    let t = top.replace('\\', "/");
    let t = t.trim_end_matches('/');
    let f = p.replace('\\', "/");
    let rest = f.strip_prefix(t)?.strip_prefix('/')?;
    (!rest.is_empty()).then(|| rest.to_string())
}

/// This host's claims and edits as the others see them: by repo key and
/// path inside the repo.
fn claims_now(w: &World) -> Result<BTreeMap<String, Value>> {
    let mut c = EngineCalls::new(w, None);
    let agents = w.engine.agents(&mut c, ACTIVE_MS).map_err(|e| match e {
        Fail::Error(m) | Fail::Bad(m) => Thrown::new(m),
        Fail::Thrown(t) => t,
    })?;
    let mut out = BTreeMap::new();
    for a in agents {
        let top = a.field("top");
        let (Some(top), Some(key)) = (top.as_str(), w.repos.remote_key(&a.field("cwd")).as_str().map(str::to_string)) else { continue };
        let agent = text(&a.field("id"));
        let branch = a.field("branch").as_str().map(|b| clip(b, 200));
        let mut add = |path: String, kind: &str, until: Value| {
            let k = format!("{key}|{path}|{agent}|{kind}");
            out.insert(k, json!({ "repoKey": clip(&key, 300), "path": clip(&path, 1000), "agent": clip(&agent, 200), "branch": branch, "kind": kind, "until": until }));
        };
        for cl in list(a.field("claims")) {
            if let Some(p) = inside(top, &text(&cl.field("pattern"))) {
                add(p, "claim", ms(crate::js::num(&cl.field("until"))));
            }
        }
        for e in list(a.field("editing")) {
            if let Some(p) = inside(top, &text(&e.field("path"))) {
                add(p, "edit", ms(crate::js::num(&e.field("at")) + EDIT_HOLD_MS));
            }
        }
    }
    Ok(out)
}

/// This host's claims, sent when they changed (or in full).
pub fn claims(w: &World, full: bool) -> Option<Value> {
    net(w).me()?;
    let now = match claims_now(w) {
        Ok(c) => c,
        Err(e) => {
            w.log("warn", "net.claims", &e.message);
            return None;
        }
    };
    // A host that joined since the last claims has none of them yet.
    let roster = net(w).roster_head().unwrap_or_default();
    let body = {
        let mut out = w.net.out.locked();
        let full = full || roster != out.claims_roster;
        let (put, released): (Vec<Value>, Vec<Value>) = if full {
            (now.values().take(5000).cloned().collect(), vec![])
        } else {
            let put = now.iter().filter(|(k, v)| out.claims.get(*k) != Some(*v)).map(|(_, v)| v.clone()).take(5000).collect::<Vec<_>>();
            let gone = out
                .claims
                .iter()
                .filter(|(k, _)| !now.contains_key(*k))
                .map(|(_, v)| json!({ "repoKey": v["repoKey"], "path": v["path"], "agent": v["agent"] }))
                .take(5000)
                .collect::<Vec<_>>();
            (put, gone)
        };
        if !full && put.is_empty() && released.is_empty() {
            return None;
        }
        out.claims = now;
        out.claims_roster = roster;
        json!({ "schema": "network/claims/v1", "full": full, "claims": put, "released": released })
    };
    send(w, To::All, Kind::Claims, body.clone());
    Some(body)
}

fn claims_in(w: &World, from: &str, b: &Value) -> Result<()> {
    let at = now_ms();
    if b.get("full").and_then(Value::as_bool).unwrap_or(false) {
        store(w, |s| s.clear_net_claims(from))?;
    }
    for r in b.get("released").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(k), Some(p), Some(a)) = (s(r, "repoKey"), s(r, "path"), s(r, "agent")) {
            store(w, |s| s.drop_net_claim(from, &k, &p, &a))?;
        }
    }
    // As the schema has them: what's past its limits is cut, and a host
    // keeps at most 5000 claims here.
    let mut count = store(w, |st| st.net_claim_count(from))?;
    for cl in b.get("claims").and_then(Value::as_array).into_iter().flatten() {
        let (Some(k), Some(p), Some(a), Some(kind)) = (s(cl, "repoKey"), s(cl, "path"), s(cl, "agent"), s(cl, "kind")) else { continue };
        if !own_id(&a) || !matches!(kind.as_str(), "claim" | "edit") || a.chars().count() > 200 {
            continue;
        }
        let (k, p) = (clip(&k, 300), clip(&p, 1000));
        let branch = s(cl, "branch").map(|x| clip(&x, 200));
        if count >= MAX_CLAIMS && !store(w, |st| st.has_net_claim(from, &k, &p, &a, &kind))? {
            continue;
        }
        store(w, |st| st.put_net_claim(from, &k, &p, &a, branch.as_deref(), &kind, n(cl, "until"), at))?;
        count = store(w, |st| st.net_claim_count(from))?;
    }
    if count >= MAX_CLAIMS {
        w.log("warn", "net.claims", &format!("{from} has {MAX_CLAIMS} claims here, the most a host keeps: the rest were left out"));
    }
    Ok(())
}

/// Agents on other hosts with a hand in the files `me` is about to edit,
/// in their clones of the same repo.
fn claim_hits(w: &World, c: &mut EngineCalls<'_>, me: &Arg, files: &[Arg]) -> Result<Arg> {
    let view = c.view(me)?;
    let (Some(top), Some(key)) = (view.field("top").as_str().map(str::to_string), w.repos.remote_key(&view.field("cwd")).as_str().map(str::to_string)) else {
        return Ok(Arg::List(vec![]));
    };
    let rows = store(w, |s| s.net_claims_in(&key, now_ms()))?;
    let mut out = Vec::new();
    for f in files {
        let file = text(f);
        let Some(rel) = inside(&top, &file) else { continue };
        for r in &rows {
            let path = r.text("path").unwrap_or("");
            if !xenonet_engine::engine::matches(&Arg::text(path), &Arg::text(rel.clone())) {
                continue;
            }
            let host = r.text("host").unwrap_or("");
            out.push(obj(vec![
                ("host", Arg::text(host)),
                ("agent", Arg::text(format!("@{host}:{}", r.text("agent").unwrap_or("")))),
                ("branch", crate::js::cell(r.get("branch"))),
                ("kind", crate::js::cell(r.get("kind"))),
                ("file", Arg::text(file.clone())),
                ("path", Arg::text(rel.clone())),
            ]));
        }
    }
    Ok(Arg::List(out))
}

// ---------- handoffs ----------

fn fail(m: impl Into<String>) -> Fail {
    Fail::Error(m.into())
}

fn thrown(t: Thrown) -> Fail {
    Fail::Thrown(t)
}

/// A handoff as a session reads it.
fn shown(r: &Row, me_host: Option<&str>) -> Value {
    let host = r.text("host");
    let maker = r.text("maker").unwrap_or("");
    let from = match host {
        Some(h) if Some(h) != me_host => format!("@{h}:{maker}"),
        _ => maker.to_string(),
    };
    json!({
        "id": r.text("id"),
        "from": from,
        "host": host.map_or_else(|| me_host.map_or(Value::Null, |h| json!(h)), |h| json!(h)),
        "goal": r.text("goal"),
        "done": r.text("done"),
        "next": r.text("next"),
        "files": r.text("files").and_then(|f| serde_json::from_str::<Value>(f).ok()).unwrap_or_else(|| json!([])),
        "branch": r.text("branch"),
        "commit": r.text("commit_sha"),
        "state": r.text("state"),
        "takenBy": r.text("taken_by"),
        "made": row_json(r)["created_at"],
        "lapses": row_json(r)["expires_at"],
    })
}

/// The handoffs open to a session: to it, or to its repo (here, or across
/// clones of the same remote), never its own.
fn offers(w: &World, me: &Arg) -> Result<Vec<Value>> {
    let rows = store(w, |s| s.open_handoffs(now_ms()))?;
    // Most starts find none, and then ask git nothing.
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let id = text(&me.field("id"));
    let me_host = net(w).me().map(|m| m.host);
    let key = w.repos.remote_key(&me.field("cwd"));
    let scope = me.field("repo").or(if me.field("projectKind").as_str() == Some("project") { me.field("projectId") } else { Arg::Null });
    let mut out = Vec::new();
    for r in &rows {
        let host = r.text("host");
        let maker = r.text("maker").unwrap_or("");
        if host.is_none() && maker == id {
            continue;
        }
        let to_agent = r.text("to_agent");
        let to_host = r.text("to_host");
        let for_me = match to_agent {
            Some(a) => a == id,
            None => {
                let here = to_host.is_none_or(|h| Some(h) == me_host.as_deref());
                let by_key = key.as_str().is_some_and(|k| r.text("to_repo") == Some(k));
                let by_scope = host.is_none() && scope.as_str().is_some_and(|sc| r.text("to_scope") == Some(sc));
                here && (by_key || by_scope)
            }
        };
        if for_me {
            out.push(shown(r, me_host.as_deref()));
        }
    }
    Ok(out)
}

/// The handoff tool: offered, taken, or the ones open to `me`.
pub fn handoff(w: &World, c: &mut EngineCalls<'_>, me: &Arg, a: &Value, envelope: &Value) -> std::result::Result<Value, Fail> {
    // Off a network nothing ticks, so the ones that lapsed go here.
    if net(w).me().is_none() {
        lapse(w);
    }
    if let Some(id) = s(a, "accept") {
        return accept(w, c, me, &id, envelope);
    }
    if ["goal", "done", "next"].iter().any(|k| s(a, k).is_some_and(|t| !t.trim().is_empty())) {
        return offer(w, c, me, a, envelope);
    }
    let view = c.view(me).map_err(thrown)?;
    let open = offers(w, &view).map_err(thrown)?;
    let mine: Vec<Value> = store(w, |s| s.open_handoffs(now_ms()))
        .map_err(thrown)?
        .iter()
        .filter(|r| r.text("host").is_none() && r.text("maker") == me.as_str())
        .map(|r| shown(r, net(w).me().map(|m| m.host).as_deref()))
        .collect();
    Ok(json!({ "open": open, "yours": mine }))
}

fn offer(w: &World, c: &mut EngineCalls<'_>, me: &Arg, a: &Value, envelope: &Value) -> std::result::Result<Value, Fail> {
    let me_id = text(me);
    let view = c.view(me).map_err(thrown)?;
    let commit = s(a, "commit").map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty());
    if let Some(sha) = &commit {
        if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Fail::Bad("commit is a commit's hash: 7 to 64 hex characters".into()));
        }
    }
    let files: Vec<String> = a.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).map(|p| clip(p, 1000)).take(200).collect()).unwrap_or_default();
    let field = |k: &str, max: usize| s(a, k).map(|t| clip(t.trim(), max)).filter(|t| !t.is_empty());
    let (goal, done, next) = (field("goal", 2000), field("done", 8000), field("next", 8000));
    let branch = field("branch", 200).or_else(|| view.field("branch").as_str().map(|b| clip(b, 200)));
    let to = s(a, "to").filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "repo".into());
    let me_net = net(w).me();
    let host = s(a, "host").filter(|h| !h.is_empty());
    if let Some(h) = &host {
        let known = me_net.as_ref().is_some_and(|m| m.host == *h) || net(w).hosts().iter().any(|x| x.id == *h);
        if !known {
            return Err(Fail::Bad(format!("{h} isn't a machine in your XenoNet network")));
        }
    }
    let key = w.repos.remote_key(&view.field("cwd")).as_str().map(str::to_string);
    let scope = view.field("repo").or(if view.field("projectKind").as_str() == Some("project") { view.field("projectId") } else { Arg::Null });
    let id = new_id("hof").map_err(thrown)?;
    let now = now_ms();
    let text_of = |goal: &Option<String>| {
        let mut parts = vec![format!("Handoff {id}: {}", goal.clone().unwrap_or_else(|| "work to pick up".into()))];
        if let Some(d) = &done {
            parts.push(format!("Done: {d}"));
        }
        if let Some(x) = &next {
            parts.push(format!("Next: {x}"));
        }
        if !files.is_empty() {
            parts.push(format!("Files: {}", files.join(", ")));
        }
        if let Some(b) = &branch {
            parts.push(format!("Branch: {b}{}", commit.as_ref().map_or(String::new(), |c| format!(" at {c}"))));
        }
        parts.push(format!("The first session to accept it gets it: use the xenonet handoff tool with accept: \"{id}\"."));
        clip(&parts.join("\n"), 3900)
    };
    let words = text_of(&goal);
    // Who it's for: an agent, or the repo (on one machine of the network,
    // or all of them).
    let (to_agent, to_host, target) = if to == "repo" {
        (None, host.clone(), None)
    } else {
        // Named as send names it; a message tells it now.
        let sent = w.engine.send(c, me, &obj(vec![("to", Arg::text(to.clone())), ("text", Arg::text(words.clone())), ("replyTo", Arg::Null)]), &arg(envelope))?;
        let first = list(sent.field("to")).first().map(|t| text(&t.field("id"))).unwrap_or(to.clone());
        (Some(first.clone()), host_of(&first).map(str::to_string), Some(sent))
    };
    if to_agent.is_none() && key.is_none() && !scope.truthy() {
        return Err(Fail::Bad("a handoff to the repo needs a repo: this session isn't in one".into()));
    }
    let row = obj(vec![
        ("id", Arg::text(id.clone())),
        ("host", Arg::Null),
        ("maker", Arg::text(me_id.clone())),
        ("to_agent", Arg::opt_text(to_agent.as_deref())),
        ("to_repo", Arg::opt_text(key.as_deref())),
        ("to_scope", scope.clone()),
        ("to_host", Arg::opt_text(to_host.as_deref())),
        ("goal", Arg::opt_text(goal.as_deref())),
        ("done", Arg::opt_text(done.as_deref())),
        ("next", Arg::opt_text(next.as_deref())),
        ("files", Arg::text(json!(files).to_string())),
        ("branch", Arg::opt_text(branch.as_deref())),
        ("commit_sha", Arg::opt_text(commit.as_deref())),
        ("created_at", Arg::Num(now)),
        ("expires_at", Arg::Num(now + HANDOFF_MS)),
    ]);
    store(w, |s| s.add_handoff(&row)).map_err(thrown)?;
    // The sessions at work in clones of the repo now hear of it, here and
    // on the other hosts; the next to start in one is offered it.
    let mut told_now = target.map(|t| list(t.field("to")).iter().map(|x| json(&x.field("id"))).collect::<Vec<_>>()).unwrap_or_default();
    if to_agent.is_none() {
        let to = match &host {
            Some(h) if me_net.as_ref().is_none_or(|m| m.host != *h) => key.as_deref().and_then(|k| k.rsplit('/').next()).map(|p| format!("machine:{h} project:{p}")),
            Some(_) => key.as_deref().and_then(|k| k.rsplit('/').next()).map(|p| format!("machine:{} project:{p}", me_net.as_ref().map_or("local", |m| m.host.as_str()))),
            None => Some("repo".into()),
        };
        if let Some(to) = to {
            if let Ok(sent) = w.engine.send(c, me, &obj(vec![("to", Arg::text(to)), ("text", Arg::text(words.clone())), ("replyTo", Arg::Null)]), &arg(envelope)) {
                told_now = list(sent.field("to")).iter().map(|x| json(&x.field("id"))).collect();
            }
        }
    }
    // The other hosts keep it too, for their sessions to be offered.
    if let (Some(m), Some(row)) = (&me_net, store(w, |s| s.handoff(&id)).map_err(thrown)?) {
        let body = body_of(&row, "offer", None);
        let dest = match (&to_agent, &to_host) {
            (Some(a), _) => host_of(a).map(|h| To::Host(h.to_string())),
            (None, Some(h)) if *h != m.host => Some(To::Host(h.clone())),
            (None, Some(_)) => None,
            (None, None) => key.as_ref().map(|_| To::All),
        };
        if let Some(d) = dest {
            send(w, d, Kind::Handoff, body);
        }
    }
    Ok(
        json!({ "id": id, "to": to_agent.map_or_else(|| json!({ "repo": key.clone().or_else(|| scope.as_str().map(str::to_string)), "host": host }), |a| json!({ "agent": a })), "toldNow": told_now, "lapses": ms(now + HANDOFF_MS) }),
    )
}

/// A handoff's frame, from its row here.
fn body_of(r: &Row, op: &str, by: Option<&str>) -> Value {
    let files: Value = r.text("files").and_then(|f| serde_json::from_str(f).ok()).unwrap_or_else(|| json!([]));
    json!({
        "schema": "network/handoff/v1",
        "id": r.text("id"),
        "op": op,
        "from": r.text("maker"),
        "to": { "agent": r.text("to_agent"), "repoKey": r.text("to_repo"), "host": r.text("to_host") },
        "goal": r.text("goal"),
        "done": r.text("done"),
        "next": r.text("next"),
        "files": files,
        "branch": r.text("branch"),
        "commit": r.text("commit_sha"),
        "by": by,
        "at": ms(now_ms()),
    })
}

/// A note to a session here from XenoNet's network, as a message from
/// `sender` (the handoff's maker, or the session that took it).
fn note(w: &World, sender: &str, target: &str, words: &str, host: Option<&str>) {
    let mut env = Value::Null;
    if let Some(h) = host {
        if let Ok(mid) = new_id("msg") {
            env = envelope(h, &mid, now_ms(), sender, "agent", None, "host", "weak", 0.0);
        }
    }
    let t = Arg::text(target);
    let stored = obj(vec![
        ("sender", Arg::text(sender)),
        ("target", t.clone()),
        ("scope", Arg::Null),
        ("body", Arg::text(words)),
        ("replyTo", Arg::Null),
        ("recipients", Arg::List(vec![t.clone()])),
        ("envelope", arg(&env)),
    ]);
    let now = now_ms();
    let Ok(id) = store(w, |s| s.send(&stored, Arg::Num(now))) else { return };
    w.mail.wake(target);
    let mut event =
        obj(vec![("id", Arg::Num(id)), ("from", Arg::text(sender)), ("to", Arg::List(vec![t.clone()])), ("target", t), ("text", Arg::text(words)), ("ts", Arg::Num(now))]);
    if !env.is_null() {
        push(&mut event, "verified", arg(&env["line"]));
    }
    told(w, "message", &event);
}

fn details(r: &Row) -> String {
    let mut parts = vec![format!("Handoff {}: {}", r.text("id").unwrap_or(""), r.text("goal").unwrap_or("work to pick up"))];
    for (k, label) in [("done", "Done"), ("next", "Next")] {
        if let Some(t) = r.text(k) {
            parts.push(format!("{label}: {t}"));
        }
    }
    if let Some(f) = r.text("files").filter(|f| *f != "[]") {
        parts.push(format!("Files: {f}"));
    }
    if let Some(b) = r.text("branch") {
        parts.push(format!("Branch: {b}{}", r.text("commit_sha").map_or(String::new(), |c| format!(" at {c}"))));
    }
    parts.join("\n")
}

fn accept(w: &World, c: &mut EngineCalls<'_>, me: &Arg, id: &str, envelope: &Value) -> std::result::Result<Value, Fail> {
    let me_id = text(me);
    let Some(r) = store(w, |s| s.handoff(id)).map_err(thrown)? else {
        return Err(Fail::Bad(format!("no handoff is {id}; the handoff tool with nothing else lists the ones open to you")));
    };
    let me_net = net(w).me();
    match r.text("state") {
        Some("taken") => return Err(fail(format!("{id} was taken by {}", r.text("taken_by").unwrap_or("another session")))),
        Some("lapsed") => return Err(fail(format!("{id} lapsed: no one took it in a week"))),
        _ => {}
    }
    // One handed to a session is that session's to take.
    if let Some(a) = r.text("to_agent").filter(|a| *a != me_id) {
        return Err(fail(format!("{id} was handed to {a}, not to you")));
    }
    match r.text("host") {
        // Made here: this host decides, and the first to accept gets it.
        None => {
            if !store(w, |s| s.take_handoff(id, &me_id, now_ms())).map_err(thrown)? {
                let now = store(w, |s| s.handoff(id)).map_err(thrown)?;
                return Err(fail(format!("{id} was taken by {}", now.as_ref().and_then(|r| r.text("taken_by")).unwrap_or("another session"))));
            }
            let maker = r.text("maker").unwrap_or("").to_string();
            if maker != me_id {
                let words = format!("I took your handoff {id}.");
                if w.engine.send(c, me, &obj(vec![("to", Arg::text(maker)), ("text", Arg::text(words)), ("replyTo", Arg::Null)]), &arg(envelope)).is_err() {
                    // The maker may have gone; the handoff is taken all the same.
                }
            }
            if let Some(m) = &me_net {
                let by = format!("@{}:{me_id}", m.host);
                send(w, To::All, Kind::Handoff, body_of(&r, "taken", Some(&by)));
            }
            Ok(json!({ "taken": true, "handoff": shown(&r, me_net.as_ref().map(|m| m.host.as_str())), "words": details(&r) }))
        }
        // Made on another host, which decides: the accept waits for it.
        Some(h) => {
            let Some(m) = &me_net else {
                return Err(fail(format!("{id} was made on {h}, and this machine isn't in a XenoNet network now")));
            };
            store(w, |s| s.ask_handoff(id, &me_id)).map_err(thrown)?;
            let by = format!("@{}:{me_id}", m.host);
            send(w, To::Host(h.to_string()), Kind::Handoff, body_of(&r, "accept", Some(&by)));
            let away = !net(w).hosts().iter().any(|x| x.id == h && x.online);
            Ok(json!({
                "taken": false,
                "waiting": true,
                "note": if away {
                    format!("{h} made it and decides who takes it; it's offline now, so your accept waits until it's back. You'll get a message with its answer.")
                } else {
                    format!("{h} made it and decides who takes it; the first to accept gets it. You'll get a message with its answer.")
                },
                "handoff": shown(&r, Some(m.host.as_str())),
            }))
        }
    }
}

fn handoff_in(w: &World, from: &str, b: &Value) -> Result<()> {
    let (Some(id), Some(op)) = (s(b, "id"), s(b, "op")) else { return Err(Thrown::new("a handoff without its id or op")) };
    if !id.starts_with("hof_") {
        return Err(Thrown::new("a handoff that isn't network/handoff/v1"));
    }
    let Some(me) = net(w).me() else { return Ok(()) };
    let now = now_ms();
    // Ids as this host writes them: its own unqualified, others' with
    // their host.
    let here = |id: &str| qualify(id, Some(from), Some(&me.host)).unwrap_or_else(|_| id.to_string());
    match op.as_str() {
        "offer" => {
            let maker = s(b, "from").unwrap_or_default();
            if !own_id(&maker) || maker.chars().count() > 200 {
                return Err(Thrown::new("a handoff whose maker isn't one of its host's agents"));
            }
            if store(w, |st| st.open_handoffs_from(from, now))? >= MAX_HANDOFFS {
                return Err(Thrown::new(format!("{from} has {MAX_HANDOFFS} handoffs open here, the most a host keeps")));
            }
            // As the schema has them: what's past its limits is cut.
            let field = |k: &str, max: usize| s(b, k).map(|t| clip(&t, max));
            let files: Vec<Value> =
                b.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).take(200).map(|p| json!(clip(p, 1000))).collect()).unwrap_or_default();
            let commit = s(b, "commit").filter(|c| (7..=64).contains(&c.len()) && c.bytes().all(|x| x.is_ascii_hexdigit()));
            let to = b.get("to").cloned().unwrap_or(Value::Null);
            let to_agent = s(&to, "agent").filter(|a| a.chars().count() <= 200).map(|a| here(&a));
            let row = obj(vec![
                ("id", Arg::text(id.clone())),
                ("host", Arg::text(from)),
                ("maker", Arg::text(maker)),
                ("to_agent", Arg::opt_text(to_agent.as_deref())),
                ("to_repo", Arg::opt_text(s(&to, "repoKey").map(|k| clip(&k, 300)).as_deref())),
                ("to_scope", Arg::Null),
                ("to_host", Arg::opt_text(s(&to, "host").filter(|h| xenonet_base::ids::is_host_id(h)).as_deref())),
                ("goal", Arg::opt_text(field("goal", 2000).as_deref())),
                ("done", Arg::opt_text(field("done", 8000).as_deref())),
                ("next", Arg::opt_text(field("next", 8000).as_deref())),
                ("files", Arg::text(Value::Array(files).to_string())),
                ("branch", Arg::opt_text(field("branch", 200).as_deref())),
                ("commit_sha", Arg::opt_text(commit.as_deref())),
                ("created_at", Arg::Num(now)),
                ("expires_at", Arg::Num(now + HANDOFF_MS)),
            ]);
            store(w, |s| s.add_handoff(&row))?;
        }
        // Another host's session wants one made here: the first gets it.
        "accept" => {
            let Some(r) = store(w, |s| s.handoff(&id))? else { return Ok(()) };
            if r.text("host").is_some() {
                return Ok(());
            }
            let by = here(&s(b, "by").unwrap_or_default());
            if !is_remote(&Arg::text(by.clone())) || host_of(&by) != Some(from) {
                return Err(Thrown::new("an accept from a session that isn't on the host that sent it"));
            }
            if r.text("to_agent").is_some_and(|a| a != by) {
                return Err(Thrown::new("an accept from a session it wasn't handed to"));
            }
            if store(w, |s| s.take_handoff(&id, &by, now))? {
                let maker = r.text("maker").unwrap_or("");
                note(w, &by, maker, &format!("I took your handoff {id}."), Some(from));
                let qualified = format!("@{from}:{}", local_of(&by));
                send(w, To::All, Kind::Handoff, body_of(&r, "taken", Some(&qualified)));
            } else if let Some(now_row) = store(w, |s| s.handoff(&id))? {
                // Taken already: the asking host hears by whom.
                let taker = now_row.text("taken_by").unwrap_or("");
                let qualified = if is_remote(&Arg::text(taker)) { taker.to_string() } else { format!("@{}:{taker}", me.host) };
                let state = now_row.text("state").unwrap_or("taken").to_string();
                send(w, To::Host(from.to_string()), Kind::Handoff, body_of(&now_row, if state == "lapsed" { "lapsed" } else { "taken" }, Some(&qualified)));
            }
        }
        // The making host decided.
        "taken" | "lapsed" => {
            let Some(r) = store(w, |s| s.handoff(&id))? else { return Ok(()) };
            if r.text("host") != Some(from) {
                return Ok(());
            }
            let by = s(b, "by").map(|x| here(&x));
            store(w, |s| s.settle_handoff(&id, &op, by.as_deref(), now))?;
            // The session that asked hears once, when it's decided.
            let was_open = r.text("state") == Some("open");
            if let Some(asker) = r.text("asked_by").filter(|_| was_open) {
                let maker = format!("@{from}:{}", r.text("maker").unwrap_or(""));
                let words = if op == "lapsed" {
                    format!("Handoff {id} lapsed before your accept was decided.")
                } else if by.as_deref() == Some(asker) {
                    format!("Handoff {id} is yours.\n{}", details(&r))
                } else {
                    format!("Handoff {id} was taken by {} before your accept.", by.as_deref().unwrap_or("another session"))
                };
                note(w, &maker, asker, &words, Some(from));
            }
        }
        _ => return Err(Thrown::new(format!("a handoff op XenoNet doesn't know: {op}"))),
    }
    Ok(())
}

/// Handoffs made here that no one took in a week lapse, and their makers
/// are told.
fn lapse(w: &World) {
    let now = now_ms();
    let Ok(rows) = store(w, |s| s.lapsing_handoffs(now)) else { return };
    for r in rows {
        let id = r.text("id").unwrap_or("").to_string();
        let _ = store(w, |s| s.settle_handoff(&id, "lapsed", None, now));
        if r.text("host").is_none() {
            let maker = r.text("maker").unwrap_or("");
            note(w, maker, maker, &format!("No one took your handoff {id} in a week, so it lapsed."), None);
            if net(w).me().is_some() {
                send(w, To::All, Kind::Handoff, body_of(&r, "lapsed", None));
            }
        }
    }
}

/// A message another host stored but no one there read by the end of the
/// outbox's week is unread: its sender sees that, not "stored" for good.
fn unread(w: &World) {
    let now = now_ms();
    let Ok(rows) = store(w, |s| s.unread_since(now - OUTBOX_MS, now)) else { return };
    for r in rows {
        let (Some(id), Some(agent)) = (r.num("message_id"), r.text("agent")) else { continue };
        w.emit(From::Engine, "receipt", json!({ "id": id, "agent": agent, "host": r.text("host"), "state": "unread", "reason": "no one there read it in a week" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_a_working_tree() {
        assert_eq!(inside("/w/atlas", "/w/atlas/src/a.ts").as_deref(), Some("src/a.ts"));
        assert_eq!(inside("/w/atlas/", "/w/atlas/src/a.ts").as_deref(), Some("src/a.ts"));
        assert_eq!(inside("/w/atlas", "/w/atlas-zoom/a.ts"), None);
        assert_eq!(inside("C:\\w\\atlas", "C:\\w\\atlas\\src\\a.ts").as_deref(), Some("src/a.ts"));
    }

    #[test]
    fn a_weak_check_stays_weak() {
        let env = envelope("laptop", "msg_1", 1.0, "@laptop:claude:abcdefgh", "agent", Some("Docs"), "session", "partial", 2.0);
        assert_eq!(env["auth"], json!({ "method": "host", "process": "n/a", "strength": "weak" }));
        assert_eq!(env["original"]["method"], "session");
        assert_eq!(env["transport"], "network");
        assert!(env["line"].as_str().unwrap().contains("on laptop, another machine in your XenoNet network"));
        assert!(own_id("claude:abcdefgh") && !own_id("@vm1:claude:abcdefgh") && !own_id(YOU) && !own_id("hello"));
    }
}
