//! Whether a request made in an agent's name comes from that agent:
//! core/engine/verify.js. Any program on this machine can reach XenoNet's
//! port and say it's any agent; the process on the other end of the
//! connection can't lie about where it runs (procs). So:
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
//! The pins are the engine store's own. procs and the clock are called as
//! the JS called them, through `Calls`: "procs", the twin's "peerOf" (the
//! process on the other end of this request's socket) and "now".

use crate::peer::{list, Calls, Result};
use crate::sql::Arg;
use crate::store::{js_text, Shared};
use std::collections::HashMap;
use std::sync::Mutex;

/// The apps whose sessions XenoNet checks, by their id in a session's name.
fn app_name(app: &str) -> Option<&'static str> {
    match app {
        "claude" => Some("Claude Code"),
        "codex" => Some("Codex"),
        "opencode" => Some("OpenCode"),
        _ => None,
    }
}

/// `APP[app]`, as JS reads a property of an object literal: its own, or
/// what Object.prototype has under that name, as String() writes it.
fn app_lookup(app: &str) -> Option<String> {
    app_name(app).map(str::to_string).or_else(|| crate::js::proto_member(app))
}

fn a_app(app: &str) -> String {
    format!("{} {}", if app == "opencode" { "an" } else { "a" }, app_lookup(app).unwrap_or_else(|| "undefined".into()))
}

fn product_name(product: &str) -> Option<&'static str> {
    match product {
        "hermes-agent" => Some("Hermes Agent"),
        "grok-bot" => Some("Grok Bot"),
        "muse" => Some("Muse"),
        "dots" => Some("the ChatGPT app"),
        _ => None,
    }
}

/// `PRODUCT[x]` in a sentence: "undefined" for one it hasn't.
fn product_word(x: &Arg) -> String {
    let key = js_text(x);
    product_name(&key).map_or_else(|| crate::js::proto_member(&key).unwrap_or_else(|| "undefined".into()), str::to_string)
}

/// The products whose apps their makers sign, by the maker's name.
fn signed_by(product: &str) -> Option<&'static str> {
    match product {
        "grok-bot" => Some("Cursor"),
        "muse" => Some("Meta"),
        _ => None,
    }
}

/// A verdict on a window request is kept this long per process.
const WINDOW_VERDICT_MS: f64 = 60e3;
/// And one on an actor's, per actor, process and the pid it names.
const ACTOR_VERDICT_MS: f64 = 60e3;

/// `a === b`.
fn same(a: &Arg, b: &Arg) -> bool {
    match (a, b) {
        (Arg::Num(x), Arg::Num(y)) => x == y,
        (Arg::Big(x), Arg::Big(y)) => x == y,
        (Arg::Text(x), Arg::Text(y)) => x == y,
        (Arg::Bool(x), Arg::Bool(y)) => x == y,
        (Arg::Undef, Arg::Undef) | (Arg::Null, Arg::Null) => true,
        _ => false,
    }
}

fn product_of(p: &Arg) -> Arg {
    p.field("product")
}

/// Whether a chain of processes runs under a product: under one of
/// Hermes's agent hosts, or for the others under their app.
fn under(chain: &[Arg], product: &Arg) -> bool {
    let role = if product.as_str() == Some("hermes-agent") { "host" } else { "app" };
    chain.iter().any(|p| {
        let pr = product_of(p);
        same(&pr.field("product"), product) && pr.field("role").as_str() == Some(role)
    })
}

/// Whether a process is an app's (a terminal, an editor, an actor app),
/// which other programs run under too. A product's agent host can live
/// inside its app, and is the product's, not the app's.
fn is_app(p: &Arg) -> bool {
    let program = p.field("program");
    let program = if program.is_nullish() { String::new() } else { js_text(&program) };
    program.contains(".app/") && product_of(p).field("role").as_str() != Some("host")
}

/// The processes an actor's request may call its own, nearest first: the
/// caller, and its parents below the first app above it.
pub fn own_of(chain: &[Arg]) -> &[Arg] {
    match chain.iter().enumerate().position(|(i, p)| i > 0 && is_app(p)) {
        Some(app) => &chain[..app],
        None => chain,
    }
}

/// Why an actor's request (or a request to register one, `registering`)
/// can't come from this chain, nearest process first, or None when it can.
pub fn actor_refusal(chain: &[Arg], product: &Arg, pid: &Arg, registering: bool) -> Option<String> {
    if let Some(agent) = chain.iter().find(|p| p.field("agent").truthy()) {
        let a = a_app(&js_text(&agent.field("agent")));
        return Some(if registering {
            format!("a session can't register an actor, only ask you to, and this came from under {a} process")
        } else {
            format!("an actor's token doesn't work from under a session, and this came from under {a} process")
        });
    }
    let own = chain.first().map(product_of).unwrap_or(Arg::Undef);
    if own.field("role").as_str() == Some("app") {
        let what = if registering { "register an actor" } else { "use an actor's token" };
        return Some(format!("{}'s own processes can't {what}: they could be relaying for its cloud", product_word(&own.field("product"))));
    }
    let key = if product.is_nullish() { String::new() } else { js_text(product) };
    if let Some(name) = product_name(&key) {
        if !under(chain, product) {
            let verb = if registering { "registers" } else { "works" };
            return Some(format!("{name}'s actor {verb} only from under {name}'s own processes"));
        }
    } else if let Some(inside) = chain.iter().find(|p| product_of(p).truthy()) {
        let name = product_word(&product_of(inside).field("product"));
        return Some(if registering { format!("a program under {name} can't register an actor of its own") } else { format!("an actor of your own can't use its token from under {name}") });
    }
    if !pid.is_nullish() && !own_of(chain).iter().any(|p| same(&p.field("pid"), pid)) {
        return Some(if chain.iter().any(|p| same(&p.field("pid"), pid)) {
            "the pid it names is an app's, which other programs run under too; name the actor's own process, or a parent of it below the app it runs in".into()
        } else {
            "the pid it names has to be the calling process or one of its parents".into()
        });
    }
    None
}

/// The app bundle a program lies in (/Applications/XenoNet.app/), or its
/// folder when it isn't in one.
pub fn bundle_of(program: &Arg) -> String {
    let p = if program.is_nullish() { String::new() } else { js_text(program) };
    match p.find(".app/") {
        Some(i) => p[..i + ".app/".len()].to_string(),
        None => p[..p.rfind('/').map_or(0, |i| i + 1)].to_string(),
    }
}

/// The file a process from procs.chain runs: its executable's real path
/// when the chain was asked for those, and otherwise the program ps names.
fn file_of(p: &Arg) -> Arg {
    match p.field("exe") {
        Arg::Undef => p.field("program"),
        exe => exe,
    }
}

/// What the verifier has counted, for the doctor.
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub checked: f64,
    pub refused: f64,
    pub unchecked: f64,
    pub unseen: f64,
    /// { agent, reason, at }.
    pub last_refusal: Option<Arg>,
}

struct Inner {
    counts: Counts,
    verdicts: HashMap<String, (f64, Arg)>,
    actor_verdicts: HashMap<String, (f64, Arg)>,
    desktops: u64,
    home: Arg,
}

pub struct Verifier {
    store: Shared,
    inner: Mutex<Inner>,
}

fn obj(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A Map key for a value, as SameValueZero tells them apart.
fn key_of(v: &Arg) -> String {
    match v {
        Arg::Num(n) if *n == 0.0 => "n:0".into(),
        Arg::Num(n) => format!("n:{n}"),
        Arg::Text(s) => format!("s:{s}"),
        other => format!("o:{}", js_text(other)),
    }
}

impl Verifier {
    pub fn new(store: Shared, home: Arg) -> Self {
        Verifier { store, inner: Mutex::new(Inner { counts: Counts::default(), verdicts: HashMap::new(), actor_verdicts: HashMap::new(), desktops: 0, home }) }
    }

    pub fn counts(&self) -> Counts {
        self.inner.lock().unwrap().counts.clone()
    }

    fn now(&self, c: &mut dyn Calls) -> Result<f64> {
        let t = c.call("", "now", vec![])?;
        Ok(match t {
            Arg::Num(n) => n,
            Arg::Null => 0.0,
            Arg::Bool(b) => f64::from(u8::from(b)),
            Arg::Text(s) => xenonet_base::text::js_number(Some(&serde_json::Value::String(s))),
            _ => f64::NAN,
        })
    }

    fn supported(&self, c: &mut dyn Calls) -> Result<bool> {
        Ok(c.get("procs.supported")?.truthy())
    }

    fn pass(&self) -> Arg {
        self.inner.lock().unwrap().counts.unchecked += 1.0;
        obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(false))])
    }

    fn refuse(&self, c: &mut dyn Calls, agent: Arg, reason: &str) -> Result<Arg> {
        self.inner.lock().unwrap().counts.refused += 1.0;
        let at = self.now(c)?;
        self.inner.lock().unwrap().counts.last_refusal = Some(obj(vec![("agent", agent), ("reason", Arg::text(reason)), ("at", Arg::num(at))]));
        Ok(obj(vec![("ok", Arg::Bool(false)), ("checked", Arg::Bool(true)), ("reason", Arg::text(reason))]))
    }

    /// A request whose process XenoNet couldn't see, where it can look:
    /// never kept as a verdict.
    fn unseen(&self, c: &mut dyn Calls, agent: Arg, reason: &str) -> Result<Arg> {
        self.inner.lock().unwrap().counts.unseen += 1.0;
        let mut v = self.refuse(c, agent, reason)?;
        if let Arg::Obj(f) = &mut v {
            f.push(("unseen".into(), Arg::Bool(true)));
        }
        Ok(v)
    }

    fn checked(&self) {
        self.inner.lock().unwrap().counts.checked += 1.0;
    }

    fn home(&self) -> Arg {
        self.inner.lock().unwrap().home.clone()
    }

    /// The process on the other end of this request's socket, or null.
    fn peer_pid(&self, c: &mut dyn Calls) -> Arg {
        c.call("", "peerOf", vec![]).unwrap_or(Arg::Null)
    }

    fn chain(&self, c: &mut dyn Calls, pid: &Arg, opts: Arg) -> Vec<Arg> {
        list(c.call("procs", "chain", vec![pid.clone(), opts]).unwrap_or(Arg::List(vec![])))
    }

    /// The process on the other end of the socket and the ones above it.
    fn see(&self, c: &mut dyn Calls, exe: bool) -> (Arg, Vec<Arg>) {
        let pid = self.peer_pid(c);
        if !pid.truthy() {
            return (Arg::Null, Vec::new());
        }
        let chain = self.chain(c, &pid, obj(vec![("exe", Arg::Bool(exe))]));
        (pid, chain)
    }

    /// `hook`: the request is one of the app's hooks, which for Claude Code
    /// may come from Claude Code itself. { ok, checked, root?, reason? }.
    pub fn check(&self, c: &mut dyn Calls, agent: &Arg, hook: bool) -> Result<Arg> {
        let name = js_text(agent);
        let app = name.split(':').next().unwrap_or("").to_string();
        if app_lookup(&app).is_none() || !self.supported(c)? {
            return Ok(self.pass());
        }
        let (pid, chain) = self.see(c, false);
        if !pid.truthy() {
            return Ok(self.pass());
        }
        if chain.is_empty() {
            return self.unseen(c, agent.clone(), &format!("requests in {name}'s name have to come from {} process, and this one had gone before XenoNet could look", a_app(&app)));
        }
        self.checked();
        let Some(holder) = chain.iter().find(|p| p.field("agent").as_str() == Some(app.as_str())) else {
            return self.refuse(c, agent.clone(), &format!("requests in {name}'s name have to come from {} process, and this one doesn't", a_app(&app)));
        };
        let root = obj(vec![("pid", holder.field("pid")), ("start", holder.field("start"))]);
        let ok = |root: &Arg, sure: bool| {
            let mut f = vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true)), ("root", root.clone())];
            if sure {
                f.push(("sure", Arg::Bool(true)));
            }
            obj(f)
        };
        // Any OpenCode may have the session open.
        if app == "opencode" {
            return Ok(ok(&root, false));
        }
        let is_root = |p: &Arg| same(&p.field("pid"), &root.field("pid")) && same(&p.field("start"), &root.field("start"));
        // The pins of processes still running; the others go.
        let mut held: Vec<Arg> = Vec::new();
        for p in self.store.with(|s| s.pins(agent.clone()))? {
            let p = row_arg(&p);
            if c.call("procs", "alive", vec![p.clone()])?.truthy() {
                held.push(p);
            } else {
                self.store.with(|s| s.unpin(agent.clone(), &p))?;
            }
        }
        // Claude Code itself, in one of its HTTP hooks: this process holds
        // the session.
        if hook && app == "claude" && same(&chain[0].field("pid"), &holder.field("pid")) && c.call("procs", "signed", vec![holder.clone()])?.truthy() {
            let t = self.now(c)?;
            self.store.with(|s| s.pin(agent.clone(), &root, Arg::Bool(true), Arg::num(t)))?;
            self.store.with(|s| s.drop_guesses(agent.clone()))?;
            return Ok(ok(&root, true));
        }
        if let Some(h) = held.iter().find(|p| is_root(p)) {
            let sure = h.field("sure");
            let t = self.now(c)?;
            self.store.with(|s| s.pin(agent.clone(), &root, sure, Arg::num(t)))?;
            return Ok(ok(&root, false));
        }
        if !held.is_empty() {
            let word = app_lookup(&app).unwrap_or_else(|| "undefined".into());
            return self.refuse(c, agent.clone(), &format!("{name} belongs to another {word} process"));
        }
        let t = self.now(c)?;
        self.store.with(|s| s.pin(agent.clone(), &root, Arg::Bool(false), Arg::num(t)))?;
        Ok(ok(&root, false))
    }

    /// Requests only you make: not from any agent, nor from under an actor
    /// product's processes. The nearer of one of those and XenoNet's own
    /// app decides.
    pub fn person(&self, c: &mut dyn Calls) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let (_, chain) = self.see(c, false);
        if chain.is_empty() {
            return self.unseen(c, Arg::Null, "this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet");
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().find(|p| p.field("agent").truthy() || product_of(p).truthy() || same(&p.field("pid"), &home));
        if let Some(near) = near {
            if near.field("agent").truthy() {
                return self.refuse(c, Arg::Null, &format!("this is yours to do, and it came from under {} process; run it in your own terminal or in XenoNet", a_app(&js_text(&near.field("agent")))));
            }
            if product_of(near).truthy() {
                return self.refuse(c, Arg::Null, &format!("this is yours to do, and it came from under {}; run it in your own terminal or in XenoNet", product_word(&product_of(near).field("product"))));
            }
        }
        Ok(obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true))]))
    }

    /// Why a call under a product whose maker signs its app isn't that
    /// app's, or None.
    fn unsigned(&self, c: &mut dyn Calls, chain: &[Arg], product: &Arg, registering: bool) -> Result<Option<String>> {
        let key = if product.is_nullish() { String::new() } else { js_text(product) };
        let Some(maker) = signed_by(&key) else {
            return Ok(None);
        };
        let app = chain.iter().find(|p| {
            let pr = product_of(p);
            same(&pr.field("product"), product) && pr.field("role").as_str() == Some("app")
        });
        if let Some(app) = app {
            if c.call("procs", "signed", vec![app.clone()]).unwrap_or(Arg::Bool(false)).truthy() {
                return Ok(None);
            }
        }
        let name = product_name(&key).unwrap_or("undefined");
        let verb = if registering { "registers" } else { "works" };
        Ok(Some(format!("{name}'s actor {verb} only from under {name}'s app as {maker} signed it, and the app this came from isn't signed by {maker}")))
    }

    /// A request with an actor's token (`who`: its slug and product id),
    /// naming `pid` as its process or not.
    pub fn actor(&self, c: &mut dyn Calls, who: &Arg, pid: &Arg) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let slug = who.field("slug");
        let id = if slug.truthy() { Arg::text(format!("actor:{}", js_text(&slug))) } else { Arg::Null };
        let blind = "XenoNet couldn't see which program sent this, so it doesn't take the token from it";
        let peer = self.peer_pid(c);
        if !peer.truthy() {
            return self.unseen(c, id, blind);
        }
        let part = |a: &Arg| if a.is_nullish() { String::new() } else { js_text(a) };
        let key = format!("{}|{}|{}|{}", part(&slug), part(&who.field("product")), js_text(&peer), part(pid));
        let kept = self.inner.lock().unwrap().actor_verdicts.get(&key).cloned();
        if let Some((at, verdict)) = kept {
            if self.now(c)? - at < ACTOR_VERDICT_MS {
                return Ok(verdict);
            }
        }
        let chain = self.chain(c, &peer, obj(vec![("exe", Arg::Bool(true))]));
        if chain.is_empty() {
            return self.unseen(c, id, blind);
        }
        self.checked();
        let product = who.field("product").or(Arg::Null);
        let why = match actor_refusal(&chain, &product, pid, false) {
            Some(w) => Some(w),
            None => self.unsigned(c, &chain, &who.field("product"), false)?,
        };
        let verdict = match why {
            Some(why) => self.refuse(c, id, &why)?,
            None => obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true)), ("root", obj(vec![("pid", chain[0].field("pid")), ("start", chain[0].field("start"))])), ("chain", Arg::List(chain))]),
        };
        let at = self.now(c)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.actor_verdicts.len() > 256 {
            inner.actor_verdicts.clear();
        }
        inner.actor_verdicts.insert(key, (at, verdict.clone()));
        Ok(verdict)
    }

    fn cwd(&self, c: &mut dyn Calls, peer: &Arg) -> Result<Arg> {
        if c.kind("procs.cwd")? != "function" {
            return Ok(Arg::Null);
        }
        Ok(c.call("procs", "cwd", vec![peer.clone()]).unwrap_or(Arg::Null).or(Arg::Null))
    }

    /// A request to register an actor, asking to be `product` (or an actor
    /// of your own), naming `pid` or not.
    pub fn register(&self, c: &mut dyn Calls, product: &Arg, pid: &Arg) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let (peer, chain) = self.see(c, true);
        if chain.is_empty() {
            return self.unseen(c, Arg::Null, "XenoNet couldn't see which program asked to register, so it can't show you who asked");
        }
        self.checked();
        let why = match actor_refusal(&chain, product, pid, true) {
            Some(w) => Some(w),
            None => self.unsigned(c, &chain, product, true)?,
        };
        if let Some(why) = why {
            return self.refuse(c, Arg::Null, &why);
        }
        let cwd = self.cwd(c, &peer)?;
        Ok(obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true)), ("chain", Arg::List(chain)), ("cwd", cwd)]))
    }

    /// A request that asks you for something and holds no secret: from a
    /// session or your own terminal, but never from under an actor
    /// product's processes.
    pub fn asker(&self, c: &mut dyn Calls) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let (peer, chain) = self.see(c, true);
        if chain.is_empty() {
            return self.unseen(c, Arg::Null, "XenoNet couldn't see which program asked, so it can't show you who asked");
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().find(|p| product_of(p).truthy() || same(&p.field("pid"), &home));
        if let Some(near) = near.filter(|n| product_of(n).truthy()) {
            return self.refuse(c, Arg::Null, &format!("a program under {} can't ask for a gateway token: it could be relaying for its cloud", product_word(&product_of(near).field("product"))));
        }
        let cwd = self.cwd(c, &peer)?;
        Ok(obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true)), ("chain", Arg::List(chain)), ("cwd", cwd)]))
    }

    /// The process on the other end of a socket, or null where XenoNet
    /// can't look.
    pub fn peer(&self, c: &mut dyn Calls) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(Arg::Null);
        }
        Ok(self.peer_pid(c))
    }

    /// A running process and the ones above it, nearest first, as pids;
    /// null where XenoNet can't look, and [] for a process that's gone.
    pub fn lineage(&self, c: &mut dyn Calls, pid: &Arg) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(Arg::Null);
        }
        Ok(Arg::List(self.chain(c, pid, obj(vec![("again", Arg::Bool(false))])).iter().map(|p| p.field("pid")).collect()))
    }

    /// Requests only XenoNet's own window makes: from a process of
    /// XenoNet's app, with no agent nearer.
    pub fn window(&self, c: &mut dyn Calls) -> Result<Arg> {
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let blind = "this is for XenoNet's own window, and XenoNet couldn't see which program asked";
        let pid = self.peer_pid(c);
        if !pid.truthy() {
            return self.unseen(c, Arg::Null, blind);
        }
        let key = key_of(&pid);
        let kept = self.inner.lock().unwrap().verdicts.get(&key).cloned();
        if let Some((at, verdict)) = kept {
            if self.now(c)? - at < WINDOW_VERDICT_MS {
                return Ok(verdict);
            }
        }
        let chain = self.chain(c, &pid, obj(vec![("exe", Arg::Bool(true))]));
        if chain.is_empty() {
            return self.unseen(c, Arg::Null, blind);
        }
        self.checked();
        let home = self.home();
        let near = chain.iter().position(|p| p.field("agent").truthy() || (!home.is_nullish() && same(&p.field("pid"), &home)));
        let verdict = match near {
            Some(i) if chain[i].field("agent").truthy() => self.refuse(c, Arg::Null, &format!("this is for XenoNet's own window, and it came from under {} process", a_app(&js_text(&chain[i].field("agent")))))?,
            None => self.refuse(c, Arg::Null, "this is for XenoNet's own window, and it came from outside XenoNet")?,
            Some(i) => {
                // Every process from the caller up to XenoNet's app is the
                // app's own, by the executable's real path.
                let near_file = file_of(&chain[i]);
                let bundle = bundle_of(&if near_file.is_nullish() { chain[i].field("program") } else { near_file });
                let foreign = chain[..i].iter().any(|p| match file_of(p) {
                    Arg::Text(f) => !f.starts_with(&bundle),
                    _ => true,
                });
                if foreign {
                    self.refuse(c, Arg::Null, "this is for XenoNet's own window, and it came from a program run in XenoNet")?
                } else {
                    obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true))])
                }
            }
        };
        let at = self.now(c)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.verdicts.len() > 256 {
            inner.verdicts.clear();
        }
        inner.verdicts.insert(key, (at, verdict.clone()));
        Ok(verdict)
    }

    /// XenoNet's Desktop, where the server is a service no app started:
    /// the process on the other end of the Desktop's connection becomes
    /// home. Only an app's own executable, by its real path, that isn't an
    /// agent or an actor product's. `connected` false: the Desktop went.
    pub fn desktop(&self, c: &mut dyn Calls, connected: bool) -> Result<Arg> {
        let mine = {
            let mut i = self.inner.lock().unwrap();
            i.desktops += 1;
            i.home = Arg::Null;
            i.verdicts.clear();
            i.desktops
        };
        if !connected {
            return Ok(obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(false))]));
        }
        if !self.supported(c)? {
            return Ok(self.pass());
        }
        let (pid, chain) = self.see(c, true);
        if !pid.truthy() || chain.is_empty() {
            return self.unseen(c, Arg::Null, "XenoNet couldn't see which program connected as its Desktop");
        }
        self.checked();
        let p = &chain[0];
        let file = file_of(p);
        let file = if file.is_nullish() { String::new() } else { js_text(&file) };
        let verdict = if p.field("agent").truthy() {
            self.refuse(c, Arg::Null, &format!("{} process can't be XenoNet's Desktop", a_app(&js_text(&p.field("agent")))))?
        } else if product_of(p).truthy() {
            self.refuse(c, Arg::Null, &format!("{}'s processes can't be XenoNet's Desktop", product_word(&product_of(p).field("product"))))?
        } else if !crate::re!(r"\.app/Contents/MacOS/[^/]+$").is_match(&file) {
            self.refuse(c, Arg::Null, "only an app's own executable can be XenoNet's Desktop")?
        } else {
            obj(vec![("ok", Arg::Bool(true)), ("checked", Arg::Bool(true))])
        };
        if verdict.field("ok").truthy() {
            let mut i = self.inner.lock().unwrap();
            if i.desktops == mine {
                i.home = p.field("pid");
            }
        }
        Ok(verdict)
    }
}

/// A row as JS reads it: whole numbers as numbers.
fn row_arg(r: &crate::sql::Row) -> Arg {
    Arg::Obj(
        r.0.iter()
            .map(|(k, c)| {
                let v = match c {
                    crate::sql::Cell::Int(i) => Arg::Num(*i as f64),
                    other => other.arg(),
                };
                (k.clone(), v)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: f64, program: &str, agent: Option<&str>, product: Option<(&str, &str)>) -> Arg {
        let product = product.map_or(Arg::Null, |(id, role)| obj(vec![("product", Arg::text(id)), ("role", Arg::text(role))]));
        obj(vec![("pid", Arg::num(pid)), ("program", Arg::text(program)), ("agent", agent.map_or(Arg::Null, Arg::text)), ("product", product)])
    }

    #[test]
    fn an_actor_is_held_to_its_own_processes() {
        let chain = vec![p(10.0, "/bin/zsh", None, None), p(5.0, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", None, None)];
        assert_eq!(own_of(&chain).len(), 1);
        assert_eq!(actor_refusal(&chain, &Arg::Null, &Arg::Null, false), None);
        assert!(actor_refusal(&chain, &Arg::Null, &Arg::num(5.0), false).unwrap().contains("an app's"));
        assert!(actor_refusal(&chain, &Arg::text("grok-bot"), &Arg::Null, true).unwrap().starts_with("Grok Bot's actor registers"));
        let agent = vec![p(10.0, "/x", Some("claude"), None)];
        assert!(actor_refusal(&agent, &Arg::Null, &Arg::Null, false).unwrap().ends_with("a Claude Code process"));
    }

    #[test]
    fn bundles_are_the_app_or_the_folder() {
        assert_eq!(bundle_of(&Arg::text("/Applications/XenoNet.app/Contents/MacOS/XenoNet")), "/Applications/XenoNet.app/");
        assert_eq!(bundle_of(&Arg::text("/usr/local/bin/node")), "/usr/local/bin/");
        assert_eq!(bundle_of(&Arg::Null), "");
    }

    #[test]
    fn an_apps_name_is_read_as_an_object_literal_reads_it() {
        assert_eq!(app_lookup("toString").as_deref(), Some("function toString() { [native code] }"));
        assert_eq!(a_app("opencode"), "an OpenCode");
        assert_eq!(app_lookup("nobody"), None);
    }
}
