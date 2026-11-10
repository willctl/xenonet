//! Grok Bot's observer (docs/design/actors/grok-bot.md): presence and
//! process ancestry, nothing more. Its desktop app is the Bots' only way
//! onto this Mac, so:
//!
//!   working   a command its app runs is running, or ran in the last 30 s
//!   idle      its app runs
//!   unknown   its app isn't running: Bots go on in the cloud with it
//!             closed, and nothing here can say they stopped, so it's never
//!             offline, and it stays listed for as long as it's installed
//!
//! A command is a process the app starts whose program lies outside its
//! bundle. Each counts as ran, gathered per minute in the project holding
//! the folder it started in: by count and place alone, never its arguments.
//! One that is claude or codex, or starts one, started a session (spawned,
//! matched to the session as attribution does).
//!
//! It reads the process table, where its commands work, and of its files
//! only whether the app is there, and two keys of its Info.plist. While the
//! app runs, what it runs is looked at every 3 seconds with pgrep.

use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_adapter::observe::attribution::{self, census, spawned, Candidate, Census, Pending, SpawnsState};
use xenonet_adapter::observe::{known, AppBundle, Core, Gather, Keep, Observer, ObserverHost, Result, Say};
use xenonet_adapter::products::Product;
use xenonet_os::sync::Locked;
use xenonet_text::text::{display, truthy};

pub const ID: &str = "actor:grok-bot";
pub const POLL_MS: f64 = 3000.0;
// It stays working this long after its last command, so a run of quick
// ones is one stretch of work.
const LINGER_MS: f64 = 30e3;
const TRACKED_MAX: usize = 512;
const STRAY_MS: f64 = 60e3;
// What a Bot can do, as grok-bot.md has it.
const CAPABILITIES: [&str; 5] = ["run-commands", "browse", "message-people", "message-agents", "schedule"];

fn in_bundle(program: &Value) -> bool {
    let p = match program {
        Value::Null => String::new(),
        v => display(v),
    };
    p.contains("/Grok Bot.app/")
}

/// What's known of a process's agent: not yet (a Node or Bun process the
/// poll saw, whose arguments the next census reads), none, or one.
#[derive(Clone, Debug, PartialEq)]
enum Agent {
    Unknown,
    None,
    Is(String),
}

impl Agent {
    fn of(a: Option<&str>) -> Self {
        a.map_or(Agent::None, |a| Agent::Is(a.into()))
    }
    fn name(&self) -> Option<&str> {
        match self {
            Agent::Is(a) => Some(a),
            _ => None,
        }
    }
}

// A process below the app, seen for the first time.
#[derive(Clone, Debug)]
struct Entry {
    start: f64,
    ppid: f64,
    command: bool,
    agent: Agent,
    handed: bool,
    place: Value,
    cwd: Option<Value>,
    started_at: f64,
}

#[derive(Default)]
struct State {
    app_pids: Vec<f64>,
    tracked: Vec<(f64, Entry)>,
    busy_at: Option<f64>,
    was_installed: bool,
    table_status: Option<String>,
    looked_at: Option<f64>,
}

impl State {
    fn entry(&mut self, pid: f64) -> Option<&mut Entry> {
        self.tracked.iter_mut().find(|(p, _)| *p == pid).map(|(_, e)| e)
    }
    fn get(&self, pid: f64) -> Option<&Entry> {
        self.tracked.iter().find(|(p, _)| *p == pid).map(|(_, e)| e)
    }
    fn is_app(&self, pid: f64) -> bool {
        self.app_pids.contains(&pid)
    }
    fn add_app(&mut self, pid: f64) {
        if !self.is_app(pid) {
            self.app_pids.push(pid);
        }
    }

    // A process below the app, seen for the first time: a command when the
    // app started it, and either way watched for a session it starts.
    fn track(&mut self, pid: f64, start: f64, parent: f64, agent: Agent, at: f64) -> bool {
        if self.get(pid).is_some_and(|e| e.start == start) {
            return false;
        }
        let started = if start != 0.0 && !start.is_nan() { start } else { at };
        let e = Entry { start, ppid: parent, command: self.is_app(parent), agent, handed: false, place: Value::Null, cwd: None, started_at: started.min(at) };
        match self.entry(pid) {
            Some(slot) => *slot = e,
            None => self.tracked.push((pid, e)),
        }
        true
    }

    // What's above a tracked process, among what's tracked.
    fn above(&self, e: &Entry) -> &'static str {
        let mut q = self.get(e.ppid);
        let mut n = 0;
        while let Some(x) = q {
            if n >= 32 {
                break;
            }
            match x.agent {
                Agent::Is(_) => return "agent",
                Agent::Unknown => return "unknown",
                Agent::None => {}
            }
            q = self.get(x.ppid);
            n += 1;
        }
        "clear"
    }
}

pub struct GrokBot {
    core: Core,
    app: AppBundle,
    state: Mutex<State>,
    spawns: SpawnsState,
}

impl Default for GrokBot {
    fn default() -> Self {
        Self::new()
    }
}

impl GrokBot {
    pub fn new() -> Self {
        GrokBot {
            core: Core::new(crate::product()),
            app: AppBundle::new("Grok Bot.app", r"^com\.anysphere\.sand(\.|$)"),
            state: Mutex::new(State::default()),
            spawns: SpawnsState::default(),
        }
    }

    fn s(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.locked()
    }

    // The new commands' places (one lsof for them all), each counted as ran
    // in its minute; and the sessions among `candidates`, each handed to
    // attribution once.
    fn taken(&self, h: &dyn ObserverHost, fresh: &[f64], at: f64, candidates: &[f64]) {
        let actors = h.actors();
        let commands: Vec<f64> = fresh.iter().copied().filter(|p| self.s().get(*p).is_some_and(|e| e.command)).collect();
        if !commands.is_empty() {
            let cwds = h.procs().cwds(&commands);
            for pid in commands {
                let cwd = cwds.iter().rev().find(|(p, _)| *p == pid).map_or(Value::Null, |(_, c)| Value::String(c.clone()));
                let place = match cwd.as_str().filter(|c| !c.is_empty()) {
                    Some(c) => actors.place_of(c),
                    None => Value::Null,
                };
                let started_at = {
                    let mut s = self.s();
                    s.busy_at = Some(s.busy_at.unwrap_or(0.0).max(at));
                    let Some(e) = s.entry(pid) else { continue };
                    e.place = place.clone();
                    e.cwd = Some(cwd);
                    e.started_at
                };
                if actors.has(ID) {
                    self.core.gather(ID, &Gather::new("ran", place, started_at));
                }
            }
        }
        let sessions: Vec<(f64, Entry)> = {
            let s = self.s();
            candidates.iter().filter_map(|p| s.get(*p).map(|e| (*p, e.clone()))).filter(|(_, e)| e.agent.name().is_some() && !e.handed && s.above(e) == "clear").collect()
        };
        if sessions.is_empty() || !actors.has(ID) {
            return;
        }
        {
            let mut s = self.s();
            for (pid, _) in &sessions {
                if let Some(e) = s.entry(*pid) {
                    e.handed = true;
                }
            }
        }
        let list: Vec<Candidate> =
            sessions.iter().filter_map(|(pid, e)| Some(Candidate { pid: *pid, start: e.start, agent: e.agent.name()?.into(), cwd: e.cwd.clone() })).collect();
        attribution::saw(&self.spawns, h, ID, &list, at);
    }

    // What it does now, in XenoNet's words, from what the app runs.
    fn say(&self, h: &dyn ObserverHost, at: f64, census: &Census) -> Result<()> {
        let (app_runs, latest, busy_at) = {
            let mut s = self.s();
            let app_runs = !s.app_pids.is_empty();
            let mut live: Vec<Entry> = s.tracked.iter().filter(|(_, e)| e.command).map(|(_, e)| e.clone()).collect();
            if !live.is_empty() {
                s.busy_at = Some(s.busy_at.unwrap_or(0.0).max(at));
            }
            xenonet_text::sort::sort_by(&mut live, |a, b| b.started_at.partial_cmp(&a.started_at).unwrap_or(std::cmp::Ordering::Equal));
            (app_runs, live.into_iter().next(), s.busy_at)
        };
        let (presence, status) = if app_runs && busy_at.is_some_and(|b| at - b < LINGER_MS) {
            let place = latest.as_ref().map_or(Value::Null, |e| e.place.clone());
            let name = h.actors().where_of(&place).get("name").cloned().unwrap_or(Value::Null);
            let status = if truthy(Some(&name)) { format!("Running commands in {}", display(&name)) } else { "Running commands".into() };
            if truthy(Some(&place)) {
                self.core.touch(h, ID, &place, "works-on", at)?;
            }
            ("working", status)
        } else if app_runs {
            ("idle", "Its app is open on this Mac".to_string())
        } else {
            ("unknown", "Its app on this Mac is closed".to_string())
        };
        self.core.say(h, ID, &Say { presence, status: Some(status), usage: None, at })?;
        // One the table doesn't hold may have started since it was read:
        // the system says whether it runs.
        let mut alive = |p: &Pending| match census.get(p.pid) {
            Some(row) => row.start == p.start,
            None => h.procs().alive(p.pid),
        };
        for s in attribution::settle(&self.spawns, h, at, Some(&mut alive))? {
            spawned(h, &self.core, ID, &s)?;
        }
        Ok(())
    }

    /// The processes tracked, as [pid, what's known of it], and the app's
    /// own, for the tests that look in.
    pub fn tracked(&self) -> (Vec<Value>, Vec<f64>) {
        let s = self.s();
        let agent = |a: &Agent| match a {
            Agent::Unknown | Agent::None => Value::Null,
            Agent::Is(x) => json!(x),
        };
        let num = xenonet_text::json::number;
        let items = s
            .tracked
            .iter()
            .map(|(pid, e)| json!([num(*pid), { "start": num(e.start), "ppid": num(e.ppid), "command": e.command, "agent": agent(&e.agent), "handed": e.handed, "place": e.place, "startedAt": num(e.started_at) }]))
            .collect();
        (items, s.app_pids.clone())
    }
}

impl Observer for GrokBot {
    fn name(&self) -> &'static str {
        "grok-bot-observer"
    }

    fn product(&self) -> &'static Product {
        self.core.product()
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn look(&self, h: &dyn ObserverHost, at: f64) -> Result<()> {
        let known = known(h, ID);
        // Whether the app where apps go is Cursor's, from its Info.plist.
        self.app.read(h);
        // Not installed where apps go and not seen before: the process
        // table is read only once a minute.
        if !known && !self.app.installed(h) {
            let mut s = self.s();
            if at - s.looked_at.unwrap_or(f64::NEG_INFINITY) < STRAY_MS {
                return Ok(());
            }
            s.looked_at = Some(at);
        }
        let mut table_state = None;
        let census = census(h, &mut table_state);
        self.s().table_status = table_state;
        let census: Census = census?;
        let is_app = |i: usize| census.product_role(i) == (Some("grok-bot"), Some("app"));
        let apps: Vec<usize> = (0..census.table.len()).filter(|&i| is_app(i)).collect();
        let installed = !apps.is_empty() || self.app.installed(h);
        self.s().was_installed = installed;
        if !installed && !known {
            return Ok(());
        }
        // The actor first, so the commands of its first look count.
        let version = if self.app.ours() { self.app.plist().and_then(|p| p.version) } else { None };
        let keep =
            Keep { name: "Grok Bot".into(), version, via: vec!["observed", "attributed"], heartbeat_ms: None, capabilities: CAPABILITIES.to_vec(), installed: Some(installed), at };
        self.core.keep(h, ID, &keep)?;
        if !h.actors().enabled(ID) {
            return Ok(());
        }
        // The app's processes, and everything below them, from the table.
        let (fresh, candidates) = {
            let mut s = self.s();
            s.app_pids = apps.iter().map(|&i| census.table[i].pid).collect();
            let mut queue: Vec<usize> = apps;
            let mut seen: Vec<f64> = s.app_pids.clone();
            let mut fresh = Vec::new();
            let mut candidates = Vec::new();
            let mut i = 0;
            while i < queue.len() && i < 4096 {
                let q = &census.table[queue[i]];
                let kids: Vec<usize> = (0..census.table.len()).filter(|&k| census.table[k].ppid == q.pid).collect();
                for k in kids {
                    let child = &census.table[k];
                    if seen.contains(&child.pid) {
                        continue;
                    }
                    seen.push(child.pid);
                    queue.push(k);
                    if in_bundle(child.row.get("program").unwrap_or(&Value::Null)) {
                        s.add_app(child.pid);
                        continue;
                    }
                    let agent = Agent::of(census.agents[k].as_deref());
                    if s.track(child.pid, child.start, q.pid, agent.clone(), at) {
                        fresh.push(child.pid);
                    }
                    // One the poll saw first may be a Node or Bun process it
                    // couldn't tell: the census has read its arguments.
                    if let Some(e) = s.entry(child.pid) {
                        if e.agent == Agent::Unknown {
                            e.agent = agent;
                        }
                        if e.agent.name().is_some() && !e.handed {
                            candidates.push(child.pid);
                        }
                    }
                }
                i += 1;
            }
            (fresh, candidates)
        };
        // What's gone is let go: what the table no longer holds, or holds a
        // newer process at its pid; one it doesn't hold goes only once it
        // no longer runs.
        let tracked: Vec<(f64, f64)> = self.s().tracked.iter().map(|(p, e)| (*p, e.start)).collect();
        for (pid, start) in tracked {
            let row = census.get(pid);
            if row.is_some_and(|r| r.start == start) {
                continue;
            }
            if row.is_none_or(|r| r.start < start) && h.procs().alive(pid) {
                continue;
            }
            self.s().tracked.retain(|(p, _)| *p != pid);
        }
        {
            let mut s = self.s();
            if s.tracked.len() > TRACKED_MAX {
                let at = s.tracked.len() - TRACKED_MAX / 2;
                let keep = s.tracked.split_off(at);
                s.tracked = keep;
            }
        }
        self.taken(h, &fresh, at, &candidates);
        self.say(h, at, &census)
    }

    /// What the doctor shows: whether the app is installed, and what this
    /// observer reads.
    fn status(&self, h: &dyn ObserverHost) -> Result<Value> {
        let (was, table) = {
            let s = self.s();
            (s.was_installed, s.table_status.clone().unwrap_or_else(|| "ok".into()))
        };
        let installed = self.app.installed(h) || was;
        let mut sources = vec![json!({ "name": "processes", "file": null, "state": table })];
        sources.extend(self.app.sources(h));
        Ok(json!({ "product": "grok-bot", "name": "Grok Bot", "installed": installed, "sources": sources, "recognized": null }))
    }

    fn poll_ms(&self) -> Option<f64> {
        Some(POLL_MS)
    }

    /// Whether to poll: its observer on, and its app running.
    fn may_poll(&self, h: &dyn ObserverHost) -> bool {
        h.observing() && !self.s().app_pids.is_empty()
    }

    /// Every 3 seconds while the app runs: its new children and theirs, by
    /// pgrep, described and placed only when new.
    fn poll(&self, h: &dyn ObserverHost, at: f64) -> Result<()> {
        let procs = h.procs();
        let tracked: Vec<f64> = self.s().tracked.iter().map(|(p, _)| *p).collect();
        for pid in tracked {
            if !procs.alive(pid) {
                self.s().tracked.retain(|(p, _)| *p != pid);
            }
        }
        let parents: Vec<f64> = {
            let s = self.s();
            s.app_pids.iter().chain(s.tracked.iter().map(|(p, _)| p)).copied().collect()
        };
        let kids = procs.children(&parents);
        let children: Vec<f64> = {
            let s = self.s();
            kids.into_iter().filter(|p| !s.is_app(*p) && s.get(*p).is_none()).collect()
        };
        if children.is_empty() {
            return Ok(());
        }
        let mut rows: Vec<Value> = procs.describe(&children);
        let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
        // Parents before children, so a command's own children know it.
        xenonet_text::sort::sort_by(&mut rows, |a, b| {
            let d = n(a, "start") - n(b, "start");
            let d = if d != 0.0 && !d.is_nan() { d } else { n(a, "pid") - n(b, "pid") };
            if d > 0.0 {
                std::cmp::Ordering::Greater
            } else if d < 0.0 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        let mut fresh = Vec::new();
        {
            let mut s = self.s();
            for p in rows {
                let (pid, ppid, start) = (n(&p, "pid"), n(&p, "ppid"), n(&p, "start"));
                if !s.is_app(ppid) && s.get(ppid).is_none() {
                    continue;
                }
                let program = p.get("program").cloned().unwrap_or(Value::Null);
                if in_bundle(&program) {
                    s.add_app(pid);
                    continue;
                }
                // Node and Bun run Claude Code and Codex too, which only
                // their arguments say: until the look's census reads those,
                // such a process's agent isn't known.
                let program = if program.is_null() { String::new() } else { display(&program) };
                let agent = match procs.agent_of(&program) {
                    Some(a) => Agent::Is(a),
                    None if procs.wants_args(&program) => Agent::Unknown,
                    None => Agent::None,
                };
                if s.track(pid, start, ppid, agent, at) {
                    fresh.push(pid);
                }
            }
        }
        self.taken(h, &fresh, at, &fresh);
        Ok(())
    }
}
