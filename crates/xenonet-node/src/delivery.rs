//! xenonet-engine's live delivery for the twins of core/delivery/: the
//! selectors, plan usage, and the policy's guards and what it keeps. The
//! policy's clock, limits and files are called through the twin.

use crate::args::JsArg;
use crate::peer::{rethrow, settle, throw_with, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::cell::RefCell;
use xenonet_engine::delivery::policy::{self, Fail, Policy};
use xenonet_engine::delivery::selector::{self, Selector};
use xenonet_engine::delivery::usage;
use xenonet_engine::peer::{list, Thrown};
use xenonet_engine::sql::Arg;

// ----- selector -----

#[napi(js_name = "selParse")]
pub fn sel_parse(text: JsArg) -> JsArg {
    JsArg(selector::parse_selector(&text.0).map_or(Arg::Null, |s| s.to_arg()))
}

#[napi(js_name = "selIs")]
pub fn sel_is(text: JsArg) -> bool {
    selector::is_selector(&text.0)
}

#[napi(js_name = "selFormat")]
pub fn sel_format(sel: JsArg) -> String {
    selector::format_selector(Selector::of(&sel.0).filter(|_| !sel.0.field("terms").is_nullish()).as_ref())
}

#[napi(js_name = "selMatches")]
pub fn sel_matches(sel: JsArg, facts: JsArg) -> bool {
    selector::matches(Selector::of(&sel.0).as_ref(), &facts.0)
}

#[napi(js_name = "selWithin")]
pub fn sel_within(inner: JsArg, outer: JsArg) -> bool {
    match Selector::of(&inner.0) {
        Some(i) => selector::within(&i, Selector::of(&outer.0).as_ref()),
        None => false,
    }
}

#[napi(js_name = "planUsage")]
pub fn plan_usage(limits: JsArg, app: JsArg, now: f64) -> Option<f64> {
    usage::plan_usage(&limits.0, &app.0, now)
}

// ----- policy -----

#[napi(js_name = "policyLimits")]
pub fn policy_limits() -> crate::js::Json {
    crate::js::Json(policy::limits())
}

#[napi(js_name = "policyIsYou")]
pub fn policy_is_you(id: JsArg) -> bool {
    policy::is_you(&id.0)
}

/// { host, slug }, or null for an id that isn't an actor's.
#[napi(js_name = "policyActorOf")]
pub fn policy_actor_of(id: JsArg) -> JsArg {
    JsArg(policy::actor_of(&id.0).map_or(Arg::Null, |(host, slug)| Arg::Obj(vec![("host".into(), host.map_or(Arg::Null, Arg::Text)), ("slug".into(), Arg::Text(slug))])))
}

#[napi(js_name = "policyRedact")]
pub fn policy_redact(text: String) -> JsArg {
    let (t, n) = policy::redact_secrets(&text);
    JsArg(Arg::Obj(vec![("text".into(), Arg::text(t)), ("redacted".into(), Arg::num(n as f64))]))
}

#[napi(js_name = "policySanitize")]
pub fn policy_sanitize(text: String, one_line: bool, max: f64) -> String {
    policy::sanitize(&text, one_line, max)
}

#[napi(js_name = "policyFramed")]
pub fn policy_framed(id: JsArg, from: JsArg, verified: JsArg, text: JsArg, user: bool) -> String {
    policy::framed(&id.0, &from.0, &verified.0, &text.0, user)
}

#[napi(js_name = "policyHashText")]
pub fn policy_hash_text(text: String) -> String {
    policy::hash_text(&text)
}

#[napi(js_name = "policyTextFor")]
pub fn policy_text_for(text: JsArg, sender: JsArg, sender_facts: JsArg, recipient_facts: JsArg) -> JsArg {
    let (t, n) = policy::text_for(&text.0, &sender.0, &sender_facts.0, &recipient_facts.0);
    JsArg(Arg::Obj(vec![("text".into(), t), ("redacted".into(), Arg::num(n as f64))]))
}

/// A refusal thrown as the JS threw it: an Error with its code and status
/// 429; an Error with its sentence; or what a collaborator threw.
fn throw(env: Env, f: Fail) -> Error {
    match f {
        Fail::Thrown(t) => rethrow(&env, t),
        Fail::Error(message) => rethrow(&env, Thrown::new(message)),
        Fail::Refused { why, code } => {
            settle(&env);
            throw_with(&env, &why, &[(c"code", serde_json::json!(code)), (c"status", serde_json::json!(429))])
        }
    }
}

#[napi]
pub struct PolicyCore {
    inner: RefCell<Policy>,
}

fn busy() -> Error {
    Error::new(Status::GenericFailure, "the delivery policy was asked again while it was busy".to_string())
}

fn opt(a: JsArg) -> Option<Arg> {
    (!matches!(a.0, Arg::Undef)).then_some(a.0)
}

impl PolicyCore {
    fn run<T>(&self, env: Env, deps: &Object<'_>, f: impl FnOnce(&mut Policy, &mut SyncCalls) -> policy::Result<T>) -> Result<T> {
        let mut p = self.inner.try_borrow_mut().map_err(|_| busy())?;
        let mut c = SyncCalls::new(&env, deps);
        match f(&mut p, &mut c) {
            Ok(v) => {
                settle(&env);
                Ok(v)
            }
            Err(e) => Err(throw(env, e)),
        }
    }
}

#[napi]
impl PolicyCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        PolicyCore { inner: RefCell::new(Policy::default()) }
    }

    /// What the policy's file keeps, read.
    #[napi]
    pub fn load(&self, env: Env, deps: Object<'_>) -> Result<()> {
        self.run(env, &deps, |p, c| {
            *p = Policy::load(c)?;
            Ok(())
        })
    }

    #[napi]
    pub fn data(&self) -> Result<JsArg> {
        Ok(JsArg(self.inner.try_borrow().map_err(|_| busy())?.data_arg()))
    }
    #[napi(js_name = "memoryAudit")]
    pub fn memory_audit(&self) -> Result<JsArg> {
        Ok(JsArg(Arg::List(self.inner.try_borrow().map_err(|_| busy())?.memory_audit.clone())))
    }
    #[napi]
    pub fn paused(&self) -> Result<bool> {
        Ok(self.inner.try_borrow().map_err(|_| busy())?.paused())
    }
    #[napi(js_name = "setPaused")]
    pub fn set_paused(&self, env: Env, deps: Object<'_>, on: bool, by: JsArg) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.set_paused(c, on, &by.0)).map(JsArg)
    }
    #[napi]
    pub fn grant(&self, env: Env, deps: Object<'_>, actor: JsArg, selector: JsArg, ttl_ms: JsArg, by: JsArg) -> Result<JsArg> {
        let ttl = opt(ttl_ms);
        self.run(env, &deps, |p, c| p.grant(c, &actor.0, &selector.0, ttl.as_ref(), &by.0)).map(JsArg)
    }
    #[napi]
    pub fn revoke(&self, env: Env, deps: Object<'_>, actor: JsArg, by: JsArg) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.revoke(c, &actor.0, &by.0)).map(JsArg)
    }
    #[napi]
    pub fn grants(&self, env: Env, deps: Object<'_>) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.grants(c)).map(JsArg)
    }
    #[napi(js_name = "scopeOf")]
    pub fn scope_of(&self, env: Env, deps: Object<'_>, id: JsArg) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.scope_of(c, &id.0).map(|s| s.to_arg())).map(JsArg)
    }
    #[napi]
    pub fn visible(&self, env: Env, deps: Object<'_>, caller: JsArg, facts: JsArg) -> Result<bool> {
        self.run(env, &deps, |p, c| p.visible(c, &caller.0, &facts.0))
    }
    #[napi(js_name = "checkSend")]
    pub fn check_send(&self, env: Env, deps: Object<'_>, sender: JsArg, recipients: JsArg, text: JsArg, depth: f64) -> Result<()> {
        let recipients = list(recipients.0);
        self.run(env, &deps, |p, c| p.check_send(c, &sender.0, &recipients, &text.0, depth))
    }
    #[napi]
    pub fn allow(&self, env: Env, deps: Object<'_>, session: JsArg, remote: bool, ttl_ms: JsArg, by: JsArg) -> Result<JsArg> {
        let ttl = opt(ttl_ms);
        self.run(env, &deps, |p, c| p.allow(c, &session.0, remote, ttl.as_ref(), &by.0)).map(JsArg)
    }
    #[napi]
    pub fn disallow(&self, env: Env, deps: Object<'_>, session: JsArg, by: JsArg) -> Result<()> {
        self.run(env, &deps, |p, c| p.disallow(c, &session.0, &by.0))
    }
    #[napi]
    pub fn consented(&self, env: Env, deps: Object<'_>, session: JsArg, remote: bool) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.consented(c, &session.0, remote)).map(JsArg)
    }
    #[napi(js_name = "wakeAllowed")]
    pub fn wake_allowed(&self, env: Env, deps: Object<'_>, sender: JsArg, target: JsArg, usage: JsArg) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.wake_allowed(c, &sender.0, &target.0, &usage.0)).map(JsArg)
    }
    #[napi(js_name = "spendWake")]
    pub fn spend_wake(&self, env: Env, deps: Object<'_>, sender: JsArg, target: JsArg) -> Result<()> {
        self.run(env, &deps, |p, c| p.spend_wake(c, &sender.0, &target.0))
    }
    #[napi]
    pub fn audit(&self, env: Env, deps: Object<'_>, entry: JsArg) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.audit(c, entry.0)).map(JsArg)
    }
    #[napi(js_name = "readAudit")]
    pub fn read_audit(&self, env: Env, deps: Object<'_>, limit: f64) -> Result<JsArg> {
        self.run(env, &deps, |p, c| p.read_audit(c, limit)).map(JsArg)
    }
}

// ----- channels -----

#[napi]
pub struct ChannelsCore {
    inner: RefCell<xenonet_engine::delivery::channel::Channels>,
}

fn now_of(env: Env, deps: &Object<'_>) -> impl FnMut() -> f64 {
    let mut c = SyncCalls::new(&env, deps);
    move || {
        use xenonet_engine::peer::Calls;
        match c.call("", "now", vec![]) {
            Ok(Arg::Num(n)) => n,
            _ => f64::NAN,
        }
    }
}

#[napi]
impl ChannelsCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        ChannelsCore { inner: RefCell::new(xenonet_engine::delivery::channel::Channels::default()) }
    }

    #[napi]
    pub fn live(&self, env: Env, deps: Object<'_>, agent: String, waiting: bool) -> Result<bool> {
        let mut now = now_of(env, &deps);
        let out = self.inner.try_borrow().map_err(|_| busy())?.live(&agent, waiting, &mut now);
        settle(&env);
        Ok(out)
    }
    /// What waited for this poll, or null to wait.
    #[napi]
    pub fn poll(&self, agent: String, now: f64) -> Result<JsArg> {
        Ok(JsArg(self.inner.try_borrow_mut().map_err(|_| busy())?.poll(&agent, now).map_or(Arg::Null, Arg::List)))
    }
    #[napi]
    pub fn polled(&self, agent: String, now: f64) -> Result<()> {
        self.inner.try_borrow_mut().map_err(|_| busy())?.polled(&agent, now);
        Ok(())
    }
    /// "not-live", "waiter" or "queued".
    #[napi]
    pub fn push(&self, env: Env, deps: Object<'_>, agent: String, item: JsArg, waiting: bool) -> Result<String> {
        use xenonet_engine::delivery::channel::Push;
        let mut now = now_of(env, &deps);
        let out = self.inner.try_borrow_mut().map_err(|_| busy())?.push(&agent, item.0, waiting, &mut now);
        settle(&env);
        Ok(match out {
            Push::NotLive => "not-live",
            Push::Waiter => "waiter",
            Push::Queued => "queued",
        }
        .into())
    }
}

#[napi(js_name = "channelWaitMs")]
pub fn channel_wait_ms(wait: f64) -> f64 {
    xenonet_engine::delivery::channel::Channels::wait_ms(wait)
}

// ----- tmux -----

use crate::machine::{start, AskCalls, Machine};
use std::sync::Arc;
use xenonet_engine::delivery::tmux::Tmux;

#[napi]
pub struct TmuxCore {
    inner: Arc<Tmux>,
}

#[napi]
impl TmuxCore {
    #[napi(constructor)]
    pub fn new(off: bool) -> Self {
        TmuxCore { inner: Arc::new(Tmux { off }) }
    }
    #[napi]
    pub fn locate(&self, pid: JsArg) -> Machine {
        let t = self.inner.clone();
        start(move |asker| t.locate(&mut AskCalls(asker), &pid.0).map(|p| p.map_or(Arg::Null, |(p, pid)| p.to_arg(Some(pid)))))
    }
    #[napi]
    pub fn deliver(&self, pid: JsArg, text: JsArg) -> Machine {
        let t = self.inner.clone();
        start(move |asker| t.deliver(&mut AskCalls(asker), &pid.0, &text.0))
    }
}

// ----- reach -----

use xenonet_engine::delivery::reach;

#[napi(js_name = "reachSender")]
pub fn reach_sender(id: JsArg) -> JsArg {
    JsArg(reach::sender_of(&id.0).to_arg())
}

/// { ok: true }, or { ok: false, why }.
#[napi(js_name = "reachDecide")]
pub fn reach_decide(action: String, sender: JsArg, target: JsArg, settings: JsArg) -> JsArg {
    let Some(a) = reach::Action::parse(&action) else {
        return JsArg(reach::Decision::Refuse(format!("nothing decides \"{action}\" here")).to_arg());
    };
    let s = reach::Settings::of(&settings.0);
    JsArg(reach::decide(a, &reach::Sender::of(&sender.0), &xenonet_engine::store::js_text(&target.0), &s).to_arg())
}

// ----- starts -----

use crate::js::Json as J;
use serde_json::{json as j, Value as V};
use xenonet_engine::delivery::starts;

#[napi(js_name = "startsConstants")]
pub fn starts_constants() -> J {
    J(j!({
        "target": starts::TARGET,
        "apps": starts::APPS,
        "modes": starts::MODES,
        "promptMax": starts::PROMPT_MAX,
        "perHour": starts::PER_HOUR,
        "perHostHour": starts::PER_HOST_HOUR,
        "freshMs": starts::FRESH_MS,
    }))
}

/// { ok: true, request } or { ok: false, why }.
#[napi(js_name = "startsRequest")]
pub fn starts_request(v: J) -> J {
    J(match starts::request(&v.0) {
        Ok(r) => j!({ "ok": true, "request": r.to_json() }),
        Err(why) => j!({ "ok": false, "why": why }),
    })
}

#[napi(js_name = "startsFramed")]
pub fn starts_framed(by: J, from: Option<String>, prompt: String) -> String {
    starts::framed(&by.0, from.as_deref(), &prompt)
}

fn request_of(r: &V) -> starts::Request {
    let s = |k: &str| r.get(k).and_then(V::as_str).map(str::to_string);
    starts::Request { folder: s("folder").unwrap_or_default(), app: s("app").unwrap_or_default(), mode: s("mode"), prompt: s("prompt").unwrap_or_default() }
}

/// [app, args].
#[napi(js_name = "startsCommand")]
pub fn starts_command(r: J, id: String, cwd: String, settings: Option<String>, server: String, prompt_file: Option<String>) -> J {
    let (app, args) = starts::command(&request_of(&r.0), &id, &cwd, settings.as_deref(), &server, prompt_file.as_deref());
    J(j!([app, args]))
}

fn history_of(h: &V) -> Vec<(f64, String)> {
    h.as_array().into_iter().flatten().map(|e| (e.get(0).and_then(V::as_f64).unwrap_or(f64::NAN), e.get(1).and_then(V::as_str).unwrap_or("").to_string())).collect()
}

#[napi(js_name = "startsAdmit")]
pub fn starts_admit(history: J, from: String, now: f64) -> Option<String> {
    starts::admit(&history_of(&history.0), &from, now)
}

#[napi(js_name = "startsKept")]
pub fn starts_kept(history: J, now: f64) -> J {
    J(j!(starts::kept(&history_of(&history.0), now).into_iter().map(|(at, h)| j!([xenonet_base::json::number(at), h])).collect::<Vec<_>>()))
}

/// { ok: true, path } or { ok: false, why }.
#[napi(js_name = "startsFolder")]
pub fn starts_folder(asked: String, home: String, known: J, reported: Vec<String>) -> J {
    let known: Vec<(String, String)> =
        known.0.as_array().into_iter().flatten().map(|e| (e.get(0).and_then(V::as_str).unwrap_or("").to_string(), e.get(1).and_then(V::as_str).unwrap_or("").to_string())).collect();
    J(match starts::folder(&asked, &home, &known, &reported) {
        Ok(path) => j!({ "ok": true, "path": path }),
        Err(why) => j!({ "ok": false, "why": why }),
    })
}

#[napi(js_name = "startsSignedOut")]
pub fn starts_signed_out(log: String) -> bool {
    starts::signed_out(&log)
}

#[napi(js_name = "startsSessionOf")]
pub fn starts_session_of(app: String, log: String) -> Option<String> {
    starts::session_of(&app, &log)
}

#[napi(js_name = "startsAnswerWords")]
pub fn starts_answer_words(state: String, host: String, a: J) -> String {
    starts::answer_words(&state, &host, &a.0)
}

#[napi(js_name = "startsSignInWords")]
pub fn starts_sign_in_words(host: String) -> String {
    starts::sign_in_words(&host)
}

// ----- resume -----

use xenonet_engine::delivery::resume;

#[napi(js_name = "resumeEnv")]
pub fn resume_env(entries: JsArg) -> JsArg {
    let pairs = list(entries.0).into_iter().map(|e| {
        let e = list(e);
        (xenonet_engine::store::js_text(&e.first().cloned().unwrap_or(Arg::Undef)), e.get(1).cloned().unwrap_or(Arg::Undef))
    });
    JsArg(Arg::List(resume::resume_env(pairs.collect()).into_iter().map(|(k, v)| Arg::List(vec![Arg::text(k), v])).collect()))
}

#[napi(js_name = "resumeCommand")]
pub fn resume_command(agent: JsArg, settings: JsArg) -> JsArg {
    JsArg(resume::resume_command(&agent.0, &settings.0).map_or(Arg::Null, |(c, a)| Arg::List(vec![Arg::text(c), Arg::List(a)])))
}

/// { why } it can't, or { cmd: [app, args] }.
#[napi(js_name = "resumePlan")]
pub fn resume_plan(env: Env, deps: Object<'_>, agent: JsArg, settings: JsArg, cwd: JsArg) -> Result<JsArg> {
    let out = resume::plan(&mut SyncCalls::new(&env, &deps), &agent.0, &settings.0, &cwd.0);
    let out = out.map_err(|t| rethrow(&env, t))?;
    settle(&env);
    Ok(JsArg(match out {
        Err(why) => Arg::Obj(vec![("why".into(), Arg::text(why))]),
        Ok((c, a)) => Arg::Obj(vec![("cmd".into(), Arg::List(vec![Arg::text(c), Arg::List(a)]))]),
    }))
}

/// [command, args] for a copy of a Claude Code session, or null.
#[napi(js_name = "resumeFork")]
pub fn resume_fork(agent: JsArg, settings: JsArg) -> JsArg {
    JsArg(resume::fork_command(&agent.0, &settings.0).map_or(Arg::Null, |(c, a)| Arg::List(vec![Arg::text(c), Arg::List(a)])))
}

/// { why } it can't, or { cmd: [app, args] }, for a fork.
#[napi(js_name = "resumePlanFork")]
pub fn resume_plan_fork(env: Env, deps: Object<'_>, agent: JsArg, settings: JsArg, cwd: JsArg) -> Result<JsArg> {
    let out = resume::plan_fork(&mut SyncCalls::new(&env, &deps), &agent.0, &settings.0, &cwd.0);
    let out = out.map_err(|t| rethrow(&env, t))?;
    settle(&env);
    Ok(JsArg(match out {
        Err(why) => Arg::Obj(vec![("why".into(), Arg::text(why))]),
        Ok((c, a)) => Arg::Obj(vec![("cmd".into(), Arg::List(vec![Arg::text(c), Arg::List(a)]))]),
    }))
}

// ----- runs -----

use xenonet_engine::delivery::runs;

#[napi(object)]
pub struct RunsConstants {
    pub cap: u32,
    #[napi(js_name = "turnMs")]
    pub turn_ms: f64,
    #[napi(js_name = "logMax")]
    pub log_max: u32,
    #[napi(js_name = "keepMs")]
    pub keep_ms: f64,
}

#[napi(js_name = "runsConstants")]
pub fn runs_constants() -> RunsConstants {
    RunsConstants { cap: runs::CAP as u32, turn_ms: runs::TURN_MS, log_max: runs::LOG_MAX as u32, keep_ms: runs::KEEP_MS }
}

#[napi(js_name = "runsCmdExe")]
pub fn runs_cmd_exe(comspec: Option<String>, system_root: Option<String>) -> String {
    runs::cmd_exe(comspec.as_deref(), system_root.as_deref())
}

/// { program, args, verbatim }, or { why }, with cmd.exe where the twin's
/// %ComSpec% and %SystemRoot% say.
#[napi(js_name = "runsLaunch")]
pub fn runs_launch(program: String, args: Vec<String>, windows: bool, comspec: Option<String>, system_root: Option<String>) -> JsArg {
    let cmd = runs::cmd_exe(comspec.as_deref(), system_root.as_deref());
    JsArg(match runs::launch_with(&program, &args, windows, &cmd) {
        Ok(l) => l.to_arg(),
        Err(why) => Arg::Obj(vec![("why".into(), Arg::text(why))]),
    })
}

fn runs_of(records: &JsArg) -> Vec<runs::Run> {
    list(records.0.clone()).iter().filter_map(runs::run_of).collect()
}

/// null, or why a run for `key` may not start now, counting `also` (the
/// keys of runs under way, record or not).
#[napi(js_name = "runsAdmit")]
pub fn runs_admit(records: JsArg, key: String, now: f64, cap: u32, alive: Function<'_, f64, bool>, also: Vec<String>) -> Option<String> {
    runs::admit_with(&runs_of(&records), &also, &key, now, cap as usize, &mut |pid| alive.call(pid).unwrap_or(false))
}

#[napi(js_name = "runsKept")]
pub fn runs_kept(records: JsArg, now: f64) -> crate::js::Json {
    crate::js::Json(serde_json::Value::Array(runs::kept(&runs_of(&records), now).iter().map(runs::Run::to_json).collect()))
}

#[napi(js_name = "runsStaleLogs")]
pub fn runs_stale_logs(files: JsArg, now: f64) -> Vec<String> {
    let files: Vec<(String, f64)> = list(files.0).iter().map(runs::file_of).collect();
    runs::stale_logs(&files, now)
}

// ----- setup -----

use xenonet_engine::delivery::setup;

#[napi(js_name = "setupNewer")]
pub fn setup_newer(v: JsArg, min: Vec<f64>) -> bool {
    setup::newer(&v.0, &min)
}

#[napi(js_name = "setupServerName")]
pub fn setup_server_name(port: JsArg) -> String {
    setup::server_name(&port.0)
}

#[napi(js_name = "setupListenSettings")]
pub fn setup_listen_settings(port: JsArg, seconds: JsArg) -> crate::js::Json {
    crate::js::Json(setup::listen_settings(&port.0, &seconds.0))
}

#[napi(js_name = "setupWriteListen")]
pub fn setup_write_listen(env: Env, deps: Object<'_>, dir: String, port: JsArg, seconds: JsArg) -> Result<String> {
    let out = setup::write_listen_settings(&mut SyncCalls::new(&env, &deps), &dir, &port.0, &seconds.0).map_err(|t| rethrow(&env, t))?;
    settle(&env);
    Ok(out)
}

#[napi(js_name = "setupLive")]
pub fn setup_live(port: JsArg, listen_file: JsArg, home: JsArg) -> Machine {
    start(move |asker| setup::live_setup(&mut AskCalls(asker), &port.0, &listen_file.0, &home.0).map(|v| crate::args::value_to_arg(&v)))
}

// ----- opencode -----

use xenonet_engine::delivery::opencode::{self as oc, OpenCode};

#[napi(js_name = "openCodeId")]
pub fn open_code_id(id: JsArg) -> String {
    oc::open_code_id(&id.0)
}

#[napi(js_name = "fromOpenCodeId")]
pub fn from_open_code_id(id: JsArg) -> String {
    oc::from_open_code_id(&id.0)
}

#[napi(js_name = "openCodeParsePs")]
pub fn open_code_parse_ps(out: String) -> JsArg {
    JsArg(Arg::List(oc::parse_ps(&out).iter().map(oc::Proc::to_arg).collect()))
}

#[napi(js_name = "openCodeSamePath")]
pub fn open_code_same_path(a: JsArg, b: JsArg, windows: bool) -> bool {
    oc::same_path(&a.0, &b.0, windows)
}

#[napi(js_name = "openCodeProcesses")]
pub fn open_code_processes(windows: bool) -> Machine {
    start(move |asker| oc::open_code_processes(&mut AskCalls(asker), windows).map(|ps| Arg::List(ps.iter().map(oc::Proc::to_arg).collect())))
}

#[napi(js_name = "openCodePorts")]
pub fn open_code_ports(pid: f64, platform: String) -> Machine {
    start(move |asker| oc::listening_ports(&mut AskCalls(asker), pid, &platform).map(|ps| Arg::List(ps.into_iter().map(Arg::num).collect())))
}

// Named apart from the native reader's OpenCodeCore (native.rs): one JS
// name each.
#[napi(js_name = "DeliveryOpenCodeCore")]
pub struct OpenCodeCore {
    inner: Arc<OpenCode>,
    windows: bool,
}

#[napi]
impl OpenCodeCore {
    #[napi(constructor)]
    pub fn new(cache_ms: f64, windows: bool) -> Self {
        OpenCodeCore { inner: Arc::new(OpenCode::new(cache_ms)), windows }
    }
    #[napi]
    pub fn servers(&self) -> Machine {
        let o = self.inner.clone();
        start(move |asker| o.servers(&mut AskCalls(asker)).map(|s| Arg::List(s.iter().map(oc::Server::to_arg).collect())))
    }
    #[napi(js_name = "serverFor")]
    pub fn server_for(&self, id: JsArg) -> Machine {
        let (o, w) = (self.inner.clone(), self.windows);
        start(move |asker| o.server_for(&mut AskCalls(asker), &id.0, w).map(|s| s.map_or(Arg::Null, |s| s.to_arg())))
    }
    #[napi]
    pub fn info(&self, id: JsArg) -> Machine {
        let (o, w) = (self.inner.clone(), self.windows);
        start(move |asker| o.info(&mut AskCalls(asker), &id.0, w))
    }
    #[napi]
    pub fn forget(&self) {
        self.inner.forget();
    }
    #[napi]
    pub fn deliver(&self, id: JsArg, text: JsArg) -> Machine {
        let (o, w) = (self.inner.clone(), self.windows);
        start(move |asker| o.deliver(&mut AskCalls(asker), &id.0, &text.0, w))
    }
}

// ----- router -----

use xenonet_engine::delivery::router::{self as rt, Router};

#[napi(js_name = "routerAckMs")]
pub fn router_ack_ms() -> f64 {
    rt::ACK_MS
}

#[napi]
pub struct RouterCore {
    inner: Arc<Router>,
}

fn sync_done<T>(env: Env, r: xenonet_engine::peer::Result<T>) -> Result<T> {
    let out = r.map_err(|t| rethrow(&env, t));
    settle(&env);
    out
}

#[napi]
impl RouterCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        RouterCore { inner: Arc::new(Router::default()) }
    }
    #[napi]
    pub fn reaches(&self, env: Env, deps: Object<'_>, id: JsArg) -> Result<JsArg> {
        sync_done(env, self.inner.reaches(&mut SyncCalls::new(&env, &deps), &id.0)).map(JsArg)
    }
    #[napi]
    pub fn known(&self, env: Env, deps: Object<'_>) -> Result<JsArg> {
        sync_done(env, self.inner.known(&mut SyncCalls::new(&env, &deps))).map(|l| JsArg(Arg::List(l)))
    }
    #[napi]
    pub fn resumable(&self, env: Env, deps: Object<'_>, view: JsArg) -> Result<JsArg> {
        sync_done(env, self.inner.resumable(&mut SyncCalls::new(&env, &deps), &view.0)).map(JsArg)
    }
    #[napi]
    pub fn forkable(&self, env: Env, deps: Object<'_>, view: JsArg) -> Result<bool> {
        sync_done(env, self.inner.forkable(&mut SyncCalls::new(&env, &deps), &view.0))
    }
    #[napi]
    pub fn wake(&self, env: Env, deps: Object<'_>, from: JsArg, agent: JsArg) -> Result<JsArg> {
        sync_done(env, self.inner.wake(&mut SyncCalls::new(&env, &deps), &from.0, &agent.0)).map(JsArg)
    }
    #[napi]
    pub fn way(&self, env: Env, deps: Object<'_>, from: JsArg, a: JsArg) -> Result<Option<String>> {
        sync_done(env, self.inner.way(&mut SyncCalls::new(&env, &deps), &from.0, &a.0)).map(|w| w.map(str::to_string))
    }
    #[napi]
    pub fn seen(&self, env: Env, deps: Object<'_>, agent: JsArg, ids: JsArg) -> Result<()> {
        let ids = list(ids.0);
        sync_done(env, self.inner.seen(&mut SyncCalls::new(&env, &deps), &agent.0, &ids))
    }
    #[napi]
    pub fn contact(&self, env: Env, deps: Object<'_>, agent: JsArg) -> Result<()> {
        sync_done(env, self.inner.contact(&mut SyncCalls::new(&env, &deps), &agent.0))
    }
    #[napi]
    pub fn describe(&self, id: JsArg) -> Machine {
        let r = self.inner.clone();
        start(move |asker| r.describe(&mut AskCalls(asker), &id.0))
    }
    /// The records' keys.
    #[napi]
    pub fn route(&self, m: JsArg) -> Machine {
        let r = self.inner.clone();
        start(move |asker| r.route(&mut AskCalls(asker), &m.0).map(|keys| Arg::List(keys.into_iter().map(Arg::text).collect())))
    }
    #[napi]
    pub fn expire(&self, key: String) -> Machine {
        let r = self.inner.clone();
        start(move |asker| r.expire(&mut AskCalls(asker), &key).map(|()| Arg::Undef))
    }
}
