//! xenonet-sources' live telemetry, Claude usage and refusal signals, for
//! the twins of core/live.js, core/claude-usage.js and
//! core/refusals/detect.js.
//!
//! The telemetry is a class (LiveCore) that holds each session's state;
//! each call that can make events gives back { emits: [[name, payload],
//! …], error }, the events in order and what JS would have thrown after
//! them, for the twin to emit and throw.

use crate::js::{from_js, to_js, type_of, Json, Jv, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{Map, Value};
use std::ptr;
use xenonet_sources::claude_usage;
use xenonet_sources::lines::TypeError;
use xenonet_sources::live::telemetry::{Call, Emit, Fields, MapOp, Telemetry, ToolCall, Where};
use xenonet_sources::live::{string_of, wiki_pages_of};
use xenonet_sources::refusals::detect;

fn check(status: sys::napi_status, what: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Error::new(Status::GenericFailure, format!("couldn't {what} ({status})")))
    }
}

unsafe fn js_string(env: sys::napi_env, s: &str) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &raw mut out) }, "make a string")?;
    Ok(out)
}

/// An object with its fields in order, undefined where a field's value is.
unsafe fn fields_js(env: sys::napi_env, f: &Fields) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
    for (k, v) in f {
        let key = unsafe { js_string(env, k)? };
        let value = match v {
            Some(v) => unsafe { to_js(env, v, 0)? },
            None => {
                let mut u = ptr::null_mut();
                check(unsafe { sys::napi_get_undefined(env, &raw mut u) }, "make undefined")?;
                u
            }
        };
        if k == "__proto__" {
            let attributes = sys::PropertyAttributes::writable | sys::PropertyAttributes::enumerable | sys::PropertyAttributes::configurable;
            let d = sys::napi_property_descriptor { utf8name: ptr::null(), name: key, method: None, getter: None, setter: None, value, attributes, data: ptr::null_mut() };
            check(unsafe { sys::napi_define_properties(env, out, 1, &raw const d) }, "define a key")?;
        } else {
            check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill an object")?;
        }
    }
    Ok(out)
}

/// A JS object's own fields, in its order, with undefined (and what JSON
/// leaves out) as None; None for null or undefined; no fields for another
/// value.
unsafe fn js_fields(env: sys::napi_env, v: sys::napi_value) -> Result<Option<Fields>> {
    use sys::ValueType::{napi_function, napi_null, napi_object, napi_symbol, napi_undefined};
    let t = unsafe { type_of(env, v)? };
    if t == napi_null || t == napi_undefined {
        return Ok(None);
    }
    if t != napi_object && t != napi_function {
        return Ok(Some(Vec::new()));
    }
    let mut keys = ptr::null_mut();
    check(
        unsafe {
            sys::napi_get_all_property_names(
                env,
                v,
                sys::KeyCollectionMode::own_only,
                sys::KeyFilter::enumerable | sys::KeyFilter::skip_symbols,
                sys::KeyConversion::numbers_to_strings,
                &raw mut keys,
            )
        },
        "list an object's keys",
    )?;
    let mut n = 0u32;
    check(unsafe { sys::napi_get_array_length(env, keys, &raw mut n) }, "count an object's keys")?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut key = ptr::null_mut();
        check(unsafe { sys::napi_get_element(env, keys, i, &raw mut key) }, "read a key")?;
        let mut item = ptr::null_mut();
        check(unsafe { sys::napi_get_property(env, v, key, &raw mut item) }, "read a property")?;
        let it = unsafe { type_of(env, item)? };
        let value = if it == napi_undefined || it == napi_function || it == napi_symbol { None } else { Some(unsafe { from_js(env, item, 0)? }) };
        out.push((unsafe { String::from_napi_value(env, key)? }, value));
    }
    Ok(Some(out))
}

fn emits(env: &Env, out: Vec<Emit>, error: Option<TypeError>) -> Result<Jv> {
    let mut items = Vec::with_capacity(out.len());
    for e in &out {
        let (name, payload) = match e {
            Emit::Activity(f) => ("activity", Jv::Raw(unsafe { fields_js(env.raw(), f)? })),
            Emit::Tokens(v) => ("tokens", Jv::Json(v.clone())),
            Emit::Refusal(v) => ("refusal", Jv::Json(v.clone())),
        };
        items.push(Jv::Arr(vec![Jv::Json(Value::String(name.into())), payload]));
    }
    Ok(Jv::Obj(vec![("emits", Jv::Arr(items)), ("error", Jv::Json(error.map_or(Value::Null, |e| Value::String(e.0))))]))
}

fn events(env: &Env, list: &[xenonet_sources::live::telemetry::Event]) -> Result<Jv> {
    let mut items = Vec::with_capacity(list.len());
    for e in list {
        items.push(Jv::Raw(unsafe { fields_js(env.raw(), &e.fields)? }));
    }
    Ok(Jv::Arr(items))
}

fn maybe(v: Option<Value>) -> Jv {
    v.map_or(Jv::Undef, Jv::Json)
}

fn or_null(v: MaybeJson) -> Value {
    v.0.unwrap_or(Value::Null)
}

/// A JS Map's key as it comes in: undefined is None.
fn key_of(v: MaybeJson) -> Option<Value> {
    v.0
}

fn type_error<T>(env: Env, what: &str) -> Result<T> {
    thrown(env, TypeError(what.into()))
}

/// The telemetry, held by the twin of LiveTelemetry.
#[napi(js_name = "LiveCore")]
pub struct JsLiveCore {
    t: Telemetry,
}

#[napi]
// On the impl, not the methods: napi-derive copies a method's attributes
// onto the two-argument wrapper it generates for it, and an expectation
// there goes unfulfilled.
#[expect(clippy::too_many_arguments, reason = "line and apply take the twin's arguments one by one, as LiveCore's JS methods did")]
impl JsLiveCore {
    /// `roots`: [{ dir, app }], the folders it reads, as real paths.
    #[napi(constructor)]
    pub fn new(home: Option<String>, roots: Json) -> Self {
        let roots = match roots.0 {
            Value::Array(a) => a.iter().map(|r| (string_of(r.get("dir")), string_of(r.get("app")))).collect(),
            _ => Vec::new(),
        };
        JsLiveCore { t: Telemetry::new(home, roots) }
    }

    #[napi(getter)]
    pub fn home(&self) -> Option<String> {
        self.t.home.clone()
    }

    #[napi(setter)]
    pub fn set_home(&mut self, home: Option<String>) {
        self.t.home = home;
    }

    #[napi]
    pub fn classify(&self, file: String) -> Json {
        Json(self.t.classify(&file).unwrap_or(Value::Null))
    }

    /// A line the tailer read (`raw`, as its normalizer gave it): its
    /// refusal, and what it says.
    #[napi]
    pub fn line(&mut self, env: Env, codex: bool, session_id: String, agent_id: MaybeJson, file: MaybeJson, raw: MaybeJson, live: bool, now: f64) -> Result<Jv> {
        let f = Where { session_id, agent_id: or_null(agent_id), file: or_null(file) };
        let mut out = Vec::new();
        let r = self.t.line(codex, &f, or_null(raw), live, now, &mut out);
        emits(&env, out, r.err())
    }

    /// A line given straight to applyClaude or applyCodex.
    #[napi]
    pub fn apply(&mut self, env: Env, codex: bool, session_id: String, agent_id: MaybeJson, raw: MaybeJson, live: bool, now: f64) -> Result<Jv> {
        let f = Where { session_id, agent_id: or_null(agent_id), file: Value::Null };
        let mut out = Vec::new();
        let raw = or_null(raw);
        let r = if codex { self.t.apply_codex(&f, raw, live, now, &mut out) } else { self.t.apply_claude(&f, raw, live, now, &mut out) };
        emits(&env, out, r.err())
    }

    /// An event a caller made for a session (made when it's new).
    #[napi]
    pub fn activity(&mut self, env: &Env, id: String, app: String, ev: Unknown<'_>, live: bool, now: f64) -> Result<Jv> {
        let Some(fields) = (unsafe { js_fields(env.raw(), ev.raw())? }) else { return type_error(*env, "Cannot read properties of undefined (reading 'ts')") };
        let i = self.t.session(&id, &app);
        let mut out = Vec::new();
        self.t.note_activity(i, fields, live, now, &mut out);
        emits(env, out, None)
    }

    #[napi(js_name = "noteHook")]
    pub fn note_hook(&mut self, env: Env, id: String, body: MaybeJson, now: f64) -> Result<Jv> {
        let mut out = Vec::new();
        let r = self.t.note_hook(&id, body.0.as_ref(), now, &mut out);
        emits(&env, out, r.err())
    }

    /// A tool call, as noteTool takes it: { name, detail, full, files, ts,
    /// agentId, server, extra, ok }. Gives { emits, tool }.
    #[napi(js_name = "noteTool")]
    pub fn note_tool(&mut self, env: &Env, id: String, call: Unknown<'_>, live: bool, now: f64) -> Result<Jv> {
        let fields = unsafe { js_fields(env.raw(), call.raw())? }.unwrap_or_default();
        let get = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        let files = match get("files").flatten() {
            Some(Value::Array(a)) => a.iter().map(|f| string_of(Some(f))).collect(),
            _ => Vec::new(),
        };
        let extra = match get("extra").flatten() {
            Some(Value::Object(m)) => Some(m),
            _ => None,
        };
        let t = ToolCall {
            name: get("name").flatten(),
            detail: string_of(get("detail").flatten().as_ref()),
            full: get("full").flatten(),
            files,
            ts: get("ts").flatten().map_or(f64::NAN, |v| xenonet_base::text::js_number(Some(&v))),
            agent: get("agentId").flatten(),
            server: get("server").flatten(),
            extra,
            ok: get("ok").flatten().unwrap_or(Value::Null),
        };
        let i = self.t.find(&id).unwrap_or_else(|| self.t.session(&id, "claude"));
        let mut out = Vec::new();
        let tool = self.t.note_tool(i, t, live, now, &mut out);
        let Jv::Obj(mut fields) = emits(env, out, None)? else { unreachable!() };
        fields.push(("tool", Jv::Json(tool.map_or(Value::Null, |(_, rec)| Value::Object(rec)))));
        Ok(Jv::Obj(fields))
    }

    #[napi(js_name = "codexItem")]
    pub fn codex_item(&mut self, env: Env, id: String, item: MaybeJson, ts: f64, live: bool, now: f64) -> Result<Jv> {
        let i = self.t.find(&id).unwrap_or_else(|| self.t.session(&id, "codex"));
        let mut out = Vec::new();
        let r = match &item.0 {
            None | Some(Value::Null) => Err(TypeError("Cannot read properties of undefined (reading 'type')".into())),
            Some(it) => self.t.codex_item(i, it, ts, live, now, &mut out),
        };
        emits(&env, out, r.err())
    }

    #[napi(js_name = "noteCodexLimits")]
    pub fn note_codex_limits(&mut self, env: Env, rl: MaybeJson, ts: f64) -> Result<()> {
        match &rl.0 {
            None | Some(Value::Null) => type_error(env, "Cannot read properties of undefined (reading 'primary')"),
            Some(v) => {
                self.t.note_codex_limits(v, ts);
                Ok(())
            }
        }
    }

    /// Start has read the history.
    #[napi]
    pub fn primed(&mut self, now: f64) {
        self.t.primed(now);
    }

    #[napi(js_name = "trimHistory")]
    pub fn trim_history(&mut self, now: f64) {
        self.t.trim_history(now);
    }

    #[napi]
    pub fn history(&self, env: Env) -> Result<Jv> {
        events(&env, self.t.history())
    }

    #[napi(js_name = "historySince")]
    pub fn history_since(&self, env: Env, since: f64) -> Result<Jv> {
        events(&env, self.t.history_since(since))
    }

    // ----- sessions -----

    #[napi]
    pub fn session(&mut self, id: String, app: String) {
        self.t.session(&id, &app);
    }

    #[napi]
    pub fn has(&self, id: String) -> bool {
        self.t.find(&id).is_some()
    }

    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.t.ids()
    }

    #[napi(js_name = "sessionGet")]
    pub fn session_get(&self, id: String, name: String) -> Jv {
        match self.t.find(&id).map(|i| self.t.at(i).get(&name)) {
            Some(Some(v)) => maybe(v),
            _ => Jv::Undef,
        }
    }

    #[napi(js_name = "sessionSet")]
    pub fn session_set(&mut self, id: String, name: String, value: MaybeJson) -> bool {
        match self.t.find(&id) {
            Some(i) => self.t.at_mut(i).set(&name, value.0),
            None => false,
        }
    }

    /// Recent tool calls JS pushed.
    #[napi(js_name = "sessionPushTools")]
    pub fn session_push_tools(&mut self, id: String, items: Json) {
        if let (Some(i), Value::Array(items)) = (self.t.find(&id), items.0) {
            for t in &items {
                self.t.at_mut(i).push_tool(t);
            }
        }
    }

    /// One of a session's Maps: `op` is get, has, set, delete, clear, size
    /// or entries.
    #[napi(js_name = "sessionMap")]
    pub fn session_map(&mut self, id: String, map: String, op: String, key: MaybeJson, value: MaybeJson) -> Jv {
        let Some(i) = self.t.find(&id) else { return Jv::Undef };
        let op = match op.as_str() {
            "get" => MapOp::Get(key_of(key)),
            "has" => MapOp::Has(key_of(key)),
            "set" => MapOp::Set(key_of(key), value.0),
            "delete" => MapOp::Delete(key_of(key)),
            "clear" => MapOp::Clear,
            "size" => MapOp::Size,
            _ => MapOp::Entries,
        };
        match self.t.at_mut(i).map_op(&map, op) {
            Some(answer) => maybe(answer),
            None => Jv::Undef,
        }
    }

    /// What the twin's copies of the session's own objects should say now.
    #[napi(js_name = "sessionMirror")]
    pub fn session_mirror(&self, id: String, texts: f64) -> Json {
        Json(self.t.find(&id).map_or(Value::Null, |i| self.t.at(i).mirror(texts)))
    }

    #[napi(js_name = "sessionAddTokens")]
    pub fn session_add_tokens(&mut self, id: String, n: f64, ts: f64, live: bool) {
        if let Some(i) = self.t.find(&id) {
            self.t.at_mut(i).add_tokens(n, ts, live);
        }
    }

    /// addCall(model, k, ts, { fast, priority }), with k's tokens as the
    /// pricing twin reads them.
    #[napi(js_name = "sessionAddCall")]
    pub fn session_add_call(&mut self, id: String, model: MaybeJson, k: MaybeJson, ts: f64, opts: MaybeJson) {
        let Some(i) = self.t.find(&id) else { return };
        let k = k.0.unwrap_or(Value::Null);
        let tok = |key: &str| k.get(key).filter(|v| !v.is_null()).map(|v| xenonet_base::text::js_number(Some(v)));
        let opt = |key: &str| xenonet_base::text::truthy(opts.0.as_ref().and_then(|o| o.get(key)));
        let call = Call {
            tokens: xenonet_base::pricing::Tokens {
                input: tok("input"),
                cache_read: tok("cacheRead"),
                cache_write5m: tok("cacheWrite5m"),
                cache_write1h: tok("cacheWrite1h"),
                output: tok("output"),
            },
            fast: opt("fast"),
            priority: opt("priority"),
        };
        self.t.at_mut(i).add_call(&model.0.unwrap_or(Value::Null), &call, ts);
    }

    #[napi(js_name = "sessionAddPrompt")]
    pub fn session_add_prompt(&mut self, id: String, text: String, ts: f64) {
        if let Some(i) = self.t.find(&id) {
            self.t.at_mut(i).add_prompt(&text, ts);
        }
    }

    #[napi(js_name = "sessionAddMessage")]
    pub fn session_add_message(&mut self, env: Env, id: String, text: MaybeJson, ts: f64) -> Result<()> {
        let Some(i) = self.t.find(&id) else { return Ok(()) };
        self.t.at_mut(i).add_message(text.0.as_ref(), ts).or_else(|e| thrown(env, e))
    }

    #[napi(js_name = "sessionEndTurn")]
    pub fn session_end_turn(&mut self, id: String, ts: f64, interrupted: bool) {
        if let Some(i) = self.t.find(&id) {
            self.t.at_mut(i).end_turn(ts, interrupted);
        }
    }

    #[napi(js_name = "sessionAddEdits")]
    pub fn session_add_edits(&mut self, env: Env, id: String, edits: MaybeJson, ts: f64) -> Result<()> {
        let Some(i) = self.t.find(&id) else { return Ok(()) };
        match edits.0 {
            Some(Value::Array(list)) => self.t.at_mut(i).add_edits(&list, ts).or_else(|e| thrown(env, e)),
            _ => type_error(env, "edits is not iterable"),
        }
    }

    #[napi(js_name = "sessionResetStall")]
    pub fn session_reset_stall(&mut self, id: String) {
        if let Some(i) = self.t.find(&id) {
            self.t.at_mut(i).reset_stall();
        }
    }

    #[napi(js_name = "sessionNoteToolResult")]
    pub fn session_note_tool_result(&mut self, id: String, ok: bool, ts: f64) {
        if let Some(i) = self.t.find(&id) {
            self.t.at_mut(i).note_result(ok, ts);
        }
    }

    #[napi(js_name = "sessionTokensPerMinute")]
    pub fn session_tokens_per_minute(&self, id: String, now: f64) -> f64 {
        self.t.find(&id).map_or(0.0, |i| self.t.at(i).tokens_per_minute(now))
    }

    #[napi(js_name = "sessionStatus")]
    pub fn session_status(&self, id: String, now: f64) -> &'static str {
        self.t.find(&id).map_or("idle", |i| self.t.at(i).status(now))
    }

    #[napi(js_name = "sessionNeedsYou")]
    pub fn session_needs_you(&self, id: String, now: f64) -> Json {
        Json(self.t.find(&id).map_or(Value::Null, |i| self.t.at(i).needs_you(now)))
    }

    #[napi(js_name = "sessionStuck")]
    pub fn session_stuck(&self, id: String, now: f64) -> Json {
        Json(self.t.find(&id).map_or(Value::Null, |i| self.t.at(i).stuck(now)))
    }

    #[napi(js_name = "sessionForecast")]
    pub fn session_forecast(&self, id: String) -> Json {
        Json(self.t.find(&id).map_or(Value::Null, |i| self.t.at(i).forecast()))
    }

    #[napi(js_name = "sessionSnapshot")]
    pub fn session_snapshot(&mut self, id: String, now: f64) -> Json {
        Json(self.t.find(&id).map_or(Value::Null, |i| self.t.at_mut(i).snapshot(now)))
    }

    // ----- plan limits -----

    #[napi]
    pub fn limits(&self, which: String) -> Json {
        Json(self.t.limits(&which))
    }

    #[napi(js_name = "setLimits")]
    pub fn set_limits(&mut self, which: String, value: MaybeJson) {
        self.t.set_limits(&which, or_null(value));
    }

    #[napi(js_name = "limitSamples")]
    pub fn limit_samples(&self, which: String) -> Json {
        Json(self.t.limit_samples(&which))
    }

    #[napi(js_name = "setLimitSamples")]
    pub fn set_limit_samples(&mut self, which: String, value: MaybeJson) {
        self.t.set_limit_samples(&which, or_null(value));
    }

    #[napi(getter, js_name = "claudeReason")]
    pub fn claude_reason(&self) -> Json {
        Json(self.t.claude_reason())
    }

    #[napi(js_name = "setClaudeReason")]
    pub fn set_claude_reason(&mut self, value: MaybeJson) {
        self.t.set_claude_reason(or_null(value));
    }

    #[napi(getter, js_name = "claudeSamples")]
    pub fn claude_samples(&self) -> Json {
        Json(self.t.claude_samples())
    }

    #[napi(getter, js_name = "pendingLabel")]
    pub fn pending_label(&self) -> Json {
        Json(self.t.pending_label())
    }

    /// Anthropic answered `r`, this time.
    #[napi(js_name = "claudeAsked")]
    pub fn claude_asked(&mut self, env: Env, r: MaybeJson, now: f64) -> Result<()> {
        self.t.claude_asked(r.0.as_ref(), now).or_else(|e| thrown(env, e))
    }

    #[napi(js_name = "desktopRead")]
    pub fn desktop_read(&mut self, samples: Json) {
        self.t.desktop_read(&samples.0);
    }

    #[napi(js_name = "settleClaude")]
    pub fn settle_claude(&mut self) {
        self.t.settle_claude();
    }

    #[napi(js_name = "claudeLimits")]
    pub fn claude_limits(&self, now: f64) -> Json {
        Json(self.t.claude_limits(now))
    }

    #[napi(js_name = "limitForecast")]
    pub fn limit_forecast(&self, now: f64) -> Json {
        Json(self.t.limit_forecast(now))
    }

    // ----- outputs -----

    #[napi]
    pub fn collab(&self, snapshots: Json) -> Json {
        let list = match snapshots.0 {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        Json(Telemetry::collab(&list))
    }

    #[napi]
    pub fn collisions(&self, now: f64) -> Json {
        Json(self.t.collisions(now))
    }

    #[napi(js_name = "collisionDiffs")]
    pub fn collision_diffs(&self, a: String, b: String) -> Json {
        Json(self.t.collision_diffs(&a, &b))
    }

    #[napi(js_name = "sessionNotes")]
    pub fn session_notes(&self, id: String, now: f64) -> Json {
        Json(self.t.session_notes(&id, now))
    }

    #[napi]
    pub fn costs(&self, env: Env, now: f64, sessions: MaybeJson) -> Result<Json> {
        match sessions.0 {
            Some(Value::Array(list)) => Ok(Json(self.t.costs(now, &list))),
            _ => type_error(env, "sessions.filter is not a function"),
        }
    }

    #[napi(js_name = "sessionCosts")]
    pub fn session_costs(&self, now: f64) -> Json {
        Json(self.t.session_costs(now))
    }

    #[napi]
    pub fn snapshot(&mut self, now: f64) -> Json {
        Json(self.t.snapshot(now))
    }
}

/// The pages a wiki write names (the telemetry's wikiPagesOf), or null.
#[napi(js_name = "liveTelemetryPagesOf")]
pub fn live_telemetry_pages_of(server: MaybeJson, input: MaybeJson) -> Option<Vec<String>> {
    wiki_pages_of(server.0.as_ref().and_then(Value::as_str), input.0.as_ref())
}

// ---------- claude-usage.js ----------

#[napi(js_name = "claudeUsageConstants")]
pub fn claude_usage_constants() -> Json {
    let mut m = Map::new();
    m.insert("url".into(), claude_usage::USAGE_URL.into());
    m.insert("everyMs".into(), xenonet_base::json::number(claude_usage::USAGE_EVERY_MS));
    m.insert("keychainService".into(), claude_usage::KEYCHAIN_SERVICE.into());
    Json(Value::Object(m))
}

/// { token, expiresAt } from a credentials text, or null.
#[napi(js_name = "claudeUsageTokenOf")]
pub fn claude_usage_token_of(text: Option<String>) -> Json {
    Json(match claude_usage::token_of(text.as_deref()) {
        Some((token, expires)) => {
            let mut m = Map::new();
            m.insert("token".into(), Value::String(token));
            m.insert("expiresAt".into(), expires.map_or(Value::Null, xenonet_base::json::number));
            Value::Object(m)
        }
        None => Value::Null,
    })
}

#[napi(js_name = "claudeUsageParse")]
pub fn claude_usage_parse(body: MaybeJson, now: f64) -> Json {
    Json(claude_usage::parse_usage(body.0.as_ref(), now).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageSignInProblem")]
pub fn claude_usage_sign_in_problem(cred: MaybeJson, now: f64) -> Json {
    Json(claude_usage::sign_in_problem(cred.0.as_ref(), now).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageUnreachable")]
pub fn claude_usage_unreachable(name: MaybeJson, message: MaybeJson) -> Json {
    Json(claude_usage::unreachable(name.0.as_ref(), message.0.as_ref()))
}

#[napi(js_name = "claudeUsageStatusProblem")]
pub fn claude_usage_status_problem(status: MaybeJson, ok: bool, retry_after: MaybeJson) -> Json {
    Json(claude_usage::status_problem(status.0.as_ref(), ok, retry_after.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageReading")]
pub fn claude_usage_reading(body: MaybeJson, now: f64) -> Json {
    Json(claude_usage::reading(body.0.as_ref(), now))
}

#[napi(js_name = "claudeUsageNextIn")]
pub fn claude_usage_next_in(r: MaybeJson) -> Jv {
    maybe(claude_usage::next_in(r.0.as_ref()))
}

#[napi(js_name = "claudeUsageDesktopVersion")]
pub fn claude_usage_desktop_version(raw: MaybeJson) -> Option<String> {
    claude_usage::desktop_version(raw.0.as_ref())
}

#[napi(js_name = "claudeUsageDesktopSamples")]
pub fn claude_usage_desktop_samples(env: Env, raw: MaybeJson, checked: MaybeJson) -> Result<Json> {
    match claude_usage::desktop_samples(raw.0.as_ref(), checked.0.as_ref()) {
        Ok(v) => Ok(Json(v)),
        Err(claude_usage::DesktopError::Plain(m)) => Err(Error::new(Status::GenericFailure, m)),
        Err(claude_usage::DesktopError::Type(m)) => thrown(env, TypeError(m)),
    }
}

// ---------- refusals/detect.js ----------

#[napi(js_name = "refusalClaudeSignal")]
pub fn refusal_claude_signal(raw: MaybeJson) -> Json {
    Json(detect::claude_signal(raw.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "refusalCodexSignal")]
pub fn refusal_codex_signal(raw: MaybeJson) -> Json {
    Json(detect::codex_signal(raw.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "refusalKeyOf")]
pub fn refusal_key_of(env: Env, sig: MaybeJson, session_id: MaybeJson) -> Result<String> {
    let session = match &session_id.0 {
        None => "undefined".to_string(),
        Some(v) => xenonet_base::text::js_string(v),
    };
    match &sig.0 {
        None => type_error(env, "Cannot read properties of undefined (reading 'provider')"),
        Some(Value::Null) => type_error(env, "Cannot read properties of null (reading 'provider')"),
        Some(s) => Ok(detect::key_of(s, &session)),
    }
}
