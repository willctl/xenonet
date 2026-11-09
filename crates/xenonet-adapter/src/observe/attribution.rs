//! Which sessions an observed actor started
//! (docs/design/actors/interface.md, section 1, "attributed"): a Claude
//! Code or Codex process running under one of a product's own processes
//! (a Hermes agent host, a command Grok Bot's app runs) was started by
//! that actor. Never by timing alone: the process's ancestry says who
//! started it, and only then is the process matched to the session it
//! holds:
//!
//!   1. the engine's pins, which say which process holds which session;
//!   2. else the one session on the map of that app that began in the
//!      process's folder within two minutes of it, if exactly one did,
//!      while the process still ran, and that no other process holds.
//!
//! A process whose session isn't known after two minutes still counts, if
//! it ran long enough to hold one. One gone within seconds held none.

use super::{text, Core, ObserverHost, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use xenonet_base::json::{number, number_string};
use xenonet_base::log::js_slice;
use xenonet_base::paths::path_key;
use xenonet_base::sync::Locked;
use xenonet_base::text::truthy;

pub const SPAWN_WAIT_MS: f64 = 2.0 * 60e3;
// ps gives a process's start to the second, and a transcript's first line
// comes after it.
const SLACK_MS: f64 = 2000.0;
/// A process gone this soon after it started, whose session XenoNet never
/// knew, held none.
pub const LIVED_MS: f64 = 15e3;
const DONE_MAX: usize = 2048;
const WEEK_MS: f64 = 7.0 * 24.0 * 3600e3;

/// A census row as the attribution reads it.
#[derive(Clone, Debug)]
pub struct Proc {
    pub row: Value,
    pub pid: f64,
    pub ppid: f64,
    pub start: f64,
}

impl Proc {
    pub fn of(row: Value) -> Self {
        let n = |k: &str| row.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
        Proc { pid: n("pid"), ppid: n("ppid"), start: n("start"), row }
    }
}

/// An agent process a product's processes started.
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    pub pid: f64,
    pub start: f64,
    pub agent: String,
    pub root: f64,
}

/// The agent processes in a census that a product's processes started:
/// the nearest process of note above each is one `is_root` picks, with no
/// other agent between (a session's own children are that session's).
/// `agent_of(i)`: what the census says row i's agent is.
pub fn started_under(table: &[Proc], agent_of: &mut dyn FnMut(usize) -> Option<String>, is_root: &mut dyn FnMut(usize) -> bool) -> Vec<Started> {
    let at = |pid: f64| table.iter().rposition(|p| p.pid == pid);
    let mut out = Vec::new();
    for (i, p) in table.iter().enumerate() {
        let Some(agent) = agent_of(i).filter(|a| a == "claude" || a == "codex") else { continue };
        let mut n = 0;
        let mut q = if p.ppid > 1.0 { at(p.ppid) } else { None };
        while let Some(j) = q {
            if n >= 64 {
                break;
            }
            if agent_of(j).is_some() {
                break;
            }
            if is_root(j) {
                out.push(Started { pid: p.pid, start: p.start, agent: agent.clone(), root: table[j].pid });
                break;
            }
            let pp = table[j].ppid;
            q = if pp > 1.0 { at(pp) } else { None };
            n += 1;
        }
    }
    out
}

/// A process seen, waiting for its session to be known.
#[derive(Clone, Debug)]
pub struct Pending {
    pub actor: String,
    pub pid: f64,
    pub start: f64,
    pub agent: String,
    pub cwd: Value,
    pub seen: f64,
    pub gone_at: Option<f64>,
}

impl Pending {
    pub fn json(&self) -> Value {
        json!({ "actor": self.actor, "pid": number(self.pid), "start": number(self.start), "agent": self.agent, "cwd": self.cwd, "seen": number(self.seen), "goneAt": self.gone_at.map(number) })
    }
}

/// A set that keeps the order things went in.
#[derive(Clone, Debug, Default)]
pub struct Ordered {
    items: Vec<String>,
    index: HashSet<String>,
}

impl Ordered {
    pub fn has(&self, k: &str) -> bool {
        self.index.contains(k)
    }
    pub fn add(&mut self, k: String) {
        if self.index.insert(k.clone()) {
            self.items.push(k);
        }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn items(&self) -> &[String] {
        &self.items
    }
    pub fn delete(&mut self, k: &str) -> bool {
        if self.index.remove(k) {
            self.items.retain(|x| x != k);
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.index.clear();
    }
    // The newer half, once there are many.
    fn trim(&mut self) {
        self.trim_to(DONE_MAX);
    }

    /// The newer half, once there are more than `max`.
    pub fn trim_to(&mut self, max: usize) {
        if self.items.len() > max {
            let keep = self.items.split_off(self.items.len() - max / 2);
            *self = Ordered { index: keep.iter().cloned().collect(), items: keep };
        }
    }
}

/// A process settled: whose, and the session it held, if known.
#[derive(Clone, Debug)]
pub struct Settled {
    pub actor: String,
    pub agent: String,
    pub cwd: Value,
    pub start: f64,
    pub session: Option<String>,
}

impl Settled {
    pub fn json(&self) -> Value {
        json!({ "actor": self.actor, "agent": self.agent, "cwd": self.cwd, "start": number(self.start), "session": self.session })
    }
}

/// The processes seen, each waiting up to two minutes for its session to
/// be known.
#[derive(Default)]
pub struct Spawns {
    pub pending: Vec<(String, Pending)>,
    pub done: Ordered,
    pub claimed: Ordered,
}

fn key(pid: f64, start: f64) -> String {
    format!("{}:{}", number_string(pid), number_string(start))
}

/// What a candidate for a Spawns saw() is: its pid, start and agent, and
/// its folder when the observer knows it already (None: not given).
#[derive(Clone, Debug)]
pub struct Candidate {
    pub pid: f64,
    pub start: f64,
    pub agent: String,
    pub cwd: Option<Value>,
}

impl Spawns {
    /// Which of these agent processes the actor started are new: each is
    /// taken once. Their folders, where not given, are read in one call.
    pub fn fresh(&self, list: &[Candidate]) -> Vec<Candidate> {
        list.iter()
            .filter(|p| {
                let k = key(p.pid, p.start);
                !self.pending.iter().any(|(x, _)| *x == k) && !self.done.has(&k)
            })
            .cloned()
            .collect()
    }

    /// The new ones taken, with the folders read for those that needed one
    /// (`cwds`: (pid, folder) pairs).
    pub fn take(&mut self, actor: &str, fresh: Vec<Candidate>, cwds: &[(f64, String)], at: f64) {
        let cwd_of = |pid: f64| cwds.iter().rev().find(|(p, _)| *p == pid).map(|(_, c)| Value::String(c.clone()));
        for p in fresh {
            let k = key(p.pid, p.start);
            let cwd = p.cwd.clone().unwrap_or_else(|| cwd_of(p.pid).unwrap_or(Value::Null));
            let item = Pending { actor: actor.into(), pid: p.pid, start: p.start, agent: p.agent, cwd, seen: at, gone_at: None };
            match self.pending.iter_mut().find(|(x, _)| *x == k) {
                Some(slot) => slot.1 = item,
                None => self.pending.push((k, item)),
            }
        }
    }
}

/// The state of a Spawns, shared by the calls on it; never held across a
/// call on the host.
pub type SpawnsState = std::sync::Mutex<Spawns>;

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(f64::NAN)
}

/// The session a process holds, if XenoNet can say: its pin, else the one
/// session of its app that began in its folder just after it did, before
/// the process was seen gone, and that no other process is pinned to (a
/// session of yours, say, begun nearby while the actor's process ran).
pub fn session_of(state: &SpawnsState, h: &dyn ObserverHost, s: &Pending) -> Result<Option<String>> {
    let claimed = |id: &str| state.locked().claimed.has(id);
    let mut pins: Vec<Value> = h
        .spawns()
        .pinned_to(s.pid)?
        .into_iter()
        .filter(|pin| {
            let agent = pin.get("agent").and_then(Value::as_str).unwrap_or("");
            (num(pin.get("start")) - s.start).abs() <= SLACK_MS && agent.starts_with(&format!("{}:", s.agent)) && !claimed(agent)
        })
        .collect();
    if !pins.is_empty() {
        xenonet_base::sort::sort_by(&mut pins, |a, b| {
            let d = |k: &str| num(b.get(k)) - num(a.get(k));
            let first = d("sure");
            let v = if first != 0.0 && !first.is_nan() { first } else { d("at") };
            if v > 0.0 {
                std::cmp::Ordering::Greater
            } else if v < 0.0 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        return Ok(pins[0].get("agent").and_then(Value::as_str).map(str::to_string));
    }
    let Some(cwd) = s.cwd.as_str().filter(|c| !c.is_empty()) else { return Ok(None) };
    let k = path_key(cwd);
    let until = js_min(s.start + SPAWN_WAIT_MS, s.gone_at.map_or(f64::INFINITY, |g| g + SLACK_MS));
    let mut found = Vec::new();
    for x in h.spawns().sessions()? {
        let Some(at) = x.get("createdAt").and_then(Value::as_f64) else { continue };
        let key_of = |f: &str| x.get(f).and_then(Value::as_str).and_then(path_key);
        let id = x.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        if x.get("app").and_then(Value::as_str) == Some(s.agent.as_str())
            && at >= s.start - SLACK_MS
            && at <= until
            && (key_of("cwd") == k || key_of("worktree") == k)
            && !claimed(&id)
            && !held_elsewhere(h, &id, s)?
        {
            found.push(id);
        }
    }
    Ok(if found.len() == 1 { Some(found.remove(0)) } else { None })
}

// Whether a session is pinned to a process other than this one.
fn held_elsewhere(h: &dyn ObserverHost, session: &str, s: &Pending) -> Result<bool> {
    Ok(h.spawns().pins(session)?.iter().any(|pin| pin.get("pid").and_then(Value::as_f64) != Some(s.pid) || (num(pin.get("start")) - s.start).abs() > SLACK_MS))
}

/// Whether a process still runs, as settle asks it (None: every one is
/// taken to).
pub type Alive<'a> = Option<&'a mut dyn FnMut(&Pending) -> bool>;

/// The processes settled now: those whose session is known, and those that
/// waited two minutes without, which count only if they ran long enough to
/// hold one.
pub fn settle(state: &SpawnsState, h: &dyn ObserverHost, at: f64, mut alive: Alive<'_>) -> Result<Vec<Settled>> {
    let mut out = Vec::new();
    let keys: Vec<String> = state.locked().pending.iter().map(|(k, _)| k.clone()).collect();
    for k in keys {
        let Some(mut s) = state.locked().pending.iter().find(|(x, _)| *x == k).map(|(_, p)| p.clone()) else { continue };
        if s.gone_at.is_none() {
            if let Some(f) = alive.as_mut() {
                if !f(&s) {
                    s.gone_at = Some(at);
                    if let Some(slot) = state.locked().pending.iter_mut().find(|(x, _)| *x == k) {
                        slot.1.gone_at = Some(at);
                    }
                }
            }
        }
        let session = session_of(state, h, &s)?;
        if session.is_none() && at - s.seen < SPAWN_WAIT_MS {
            continue;
        }
        {
            let mut st = state.locked();
            st.pending.retain(|(x, _)| *x != k);
            st.done.add(k);
            if let Some(id) = &session {
                st.claimed.add(id.clone());
            }
        }
        if session.is_none() && s.gone_at.is_some_and(|g| g - s.start < LIVED_MS) {
            continue;
        }
        out.push(Settled { actor: s.actor, agent: s.agent, cwd: s.cwd, start: s.start, session });
    }
    let mut st = state.locked();
    st.done.trim();
    st.claimed.trim();
    Ok(out)
}

/// The agent processes the actor started, as startedUnder finds them: each
/// taken once, with the folder it works in.
pub fn saw(state: &SpawnsState, h: &dyn ObserverHost, actor: &str, list: &[Candidate], at: f64) {
    let fresh = state.locked().fresh(list);
    if fresh.is_empty() {
        return;
    }
    let unknown: Vec<f64> = fresh.iter().filter(|p| p.cwd.is_none()).map(|p| p.pid).collect();
    let cwds = if unknown.is_empty() { Vec::new() } else { h.procs().cwds(&unknown) };
    state.locked().take(actor, fresh, &cwds, at);
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// A census as the attribution reads it: its rows, and what each is.
pub struct Census {
    pub table: Vec<Proc>,
    pub agents: Vec<Option<String>>,
    pub products: Vec<Value>,
}

impl Census {
    /// From the process table's reading: {table: [[pid, row]], of:
    /// [{agent, product}] for each row}.
    pub fn of(v: &Value) -> Self {
        let rows: Vec<Proc> =
            v.get("table").and_then(Value::as_array).map(|l| l.iter().map(|pair| Proc::of(pair.get(1).cloned().unwrap_or(Value::Null))).collect()).unwrap_or_default();
        let of: Vec<Value> = v.get("of").and_then(Value::as_array).cloned().unwrap_or_default();
        let agents = (0..rows.len()).map(|i| of.get(i).and_then(|v| v.get("agent")).and_then(Value::as_str).map(str::to_string)).collect();
        let products = (0..rows.len()).map(|i| of.get(i).and_then(|v| v.get("product")).cloned().unwrap_or(Value::Null)).collect();
        Census { table: rows, agents, products }
    }

    pub fn get(&self, pid: f64) -> Option<&Proc> {
        self.table.iter().rev().find(|p| p.pid == pid)
    }

    /// What the census says of row i's product: its id and role, when it's
    /// a product's.
    pub fn product_role(&self, i: usize) -> (Option<&str>, Option<&str>) {
        let p = &self.products[i];
        (p.get("product").and_then(Value::as_str), p.get("role").and_then(Value::as_str))
    }
}

/// The census, read for an observer: the table and what each process is,
/// with whether the table could be read ("ok", "unsupported" or
/// "unreadable") in `table_state`.
pub fn census(h: &dyn ObserverHost, table_state: &mut Option<String>) -> Result<Census> {
    match h.procs().census() {
        Ok(c) => {
            *table_state = Some(if h.procs().supported() { "ok" } else { "unsupported" }.into());
            Ok(c)
        }
        Err(e) => {
            *table_state = Some("unreadable".into());
            Err(e)
        }
    }
}

/// A process the attribution settled, recorded as the session it started
/// (or the place, when its session never showed).
pub fn spawned(h: &dyn ObserverHost, core: &Core, actor: &str, s: &Settled) -> Result<()> {
    let o = match &s.session {
        Some(session) => json!({
            "ref": format!("spawned:{session}"),
            "kind": "spawned",
            "target": session,
            "app": s.agent,
            "via": "attributed",
            "at": number(s.start),
        }),
        None => {
            let key = s.cwd.as_str().and_then(path_key).unwrap_or_default();
            let r = format!("spawned:{}:{}:{key}", s.agent, number_string(s.start));
            let target = match s.cwd.as_str().filter(|c| !c.is_empty()) {
                Some(cwd) => h.actors().place_of(cwd),
                None => Value::Null,
            };
            json!({
                "ref": js_slice(&r, 200),
                "kind": "spawned",
                "target": target,
                "app": s.agent,
                "via": "attributed",
                "at": number(s.start),
            })
        }
    };
    core.happened(h, actor, &o)?;
    Ok(())
}

/// A waiting spawned report, as the actors list them.
struct Wait {
    index: usize,
    actor: String,
    app: Value,
    place: Value,
    own: Vec<Value>,
    at: f64,
}

fn waits(list: Vec<Value>) -> Vec<Wait> {
    list.into_iter()
        .enumerate()
        .map(|(index, w)| Wait {
            index,
            actor: w.get("actor").map_or_else(|| "undefined".into(), text),
            app: w.get("app").cloned().unwrap_or(Value::Null),
            place: w.get("place").cloned().unwrap_or(Value::Null),
            own: w.get("own").and_then(Value::as_array).cloned().unwrap_or_default(),
            at: num(w.get("at")),
        })
        .collect()
}

/// A registered actor's spawned reports that named no session, only an app
/// and a place, each waiting up to two minutes for a session to start
/// under the actor's own processes in that place. When one does, it's
/// recorded again: a second event, attributed, with the session for its
/// target.
pub fn reported_look(state: &SpawnsState, h: &dyn ObserverHost, at: f64) -> Result<()> {
    let actors = h.actors();
    let waiting = waits(actors.expire_waiting(at));
    if waiting.is_empty() {
        state.locked().pending.clear();
        return Ok(());
    }
    let census = h.procs().census()?;
    let mut by_actor: Vec<(String, Vec<&Wait>)> = Vec::new();
    for w in &waiting {
        match by_actor.iter_mut().find(|(a, _)| *a == w.actor) {
            Some((_, l)) => l.push(w),
            None => by_actor.push((w.actor.clone(), vec![w])),
        }
    }
    for (actor, l) in &by_actor {
        let mut roots: Vec<f64> = actors.pids_of(actor);
        for w in l {
            roots.extend(w.own.iter().filter_map(Value::as_f64));
        }
        let from = l.iter().map(|w| w.at).fold(f64::INFINITY, js_min) - SPAWN_WAIT_MS;
        let mut agent_of = |i: usize| census.agents[i].clone();
        let mut is_root = |i: usize| roots.contains(&census.table[i].pid);
        let started: Vec<Candidate> = started_under(&census.table, &mut agent_of, &mut is_root)
            .into_iter()
            .filter(|p| p.start >= from && l.iter().any(|w| !truthy(Some(&w.app)) || w.app.as_str() == Some(p.agent.as_str())))
            .map(|p| Candidate { pid: p.pid, start: p.start, agent: p.agent, cwd: None })
            .collect();
        if !started.is_empty() {
            saw(state, h, actor, &started, at);
        }
    }
    let mut alive = |p: &Pending| census.get(p.pid).is_some_and(|q| q.start == p.start);
    let settled = settle(state, h, at, Some(&mut alive))?;
    for s in settled {
        let Some(session) = s.session.clone() else { continue };
        // A session it named itself already isn't the one it didn't.
        let named = actors
            .targets_of(&s.actor)
            .iter()
            .any(|t| t.get("id").and_then(Value::as_str) == Some(session.as_str()) && t.get("relation").and_then(Value::as_str) == Some("spawned"));
        if named {
            continue;
        }
        let place = match s.cwd.as_str().filter(|c| !c.is_empty()) {
            Some(cwd) => actors.place_of(cwd),
            None => Value::Null,
        };
        let live = waits(actors.waiting());
        let found = live.iter().find(|w| {
            w.actor == s.actor
                && (!truthy(Some(&w.app)) || w.app.as_str() == Some(s.agent.as_str()))
                && (w.place.is_null() || w.place == place)
                && (s.start - w.at).abs() <= SPAWN_WAIT_MS
        });
        let Some(w) = found else { continue };
        actors.drop_waiting_at(w.index);
        let r = format!("spawned:{session}");
        if !actors.has_ref(&s.actor, &r, at - WEEK_MS)? {
            let o = json!({
                "kind": "spawned",
                "target": session,
                "app": s.agent,
                "via": "attributed",
                "at": number(s.start),
                "ref": r,
            });
            actors.record(&s.actor, &o)?;
        }
    }
    Ok(())
}
