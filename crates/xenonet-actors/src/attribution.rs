//! Which sessions an observed actor started (core/actors/attribution.js;
//! docs/design/actors/interface.md, section 1, "attributed"): a Claude
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
//!
//! The store, the map, procs and the actors are the collaborators the JS
//! was given, called through `Calls` at the paths `Paths` names.

use serde_json::{json, Value};
use std::collections::HashSet;
use xenonet_base::paths::path_key;
use xenonet_engine::js::truthy;
use xenonet_engine::peer::{list, Calls, Result};
use xenonet_engine::sql::Arg;

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
pub fn started_under(table: &[Proc], agent_of: &mut dyn FnMut(usize) -> Result<Option<String>>, is_root: &mut dyn FnMut(usize) -> Result<bool>) -> Result<Vec<Started>> {
    let at = |pid: f64| table.iter().rposition(|p| p.pid == pid);
    let mut out = Vec::new();
    for (i, p) in table.iter().enumerate() {
        let agent = agent_of(i)?;
        let Some(agent) = agent.filter(|a| a == "claude" || a == "codex") else { continue };
        let mut n = 0;
        let mut q = if p.ppid > 1.0 { at(p.ppid) } else { None };
        while let Some(j) = q {
            if n >= 64 {
                break;
            }
            if agent_of(j)?.is_some() {
                break;
            }
            if is_root(j)? {
                out.push(Started { pid: p.pid, start: p.start, agent: agent.clone(), root: table[j].pid });
                break;
            }
            let pp = table[j].ppid;
            q = if pp > 1.0 { at(pp) } else { None };
            n += 1;
        }
    }
    Ok(out)
}

/// Where a Spawns' collaborators are among a module's: its store's pins,
/// the map's sessions, and procs' folders.
#[derive(Clone, Debug)]
pub struct Paths {
    pub store: String,
    pub map: String,
    pub procs: String,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { store: "store".into(), map: "map".into(), procs: "procs".into() }
    }
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
        json!({ "actor": self.actor, "pid": num(self.pid), "start": num(self.start), "agent": self.agent, "cwd": self.cwd, "seen": num(self.seen), "goneAt": self.gone_at.map(num) })
    }
}

fn num(n: f64) -> Value {
    xenonet_base::json::number(n)
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
        if self.items.len() > DONE_MAX {
            let keep = self.items.split_off(self.items.len() - DONE_MAX / 2);
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
        json!({ "actor": self.actor, "agent": self.agent, "cwd": self.cwd, "start": num(self.start), "session": self.session })
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
    format!("{}:{}", xenonet_base::json::number_string(pid), xenonet_base::json::number_string(start))
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

impl Candidate {
    pub fn of(a: &Arg) -> Self {
        let n = |k: &str| a.field(k).as_f64().unwrap_or(f64::NAN);
        Candidate {
            pid: n("pid"),
            start: n("start"),
            agent: xenonet_engine::store::js_text(&a.field("agent")),
            cwd: match a.field("cwd") {
                Arg::Undef => None,
                v => Some(v.to_json()),
            },
        }
    }
}

impl Spawns {
    /// Which of these agent processes the actor started are new: each is
    /// taken once. Their folders, where not given, are read in one call.
    pub fn fresh(&self, list: &[Candidate]) -> Vec<Candidate> {
        list.iter().filter(|p| { let k = key(p.pid, p.start); !self.pending.iter().any(|(x, _)| *x == k) && !self.done.has(&k) }).cloned().collect()
    }

    /// The new ones taken, with the folders read for those that needed one
    /// (`cwds`: [pid, folder] pairs).
    pub fn take(&mut self, actor: &str, fresh: Vec<Candidate>, cwds: &[Arg], at: f64) {
        let cwd_of = |pid: f64| {
            cwds.iter().rev().find(|p| p.as_list_item(0).as_f64() == Some(pid)).map(|p| p.as_list_item(1).to_json()).filter(|v| !v.is_null())
        };
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
/// call to a collaborator.
pub type SpawnsState = std::sync::Mutex<Spawns>;

/// The session a process holds, if XenoNet can say: its pin, else the one
/// session of its app that began in its folder just after it did, before
/// the process was seen gone, and that no other process is pinned to (a
/// session of yours, say, begun nearby while the actor's process ran).
pub fn session_of(state: &SpawnsState, calls: &mut dyn Calls, paths: &Paths, s: &Pending) -> Result<Option<String>> {
    let claimed = |id: &str| state.lock().unwrap().claimed.has(id);
    let mut pins: Vec<Arg> = list(calls.call(&paths.store, "pinnedTo", vec![Arg::Num(s.pid)])?)
        .into_iter()
        .filter(|pin| {
            let agent = pin.field("agent");
            (pin.field("start").as_f64().unwrap_or(f64::NAN) - s.start).abs() <= SLACK_MS && agent.as_str().is_some_and(|a| a.starts_with(&format!("{}:", s.agent))) && !claimed(agent.as_str().unwrap_or(""))
        })
        .collect();
    if !pins.is_empty() {
        pins.sort_by(|a, b| {
            let d = |k: &str| b.field(k).as_f64().unwrap_or(f64::NAN) - a.field(k).as_f64().unwrap_or(f64::NAN);
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
        return Ok(pins[0].field("agent").as_str().map(str::to_string));
    }
    let Some(cwd) = s.cwd.as_str().filter(|c| !c.is_empty()) else { return Ok(None) };
    let k = path_key(cwd);
    let until = js_min(s.start + SPAWN_WAIT_MS, s.gone_at.map_or(f64::INFINITY, |g| g + SLACK_MS));
    let sessions = opt_call(calls, &paths.map, "sessions")?;
    let mut found = Vec::new();
    for x in list(sessions) {
        let created = x.field("createdAt");
        let Some(at) = created.as_f64().filter(|_| !created.is_nullish()) else { continue };
        let key_of = |f: &str| x.field(f).as_str().and_then(path_key);
        let id = x.field("id");
        if x.field("app").as_str() == Some(s.agent.as_str()) && at >= s.start - SLACK_MS && at <= until && (key_of("cwd") == k || key_of("worktree") == k) && !claimed(id.as_str().unwrap_or("")) && !held_elsewhere(calls, paths, &id, s)? {
            found.push(id);
        }
    }
    Ok(if found.len() == 1 { found[0].as_str().map(str::to_string) } else { None })
}

// Whether a session is pinned to a process other than this one.
fn held_elsewhere(calls: &mut dyn Calls, paths: &Paths, session: &Arg, s: &Pending) -> Result<bool> {
    Ok(list(calls.call(&paths.store, "pins", vec![session.clone()])?).iter().any(|pin| pin.field("pid").as_f64() != Some(s.pid) || (pin.field("start").as_f64().unwrap_or(f64::NAN) - s.start).abs() > SLACK_MS))
}

/// Whether a process still runs, as settle asks it (None: every one is
/// taken to).
pub type Alive<'a> = Option<&'a mut dyn FnMut(&mut dyn Calls, &Pending) -> Result<bool>>;

/// The processes settled now: those whose session is known, and those that
/// waited two minutes without, which count only if they ran long enough to
/// hold one.
pub fn settle(state: &SpawnsState, calls: &mut dyn Calls, paths: &Paths, at: f64, mut alive: Alive) -> Result<Vec<Settled>> {
    let mut out = Vec::new();
    let keys: Vec<String> = state.lock().unwrap().pending.iter().map(|(k, _)| k.clone()).collect();
    for k in keys {
        let Some(mut s) = state.lock().unwrap().pending.iter().find(|(x, _)| *x == k).map(|(_, p)| p.clone()) else { continue };
        if s.gone_at.is_none() {
            if let Some(f) = alive.as_mut() {
                if !f(calls, &s)? {
                    s.gone_at = Some(at);
                    if let Some(slot) = state.lock().unwrap().pending.iter_mut().find(|(x, _)| *x == k) {
                        slot.1.gone_at = Some(at);
                    }
                }
            }
        }
        let session = session_of(state, calls, paths, &s)?;
        if session.is_none() && at - s.seen < SPAWN_WAIT_MS {
            continue;
        }
        {
            let mut st = state.lock().unwrap();
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
    let mut st = state.lock().unwrap();
    st.done.trim();
    st.claimed.trim();
    Ok(out)
}

/// The agent processes the actor started, as startedUnder finds them: each
/// taken once, with the folder it works in.
pub fn saw(state: &SpawnsState, calls: &mut dyn Calls, paths: &Paths, actor: &str, list: &[Candidate], at: f64) -> Result<()> {
    let fresh = state.lock().unwrap().fresh(list);
    if fresh.is_empty() {
        return Ok(());
    }
    let unknown: Vec<Arg> = fresh.iter().filter(|p| p.cwd.is_none()).map(|p| Arg::Num(p.pid)).collect();
    let cwds = if unknown.is_empty() { Vec::new() } else { list_of(calls.call(&paths.procs, "cwds", vec![Arg::List(unknown)])?) };
    state.lock().unwrap().take(actor, fresh, &cwds, at);
    Ok(())
}

trait ListItem {
    fn as_list_item(&self, i: usize) -> Arg;
}

impl ListItem for Arg {
    fn as_list_item(&self, i: usize) -> Arg {
        match self {
            Arg::List(l) => l.get(i).cloned().unwrap_or(Arg::Undef),
            _ => Arg::Undef,
        }
    }
}

fn list_of(a: Arg) -> Vec<Arg> {
    list(a)
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `<path>?.<method>?.()`: Undef when there's nothing to call.
pub fn opt_call(calls: &mut dyn Calls, path: &str, method: &str) -> Result<Arg> {
    if calls.get(path)?.is_nullish() {
        return Ok(Arg::Undef);
    }
    if !matches!(calls.get(&format!("{path}.{method}"))?, Arg::Other) {
        return Ok(Arg::Undef);
    }
    calls.call(path, method, vec![])
}

/// A census as the attribution reads it: its rows, and what each is.
pub struct Census {
    pub table: Vec<Proc>,
    pub agents: Vec<Option<String>>,
    pub products: Vec<Value>,
}

impl Census {
    /// From the twin's reading of procs.census(): {table: [[pid, row]],
    /// of: [{agent, product}] for each row}.
    pub fn of(a: &Arg) -> Self {
        let rows: Vec<Proc> = list(a.field("table")).iter().map(|pair| Proc::of(pair.as_list_item(1).to_json())).collect();
        let of = list(a.field("of"));
        let agents = (0..rows.len()).map(|i| of.get(i).and_then(|v| v.field("agent").as_str().map(str::to_string))).collect();
        let products = (0..rows.len()).map(|i| of.get(i).map_or(Value::Null, |v| v.field("product").to_json())).collect();
        Census { table: rows, agents, products }
    }

    pub fn get(&self, pid: f64) -> Option<&Proc> {
        self.table.iter().rev().find(|p| p.pid == pid)
    }
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

fn waits(a: Arg) -> Vec<Wait> {
    list(a)
        .into_iter()
        .enumerate()
        .map(|(index, w)| Wait {
            index,
            actor: xenonet_engine::store::js_text(&w.field("actor")),
            app: w.field("app").to_json(),
            place: w.field("place").to_json(),
            own: list(w.field("own")).iter().map(Arg::to_json).collect(),
            at: w.field("at").as_f64().unwrap_or(f64::NAN),
        })
        .collect()
}

/// A registered actor's spawned reports that named no session, only an app
/// and a place, each waiting up to two minutes for a session to start
/// under the actor's own processes in that place. When one does, it's
/// recorded again: a second event, attributed, with the session for its
/// target. Its collaborators: "actors" and "procs" (and the Spawns' own,
/// at `paths`); "census" reads procs.census() for it.
pub fn reported_look(state: &std::sync::Mutex<Spawns>, calls: &mut dyn Calls, paths: &Paths, at: f64) -> Result<()> {
    let waiting = waits(calls.call("actors", "expireWaiting", vec![Arg::Num(at)])?);
    if waiting.is_empty() {
        state.lock().unwrap().pending.clear();
        return Ok(());
    }
    let census = Census::of(&calls.call("", "census", vec![])?);
    let mut by_actor: Vec<(String, Vec<&Wait>)> = Vec::new();
    for w in &waiting {
        match by_actor.iter_mut().find(|(a, _)| *a == w.actor) {
            Some((_, l)) => l.push(w),
            None => by_actor.push((w.actor.clone(), vec![w])),
        }
    }
    for (actor, l) in &by_actor {
        let mut roots: Vec<f64> = list(calls.call("actors", "pidsOf", vec![Arg::text(actor)])?).iter().filter_map(Arg::as_f64).collect();
        for w in l {
            roots.extend(w.own.iter().filter_map(Value::as_f64));
        }
        let from = l.iter().map(|w| w.at).fold(f64::INFINITY, js_min) - SPAWN_WAIT_MS;
        let mut agent_of = |i: usize| Ok(census.agents[i].clone());
        let mut is_root = |i: usize| Ok(roots.contains(&census.table[i].pid));
        let started: Vec<Candidate> = started_under(&census.table, &mut agent_of, &mut is_root)?
            .into_iter()
            .filter(|p| p.start >= from && l.iter().any(|w| !truthy(Some(&w.app)) || w.app.as_str() == Some(p.agent.as_str())))
            .map(|p| Candidate { pid: p.pid, start: p.start, agent: p.agent, cwd: None })
            .collect();
        if !started.is_empty() {
            saw(state, calls, paths, actor, &started, at)?;
        }
    }
    let mut alive = |_: &mut dyn Calls, p: &Pending| Ok(census.get(p.pid).is_some_and(|q| q.start == p.start));
    let settled = settle(state, calls, paths, at, Some(&mut alive))?;
    for s in settled {
        let Some(session) = s.session.clone() else { continue };
        // A session it named itself already isn't the one it didn't.
        let named = list(calls.call("actors", "targetsOf", vec![Arg::text(&s.actor)])?).iter().any(|t| t.field("id").as_str() == Some(session.as_str()) && t.field("relation").as_str() == Some("spawned"));
        if named {
            continue;
        }
        let place = if truthy(Some(&s.cwd)) { calls.call("actors", "placeOf", vec![xenonet_actors_value(&s.cwd)])?.to_json() } else { Value::Null };
        let live = waits(calls.get("actors.waiting")?);
        let found = live.iter().find(|w| w.actor == s.actor && (!truthy(Some(&w.app)) || w.app.as_str() == Some(s.agent.as_str())) && (w.place.is_null() || w.place == place) && (s.start - w.at).abs() <= SPAWN_WAIT_MS);
        let Some(w) = found else { continue };
        calls.call("actors", "dropWaitingAt", vec![Arg::Num(w.index as f64)])?;
        let r = format!("spawned:{session}");
        let had = calls.call("actors.store", "hasActorRef", vec![Arg::text(&s.actor), Arg::text(&r), Arg::Num(at - WEEK_MS)])?;
        if !had.truthy() {
            let o = Arg::Obj(vec![
                ("kind".into(), Arg::text("spawned")),
                ("target".into(), Arg::text(&session)),
                ("app".into(), Arg::text(&s.agent)),
                ("via".into(), Arg::text("attributed")),
                ("at".into(), Arg::Num(s.start)),
                ("ref".into(), Arg::text(r)),
            ]);
            calls.call("actors", "record", vec![Arg::text(&s.actor), o])?;
        }
    }
    Ok(())
}

fn xenonet_actors_value(v: &Value) -> Arg {
    crate::registry::value_arg(v)
}

