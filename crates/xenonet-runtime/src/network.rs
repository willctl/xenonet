//! The engine across machines (docs/design/network.md, sections 5, 6, 8,
//! 10 and 11): what this host tells the others through the seam
//! (`xenonet_model::ops`), and what it does with what they tell it.
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
//!   sent, with its id across the network (its envelope's mid), and goes
//!   as an entry of this machine's feed, again each time the link comes
//!   back, until a stored receipt comes back. One from another host is
//!   stored with its sender named by its agt_ and an envelope
//!   that says how it came, then delivered as local mail. Receipts go
//!   back: stored at once, then delivered, seen or held. One stored there
//!   that no one read in a week is unread here.
//! - **Feeds.** What another member sends is an entry of its feed
//!   (xenonet-log's `feed`), which the network checked against the member's
//!   key: offered to the author's feed in the store and applied in the
//!   transaction that writes what it does, so each is applied exactly once
//!   (`entry_in`). Kept entries (handoffs) apply in their author's order,
//!   held behind a gap until it fills; this machine acks what it applied
//!   and asks for what's missing. A fork freezes the author's feed until
//!   the person unfreezes it at the window.
//! - **Claims across clones.** Claims and observed edits go out as a repo
//!   key and a path inside the repo; others' come back for the edit
//!   warning.
//! - **Handoffs.** Work left for another session, kept by the host that
//!   made it, which decides who takes it.
//! - **Intents.** The person's intents, their assigns and the reports on
//!   them are kept entries too, applied by `crate::intents`.
//!
//! Every act from another machine is decided here before it's taken
//! (`crate::ground`, xenonet-authz): a message, a start, an ask, a
//! handoff, claims, and the person's answer. The person's acts carry their
//! device's signature (`network/message/v3`'s `authority`), checked
//! against this machine's copy of their chain; an agent's is its
//! machine's word, which checked its process.
//!
//! Every path here runs only while `Net::me()` says this machine is in a
//! network, except handoffs, which work on one machine too.

use crate::engine::{event, Side};
use crate::runtime::Runtime;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use xenonet_adapter::apps::is_session_id;
use xenonet_api::From;
use xenonet_authz::Information;
use xenonet_engine::delivery::policy::Guard;
use xenonet_engine::engine::{is_actor_agent, is_agent_id, Engine, Fail, Result, ACTIVE_MS};
use xenonet_engine::host::Collaborators;
use xenonet_engine::network::{is_remote, new_id};
use xenonet_model::ids::{parse_actor_id, AgentId, YOU};
use xenonet_model::ops::{Host, Incoming, Kind, Net, Outgoing, Sink, To};
use xenonet_net::feed::{Feed, Kind as EntryKind, Offer};
use xenonet_net::sink::{Arrived, Feeds};
use xenonet_os::sync::Locked;
use xenonet_store::engine::EngineStore;
use xenonet_store::sql::Row;
use xenonet_text::time::now_ms;

/// How often presence and claims go out when they changed.
pub const PRESENCE_MS: f64 = 2000.0;
/// How long a message is sent again until its receipt comes: a week.
const OUTBOX_MS: f64 = 7.0 * 24.0 * 3600e3;
/// A handoff no one takes lapses after a week.
const HANDOFF_MS: f64 = 7.0 * 24.0 * 3600e3;
/// An edit counts as the agent's for half an hour, as the engine says.
const EDIT_HOLD_MS: f64 = 30.0 * 60e3;
/// Messages in one thread, across hosts.
const MAX_HOPS: f64 = 6.0;
/// What another host may keep here: its claims, and handoffs open.
const MAX_CLAIMS: f64 = 5000.0;
/// The longest a claim from another host holds here.
const CLAIM_MAX_MS: f64 = 8.0 * 3600e3;
/// The most files presence says an agent is changing.
const EDITS_SENT: usize = 50;
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

/// What this host last told the others: its presence (each agent as sent),
/// and the timer that tells it again.
#[derive(Default)]
pub struct NetState {
    out: Mutex<Out>,
    ticker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Held while a body from another host is taken, one at a time, so a
    /// repeat that arrives beside its first copy is still seen as one.
    taking: Mutex<()>,
    /// The network's governance: its chains and the person here, which
    /// every act from another machine is decided on.
    pub(crate) governance: std::sync::RwLock<Option<Arc<dyn xenonet_net::ground::Governance>>>,
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
    /// Who may start and wake sessions here, as the last presence said.
    reach: Value,
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
pub fn set_net(w: &Runtime, net: Arc<dyn Net>) {
    w.engine.set_net(net);
}

/// Puts the network's link behind the engine and starts telling the other
/// hosts of this one (presence and claims every 2 s when they changed,
/// handoffs that lapse). The sink, which the link calls with what arrives,
/// and the feeds, which take other members' entries, are plugged into the
/// link's slot.
pub fn plug(w: &Arc<Runtime>, link: &dyn xenonet_api::cores::NetLink) -> Arc<EngineSink> {
    set_net(w, link.net());
    crate::ground::set_governance(w, link.governance());
    let t = w.every(PRESENCE_MS, tick);
    if let Some(old) = w.net.ticker.locked().replace(t) {
        old.abort();
    }
    let sink = EngineSink::new(w);
    link.sinks().set(sink.clone());
    link.sinks().set_feeds(sink.clone());
    sink
}

/// A call from the link, for the sink's own thread.
type Job = Box<dyn FnOnce(&Runtime) + Send + 'static>;

/// What the network calls in the engine. The calls run one at a time, in
/// the order the link made them, on a thread of the sink's own: the engine
/// reads git and its database, and two messages from one host must reach
/// its sessions in the order they were sent, which separate blocking
/// threads don't keep.
pub struct EngineSink {
    w: Weak<Runtime>,
    jobs: Option<Mutex<std::sync::mpsc::Sender<Job>>>,
}

impl EngineSink {
    pub fn new(w: &Arc<Runtime>) -> Arc<EngineSink> {
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

    fn run(&self, f: impl FnOnce(&Runtime) + Send + 'static) {
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

// What arrived in a network is decided against that network's chain and
// answered into it (`xenonet_net::ground::within`), never another's.
impl Sink for EngineSink {
    fn receive(&self, incoming: Incoming) {
        self.run(move |w| {
            let network = incoming.network.clone();
            xenonet_net::ground::within(&network, || receive(w, incoming));
        });
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

impl Feeds for EngineSink {
    fn entry(&self, arrived: Arrived) {
        self.run(move |w| {
            let network = arrived.entry.group.clone();
            xenonet_net::ground::within(&network, || entry_in(w, arrived));
        });
    }
    // On the caller's thread: the window waits for it, and the store's
    // transaction keeps it apart from what the sink's thread applies.
    fn unfreeze(&self, author: &str, network: &str) -> std::result::Result<usize, String> {
        let w = self.w.upgrade().ok_or("XenoNet is stopping")?;
        xenonet_net::ground::within(network, || unfreeze(&w, author, network))
    }
}

pub(crate) fn net(w: &Runtime) -> Arc<dyn Net> {
    w.engine.net.get()
}

pub(crate) fn send(w: &Runtime, to: To, kind: Kind, body: Value) {
    send_with(w, to, kind, body, None);
}

/// A body with the authority its act rests on, which goes beside its feed
/// entry for its one recipient.
pub(crate) fn send_with(w: &Runtime, to: To, kind: Kind, body: Value, authority: Option<Value>) {
    if let Err(e) = net(w).send(Outgoing { network: None, to, kind, body, authority }) {
        w.log("warn", "net.send", &format!("a {} for the network wasn't queued: {e}", kind.as_str()));
    }
}

fn store<T>(w: &Runtime, f: impl FnOnce(&EngineStore) -> xenonet_store::sql::Result<T>) -> Result<T> {
    w.store.with(|s| f(s)).map_err(|e| Fail::from(crate::store::thrown(e)))
}

/// The engine with its collaborators over this world, for one call.
fn engine<T>(w: &Runtime, f: impl FnOnce(&Engine, &Collaborators<'_>) -> Result<T>) -> Result<T> {
    let side = Side::new(w, None);
    f(&w.engine, &side.collaborators())
}

fn fail(m: impl Into<String>) -> Fail {
    Fail::Error(m.into())
}

/// A failure's words, for a log line or a receipt.
fn why(f: &Fail) -> String {
    match f {
        Fail::Error(m) | Fail::Bad(m) => m.clone(),
        Fail::Refused { why, .. } => why.clone(),
        Fail::Thrown(t) => t.message.clone(),
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn n(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

/// A number as the API writes it: an integer when it is one.
fn number(x: f64) -> Value {
    xenonet_text::json::number(x)
}

/// A field as a number: NaN for one that isn't there.
fn num_of(v: &Value, k: &str) -> f64 {
    v.get(k).map_or(f64::NAN, |x| xenonet_text::text::to_number(Some(x)))
}

fn items(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// An object with these fields, in this order.
fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Map<String, Value>>())
}

fn push(a: &mut Value, k: &str, v: Value) {
    if let Value::Object(f) = a {
        f.insert(k.to_string(), v);
    }
}

/// At most `max` characters, as a schema's limit asks.
pub(crate) fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn opt(v: Option<String>, max: usize) -> Value {
    v.map_or(Value::Null, |s| Value::String(clip(&s, max)))
}

pub(crate) fn ms(x: f64) -> Value {
    if x.is_finite() && x >= 0.0 {
        json!(x.round() as i64)
    } else {
        Value::Null
    }
}

fn row_json(r: &Row) -> Value {
    r.to_json()
}

/// The project a folder is a confirmed checkout of here (`prj_…`), what
/// agents are matched by across machines; None for a folder in no project
/// yet, or a cwd that isn't a path.
fn project_key(w: &Runtime, cwd: Option<&Value>) -> Option<String> {
    let db = w.setup.db.as_ref()?;
    cwd.and_then(Value::as_str).filter(|c| !c.is_empty()).and_then(|c| crate::checkouts::project_of(db, c))
}

// ---------- from the link ----------

/// The link came up, or went down. On the way up this host sends its full
/// presence, and again what has no stored receipt.
pub fn linked(w: &Runtime, up: bool) {
    if up && net(w).me().is_some() {
        presence(w, true);
        resend(w);
    }
}

/// Entries were lost on the way: a full presence, and again what has no stored
/// receipt.
pub fn gap(w: &Runtime) {
    linked(w, true);
}

/// A host left the network: its agents, claims and waiting handoffs go.
pub fn removed(w: &Runtime, host: &str) {
    if let Err(e) = store(w, |s| s.drop_net_host(host)) {
        w.log("warn", "net.removed", &why(&e));
    }
}

/// What this host tells the others now, if it changed: presence; and
/// handoffs made here that lapsed, and messages no one read; and the
/// intents that expired or wait to be started here.
pub fn tick(w: &Runtime) {
    if net(w).me().is_none() {
        return;
    }
    presence(w, false);
    lapse(w);
    unread(w);
    crate::intents::tick(w);
}

/// A body from another host.
pub fn receive(w: &Runtime, incoming: Incoming) {
    if net(w).me().is_none() {
        return;
    }
    let Incoming { from, kind, body, .. } = incoming;
    let _one = w.net.taking.locked();
    let done = match kind {
        Kind::Presence => presence_in(w, &from, &body),
        // Messages, receipts, claims and handoffs come as entries of their
        // author's feed (`entry_in`), never as bare bodies.
        Kind::Message | Kind::Receipt | Kind::Claims | Kind::Handoff | Kind::Intent | Kind::Assign | Kind::Report => Err(fail("it comes only as an entry of its machine's feed")),
        // The person's answer to a run's permission prompt, from another
        // machine's console (`network/operator/v2`).
        Kind::Operator => {
            crate::asks::answer_in(w, &from, &body);
            Ok(())
        }
    };
    if let Err(e) = done {
        w.log("warn", "net.receive", &format!("a {} from {from} wasn't taken: {}", kind.as_str(), why(&e)));
    }
}

// ---------- feeds ----------

fn kept_error(e: xenonet_store::StoreError) -> xenonet_store::sql::SqlError {
    xenonet_store::sql::SqlError::Other(e.message)
}

/// An entry of another member's feed, which the network checked against
/// the member's key: offered to its author's feed in the store and, when
/// it applies, applied with the held entries it released, all in one
/// transaction, so each is applied exactly once whatever way it came and
/// however often. Then this machine acks what it holds of the author's
/// kept entries, or asks for what's missing.
pub fn entry_in(w: &Runtime, a: Arrived) {
    if net(w).me().is_none() {
        return;
    }
    let Some(db) = w.setup.db.clone() else { return };
    let _one = w.net.taking.locked();
    let Arrived { host, entry, authority, .. } = a;
    let (author, network, kind, at) = (entry.author.clone(), entry.group.clone(), entry.kind, entry.pos());
    let copy = entry.body.clone();
    // A start or a run's ask waits on other threads that write the store
    // (the run it starts, the prompt it holds): it's taken in the
    // transaction, and acted on once that's committed, at most once.
    let mut after = None;
    let offered = db.transaction(|| -> std::result::Result<Offer, xenonet_store::sql::SqlError> {
        let offer = Feed::new(db.feeds()).offer(entry).map_err(kept_error)?;
        if let Offer::New(entries) = &offer {
            for (i, e) in entries.iter().enumerate() {
                // The authority came beside the entry that arrived, which
                // is the first; the ones it released rest on their kind.
                let none = json!({});
                let auth = if i == 0 { &authority } else { &none };
                if waits_on_others(e) {
                    after = Some((e.clone(), auth.clone()));
                } else {
                    apply(w, &host, e, auth);
                }
            }
            // What a compact entry ended: the feed closed those places.
            // XenoNet takes no kept kind that stays open here yet besides
            // handoffs, which don't record their place, so this is noted.
            if !entries.ended.is_empty() {
                w.log("info", "net.feed", &format!("{host} compacted its feed: {} entries applied here before are no longer open", entries.ended.len()));
            }
        }
        Ok(offer)
    });
    if let (Ok(_), Some((e, auth))) = (&offered, after) {
        apply(w, &host, &e, &auth);
    }
    let ack = |w: &Runtime| match Feed::new(db.feeds()).ack(&author, &network) {
        Ok(Some(a)) => net(w).ack(&network, &host, a.inc, a.high),
        Ok(None) => {}
        Err(e) => w.log("warn", "net.feed", &format!("where this machine stands with {host}'s feed wasn't read: {}", e.message)),
    };
    let kept = kind.class() == xenonet_net::feed::Class::Kept;
    match offered {
        Ok(Offer::New(_)) if kept => ack(w),
        Ok(Offer::New(_)) => {}
        // The author didn't hear this machine took it: a message's stored
        // receipt goes again, and a kept entry's ack.
        Ok(Offer::Duplicate) => {
            if kind == EntryKind::Message {
                again(w, &host, &copy);
            }
            if kept {
                ack(w);
            }
        }
        Ok(Offer::Gap(want)) => net(w).want(&network, &host, want.inc, want.from),
        Ok(Offer::Fork(_)) => w.log(
            "warn",
            "net.feed",
            &format!("{host} signed two different entries at {}.{} of its feed: nothing more from it is taken until you unfreeze it in XenoNet's window", at.inc, at.seq),
        ),
        Ok(Offer::Frozen(_)) => w.log("info", "net.feed", &format!("an entry from {host} was dropped: its feed is frozen here")),
        // From history the author gave up in a restore: it hears how far
        // this machine holds its feed.
        Ok(Offer::Stale) => ack(w),
        Ok(Offer::Expired) => w.log("info", "net.feed", &format!("a {} from {host} was refused: it was signed longer ago than its feed keeps entries", kind.as_str())),
        Err(e) => w.log("warn", "net.feed", &format!("an entry from {host} wasn't taken: {e}")),
    }
}

/// Whether an entry is a start or a run's ask, which waits on what other
/// threads write.
fn waits_on_others(e: &xenonet_net::feed::FeedEntry) -> bool {
    e.kind == EntryKind::Message && matches!(e.body.get("target").and_then(Value::as_str), Some(t) if t == xenonet_engine::delivery::starts::TARGET || t == crate::asks::TARGET)
}

/// What an applied entry does here. What it says is the author's member's
/// act, decided by its kind's own rules; one that isn't right is noted and
/// passed by, never taken again.
fn apply(w: &Runtime, host: &str, e: &xenonet_net::feed::FeedEntry, authority: &Value) {
    let done = match e.kind {
        EntryKind::Message => agent_in(w, &e.author, &e.body).and_then(|()| message_in(w, host, &e.body, authority, e.at as f64)),
        EntryKind::Receipt => receipt_in(w, host, &e.body),
        EntryKind::Handoff => handoff_in(w, host, &e.body),
        // The feed applies a compact entry itself: what it leaves out is
        // closed there (`Applied::ended`).
        EntryKind::Compact => Ok(()),
        EntryKind::Claim => claim_in(w, host, &e.author, &e.body),
        EntryKind::Intent => crate::intents::intent_in(w, host, &e.body).map_err(fail),
        EntryKind::Assign => crate::intents::assign_in(w, host, &e.body).map_err(fail),
        EntryKind::Report => crate::intents::report_in(w, host, &e.body).map_err(fail),
        // Xenolith's kinds belong in a space's feed, never a network's.
        EntryKind::CtxNote | EntryKind::CtxPropose | EntryKind::CtxPage | EntryKind::CtxRemove | EntryKind::CtxLog | EntryKind::CtxForget | EntryKind::CtxSnapshot => {
            Err(Fail::Bad(format!("a {} belongs in a space's feed, not a network's", e.kind.as_str())))
        }
    };
    if let Err(f) = done {
        w.log("warn", "net.receive", &format!("a {} from {host} wasn't taken: {}", e.kind.as_str(), why(&f)));
    }
}

/// The agent an entry names as its sender (`agent`, an `agt_…`): another
/// machine's, kept as the entry's author's the first time, and refused
/// when the store has it as another machine's (this one's included), so
/// no machine speaks as another's agent.
fn agent_in(w: &Runtime, author: &str, b: &Value) -> Result<()> {
    let Some(named) = b.get("agent").filter(|a| !a.is_null()) else { return Ok(()) };
    let id = named.as_str().and_then(|a| AgentId::parse(a).ok()).ok_or_else(|| fail("its agent isn't an agent's id"))?;
    let session = s(b, "sender");
    match w.store.with(|s| s.db().remote_agent(&id, author, session.as_deref(), now_ms() as i64)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(fail(format!("{id} isn't {author}'s agent"))),
        Err(e) => Err(fail(e.message)),
    }
}

/// A message entry this machine applied before, again: its stored receipt
/// goes again, since the sender didn't hear it.
fn again(w: &Runtime, host: &str, b: &Value) {
    let (Some(mid), Some(target)) = (s(b, "mid"), s(b, "target")) else { return };
    let Ok(Some(m)) = store(w, |st| st.message_by_mid(&mid, Some(host))) else { return };
    let id = m.num("id").unwrap_or(f64::NAN);
    if let Ok(Some(_)) = store(w, |st| st.delivery(id, &target)) {
        receipt(w, host, &mid, &target, "stored", None);
    }
}

/// Takes `author`'s feed in `network` again after a fork, keeping the entry
/// this machine had, and applies the entries held behind it: the person's,
/// from the window (the network checked that). How many it applied.
pub fn unfreeze(w: &Runtime, author: &str, network: &str) -> std::result::Result<usize, String> {
    let db = w.setup.db.clone().ok_or("this machine has no store")?;
    let host = crate::ground::governance(w)
        .and_then(|g| g.facts().view.and_then(|v| v.members.values().find(|m| m.machine.id() == author).map(|m| m.id.clone())))
        .ok_or_else(|| format!("{author} isn't a member of the network"))?;
    let _one = w.net.taking.locked();
    let none = json!({});
    let released = db
        .transaction(|| -> std::result::Result<usize, xenonet_store::sql::SqlError> {
            let entries = Feed::new(db.feeds()).unfreeze(author, network).map_err(kept_error)?;
            for e in &entries {
                apply(w, &host, e, &none);
            }
            Ok(entries.len())
        })
        .map_err(|e| e.to_string())?;
    if let Ok(Some(a)) = Feed::new(db.feeds()).ack(author, network) {
        net(w).ack(network, &host, a.inc, a.high);
    }
    Ok(released)
}

// ---------- presence ----------

fn tilde(w: &Runtime, p: &str) -> String {
    xenonet_os::paths::tilde(p, &w.setup.home)
}

/// This host's agents as presence lists them: with their detail, unless
/// the host shares titles only.
fn agents_now(w: &Runtime, titles_only: bool) -> Result<BTreeMap<String, Value>> {
    let mut agents = engine(w, |e, c| e.agents(c, ACTIVE_MS))?;
    // Every actor of this machine's, at work or not: another machine names
    // one by its agt_, which only presence tells it.
    if let Some(a) = &w.actors {
        let listed: Vec<String> = agents.iter().filter_map(|v| s(v, "id")).collect();
        for id in a.read(xenonet_actors::registry::Actors::ids) {
            if id != YOU && !listed.contains(&id) {
                if let Ok(v) = engine(w, |e, c| e.view(c, &id)) {
                    agents.push(v);
                }
            }
        }
    }
    // The rule for text that leaves the machine: the home folder,
    // and any other folder named for a person that the agents work in, as ~.
    let scrub = (!titles_only).then(|| {
        let home = w.setup.home.as_str();
        let mut also: Vec<String> = Vec::new();
        for a in &agents {
            if let Some(f) = crate::scrub::user_folder_of(a.get("cwd"), Some(home)) {
                if !also.contains(&f) {
                    also.push(f);
                }
            }
        }
        crate::scrub::Scrubber::new(Some(home), &also)
    });
    let now = now_ms();
    let mut out = BTreeMap::new();
    for a in agents {
        let id = s(&a, "id").unwrap_or_default();
        if is_remote(&id) || id == YOU {
            continue;
        }
        let key = project_key(w, a.get("cwd"));
        // What it's changing, inside its project, so another machine's
        // agent about to edit the same file in its checkout is warned.
        let edits: Vec<Value> = match (&key, &w.setup.db) {
            (Some(k), Some(db)) => items(&a, "editing")
                .iter()
                .filter(|e| n(e, "at").is_some_and(|at| now - at < EDIT_HOLD_MS))
                .filter_map(|e| crate::checkouts::place(db, &s(e, "path")?).ok().flatten())
                .filter(|p| p.project == *k && !p.rel.is_empty())
                .map(|p| json!(clip(&p.rel, 1000)))
                .take(EDITS_SENT)
                .collect(),
            _ => Vec::new(),
        };
        let waits = a.get("needsYou").is_some_and(|v| xenonet_text::text::truthy(Some(v)));
        let state = if waits { "waiting".to_string() } else { a.get("status").and_then(Value::as_str).unwrap_or("idle").to_string() };
        // Another machine names it by its agt_ alone; an actor says its
        // slug too, which scopes there match.
        let agent = wire_id(w, &id);
        if AgentId::parse(&agent).is_err() {
            continue;
        }
        out.insert(
            agent.clone(),
            json!({
                "id": agent,
                "app": opt(s(&a, "app"), 32),
                "actor": parse_actor_id(&id),
                "title": opt(s(&a, "title"), 200),
                "state": clip(&state, 32),
                "doing": opt(s(&a, "focus"), 280),
                "projectKey": opt(key, 300),
                "edits": edits,
                "branch": opt(s(&a, "branch"), 200),
                "cwd": opt(s(&a, "cwd").map(|p| tilde(w, &p)), 500),
                // To the half minute, so an agent at work isn't a change
                // every tick.
                "lastActive": ms((n(&a, "lastActive").unwrap_or(f64::NAN) / 30e3).floor() * 30e3),
                "waitsOnYou": waits,
            }),
        );
        if let Some(o) = out.get_mut(&agent).and_then(Value::as_object_mut).filter(|o| o["actor"].is_null()) {
            o.remove("actor");
        }
        if let (Some(scrub), Some(v)) = (&scrub, out.get_mut(&agent)) {
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
fn detail(v: &mut Value, live: &Value, waits: bool, scrub: &crate::scrub::Scrubber) {
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
        "actor": s(a, "actor").filter(|slug| xenonet_model::ids::is_actor_slug(slug)),
        "title": text("title", 200),
        "state": text("state", 32),
        "doing": text("doing", 280),
        "projectKey": text("projectKey", 300),
        "edits": a.get("edits").and_then(Value::as_array).map_or_else(Vec::new, |e| e.iter().filter_map(Value::as_str).take(EDITS_SENT).map(|p| json!(clip(p, 1000))).collect()),
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
fn you_here(w: &Runtime) -> (bool, bool) {
    let Some(a) = &w.actors else { return (false, false) };
    let p = a.read(|r| r.presence(YOU));
    (matches!(p.as_deref(), Some("working" | "idle")), p.as_deref() == Some("working"))
}

/// This host's presence, sent to every other host when it changed (or in
/// full). The body it sent, if it sent one.
pub fn presence(w: &Runtime, full: bool) -> Option<Value> {
    net(w).me()?;
    let now = match agents_now(w, net(w).titles_only()) {
        Ok(a) => a,
        Err(e) => {
            w.log("warn", "net.presence", &why(&e));
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
    // Who may start and wake sessions here, and what this XenoNet takes:
    // a change to the settings goes out as one.
    let reach = net(w).reach().to_json();
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
        if !full && agents.is_empty() && removed.is_empty() && you == out.you && you_at == out.you_at && roster == out.roster && reach == out.reach {
            return None;
        }
        out.counter = (out.counter + 1.0).max(now_ms().round());
        out.agents = now;
        out.you = you;
        out.you_at = you_at;
        out.roster.clone_from(&roster);
        out.reach = reach.clone();
        json!({ "schema": "network/presence/v1", "counter": out.counter, "full": full, "agents": agents, "removed": removed, "you": you, "youAt": you_at.map_or(Value::Null, ms), "roster": roster, "reach": reach, "can": xenonet_model::ops::CAN })
    };
    send(w, To::All, Kind::Presence, body.clone());
    Some(body)
}

/// Whether an id another machine names is an agent's: an `agt_`, which
/// its machine made. Never you.
fn own_id(id: &str) -> bool {
    AgentId::parse(id).is_ok()
}

/// The machine signing key id of `host`, a member of this machine's
/// network, as the roster has it.
fn machine_of(w: &Runtime, host: &str) -> Option<String> {
    let g = crate::ground::governance(w)?;
    let view = g.facts().view?;
    view.members.values().find(|m| m.id == host).map(|m| m.machine.id().clone())
}

fn presence_in(w: &Runtime, from: &str, body: &Value) -> Result<()> {
    let counter = n(body, "counter").ok_or_else(|| fail("a presence without a counter"))?;
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
    // An agt_ is its machine's alone: one the store has as another
    // machine's (this one's included) is left out.
    let machine = machine_of(w, from);
    for a in body.get("agents").and_then(Value::as_array).into_iter().flatten() {
        let Some(kept) = declared(a) else { continue };
        let id = s(&kept, "id").unwrap_or_default();
        let session = s(&kept, "actor").map(|slug| format!("actor:{slug}"));
        let theirs = match (&machine, AgentId::parse(&id)) {
            (Some(m), Ok(agt)) => w.store.with(|st| st.db().remote_agent(&agt, m, session.as_deref(), now_ms() as i64)).unwrap_or(false),
            _ => false,
        };
        if !theirs {
            continue;
        }
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
    let key = s(a, "projectKey");
    let cwd = s(a, "cwd");
    let project = cwd.as_deref().and_then(|c| c.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()).map(str::to_string);
    let state = s(a, "state").unwrap_or_else(|| "idle".into());
    let status = match state.as_str() {
        "working" => "working",
        "ended" => "ended",
        _ => "idle",
    };
    let title = s(a, "title").unwrap_or_else(|| local.clone());
    let mut v = json!({
        "id": local,
        "app": s(a, "app"),
        "actor": a.get("actor").cloned().unwrap_or(Value::Null),
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
        "projectKey": key,
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
pub fn remote_views(w: &Runtime, detail: bool) -> Vec<Value> {
    let net = net(w);
    if net.me().is_none() {
        return Vec::new();
    }
    let hosts = net.hosts();
    let rows = store(w, EngineStore::net_presences).unwrap_or_default();
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

/// An agent on another host by its agt_, as its host's presence shows
/// it, or a stand-in when the presence doesn't list it (any more).
pub(crate) fn remote_view(w: &Runtime, id: &str) -> Value {
    remote_views(w, false).into_iter().find(|v| v["id"] == id).unwrap_or_else(|| stub(w, id))
}

/// What another host last said of a delivery: { host, receipt }, or None.
pub(crate) fn receipt_of(w: &Runtime, message_id: f64, agent: &str) -> Result<Option<Value>> {
    let d = store(w, |s| s.delivery(message_id, agent))?;
    Ok(d.map(|d| json!({ "host": d.get("host").to_json(), "receipt": d.get("receipt").to_json() })))
}

/// An agent on a host whose presence doesn't list it (any more), on the
/// machine whose feed first named it.
fn stub(w: &Runtime, id: &str) -> Value {
    let row = AgentId::parse(id).ok().and_then(|a| w.store.with(|s| s.db().agent(&a)).ok().flatten());
    let app = row.as_ref().map(|r| r.app.clone()).filter(|a| !a.is_empty());
    let actor = row.as_ref().filter(|r| r.app == "actor").and_then(|r| r.vendor_id.clone());
    json!({ "id": id, "app": app, "actor": actor, "title": id, "project": null, "branch": null, "host": host_of_agent(w, id) })
}

// ---------- agents across machines ----------

/// An agent as it's named to another machine: this machine's session or
/// actor by its `agt_` (made the first time it's named), you as you, and
/// another machine's `agt_` as it is.
pub(crate) fn wire_id(w: &Runtime, id: &str) -> String {
    if id == YOU || AgentId::parse(id).is_ok() {
        return id.to_string();
    }
    match w.store.with(|s| s.db().agent_of(id, None, now_ms() as i64)) {
        Ok(a) => a.as_str().to_string(),
        Err(_) => id.to_string(),
    }
}

/// An agent another machine named, as this one writes it: one of this
/// machine's own `agt_` as its session or actor, anything else as it came.
pub(crate) fn here_id(w: &Runtime, id: &str) -> String {
    let Ok(a) = AgentId::parse(id) else { return id.to_string() };
    w.store.with(|s| s.db().session_of(&a)).ok().flatten().unwrap_or_else(|| id.to_string())
}

/// The machine (its host id) another machine's agent is on, by its
/// `agt_`: as presence lists it, else the machine whose feed first named
/// it, as the roster calls that machine. None for this machine's own or an
/// unknown one.
pub(crate) fn host_of_agent(w: &Runtime, id: &str) -> Option<String> {
    let a = AgentId::parse(id).ok()?;
    if let Some(v) = remote_views(w, false).into_iter().find(|v| v["id"] == id) {
        return s(&v, "host");
    }
    let row = w.store.with(|s| s.db().agent(&a)).ok().flatten()?;
    if row.machine == xenonet_store::projects::LOCAL {
        return None;
    }
    let g = crate::ground::governance(w)?;
    let view = g.facts().view?;
    view.members.values().find(|m| m.machine.id() == row.machine).map(|m| m.id.clone())
}

// ---------- messages out ----------

pub(crate) fn sender_kind(me: &str) -> &'static str {
    if me == YOU {
        "person"
    } else if parse_actor_id(me).is_some() {
        "actor"
    } else {
        "agent"
    }
}

/// A message stored here as sent goes to each agent on another host as an
/// entry of this machine's feed of its own, which goes again until its
/// stored receipt comes. One
/// from you carries your device's signature for each machine it goes to,
/// which only the window's own send makes (it asks the person where the
/// device key asks: Touch ID); without one, the machine it's for refuses
/// it as you.
pub(crate) fn sent(w: &Runtime, a: &Value) -> Result<()> {
    let id = num_of(a, "id");
    let env = a.get("envelope").cloned().unwrap_or(Value::Null);
    let mid = match s(&env, "mid") {
        Some(m) if m.starts_with("msg_") => m,
        _ => new_id("msg")?,
    };
    store(w, |s| s.set_message_net(id, None, &mid))?;
    let me = s(a, "sender").unwrap_or_default();
    let at_window = env.get("auth").and_then(|a| a.get("method")).and_then(Value::as_str) == Some("window");
    let reply_mid = match n(a, "replyTo") {
        Some(r) => store(w, |s| s.message(r))?.and_then(|m| m.text("mid").map(str::to_string)),
        None => None,
    };
    let text = s(a, "text").unwrap_or_default();
    let scope = a.get("scope").map_or(Value::Null, |v| Value::String(xenonet_text::text::display(v)));
    // Who sent it, by its agt_ (made the first time it's named), which the
    // receiver keeps as this machine's agent and names it by.
    let agent = (me != YOU).then(|| wire_id(w, &me)).filter(|a| AgentId::parse(a).is_ok());
    let mut signed: BTreeMap<String, Value> = BTreeMap::new();
    // An actor's message carries a token minted for it here, once, whatever
    // machines it goes to: its own token was decided when it sent it.
    let mut minted: Option<Value> = None;
    for r in items(a, "recipients") {
        let rid = s(&r, "id").unwrap_or_default();
        if !is_remote(&rid) {
            continue;
        }
        let Some(host) = host_of_agent(w, &rid) else { continue };
        let authority = match parse_actor_id(&me) {
            _ if me == YOU => signed.entry(host.clone()).or_insert_with(|| person_authority(w, &host, at_window)).clone(),
            Some(slug) => minted.get_or_insert_with(|| actor_authority(w, slug)).clone(),
            None => json!({}),
        };
        let body = json!({
            "schema": "network/send/v1",
            "mid": mid,
            "sender": me,
            "senderKind": sender_kind(&me),
            "agent": agent,
            "target": rid,
            "body": text,
            "replyTo": reply_mid,
            "scope": scope,
            "kind": "message",
            "hops": num_of(a, "hops").max(0.0),
            "at": ms(num_of(a, "at")),
        });
        let kept = json!({ "body": body, "authority": authority });
        store(w, |s| s.set_delivery_net(id, &rid, &host, &kept.to_string()))?;
        send_with(w, To::Host(host), Kind::Message, body, Some(authority));
    }
    Ok(())
}

/// The person's authority for a message to `host`: their device's
/// signature, bound to that machine. Empty when it can't be made here
/// (not from the window, or no identity yet), which that machine refuses.
fn person_authority(w: &Runtime, host: &str, at_window: bool) -> Value {
    let (Some(gov), Ok(machine)) = (crate::ground::governance(w), crate::ground::machine(host)) else { return json!({}) };
    let resource = xenonet_authz::Resource::Machine(machine);
    let now = gov.facts().now;
    match crate::ground::authority(&gov.identity(), at_window, &resource, &xenonet_authz::Act::Message, &format!("send a message as you to {host}"), now) {
        Ok(a) => a,
        Err(why) => {
            w.log("info", "net.message", &format!("a message as you to {host} goes without your signature, and {host} will refuse it: {why}"));
            json!({})
        }
    }
}

/// The authority of a message from this machine's actor `slug`: a token
/// for messages, minted for it here. Empty when it can't be made, which
/// the machine it goes to refuses.
fn actor_authority(w: &Runtime, slug: &str) -> Value {
    crate::ground::actor_authority(w, slug, vec![xenonet_authz::Action::Message]).unwrap_or_else(|why| {
        w.log("info", "net.message", &format!("a message from actor:{slug} goes without a token, and will be refused: {why}"));
        json!({})
    })
}

/// Sends again what has no stored receipt, oldest first.
pub fn resend(w: &Runtime) {
    let rows = store(w, |s| s.outbox(now_ms() - OUTBOX_MS)).unwrap_or_default();
    for r in rows {
        let (Some(host), Some(frame)) = (r.text("host"), r.text("frame")) else { continue };
        let Ok(kept) = serde_json::from_str::<Value>(frame) else { continue };
        send_with(w, To::Host(host.to_string()), Kind::Message, kept["body"].clone(), kept.get("authority").cloned());
    }
}

// ---------- messages in ----------

fn receipt(w: &Runtime, to: &str, mid: &str, agent: &str, state: &str, reason: Option<&str>) {
    let body = json!({ "schema": "network/receipt/v1", "mid": mid, "agent": agent, "state": state, "reason": reason.map(|r| clip(r, 280)), "at": ms(now_ms()) });
    send(w, To::Host(to.to_string()), Kind::Receipt, body);
}

/// How XenoNet knows who sent a message from another host: that host's
/// signed frame, and what this machine decided it rests on: the person's
/// device signature, or the sending machine's word for its own agent.
fn envelope(from: &str, mid: &str, at: f64, sender: &str, kind: &str, name: Option<&str>, hops: f64) -> Value {
    let person = kind == "person";
    let actor = kind == "actor";
    let tail = "This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.";
    let line = if person {
        format!("verified: you, by your device's signature, checked against this machine's copy of your identity; it came from {from}'s signed frame. {tail}")
    } else if actor {
        format!(
            "verified: the actor \"{}\" ({sender}), on {from}, another machine in your XenoNet network, by a token {from} minted for it, checked against {from}'s signing key in your network's chain. {tail}",
            name.unwrap_or(sender),
        )
    } else {
        format!(
            "verified: the session \"{}\" ({sender}), on {from}, another machine in your XenoNet network, by {from}'s signed frame; {from} checked its process. {tail}",
            name.unwrap_or(sender)
        )
    };
    let shown = if person { "you" } else { kind };
    let authority = if person {
        "person"
    } else if actor {
        "token"
    } else {
        "machine"
    };
    json!({
        "schema": "message/envelope/v1",
        "mid": mid,
        "at": ms(at),
        "sender": { "id": sender, "kind": shown, "name": name, "product": null },
        "auth": { "method": "host", "process": "n/a", "attestation": if person || actor { "n/a" } else { "process-checked" } },
        "client": null,
        "transport": "network",
        "host": from,
        "original": { "kind": shown, "authority": authority },
        "hops": hops,
        "line": line,
    })
}

/// A message as the store takes it: to `target`, with its recipients and
/// envelope. Its text is information: stored and shown, never decided on.
fn stored_message(sender: &str, target: &str, body: &Information<String>, reply_to: Option<f64>, env: &Value) -> Value {
    obj(vec![
        ("sender", json!(sender)),
        ("target", json!(target)),
        ("scope", Value::Null),
        ("body", json!(body.read())),
        ("replyTo", reply_to.map_or(Value::Null, number)),
        ("recipients", json!([target])),
        ("envelope", env.clone()),
    ])
}

/// Who an act from another machine says it's from, as its body carries it.
pub(crate) struct SentBy<'a> {
    /// The machine that sent it.
    pub host: &'a str,
    /// person, agent or actor.
    pub kind: &'a str,
    pub sender: &'a str,
    /// The body's `authority`.
    pub authority: &'a Value,
}

/// Who sent an act, as this machine takes them: the person by their
/// device's signature over `act` done to `resource`; an actor by the token
/// its machine minted for the act, which must be for the actor the frame
/// names (kept, with its root, for the decision); else an agent of that
/// machine, vouched for by it.
pub(crate) fn principal_of(
    w: &Runtime,
    g: &mut crate::ground::EngineGround,
    from: &SentBy<'_>,
    resource: &xenonet_authz::Resource,
    act: &xenonet_authz::Act,
) -> std::result::Result<(xenonet_authz::Principal, Option<xenonet_authz::Token>), String> {
    let machine = crate::ground::machine(from.host)?;
    let carried = crate::ground::Carried::parse(from.authority);
    match from.kind {
        "person" => crate::ground::person(w, g, Some(from.host), &carried, resource, act).map(|p| (p, None)),
        "actor" => {
            let (p, token) = crate::ground::actor(g, &machine, &carried)?;
            let named = parse_actor_id(from.sender).map(str::to_string);
            match &p {
                xenonet_authz::Principal::Actor { slug, .. } if named.as_deref() == Some(slug.as_str()) => Ok((p, Some(token))),
                _ => Err(format!("that token isn't {}'s", from.sender)),
            }
        }
        _ => crate::ground::agent(from.sender, &machine).map(|p| (p, None)),
    }
}

/// `principal_of`, then the request it makes from another machine, judged.
pub(crate) fn judge_remote(
    w: &Runtime,
    g: &mut crate::ground::EngineGround,
    from: &SentBy<'_>,
    resource: xenonet_authz::Resource,
    act: xenonet_authz::Act,
) -> std::result::Result<(), String> {
    let (p, token) = principal_of(w, g, from, &resource, &act)?;
    crate::ground::judge_by(g, &crate::ground::remote(g, p, resource, act), token.as_ref(), None)
}

fn message_in(w: &Runtime, from: &str, b: &Value, authority: &Value, at: f64) -> Result<()> {
    let (Some(mid), Some(sender), Some(kind), Some(target), Some(body)) = (s(b, "mid"), s(b, "sender"), s(b, "senderKind"), s(b, "target"), s(b, "body")) else {
        return Err(fail("a message without its mid, sender, target or body"));
    };
    // What it says is information from here on: the decisions below read
    // the frame's sender and the act's authority, never the text.
    let body = Information::new(body);
    // An agent or actor is named by its agt_ (`agent`), which agent_in
    // checked is the sending machine's; `sender` is its name there, which
    // an actor's token is checked against.
    let agent = s(b, "agent");
    let sender_ok = match kind.as_str() {
        "person" => sender == YOU,
        "agent" => agent.as_deref().is_some_and(own_id) && is_session_id(&sender),
        "actor" => agent.as_deref().is_some_and(own_id) && parse_actor_id(&sender).is_some(),
        _ => false,
    };
    if !mid.starts_with("msg_") || !sender_ok || body.read().is_empty() || body.read().chars().count() > 16000 {
        return Err(fail("a message that isn't network/send/v1"));
    }
    let authority = authority.clone();
    // A start request: the starts' own (starts.rs), answered by receipts
    // keyed by its mid. An older XenoNet refuses the target below.
    if target == xenonet_engine::delivery::starts::TARGET {
        crate::starts::start_in(w, from, &mid, &kind, &sender, agent.as_deref().unwrap_or(YOU), body, &authority, at);
        return Ok(());
    }
    let Some(mut g) = crate::ground::ground(w, None) else {
        return Err(fail("this machine's governance isn't plugged in, so it takes nothing from another machine"));
    };
    // A permission prompt of a run on another machine, for this console,
    // or its end (asks.rs): a member's message to this machine.
    if target == crate::asks::TARGET {
        let own = xenonet_authz::Resource::Machine(g.facts.machine.clone());
        let decided = judge_remote(w, &mut g, &SentBy { host: from, kind: &kind, sender: &sender, authority: &authority }, own, xenonet_authz::Act::Message);
        match decided {
            Ok(()) => crate::asks::ask_in(w, from, &body),
            Err(why) => w.log("info", "net.ask", &format!("an ask from {from} was refused: {why}")),
        }
        return Ok(());
    }
    // The target by its agt_: one of this machine's sessions or actors.
    let local = here_id(w, &target);
    if !(is_agent_id(&local) || is_actor_agent(&local)) {
        receipt(w, from, &mid, &target, "refused", Some("no session or actor here has that id"));
        return Ok(());
    }
    // Decided here: who sent it, on this machine's own facts. A message as
    // you carries your device's signature, bound to this machine.
    let resource = xenonet_authz::Resource::Machine(g.facts.machine.clone());
    let decided = judge_remote(w, &mut g, &SentBy { host: from, kind: &kind, sender: &sender, authority: &authority }, resource, xenonet_authz::Act::Message);
    if let Err(why) = decided {
        receipt(w, from, &mid, &target, "refused", Some(&why));
        return Ok(());
    }
    // The person is one person, on every machine.
    let sender_id = if kind == "person" { YOU.to_string() } else { agent.clone().unwrap_or_default() };
    // A repeat (the sender didn't hear the stored receipt): say so again.
    let had = store(w, |s| s.message_by_mid(&mid, Some(from)))?;
    if let Some(m) = &had {
        let id = m.num("id").unwrap_or(f64::NAN);
        if store(w, |s| s.delivery(id, &local))?.is_some() {
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
    let view = engine(w, |e, c| e.view(c, &local))?;
    // An actor XenoNet only observes reads nothing: refused, as a message
    // sent on this machine is, rather than stored for no one.
    let unreached = engine(w, |_, c| Engine::unreached(c, &view))?;
    if let Some(why) = unreached {
        receipt(w, from, &mid, &target, "refused", Some(&why));
        return Ok(());
    }
    // This host's own limits hold for it too: paused, too fast, a repeat.
    if let Err(e) = w.policy.check_send(&sender_id, std::slice::from_ref(&view), body.read(), hops) {
        receipt(w, from, &mid, &target, "refused", Some(&why(&Fail::from(e))));
        return Ok(());
    }
    let name = remote_views(w, false).into_iter().find(|v| v["id"] == sender_id).and_then(|v| s(&v, "title"));
    let env = envelope(from, &mid, now_ms(), &sender_id, &kind, name.as_deref(), hops);
    let now = now_ms();
    let id = match &had {
        Some(m) => {
            let id = m.num("id").unwrap_or(f64::NAN);
            store(w, |s| s.deliver(id, local.as_str()))?;
            id
        }
        None => {
            let stored = stored_message(&sender_id, &local, &body, reply_to, &env);
            let id = store(w, |s| s.send(&stored, now))?;
            store(w, |s| s.set_message_net(id, Some(from), &mid))?;
            id
        }
    };
    // A session that bypasses its permission checks holds it until it asks.
    let held = w.engine.bypassed(&local);
    receipt(w, from, &mid, &target, if held { "held" } else { "stored" }, None);
    w.mail.wake(&local);
    // How it reads when it's pushed into the session: its framing first.
    let framing = if kind == "person" { xenonet_engine::network::from_you_line(from) } else { xenonet_engine::network::from_host_line(from) };
    let mut event_body = obj(vec![
        ("id", number(id)),
        ("from", json!(sender_id)),
        ("to", json!([local])),
        ("target", json!(local)),
        ("text", json!(format!("{framing}\n{}", body.read()))),
        ("ts", number(now)),
        ("verified", env["line"].clone()),
        ("attestation", env["auth"]["attestation"].clone()),
        ("host", json!(from)),
        ("mid", json!(mid)),
    ]);
    if let Some(name) = name {
        push(&mut event_body, "fromName", json!(format!("{name} on {from}")));
    }
    if held {
        push(&mut event_body, "held", Value::Bool(true));
    }
    event(w, "message", &event_body);
    Ok(())
}

/// The messages a reply chains back through in this host's store, the one
/// it answers included: how deep its thread is here.
fn local_depth(w: &Runtime, reply_to: Option<f64>) -> f64 {
    let mut depth = 0.0;
    let mut next = reply_to;
    while let Some(id) = next {
        if depth >= 50.0 {
            break;
        }
        let Ok(Some(m)) = store(w, |s| s.message(id)) else { break };
        depth += 1.0;
        next = m.num("reply_to");
    }
    depth
}

fn receipt_in(w: &Runtime, from: &str, b: &Value) -> Result<()> {
    let (Some(mid), Some(agent), Some(state)) = (s(b, "mid"), s(b, "agent"), s(b, "state")) else {
        return Err(fail("a receipt without its mid, agent or state"));
    };
    // An answer to a start request sent from here.
    if crate::starts::answer_in(w, from, &mid, b) {
        return Ok(());
    }
    let Some(m) = store(w, |s| s.message_by_mid(&mid, None))? else { return Ok(()) };
    let id = m.num("id").unwrap_or(f64::NAN);
    // The recipient by its agt_, as the delivery here names it.
    let who = agent;
    let changed = store(w, |s| s.set_receipt(id, &who, &state, now_ms()))?;
    if changed {
        w.emit(From::Engine, "receipt", json!({ "id": id, "agent": who, "host": from, "state": state, "reason": b.get("reason").cloned().unwrap_or(Value::Null) }));
    }
    Ok(())
}

/// Messages a session took: those from other hosts get a seen receipt.
pub fn seen(w: &Runtime, agent: &str, ids: &[Value]) {
    if net(w).me().is_none() {
        return;
    }
    for id in ids {
        let Ok(Some(m)) = store(w, |s| s.message(xenonet_text::text::to_number(Some(id)))) else { continue };
        if let (Some(host), Some(mid)) = (m.text("host"), m.text("mid")) {
            receipt(w, host, mid, &wire_id(w, agent), "seen", None);
        }
    }
}

/// A message from another host that the router pushed straight into its
/// session: a delivered receipt.
pub fn delivered(w: &Runtime, m: &Value) {
    let (Some(host), Some(mid)) = (s(m, "host"), s(m, "mid")) else { return };
    let id = m.get("id").and_then(Value::as_f64);
    for agent in items(m, "to") {
        let Some(agent) = agent.as_str() else { continue };
        let live = w.router.core.list(200).iter().any(|r| r.get("message").and_then(Value::as_f64) == id && r["agent"] == agent && r["state"] == "live");
        if live {
            receipt(w, &host, &mid, &wire_id(w, agent), "delivered", None);
        }
    }
}

// ---------- claims ----------

/// A claim or a release this machine made (`network/claim/v1`), to every
/// other member as a kept entry of its feed.
pub(crate) fn claimed(w: &Runtime, mut body: Value) {
    if net(w).me().is_some() {
        // Its agent by its agt_.
        if let Some(a) = s(&body, "agent") {
            body["agent"] = json!(wire_id(w, &a));
        }
        send(w, To::All, Kind::Claims, body);
    }
}

/// `from`'s machine-level act here (a handoff), decided: a member of the
/// network.
fn member_act(w: &Runtime, from: &str, act: xenonet_authz::Act) -> Result<()> {
    let Some(g) = crate::ground::ground(w, None) else { return Err(fail("this machine's governance isn't plugged in")) };
    let id = crate::ground::machine(from).map_err(fail)?;
    let request = crate::ground::remote(&g, xenonet_authz::Principal::Machine { id }, xenonet_authz::Resource::Machine(g.facts.machine.clone()), act);
    crate::ground::judge(&g, &request, None).map_err(fail)
}

/// A claim entry of `from`'s feed (its author's key id `author`): decided
/// as that member's claim in its project, of one of its own agents, and
/// kept as its; or a release of what it kept.
fn claim_in(w: &Runtime, from: &str, author: &str, b: &Value) -> Result<()> {
    let c: xenonet_model::network::Claim = serde_json::from_value(b.clone()).map_err(|e| fail(format!("a claim that isn't network/claim/v1: {e}")))?;
    let (agent, path) = (c.agent.as_str().to_string(), c.path.as_str().to_string());
    if !own_id(&agent) {
        return Err(fail(format!("{from} claimed for {agent}, which isn't one of its own agents")));
    }
    if path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err(fail("a claim's path is inside its project"));
    }
    let project = c.project.as_str().to_string();
    let Some(g) = crate::ground::ground(w, None) else { return Err(fail("this machine's governance isn't plugged in")) };
    let id = crate::ground::machine(from).map_err(fail)?;
    let p = xenonet_authz::ProjectId::new(&project).map_err(|e| fail(e.message))?;
    let request = crate::ground::remote(&g, xenonet_authz::Principal::Machine { id }, xenonet_authz::Resource::Project(p.clone()), xenonet_authz::Act::Claim { project: p });
    crate::ground::judge(&g, &request, None).map_err(fail)?;
    // An agt_ is its machine's alone: the feed's author's.
    let ours = AgentId::parse(&agent).is_ok_and(|a| w.store.with(|st| st.db().remote_agent(&a, author, None, now_ms() as i64)).unwrap_or(false));
    if !ours {
        return Err(fail(format!("{agent} isn't {from}'s agent")));
    }
    let shown = agent.clone();
    if c.op == xenonet_model::network::ClaimOp::Release {
        store(w, |s| s.release_remote(from, &shown, &project, (!path.is_empty()).then_some(path.as_str())))?;
        return Ok(());
    }
    if path.is_empty() {
        return Err(fail("a claim names a path"));
    }
    // A host keeps at most MAX_CLAIMS here, and none longer than eight
    // hours from now, whatever it says.
    if store(w, |s| s.remote_claim_count(from))? >= MAX_CLAIMS {
        w.log("warn", "net.claims", &format!("{from} has {MAX_CLAIMS} claims here, the most a host keeps: the rest were left out"));
        return Ok(());
    }
    let now = now_ms();
    let created = c.created_at.as_ref().map_or(now, |m| m.0.min(now));
    let until = c.expires_at.as_ref().map_or(now + CLAIM_MAX_MS, |m| m.0.min(now + CLAIM_MAX_MS));
    let row = json!({
        "project": project, "path": path, "agent": shown, "author": author, "host": from,
        "branch": c.branch.as_ref().map(|b| b.as_str().to_string()), "note": c.note.as_ref().map(|n| n.as_str().to_string()),
        "created_at": created, "expires_at": until,
    });
    store(w, |s| s.claim(&row))
}

/// Agents on other hosts with a hand in the files `me` is about to edit,
/// in their checkouts of the same project: their claims, kept here, and
/// what their presence says they're changing.
pub(crate) fn claim_hits(w: &Runtime, me: &str, files: &[String]) -> Result<Vec<Value>> {
    let Some(db) = w.setup.db.clone() else { return Ok(Vec::new()) };
    let now = now_ms();
    let presences = store(w, EngineStore::net_presences)?;
    let mut out = Vec::new();
    let _ = me;
    for file in files {
        let Ok(Some(at)) = crate::checkouts::place(&db, file) else { continue };
        if !at.confirmed {
            continue;
        }
        for r in store(w, |s| s.claims_in(&at.project, now))? {
            let (Some(host), Some(path)) = (r.text("host"), r.text("path")) else { continue };
            if !xenonet_engine::engine::matches(path, &at.rel) {
                continue;
            }
            out.push(obj(vec![
                ("host", json!(host)),
                ("agent", json!(r.text("agent"))),
                ("branch", r.get("branch").to_json()),
                ("kind", json!("claim")),
                ("file", json!(file)),
                ("path", json!(at.rel)),
            ]));
        }
        for p in &presences {
            let host = p.text("host").unwrap_or("");
            let agents: Vec<Value> = p.text("agents").and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
            for a in agents.iter().filter(|a| s(a, "projectKey").as_deref() == Some(at.project.as_str())) {
                if a.get("edits").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).any(|e| e == at.rel) {
                    out.push(obj(vec![
                        ("host", json!(host)),
                        ("agent", json!(s(a, "id").unwrap_or_default())),
                        ("branch", a.get("branch").cloned().unwrap_or(Value::Null)),
                        ("kind", json!("edit")),
                        ("file", json!(file)),
                        ("path", json!(at.rel)),
                    ]));
                }
            }
        }
    }
    Ok(out)
}

// ---------- handoffs ----------

/// A handoff as a session reads it.
fn shown(r: &Row, me_host: Option<&str>) -> Value {
    let host = r.text("host");
    // Another machine's maker by its agt_, its machine beside it.
    let from = r.text("maker").unwrap_or("").to_string();
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

/// The repo a session's handoffs are scoped to: its repo, else its
/// project.
fn scope_of(view: &Value) -> Value {
    match view.get("repo") {
        Some(r) if !r.is_null() => r.clone(),
        _ if view.get("projectKind").and_then(Value::as_str) == Some("project") => view.get("projectId").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// The handoffs open to a session: to it, or to its repo (here, or across
/// clones of the same remote), never its own.
pub(crate) fn offers(w: &Runtime, me: &Value) -> Result<Vec<Value>> {
    let rows = store(w, |s| s.open_handoffs(now_ms()))?;
    // Most starts find none, and then ask git nothing.
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let id = s(me, "id").unwrap_or_default();
    let me_host = net(w).me().map(|m| m.host);
    let key = project_key(w, me.get("cwd"));
    let scope = scope_of(me);
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
                let by_key = key.as_deref().is_some_and(|k| r.text("to_repo") == Some(k));
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

/// A message from `me`, as the handoff tool sends one: `{ to, text }`.
fn send_from(w: &Runtime, me: &str, to: &str, text: &str, envelope: &Value) -> Result<Value> {
    let ask = json!({ "to": to, "text": text, "replyTo": null });
    engine(w, |e, c| e.send(c, me, &ask, envelope))
}

/// Who a sent message went to: the ids in its `to`.
fn told_ids(sent: &Value) -> Vec<Value> {
    items(sent, "to").iter().map(|x| x.get("id").cloned().unwrap_or(Value::Null)).collect()
}

/// The handoff tool: offered, taken, or the ones open to `me`.
pub fn handoff(w: &Runtime, me: &str, a: &Value, envelope: &Value) -> Result<Value> {
    // Off a network nothing ticks, so the ones that lapsed go here.
    if net(w).me().is_none() {
        lapse(w);
    }
    if let Some(id) = s(a, "accept") {
        return accept(w, me, &id, envelope);
    }
    if ["goal", "done", "next"].iter().any(|k| s(a, k).is_some_and(|t| !t.trim().is_empty())) {
        return offer(w, me, a, envelope);
    }
    let view = engine(w, |e, c| e.view(c, me))?;
    let open = offers(w, &view)?;
    let mine: Vec<Value> = store(w, |s| s.open_handoffs(now_ms()))?
        .iter()
        .filter(|r| r.text("host").is_none() && r.text("maker") == Some(me))
        .map(|r| shown(r, net(w).me().map(|m| m.host).as_deref()))
        .collect();
    Ok(json!({ "open": open, "yours": mine }))
}

fn offer(w: &Runtime, me: &str, a: &Value, envelope: &Value) -> Result<Value> {
    let view = engine(w, |e, c| e.view(c, me))?;
    let commit = s(a, "commit").map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty());
    if let Some(sha) = &commit {
        if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Fail::Bad("commit is a commit's hash: 7 to 64 hex characters".into()));
        }
    }
    let files: Vec<String> = a.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).map(|p| clip(p, 1000)).take(200).collect()).unwrap_or_default();
    let field = |k: &str, max: usize| s(a, k).map(|t| clip(t.trim(), max)).filter(|t| !t.is_empty());
    let (goal, done, next) = (field("goal", 2000), field("done", 8000), field("next", 8000));
    let branch = field("branch", 200).or_else(|| s(&view, "branch").map(|b| clip(&b, 200)));
    let to = s(a, "to").filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "repo".into());
    let me_net = net(w).me();
    let host = s(a, "host").filter(|h| !h.is_empty());
    if let Some(h) = &host {
        let known = me_net.as_ref().is_some_and(|m| m.host == *h) || net(w).hosts().iter().any(|x| x.id == *h);
        if !known {
            return Err(Fail::Bad(format!("{h} isn't a machine in your XenoNet network")));
        }
    }
    let key = project_key(w, view.get("cwd"));
    let scope = scope_of(&view);
    let id = new_id("hof")?;
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
        let sent = send_from(w, me, &to, &words, envelope)?;
        let first = items(&sent, "to").first().and_then(|t| s(t, "id")).unwrap_or(to);
        let host = if is_remote(&first) { host_of_agent(w, &first) } else { None };
        (Some(first.clone()), host, Some(sent))
    };
    if to_agent.is_none() && key.is_none() && !xenonet_text::text::truthy(Some(&scope)) {
        return Err(Fail::Bad("a handoff to the repo needs a repo: this session isn't in one".into()));
    }
    let row = obj(vec![
        ("id", json!(id)),
        ("host", Value::Null),
        ("maker", json!(me)),
        ("to_agent", json!(to_agent)),
        ("to_repo", json!(key)),
        ("to_scope", scope.clone()),
        ("to_host", json!(to_host)),
        ("goal", json!(goal)),
        ("done", json!(done)),
        ("next", json!(next)),
        ("files", json!(json!(files).to_string())),
        ("branch", json!(branch)),
        ("commit_sha", json!(commit)),
        ("created_at", number(now)),
        ("expires_at", number(now + HANDOFF_MS)),
    ]);
    store(w, |s| s.add_handoff(&row))?;
    // The sessions at work in clones of the repo now hear of it, here and
    // on the other hosts; the next to start in one is offered it.
    let mut told_now = target.as_ref().map(told_ids).unwrap_or_default();
    if to_agent.is_none() {
        let to = match &host {
            Some(h) if me_net.as_ref().is_none_or(|m| m.host != *h) => key.as_deref().and_then(|k| k.rsplit('/').next()).map(|p| format!("machine:{h} project:{p}")),
            Some(_) => key.as_deref().and_then(|k| k.rsplit('/').next()).map(|p| format!("machine:{} project:{p}", me_net.as_ref().map_or("local", |m| m.host.as_str()))),
            None => Some("repo".into()),
        };
        if let Some(to) = to {
            if let Ok(sent) = send_from(w, me, &to, &words, envelope) {
                told_now = told_ids(&sent);
            }
        }
    }
    // The other hosts keep it too, for their sessions to be offered.
    if let (Some(m), Some(row)) = (&me_net, store(w, |s| s.handoff(&id))?) {
        let body = body_of(&row, "offer", None);
        let dest = match (&to_agent, &to_host) {
            (Some(a), _) => is_remote(a).then(|| host_of_agent(w, a)).flatten().map(Some),
            (None, Some(h)) if *h != m.host => Some(Some(h.clone())),
            (None, Some(_)) => None,
            (None, None) => key.as_ref().map(|_| None),
        };
        if let Some(d) = dest {
            send_handoff(w, d.as_deref(), body);
        }
    }
    Ok(
        json!({ "id": id, "to": to_agent.map_or_else(|| json!({ "repo": key.clone().or_else(|| scope.as_str().map(str::to_string)), "host": host }), |a| json!({ "agent": a })), "toldNow": told_now, "lapses": ms(now + HANDOFF_MS) }),
    )
}

/// A handoff, as an entry of this machine's feed: kept, so every member
/// gets it in order, and it names the one machine it's for, if one.
fn send_handoff(w: &Runtime, for_host: Option<&str>, mut body: Value) {
    if let Some(h) = for_host {
        body["for"] = json!(h);
    }
    // Its agents by their agt_.
    for at in ["/from", "/by", "/to/agent"] {
        if let Some(v) = body.pointer_mut(at) {
            if let Some(id) = v.as_str().map(str::to_string) {
                *v = json!(wire_id(w, &id));
            }
        }
    }
    send(w, To::All, Kind::Handoff, body);
}

/// A handoff's body, from its row here.
fn body_of(r: &Row, op: &str, by: Option<&str>) -> Value {
    let files: Value = r.text("files").and_then(|f| serde_json::from_str(f).ok()).unwrap_or_else(|| json!([]));
    json!({
        "schema": "network/handoff/v1",
        "id": r.text("id"),
        "op": op,
        "from": r.text("maker"),
        "to": { "agent": r.text("to_agent"), "projectKey": r.text("to_repo"), "host": r.text("to_host") },
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
pub(crate) fn note(w: &Runtime, sender: &str, target: &str, words: &str, host: Option<&str>) {
    note_with(w, sender, target, words, host, Way::Plain);
}

/// A note that waits for the session's next turn, and never wakes it: a
/// start's started and ended, which a wake under `wake: network` would
/// otherwise turn into a turn of their own.
pub(crate) fn quiet_note(w: &Runtime, sender: &str, target: &str, words: &str) {
    note_with(w, sender, target, words, None, Way::Quiet);
}

/// A start's end, told to the session that asked for it: delivered at
/// once, waking the session if it's idle. The ask is its consent to this
/// one wake, so the machine's wake setting doesn't stand in the way; the
/// wake budgets, a paused XenoNet and a plan near its limit still do.
pub(crate) fn start_end_note(w: &Runtime, sender: &str, target: &str, words: &str) {
    note_with(w, sender, target, words, None, Way::StartEnd);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Plain,
    Quiet,
    StartEnd,
}

fn note_with(w: &Runtime, sender: &str, target: &str, words: &str, host: Option<&str>, way: Way) {
    let mut env = Value::Null;
    if let Some(h) = host {
        if let Ok(mid) = new_id("msg") {
            env = envelope(h, &mid, now_ms(), sender, "agent", None, 0.0);
        }
    }
    let stored = stored_message(sender, target, &Information::new(words.to_string()), None, &env);
    let now = now_ms();
    let Ok(id) = store(w, |s| s.send(&stored, now)) else { return };
    w.mail.wake(target);
    let mut event_body = obj(vec![("id", number(id)), ("from", json!(sender)), ("to", json!([target])), ("target", json!(target)), ("text", json!(words)), ("ts", number(now))]);
    if !env.is_null() {
        push(&mut event_body, "verified", env["line"].clone());
    }
    match way {
        Way::Quiet => push(&mut event_body, "quiet", Value::Bool(true)),
        Way::StartEnd => push(&mut event_body, "startEnd", Value::Bool(true)),
        Way::Plain => {}
    }
    event(w, "message", &event_body);
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

fn accept(w: &Runtime, me: &str, id: &str, envelope: &Value) -> Result<Value> {
    let Some(r) = store(w, |s| s.handoff(id))? else {
        return Err(Fail::Bad(format!("no handoff is {id}; the handoff tool with nothing else lists the ones open to you")));
    };
    let me_net = net(w).me();
    match r.text("state") {
        Some("taken") => return Err(fail(format!("{id} was taken by {}", r.text("taken_by").unwrap_or("another session")))),
        Some("lapsed") => return Err(fail(format!("{id} lapsed: no one took it in a week"))),
        _ => {}
    }
    // One handed to a session is that session's to take.
    if let Some(a) = r.text("to_agent").filter(|a| *a != me) {
        return Err(fail(format!("{id} was handed to {a}, not to you")));
    }
    match r.text("host") {
        // Made here: this host decides, and the first to accept gets it.
        None => {
            if !store(w, |s| s.take_handoff(id, me, now_ms()))? {
                let now = store(w, |s| s.handoff(id))?;
                return Err(fail(format!("{id} was taken by {}", now.as_ref().and_then(|r| r.text("taken_by")).unwrap_or("another session"))));
            }
            let maker = r.text("maker").unwrap_or("").to_string();
            if maker != me {
                // The maker may have gone; the handoff is taken all the same.
                let _ = send_from(w, me, &maker, &format!("I took your handoff {id}."), envelope);
            }
            if me_net.is_some() {
                send_handoff(w, None, body_of(&r, "taken", Some(me)));
            }
            Ok(json!({ "taken": true, "handoff": shown(&r, me_net.as_ref().map(|m| m.host.as_str())), "words": details(&r) }))
        }
        // Made on another host, which decides: the accept waits for it.
        Some(h) => {
            let Some(m) = &me_net else {
                return Err(fail(format!("{id} was made on {h}, and this machine isn't in a XenoNet network now")));
            };
            store(w, |s| s.ask_handoff(id, me))?;
            send_handoff(w, Some(h), body_of(&r, "accept", Some(me)));
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

fn handoff_in(w: &Runtime, from: &str, b: &Value) -> Result<()> {
    member_act(w, from, xenonet_authz::Act::Handoff)?;
    let (Some(id), Some(op)) = (s(b, "id"), s(b, "op")) else { return Err(fail("a handoff without its id or op")) };
    if !id.starts_with("hof_") {
        return Err(fail("a handoff that isn't network/handoff/v1"));
    }
    let Some(me) = net(w).me() else { return Ok(()) };
    // Every member gets every handoff, in its maker's order; one meant for
    // another machine is passed by.
    if s(b, "for").is_some_and(|h| h != me.host) {
        return Ok(());
    }
    let now = now_ms();
    // Ids as this host writes them: its own by their sessions' and actors'
    // ids, others' by their agt_.
    let here = |id: &str| here_id(w, id);
    match op.as_str() {
        "offer" => {
            let maker = s(b, "from").unwrap_or_default();
            if !own_id(&maker) || maker.chars().count() > 200 {
                return Err(fail("a handoff whose maker isn't one of its host's agents"));
            }
            if store(w, |st| st.open_handoffs_from(from, now))? >= MAX_HANDOFFS {
                return Err(fail(format!("{from} has {MAX_HANDOFFS} handoffs open here, the most a host keeps")));
            }
            // As the schema has them: what's past its limits is cut. What
            // it says is information, stored to show the taker, never
            // decided on: the act was the member's handoff, decided above.
            let field = |k: &str, max: usize| s(b, k).map(|t| clip(&t, max));
            let note = |k: &str, max: usize| field(k, max).map(Information::new);
            let (goal, done, next) = (note("goal", 2000), note("done", 8000), note("next", 8000));
            let text = |t: &Option<Information<String>>| t.as_ref().map(|t| t.read().clone());
            let files: Vec<Value> =
                b.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).take(200).map(|p| json!(clip(p, 1000))).collect()).unwrap_or_default();
            let commit = s(b, "commit").filter(|c| (7..=64).contains(&c.len()) && c.bytes().all(|x| x.is_ascii_hexdigit()));
            let to = b.get("to").cloned().unwrap_or(Value::Null);
            let to_agent = s(&to, "agent").filter(|a| a.chars().count() <= 200).map(|a| here(&a));
            let row = obj(vec![
                ("id", json!(id)),
                ("host", json!(from)),
                ("maker", json!(maker)),
                ("to_agent", json!(to_agent)),
                ("to_repo", json!(s(&to, "projectKey").map(|k| clip(&k, 300)))),
                ("to_scope", Value::Null),
                ("to_host", json!(s(&to, "host").filter(|h| xenonet_model::ids::is_host_id(h)))),
                ("goal", json!(text(&goal))),
                ("done", json!(text(&done))),
                ("next", json!(text(&next))),
                ("files", json!(Value::Array(files).to_string())),
                ("branch", json!(field("branch", 200))),
                ("commit_sha", json!(commit)),
                ("created_at", number(now)),
                ("expires_at", number(now + HANDOFF_MS)),
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
            if !is_remote(&by) || host_of_agent(w, &by).as_deref() != Some(from) {
                return Err(fail("an accept from a session that isn't on the host that sent it"));
            }
            if r.text("to_agent").is_some_and(|a| a != by) {
                return Err(fail("an accept from a session it wasn't handed to"));
            }
            if store(w, |s| s.take_handoff(&id, &by, now))? {
                let maker = r.text("maker").unwrap_or("");
                note(w, &by, maker, &format!("I took your handoff {id}."), Some(from));
                send_handoff(w, None, body_of(&r, "taken", Some(&by)));
            } else if let Some(now_row) = store(w, |s| s.handoff(&id))? {
                // Taken already: the asking host hears by whom.
                let taker = now_row.text("taken_by").unwrap_or("");
                let state = now_row.text("state").unwrap_or("taken").to_string();
                send_handoff(w, Some(from), body_of(&now_row, if state == "lapsed" { "lapsed" } else { "taken" }, Some(taker)));
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
                let maker = r.text("maker").unwrap_or("").to_string();
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
        _ => return Err(fail(format!("a handoff op XenoNet doesn't know: {op}"))),
    }
    Ok(())
}

/// Handoffs made here that no one took in a week lapse, and their makers
/// are told.
fn lapse(w: &Runtime) {
    let now = now_ms();
    let Ok(rows) = store(w, |s| s.lapsing_handoffs(now)) else { return };
    for r in rows {
        let id = r.text("id").unwrap_or("").to_string();
        let _ = store(w, |s| s.settle_handoff(&id, "lapsed", None, now));
        if r.text("host").is_none() {
            let maker = r.text("maker").unwrap_or("");
            note(w, maker, maker, &format!("No one took your handoff {id} in a week, so it lapsed."), None);
            if net(w).me().is_some() {
                send_handoff(w, None, body_of(&r, "lapsed", None));
            }
        }
    }
}

/// A message another host stored but no one there read by the end of the
/// outbox's week is unread: its sender sees that, not "stored" for good.
fn unread(w: &Runtime) {
    unread_at(w, now_ms());
}

/// `unread` as of `now`, which a test can move a week on.
pub fn unread_at(w: &Runtime, now: f64) {
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
    fn an_envelope_says_what_this_machine_decided_it_rests_on() {
        let env = envelope("laptop", "msg_1", 1.0, "agt_abcdefghijklmnopqrstuvwxyz234567", "agent", Some("Docs"), 2.0);
        assert_eq!(env["auth"], json!({ "method": "host", "process": "n/a", "attestation": "process-checked" }));
        assert_eq!(env["original"], json!({ "kind": "agent", "authority": "machine" }));
        assert_eq!(env["transport"], "network");
        assert!(env["line"].as_str().unwrap().contains("on laptop, another machine in your XenoNet network"));
        let you = envelope("laptop", "msg_2", 1.0, YOU, "person", None, 0.0);
        assert_eq!((you["sender"]["kind"].clone(), you["original"]["authority"].clone()), (json!("you"), json!("person")));
        assert!(you["line"].as_str().unwrap().starts_with("verified: you, by your device's signature"));
        assert!(own_id("agt_abcdefghijklmnopqrstuvwxyz234567") && !own_id("claude:abcdefgh") && !own_id("@vm1:claude:abcdefgh") && !own_id(YOU) && !own_id("hello"));
    }
}
