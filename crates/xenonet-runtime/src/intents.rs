//! Intents (docs/design/goals.md, "intent to an outcome"): the person
//! makes one at a device, naming the machine to work it, the project and
//! app, and the delegation of theirs it runs under; that machine works it
//! with a provider session and reports what came of it. Every entry is a
//! kept entry of its author's feed (`network/intent/v1`), so each member
//! holds every intent of the network, in order, and each is applied once.
//!
//! - **intent.make** (an `intent` entry, op make): the person's device
//!   signs `Act::Intend` (the goal's hash, the project, app, delegation,
//!   until, network and a random nonce, on the machine it names); the id is
//!   that act's hash, so a repeat is the same intent. Only the window signs
//!   one without presence: from anywhere else it needs the person's
//!   presence, which XenoNet asks only at its window. A receiver takes it
//!   only from the feed of the machine whose device signed it.
//! - **intent.assign** (an `assign` entry): the maker's machine gives it to
//!   the machine it names. Taken only from the maker's machine.
//! - **The target** decides `Act::Intend` (xenonet-authz) under the
//!   delegation the intent names, from the maker's device, then starts a
//!   provider session in a checkout of the project the person confirmed
//!   there, delegated from the maker: its permission prompts are judged
//!   under that delegation alone (`runs.intent`, kept across restarts).
//!   One held on stale governance is decided again on the next tick.
//! - **Reports** (`report` entries): start (with the agent working it), a
//!   note, done (or failed) with its outcome, from the target; from the
//!   session working it or an actor whose grant names it through the MCP
//!   tools `intent_note` and `intent_done`, decided here first.
//! - **intent.cancel** (an `intent` entry, op cancel) is the person's,
//!   signed by their device, from any of their machines' windows; it stops
//!   the run. Revoking the delegation stops what wasn't started yet, and
//!   the Allows of its run, from the moment this machine holds it.
//! - An open intent past its `until` expires here, and its run stops.
//!
//! An intent's goal, notes and outcome are information: shown and stored,
//! never decided on.

use crate::ground::{self, Carried};
use crate::network::{net, send};
use crate::runtime::Runtime;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use xenonet_api::cores::{CoreError, Reporter, R};
use xenonet_authz::{Act, AppId, DelegationId, Information, IntentId, IntentStep, MachineId, NetworkId, Principal, ProjectId, Resource};
use xenonet_id::codec::{b64, random, sha256, unb64_n};
use xenonet_model::ops::{Kind, To};
use xenonet_os::sync::Locked;
use xenonet_store::intents::IntentRow;
use xenonet_store::Db;
use xenonet_text::time::now_ms;

/// The body schema of every intent entry, told apart by its op.
pub const SCHEMA: &str = "network/intent/v1";
/// The most characters of a goal, a note and an outcome.
pub const GOAL_MAX: usize = 4000;
const NOTE_MAX: usize = 2000;
const OUTCOME_MAX: usize = 4000;
/// How long an intent holds when the person doesn't say, and at most.
const DEFAULT_HOURS: f64 = 24.0;
const MAX_HOURS: f64 = 30.0 * 24.0;
const HOUR_MS: f64 = 3600e3;

/// The intents being started here now, so two ticks never start one twice.
#[derive(Default)]
pub struct IntentSide {
    working: Mutex<HashSet<String>>,
}

/// What a run started for an intent carries: the intent, the delegation it
/// names and the machine it was made on (None for this one).
#[derive(Clone, Debug)]
pub struct Intended {
    pub id: String,
    pub delegation: String,
    pub from: Option<String>,
}

fn status(code: u16, message: impl Into<String>) -> CoreError {
    CoreError::status(code, message)
}

fn db(w: &Runtime) -> Result<Arc<Db>, String> {
    w.setup.db.clone().ok_or_else(|| "this machine has no store".to_string())
}

/// This machine's host id in its network.
fn me(w: &Runtime) -> Option<String> {
    net(w).me().map(|m| m.host)
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// An intent as the API and the page show it.
pub fn json_of(i: &IntentRow, here: Option<&str>) -> Value {
    json!({
        "id": i.id,
        "network": i.network,
        "maker": i.machine,
        "target": i.target,
        "here": here == Some(i.target.as_str()),
        "project": i.project,
        "app": i.app,
        "goal": i.goal,
        "delegation": i.delegation,
        "until": i.until,
        "state": i.state,
        "agent": i.agent,
        "note": i.note,
        "outcome": i.outcome,
        "madeAt": i.made_at,
        "endedAt": i.ended_at,
    })
}

/// The act that makes an intent, and the resource it's done to: what the
/// person's device signs, and whose hash is its id.
pub fn intend_of(i: &IntentRow) -> Result<(Resource, Act), String> {
    let e = |e: xenonet_authz::AuthzError| e.message;
    let nonce = unb64_n::<16>(&i.nonce).ok_or("an intent's nonce isn't 16 bytes")?;
    let act = Act::Intend {
        goal: sha256(i.goal.as_bytes()),
        project: ProjectId::new(&i.project).map_err(e)?,
        app: AppId::new(&i.app).map_err(e)?,
        delegation: DelegationId::new(&i.delegation).map_err(e)?,
        until: i.until,
        network: NetworkId::new(&i.network).map_err(e)?,
        nonce,
    };
    Ok((Resource::Intent { machine: MachineId::new(&i.target).map_err(e)?, id: None }, act))
}

/// An intent's id: its act's hash.
pub fn id_of(resource: &Resource, act: &Act) -> String {
    xenonet_model::ids::IntentId::of_act(&xenonet_authz::act_hash(resource, act)).as_str().to_string()
}

/// A report's act on the intent `i`, and what it's done to.
fn report_of(i: &IntentRow, step: IntentStep) -> Result<(Resource, Act), String> {
    let id = IntentId::new(&i.id).map_err(|e| e.message)?;
    let target = MachineId::new(&i.target).map_err(|e| e.message)?;
    Ok((Resource::Intent { machine: target, id: Some(id.clone()) }, Act::Report { intent: id, step }))
}

/// The person's authority for an act on an intent: their device's
/// signature, at the window. Anywhere else it would need their presence,
/// which this machine asks for only at its window, so it's refused.
fn signed(w: &Runtime, at_window: bool, resource: &Resource, act: &Act, reason: &str) -> Result<Value, CoreError> {
    let gov = ground::governance(w).ok_or_else(|| status(409, "this machine's governance isn't plugged in, so it makes no intents"))?;
    let identity = gov.identity();
    if !at_window {
        return match identity.prove(false, "intend", &xenonet_authz::act_hash(resource, act), reason, gov.facts().now) {
            Err(e) => Err(status(403, format!("an intent is signed as you: in XenoNet's window your device signs it, and anywhere else it needs your presence ({})", e.message))),
            Ok(_) => Err(status(403, "an intent is signed as you, in XenoNet's window")),
        };
    }
    ground::authority(&identity, true, resource, act, reason, gov.facts().now).map_err(|why| status(403, why))
}

// ---------- making, cancelling, listing ----------

/// The delegation of the person's an intent made here for `target` runs
/// under: `named`, or the first unended one for starts naming this
/// machine's device and `target`'s.
fn pick_delegation(w: &Runtime, target: &str, named: Option<&str>) -> Result<String, CoreError> {
    let gov = ground::governance(w).ok_or_else(|| status(409, "this machine's governance isn't plugged in"))?;
    let facts = gov.facts();
    let identity = gov.identity();
    let here = identity.device_id().ok_or_else(|| status(409, "this machine has no device of yours, so it makes no intents"))?;
    let theirs = MachineId::new(target).ok().and_then(|m| xenonet_authz::Ground::device_of(&facts, &m)).map(|d| d.as_str().to_string());
    let chain = identity.chain();
    let person = chain.person().ok_or_else(|| status(409, "this machine has no identity of yours yet: make or join a network from XenoNet's window"))?;
    let found = person
        .delegations
        .values()
        .map(|(d, _)| d)
        .find(|d| named.is_none_or(|n| d.id == n) && d.until > facts.now && d.covers("start") && d.devices.has(&here) && theirs.as_ref().is_some_and(|t| d.devices.has(t)));
    match (found, named) {
        (Some(d), _) => Ok(d.id.clone()),
        (None, Some(n)) => Err(status(409, format!("your delegation {n} doesn't name this machine's device and {target}'s for starts, or it ended"))),
        (None, None) => {
            Err(status(409, format!("no delegation of yours names this machine's device and {target}'s for starts: set {target} to Delegated in Machines, which issues one")))
        }
    }
}

/// The person's new intent, made here: kept, told to the network as an
/// entry of this machine's feed, and given to the machine it names.
pub fn make(w: &Runtime, body: &Value, at_window: bool) -> R<Value> {
    let gov = ground::governance(w).ok_or_else(|| status(409, "this machine's governance isn't plugged in, so it makes no intents"))?;
    let facts = gov.facts();
    let (Some(here), Some(network)) = (me(w), facts.network()) else {
        return Err(status(409, "an intent travels in your network: make or join one first"));
    };
    let goal = text(body, "goal").ok_or_else(|| status(400, "say what the intent is for: its goal"))?;
    if goal.chars().count() > GOAL_MAX {
        return Err(status(400, format!("a goal is at most {GOAL_MAX} characters")));
    }
    let target = text(body, "target").unwrap_or_else(|| here.clone());
    let known = target == here || facts.view.as_ref().is_some_and(|v| v.member(&target).is_some());
    if !known {
        return Err(status(400, format!("{target} isn't a machine in your network")));
    }
    let project = text(body, "project").ok_or_else(|| status(400, "say which project it's worked in: a prj_ id"))?;
    xenonet_model::ids::ProjectId::parse(&project).map_err(|_| status(400, format!("{project} isn't a project's id")))?;
    // The app as a start takes it: the registry's first when none is named.
    let app = xenonet_engine::delivery::starts::request(&json!({ "app": text(body, "app"), "prompt": goal })).map_err(|e| status(400, e))?.app;
    let hours = body.get("hours").and_then(Value::as_f64).unwrap_or(DEFAULT_HOURS);
    if !(hours > 0.0 && hours <= MAX_HOURS) {
        return Err(status(400, format!("an intent holds for more than no time and at most {MAX_HOURS} hours")));
    }
    let delegation = pick_delegation(w, &target, text(body, "delegation").as_deref())?;
    let now = facts.now;
    let nonce = random::<16>().map_err(|e| status(500, e.message))?;
    let mut row = IntentRow {
        id: String::new(),
        network: network.as_str().to_string(),
        person: gov.identity().person_id().unwrap_or_default(),
        device: String::new(),
        machine: here.clone(),
        delegation,
        project,
        app,
        target: target.clone(),
        goal,
        until: now + (hours * HOUR_MS) as i64,
        state: "made".into(),
        agent: None,
        note: None,
        outcome: None,
        nonce: b64(&nonce),
        authority: String::new(),
        made_at: now,
        ended_at: None,
    };
    let (resource, act) = intend_of(&row).map_err(|e| status(400, e))?;
    let authority = signed(w, at_window, &resource, &act, &format!("ask {target} to work an intent of yours"))?;
    row.id = id_of(&resource, &act);
    row.device = authority["person"]["device"].as_str().unwrap_or_default().to_string();
    row.authority = authority.to_string();
    let db = db(w).map_err(|e| status(500, e))?;
    db.put_intent(&row).map_err(|e| status(500, e.message))?;
    send(w, To::All, Kind::Intent, make_body(&row, &authority));
    // The maker's machine gives it to the machine it names.
    if target != here {
        send(w, To::All, Kind::Assign, json!({ "schema": SCHEMA, "op": "assign", "intent": row.id, "target": target, "until": row.until }));
    }
    let _ = db.move_intent(&row.id, &["made"], "assigned", now);
    w.log("info", "intent", &format!("you made {} for {target}", row.id));
    if target == here {
        work_soon(w, &row.id);
    }
    changed(w);
    let kept = db.intent(&row.id).map_err(|e| status(500, e.message))?.unwrap_or(row);
    Ok(json_of(&kept, Some(&here)))
}

/// intent.make's body: the intent and the person's signature over it.
pub fn make_body(i: &IntentRow, authority: &Value) -> Value {
    json!({
        "schema": SCHEMA, "op": "make", "id": i.id, "goal": i.goal, "project": i.project, "app": i.app,
        "target": i.target, "delegation": i.delegation, "until": i.until, "network": i.network,
        "nonce": i.nonce, "at": i.made_at, "authority": authority,
    })
}

/// The person's cancel of an intent, from this machine's window: kept,
/// told to the network, and the run stopped when it's worked here.
pub fn cancel(w: &Runtime, id: &str, at_window: bool) -> R<Value> {
    let db = db(w).map_err(|e| status(500, e))?;
    let i = db.intent(id).map_err(|e| status(500, e.message))?.ok_or_else(|| status(404, format!("this machine holds no intent {id}")))?;
    if !i.open() {
        return Err(status(409, format!("{id} has ended: it's {}", i.state)));
    }
    let (resource, act) = report_of(&i, IntentStep::Cancel).map_err(|e| status(400, e))?;
    let authority = signed(w, at_window, &resource, &act, &format!("cancel an intent of yours on {}", i.target))?;
    let now = now_ms() as i64;
    db.move_intent(id, &["made", "assigned", "running"], "cancelled", now).map_err(|e| status(500, e.message))?;
    send(w, To::All, Kind::Intent, json!({ "schema": SCHEMA, "op": "cancel", "intent": id, "until": i.until, "authority": authority }));
    if me(w).as_deref() == Some(i.target.as_str()) {
        stop_run(w, id);
    }
    w.log("info", "intent", &format!("you cancelled {id}"));
    changed(w);
    let kept = db.intent(id).map_err(|e| status(500, e.message))?.unwrap_or(i);
    Ok(json_of(&kept, me(w).as_deref()))
}

/// The intents this machine keeps, and what a new one may name: the
/// machines (this one first), the person's delegations for starts, and
/// the projects with a checkout here they confirmed.
pub fn list(w: &Runtime) -> R<Value> {
    let db = db(w).map_err(|e| status(500, e))?;
    let here = me(w);
    let intents: Vec<Value> = db.intents().map_err(|e| status(500, e.message))?.iter().map(|i| json_of(i, here.as_deref())).collect();
    let gov = ground::governance(w);
    let mut machines: Vec<String> = here.iter().cloned().collect();
    if let Some(v) = gov.as_ref().and_then(|g| g.facts().view) {
        machines.extend(v.members.keys().filter(|h| Some(*h) != here.as_ref()).cloned());
    }
    let now = now_ms() as i64;
    let delegations: Vec<Value> = gov
        .as_ref()
        .and_then(|g| g.identity().chain().person().map(|p| p.delegations.values().map(|(d, _)| d.clone()).collect::<Vec<_>>()))
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.until > now && d.covers("start"))
        .map(|d| json!({ "id": d.id, "until": d.until, "devices": match &d.devices { xenonet_net::identity::person::DeviceSet::All => json!("all"), xenonet_net::identity::person::DeviceSet::Only(l) => json!(l) } }))
        .collect();
    let projects: Vec<Value> =
        db.confirmed_projects(None).unwrap_or_default().iter().map(|p| json!({ "id": p.as_str(), "name": db.project(p).ok().flatten().map(|x| x.name) })).collect();
    Ok(json!({ "here": here, "intents": intents, "machines": machines, "delegations": delegations, "projects": projects }))
}

/// The page hears that the intents changed.
fn changed(w: &Runtime) {
    w.emit(xenonet_api::From::Engine, "intents", json!({ "at": now_ms() }));
}

// ---------- from other machines ----------

/// An `intent` entry of `host`'s feed: an intent made there, or the
/// person's cancel of one.
pub(crate) fn intent_in(w: &Runtime, host: &str, body: &Value) -> Result<(), String> {
    match body.get("op").and_then(Value::as_str) {
        Some("make") => make_in(w, host, body),
        Some("cancel") => cancel_in(w, host, body),
        _ => Err("an intent entry is a make or a cancel".into()),
    }
}

/// An intent made on `host`: taken when the person's device on `host`
/// signed it, for a machine of the network, before it ends; kept once.
fn make_in(w: &Runtime, host: &str, b: &Value) -> Result<(), String> {
    let field = |k: &str| text(b, k).ok_or_else(|| format!("an intent without its {k}"));
    let mut row = IntentRow {
        id: field("id")?,
        network: field("network")?,
        person: String::new(),
        device: String::new(),
        machine: host.to_string(),
        delegation: field("delegation")?,
        project: field("project")?,
        app: field("app")?,
        target: field("target")?,
        goal: field("goal")?,
        until: b.get("until").and_then(Value::as_i64).ok_or("an intent without its until")?,
        state: "made".into(),
        agent: None,
        note: None,
        outcome: None,
        nonce: field("nonce")?,
        authority: String::new(),
        made_at: b.get("at").and_then(Value::as_i64).unwrap_or(0),
        ended_at: None,
    };
    if row.goal.chars().count() > GOAL_MAX {
        return Err("a goal longer than an intent's".into());
    }
    let (resource, act) = intend_of(&row)?;
    if id_of(&resource, &act) != row.id {
        return Err("an intent whose id isn't its act's hash".into());
    }
    let db = db(w)?;
    if db.intent(&row.id).map_err(|e| e.message)?.is_some() {
        return Ok(());
    }
    let g = ground::ground(w, None).ok_or("this machine's governance isn't plugged in")?;
    if g.facts.network().is_none_or(|n| n.as_str() != row.network) {
        return Err(format!("{} travels in another network", row.id));
    }
    // Only from the feed of the machine whose device signed it.
    let authority = b.get("authority").cloned().unwrap_or_else(|| json!({}));
    let carried = Carried::parse(&authority);
    let device = carried.person.as_ref().map(|(_, d, _)| d.clone()).ok_or("an intent comes with the person's device's signature")?;
    let on_host = g.facts.view.as_ref().and_then(|v| v.member(host)).and_then(|m| m.device.clone());
    if on_host.as_deref() != Some(device.as_str()) {
        return Err(format!("{} was signed by a device that isn't {host}'s, so {host} doesn't speak for it", row.id));
    }
    let known = g.facts.view.as_ref().is_some_and(|v| v.member(&row.target).is_some());
    if !known {
        return Err(format!("{} names {}, which isn't in the network", row.id, row.target));
    }
    let Principal::Person(signed) = ground::person(w, &g, Some(host), &carried, &resource, &act)? else {
        return Err("an intent is the person's".into());
    };
    row.person = signed.person().as_str().to_string();
    row.device = device;
    row.authority = authority.to_string();
    if row.until < g.facts.now {
        w.log("info", "intent", &format!("{} from {host} had ended before it came", row.id));
        return Ok(());
    }
    db.put_intent(&row).map_err(|e| e.message)?;
    changed(w);
    Ok(())
}

/// The maker's machine gave an intent to the machine it names: taken only
/// from the maker's machine; worked when it names this one.
pub(crate) fn assign_in(w: &Runtime, host: &str, b: &Value) -> Result<(), String> {
    let id = text(b, "intent").ok_or("an assign without its intent")?;
    let db = db(w)?;
    let i = db.intent(&id).map_err(|e| e.message)?.ok_or_else(|| format!("an assign of {id}, which this machine doesn't hold"))?;
    if i.machine != host {
        return Err(format!("an assign of {id} from {host}: only {}, where it was made, assigns it", i.machine));
    }
    if text(b, "target").as_deref() != Some(i.target.as_str()) {
        return Err(format!("an assign of {id} to another machine than it names"));
    }
    db.move_intent(&id, &["made"], "assigned", now_ms() as i64).map_err(|e| e.message)?;
    changed(w);
    if me(w).as_deref() == Some(i.target.as_str()) {
        work_soon(w, &id);
    }
    Ok(())
}

/// The person's cancel, signed by their device, from any of their
/// machines: decided here, kept, and the run stopped when it's worked
/// here.
fn cancel_in(w: &Runtime, host: &str, b: &Value) -> Result<(), String> {
    let id = text(b, "intent").ok_or("a cancel without its intent")?;
    let db = db(w)?;
    let Some(i) = db.intent(&id).map_err(|e| e.message)? else { return Ok(()) };
    if !i.open() {
        return Ok(());
    }
    let (resource, act) = report_of(&i, IntentStep::Cancel)?;
    let g = ground::ground(w, None).ok_or("this machine's governance isn't plugged in")?;
    let carried = Carried::parse(b.get("authority").unwrap_or(&Value::Null));
    let p = ground::person(w, &g, Some(host), &carried, &resource, &act)?;
    ground::judge(&g, &ground::remote(&g, p, resource, act), None)?;
    db.move_intent(&id, &["made", "assigned", "running"], "cancelled", now_ms() as i64).map_err(|e| e.message)?;
    if me(w).as_deref() == Some(i.target.as_str()) {
        stop_run(w, &id);
    }
    w.log("info", "intent", &format!("you cancelled {id} at {host}"));
    changed(w);
    Ok(())
}

/// A report on an intent from the machine it's worked on.
pub(crate) fn report_in(w: &Runtime, host: &str, b: &Value) -> Result<(), String> {
    let id = text(b, "intent").ok_or("a report without its intent")?;
    let step = b.get("op").and_then(Value::as_str).and_then(IntentStep::parse).filter(|s| *s != IntentStep::Cancel).ok_or("a report is a start, a note or done")?;
    let db = db(w)?;
    let Some(i) = db.intent(&id).map_err(|e| e.message)? else { return Err(format!("a report on {id}, which this machine doesn't hold")) };
    let (resource, act) = report_of(&i, step)?;
    let g = ground::ground(w, None).ok_or("this machine's governance isn't plugged in")?;
    ground::judge(&g, &ground::remote(&g, Principal::Machine { id: ground::machine(host)? }, resource, act), None)?;
    took(&db, &i, step, b)?;
    changed(w);
    Ok(())
}

/// What a report says, kept: its run started (and its agent), a note, or
/// done with its outcome.
fn took(db: &Db, i: &IntentRow, step: IntentStep, b: &Value) -> Result<(), String> {
    let now = now_ms() as i64;
    let e = |e: xenonet_store::StoreError| e.message;
    match step {
        IntentStep::Start => {
            // The target moved it to running before its run started; a
            // receiver moves it now. Either way it names its agent, unless
            // it ended meanwhile.
            db.move_intent(&i.id, &["made", "assigned"], "running", now).map_err(e)?;
            let running = db.intent(&i.id).map_err(e)?.is_some_and(|x| x.state == "running");
            if let (true, Some(a)) = (running, text(b, "agent")) {
                db.set_intent_agent(&i.id, &a).map_err(e)?;
            }
        }
        IntentStep::Note => {
            if let Some(t) = text(b, "text") {
                db.set_intent_note(&i.id, &clip(&t, NOTE_MAX)).map_err(e)?;
            }
        }
        IntentStep::Done => {
            let to = if b.get("state").and_then(Value::as_str) == Some("failed") { "failed" } else { "done" };
            if db.move_intent(&i.id, &["made", "assigned", "running"], to, now).map_err(e)? {
                if let Some(o) = text(b, "outcome") {
                    db.set_intent_outcome(&i.id, &clip(&o, OUTCOME_MAX)).map_err(e)?;
                }
            }
        }
        IntentStep::Cancel => {}
    }
    Ok(())
}

// ---------- working an intent here ----------

/// Works the intent `id` on a blocking thread, soon: what an assign does,
/// outside the transaction that applied it.
fn work_soon(w: &Runtime, id: &str) {
    let id = id.to_string();
    w.spawn_blocking(move |w| work(w, &id));
}

/// Works an intent given to this machine: decided under the delegation it
/// names, then a provider session started in a checkout of its project
/// the person confirmed here, delegated from the maker. A decision held
/// on stale governance leaves it assigned, for the next tick.
pub fn work(w: &Runtime, id: &str) {
    if !w.intents.working.locked().insert(id.to_string()) {
        return;
    }
    let outcome = work_now(w, id);
    w.intents.working.locked().remove(id);
    match outcome {
        Ok(()) => {}
        Err(Worked::Held(why)) => w.log("info", "intent", &format!("{id} waits: {why}")),
        Err(Worked::Failed(why)) => {
            w.log("warn", "intent", &format!("{id} wasn't started: {why}"));
            report_here(w, id, IntentStep::Done, &json!({ "state": "failed", "outcome": why }));
        }
    }
}

enum Worked {
    Held(String),
    Failed(String),
}

fn work_now(w: &Runtime, id: &str) -> Result<(), Worked> {
    let failed = Worked::Failed;
    let db = db(w).map_err(failed)?;
    let Some(i) = db.intent(id).map_err(|e| Worked::Failed(e.message))? else { return Ok(()) };
    let Some(here) = me(w) else { return Ok(()) };
    if i.state != "assigned" || i.target != here {
        return Ok(());
    }
    // Decided under the delegation it names, from the maker's device.
    let g = ground::ground(w, None).ok_or_else(|| Worked::Held("this machine's governance isn't plugged in".into()))?;
    let (resource, act) = intend_of(&i).map_err(failed)?;
    let carried = Carried::parse(&serde_json::from_str(&i.authority).unwrap_or(Value::Null));
    let from = (i.machine != here).then_some(i.machine.as_str());
    let p = ground::person(w, &g, from, &carried, &resource, &act).map_err(failed)?;
    let request = if from.is_some() { ground::remote(&g, p, resource, act) } else { ground::local(&g, p, resource, act) };
    match ground::decide_with(&g, &request, None) {
        xenonet_authz::Decision::Allow(_) => {}
        d @ xenonet_authz::Decision::Hold(_) => return Err(Worked::Held(ground::why(&d))),
        d @ xenonet_authz::Decision::Refuse(_) => return Err(Worked::Failed(format!("{here} refused it: {}", ground::why(&d)))),
    }
    // A checkout of its project the person confirmed here.
    let project = xenonet_model::ids::ProjectId::parse(&i.project).map_err(|_| Worked::Failed(format!("{} isn't a project's id", i.project)))?;
    let project = db.resolve(&project).ok().flatten().unwrap_or(project);
    let folder = db
        .checkouts()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.machine == xenonet_store::projects::LOCAL && c.project == project && c.confirmed && c.gone_at.is_none())
        .find_map(|c| c.path)
        .ok_or_else(|| Worked::Failed(format!("{here} has no checkout of {} you confirmed, so it starts nothing there", i.project)))?;
    if !db.move_intent(id, &["assigned"], "running", now_ms() as i64).map_err(|e| Worked::Failed(e.message))? {
        return Ok(());
    }
    let request = xenonet_engine::delivery::starts::Request { folder, app: i.app.clone(), mode: None, prompt: String::new(), delegate: true };
    let start = crate::starts::Apart {
        request,
        prompt: Information::new(i.goal.clone()),
        intent: Some(Intended { id: i.id.clone(), delegation: i.delegation.clone(), from: from.map(str::to_string) }),
    };
    let by = json!({ "kind": "you", "id": xenonet_model::ids::YOU, "title": "you" });
    let key = i.id.clone();
    let answer: crate::starts::Answer = Arc::new(move |w: &Runtime, state: &str, a: Value| ran(w, &key, state, &a));
    match crate::starts::run_here(w, &by, from, crate::starts::Kind::You, xenonet_model::ids::YOU, &start, answer) {
        Ok(v) => {
            let agent = v.get("agent").and_then(Value::as_str).map(str::to_string);
            w.log("info", "intent", &format!("{id} runs here, for {}", from.unwrap_or("this machine")));
            report_here(w, id, IntentStep::Start, &json!({ "agent": agent }));
            Ok(())
        }
        Err(why) => Err(Worked::Failed(why)),
    }
}

/// What the run started for an intent came to, when it ended before its
/// session said: done with its last words, or failed.
fn ran(w: &Runtime, id: &str, state: &str, a: &Value) {
    if state != "ended" {
        return;
    }
    let Ok(Some(i)) = db(w).and_then(|d| d.intent(id).map_err(|e| e.message)) else { return };
    if i.state != "running" {
        return;
    }
    let kind = a.get("outcome").and_then(Value::as_str).unwrap_or("silent");
    let words = text(a, "last").or_else(|| text(a, "reason")).unwrap_or_else(|| format!("its session ended ({kind}) without saying what came of it"));
    let state = if kind == "failed" { "failed" } else { "done" };
    report_here(w, id, IntentStep::Done, &json!({ "state": state, "outcome": words }));
}

/// A report on an intent worked here, kept and told to the network.
fn report_here(w: &Runtime, id: &str, step: IntentStep, body: &Value) {
    let Ok(db) = db(w) else { return };
    let Ok(Some(i)) = db.intent(id) else { return };
    if let Err(why) = took(&db, &i, step, body) {
        w.log("warn", "intent", &format!("a report on {id} wasn't kept: {why}"));
        return;
    }
    let mut b = json!({ "schema": SCHEMA, "op": step.as_str(), "intent": id });
    for k in ["agent", "text", "state", "outcome"] {
        if let Some(v) = body.get(k).filter(|v| !v.is_null()) {
            b[k] = v.clone();
        }
    }
    if let Some(o) = b.get("outcome").and_then(Value::as_str).map(|o| clip(o, OUTCOME_MAX)) {
        b["outcome"] = json!(o);
    }
    if let Some(t) = b.get("text").and_then(Value::as_str).map(|t| clip(t, NOTE_MAX)) {
        b["text"] = json!(t);
    }
    if net(w).me().is_some() {
        send(w, To::All, Kind::Report, b);
    }
    changed(w);
}

/// Stops the run started for an intent, when one runs here.
fn stop_run(w: &Runtime, id: &str) {
    let Ok(Some(session)) = db(w).and_then(|d| d.intent_run(id).map_err(|e| e.message)) else { return };
    if let Some(r) = w.router.resume.as_ref() {
        if r.runner.stop(&session) {
            w.log("info", "intent", &format!("{id}'s run, {session}, was stopped"));
        }
    }
}

/// The intent a session started here works, and the delegation it names.
pub fn of_session(w: &Runtime, session: &str) -> Option<(String, String)> {
    w.store.0.db().delegated(session).ok().flatten().and_then(|d| d.intent)
}

// ---------- reports through this machine's door ----------

/// A note or the outcome of an intent worked here, from its session (the
/// MCP tools) or an actor whose grant names it: decided here, then kept
/// and told to the network.
pub fn report(w: &Runtime, by: Reporter, id: &str, step: &str, body: &Value) -> R<Value> {
    let step = match step {
        "note" => IntentStep::Note,
        "done" => IntentStep::Done,
        other => return Err(status(404, format!("{other} isn't a report on an intent: note or done"))),
    };
    let db = db(w).map_err(|e| status(500, e))?;
    let i = db.intent(id).map_err(|e| status(500, e.message))?.ok_or_else(|| status(404, format!("this machine holds no intent {id}")))?;
    let here = me(w).unwrap_or_default();
    if i.target != here {
        return Err(status(409, format!("{id} is worked on {}: its reports are made there", i.target)));
    }
    let g = ground::ground(w, None).ok_or_else(|| status(409, "this machine's governance isn't plugged in"))?;
    let (resource, act) = report_of(&i, step).map_err(|e| status(400, e))?;
    let (principal, token) = match by {
        Reporter::Session { id: session, attestation } => {
            // The session's agent, as this machine knows it: none for a
            // session it hasn't seen, which isn't the one working it.
            let agent = w.store.0.db().agent_known(&session).ok().flatten().ok_or_else(|| status(403, format!("{session} isn't the session working {id}")))?;
            let attestation = match attestation {
                xenonet_api::cores::door::Attestation::ProcessChecked => xenonet_authz::Attestation::ProcessChecked,
                xenonet_api::cores::door::Attestation::Unchecked => xenonet_authz::Attestation::Unchecked,
            };
            let agent = xenonet_authz::AgentId::new(agent.as_str()).map_err(|e| status(400, e.message))?;
            (Principal::Agent { id: agent, machine: g.facts.machine.clone(), attestation }, None)
        }
        Reporter::Actor { token } => {
            let t = xenonet_authz::Token::parse(&token, xenonet_authz::ACTOR_PREFIX, &g, g.facts.now).map_err(|e| status(403, e.message))?;
            let b = t.bearer().clone();
            (Principal::Actor { slug: b.slug, machine: b.machine }, Some(t))
        }
    };
    ground::judge_by(&g, &ground::local(&g, principal, resource, act), token.as_ref(), None).map_err(|why| status(403, why))?;
    let said = match step {
        IntentStep::Note => {
            let t = text(body, "text").ok_or_else(|| status(400, "a note says something: text"))?;
            json!({ "text": clip(&t, NOTE_MAX) })
        }
        _ => {
            let o = text(body, "outcome").ok_or_else(|| status(400, "say what came of it: outcome"))?;
            let failed = body.get("failed").and_then(Value::as_bool) == Some(true);
            json!({ "state": if failed { "failed" } else { "done" }, "outcome": clip(&o, OUTCOME_MAX) })
        }
    };
    report_here(w, id, step, &said);
    let kept = db.intent(id).map_err(|e| status(500, e.message))?.unwrap_or(i);
    Ok(json_of(&kept, Some(&here)))
}

// ---------- the timer ----------

/// On the network's tick: open intents past their time expire (a run here
/// stops), and intents given to this machine that weren't started yet (a
/// decision held, or a restart) are worked again.
pub fn tick(w: &Runtime) {
    let Ok(db) = db(w) else { return };
    let now = now_ms() as i64;
    let here = me(w);
    for i in db.intents_past(now).unwrap_or_default() {
        if db.move_intent(&i.id, &["made", "assigned", "running"], "expired", now).unwrap_or(false) {
            if here.as_deref() == Some(i.target.as_str()) {
                stop_run(w, &i.id);
            }
            w.log("info", "intent", &format!("{} expired", i.id));
            changed(w);
        }
    }
    let Some(here) = here else { return };
    for i in db.intents().unwrap_or_default().into_iter().filter(|i| i.state == "assigned" && i.target == here) {
        if !w.intents.working.locked().contains(&i.id) {
            work_soon(w, &i.id);
        }
    }
}

/// The first prompt of an intent's run: whose intent it is, made where,
/// under which delegation, the delegation's rules, the goal, and how to
/// report.
pub fn framed(i: &Intended, goal: &str) -> String {
    let at = i.from.as_deref().map_or_else(|| "this machine's console".to_string(), |h| format!("{h}'s console"));
    format!(
        "[XenoNet] This is an intent your user made at {at} ({}), which runs here on their standing delegation {}; XenoNet here checked it.\n\n{}\n\n{goal}\n\n[XenoNet] Say how it goes with the xenonet MCP tool intent_note, and when it's done (or can't be), call intent_done with intent {} and what came of it.",
        i.id,
        i.delegation,
        xenonet_engine::delivery::starts::DELEGATED_RULES,
        i.id
    )
}

/// The Intents core, for the routes.
pub struct IntentsDriver(pub Arc<Runtime>);

impl xenonet_api::cores::Intents for IntentsDriver {
    fn list(&self) -> R<Value> {
        list(&self.0)
    }
    fn make(&self, body: Value, at_window: bool) -> R<Value> {
        make(&self.0, &body, at_window)
    }
    fn cancel(&self, id: &str, at_window: bool) -> R<Value> {
        cancel(&self.0, id, at_window)
    }
    fn report(&self, by: Reporter, id: &str, step: &str, body: Value) -> R<Value> {
        report(&self.0, by, id, step, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> IntentRow {
        IntentRow {
            id: String::new(),
            network: "net_0123456789abcdef0123456789abcdef".into(),
            person: "prs_a".into(),
            device: "xns1_d".into(),
            machine: "laptop".into(),
            delegation: "dlg_0123456789abcdef".into(),
            project: "prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            app: "claude".into(),
            target: "desk".into(),
            goal: "fix the build".into(),
            until: 2_000,
            state: "made".into(),
            agent: None,
            note: None,
            outcome: None,
            nonce: b64(&[1; 16]),
            authority: "{}".into(),
            made_at: 1_000,
            ended_at: None,
        }
    }

    #[test]
    fn an_intents_id_is_its_acts_hash_and_every_field_is_in_it() {
        let (r, a) = intend_of(&row()).unwrap();
        let id = id_of(&r, &a);
        assert!(id.starts_with("int_"), "{id}");
        let changed: [fn(&mut IntentRow); 8] = [
            |i| i.goal.push('!'),
            |i| i.project = "prj_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            |i| i.app = "codex".into(),
            |i| i.delegation = "dlg_fedcba9876543210".into(),
            |i| i.until += 1,
            |i| i.network = "net_fedcba9876543210fedcba9876543210".into(),
            |i| i.nonce = b64(&[2; 16]),
            |i| i.target = "studio".into(),
        ];
        for change in changed {
            let mut i = row();
            change(&mut i);
            let (r, a) = intend_of(&i).unwrap();
            assert_ne!(id_of(&r, &a), id);
        }
        // What isn't signed doesn't change it: its state, its words after.
        let (r, a) = intend_of(&IntentRow { state: "running".into(), note: Some("x".into()), ..row() }).unwrap();
        assert_eq!(id_of(&r, &a), id);
    }

    #[test]
    fn an_intents_run_is_told_whose_it_is_and_how_to_report() {
        let i = Intended { id: "int_a".into(), delegation: "dlg_a".into(), from: Some("laptop".into()) };
        let t = framed(&i, "fix the build");
        assert!(t.starts_with("[XenoNet] This is an intent your user made at laptop's console (int_a), which runs here on their standing delegation dlg_a"), "{t}");
        assert!(t.contains("\n\nfix the build\n\n") && t.contains("intent_done with intent int_a"), "{t}");
    }
}
