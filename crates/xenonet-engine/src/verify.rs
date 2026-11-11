//! Whether a request made in an agent's name comes from that agent. Any
//! program on this machine can reach XenoNet's port and say it's any
//! agent; the process on the other end of the connection can't lie about
//! where it runs (procs). So:
//!
//!   1. A request in a Claude Code session's name must come from a Claude
//!      Code process or from under one, a Codex thread's from a Codex
//!      process or from under it, and an OpenCode session's from an
//!      OpenCode process or from under it. Every OpenCode reads the same
//!      database and can open any session, so OpenCode sessions aren't
//!      pinned to a process.
//!   2. Claude Code sends its HTTP hooks from its own process, which makes
//!      them the word on which process holds which session: when that
//!      process is signed by Anthropic, they pin the session to it (its pid
//!      and start time, since pids are reused).
//!   3. A request for a session pinned to a process that's still running
//!      must come from under that process. A session with no such pin goes
//!      to the first process that names it, until Claude Code's own hooks
//!      say otherwise.
//!
//! What's yours (connecting your agents, removing actors) is refused from
//! under any agent; the window's own routes only pass from XenoNet's own
//! app with no agent nearer; actors' tokens are held to their processes;
//! where XenoNet can look, a request whose process it can't see is refused.
//! The module's own comments say each rule in full.
//!
//! The pins are the engine store's own. The processes and the clock come
//! through `Host`: procs' lookups, and the process on the other end of
//! this request's socket. What's known of each agent app comes from the
//! registry and its adapter (`processes`), and of each actor product from
//! `xenonet_adapter::products`.

use crate::procs::Link;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use xenonet_adapter::products;
use xenonet_os::sync::Locked;
use xenonet_store::engine::Shared;
use xenonet_text::json::number;

/// What a check reads of the machine: procs' lookups and the clock.
pub trait Host {
    /// The time, in epoch ms.
    fn now(&mut self) -> f64;
    /// Whether XenoNet does everything procs does here (macOS, where
    /// codesign can say who signed a running program).
    fn supported(&mut self) -> bool;
    /// Whether XenoNet sees the process on the other end of a connection
    /// and the ones above it: macOS, Linux and Windows.
    fn chains(&mut self) -> bool;
    /// The process on the other end of this request's socket, or None.
    fn peer(&mut self) -> Option<f64>;
    /// A process and the ones above it, nearest first; [] for one that's
    /// gone. `again`: read a process the shared table doesn't have yet by
    /// itself. `exe`: with each one's executable's real path.
    fn chain(&mut self, pid: f64, again: bool, exe: bool) -> Vec<Link>;
    /// Whether the process (pid, start time) is still running.
    fn alive(&mut self, pid: f64, start: f64) -> Result<bool>;
    /// Whether a process runs its app's own code as the app's maker signed
    /// it.
    fn signed(&mut self, p: &Link) -> bool;
    /// The folder a process works in, where it can be told.
    fn cwd(&mut self, pid: f64) -> Option<String>;
}

/// What stopped a check: the store, or procs.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Store(String),
    Procs(String),
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Store(m) | Fail::Procs(m) => f.write_str(m),
        }
    }
}

impl From<xenonet_store::sql::SqlError> for Fail {
    fn from(e: xenonet_store::sql::SqlError) -> Self {
        Fail::Store(e.message().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

/// A check's answer: `{ ok, checked, reason?, root?, sure?, chain?, cwd?,
/// unseen? }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub ok: bool,
    /// Whether the process was looked at: false where XenoNet can't look
    /// and lets the request through.
    pub checked: bool,
    pub reason: Option<String>,
    /// The process holding the session or the token: its pid and start
    /// time.
    pub root: Option<(f64, f64)>,
    /// A session pinned by its own app's hook.
    pub sure: bool,
    /// The caller and the processes above it, for a request that shows you
    /// who asked.
    pub chain: Option<Vec<Link>>,
    /// Where the caller works, when asked: None when the check doesn't
    /// say, Some(None) when it couldn't be told.
    pub cwd: Option<Option<String>>,
    /// Refused because XenoNet couldn't see the process, where it can look.
    pub unseen: bool,
}

impl Verdict {
    fn new(ok: bool, checked: bool) -> Verdict {
        Verdict { ok, checked, reason: None, root: None, sure: false, chain: None, cwd: None, unseen: false }
    }

    /// `{ ok: true, checked: true }`.
    pub fn passed() -> Verdict {
        Verdict::new(true, true)
    }

    /// The verdict as the routes answer it.
    pub fn json(&self) -> Value {
        let mut o = Map::new();
        o.insert("ok".into(), Value::Bool(self.ok));
        o.insert("checked".into(), Value::Bool(self.checked));
        if let Some(reason) = &self.reason {
            o.insert("reason".into(), Value::String(reason.clone()));
        }
        if let Some((pid, start)) = self.root {
            o.insert("root".into(), root_json(pid, start));
        }
        if self.sure {
            o.insert("sure".into(), Value::Bool(true));
        }
        if let Some(chain) = &self.chain {
            o.insert("chain".into(), Value::Array(chain.iter().map(Link::json).collect()));
        }
        if let Some(cwd) = &self.cwd {
            o.insert("cwd".into(), cwd.clone().map_or(Value::Null, Value::String));
        }
        if self.unseen {
            o.insert("unseen".into(), Value::Bool(true));
        }
        Value::Object(o)
    }
}

fn root_json(pid: f64, start: f64) -> Value {
    let mut o = Map::new();
    o.insert("pid".into(), number(pid));
    o.insert("start".into(), number(start));
    Value::Object(o)
}

/// The apps whose sessions XenoNet checks, by their id in a session's name:
/// those whose processes it tells apart (the registry's `processes`, by
/// `processes`): Claude Code, Codex, Grok Build and OpenCode. A request in another
/// app's session's name is refused, wherever XenoNet runs: no check could
/// ever pass it, so it can't call as a session. Actors call with their
/// tokens, and the runs an actor reports come through the gateway, so
/// neither meets this rule.
fn app_name(app: &str) -> Option<&'static str> {
    crate::processes::agent(app).map(|a| a.name)
}

/// Why a request in `agent`'s name, a session of `app`, which XenoNet
/// doesn't check, is refused.
fn uncheckable(agent: &str, app: &str) -> String {
    let why = match xenonet_adapter::apps::app(app) {
        Some(a) => format!("XenoNet can't tell {}'s processes apart, so it can't check that the request comes from that session", a.name),
        None => format!("XenoNet doesn't know an app called \"{app}\""),
    };
    format!("requests in {agent}'s name are refused: {why}")
}

/// An app's own rules, for one XenoNet checks.
fn rules(app: &str) -> Option<crate::processes::Rules> {
    crate::processes::agent(app).map(|a| a.rules)
}

/// An app's name, looked up as a record's field: its own, or for a name
/// every record has (`object_member`), what that member reads as.
fn app_lookup(app: &str) -> Option<String> {
    app_name(app).map(str::to_string).or_else(|| object_member(app))
}

// What a name every plain record has reads as is xenonet-text's.
pub use xenonet_text::text::object_member;

fn a_app(app: &str) -> String {
    let an = app_name(app).is_some_and(|n| n.starts_with(['A', 'E', 'I', 'O', 'U', 'a', 'e', 'i', 'o', 'u']));
    format!("{} {}", if an { "an" } else { "a" }, app_lookup(app).unwrap_or_else(|| "undefined".into()))
}

/// How a reason names a product: "undefined" for one XenoNet hasn't.
fn product_word(id: &str) -> &'static str {
    products::called(id).unwrap_or("undefined")
}

/// The product a process belongs to, by id.
fn product_id(p: &Link) -> Option<&'static str> {
    p.product.map(|pr| pr.product)
}

/// A verdict on a window request is kept this long per process.
const WINDOW_VERDICT_MS: f64 = 60e3;
/// And one on an actor's, per actor, process and the pid it names.
const ACTOR_VERDICT_MS: f64 = 60e3;

/// Whether a chain of processes runs under a product: under one of its
/// agent hosts, or under its app, as the product has it.
fn under(chain: &[Link], product: &str) -> bool {
    let role = products::own_role(product);
    chain.iter().any(|p| p.product.is_some_and(|pr| pr.product == product && pr.role == role))
}

/// Whether a process is an app's (a terminal, an editor, an actor app),
/// which other programs run under too. A product's agent host can live
/// inside its app, and is the product's, not the app's.
fn is_app(p: &Link) -> bool {
    p.row.program.contains(".app/") && p.product.is_none_or(|pr| pr.role != "host")
}

/// The processes an actor's request may call its own, nearest first: the
/// caller, and its parents below the first app above it.
pub fn own_of(chain: &[Link]) -> &[Link] {
    match chain.iter().enumerate().position(|(i, p)| i > 0 && is_app(p)) {
        Some(app) => &chain[..app],
        None => chain,
    }
}

/// Why an actor's request (or a request to register one, `registering`)
/// can't come from this chain, nearest process first, or None when it can.
/// `product`: the product the actor is one of, or None for one of your
/// own; `pid`: the process the request names as the actor's, if any.
pub fn actor_refusal(chain: &[Link], product: Option<&str>, pid: Option<f64>, registering: bool) -> Option<String> {
    if let Some(agent) = chain.iter().find_map(|p| p.agent) {
        let a = a_app(agent);
        return Some(if registering {
            format!("a session can't register an actor, only ask you to, and this came from under {a} process")
        } else {
            format!("an actor's token doesn't work from under a session, and this came from under {a} process")
        });
    }
    if let Some(own) = chain.first().and_then(|p| p.product).filter(|pr| pr.role == "app") {
        let what = if registering { "register an actor" } else { "use an actor's token" };
        return Some(format!("{}'s own processes can't {what}: they could be relaying for its cloud", product_word(own.product)));
    }
    if let Some(name) = product.and_then(products::called) {
        if !under(chain, product.unwrap_or_default()) {
            let verb = if registering { "registers" } else { "works" };
            return Some(format!("{name}'s actor {verb} only from under {name}'s own processes"));
        }
    } else if let Some(inside) = chain.iter().find_map(product_id) {
        let name = product_word(inside);
        return Some(if registering {
            format!("a program under {name} can't register an actor of its own")
        } else {
            format!("an actor of your own can't use its token from under {name}")
        });
    }
    if let Some(pid) = pid {
        if !own_of(chain).iter().any(|p| p.row.pid == pid) {
            return Some(if chain.iter().any(|p| p.row.pid == pid) {
                "the pid it names is an app's, which other programs run under too; name the actor's own process, or a parent of it below the app it runs in".into()
            } else {
                "the pid it names has to be the calling process or one of its parents".into()
            });
        }
    }
    None
}

/// The app bundle a program lies in (/Applications/XenoNet.app/), or its
/// folder when it isn't in one; "" for no program.
pub fn bundle_of(program: Option<&str>) -> String {
    let p = program.unwrap_or("");
    match p.find(".app/") {
        Some(i) => p[..i + ".app/".len()].to_string(),
        None => p[..p.rfind('/').map_or(0, |i| i + 1)].to_string(),
    }
}

/// The file a process from procs.chain runs: its executable's real path
/// when the chain was asked for those (None when it couldn't be read),
/// and otherwise the program ps names.
fn file_of(p: &Link) -> Option<&str> {
    match &p.exe {
        None => Some(&p.row.program),
        Some(exe) => exe.as_deref(),
    }
}

/// What the verifier refused last: the agent named (None for a request
/// in no agent's name), why, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub agent: Option<String>,
    pub reason: String,
    pub at: f64,
}

/// What the verifier has counted, for the doctor.
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub checked: f64,
    pub refused: f64,
    pub unchecked: f64,
    pub unseen: f64,
    pub last_refusal: Option<Refusal>,
}

struct Inner {
    counts: Counts,
    /// A window verdict by the process's pid (its bits).
    verdicts: HashMap<u64, (f64, Verdict)>,
    actor_verdicts: HashMap<String, (f64, Verdict)>,
    desktops: u64,
    home: Option<f64>,
}

pub struct Verifier {
    store: Shared,
    inner: Mutex<Inner>,
}

/// A pid as a map's key: the same for 0 and -0.
fn key_of(pid: f64) -> u64 {
    if pid == 0.0 {
        0
    } else {
        pid.to_bits()
    }
}

/// A number as a reason or a key writes it.
fn num_text(n: f64) -> String {
    xenonet_text::json::number_string(n)
}

impl Verifier {
    /// `home`: the process of XenoNet's own app, when an app started the
    /// server; else the Desktop that connects says (`desktop`).
    pub fn new(store: Shared, home: Option<f64>) -> Self {
        Verifier { store, inner: Mutex::new(Inner { counts: Counts::default(), verdicts: HashMap::new(), actor_verdicts: HashMap::new(), desktops: 0, home }) }
    }

    pub fn counts(&self) -> Counts {
        self.inner.locked().counts.clone()
    }

    fn pass(&self) -> Verdict {
        self.inner.locked().counts.unchecked += 1.0;
        Verdict::new(true, false)
    }

    fn refuse(&self, h: &mut dyn Host, agent: Option<&str>, reason: &str) -> Verdict {
        self.inner.locked().counts.refused += 1.0;
        let at = h.now();
        self.inner.locked().counts.last_refusal = Some(Refusal { agent: agent.map(str::to_string), reason: reason.to_string(), at });
        let mut v = Verdict::new(false, true);
        v.reason = Some(reason.to_string());
        v
    }

    /// A request whose process XenoNet couldn't see, where it can look:
    /// never kept as a verdict.
    fn unseen(&self, h: &mut dyn Host, agent: Option<&str>, reason: &str) -> Verdict {
        self.inner.locked().counts.unseen += 1.0;
        let mut v = self.refuse(h, agent, reason);
        v.unseen = true;
        v
    }

    fn checked(&self) {
        self.inner.locked().counts.checked += 1.0;
    }

    fn home(&self) -> Option<f64> {
        self.inner.locked().home
    }

    /// The process on the other end of this request's socket, or None: a
    /// pid of 0, or none that can be told, is no process.
    fn peer_pid(h: &mut dyn Host) -> Option<f64> {
        h.peer().filter(|p| *p != 0.0 && !p.is_nan())
    }

    /// The process on the other end of the socket and the ones above it.
    fn see(h: &mut dyn Host, exe: bool) -> (Option<f64>, Vec<Link>) {
        let Some(pid) = Self::peer_pid(h) else { return (None, Vec::new()) };
        let chain = h.chain(pid, true, exe);
        (Some(pid), chain)
    }

    /// `hook`: the request is one of the app's hooks, which for Claude Code
    /// may come from Claude Code itself. { ok, checked, root?, reason? }.
    pub fn check(&self, h: &mut dyn Host, agent: &str, hook: bool) -> Result<Verdict> {
        let app = agent.split(':').next().unwrap_or("");
        if app_name(app).is_none() {
            return Ok(self.refuse(h, Some(agent), &uncheckable(agent, app)));
        }
        if !h.supported() {
            return Ok(self.pass());
        }
        let (pid, chain) = Self::see(h, false);
        if pid.is_none() {
            return Ok(self.pass());
        }
        if chain.is_empty() {
            return Ok(self.unseen(
                h,
                Some(agent),
                &format!("requests in {agent}'s name have to come from {} process, and this one had gone before XenoNet could look", a_app(app)),
            ));
        }
        self.checked();
        let Some(holder) = chain.iter().find(|p| p.agent == Some(app)) else {
            return Ok(self.refuse(h, Some(agent), &format!("requests in {agent}'s name have to come from {} process, and this one doesn't", a_app(app))));
        };
        let root = (holder.row.pid, holder.row.start);
        let ok = |sure: bool| {
            let mut v = Verdict::passed();
            v.root = Some(root);
            v.sure = sure;
            v
        };
        // Any OpenCode may have the session open.
        if rules(app).is_some_and(|r| r.any_process) {
            return Ok(ok(false));
        }
        // The pins of processes still running; the others go.
        let mut held: Vec<(f64, f64, bool)> = Vec::new();
        for p in self.store.with(|s| s.pins(agent))? {
            let (pid, start) = (p.num("pid").unwrap_or(f64::NAN), p.num("start").unwrap_or(f64::NAN));
            if h.alive(pid, start)? {
                held.push((pid, start, p.num("sure").is_some_and(|s| s != 0.0)));
            } else {
                self.store.with(|s| s.unpin(agent, pid, start))?;
            }
        }
        // Claude Code itself, in one of its HTTP hooks: this process holds
        // the session.
        if hook && rules(app).is_some_and(|r| r.pins_by_hook) && chain[0].row.pid == holder.row.pid && h.signed(holder) {
            let t = h.now();
            self.store.with(|s| s.pin(agent, root.0, root.1, true, t))?;
            self.store.with(|s| s.drop_guesses(agent))?;
            return Ok(ok(true));
        }
        if let Some((_, _, sure)) = held.iter().find(|(pid, start, _)| *pid == root.0 && *start == root.1) {
            let t = h.now();
            self.store.with(|s| s.pin(agent, root.0, root.1, *sure, t))?;
            return Ok(ok(false));
        }
        if !held.is_empty() {
            let word = app_lookup(app).unwrap_or_else(|| "undefined".into());
            return Ok(self.refuse(h, Some(agent), &format!("{agent} belongs to another {word} process")));
        }
        let t = h.now();
        self.store.with(|s| s.pin(agent, root.0, root.1, false, t))?;
        Ok(ok(false))
    }

    /// Requests only you make: not from any agent, nor from under an actor
    /// product's processes. The nearer of one of those and XenoNet's own
    /// app decides. Wherever XenoNet sees processes' chains: macOS, Linux
    /// and Windows.
    pub fn person(&self, h: &mut dyn Host) -> Verdict {
        if !h.chains() {
            return self.pass();
        }
        let (_, chain) = Self::see(h, false);
        if chain.is_empty() {
            return self.unseen(h, None, "this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet");
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().find(|p| p.agent.is_some() || p.product.is_some() || home == Some(p.row.pid));
        if let Some(near) = near {
            if let Some(agent) = near.agent {
                return self.refuse(h, None, &format!("this is yours to do, and it came from under {} process; run it in your own terminal or in XenoNet", a_app(agent)));
            }
            if let Some(product) = product_id(near) {
                return self.refuse(h, None, &format!("this is yours to do, and it came from under {}; run it in your own terminal or in XenoNet", product_word(product)));
            }
        }
        Verdict::passed()
    }

    /// Why a call under a product whose maker signs its app isn't that
    /// app's, or None.
    fn unsigned(h: &mut dyn Host, chain: &[Link], product: Option<&str>, registering: bool) -> Option<String> {
        let product = product?;
        let maker = products::signer(product)?;
        let app = chain.iter().find(|p| p.product.is_some_and(|pr| pr.product == product && pr.role == "app"));
        if let Some(app) = app {
            if h.signed(app) {
                return None;
            }
        }
        let name = product_word(product);
        let verb = if registering { "registers" } else { "works" };
        Some(format!("{name}'s actor {verb} only from under {name}'s app as {maker} signed it, and the app this came from isn't signed by {maker}"))
    }

    /// A request with an actor's token (`slug`: the actor's, `product`: the
    /// product it's one of, or None for one of your own), naming `pid` as
    /// its process or not. A `pid` that's no process's (NaN) names one the
    /// chain can't have.
    pub fn actor(&self, h: &mut dyn Host, slug: &str, product: Option<&str>, pid: Option<f64>) -> Verdict {
        if !h.supported() {
            return self.pass();
        }
        let id = (!slug.is_empty()).then(|| format!("actor:{slug}"));
        let blind = "XenoNet couldn't see which program sent this, so it doesn't take the token from it";
        let Some(peer) = Self::peer_pid(h) else {
            return self.unseen(h, id.as_deref(), blind);
        };
        let key = format!("{slug}|{}|{}|{}", product.unwrap_or(""), num_text(peer), pid.map(num_text).unwrap_or_default());
        let kept = self.inner.locked().actor_verdicts.get(&key).cloned();
        if let Some((at, verdict)) = kept {
            if h.now() - at < ACTOR_VERDICT_MS {
                return verdict;
            }
        }
        let chain = h.chain(peer, true, true);
        if chain.is_empty() {
            return self.unseen(h, id.as_deref(), blind);
        }
        self.checked();
        let why = match actor_refusal(&chain, product, pid, false) {
            Some(w) => Some(w),
            None => Self::unsigned(h, &chain, product, false),
        };
        let verdict = match why {
            Some(why) => self.refuse(h, id.as_deref(), &why),
            None => {
                let mut v = Verdict::passed();
                v.root = Some((chain[0].row.pid, chain[0].row.start));
                v.chain = Some(chain);
                v
            }
        };
        let at = h.now();
        let mut inner = self.inner.locked();
        if inner.actor_verdicts.len() > 256 {
            inner.actor_verdicts.clear();
        }
        inner.actor_verdicts.insert(key, (at, verdict.clone()));
        verdict
    }

    /// A request to register an actor, asking to be `product` (or an actor
    /// of your own), naming `pid` or not.
    pub fn register(&self, h: &mut dyn Host, product: Option<&str>, pid: Option<f64>) -> Verdict {
        if !h.supported() {
            return self.pass();
        }
        let (peer, chain) = Self::see(h, true);
        if chain.is_empty() {
            return self.unseen(h, None, "XenoNet couldn't see which program asked to register, so it can't show you who asked");
        }
        self.checked();
        let why = match actor_refusal(&chain, product, pid, true) {
            Some(w) => Some(w),
            None => Self::unsigned(h, &chain, product, true),
        };
        if let Some(why) = why {
            return self.refuse(h, None, &why);
        }
        let mut v = Verdict::passed();
        v.cwd = Some(peer.and_then(|p| h.cwd(p)));
        v.chain = Some(chain);
        v
    }

    /// A request that asks you for something and holds no secret: from a
    /// session or your own terminal, but never from under an actor
    /// product's processes.
    pub fn asker(&self, h: &mut dyn Host) -> Verdict {
        if !h.supported() {
            return self.pass();
        }
        let (peer, chain) = Self::see(h, true);
        if chain.is_empty() {
            return self.unseen(h, None, "XenoNet couldn't see which program asked, so it can't show you who asked");
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().find(|p| p.product.is_some() || home == Some(p.row.pid));
        if let Some(product) = near.and_then(product_id) {
            return self.refuse(h, None, &format!("a program under {} can't ask for a gateway token: it could be relaying for its cloud", product_word(product)));
        }
        let mut v = Verdict::passed();
        v.cwd = Some(peer.and_then(|p| h.cwd(p)));
        v.chain = Some(chain);
        v
    }

    /// The process on the other end of a socket, or None where XenoNet
    /// can't look.
    pub fn peer(&self, h: &mut dyn Host) -> Option<f64> {
        if !h.supported() {
            return None;
        }
        Self::peer_pid(h)
    }

    /// A running process and the ones above it, nearest first, as pids;
    /// None where XenoNet can't look, and [] for a process that's gone (or
    /// none named).
    pub fn lineage(&self, h: &mut dyn Host, pid: Option<f64>) -> Option<Vec<f64>> {
        if !h.supported() {
            return None;
        }
        Some(pid.map_or_else(Vec::new, |pid| h.chain(pid, false, false).iter().map(|p| p.row.pid).collect()))
    }

    /// Requests only XenoNet's own window makes: from a process of
    /// XenoNet's app, with no agent nearer.
    pub fn window(&self, h: &mut dyn Host) -> Verdict {
        if !h.supported() {
            return self.pass();
        }
        let blind = "this is for XenoNet's own window, and XenoNet couldn't see which program asked";
        let Some(pid) = Self::peer_pid(h) else {
            return self.unseen(h, None, blind);
        };
        let key = key_of(pid);
        let kept = self.inner.locked().verdicts.get(&key).cloned();
        if let Some((at, verdict)) = kept {
            if h.now() - at < WINDOW_VERDICT_MS {
                return verdict;
            }
        }
        let chain = h.chain(pid, true, true);
        if chain.is_empty() {
            return self.unseen(h, None, blind);
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().position(|p| p.agent.is_some() || (home.is_some() && home == Some(p.row.pid)));
        let verdict = match near {
            Some(i) if chain[i].agent.is_some() => {
                self.refuse(h, None, &format!("this is for XenoNet's own window, and it came from under {} process", a_app(chain[i].agent.unwrap_or_default())))
            }
            None => self.refuse(h, None, "this is for XenoNet's own window, and it came from outside XenoNet"),
            Some(i) => {
                // Every process from the caller up to XenoNet's app is the
                // app's own, by the executable's real path.
                let bundle = bundle_of(Some(file_of(&chain[i]).unwrap_or(&chain[i].row.program)));
                let foreign = chain[..i].iter().any(|p| !file_of(p).is_some_and(|f| f.starts_with(&bundle)));
                if foreign {
                    self.refuse(h, None, "this is for XenoNet's own window, and it came from a program run in XenoNet")
                } else {
                    Verdict::passed()
                }
            }
        };
        let at = h.now();
        let mut inner = self.inner.locked();
        if inner.verdicts.len() > 256 {
            inner.verdicts.clear();
        }
        inner.verdicts.insert(key, (at, verdict.clone()));
        verdict
    }

    /// XenoNet's Desktop, where the server is a service no app started:
    /// the process on the other end of the Desktop's connection becomes
    /// home. Only an app's own executable, by its real path, that isn't an
    /// agent or an actor product's. `connected` false: the Desktop went.
    pub fn desktop(&self, h: &mut dyn Host, connected: bool) -> Verdict {
        let mine = {
            let mut i = self.inner.locked();
            i.desktops += 1;
            i.home = None;
            i.verdicts.clear();
            i.desktops
        };
        if !connected {
            return Verdict::new(true, false);
        }
        if !h.supported() {
            // Where XenoNet sees chains but can't vouch for an app's own
            // executable (Linux, Windows), the Desktop still passes
            // unchecked, and becomes home for `person` only with no agent
            // and no actor product anywhere above it.
            if h.chains() {
                let (pid, chain) = Self::see(h, false);
                if pid.is_some() && !chain.is_empty() && !chain.iter().any(|p| p.agent.is_some() || p.product.is_some()) {
                    let mut i = self.inner.locked();
                    if i.desktops == mine {
                        i.home = pid;
                    }
                }
            }
            return self.pass();
        }
        let (pid, chain) = Self::see(h, true);
        if pid.is_none() || chain.is_empty() {
            return self.unseen(h, None, "XenoNet couldn't see which program connected as its Desktop");
        }
        self.checked();
        let p = &chain[0];
        let file = file_of(p).unwrap_or("");
        let verdict = if let Some(agent) = p.agent {
            self.refuse(h, None, &format!("{} process can't be XenoNet's Desktop", a_app(agent)))
        } else if let Some(product) = product_id(p) {
            self.refuse(h, None, &format!("{}'s processes can't be XenoNet's Desktop", product_word(product)))
        } else if !crate::re!(r"\.app/Contents/MacOS/[^/]+$").is_match(file) {
            self.refuse(h, None, "only an app's own executable can be XenoNet's Desktop")
        } else {
            Verdict::passed()
        };
        if verdict.ok {
            let mut i = self.inner.locked();
            if i.desktops == mine {
                i.home = Some(p.row.pid);
            }
        }
        verdict
    }

    /// XenoNet's tray app, connecting to the component socket for the key
    /// its routes take. Only XenoNet Tray.app's own executable, by its real
    /// path, in one of the Applications folders `apps` names (/Applications
    /// and the home folder's), with no agent and no actor product anywhere
    /// above it. A program an agent runs, or one copied elsewhere, never
    /// gets the key. Never passed unchecked: the tray is a Mac's, and where
    /// XenoNet can't look there's no telling who asks.
    pub fn tray(&self, h: &mut dyn Host, apps: &[String]) -> Verdict {
        if !h.supported() {
            return self.refuse(h, None, "XenoNet can't see which program connects as its tray on this system");
        }
        let (pid, chain) = Self::see(h, true);
        if pid.is_none() || chain.is_empty() {
            return self.unseen(h, None, "XenoNet couldn't see which program connected as its tray");
        }
        self.checked();
        if let Some(agent) = chain.iter().find_map(|p| p.agent) {
            return self.refuse(h, None, &format!("XenoNet's tray can't run under {} process", a_app(agent)));
        }
        if let Some(product) = chain.iter().find_map(product_id) {
            return self.refuse(h, None, &format!("XenoNet's tray can't run under {}", product_word(product)));
        }
        let file = file_of(&chain[0]).unwrap_or("");
        let own = apps.iter().filter(|dir| !dir.is_empty()).any(|dir| {
            let inside = format!("{}/{TRAY_APP}/Contents/MacOS/", dir.trim_end_matches('/'));
            file.strip_prefix(&inside).is_some_and(|exe| !exe.is_empty() && !exe.contains('/'))
        });
        if !own {
            return self.refuse(h, None, &format!("only {TRAY_APP}'s own executable, in /Applications or your own Applications folder, can be XenoNet's tray"));
        }
        Verdict::passed()
    }
}

/// The tray app's bundle, as it's installed.
pub const TRAY_APP: &str = "XenoNet Tray.app";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{Product, Row};
    use serde_json::json;

    fn product_of(product: Option<(&'static str, &'static str)>) -> Option<Product> {
        product.map(|(id, role)| Product { product: id, role })
    }

    /// A process with no start time and no executable read.
    fn p(pid: f64, program: &str, agent: Option<&'static str>, product: Option<(&'static str, &'static str)>) -> Link {
        Link { row: Row { pid, ppid: 0.0, start: 0.0, program: program.into() }, exe: None, agent, product: product_of(product) }
    }

    #[test]
    fn an_actor_is_held_to_its_own_processes() {
        xenonet_platforms::install();
        let chain = vec![p(10.0, "/bin/zsh", None, None), p(5.0, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", None, None)];
        assert_eq!(own_of(&chain).len(), 1);
        assert_eq!(actor_refusal(&chain, None, None, false), None);
        assert!(actor_refusal(&chain, None, Some(5.0), false).unwrap().contains("an app's"));
        assert!(actor_refusal(&chain, Some("grok-bot"), None, true).unwrap().starts_with("Grok Bot's actor registers"));
        let agent = vec![p(10.0, "/x", Some("claude"), None)];
        assert!(actor_refusal(&agent, None, None, false).unwrap().ends_with("a Claude Code process"));
    }

    #[test]
    fn bundles_are_the_app_or_the_folder() {
        xenonet_platforms::install();
        assert_eq!(bundle_of(Some("/Applications/XenoNet.app/Contents/MacOS/XenoNet")), "/Applications/XenoNet.app/");
        assert_eq!(bundle_of(Some("/usr/local/bin/node")), "/usr/local/bin/");
        assert_eq!(bundle_of(None), "");
    }

    #[test]
    fn an_apps_name_is_read_as_an_object_literal_reads_it() {
        xenonet_platforms::install();
        assert_eq!(app_lookup("toString").as_deref(), Some("function toString() { [native code] }"));
        assert_eq!(a_app("opencode"), "an OpenCode");
        assert_eq!(app_lookup("nobody"), None);
    }

    // ---------- a made-up process table ----------

    const XENONET: &str = "/Applications/XenoNet.app/Contents/MacOS/XenoNet";
    const TERMINAL: &str = "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal";
    const GROK: &str = "/Applications/Grok Bot.app/Contents/MacOS/Grok Bot";

    /// A process as procs.chain gives one: its pid and start time, the
    /// file it runs, and the agent or actor product ps took it for.
    fn proc(pid: f64, exe: &str, agent: Option<&'static str>, product: Option<(&'static str, &'static str)>) -> Link {
        Link { row: Row { pid, ppid: 0.0, start: pid * 10.0, program: exe.into() }, exe: Some(Some(exe.into())), agent, product: product_of(product) }
    }

    /// procs and the clock as a test has them: whether XenoNet can look,
    /// the time, the process on the other end of the request, each
    /// process's chain (nearest first), the ones that have gone, the ones
    /// signed, and a working folder.
    struct Table {
        supported: bool,
        // procs' `chains`, when it says (Linux and Windows: true while
        // `supported` is false).
        sees: Option<bool>,
        now: f64,
        peer: Option<f64>,
        chains: Vec<Vec<Link>>,
        gone: Vec<f64>,
        signed: Vec<f64>,
        cwd: Option<&'static str>,
    }

    impl Table {
        fn new(chains: Vec<Vec<Link>>) -> Table {
            Table { supported: true, sees: None, now: 1_000_000.0, peer: None, chains, gone: vec![], signed: vec![], cwd: None }
        }
        /// A request from this process.
        fn from(&mut self, pid: f64) -> &mut Table {
            self.peer = Some(pid);
            self
        }
    }

    impl Host for Table {
        fn now(&mut self) -> f64 {
            self.now
        }
        fn supported(&mut self) -> bool {
            self.supported
        }
        fn chains(&mut self) -> bool {
            self.sees.unwrap_or(self.supported)
        }
        fn peer(&mut self) -> Option<f64> {
            self.peer
        }
        fn chain(&mut self, pid: f64, _again: bool, _exe: bool) -> Vec<Link> {
            self.chains.iter().find(|c| c.first().is_some_and(|p| p.row.pid == pid)).cloned().unwrap_or_default()
        }
        fn alive(&mut self, pid: f64, _start: f64) -> Result<bool> {
            Ok(!self.gone.contains(&pid))
        }
        fn signed(&mut self, p: &Link) -> bool {
            self.signed.contains(&p.row.pid)
        }
        fn cwd(&mut self, _pid: f64) -> Option<String> {
            self.cwd.map(str::to_string)
        }
    }

    fn verifier(home: Option<f64>) -> Verifier {
        Verifier::new(Shared::new(xenonet_store::engine::EngineStore::memory(0.0).unwrap()), home)
    }

    /// A verdict, as (ok, the reason it says).
    fn said(v: &Verdict) -> (bool, String) {
        (v.ok, v.reason.clone().unwrap_or_default())
    }

    fn passed() -> (bool, String) {
        (true, String::new())
    }

    fn refused(why: &str) -> (bool, String) {
        (false, why.to_string())
    }

    #[test]
    fn a_sessions_requests_come_from_its_own_apps_process() {
        xenonet_platforms::install();
        let shell = proc(40.0, "/bin/zsh", None, None);
        let claude = |pid: f64| proc(pid, "/Users/me/.local/bin/claude", Some("claude"), None);
        let mut t = Table::new(vec![
            vec![proc(30.0, "/usr/bin/curl", None, None), shell, proc(5.0, TERMINAL, None, None)],
            vec![proc(31.0, "/usr/bin/node", None, None), claude(20.0)],
            vec![proc(32.0, "/usr/bin/node", None, None), claude(21.0)],
            vec![claude(22.0)],
            vec![proc(33.0, "/usr/bin/node", None, None), proc(23.0, "/usr/local/bin/codex", Some("codex"), None)],
            vec![proc(34.0, "/usr/bin/node", None, None), proc(24.0, "/usr/local/bin/opencode", Some("opencode"), None)],
            vec![proc(35.0, "/usr/bin/node", None, None), proc(25.0, "/usr/local/bin/opencode", Some("opencode"), None)],
        ]);
        let v = verifier(None);
        let a = "claude:a";
        // Where XenoNet can't look, a checked app's session passes.
        t.supported = false;
        assert_eq!(said(&v.check(&mut t, a, false).unwrap()), passed());
        // A session of an app XenoNet doesn't check is refused, there too:
        // no check could ever pass it.
        let cursor = "requests in cursor:a's name are refused: XenoNet can't tell Cursor's processes apart, so it can't check that the request comes from that session";
        assert_eq!(said(&v.check(&mut t, "cursor:a", false).unwrap()), refused(cursor));
        t.supported = true;
        assert_eq!(said(&v.check(&mut t, "cursor:a", false).unwrap()), refused(cursor));
        assert_eq!(said(&v.check(t.from(30.0), "gemini-cli:a", true).unwrap()), refused(&cursor.replace("cursor", "gemini-cli").replace("Cursor", "Gemini CLI")), "a hook's too");
        assert_eq!(said(&v.check(&mut t, "nobody:a", false).unwrap()), refused("requests in nobody:a's name are refused: XenoNet doesn't know an app called \"nobody\""));
        assert_eq!(said(&v.check(&mut t, "toString:a", false).unwrap()), refused("requests in toString:a's name are refused: XenoNet doesn't know an app called \"toString\""));
        t.peer = None;
        assert_eq!(said(&v.check(&mut t, a, false).unwrap()), passed(), "no peer to look at");
        assert_eq!((v.counts().unchecked, v.counts().refused), (2.0, 5.0));
        // A process that's gone, or isn't the app's.
        let gone = "requests in claude:a's name have to come from a Claude Code process, and this one had gone before XenoNet could look";
        assert_eq!(said(&v.check(t.from(99.0), a, false).unwrap()), refused(gone));
        assert_eq!(v.counts().unseen, 1.0);
        let not = "requests in claude:a's name have to come from a Claude Code process, and this one doesn't";
        assert_eq!(said(&v.check(t.from(30.0), a, false).unwrap()), refused(not));
        assert_eq!(said(&v.check(t.from(33.0), a, false).unwrap()), refused(not), "a Codex process for a Claude Code session");
        assert_eq!(said(&v.check(t.from(33.0), "codex:b", false).unwrap()), passed());
        // The first process to name a session holds it while it runs.
        let r = v.check(t.from(31.0), a, false).unwrap();
        assert_eq!((r.ok, r.root.map(|r| r.0), r.sure), (true, Some(20.0), false));
        assert_eq!(r.json(), json!({ "ok": true, "checked": true, "root": { "pid": 20, "start": 200 } }));
        assert_eq!(said(&v.check(t.from(31.0), a, false).unwrap()), passed());
        assert_eq!(said(&v.check(t.from(32.0), a, false).unwrap()), refused("claude:a belongs to another Claude Code process"));
        t.gone.push(20.0);
        assert_eq!(said(&v.check(t.from(32.0), a, false).unwrap()), passed(), "once it's gone, another may have it");
        // Claude Code's own hook, from a signed Claude Code, pins it surely,
        // and takes back the guesses.
        let b = "claude:b";
        assert_eq!(said(&v.check(t.from(32.0), b, false).unwrap()), passed());
        assert_eq!(said(&v.check(t.from(22.0), b, true).unwrap()), refused("claude:b belongs to another Claude Code process"), "a hook from a Claude Code not signed");
        t.signed.push(22.0);
        let r = v.check(t.from(22.0), b, true).unwrap();
        assert_eq!((r.ok, r.sure), (true, true));
        assert_eq!(r.json()["sure"], json!(true));
        assert_eq!(said(&v.check(t.from(32.0), b, false).unwrap()), refused("claude:b belongs to another Claude Code process"));
        // Any OpenCode may have an OpenCode session open.
        let o = "opencode:c";
        assert_eq!((said(&v.check(t.from(34.0), o, false).unwrap()), said(&v.check(t.from(35.0), o, false).unwrap())), (passed(), passed()));
        let last = v.counts().last_refusal.unwrap();
        assert_eq!((last.agent.as_deref(), last.at), (Some(b), 1_000_000.0));
    }

    #[test]
    fn whats_yours_comes_only_from_outside_every_agent_and_actor_product() {
        xenonet_platforms::install();
        let mut t = Table::new(vec![
            vec![proc(30.0, "/usr/bin/curl", None, None), proc(29.0, "/bin/zsh", None, None), proc(5.0, TERMINAL, None, None)],
            vec![proc(31.0, "/usr/bin/curl", None, None), proc(23.0, "/usr/local/bin/codex", Some("codex"), None), proc(5.0, TERMINAL, None, None)],
            vec![proc(32.0, "/usr/bin/curl", None, None), proc(7.0, GROK, None, Some(("grok-bot", "app")))],
            vec![proc(33.0, "/bin/zsh", None, None), proc(100.0, XENONET, None, None), proc(23.0, "/usr/local/bin/codex", Some("codex"), None)],
        ]);
        let v = verifier(Some(100.0));
        assert_eq!(said(&v.person(t.from(30.0))), passed());
        assert_eq!(said(&v.person(t.from(31.0))), refused("this is yours to do, and it came from under a Codex process; run it in your own terminal or in XenoNet"));
        assert_eq!(said(&v.person(t.from(32.0))), refused("this is yours to do, and it came from under Grok Bot; run it in your own terminal or in XenoNet"));
        assert_eq!(said(&v.person(t.from(33.0))), passed(), "XenoNet's own terminal is nearer than the agent that opened XenoNet");
        assert_eq!(said(&v.person(t.from(99.0))), refused("this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet"));
        // Asking for a gateway token: a session may, a product's program may not.
        assert_eq!(said(&v.asker(t.from(31.0))), passed());
        assert_eq!(said(&v.asker(t.from(32.0))), refused("a program under Grok Bot can't ask for a gateway token: it could be relaying for its cloud"));
        assert_eq!(said(&v.asker(t.from(99.0))), refused("XenoNet couldn't see which program asked, so it can't show you who asked"));
        assert_eq!(v.asker(t.from(31.0)).json()["cwd"], Value::Null, "a folder that couldn't be told");
        t.cwd = Some("/Users/me/projects/atlas");
        assert_eq!(v.asker(t.from(30.0)).cwd, Some(Some("/Users/me/projects/atlas".into())));
        t.supported = false;
        assert_eq!((said(&v.person(&mut t)), said(&v.asker(&mut t))), (passed(), passed()));
    }

    #[test]
    fn on_linux_and_windows_whats_yours_is_held_to_the_chain_alone() {
        xenonet_platforms::install();
        // As procs reads them there: a program's path, no app bundles.
        let claude = |pid: f64| proc(pid, "C:/Users/me/.local/bin/claude.exe", Some("claude"), None);
        let desktop = proc(100.0, "C:/Users/me/AppData/Local/Programs/XenoNet/XenoNet.exe", None, None);
        let explorer = proc(5.0, "C:/Windows/explorer.exe", None, None);
        let launcher = "C:/Users/me/AppData/Local/XenoNet/runtime/launcher/xenonet.exe";
        let mut t = Table::new(vec![
            vec![proc(30.0, launcher, None, None), proc(29.0, "C:/Windows/System32/cmd.exe", None, None), claude(20.0), explorer.clone()],
            vec![proc(31.0, launcher, None, None), proc(28.0, "C:/Program Files/PowerShell/7/pwsh.exe", None, None), explorer.clone()],
            vec![desktop.clone(), explorer],
            vec![proc(33.0, "C:/Windows/System32/cmd.exe", None, None), desktop, claude(21.0)],
            vec![proc(101.0, "C:/w/xenonet/node_modules/electron/dist/electron.exe", None, None), claude(22.0)],
        ]);
        t.supported = false;
        t.sees = Some(true);
        let v = verifier(None);
        let under_claude = "this is yours to do, and it came from under a Claude Code process; run it in your own terminal or in XenoNet";
        // xenonet connect --yes from under Claude Code is refused; from your
        // own terminal it passes, checked.
        assert_eq!(said(&v.person(t.from(30.0))), refused(under_claude));
        assert_eq!(v.person(t.from(31.0)).json(), json!({ "ok": true, "checked": true }));
        assert_eq!(said(&v.person(t.from(99.0))), refused("this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet"));
        // The rest stays as it was there: unchecked.
        assert!(!v.check(t.from(30.0), "codex:a", false).unwrap().checked);
        assert!(!v.window(t.from(101.0)).checked);
        // A Desktop with an agent above it passes unchecked, but isn't home.
        assert!(!v.desktop(t.from(101.0), true).checked);
        assert_eq!(said(&v.person(t.from(33.0))), refused(under_claude));
        // XenoNet's own Desktop, with nothing of an agent's above it, is
        // home: its terminal is nearer than an agent above it.
        v.desktop(t.from(100.0), true);
        assert_eq!(said(&v.person(t.from(33.0))), passed());
        // Where XenoNet sees no chains at all, it passes, unchecked.
        t.sees = Some(false);
        assert_eq!(v.person(t.from(30.0)).json(), json!({ "ok": true, "checked": false }));
    }

    #[test]
    fn the_windows_routes_answer_only_xenonets_own_app() {
        xenonet_platforms::install();
        let helper = "/Applications/XenoNet.app/Contents/Frameworks/XenoNet Helper (Renderer).app/Contents/MacOS/XenoNet Helper (Renderer)";
        let home = proc(100.0, XENONET, None, None);
        let mut t = Table::new(vec![
            vec![proc(101.0, helper, None, None), home.clone()],
            vec![proc(102.0, "/bin/zsh", None, None), home.clone()],
            vec![proc(103.0, "/Applications/XenoNet.app2/Contents/MacOS/x", None, None), home.clone()],
            vec![proc(104.0, "/Users/me/.local/bin/claude", Some("claude"), None), home.clone()],
            vec![proc(105.0, "/usr/bin/curl", None, None), proc(5.0, TERMINAL, None, None)],
        ]);
        let v = verifier(None);
        // Until a Desktop connects there's no XenoNet app to come from.
        assert_eq!(said(&v.window(t.from(101.0))), refused("this is for XenoNet's own window, and it came from outside XenoNet"));
        t.now += 60e3;
        // The Desktop: an app's own executable, not an agent's or a product's.
        let mut d = Table::new(vec![
            vec![proc(110.0, "/Users/me/.local/bin/claude", Some("claude"), None)],
            vec![proc(111.0, GROK, None, Some(("grok-bot", "app")))],
            vec![proc(112.0, "/usr/local/bin/node", None, None)],
            vec![home],
        ]);
        assert_eq!(said(&v.desktop(&mut d, true)), refused("XenoNet couldn't see which program connected as its Desktop"));
        assert_eq!(said(&v.desktop(d.from(110.0), true)), refused("a Claude Code process can't be XenoNet's Desktop"));
        assert_eq!(said(&v.desktop(d.from(111.0), true)), refused("Grok Bot's processes can't be XenoNet's Desktop"));
        assert_eq!(said(&v.desktop(d.from(112.0), true)), refused("only an app's own executable can be XenoNet's Desktop"));
        assert_eq!(said(&v.desktop(d.from(100.0), true)), passed());
        // Now the window's own processes pass, and nothing else.
        assert_eq!(said(&v.window(t.from(101.0))), passed());
        assert_eq!(said(&v.window(t.from(102.0))), refused("this is for XenoNet's own window, and it came from a program run in XenoNet"));
        assert_eq!(said(&v.window(t.from(103.0))), refused("this is for XenoNet's own window, and it came from a program run in XenoNet"));
        assert_eq!(said(&v.window(t.from(104.0))), refused("this is for XenoNet's own window, and it came from under a Claude Code process"));
        assert_eq!(said(&v.window(t.from(105.0))), refused("this is for XenoNet's own window, and it came from outside XenoNet"));
        assert_eq!(said(&v.window(t.from(99.0))), refused("this is for XenoNet's own window, and XenoNet couldn't see which program asked"));
        t.peer = None;
        assert_eq!(said(&v.window(&mut t)), refused("this is for XenoNet's own window, and XenoNet couldn't see which program asked"));
        // A verdict holds for a minute per process.
        t.chains[1][0] = proc(102.0, helper, None, None);
        assert_eq!(said(&v.window(t.from(102.0))), refused("this is for XenoNet's own window, and it came from a program run in XenoNet"));
        t.now += 60e3;
        assert_eq!(said(&v.window(t.from(102.0))), passed());
        // The Desktop going takes its process's standing with it.
        assert_eq!(said(&v.desktop(&mut d, false)), passed());
        assert_eq!(said(&v.window(t.from(101.0))), refused("this is for XenoNet's own window, and it came from outside XenoNet"));
        t.supported = false;
        assert_eq!(said(&v.window(&mut t)), passed());
    }

    #[test]
    fn the_tray_is_only_xenonet_trays_own_executable_in_an_applications_folder() {
        xenonet_platforms::install();
        const TRAY: &str = "/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray";
        const MINE: &str = "/Users/me/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray";
        let launchd = proc(1.0, "/sbin/launchd", None, None);
        let mut t = Table::new(vec![
            vec![proc(120.0, TRAY, None, None), launchd.clone()],
            vec![proc(121.0, MINE, None, None), launchd.clone()],
            // Run from where it was built, or a copy elsewhere.
            vec![proc(122.0, "/Users/me/src/xenonet/tray/macos/.build/XenoNet Tray.app/Contents/MacOS/XenoNet Tray", None, None), launchd.clone()],
            vec![proc(123.0, "/Users/me/Downloads/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray", None, None), launchd.clone()],
            // A helper inside the bundle, and another app's executable.
            vec![proc(124.0, "/Applications/XenoNet Tray.app/Contents/MacOS/tools/helper", None, None), launchd.clone()],
            vec![proc(125.0, XENONET, None, None), launchd.clone()],
            // The tray itself, started from under an agent or an actor's app.
            vec![proc(126.0, TRAY, None, None), proc(20.0, "/Users/me/.local/bin/claude", Some("claude"), None), launchd],
            vec![proc(127.0, TRAY, None, None), proc(7.0, GROK, None, Some(("grok-bot", "app")))],
            vec![proc(128.0, "/usr/bin/nc", None, None), proc(40.0, "/bin/zsh", None, None), proc(5.0, TERMINAL, None, None)],
        ]);
        let v = verifier(None);
        let apps = ["/Applications".to_string(), "/Users/me/Applications/".to_string()];
        assert_eq!(said(&v.tray(t.from(120.0), &apps)), passed());
        assert_eq!(said(&v.tray(t.from(121.0), &apps)), passed(), "in the home folder's Applications");
        let elsewhere = "only XenoNet Tray.app's own executable, in /Applications or your own Applications folder, can be XenoNet's tray";
        for pid in [122.0, 123.0, 124.0, 125.0, 128.0] {
            assert_eq!(said(&v.tray(t.from(pid), &apps)), refused(elsewhere), "{pid}");
        }
        assert_eq!(said(&v.tray(t.from(126.0), &apps)), refused("XenoNet's tray can't run under a Claude Code process"));
        assert_eq!(said(&v.tray(t.from(127.0), &apps)), refused("XenoNet's tray can't run under Grok Bot"));
        assert_eq!(said(&v.tray(t.from(99.0), &apps)), refused("XenoNet couldn't see which program connected as its tray"));
        assert_eq!(v.counts().unseen, 1.0);
        // The real path decides: what ps names, argv[0], is the process's own to set.
        t.chains[0][0] = Link { row: Row { pid: 120.0, ppid: 0.0, start: 1200.0, program: TRAY.into() }, exe: Some(None), agent: None, product: None };
        assert_eq!(said(&v.tray(t.from(120.0), &apps)), refused(elsewhere), "an executable whose path couldn't be read");
        assert_eq!(said(&v.tray(t.from(121.0), &[])), refused(elsewhere), "no Applications folder named");
        // Where XenoNet can't look, nothing passes.
        t.supported = false;
        assert_eq!(said(&v.tray(t.from(121.0), &apps)), refused("XenoNet can't see which program connects as its tray on this system"));
        assert_eq!(v.counts().unchecked, 0.0);
    }

    #[test]
    fn an_actors_token_works_only_from_its_own_processes() {
        xenonet_platforms::install();
        let script = proc(50.0, "/Users/me/bin/wren", None, None);
        let shell = proc(49.0, "/bin/zsh", None, None);
        let terminal = proc(5.0, TERMINAL, None, None);
        let grok_app = proc(7.0, GROK, None, Some(("grok-bot", "app")));
        let mut t = Table::new(vec![
            vec![script, shell, terminal.clone()],
            vec![proc(51.0, "/usr/bin/node", None, None), proc(20.0, "/Users/me/.local/bin/claude", Some("claude"), None)],
            vec![proc(52.0, "/Applications/Grok Bot.app/Contents/Resources/relay", None, None), grok_app.clone()],
            vec![grok_app],
            vec![
                proc(53.0, "/Applications/Muse.app/Contents/Resources/helper", None, Some(("muse", "host"))),
                proc(8.0, "/Applications/Muse.app/Contents/MacOS/Muse", None, Some(("muse", "app"))),
            ],
            vec![proc(54.0, "/opt/hermes/bin/agent", None, Some(("hermes-agent", "host"))), terminal.clone()],
            vec![proc(55.0, "/usr/bin/python3", None, None), terminal],
        ]);
        let v = verifier(None);
        let (own, grok, hermes) = (("wren", None), ("grok", Some("grok-bot")), ("hermes", Some("hermes-agent")));
        assert_eq!(said(&v.actor(&mut t, own.0, own.1, None)), refused("XenoNet couldn't see which program sent this, so it doesn't take the token from it"));
        let ok = v.actor(t.from(50.0), own.0, own.1, None);
        assert_eq!((ok.ok, ok.root.map(|r| r.0)), (true, Some(50.0)));
        let j = ok.json();
        assert_eq!(j["root"], json!({ "pid": 50, "start": 500 }));
        assert_eq!(j["chain"][0], json!({ "pid": 50, "ppid": 0, "start": 500, "program": "/Users/me/bin/wren", "exe": "/Users/me/bin/wren", "agent": null, "product": null }));
        assert_eq!(said(&v.actor(t.from(50.0), own.0, own.1, Some(49.0))), passed(), "its shell, below the terminal");
        assert_eq!(
            said(&v.actor(t.from(50.0), own.0, own.1, Some(5.0))),
            refused("the pid it names is an app's, which other programs run under too; name the actor's own process, or a parent of it below the app it runs in")
        );
        assert_eq!(said(&v.actor(t.from(50.0), own.0, own.1, Some(999.0))), refused("the pid it names has to be the calling process or one of its parents"));
        assert_eq!(said(&v.actor(t.from(51.0), own.0, own.1, None)), refused("an actor's token doesn't work from under a session, and this came from under a Claude Code process"));
        assert_eq!(said(&v.actor(t.from(7.0), grok.0, grok.1, None)), refused("Grok Bot's own processes can't use an actor's token: they could be relaying for its cloud"));
        assert_eq!(said(&v.actor(t.from(55.0), grok.0, grok.1, None)), refused("Grok Bot's actor works only from under Grok Bot's own processes"));
        assert_eq!(said(&v.actor(t.from(53.0), own.0, own.1, None)), refused("an actor of your own can't use its token from under Muse"));
        // A product its maker signs: only from under the app as signed.
        let unsigned = "Grok Bot's actor works only from under Grok Bot's app as Cursor signed it, and the app this came from isn't signed by Cursor";
        assert_eq!(said(&v.actor(t.from(52.0), grok.0, grok.1, None)), refused(unsigned));
        t.signed.push(7.0);
        assert_eq!(said(&v.actor(t.from(52.0), grok.0, grok.1, None)), refused(unsigned), "a verdict holds for a minute");
        t.now += 60e3;
        assert_eq!(said(&v.actor(t.from(52.0), grok.0, grok.1, None)), passed());
        // Hermes Agent's actor runs under one of its agent hosts.
        assert_eq!(said(&v.actor(t.from(54.0), hermes.0, hermes.1, None)), passed());
        assert_eq!(said(&v.actor(t.from(55.0), hermes.0, hermes.1, None)), refused("Hermes Agent's actor works only from under Hermes Agent's own processes"));
        t.supported = false;
        assert_eq!(said(&v.actor(&mut t, own.0, own.1, None)), passed());
    }

    #[test]
    fn registering_an_actor_is_held_to_the_same_and_shows_who_asked() {
        xenonet_platforms::install();
        let mut t = Table::new(vec![
            vec![proc(50.0, "/Users/me/bin/wren", None, None), proc(5.0, TERMINAL, None, None)],
            vec![proc(51.0, "/usr/bin/node", None, None), proc(23.0, "/usr/local/bin/codex", Some("codex"), None)],
            vec![proc(52.0, "/usr/bin/python3", None, None), proc(7.0, GROK, None, Some(("grok-bot", "app")))],
        ]);
        t.cwd = Some("/Users/me/wren");
        let v = verifier(None);
        let r = v.register(t.from(50.0), None, None);
        assert_eq!((r.ok, r.cwd.clone(), r.chain.as_ref().map(Vec::len)), (true, Some(Some("/Users/me/wren".into())), Some(2)));
        assert_eq!(r.json()["cwd"], json!("/Users/me/wren"));
        assert_eq!(said(&v.register(t.from(51.0), None, None)), refused("a session can't register an actor, only ask you to, and this came from under a Codex process"));
        assert_eq!(said(&v.register(t.from(52.0), None, None)), refused("a program under Grok Bot can't register an actor of its own"));
        assert_eq!(
            said(&v.register(t.from(52.0), Some("grok-bot"), None)),
            refused("Grok Bot's actor registers only from under Grok Bot's app as Cursor signed it, and the app this came from isn't signed by Cursor")
        );
        assert_eq!(said(&v.register(t.from(99.0), None, None)), refused("XenoNet couldn't see which program asked to register, so it can't show you who asked"));
        // Where it can look, XenoNet says which process is on the other end.
        assert_eq!((v.peer(t.from(50.0)), v.lineage(&mut t, Some(50.0))), (Some(50.0), Some(vec![50.0, 5.0])));
        assert_eq!(v.lineage(&mut t, Some(99.0)), Some(vec![]));
        t.supported = false;
        assert_eq!((v.peer(&mut t), v.lineage(&mut t, Some(50.0))), (None, None));
        assert_eq!(said(&v.register(&mut t, None, None)), passed());
    }
}
