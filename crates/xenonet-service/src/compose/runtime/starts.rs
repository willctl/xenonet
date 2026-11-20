//! Starting sessions (docs/guide/network.md), on
//! this machine and on the others in its network.
//!
//! - **Here:** xenonet-authz decides (`crate::compose::runtime::ground`) by this machine's
//!   start setting in the network's chain, the person's signature for a
//!   start as you, and the person's delegation (in their chain, naming this
//!   machine's device and the one the start came from) for a delegated
//!   one; the limits
//!   hold (20 starts an hour, 10 from one machine, and the runner's cap),
//!   the folder is one this machine knows, and the shared runner starts the
//!   app's CLI with the first prompt framed with who asked. A watcher tells
//!   whoever asked when the run ends, and whether Claude Code needs a
//!   sign-in.
//! - **Another machine:** a `message` frame to `xenonet:start` with the
//!   request as its JSON body, sent only while that machine is online and
//!   its presence says its XenoNet takes starts. A start as you carries
//!   your device's signature over the run it starts (app, folder, mode),
//!   which that machine checks against its copy of your identity. Its answers are receipts
//!   keyed by the frame's mid: started, refused, then ended. An older
//!   XenoNet refuses the target as it refuses any id it doesn't have,
//!   which reads here as too old. Each answer goes to whoever asked: a
//!   message to a session or an actor, or a line in your console.
//!
//! The requests sent from here are kept in memory: after a restart, a late
//! answer to one is let go. The runs other machines were asked for, and the
//! network each run started here for another machine came through, are kept
//! in the store (as two settings), for a day after last used and a week at
//! most.

use crate::compose::runtime::network::{clip, ms, net, quiet_note, sender_kind, start_end_note};
use crate::compose::runtime::runs::{Runner, Turn};
use crate::compose::runtime::sandbox;
use crate::compose::runtime::Runtime;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_api::From;
use xenonet_authz::Information;
use xenonet_engine::delivery::starts::{self, Request, TARGET};
use xenonet_model::ids::YOU;
use xenonet_model::ops::{Kind as FrameKind, To};
use xenonet_os::sync::Locked;
use xenonet_store::delivery::Delegated;
use xenonet_text::time::now_ms;

/// How long Claude Code waits on an MCP tool: a minute past the permission
/// tool's hold, so a prompt waiting for you isn't cut short.
pub(crate) fn mcp_tool_timeout_ms(w: &Runtime) -> u64 {
    (w.setup.permission_hold_ms.unwrap_or(crate::compose::runtime::asks::HOLD_MS) + 60e3).round() as u64
}

/// A request sent from here, waiting for its answers.
struct Asked {
    by: Value,
    host: String,
    /// The network it was sent into, and the only one its answers and its
    /// run's prompts are taken from.
    network: String,
    at: f64,
}

/// A run a machine's own word names, by its session and its agent: a
/// receipt for a start request of ours, or the report of an intent we made
/// or arbitrate.
#[derive(Clone, Default, PartialEq)]
struct Named {
    session: Option<String>,
    agent: Option<String>,
}

/// A run kept as named: whose it is, in which network, when it was last
/// named or last asked, and when it was first named.
struct Kept {
    host: String,
    network: String,
    run: Named,
    at: f64,
    since: f64,
}

/// A run started here for another machine, with the network its start came
/// through: when it last asked, and when it was started.
struct RunNet {
    network: String,
    at: f64,
    since: f64,
}

/// How long a run stays named after it was last named or last asked from:
/// a day. A run that ended can be resumed by a later message, and its turn
/// asks under the same session; a day covers an evening's break and the
/// next morning, and a machine that was once asked for a run keeps no
/// longer a claim on this console than that. Every ask taken renews it, up
/// to `NAMED_MAX_MS`.
const NAMED_FOR_MS: f64 = 24.0 * 3600e3;

/// The longest a run stays named or networked, renewals or not: seven days
/// from the start. A week covers a run resumed over a few days of work; past
/// it a machine asked for one run must be asked again, so no run it was
/// once asked for lets it prompt this console for good.
const NAMED_MAX_MS: f64 = 7.0 * 24.0 * 3600e3;

/// The most runs kept as named, and the most with a network; past it the
/// ones longest unused go first.
const KEPT_MAX: usize = 200;
const RUN_NETWORKS_MAX: usize = 500;

/// The settings the two are kept in, which leave the store's schema (and
/// every store on disk) as it is.
const KEPT_SETTING: &str = "named-runs";
const RUN_NETWORKS_SETTING: &str = "run-networks";

fn live(at: f64, since: f64, now: f64) -> bool {
    now - at < NAMED_FOR_MS && now - since < NAMED_MAX_MS
}

#[derive(Default)]
pub struct StartSide {
    /// Where the runs named and the networks of the runs are kept, so a
    /// restart forgets neither.
    db: Option<Arc<xenonet_store::Db>>,
    /// Starts here in the last hour: when, and from which machine ("" for
    /// this one).
    history: Mutex<Vec<(f64, String)>>,
    /// Requests sent from here to other machines, by their mid.
    asked: Mutex<HashMap<String, Asked>>,
    /// Requests from other machines already taken, by their mid.
    taken: Mutex<HashMap<String, f64>>,
    /// Copies of sessions XenoNet forked to answer a message, each with the
    /// session it came from: the last 200.
    forks: Mutex<Vec<(String, String)>>,
    /// The runs other machines named, which their prompts are taken for.
    kept: Mutex<Vec<Kept>>,
    /// The network each run another machine asked for was started through,
    /// by its session: the only network its prompts are sent into.
    run_networks: Mutex<HashMap<String, RunNet>>,
    /// Writes to the store held while a transaction is open around what
    /// names a run (`hold_writes`).
    held: Mutex<Held>,
    /// The environment the runs XenoNet starts are made from: the
    /// service's own, unless a test gave another.
    environment: Mutex<Option<Vec<(String, String)>>>,
    /// The sign-in the runs are given, unless the credential store has one:
    /// only a test gives it (`set_sign_in`).
    sign_in: Mutex<Option<String>>,
}

#[derive(Default)]
struct Held {
    depth: usize,
    kept: bool,
    runs: bool,
}

/// What `StartSide::hold_writes` returns: the writes held are made when the
/// last of these is dropped.
pub struct WritesHeld<'a>(&'a StartSide);

impl Drop for WritesHeld<'_> {
    fn drop(&mut self) {
        let (kept, runs) = {
            let mut h = self.0.held.locked();
            h.depth -= 1;
            if h.depth > 0 {
                return;
            }
            (std::mem::take(&mut h.kept), std::mem::take(&mut h.runs))
        };
        if kept {
            self.0.save_kept(&self.0.kept.locked());
        }
        if runs {
            self.0.save_run_networks(&self.0.run_networks.locked());
        }
    }
}

impl StartSide {
    /// The environment a run is made from (the service's), before what
    /// each kind of run takes out and puts in.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.environment.locked().clone().unwrap_or_else(|| std::env::vars().collect())
    }

    /// Gives the runs another environment than the process's, so a test
    /// needn't change the process's (which other threads read).
    #[doc(hidden)]
    pub fn set_environment(&self, vars: Vec<(String, String)>) {
        *self.environment.locked() = Some(vars);
    }

    /// Gives the runs this token as their sign-in, in place of the
    /// credential store's, which a test mustn't touch.
    #[doc(hidden)]
    pub fn set_sign_in(&self, token: &str) {
        *self.sign_in.locked() = Some(token.to_string());
    }

    fn sign_in_override(&self) -> Option<String> {
        self.sign_in.locked().clone()
    }

    /// Holds what names a run to the store until the guard is dropped, so a
    /// transaction around it (an entry being taken) that rolls back doesn't
    /// take back what this side kept: it is written, as it stands here, once
    /// the transaction is over.
    pub fn hold_writes(&self) -> WritesHeld<'_> {
        self.held.locked().depth += 1;
        WritesHeld(self)
    }

    /// The side with what the store kept: the runs named and the networks
    /// of the runs, those still within their time.
    pub fn open(db: Option<Arc<xenonet_store::Db>>) -> Self {
        let side = StartSide { db, ..StartSide::default() };
        let now = now_ms();
        if let Some(db) = &side.db {
            let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
            let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            if let Some(rows) = db.setting(KEPT_SETTING).ok().as_ref().and_then(Value::as_array) {
                let mut kept = side.kept.locked();
                for r in rows {
                    let (Some(host), Some(network), Some(at), Some(since)) = (text(r, "host"), text(r, "network"), num(r, "at"), num(r, "since")) else { continue };
                    if live(at, since, now) {
                        kept.push(Kept { host, network, run: Named { session: text(r, "session"), agent: text(r, "agent") }, at, since });
                    }
                }
            }
            if let Some(rows) = db.setting(RUN_NETWORKS_SETTING).ok().as_ref().and_then(Value::as_object) {
                let mut runs = side.run_networks.locked();
                for (session, r) in rows {
                    let (Some(network), Some(at), Some(since)) = (text(r, "network"), num(r, "at"), num(r, "since")) else { continue };
                    if live(at, since, now) {
                        runs.insert(session.clone(), RunNet { network, at, since });
                    }
                }
            }
        }
        side
    }

    fn save_kept(&self, kept: &[Kept]) {
        {
            let mut h = self.held.locked();
            if h.depth > 0 {
                h.kept = true;
                return;
            }
        }
        let rows: Vec<Value> =
            kept.iter().map(|k| json!({ "host": k.host, "network": k.network, "session": k.run.session, "agent": k.run.agent, "at": k.at, "since": k.since })).collect();
        if let Some(db) = &self.db {
            let _ = db.set_setting(KEPT_SETTING, Some(&Value::Array(rows)));
        }
    }

    fn save_run_networks(&self, runs: &HashMap<String, RunNet>) {
        {
            let mut h = self.held.locked();
            if h.depth > 0 {
                h.runs = true;
                return;
            }
        }
        let rows: serde_json::Map<String, Value> = runs.iter().map(|(s, r)| (s.clone(), json!({ "network": r.network, "at": r.at, "since": r.since }))).collect();
        if let Some(db) = &self.db {
            let _ = db.set_setting(RUN_NETWORKS_SETTING, Some(&Value::Object(rows)));
        }
    }

    /// Keeps the network the run `session` was started through, when a
    /// frame asked for it. A run started here for no one has none.
    pub fn note_network(&self, session: &str, network: Option<String>) {
        let Some(network) = network else { return };
        let now = now_ms();
        let mut runs = self.run_networks.locked();
        runs.retain(|_, r| live(r.at, r.since, now));
        // Past the most kept, the one longest unused goes, never all.
        while runs.len() >= RUN_NETWORKS_MAX {
            let Some(oldest) = runs.iter().min_by(|a, b| a.1.at.total_cmp(&b.1.at)).map(|(k, _)| k.clone()) else { break };
            runs.remove(&oldest);
        }
        runs.insert(session.to_string(), RunNet { network, at: now, since: now });
        self.save_run_networks(&runs);
    }

    /// The network the run `session` (or the session it was forked from) was
    /// started through; None for a run started here for no one, or one past
    /// its time. Asking renews it, to the longest a run is kept.
    pub fn network_of(&self, session: &str, parent: Option<&str>) -> Option<String> {
        let now = now_ms();
        let mut runs = self.run_networks.locked();
        let key = [Some(session), parent].into_iter().flatten().find(|k| runs.get(*k).is_some())?.to_string();
        let r = runs.get_mut(&key)?;
        if !live(r.at, r.since, now) {
            runs.remove(&key);
            self.save_run_networks(&runs);
            return None;
        }
        r.at = now;
        let network = r.network.clone();
        self.save_run_networks(&runs);
        Some(network)
    }

    /// Whether `host`, in `network`, has a run named as its own, by
    /// `session` or by `agent`: one it was asked from here to start (its
    /// own receipts name it) or one it works for an intent made or
    /// arbitrated here. Nothing else it says about a run is believed. A
    /// run named stays named for a day (`NAMED_FOR_MS`), renewed each time
    /// an ask is taken for it, and for a week at most (`NAMED_MAX_MS`).
    pub fn names_run(&self, host: &str, network: &str, session: &str, agent: Option<&str>) -> bool {
        let now = now_ms();
        let mut kept = self.kept.locked();
        kept.retain(|k| live(k.at, k.since, now));
        let found = kept.iter_mut().find(|k| {
            k.host == host && k.network == network && ((!session.is_empty() && k.run.session.as_deref() == Some(session)) || (agent.is_some() && k.run.agent.as_deref() == agent))
        });
        let named = found.map(|k| k.at = now).is_some();
        if named {
            self.save_kept(&kept);
        }
        named
    }

    /// A run `host` started for a request of ours, in `network`, as its
    /// receipt named it: what `answer_in` keeps, and what a test of a
    /// machine that was never asked leaves out.
    pub fn note_run(&self, mid: &str, host: &str, network: &str, session: Option<&str>, agent: Option<&str>) {
        if !self.asked.locked().get(mid).is_some_and(|a| a.host == host && a.network == network) {
            return;
        }
        self.keep(host, network, session, agent);
    }

    /// The run `host` works for an intent this machine made or arbitrates,
    /// in the intent's `network`, as the intent's start report names it
    /// (by its agent).
    pub fn note_intent_run(&self, host: &str, network: &str, agent: Option<&str>) {
        self.keep(host, network, None, agent);
    }

    fn keep(&self, host: &str, network: &str, session: Option<&str>, agent: Option<&str>) {
        let run = Named { session: session.map(str::to_string), agent: agent.map(str::to_string) };
        if run == Named::default() {
            return;
        }
        let now = now_ms();
        let mut kept = self.kept.locked();
        kept.retain(|k| live(k.at, k.since, now));
        if let Some(k) = kept.iter_mut().find(|k| k.host == host && k.network == network && k.run == run) {
            k.at = now;
        } else {
            // Past the most kept, the one longest unused goes.
            while kept.len() >= KEPT_MAX {
                let Some(i) = kept.iter().enumerate().min_by(|a, b| a.1.at.total_cmp(&b.1.at)).map(|(i, _)| i) else { break };
                kept.remove(i);
            }
            kept.push(Kept { host: host.to_string(), network: network.to_string(), run, at: now, since: now });
        }
        self.save_kept(&kept);
    }
}

/// One turn: of XENONET_RUNS_BIN's program when it names one (the tests'
/// fake), else of the app's CLI as the runner finds it.
pub fn run_turn(w: &Runtime, runner: &Arc<Runner>, t: Turn<'_>) -> Result<u32, String> {
    match &w.setup.runs_bin {
        Some(bin) => runner.run_program(t, bin),
        None => runner.run(t, &w.setup.home),
    }
}

/// This machine as answers name it: its host id in a network, else "this
/// machine".
pub fn here_name(w: &Runtime) -> String {
    net(w).me().map_or_else(|| "this machine".into(), |m| m.host)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// Who asked for a start, as this machine took them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    You,
    Session,
    Actor,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::You => "you",
            Kind::Session => "session",
            Kind::Actor => "actor",
        }
    }
}

fn kind_of(k: &str) -> Kind {
    match k {
        "you" | "person" => Kind::You,
        "actor" => Kind::Actor,
        _ => Kind::Session,
    }
}

/// Delegated work asked of a machine not set to take it: what the asker
/// is told, saying where the person sets it.
fn delegated_words(w: &Runtime, r: &Request, kind: Kind) -> Option<String> {
    if !r.delegate || kind == Kind::You || net(w).reach().start == "delegate" {
        return None;
    }
    let me = net(w).me().map_or_else(|| "this machine".to_string(), |m| m.host);
    Some(format!("{me} isn't set to take delegated work: your user sets that once, in Machines under Start sessions, as Delegated (xenonet net set --on {me} --start delegate)"))
}

/// A start request without its prompt, and the prompt apart, as
/// information: what's decided is the request's app, folder and mode, and
/// the prompt goes to the run only once a start was allowed.
pub struct Apart {
    pub request: Request,
    pub prompt: Information<String>,
    /// For a run started for an intent: the intent, which frames its
    /// prompt and names the delegation that alone answers its prompts.
    pub intent: Option<crate::compose::runtime::intents::Intended>,
}

fn apart(mut r: Request) -> Apart {
    let prompt = Information::new(std::mem::take(&mut r.prompt));
    Apart { request: r, prompt, intent: None }
}

/// The run a start makes, and the act, as a request names them: on
/// `machine`, from the request's own app, folder and mode. `signer` is the
/// identity that signs it (the person's id), whose wall a `cell` start runs
/// behind: the request never names one, and the act is built with the
/// wall the signer's id gives, so the signature covers that wall and no
/// other.
fn run_act(machine: &xenonet_authz::MachineId, r: &Request, signer: Option<&str>) -> Result<(xenonet_authz::Resource, xenonet_authz::Act), String> {
    let app = xenonet_authz::AppId::new(&r.app).map_err(|e| e.message)?;
    let folder = xenonet_authz::Folder::new(&r.folder).map_err(|e| e.message)?;
    let mode = if r.delegate { xenonet_authz::StartMode::Delegated } else { xenonet_authz::StartMode::Asks };
    // What the request says: a request that says nothing asks for no
    // level, and xenonet-authz refuses that from anyone but the person.
    let confine = match &r.confine {
        starts::ConfineAsk::Unset => None,
        starts::ConfineAsk::Off => Some(xenonet_authz::Confine::Off),
        starts::ConfineAsk::Vendor(d) => Some(xenonet_authz::Confine::Vendor { domains: d.clone() }),
        starts::ConfineAsk::Cell => {
            let signer = signer.ok_or(CELL_NOT_YOURS)?;
            Some(xenonet_authz::Confine::Cell { wall: xenonet_engine::confine::wall_for_identity(signer).map_err(|e| e.to_string())?.to_string() })
        }
    };
    Ok((xenonet_authz::Resource::Run { machine: machine.clone(), app: app.clone(), folder: folder.clone() }, xenonet_authz::Act::Start { app, folder, mode, confine }))
}

/// Why a start in a cell is for the person alone.
const CELL_NOT_YOURS: &str =
    "a start in a cell is yours alone to make, from your own console: cells of different walls don't have keys of their own yet, so no session, actor or other machine starts one";

/// What a `cell` start says when it is asked for: cells are not wired to
/// session starts yet.
const CELL_NOT_WIRED: &str =
    "cell starts aren't wired yet: a cell has no way to carry the app's sign-in or bring its session back to the map, so nothing was started. Use vendor, the app's own sandbox";

/// A plain UUID, for a new session's id.
pub(crate) fn uuid() -> Result<String, String> {
    let id = xenonet_engine::network::new_id("s").map_err(|t| t.message)?;
    Ok(id.split_once('_').map_or(id.clone(), |(_, u)| u.to_string()))
}

/// The folders this machine knows: its projects', folders' and Codex
/// projects' paths by their ids on the map, and the folders its sessions
/// are in.
fn folders(w: &Runtime) -> (Vec<(String, String)>, Vec<String>) {
    let mut known = Vec::new();
    let graph = w.state().graph().ok().flatten().unwrap_or(Value::Null);
    let nodes: Vec<Value> = match graph.get("nodes") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.values().cloned().collect(),
        _ => Vec::new(),
    };
    for n in &nodes {
        let (Some(id), Some(path)) = (s(n, "id"), s(n, "path")) else { continue };
        if matches!(n.get("kind").and_then(Value::as_str), Some("project" | "zone" | "native")) {
            known.push((id, path));
        }
    }
    let reported = w.state().nodes().unwrap_or_default().iter().filter_map(|n| s(n, "cwd")).collect();
    (known, reported)
}

/// What a start tells whoever asked: started (at once, and again for Codex
/// once its thread has an id), and ended.
pub type Answer = Arc<dyn Fn(&Runtime, &str, Value) + Send + Sync>;

/// A start on this machine, asked for by `by` ({ kind, id, title }) from
/// `from` (another machine, or None for this one), `kind` and `id` as this
/// machine took them, once xenonet-authz allowed it, with its first
/// `prompt`. Its first answer, or why it didn't start; `answer` hears the
/// rest.
pub fn run_here(w: &Runtime, by: &Value, from: Option<&str>, kind: Kind, id: &str, start: &Apart, answer: Answer) -> Result<Value, String> {
    let (r, prompt) = (&start.request, &start.prompt);
    let reach = net(w).reach();
    if w.setup.demo {
        return Err("XenoNet's demo starts no sessions".into());
    }
    let Some(resumes) = w.router.resume.as_ref() else { return Err("this XenoNet runs no sessions".into()) };
    // Paused, XenoNet starts nothing, whoever asks.
    if w.policy.with(|p, _| p.paused()) {
        return Err("XenoNet is paused here, so it starts nothing until it's resumed".into());
    }
    let now = now_ms();
    // Ten an hour from each machine, and from each asker here.
    let bucket = from.map_or_else(|| starts::asker(id), str::to_string);
    {
        let mut h = w.starts.history.locked();
        *h = starts::kept(&h, now);
        if let Some(why) = starts::admit(&h, &bucket, now) {
            return Err(why);
        }
    }
    let (known, reported) = folders(w);
    let cwd = starts::folder(&r.folder, &w.setup.home, &known, &reported, cfg!(windows))?;
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(format!("{} isn't there", r.folder));
    }
    // This machine's limits on messages hold for a start too: too fast
    // from one sender, a repeat of the same prompt.
    use xenonet_engine::delivery::policy::Fail as Held;
    match w.policy.with(|p, h| p.check_send(h, id, &[json!(TARGET)], prompt.read(), 0.0)) {
        Ok(()) => {}
        Err(Held::Refused { code: "duplicate", .. }) => return Err("the same start was asked for under a minute ago".into()),
        Err(Held::Refused { why, .. } | Held::Error(why)) => return Err(format!("this machine's limits hold it: {why}")),
    }
    // Anyone but the person starts a run at vendor: xenonet-authz decided
    // that at the door, and this holds it again, for whatever calls here.
    if r.confine == starts::ConfineAsk::Cell {
        if kind != Kind::You {
            return Err(CELL_NOT_YOURS.into());
        }
        return Err(CELL_NOT_WIRED.into());
    }
    // An intent's run carries the person's id (the intent is theirs) but is
    // unattended, and its last words go out as the outcome: for what is
    // allowed it is not the person's own run.
    let own = kind == Kind::You && start.intent.is_none();
    // Who a run that isn't the person's own is for: the store marks its
    // session as confined for them, so it is never resumed bare.
    let for_others = (!own).then(|| match (&start.intent, from) {
        (Some(_), _) => "an intent".to_string(),
        (None, Some(machine)) => format!("a {} on {machine}", kind.as_str()),
        (None, None) => format!("a {} here", kind.as_str()),
    });
    let record = match (r.confine.domains(), kind) {
        (Some(domains), _) => Some(sandbox::Record { folder: cwd.clone(), domains: domains.to_vec(), for_others }),
        (None, Kind::You) => None,
        (None, _) => return Err("a run started by anyone but the person runs in its app's sandbox: this start asked for none".into()),
    };
    // An app whose sandbox reads the whole disk is for the person's own runs.
    if record.is_some() {
        starts::sandbox_enough(&r.app, own)?;
    }
    // Anyone but the person reaches only the domains the person allowed.
    if let (Some(rec), false) = (&record, own) {
        starts::domains_within(&rec.domains, &sandbox_domains(w))?;
    }
    // A sandbox is a project folder: checked here, on the folder as it
    // resolved on the machine that runs the start, whoever asked.
    if record.is_some() {
        starts::sandbox_folder(&cwd, &w.setup.home, cfg!(any(windows, target_os = "macos")))?;
        // The folder's hooks are looked for when the run's sandbox is made
        // (`sandbox::turn`), as for every later turn.
    }
    // The app has a sandbox to drive here, or the start says why not.
    let sandbox_words = match &record {
        Some(rec) => {
            let (port, bin) = (w.setup.port.to_string(), crate::compose::connect::bin(&w.setup));
            let plan = xenonet_adapter::delivery::Plan { folder: &rec.folder, home: &w.setup.home, domains: &rec.domains, port: &port, bin: &bin };
            Some(starts::sandbox_plan(r, &plan, cfg!(windows))?)
        }
        None => None,
    };
    // In acceptEdits, auto and dontAsk Claude Code approves edits and
    // commands without calling the permission tool, so what judges a run
    // that isn't the person's never sees them: only the person picks a mode.
    if let (Some(mode), false) = (&r.mode, own) {
        return Err(format!(
            "only the person picks a permission mode, and this start isn't theirs: it asked for {mode}, which would let the run approve its own edits without XenoNet judging them"
        ));
    }
    if let Some(started) = over_acp(w, r, by, from, kind, prompt, &cwd, &answer, start.intent.as_ref(), record.is_some()) {
        if started.is_ok() {
            w.starts.history.locked().push((now, bucket));
        }
        return started;
    }
    let asker = id.to_string();
    let id = uuid()?;
    let ways = xenonet_engine::delivery::apps::of(&r.app).unwrap_or_default();
    let session = (!ways.picks_id).then(|| format!("{}:{id}", r.app));
    let key = session.clone().unwrap_or_else(|| format!("start:{id}"));
    let runs = std::path::Path::new(&w.setup.state_dir).join("runs");
    // Under the person's delegation, a session or actor asking is told it was
    // your user's delegation, set where and when, as this machine keeps it.
    let delegation = (reach.start == "delegate" && kind != Kind::You).then(|| reach.delegated.clone()).flatten();
    let on = delegation.as_ref().map(|(_, at)| xenonet_text::time::iso(*at as f64).chars().take(10).collect::<String>());
    let text = match &start.intent {
        Some(i) => {
            let framed = crate::compose::runtime::intents::framed(i, prompt.read());
            // After the line that says who asked, as any run's is.
            match (&sandbox_words, framed.split_once("\n\n")) {
                (Some(words), Some((head, rest))) => format!("{head}\n\n[XenoNet] This run is sandboxed. {words}\n\n{rest}"),
                _ => framed,
            }
        }
        None => starts::framed(by, from, prompt.read(), delegation.as_ref().map(|(by, _)| by.as_str()).zip(on.as_deref()), sandbox_words.as_deref()),
    };
    // An app that reads its prompt from a file (Grok Build) gets one only
    // its owner can read, taken away when the run ends.
    let prompt_file = if starts::prompt_in_file(&r.app) {
        let f = runs.join(format!("{id}.prompt"));
        write_private(&f, &text).map_err(|e| format!("the prompt couldn't be written for Grok Build: {e}"))?;
        Some(f.to_string_lossy().into_owned())
    } else {
        None
    };
    let server = xenonet_engine::delivery::setup::mcp_server_name(f64::from(w.setup.port));
    let command = starts::command(r, &id, &cwd, resumes.settings.as_deref(), &server, prompt_file.as_deref());
    let sandboxed = command.and_then(|(app, args)| match &record {
        Some(rec) => sandbox::turn(w, &r.app, rec, &id, args).map(|(args, _)| (app, args)),
        None => Ok((app, args)),
    });
    let (app, args) = match sandboxed {
        Ok(c) => c,
        Err(why) => {
            if let Some(f) = &prompt_file {
                let _ = std::fs::remove_file(f);
            }
            return Err(why);
        }
    };
    // A sandboxed run gets the scrubbed environment an ACP agent gets, and
    // its sign-in; the service's own, without session marks, otherwise.
    let mut env: Vec<(String, String)> = if record.is_some() { sandbox::env(w) } else { xenonet_engine::delivery::resume::resume_env(w.starts.environment()) };
    if let Some(v) = ways.tool_timeout {
        env.retain(|(k, _)| k != v);
        env.push((v.into(), mcp_tool_timeout_ms(w).to_string()));
    }
    let sign_in = ways.sign_in.and_then(|s| signed_env(w, &s, &mut env));
    // Kept before the run starts, under its session (Codex names its own
    // later, so it is kept under the run's key until then): a run whose
    // sandbox can't be kept isn't started, since a turn for it later would
    // have nothing to hold it.
    if let Some(rec) = &record {
        if let Err(why) = sandbox::keep(w, session.as_deref().unwrap_or(&key), rec) {
            if let Some(f) = &prompt_file {
                let _ = std::fs::remove_file(f);
            }
            return Err(why);
        }
    }
    let turn = Turn { key: &key, app: &app, args: &args, cwd: &cwd, env: &env, text: if prompt_file.is_some() { String::new() } else { text }, sign_in: sign_in.as_deref() };
    let pid = match run_turn(w, &resumes.runner, turn) {
        Ok(pid) => pid,
        Err(why) => {
            if let Some(f) = &prompt_file {
                let _ = std::fs::remove_file(f);
            }
            return Err(why);
        }
    };
    w.starts.history.locked().push((now, bucket));
    if from.is_some() {
        w.starts.note_network(&key, xenonet_net::ground::current());
    }
    // The session's agent, from the start: its own id when the app takes
    // the one XenoNet gave it, else a pending row its process takes over
    // once the app names it.
    let db = w.store.0.db();
    let agent = match &session {
        Some(s) => db.agent_of(s, Some(i64::from(pid)), now as i64),
        None => db.pending_agent(&r.app, Some(i64::from(pid)), Some(&asker), Some(&cwd), now as i64),
    };
    let agent = match agent {
        Ok(a) => Some(a.as_str().to_string()),
        Err(e) => {
            w.log("warn", "start.agent", &format!("the started session's agent couldn't be kept: {}", e.message));
            None
        }
    };
    if let (Some(session), true) = (&session, delegation.is_some() || start.intent.is_some()) {
        // Kept in the store, so its prompts are judged against the
        // delegation after a restart too; a week on, its runs are over.
        // One started for an intent keeps it, and the delegation it names.
        let intent = start.intent.as_ref().map(|i| (i.id.clone(), i.delegation.clone()));
        let d = Delegated { cwd: cwd.clone(), from: from.map(str::to_string), at: now, intent };
        if let Err(e) = w.store.0.db().put_delegated(session, &d, xenonet_engine::delivery::runs::KEEP_MS) {
            w.log("warn", "start.delegated", &format!("{session}'s delegation couldn't be kept: {}", e.message));
        }
    }
    let folder = xenonet_os::paths::tilde(&cwd, &w.setup.home);
    w.log(
        "info",
        "start",
        &format!("started {} in {folder} for {}", session.as_deref().unwrap_or("a Codex session"), from.map_or_else(|| "this machine".to_string(), str::to_string)),
    );
    if let Some(world) = w.arc() {
        let runner = resumes.runner.clone();
        let (key, app, logs) = (key.clone(), app.clone(), runs);
        let (session, folder, prompt_file) = (session.clone(), folder.clone(), prompt_file);
        let record = record.clone();
        std::thread::spawn(move || watch(&world, &runner, &key, pid, &app, &logs, session, &folder, prompt_file, record, &answer));
    }
    let mut first = json!({ "session": session, "agent": agent, "pid": pid, "folder": folder, "app": app });
    if let (Some(rec), Some(words)) = (&record, &sandbox_words) {
        first["sandbox"] = json!({ "domains": rec.domains, "words": words });
    }
    Ok(first)
}

/// What a sandboxed start does about ACP, which has no sandbox XenoNet
/// drives: an app with a headless start is started that way (None), and one
/// without isn't started.
fn acp_for_sandboxed(name: &str, headless: bool) -> Option<Result<Value, String>> {
    (!headless).then(|| Err(format!("{name} would be started over ACP, which has no sandbox XenoNet drives, so a sandboxed start of it isn't made")))
}

/// A start over ACP, for an app the registry gives `acp`, with no mode
/// asked for (ACP has no word for XenoNet's modes): its first answer once
/// its session is open, or why it couldn't be. None leaves it to the
/// app's headless start: no ACP side here, an app not started over ACP,
/// a mode asked for, or an agent that didn't start or didn't speak ACP as
/// XenoNet does (revision 25 keeps the headless start behind it until the
/// app's ACP mode is proven).
#[expect(clippy::too_many_arguments, reason = "the start's parts, as run_here took them")]
fn over_acp(
    w: &Runtime,
    r: &Request,
    by: &Value,
    from: Option<&str>,
    kind: Kind,
    prompt: &Information<String>,
    cwd: &str,
    answer: &Answer,
    intent: Option<&crate::compose::runtime::intents::Intended>,
    sandboxed: bool,
) -> Option<Result<Value, String>> {
    let side = w.acp.get()?.clone();
    let platform = xenonet_adapter::platforms().get(&r.app)?;
    platform.app().acp.as_ref()?;
    let headless = xenonet_engine::delivery::apps::of(&r.app).is_some_and(|w| w.start.is_some());
    if sandboxed {
        return acp_for_sandboxed(&platform.app().name, headless);
    }
    if r.mode.is_some() {
        return None;
    }
    let Some(command) = crate::compose::runtime::runs::acp_command(platform.as_ref(), &w.setup.home) else {
        return (!headless).then(|| Err(format!("{} isn't installed here", platform.app().name)));
    };
    let world = w.arc()?;
    let reach = net(w).reach();
    let delegation = (reach.start == "delegate" && kind != Kind::You).then(|| reach.delegated.clone()).flatten();
    let on = delegation.as_ref().map(|(_, at)| xenonet_text::time::iso(*at as f64).chars().take(10).collect::<String>());
    let folder = xenonet_os::paths::tilde(cwd, &w.setup.home);
    let start = crate::compose::runtime::acp::Start {
        app: r.app.clone(),
        program: command.program,
        args: command.args,
        cwd: cwd.to_string(),
        folder: folder.clone(),
        by: by.clone(),
        from: from.map(str::to_string),
        delegation: delegation.as_ref().map(|(by, _)| by.clone()).zip(on),
        // An intent's run is told whose intent it is, and how to report.
        prompt: Information::new(intent.map_or_else(|| prompt.read().clone(), |i| crate::compose::runtime::intents::framed(i, prompt.read()))),
        answer: answer.clone(),
    };
    match side.start(&world, start) {
        Ok(started) => {
            if delegation.is_some() || intent.is_some() {
                let d = Delegated { cwd: cwd.to_string(), from: from.map(str::to_string), at: now_ms(), intent: intent.map(|i| (i.id.clone(), i.delegation.clone())) };
                if let Err(e) = w.store.0.db().put_delegated(&started.session, &d, xenonet_engine::delivery::runs::KEEP_MS) {
                    w.log("warn", "start.delegated", &format!("{}'s delegation couldn't be kept: {}", started.session, e.message));
                }
            }
            if from.is_some() {
                w.starts.note_network(&started.session, xenonet_net::ground::current());
            }
            w.log("info", "start", &format!("started {} over ACP in {folder} for {}", started.session, from.unwrap_or("this machine")));
            Some(Ok(json!({ "session": started.session, "pid": started.pid, "folder": folder, "app": r.app })))
        }
        Err(crate::compose::runtime::acp::Failed::SignIn(why)) => Some(Err(why)),
        Err(crate::compose::runtime::acp::Failed::Unproven(why)) if headless => {
            w.log("warn", "start.acp", &format!("{} didn't start over ACP ({why}): its headless start takes it", r.app));
            None
        }
        Err(crate::compose::runtime::acp::Failed::Unproven(why)) => Some(Err(why)),
    }
}

// ---------- signing in for unattended runs ----------

fn secrets(w: &Runtime) -> crate::compose::runtime::wakes::Secrets {
    crate::compose::runtime::wakes::Secrets::new(w.setup.demo)
}

/// The long-lived Claude Code token XenoNet's runs sign in with, from your
/// credential store: for its runs' environment alone, never logged or
/// shown.
pub fn run_token(w: &Runtime) -> Option<String> {
    if let Some(t) = w.starts.sign_in_override() {
        return Some(t);
    }
    secrets(w).get(starts::SIGN_IN_TARGET).ok().flatten().filter(|t| !t.is_empty())
}

/// XenoNet's sign-in for a run, when you gave it one. On unix, for an app
/// that can read it from a descriptor (Claude Code), the environment gets
/// the variable that says so (with the descriptor's number) and no token,
/// and the token comes back for the caller to hand to the run
/// (`Turn::sign_in`): a command of the same user can read another
/// process's environment on macOS, and never a descriptor. Elsewhere the
/// token is in the variable the app's adapter names (Claude Code's
/// CLAUDE_CODE_OAUTH_TOKEN) and nothing comes back.
pub fn signed_env(w: &Runtime, s: &xenonet_adapter::delivery::SignIn, env: &mut Vec<(String, String)>) -> Option<String> {
    let t = run_token(w)?;
    env.retain(|(k, _)| k != s.variable && Some(k.as_str()) != s.fd_variable);
    #[cfg(unix)]
    if let Some(fd) = s.fd_variable {
        env.push((fd.into(), crate::compose::runtime::runs::SIGN_IN_FD.to_string()));
        return Some(t);
    }
    env.push((s.variable.into(), t));
    None
}

/// The setting the domains a run that isn't the person's own may reach are
/// kept in, which leaves the store's schema as it is.
const SANDBOX_DOMAINS_SETTING: &str = "sandbox-domains";

/// The domains a sandboxed run that isn't the person's own may reach, as
/// the person set them: none until they do.
pub fn sandbox_domains(w: &Runtime) -> Vec<String> {
    w.store
        .0
        .db()
        .setting(SANDBOX_DOMAINS_SETTING)
        .ok()
        .as_ref()
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// `xenonet runs domains`: `status` (the list), `set` it to `domains`, or
/// `clear` it. `{ domains }` back.
pub fn sandbox_domains_for(w: &Runtime, action: &str, domains: &[String]) -> Result<Value, (u16, String)> {
    match action {
        "status" => {}
        "set" => {
            let list = starts::allowed_domains(domains).map_err(|why| (400, why))?;
            w.store.0.db().set_setting(SANDBOX_DOMAINS_SETTING, Some(&json!(list))).map_err(|e| (500, format!("the domains couldn't be kept: {}", e.message)))?;
            w.log("info", "runs.sandbox-domains", &format!("runs not started by you may now reach {}", if list.is_empty() { "no domain".to_string() } else { list.join(", ") }));
        }
        "clear" => {
            w.store.0.db().set_setting(SANDBOX_DOMAINS_SETTING, None).map_err(|e| (500, format!("the domains couldn't be cleared: {}", e.message)))?;
            w.log("info", "runs.sandbox-domains", "runs not started by you may now reach no domain");
        }
        other => return Err((400, format!("{other} isn't something xenonet runs domains does: status, set or clear"))),
    }
    Ok(json!({ "domains": sandbox_domains(w) }))
}

/// `xenonet runs sign-in` and its kin: `status` (whether a token is kept,
/// and where), `sign-in` with the token you pasted, `forget`. The answer
/// never carries the token.
pub fn sign_in(w: &Runtime, action: &str, pasted: &str) -> Result<Value, (u16, String)> {
    let s = secrets(w);
    let kind = s.kind();
    match action {
        "sign-in" => {
            let token = starts::sign_in_token(pasted).map_err(|why| (400, why))?;
            let comment = "Claude Code's token for the sessions XenoNet runs, from claude setup-token";
            s.set(starts::SIGN_IN_TARGET, &token, comment).map_err(|t| (500, format!("your credential store didn't keep it: {t}")))?;
            w.log("info", "runs.sign-in", &format!("a Claude Code token for XenoNet's runs is kept in {kind}"));
        }
        "forget" => {
            let _ = s.remove(starts::SIGN_IN_TARGET);
            w.log("info", "runs.sign-in", "XenoNet forgot its runs' Claude Code token");
        }
        "status" => {}
        other => return Err((400, format!("{other} isn't something xenonet runs does: status, sign-in or forget"))),
    }
    let kept = s.has(starts::SIGN_IN_TARGET).is_ok_and(|v| v);
    Ok(json!({ "signedIn": kept, "store": kind }))
}

/// The folder a session was started in under the person's delegation, if
/// it was, and the machine it was started from (None for this one): what
/// a delegation covers is judged against them.
pub fn delegated(w: &Runtime, session: &str) -> Option<(String, Option<String>)> {
    w.store.0.db().delegated(session).ok().flatten().map(|d| (d.cwd, d.from))
}

/// The forks XenoNet made, newest last: each copy and the session it was
/// forked from, for the console to link them.
pub fn forks(w: &Runtime) -> Vec<(String, String)> {
    w.starts.forks.locked().clone()
}

/// A copy XenoNet forked from `parent`, kept with the others (the last
/// 200).
#[doc(hidden)]
pub fn note_fork(w: &Runtime, child: &str, parent: &str) {
    let mut f = w.starts.forks.locked();
    f.push((child.to_string(), parent.to_string()));
    let from = f.len().saturating_sub(200);
    f.drain(..from);
}

/// Forgets which copies were forked from which, as a restart does.
#[doc(hidden)]
pub fn forget_forks(w: &Runtime) {
    w.starts.forks.locked().clear();
}

/// The run a copy was forked from, followed back through copies of copies
/// to the one that is no copy: None for a session that is not a copy.
pub fn fork_root(w: &Runtime, session: &str) -> Option<String> {
    let forks = w.starts.forks.locked();
    let mut root = None;
    let mut at = session.to_string();
    for _ in 0..8 {
        let Some((_, parent)) = forks.iter().rev().find(|(child, _)| *child == at) else { break };
        at = parent.clone();
        root = Some(parent.clone());
    }
    root
}

/// Grok Build's prompt for a turn, in a file only its owner can read, in
/// the runs folder: its path.
pub fn prompt_file(w: &Runtime, text: &str) -> Result<String, String> {
    let f = std::path::Path::new(&w.setup.state_dir).join("runs").join(format!("{}.prompt", uuid()?));
    write_private(&f, text).map_err(|e| format!("the prompt couldn't be written for Grok Build: {e}"))?;
    Ok(f.to_string_lossy().into_owned())
}

/// A run's log, read as it grows: each read gives the whole lines written
/// since the last one, so a watcher never reads the log from its start
/// again.
struct Tail {
    file: std::path::PathBuf,
    at: u64,
    part: Vec<u8>,
}

impl Tail {
    fn new(file: std::path::PathBuf) -> Tail {
        Tail { file, at: 0, part: Vec::new() }
    }

    /// The bytes written since the last read.
    fn more(&mut self) {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&self.file) else { return };
        if f.seek(SeekFrom::Start(self.at)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_ok() {
            self.at += buf.len() as u64;
            self.part.extend_from_slice(&buf);
        }
    }

    /// The whole lines written since the last read; a line still being
    /// written waits for the next.
    fn lines(&mut self) -> String {
        self.more();
        let Some(end) = self.part.iter().rposition(|b| *b == b'\n') else { return String::new() };
        let done: Vec<u8> = self.part.drain(..=end).collect();
        String::from_utf8_lossy(&done).into_owned()
    }

    /// Everything not read yet, a last unfinished line included: once the
    /// run has ended.
    fn rest(&mut self) -> String {
        self.more();
        String::from_utf8_lossy(&std::mem::take(&mut self.part)).into_owned()
    }
}

/// After a resumed or forked turn starts: a fork's copy, once its stream
/// names it, linked to the session it came from (the fork event); the
/// prompt file taken away when the turn ends; and a hint for you when
/// Claude Code couldn't sign in for it.
#[expect(clippy::too_many_arguments, reason = "the turn's parts, and the sandbox it ran in")]
pub fn after_resume(w: &Runtime, runner: &Arc<Runner>, app: &str, key: &str, pid: u32, fork_of: Option<&str>, prompt_file: Option<String>, sandboxed: Option<sandbox::Record>) {
    // Whether the app's runs sign in, so a turn it couldn't sign in for
    // gets you a hint.
    let signs_in = xenonet_engine::delivery::apps::of(app).is_some_and(|w| w.sign_in.is_some());
    if fork_of.is_none() && prompt_file.is_none() && !signs_in {
        return;
    }
    let Some(world) = w.arc() else { return };
    let (runner, key, parent, app) = (runner.clone(), key.to_string(), fork_of.map(str::to_string), app.to_string());
    let logs = std::path::Path::new(&w.setup.state_dir).join("runs");
    std::thread::spawn(move || {
        let mut named = parent.is_none();
        let mut tail: Option<Tail> = None;
        let mut out = false;
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let Some(rec) = runner.list().into_iter().rev().find(|r| r.key == key && r.pid == Some(f64::from(pid))) else { break };
            let log = tail.get_or_insert_with(|| Tail::new(logs.join(format!("{}.log", rec.id))));
            let new = if rec.end.is_some() { log.rest() } else { log.lines() };
            out = out || starts::signed_out(&app, &new);
            if !named {
                if let (Some(child), Some(parent)) = (starts::session_of(&app, &new), parent.as_deref()) {
                    named = true;
                    note_fork(&world, &child, parent);
                    // A copy of a sandboxed session is sandboxed in its own right.
                    if let Some(Err(why)) = sandboxed.as_ref().map(|rec| sandbox::fork_named(&world, &child, rec)) {
                        world.log("warn", "fork", &format!("{child}'s sandbox couldn't be kept, so its run is stopped: {why}"));
                        runner.stop(&key);
                    }
                    world.log("info", "fork", &format!("{child} answers a message for {parent}, which is open elsewhere"));
                    world.emit(From::Engine, "fork", json!({ "child": child, "parent": parent, "at": ms(now_ms()) }));
                }
            }
            if rec.end.is_some() {
                // A turn Claude Code couldn't sign in for: a hint for you.
                if out {
                    sign_in_hint(&world, &here_name(&world));
                }
                break;
            }
        }
        if let Some(f) = prompt_file {
            let _ = std::fs::remove_file(f);
        }
    });
}

/// A file only its owner can read.
fn write_private(file: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    std::io::Write::write_all(&mut o.open(file)?, text.as_bytes())
}

/// The receipt for the end of a turn of a session XenoNet holds open over
/// ACP, as a run's: what it came to (`finish`), how long it took, and
/// whether the session is still held open for a reply (`live`).
pub fn acp_turn_end(w: &Runtime, app: &str, session: &str, finish: &starts::Finish, minutes: f64, live: bool) -> Value {
    end_body(w, app, Some(&session.to_string()), Some(finish), "", false, false, minutes, live)
}

/// The receipt for one end of a started run: what the session did
/// (`outcome`), its last words, and the tools it was denied. `live` says
/// its process is still running, listening for a reply.
#[expect(clippy::too_many_arguments, reason = "the end's parts")]
fn end_body(w: &Runtime, app: &str, session: Option<&String>, finish: Option<&starts::Finish>, plain: &str, said: bool, signed_out: bool, minutes: f64, live: bool) -> Value {
    let mut body = json!({ "session": session, "minutes": minutes, "app": app, "live": live });
    let mut kind = starts::outcome(finish, said);
    if signed_out {
        kind = "failed";
        let here = here_name(w);
        body["reason"] = json!(starts::sign_in_words(&here));
        body["signIn"] = json!(true);
        sign_in_hint(w, &here);
    }
    body["outcome"] = json!(kind);
    let text = finish.map(|f| f.text.as_str()).filter(|t| !t.is_empty()).unwrap_or(plain);
    if !text.is_empty() {
        body["last"] = json!(starts::clipped(text));
    }
    if let Some(f) = finish.filter(|f| !f.denied.is_empty()) {
        body["denied"] = json!(f.denied);
    }
    body
}

/// Watches a started run until it ends: Codex's thread id once it says
/// one; each turn's end, told as it happens (a session that waits for a
/// reply after its turn is still running, but has said its piece), with
/// what it did and its last words; and how the run ended, and whether
/// Claude Code needs a sign-in.
#[expect(clippy::too_many_arguments, reason = "the run's parts")]
fn watch(
    w: &Runtime,
    runner: &Runner,
    key: &str,
    pid: u32,
    app: &str,
    logs: &std::path::Path,
    mut session: Option<String>,
    folder: &str,
    prompt_file: Option<String>,
    sandboxed: Option<sandbox::Record>,
    answer: &Answer,
) {
    let mine = |r: &xenonet_engine::delivery::runs::Run| r.key == key && r.pid == Some(f64::from(pid));
    let mut tail: Option<Tail> = None;
    let mut out = false;
    let mut said = false;
    let mut turns = 0;
    // Whether the last turn told was told while the process still ran.
    let mut open = false;
    // What the run printed that wasn't an event: an error before the
    // session could say anything.
    let mut plain = String::new();
    let ended = loop {
        std::thread::sleep(Duration::from_millis(400));
        let Some(rec) = runner.list().into_iter().rev().find(|r| mine(r)) else { break None };
        let log = tail.get_or_insert_with(|| Tail::new(logs.join(format!("{}.log", rec.id))));
        let new = if rec.end.is_some() { log.rest() } else { log.lines() };
        out = out || starts::signed_out(app, &new);
        said = said || starts::reported(app, &new);
        if let Some(line) = new.lines().map(str::trim).rfind(|l| !l.is_empty() && serde_json::from_str::<Value>(l).is_err()) {
            plain = line.chars().take(400).collect();
        }
        if session.is_none() {
            if let Some(found) = starts::session_of(app, &new) {
                session = Some(found.clone());
                // An app that names its session once it runs (Codex) is
                // kept as sandboxed from then on.
                if let Some(rec) = &sandboxed {
                    if let Err(why) = sandbox::keep(w, &found, rec) {
                        w.log("warn", "start.sandbox", &format!("{found}'s run is stopped: {why}"));
                        runner.stop(key);
                        answer(w, "ended", json!({ "session": found, "minutes": 0, "app": app, "live": false, "outcome": "failed", "reason": why }));
                        break None;
                    }
                }
                answer(w, "started", json!({ "session": found, "pid": pid, "folder": folder, "app": app }));
            }
        }
        if let Some(f) = starts::finish(app, &new) {
            turns += 1;
            open = rec.end.is_none();
            let minutes = (rec.ms.unwrap_or_else(|| now_ms() - rec.at) / 60e3).round();
            answer(w, "ended", end_body(w, app, session.as_ref(), Some(&f), "", said, out, minutes, open));
            said = false;
        }
        if rec.end.is_some() {
            break Some(rec);
        }
    };
    if let Some(f) = prompt_file {
        let _ = std::fs::remove_file(f);
    }
    let Some(rec) = ended else { return };
    let minutes = (rec.ms.unwrap_or(0.0) / 60e3).round();
    if turns > 0 {
        // Every turn was told as it ended. If the last was told while the
        // process listened on, this lets the answer go.
        if open {
            answer(w, "ended", json!({ "session": session, "minutes": minutes, "app": app, "live": false, "closed": true }));
        }
        return;
    }
    let mut body = end_body(w, app, session.as_ref(), None, &plain, said, out, minutes, false);
    body["exit"] = json!(rec.exit);
    if rec.timed_out {
        body["outcome"] = json!("failed");
        body["reason"] = json!("it ran half an hour, the most a turn runs");
    } else if rec.exit.is_some_and(|e| e != 0.0) && body["outcome"] == "silent" {
        body["outcome"] = json!("failed");
    }
    answer(w, "ended", body);
}

/// What the requester hears of an answer from `host`: a message to a
/// session or an actor, or a line in your console.
pub fn tell(w: &Runtime, by: &Value, host: &str, state: &str, a: &Value) {
    // The last word of a run whose turns were all told: nothing to add.
    if a.get("closed").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let words = starts::answer_words(state, host, a);
    // Claude Code there needs a sign-in for XenoNet's runs: a hint in your
    // console, whoever asked.
    if a.get("signIn").and_then(Value::as_bool) == Some(true) {
        sign_in_hint(w, host);
    }
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    if kind == "you" {
        let mut ev = a.clone();
        if !ev.is_object() {
            ev = json!({});
        }
        ev["state"] = json!(state);
        ev["host"] = json!(host);
        ev["words"] = json!(words);
        ev["at"] = ms(now_ms());
        w.emit(From::Engine, "start", ev);
        return;
    }
    // To the session that asked. A start's beginning waits for its next
    // turn. Its end is delivered at once, waking the session if it's idle,
    // unless the started session told it itself: the message did that.
    if let Some(id) = s(by, "id") {
        if state == "ended" && a.get("outcome").and_then(Value::as_str) != Some("reported") {
            start_end_note(w, &id, &id, &words);
        } else {
            quiet_note(w, &id, &id, &words);
        }
    }
}

/// A start asked for through the API (`POST /api/start`, the MCP tool, the
/// CLI): `by` is { kind, id, title }, `req` { machine?, folder?, app?,
/// mode?, prompt }; `at_window` says the request came with the window's
/// key, so a start as you may ask for your device's signature. The first
/// answer, or a status and why.
pub fn ask(w: &Runtime, by: Value, req: &Value, at_window: bool) -> Result<Value, (u16, String)> {
    let mut r = starts::request(req).map_err(|why| (400, why))?;
    let me = net(w).me();
    // This machine goes by a host id in each network: any of them is this machine.
    let mine = net(w).networks();
    let machine = s(req, "machine").filter(|m| !m.is_empty() && mine.iter().all(|me| me.host != *m));
    let kind = kind_of(by.get("kind").and_then(Value::as_str).unwrap_or(""));
    // Vendor unless the person's own console start on this machine says
    // otherwise (its level is off by default): a start for another machine,
    // under a delegation, or by a session or an actor says nothing of it
    // only when its asker never thought of it, and runs at vendor then.
    if r.confine == starts::ConfineAsk::Unset {
        r.confine = if kind != Kind::You || machine.is_some() || r.delegate { starts::ConfineAsk::Vendor(Vec::new()) } else { starts::ConfineAsk::Off };
    }
    // Only the person starts a run without the sandbox, or in a cell, and it
    // is refused here as well as at the door: a start sent to another
    // machine is decided there, and an older XenoNet would take "off" for no
    // word.
    if r.confine == starts::ConfineAsk::Off && kind != Kind::You {
        return Err((409, "a session or an actor can't start a run without its app's sandbox: only you can, from your own console".into()));
    }
    if r.confine == starts::ConfineAsk::Cell && kind != Kind::You {
        return Err((409, CELL_NOT_YOURS.into()));
    }
    // An actor's own token, as it called with it: decided here, and never
    // kept with the start.
    let mut by = by;
    let token_text = by.as_object_mut().and_then(|o| o.remove("token")).and_then(|t| t.as_str().map(str::to_string));
    let Some(g) = crate::compose::runtime::ground::ground(w, None) else { return Err((503, "this machine's governance isn't plugged in, so it starts nothing".into())) };
    // The identity that signs a start of yours, whose wall a cell runs behind.
    let signer = (kind == Kind::You).then(|| g.facts.identity.person_id()).flatten();
    let token = match (kind, &token_text) {
        (Kind::Actor, Some(t)) => Some(xenonet_authz::Token::parse(t, xenonet_authz::ACTOR_PREFIX, &g, g.facts.now).map_err(|e| (403, e.message))?),
        (Kind::Actor, None) => return Err((403, "an actor starts a session with its token".into())),
        _ => None,
    };
    let Some(host) = machine else {
        // Decided at this machine's own door, on the request without its
        // prompt.
        let start = apart(r);
        let r = &start.request;
        let id = s(&by, "id").unwrap_or_default();
        let (resource, act) = run_act(&g.facts.machine, r, signer.as_deref()).map_err(|why| (400, why))?;
        let principal = match (&token, kind) {
            (_, Kind::You) => {
                let auth =
                    crate::compose::runtime::ground::authority(&g.facts.identity, at_window, &resource, &act, &format!("start {} in {} as you", r.app, r.folder), g.facts.now)
                        .map_err(|why| (403, why))?;
                crate::compose::runtime::ground::person(w, &g, None, &crate::compose::runtime::ground::Carried::parse(&auth), &resource, &act)
            }
            (Some(t), _) => Ok(xenonet_authz::Principal::Actor { slug: t.bearer().slug.clone(), machine: t.bearer().machine.clone() }),
            _ => crate::compose::runtime::ground::agent(&id, &g.facts.machine),
        }
        .map_err(|why| (403, why))?;
        crate::compose::runtime::ground::judge_by(&g, &crate::compose::runtime::ground::local(&g, principal, resource, act), token.as_ref(), None)
            .map_err(|why| (409, delegated_words(w, r, kind).unwrap_or(why)))?;
        let who = by.clone();
        let answer: Answer = Arc::new(move |w: &Runtime, state: &str, a: Value| tell(w, &who, "this machine", state, &a));
        let first = run_here(w, &by, None, kind, &id, &start, answer).map_err(|why| (409, why))?;
        let mut out = first;
        out["state"] = json!("started");
        out["host"] = me.map_or(Value::Null, |m| json!(m.host));
        return Ok(out);
    };
    let Some(_) = me else { return Err((409, "this machine isn't in a XenoNet network: start sessions on another machine once it's in one".into())) };
    let Some(h) = net(w).hosts().into_iter().find(|h| h.id == host) else { return Err((404, format!("{host} isn't a machine in your network"))) };
    // A start is never kept for later: it goes only to a machine that's
    // there, and new enough to take it.
    if !h.online {
        return Err((409, format!("{host} is offline, so nothing was started: a start is never kept for later")));
    }
    if !net(w).host_can(&host, "start") {
        return Err((409, format!("{host}'s XenoNet is too old to start sessions: update it there")));
    }
    // An older XenoNet would take a sandboxed request as an unsandboxed one.
    if r.confine != starts::ConfineAsk::Off && !net(w).host_can(&host, "sandbox") {
        return Err((
            409,
            format!("{host}'s XenoNet is too old to start sandboxed sessions, and nothing was started: update it there, or start without the sandbox from your own console"),
        ));
    }
    // The network it goes through: the one that has the machine, which
    // the request names when two of this machine's networks do.
    // The request carries the prompt and the asking session's title, so it
    // goes only into a network shown the project that session is in; an
    // actor or a session in no project, only into one shown everything. The
    // person, signed, may start anywhere.
    let shown = (kind != Kind::You).then(|| {
        let project = s(&by, "id").and_then(|id| crate::compose::runtime::network::project_of_session(w, &id));
        net(w).networks_for(project.as_ref().map(xenonet_authz::ProjectId::as_str))
    });
    let network = network_for(w, &host, s(req, "network").as_deref(), shown.as_deref()).map_err(|why| (409, why))?;
    let mid = xenonet_engine::network::new_id("msg").map_err(|t| (500, t.message))?;
    let id = s(&by, "id").unwrap_or_else(|| YOU.into());
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    let sender = if kind == "you" { YOU.to_string() } else { id };
    let mut request = r.to_json();
    request["request"] = json!(mid);
    request["by"] = json!({ "kind": kind, "id": sender, "title": by.get("title").and_then(Value::as_str).map(|t| clip(t, 200)) });
    let text = request.to_string();
    if text.chars().count() > 16000 {
        return Err((400, "that prompt is too long to send to another machine: keep it under 15,000 characters".into()));
    }
    // As you: your device's signature over the run it starts there. An
    // actor: a token minted for the start, once its own has the right.
    let authority = if kind == "you" {
        let there = crate::compose::runtime::ground::machine(&host).map_err(|why| (400, why))?;
        let (resource, act) = run_act(&there, &r, signer.as_deref()).map_err(|why| (400, why))?;
        crate::compose::runtime::ground::authority(&g.facts.identity, at_window, &resource, &act, &format!("start {} on {host} as you", r.app), g.facts.now)
            .map_err(|why| (403, why))?
    } else if let Some(t) = &token {
        let slug = &t.bearer().slug;
        if !t.rights().contains(&xenonet_authz::Action::Start) {
            return Err((403, "starting a session needs the spawn-sessions capability, which this actor wasn't allowed".into()));
        }
        crate::compose::runtime::ground::actor_authority(w, slug.as_str(), vec![xenonet_authz::Action::Start]).map_err(|why| (503, why))?
    } else {
        json!({})
    };
    let body = json!({
        "schema": "network/send/v1",
        "mid": mid,
        "sender": sender,
        "senderKind": sender_kind(&sender),
        // Who asks, by its agt_, which the other machine names it by.
        "agent": (sender != YOU).then(|| crate::compose::runtime::network::wire_id(w, &sender)),
        "target": TARGET,
        "body": text,
        "replyTo": null,
        "scope": "direct",
        "kind": "message",
        "hops": 0,
        "at": ms(now_ms()),
    });
    w.starts.asked.locked().insert(mid.clone(), Asked { by: by.clone(), host: host.clone(), network: network.clone(), at: now_ms() });
    crate::compose::runtime::network::send_in(w, Some(network), To::Host(host.clone()), FrameKind::Message, body, Some(authority));
    Ok(
        json!({ "request": mid, "host": host, "state": "sent", "way": format!("sent to {host}: the answer comes as a message, started or refused, and again when the session ends") }),
    )
}

/// The network a request to `host` goes through: the only one that has it,
/// else the one `named` says (by id or name). A machine that is in several
/// of this machine's networks is not guessed at. Only the networks in
/// `shown` count, when it is given: those shown what the start is from.
fn network_for(w: &Runtime, host: &str, named: Option<&str>, shown: Option<&[String]>) -> Result<String, String> {
    let net = net(w);
    let mut has = net.networks_with(host);
    if !has.is_empty() && shown.is_some_and(|shown| !has.iter().any(|n| shown.contains(n))) {
        return Err(format!(
            "no network this machine shares with {host} is shown the project of the session asking, so nothing was started: share that project into one, or start it from a session in a project one is shown"
        ));
    }
    if let Some(shown) = shown {
        has.retain(|n| shown.contains(n));
    }
    let in_names = || net.networks().into_iter().filter(|m| has.contains(&m.network)).map(|m| format!("{} ({})", m.name, m.network)).collect::<Vec<_>>().join(", ");
    match (has.as_slice(), named.map(str::trim).filter(|n| !n.is_empty())) {
        ([], _) => Err(format!("{host} isn't a machine in your network")),
        ([one], None) => Ok(one.clone()),
        (_, None) => Err(format!("{host} is in several of this machine's networks ({}): say which with network", in_names())),
        (_, Some(n)) => {
            let found: Vec<String> = net.networks().into_iter().filter(|m| has.contains(&m.network) && (m.network == n || m.name == n)).map(|m| m.network).collect();
            match found.as_slice() {
                [one] => Ok(one.clone()),
                [] => Err(format!("{host} isn't in a network of yours called {n}: {}", in_names())),
                _ => Err(format!("two of this machine's networks are called {n}: say which by its id")),
            }
        }
    }
}

/// A start's receipt, into `network`: the one its request came in, which
/// a run's watcher on another thread no longer has in hand.
fn start_receipt(w: &Runtime, network: Option<&str>, to: &str, mid: &str, state: &str, extra: &Value) {
    let mut body = json!({ "schema": "network/receipt/v1", "mid": mid, "agent": TARGET, "state": state, "reason": null, "at": ms(now_ms()) });
    for (k, v) in extra.as_object().into_iter().flatten() {
        body[k.as_str()] = if k == "reason" { v.as_str().map_or(Value::Null, |r| json!(clip(r, 280))) } else { v.clone() };
    }
    crate::compose::runtime::network::send_in(w, network.map(str::to_string), To::Host(to.to_string()), FrameKind::Receipt, body, None);
}

/// What a start in `folder` reaches here: the project the folder is in,
/// through a checkout the person confirmed, or none (a folder in no
/// project, or one that isn't there, which can't be told apart from here).
fn folder_reach(w: &Runtime, folder: &str) -> xenonet_authz::Reaching {
    let (known, reported) = folders(w);
    let project = starts::folder(folder, &w.setup.home, &known, &reported, cfg!(windows))
        .ok()
        .filter(|cwd| std::path::Path::new(cwd).is_dir())
        .and_then(|cwd| w.setup.db.as_ref().and_then(|db| crate::compose::runtime::checkouts::project_of(db, &cwd)))
        .and_then(|p| xenonet_authz::ProjectId::new(&p).ok());
    project.map_or(xenonet_authz::Reaching::Unplaced, xenonet_authz::Reaching::Project)
}

/// A start request from another machine (`message_in` routes the target
/// here): `kind` and `sender` as its frame says, the `authority` it came
/// with, `at` when it was sent, by this clock. Decided here, on this
/// machine's own facts.
#[expect(clippy::too_many_arguments, reason = "the frame's parts")]
pub fn start_in(w: &Runtime, from: &str, mid: &str, kind: &str, sender: &str, agent: &str, body: Information<String>, authority: &Value, at: f64) {
    // What the request came in, which the run's receipts go back into
    // from whatever thread they are made on.
    let network = xenonet_net::ground::current();
    let refuse = |why: &str| start_receipt(w, network.as_deref(), from, mid, "refused", &json!({ "reason": why }));
    let now = now_ms();
    {
        let mut taken = w.starts.taken.locked();
        taken.retain(|_, t| now - *t < 2.0 * starts::FRESH_MS);
        if taken.insert(format!("{from}|{mid}"), now).is_some() {
            return;
        }
    }
    if now - at > starts::FRESH_MS {
        return refuse("it waited too long on its way to be started: a start is never kept for later");
    }
    let Ok(v) = serde_json::from_str::<Value>(body.read()) else { return refuse("that isn't a start request") };
    // Decided on the request without its prompt, which is information.
    let start = match starts::request(&v) {
        Ok(r) => apart(r),
        Err(why) => return refuse(&why),
    };
    let r = &start.request;
    let k = kind_of(kind);
    let Some(mut g) = crate::compose::runtime::ground::ground(w, None) else { return refuse("this machine's governance isn't plugged in, so it starts nothing") };
    // The folder it runs in, by the project it is in here: what the network
    // the request came in must be shown.
    let reaching = vec![folder_reach(w, &r.folder)];
    // The signer of a person's start, from the authority it came with: its
    // wall is derived from that, never taken from the request. The
    // signature is checked over the act built with it.
    let signer = (k == Kind::You).then(|| authority.get("person").and_then(|p| p.get("person")).and_then(Value::as_str)).flatten();
    let decided = run_act(&g.facts.machine, r, signer).and_then(|(resource, act)| {
        crate::compose::runtime::network::judge_remote_reaching(
            w,
            &mut g,
            &crate::compose::runtime::network::SentBy { host: from, kind, sender, authority },
            resource,
            act,
            reaching,
        )
    });
    if let Err(why) = decided {
        let why = delegated_words(w, r, k).unwrap_or(why);
        w.log("info", "start", &format!("a start from {from} was refused: {why}"));
        return refuse(&why);
    }
    // Who asked, by its agt_ (its machine's), or you.
    let qualified = if k == Kind::You { YOU.to_string() } else { agent.to_string() };
    let by = json!({ "kind": k.as_str(), "id": qualified, "title": v.get("by").and_then(|b| b.get("title")).and_then(Value::as_str) });
    let (to, m, n) = (from.to_string(), mid.to_string(), network.clone());
    let answer: Answer = Arc::new(move |w: &Runtime, state: &str, a: Value| start_receipt(w, n.as_deref(), &to, &m, state, &a));
    match run_here(w, &by, Some(from), k, &qualified, &start, answer) {
        Ok(first) => start_receipt(w, network.as_deref(), from, mid, "started", &first),
        Err(why) => {
            w.log("info", "start", &format!("a start from {from} was refused: {why}"));
            refuse(&why);
        }
    }
}

/// A receipt for a start request sent from here: true when it was one.
pub fn answer_in(w: &Runtime, from: &str, mid: &str, b: &Value) -> bool {
    let state = b.get("state").and_then(Value::as_str).unwrap_or("");
    let (by, host) = {
        let mut asked = w.starts.asked.locked();
        let Some(a) = asked.get(mid) else { return false };
        // Only the machine asked, and only in the network the request went
        // through, answers it.
        if a.host != from || xenonet_net::ground::current().is_some_and(|n| n != a.network) {
            return true;
        }
        let out = (a.by.clone(), a.host.clone());
        // An end whose process still listens for a reply is not the last.
        if state == "refused" || (state == "ended" && b.get("live").and_then(Value::as_bool) != Some(true)) {
            asked.remove(mid);
        }
        // A week on, an answer that never came is let go.
        asked.retain(|_, a| now_ms() - a.at < 7.0 * 24.0 * 3600e3);
        out
    };
    // The run it names, for the prompts it will send.
    if state != "refused" {
        if let Some(network) = xenonet_net::ground::current() {
            w.starts.note_run(mid, from, &network, b.get("session").and_then(Value::as_str), b.get("agent").and_then(Value::as_str).filter(|a| a.starts_with("agt_")));
        }
    }
    let mut a = b.clone();
    // An older XenoNet refuses the target as an id it doesn't have.
    if state == "refused" && b.get("reason").and_then(Value::as_str) == Some("no session or actor here has that id") {
        a["reason"] = json!(format!("{host}'s XenoNet is too old to start sessions: update it there"));
    }
    tell(w, &by, &host, state, &a);
    true
}

/// Claude Code on `host` needs a sign-in for XenoNet's runs: a Needs-you
/// hint in the console.
pub fn sign_in_hint(w: &Runtime, host: &str) {
    // The app whose runs sign in with the token you gave them.
    let Some(app) = xenonet_engine::delivery::apps::apps().into_iter().find(|(_, w)| w.sign_in.is_some()).map(|(a, _)| a) else { return };
    let words = format!("{} on {host} needs a sign-in for XenoNet's runs", app.name);
    w.emit(From::Engine, "sign-in", json!({ "host": host, "app": app.id, "words": words, "fix": starts::sign_in_words(host), "at": ms(now_ms()) }));
}

#[cfg(test)]
mod tests {
    use super::{acp_for_sandboxed, now_ms, Asked, StartSide, Tail, NAMED_FOR_MS, NAMED_MAX_MS};
    use serde_json::json;
    use std::sync::Arc;
    use xenonet_engine::delivery::starts;
    use xenonet_os::sync::Locked;

    // The token is kept under a name the credential store takes, and read
    // back from it: each store checks the name the same way.
    #[test]
    fn a_sandboxed_start_never_goes_over_acp() {
        assert!(acp_for_sandboxed("Claude Code", true).is_none(), "its headless start takes it, sandboxed");
        let why = acp_for_sandboxed("Nova", false).unwrap().unwrap_err();
        assert!(why.contains("ACP") && why.contains("sandbox"), "{why}");
    }

    #[test]
    fn a_runs_token_is_kept_and_read_back() {
        assert!(xenonet_actors::secret_store::is_target(starts::SIGN_IN_TARGET), "{}", starts::SIGN_IN_TARGET);
        let s = crate::compose::runtime::wakes::Secrets::new(true);
        let token = format!("sk-ant-oat01-{}", "a".repeat(80));
        assert_eq!(s.set(starts::SIGN_IN_TARGET, &token, "Kept by XenoNet").ok(), Some(true));
        assert_eq!(s.get(starts::SIGN_IN_TARGET).ok(), Some(Some(token)));
    }

    #[test]
    fn a_log_is_read_as_it_grows_by_whole_lines() {
        let dir = std::env::temp_dir().join(format!("xenonet-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("run.log");
        let mut t = Tail::new(file.clone());
        assert_eq!(t.lines(), "", "no log yet");
        std::fs::write(&file, "{\"a\":1}\n{\"session_").unwrap();
        assert_eq!(t.lines(), "{\"a\":1}\n");
        assert_eq!(t.lines(), "", "the half line waits");
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, b"id\":\"x\"}\nNot logged in").unwrap();
        assert_eq!(t.lines(), "{\"session_id\":\"x\"}\n", "only what's new, the line made whole");
        assert_eq!(t.rest(), "Not logged in", "at the end, the last unfinished line too");
        assert_eq!(t.rest(), "");
        let _ = std::fs::remove_dir_all(dir);
    }
    fn asked_of(side: &StartSide, mid: &str, host: &str, network: &str) {
        side.asked.locked().insert(mid.into(), Asked { by: json!({}), host: host.into(), network: network.into(), at: now_ms() });
    }

    #[test]
    fn a_machine_names_only_the_runs_it_was_asked_to_start_in_the_network_it_was_asked_through() {
        let side = StartSide::default();
        asked_of(&side, "msg_1", "contractor-vm", "net_work");
        asked_of(&side, "msg_2", "tower", "net_home");
        // Nothing is named before its receipt says so.
        assert!(!side.names_run("contractor-vm", "net_work", "claude:a", None));
        side.note_run("msg_1", "contractor-vm", "net_work", Some("claude:a"), Some("agt_one"));
        side.note_run("msg_2", "tower", "net_home", Some("claude:b"), None);
        assert!(side.names_run("contractor-vm", "net_work", "claude:a", None), "by its session");
        assert!(side.names_run("contractor-vm", "net_work", "claude:other", Some("agt_one")), "or by its agent");
        assert!(!side.names_run("contractor-vm", "net_work", "claude:b", None), "a run asked of tower is not contractor-vm's");
        assert!(!side.names_run("contractor-vm", "net_home", "claude:a", None), "nor in another network");
        assert!(!side.names_run("tower", "net_home", "claude:a", None), "nor from another machine");
        assert!(!side.names_run("contractor-vm", "net_work", "", None), "an ask that names no run names none");
        // A receipt from another machine, or through another network, names nothing.
        side.note_run("msg_1", "tower", "net_work", Some("claude:c"), None);
        side.note_run("msg_1", "contractor-vm", "net_home", Some("claude:c"), None);
        assert!(!side.names_run("contractor-vm", "net_work", "claude:c", None) && !side.names_run("tower", "net_work", "claude:c", None));
        // Once its last receipt has come, the request is dropped, and the run is not open.
        // The request is over, the run is still its own: a later turn of it
        // asks under the same session.
        side.asked.locked().remove("msg_1");
        assert!(side.names_run("contractor-vm", "net_work", "claude:a", Some("agt_one")));
        // A day after it was last named or asked from, it isn't.
        side.kept.locked().iter_mut().for_each(|k| k.at = now_ms() - NAMED_FOR_MS - 1.0);
        assert!(!side.names_run("contractor-vm", "net_work", "claude:a", Some("agt_one")));
    }

    #[test]
    fn a_run_asked_from_stays_named_and_one_not_asked_from_expires() {
        let side = StartSide::default();
        side.note_intent_run("tower", "net_home", Some("agt_two"));
        assert!(side.names_run("tower", "net_home", "", Some("agt_two")), "an intent's run, by its agent");
        assert!(!side.names_run("contractor-vm", "net_home", "", Some("agt_two")), "not another machine's");
        assert!(!side.names_run("tower", "net_work", "", Some("agt_two")), "nor in another network");
        side.note_intent_run("tower", "net_home", None);
        assert_eq!(side.kept.locked().len(), 1, "a report naming no agent names no run");
        side.kept.locked()[0].at = now_ms() - NAMED_FOR_MS + 60e3;
        assert!(side.names_run("tower", "net_home", "", Some("agt_two")));
        assert!(side.kept.locked()[0].at > now_ms() - 60e3, "an ask taken renews it");
    }

    fn store(name: &str) -> (std::path::PathBuf, Arc<xenonet_store::Db>) {
        let dir = std::env::temp_dir().join(format!("xenonet-named-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(xenonet_store::Db::open(&dir).unwrap());
        (dir, db)
    }

    #[test]
    fn the_runs_named_and_their_networks_are_kept_across_a_restart_and_forgotten_by_age() {
        let (dir, db) = store("kept");
        let side = StartSide::open(Some(db.clone()));
        side.note_intent_run("tower", "net_home", Some("agt_two"));
        side.note_intent_run("tower", "net_home", Some("agt_old"));
        side.note_network("claude:a", Some("net_work".into()));
        side.note_network("claude:old", Some("net_work".into()));
        // One of each was last used just under a day ago, and one a week and a moment after its start.
        side.kept.locked().iter_mut().find(|k| k.run.agent.as_deref() == Some("agt_old")).unwrap().since = now_ms() - NAMED_MAX_MS - 1.0;
        side.run_networks.locked().get_mut("claude:old").unwrap().since = now_ms() - NAMED_MAX_MS - 1.0;
        side.note_intent_run("tower", "net_home", Some("agt_two"));
        side.note_network("claude:b", Some("net_home".into()));
        let again = StartSide::open(Some(db));
        assert!(again.names_run("tower", "net_home", "", Some("agt_two")), "a named run survives a restart");
        assert!(!again.names_run("tower", "net_home", "", Some("agt_old")), "one past its week does not");
        assert_eq!(again.network_of("claude:a", None).as_deref(), Some("net_work"));
        assert_eq!(again.network_of("claude:b", None).as_deref(), Some("net_home"));
        assert_eq!(again.network_of("claude:old", None), None, "a run past its week has no network");
        assert_eq!(again.network_of("claude:child", Some("claude:a")).as_deref(), Some("net_work"), "a fork has its parent's");
        let _ = std::fs::remove_dir_all(dir);
    }

    // What names a run is written to the store when the transaction around
    // the entry that named it ends, whether or not it commits: the side
    // keeps what it was told, so the store must not lose it to a rollback.
    #[test]
    fn what_is_named_inside_a_transaction_that_rolls_back_is_still_kept_in_the_store() {
        let (dir, db) = store("rollback");
        let side = StartSide::open(Some(db.clone()));
        let writes = side.hold_writes();
        let rolled = db.transaction(|| -> Result<(), xenonet_store::sql::SqlError> {
            side.note_intent_run("tower", "net_home", Some("agt_two"));
            side.note_network("claude:a", Some("net_work".into()));
            Err(xenonet_store::sql::SqlError::Other("the entry wasn't taken".into()))
        });
        assert!(rolled.is_err());
        drop(writes);
        assert!(side.names_run("tower", "net_home", "", Some("agt_two")), "the side kept it");
        let again = StartSide::open(Some(db));
        assert!(again.names_run("tower", "net_home", "", Some("agt_two")), "and the store has what the side has");
        assert_eq!(again.network_of("claude:a", None).as_deref(), Some("net_work"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn renewing_stops_a_week_after_the_start_and_a_full_table_loses_its_oldest_only() {
        let side = StartSide::default();
        side.note_intent_run("tower", "net_home", Some("agt_two"));
        side.note_network("claude:a", Some("net_home".into()));
        // Asked all week, a day apart: still named, until the week is out.
        for _ in 0..6 {
            side.kept.locked()[0].at = now_ms() - NAMED_FOR_MS + 1e3;
            side.kept.locked()[0].since -= NAMED_FOR_MS;
            assert!(side.names_run("tower", "net_home", "", Some("agt_two")));
        }
        side.kept.locked()[0].at = now_ms() - NAMED_FOR_MS + 1e3;
        side.kept.locked()[0].since -= NAMED_FOR_MS;
        assert!(!side.names_run("tower", "net_home", "", Some("agt_two")), "past a week from its start, an ask renews nothing");
        side.run_networks.locked().get_mut("claude:a").unwrap().since = now_ms() - NAMED_MAX_MS - 1.0;
        assert_eq!(side.network_of("claude:a", None), None);
        // A full table drops the one longest unused, not all of them.
        for i in 0..super::RUN_NETWORKS_MAX {
            side.note_network(&format!("claude:{i}"), Some("net_home".into()));
            side.run_networks.locked().get_mut(&format!("claude:{i}")).unwrap().at = now_ms() - 1e3 * (super::RUN_NETWORKS_MAX - i) as f64;
        }
        side.note_network("claude:new", Some("net_home".into()));
        assert_eq!(side.run_networks.locked().len(), super::RUN_NETWORKS_MAX);
        assert!(side.network_of("claude:0", None).is_none() && side.network_of("claude:new", None).is_some() && side.network_of("claude:1", None).is_some());
    }
}
