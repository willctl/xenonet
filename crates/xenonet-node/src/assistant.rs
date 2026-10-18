//! xenonet-assistant for the twins of core/assistant/.

use crate::app::{answer, fs_answer, s, AppTask};
use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use std::rc::Rc;
use xenonet_app::task;
use xenonet_assistant::{audit, client, credentials, cursors, fixtures, projection, protocol, service, source, tunnel_client, tunnels};

fn obj(v: Option<Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn arr(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

fn ok_or(r: std::result::Result<Value, String>) -> Jv {
    answer(r.map(Jv::Json))
}

#[napi(js_name = "asstConstants")]
pub fn asst_constants() -> Json {
    let l = &service::LIMITS;
    Json(json!({
        "maxLine": protocol::MAX_LINE,
        "helloKeys": protocol::HELLO_KEYS,
        "callMs": client::CALL_MS,
        "cursorMs": cursors::CURSOR_MS,
        "tokenPrefix": credentials::TOKEN_PREFIX,
        "limits": { "inputBytes": l.input_bytes, "outputBytes": l.output_bytes, "concurrent": l.concurrent, "timeoutMs": l.timeout_ms, "page": l.page, "pageMax": l.page_max },
        "messages": service::MESSAGES.iter().map(|(c, m)| (c.to_string(), json!(m))).collect::<Map<_, _>>(),
        "labelMax": projection::LABEL_MAX,
        "liveMs": projection::LIVE_MS,
        "windowMs": projection::WINDOW_MS,
        "lookbackMaxMs": projection::LOOKBACK_MAX_MS,
        "sourceNames": projection::SOURCE_NAMES,
        "phaseMs": fixtures::PHASE_MS,
        "fixture": fixtures::fixture(),
        "viewMs": source::VIEW_MS,
        "tunnel": { "transport": tunnels::TUNNEL_TRANSPORT, "guide": tunnels::GUIDE, "releases": tunnel_client::RELEASES, "tunnelsPage": tunnel_client::TUNNELS_PAGE, "pollMs": tunnels::POLL_MS, "firstPollMs": tunnels::FIRST_POLL_MS },
    }))
}

// ----- protocol -----

#[napi(js_name = "asstEndpointFile")]
pub fn asst_endpoint_file(env: Json, machine: Json) -> String {
    let m = &machine.0;
    let t = |k: &str| m.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    protocol::endpoint_file(&obj(Some(env.0)), &xenonet_components::runtime::Machine { platform: t("platform"), arch: t("arch"), home: t("home") })
}

// ----- audit -----

#[napi(js_name = "asstAccess")]
pub fn asst_access(a: MaybeJson, now: f64) -> Json {
    Json(audit::access(&a.0.unwrap_or_else(|| json!({})), now))
}

#[napi(js_name = "asstAuditLimit")]
pub fn asst_audit_limit(limit: f64) -> f64 {
    audit::limit_of(limit)
}

#[napi(js_name = "asstAuditRecent")]
pub fn asst_audit_recent(rows: Json) -> Json {
    Json(Value::Array(audit::recent(&arr(Some(rows.0)))))
}

// ----- cursors -----

/// Page tokens, signed with a key that's never written down.
#[napi(js_name = "AsstCursors")]
pub struct AsstCursors {
    inner: Rc<cursors::Cursors>,
}

fn for_of(f: &Value) -> cursors::For<'_> {
    cursors::For { grant: f.get("grant"), revision: f.get("revision"), op: f.get("op"), filters: f.get("filters") }
}

#[napi]
impl AsstCursors {
    #[napi(constructor)]
    pub fn new(key: Option<Buffer>) -> Self {
        AsstCursors { inner: Rc::new(cursors::Cursors::new(key.map(|k| k.to_vec()))) }
    }

    /// `f`: { grant, revision, op, filters, after }.
    #[napi]
    pub fn make(&self, f: Json, now: f64) -> String {
        self.inner.make(&for_of(&f.0), f.0.get("after"), now)
    }

    /// { ok: true, after } or { ok: false, code }.
    #[napi]
    pub fn read(&self, cursor: String, f: Json, now: f64) -> Jv {
        match self.inner.read(&cursor, &for_of(&f.0), now) {
            Ok(after) => Jv::Obj(vec![("ok", Jv::Bool(true)), ("after", after.map_or(Jv::Undef, Jv::Json))]),
            Err(code) => Jv::Obj(vec![("ok", Jv::Bool(false)), ("code", s(code))]),
        }
    }
}

// ----- credentials -----

#[napi(js_name = "asstIsToken")]
pub fn asst_is_token(s: MaybeJson) -> bool {
    matches!(&s.0, Some(Value::String(t)) if credentials::is_token(t))
}

#[napi(js_name = "asstIsGrant")]
pub fn asst_is_grant(g: MaybeJson) -> bool {
    credentials::is_grant(&g.0.map_or_else(|| "undefined".into(), |v| xenonet_base::text::js_string(&v)))
}

#[napi(js_name = "asstHashToken")]
pub fn asst_hash_token(token: String) -> String {
    credentials::hash_token(&token)
}

#[napi(js_name = "asstNewToken")]
pub fn asst_new_token() -> Json {
    let (token, hash) = credentials::new_token();
    Json(json!({ "token": token, "hash": hash }))
}

#[napi(js_name = "asstAssistantDir")]
pub fn asst_assistant_dir(state_dir: String) -> String {
    credentials::assistant_dir(&state_dir)
}

struct JsPrivate<'a> {
    protect: &'a Function<'a, String, Json>,
    readers: &'a Function<'a, String, Json>,
}

// The twin's callbacks answer { ok } or { error } rather than throw.
fn settled(r: Result<Json>) -> std::result::Result<Value, String> {
    match r {
        Ok(Json(v)) => match v.get("error") {
            Some(e) => Err(e.as_str().map_or_else(|| xenonet_base::text::js_string(e), String::from)),
            None => Ok(v.get("ok").cloned().unwrap_or(Value::Null)),
        },
        Err(e) => Err(e.reason.clone()),
    }
}

impl credentials::Private for JsPrivate<'_> {
    fn protect(&mut self, file: &str) -> std::result::Result<(), String> {
        settled(self.protect.call(file.to_string())).map(|_| ())
    }
    fn readers(&mut self, file: &str) -> std::result::Result<Vec<String>, String> {
        settled(self.readers.call(file.to_string())).map(|v| v.as_array().map(|a| a.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default())
    }
}

/// `c`: { grant, token, device, expiresAt }; protect(file) and
/// readers(file) answer { ok } or { error: message }.
#[napi(js_name = "asstWriteCredential")]
pub fn asst_write_credential(dir: String, c: Json, protect: Function<'_, String, Json>, readers: Function<'_, String, Json>) -> Jv {
    let c = c.0;
    let mut p = JsPrivate { protect: &protect, readers: &readers };
    let expires = c.get("expiresAt").cloned().unwrap_or(Value::Null);
    fs_answer(credentials::write_credential(&dir, c.get("grant").unwrap_or(&Value::Null), c.get("token").unwrap_or(&Value::Null), c.get("device"), &expires, &mut p).map(s))
}

#[napi(js_name = "asstReadCredential")]
pub fn asst_read_credential(dir: String, grant: MaybeJson) -> Json {
    Json(credentials::read_credential(&dir, &grant.0.unwrap_or(Value::Null)).unwrap_or(Value::Null))
}

#[napi(js_name = "asstDeleteCredential")]
pub fn asst_delete_credential(dir: String, grant: MaybeJson) -> Jv {
    fs_answer(credentials::delete_credential(&dir, &grant.0.unwrap_or(Value::Null)).map(|()| Jv::Undef))
}

#[napi(js_name = "asstMarkRevision")]
pub fn asst_mark_revision(dir: String, grant: MaybeJson, revision: MaybeJson) -> Jv {
    let r = revision.0.unwrap_or(Value::String("undefined".into()));
    fs_answer(credentials::mark_revision(&dir, &grant.0.unwrap_or(Value::Null), &r).map(|()| Jv::Undef))
}

#[napi(js_name = "asstRevisionOf")]
pub fn asst_revision_of(dir: String, grant: MaybeJson) -> Option<f64> {
    credentials::revision_of(&dir, &grant.0.unwrap_or(Value::Null))
}

#[napi(js_name = "asstMarkRevoked")]
pub fn asst_mark_revoked(dir: String, grant: MaybeJson, at: f64) -> Jv {
    fs_answer(credentials::mark_revoked(&dir, &grant.0.unwrap_or(Value::Null), at).map(|()| Jv::Undef))
}

#[napi(js_name = "asstIsMarkedRevoked")]
pub fn asst_is_marked_revoked(dir: String, grant: MaybeJson) -> bool {
    credentials::is_marked_revoked(&dir, &grant.0.unwrap_or(Value::String("undefined".into())))
}

#[napi(js_name = "asstMarkedRevoked")]
pub fn asst_marked_revoked(dir: String) -> Vec<String> {
    credentials::marked_revoked(&dir)
}

#[napi(js_name = "asstMarkDisabled")]
pub fn asst_mark_disabled(dir: String, at: f64) -> Jv {
    fs_answer(credentials::mark_disabled(&dir, at).map(|()| Jv::Undef))
}

#[napi(js_name = "asstIsDisabled")]
pub fn asst_is_disabled(dir: String) -> bool {
    credentials::is_disabled(&dir)
}

#[napi(js_name = "asstClearDisabled")]
pub fn asst_clear_disabled(dir: String) -> Jv {
    fs_answer(credentials::clear_disabled(&dir).map(|()| Jv::Undef))
}

// ----- fixtures -----

#[napi(js_name = "asstPhaseAt")]
pub fn asst_phase_at(t: f64) -> Json {
    let (phase, since) = fixtures::phase_at(t);
    Json(json!({ "phase": phase, "since": xenonet_base::json::number(since) }))
}

#[napi(js_name = "asstFixtureView")]
pub fn asst_fixture_view(t: f64) -> Json {
    Json(fixtures::fixture_view(t))
}

#[napi(js_name = "asstFixturePolicy")]
pub fn asst_fixture_policy(policy: Json) -> Json {
    Json(fixtures::fixture_policy(&policy.0))
}

#[napi(js_name = "asstFixtureStats")]
pub fn asst_fixture_stats(id: String) -> Json {
    Json(fixtures::fixture_stats(&id))
}

#[napi(js_name = "asstStandalonePolicy")]
pub fn asst_standalone_policy(labels: bool) -> Json {
    Json(fixtures::standalone_policy(labels))
}

// ----- projection -----

#[napi(js_name = "asstCleanLabel")]
pub fn asst_clean_label(text: MaybeJson) -> Json {
    Json(projection::label_of(text.0.as_ref()))
}

#[napi(js_name = "asstSessionStatus")]
pub fn asst_session_status(live: MaybeJson, source_state: Option<String>) -> Json {
    let (status, basis) = projection::session_status(live.0.as_ref(), source_state.as_deref().unwrap_or("ready"));
    Json(json!({ "status": status, "basis": basis }))
}

#[napi(js_name = "asstStallReason")]
pub fn asst_stall_reason(reasons: MaybeJson) -> &'static str {
    projection::stall_reason(reasons.0.as_ref())
}

fn visible_jv(v: &projection::Visible<'_>) -> Value {
    json!({ "s": v.s, "project": v.project, "updated": xenonet_base::json::number(v.updated), "source": v.source, "status": v.status, "basis": v.basis })
}

#[napi(js_name = "asstVisibleSessions")]
pub fn asst_visible_sessions(view: Json, policy: Json, now: f64) -> Json {
    Json(Value::Array(projection::visible_sessions(&view.0, &policy.0, now).iter().map(visible_jv).collect()))
}

#[napi(js_name = "asstSessionSummary")]
pub fn asst_session_summary(v: Json, policy: Json) -> Json {
    let x = &v.0;
    let visible = projection::Visible {
        s: x.get("s").unwrap_or(&Value::Null),
        project: x.get("project").cloned().unwrap_or(Value::Null),
        updated: x.get("updated").and_then(Value::as_f64).unwrap_or(f64::NAN),
        source: x.get("source").map(xenonet_base::text::js_string).unwrap_or_default(),
        status: status_name(x.get("status")),
        basis: basis_name(x.get("basis")),
    };
    Json(projection::session_summary(&visible, &policy.0))
}

fn status_name(v: Option<&Value>) -> &'static str {
    ["working", "waiting", "idle", "completed", "stuck", "unknown"].into_iter().find(|n| v.and_then(Value::as_str) == Some(n)).unwrap_or("unknown")
}

fn basis_name(v: Option<&Value>) -> &'static str {
    ["observed_turn_active", "observed_wait", "observed_turn_end", "observed_idle", "observed_interrupt", "observed_inactive", "inferred_stall", "source_unavailable"]
        .into_iter()
        .find(|n| v.and_then(Value::as_str) == Some(n))
        .unwrap_or("source_unavailable")
}

#[napi(js_name = "asstFreshnessOf")]
pub fn asst_freshness_of(view: Json, now: f64, names: Option<Vec<String>>) -> Json {
    let f = projection::freshness_of(&view.0, now, &names.unwrap_or_else(projection::all_sources));
    Json(json!({ "freshness": f.freshness, "observedAt": f.observed_at.map_or(Value::Null, xenonet_base::json::number), "partial": f.partial, "warnings": f.warnings }))
}

#[napi(js_name = "asstSourcesData")]
pub fn asst_sources_data(view: Json) -> Json {
    Json(projection::sources_data(&view.0))
}

#[napi(js_name = "asstAttentionItems")]
pub fn asst_attention_items(view: Json, policy: Json, now: f64, id_of: Function<'_, String, String>) -> Result<Json> {
    let failed = std::cell::RefCell::new(None);
    let f = |parts: &str| match id_of.call(parts.to_string()) {
        Ok(id) => id,
        Err(e) => {
            failed.borrow_mut().get_or_insert(e);
            String::new()
        }
    };
    let items = projection::attention_items(&view.0, &policy.0, now, &f);
    failed.into_inner().map_or(Ok(Json(Value::Array(items))), Err)
}

#[napi(js_name = "asstActorSummaries")]
pub fn asst_actor_summaries(view: Json, policy: Json, now: f64, project_id: MaybeJson) -> Json {
    Json(Value::Array(projection::actor_summaries(&view.0, &policy.0, now, project_id.0.as_ref())))
}

#[napi(js_name = "asstStatsData")]
pub fn asst_stats_data(raw: MaybeJson, now: f64) -> Json {
    Json(projection::stats_data(&raw.0.unwrap_or(Value::Null), now))
}

// ----- source -----

#[napi(js_name = "asstSourceOf")]
pub fn asst_source_of(g: MaybeJson, l: MaybeJson, t: f64) -> Json {
    Json(source::source_of(g.0.as_ref(), l.0.as_ref(), t))
}

struct JsAround<'a> {
    home_of: &'a Function<'a, Json, Json>,
    product_of: &'a Function<'a, Json, Json>,
    registered: &'a Function<'a, Json, bool>,
    failed: Option<Error>,
}

impl source::Around for JsAround<'_> {
    fn home_of(&mut self, id: &Value) -> Value {
        match self.home_of.call(Json(id.clone())) {
            Ok(Json(v)) => v,
            Err(e) => {
                self.failed.get_or_insert(e);
                Value::Null
            }
        }
    }
    fn product_of(&mut self, id: Option<&Value>) -> Value {
        match self.product_of.call(Json(id.cloned().unwrap_or(Value::Null))) {
            Ok(Json(v)) => v,
            Err(e) => {
                self.failed.get_or_insert(e);
                Value::Null
            }
        }
    }
    fn registered(&mut self, id: &Value) -> bool {
        match self.registered.call(Json(id.clone())) {
            Ok(b) => b,
            Err(e) => {
                self.failed.get_or_insert(e);
                false
            }
        }
    }
}

/// `x`: { t, nodes, live, actors, hasActors, claude, codex }.
#[napi(js_name = "asstView")]
pub fn asst_view(x: Json, home_of: Function<'_, Json, Json>, product_of: Function<'_, Json, Json>, registered: Function<'_, Json, bool>) -> Result<Json> {
    let x = x.0;
    let mut a = JsAround { home_of: &home_of, product_of: &product_of, registered: &registered, failed: None };
    let get = |k: &str| x.get(k).cloned().unwrap_or(Value::Null);
    let v = source::view(
        x.get("t").and_then(Value::as_f64).unwrap_or(0.0),
        &arr(x.get("nodes").cloned()),
        &get("live"),
        &arr(x.get("actors").cloned()),
        x.get("hasActors") == Some(&json!(true)),
        get("claude"),
        get("codex"),
        &mut a,
    );
    a.failed.map_or(Ok(Json(v)), Err)
}

#[napi(js_name = "asstActor")]
pub fn asst_actor(snapshot: Json, product: MaybeJson) -> Json {
    Json(source::actor(&snapshot.0, product.0.unwrap_or(Value::Null)))
}

// ----- service -----

/// The service's checks and answers; the grant's standing and rate are the
/// twin's to ask.
#[napi(js_name = "AsstService")]
pub struct AsstService {
    cfg: Rc<service::Config>,
    cursors: Rc<cursors::Cursors>,
    inflight: service::Inflight,
}

fn limits_of(l: &Value) -> service::Limits {
    let d = &service::LIMITS;
    let n = |k: &str, v: f64| l.get(k).and_then(Value::as_f64).unwrap_or(v);
    service::Limits {
        input_bytes: n("inputBytes", d.input_bytes as f64) as usize,
        output_bytes: n("outputBytes", d.output_bytes as f64) as usize,
        concurrent: n("concurrent", d.concurrent),
        timeout_ms: n("timeoutMs", d.timeout_ms),
        page: n("page", d.page),
        page_max: n("pageMax", d.page_max),
    }
}

#[napi]
impl AsstService {
    /// `config`: { operations: [[tool, op]…], attentionKinds, resultSchema,
    /// errorSchema, version, limits }.
    #[napi(constructor)]
    pub fn new(config: Json, cursors: &AsstCursors) -> Self {
        let c = config.0;
        let t = |k: &str| c.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
        let cfg = service::Config {
            operations: c
                .get("operations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())))
                .collect(),
            attention_kinds: c.get("attentionKinds").and_then(Value::as_array).into_iter().flatten().map(xenonet_base::text::js_string).collect(),
            result_schema: t("resultSchema"),
            error_schema: t("errorSchema"),
            version: t("version"),
            limits: limits_of(c.get("limits").unwrap_or(&Value::Null)),
        };
        AsstService { cfg: Rc::new(cfg), cursors: cursors.inner.clone(), inflight: service::Inflight::default() }
    }

    /// { requestId, op, tool, args, no?: { code, reason } }.
    #[napi]
    pub fn begin(&self, request: MaybeJson) -> Json {
        let b = service::begin(&self.cfg, &request.0.unwrap_or(Value::Null));
        let mut m = Map::new();
        m.insert("requestId".into(), b.request_id);
        m.insert("op".into(), json!(b.op));
        m.insert("tool".into(), json!(b.tool));
        m.insert("args".into(), b.args);
        if let Some(no) = b.no {
            m.insert("no".into(), json!({ "code": no.code, "reason": no.reason }));
        }
        Json(Value::Object(m))
    }

    #[napi]
    pub fn enter(&mut self, grant: String) -> bool {
        let c = self.cfg.limits.concurrent;
        self.inflight.enter(&grant, c)
    }

    #[napi]
    pub fn leave(&mut self, grant: String) {
        self.inflight.leave(&grant);
    }

    #[napi(js_name = "errorEnvelope")]
    pub fn error_envelope(&self, request_id: MaybeJson, code: String, retry_after_ms: Option<f64>) -> Json {
        Json(service::error_envelope(&self.cfg.error_schema, request_id.0.as_ref(), &code, retry_after_ms))
    }

    /// One answer, as a task: it asks for now, view, actor, stats, idKey,
    /// cursorNow and check.
    #[napi]
    pub fn answer(&self, op: String, tool: String, args: Json, policy: Json, request_id: MaybeJson) -> AppTask {
        let (cfg, cursors) = (self.cfg.clone(), self.cursors.clone());
        let rid = request_id.0.unwrap_or(Value::Null);
        AppTask::of(task::Task::new(move |io| async move { service::answer(io, cfg, cursors, op, tool, args.0, policy.0, rid).await }))
    }
}

/// An error's envelope with the schema given.
#[napi(js_name = "asstErrorEnvelope")]
pub fn asst_error_envelope(schema: String, request_id: MaybeJson, code: String, retry_after_ms: Option<f64>) -> Json {
    Json(service::error_envelope(&schema, request_id.0.as_ref(), &code, retry_after_ms))
}

// ----- client -----

/// One grant's adapter: when to try the service again, and the grant's
/// standing on disk.
#[napi(js_name = "AsstAdapter")]
pub struct AsstAdapter {
    inner: client::Adapter,
}

fn refused_jv(r: client::Refused) -> Jv {
    Jv::Obj(vec![("error", Jv::Obj(vec![("code", s(r.code)), ("detail", s(r.detail))]))])
}

#[napi]
impl AsstAdapter {
    #[napi(factory)]
    pub fn of(grant: MaybeJson) -> Result<Self> {
        client::Adapter::new(&grant.0.unwrap_or(Value::String("undefined".into()))).map(|inner| AsstAdapter { inner }).map_err(Error::from_reason)
    }

    #[napi(js_name = "mayTry")]
    pub fn may_try(&self, now: f64) -> bool {
        self.inner.may_try(now)
    }

    #[napi(getter, js_name = "retryAt")]
    pub fn retry_at(&self) -> f64 {
        self.inner.retry_at
    }

    #[napi(getter, js_name = "stateDir")]
    pub fn state_dir(&self) -> Option<String> {
        self.inner.state_dir.clone()
    }

    /// { ok: { port, pid } } or { error: { code, detail } }.
    #[napi]
    pub fn endpoint(&mut self, text: Option<String>, alive: Function<'_, f64, bool>) -> Jv {
        let mut a = |pid: f64| alive.call(pid).unwrap_or(false);
        match self.inner.endpoint(text.as_deref(), &mut a) {
            Ok(e) => Jv::Obj(vec![("ok", Jv::Obj(vec![("port", Jv::Num(e.port)), ("pid", Jv::Num(e.pid))]))]),
            Err(r) => refused_jv(r),
        }
    }

    /// { ok: token } or { error: { code, detail } }; the token goes only
    /// into hello.
    #[napi]
    pub fn token(&self) -> Jv {
        match self.inner.token() {
            Ok(t) => Jv::Obj(vec![("ok", s(t))]),
            Err(r) => refused_jv(r),
        }
    }

    #[napi]
    pub fn connected(&mut self) {
        self.inner.connected();
    }

    #[napi]
    pub fn failed(&mut self, now: f64, random: f64) {
        self.inner.failed(now, random);
    }

    #[napi]
    pub fn standing(&self, now: f64) -> Json {
        Json(self.inner.standing(now))
    }
}

#[napi(js_name = "asstHelloCode")]
pub fn asst_hello_code(message: String) -> String {
    client::hello_code(&message).to_string()
}

#[napi(js_name = "asstReadCode")]
pub fn asst_read_code(message: String) -> &'static str {
    client::read_code(&message)
}

#[napi(js_name = "asstIsAnswer")]
pub fn asst_is_answer(r: MaybeJson) -> bool {
    r.0.as_ref().is_some_and(client::is_answer)
}

// ----- tunnel-client -----

#[napi(js_name = "asstIsTunnelId")]
pub fn asst_is_tunnel_id(s: String) -> bool {
    tunnel_client::is_tunnel_id(&s)
}

#[napi(js_name = "asstIsRuntimeKey")]
pub fn asst_is_runtime_key(s: String) -> bool {
    tunnel_client::is_runtime_key(&s)
}

#[napi(js_name = "asstHomeOf")]
pub fn asst_home_of(env: Json, home: Option<String>) -> String {
    tunnel_client::home_of(&obj(Some(env.0)), home.as_deref())
}

#[napi(js_name = "asstClientPlaces")]
pub fn asst_client_places(env: Json, platform: String, home: String) -> Vec<String> {
    tunnel_client::client_places(&obj(Some(env.0)), &platform, &home)
}

/// exists(file): { ok: bool } or { error }.
#[napi(js_name = "asstFindClient")]
pub fn asst_find_client(env: Json, platform: String, home: String, exists: Function<'_, String, Json>) -> Option<String> {
    let mut e = |f: &str| settled(exists.call(f.to_string())).map(|v| v == json!(true)).map_err(|_| ());
    tunnel_client::find_client(&obj(Some(env.0)), &platform, &home, &mut e)
}

#[napi(js_name = "asstVersionIn")]
pub fn asst_version_in(stdout: String) -> Option<String> {
    tunnel_client::version_in(&stdout)
}

#[napi(js_name = "asstProfileDirs")]
pub fn asst_profile_dirs(env: Json, platform: String, home: String) -> Vec<String> {
    tunnel_client::profile_dirs(&obj(Some(env.0)), &platform, &home)
}

#[napi(js_name = "asstReadProfile")]
pub fn asst_read_profile(text: String) -> Json {
    Json(tunnel_client::read_profile(&text))
}

struct JsFiles<'a> {
    list: &'a Function<'a, String, Json>,
    read: &'a Function<'a, String, Json>,
    stat: &'a Function<'a, String, Json>,
}

impl tunnel_client::Files for JsFiles<'_> {
    fn list(&mut self, dir: &str) -> std::result::Result<Vec<String>, ()> {
        settled(self.list.call(dir.to_string())).map(|v| v.as_array().map(|a| a.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default()).map_err(|_| ())
    }
    fn read(&mut self, file: &str) -> std::result::Result<String, ()> {
        settled(self.read.call(file.to_string())).map(|v| xenonet_base::text::js_string(&v)).map_err(|_| ())
    }
    fn mtime(&mut self, file: &str) -> std::result::Result<f64, ()> {
        settled(self.stat.call(file.to_string())).map(|v| xenonet_base::text::js_number(Some(&v))).map_err(|_| ())
    }
}

/// list(dir), read(file) and stat(file) (its mtimeMs) each answer { ok }
/// or { error }.
#[napi(js_name = "asstFindProfiles")]
pub fn asst_find_profiles(dirs: Vec<String>, list: Function<'_, String, Json>, read: Function<'_, String, Json>, stat: Function<'_, String, Json>) -> Json {
    let mut f = JsFiles { list: &list, read: &read, stat: &stat };
    Json(Value::Array(tunnel_client::find_profiles(&dirs, &mut f)))
}

#[napi(js_name = "asstCommandLine")]
pub fn asst_command_line(argv: Json) -> String {
    tunnel_client::command_line(&arr(Some(argv.0)))
}

#[napi(js_name = "asstAdapterArgv")]
pub fn asst_adapter_argv(a: Json) -> Jv {
    let a = a.0;
    let get = |k: &str| a.get(k).cloned().unwrap_or(Value::Null);
    let port = a.get("port").cloned().unwrap_or_else(|| json!(47840));
    ok_or(tunnel_client::adapter_argv(&get("exec"), &get("entry"), &get("grant"), &port).map(Value::Array))
}

#[napi(js_name = "asstProfileYaml")]
pub fn asst_profile_yaml(a: Json) -> Jv {
    let a = a.0;
    let get = |k: &str| a.get(k).cloned().unwrap_or(Value::String("undefined".into()));
    ok_or(tunnel_client::profile_yaml(&get("tunnelId"), &get("command"), a.get("label")).map(Value::String))
}

#[napi(js_name = "asstLogLine")]
pub fn asst_log_line(line: String) -> Json {
    Json(tunnel_client::log_line(&line).unwrap_or(Value::Null))
}

#[napi(js_name = "asstHealthOf")]
pub fn asst_health_of(h: MaybeJson) -> Json {
    Json(tunnel_client::health_of(h.0.as_ref()))
}

#[napi(js_name = "asstMaskKeys")]
pub fn asst_mask_keys(text: String) -> String {
    tunnel_client::mask_keys(&text)
}

// ----- tunnels -----

fn binding_jv(b: &tunnels::Binding) -> Value {
    json!({ "tunnelId": b.tunnel_id, "keyTarget": b.key_target, "autostart": b.autostart })
}

fn binding_from(v: &Value) -> tunnels::Binding {
    let t = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    tunnels::Binding { tunnel_id: t("tunnelId"), key_target: t("keyTarget"), autostart: v.get("autostart") == Some(&json!(true)) }
}

fn tunnel_refused(r: tunnels::Refused) -> Jv {
    Jv::Obj(vec![("error", Jv::Obj(vec![("status", Jv::Num(f64::from(r.status))), ("code", s(r.code)), ("message", s(r.message))]))])
}

#[napi(js_name = "asstKeyTargetFor")]
pub fn asst_key_target_for(tunnel_id: String) -> String {
    tunnels::key_target_for(&tunnel_id)
}

#[napi(js_name = "asstBindingOf")]
pub fn asst_binding_of(text: Option<String>, target_ok: Function<'_, String, bool>) -> Json {
    let ok = |s: &str| target_ok.call(s.to_string()).unwrap_or(false);
    Json(binding_jv(&tunnels::binding_of(text.as_deref(), &ok)))
}

#[napi(js_name = "asstKeyTargetOf")]
pub fn asst_key_target_of(binding: Json) -> Option<String> {
    tunnels::key_target_of(&binding_from(&binding.0))
}

/// { ok: { binding, text } } or { error: { status, code, message } }.
#[napi(js_name = "asstConfigure")]
pub fn asst_configure(binding: Json, tunnel_id: MaybeJson, key_target: MaybeJson, autostart: MaybeJson, target_ok: Function<'_, String, bool>) -> Jv {
    let ok = |s: &str| target_ok.call(s.to_string()).unwrap_or(false);
    match tunnels::configure(&binding_from(&binding.0), tunnel_id.0.as_ref(), key_target.0.as_ref(), autostart.0.as_ref(), &ok) {
        Ok(b) => Jv::Obj(vec![("ok", Jv::Obj(vec![("binding", Jv::Json(binding_jv(&b))), ("text", s(b.text()))]))]),
        Err(r) => tunnel_refused(r),
    }
}

#[napi(js_name = "asstRuntimeKey")]
pub fn asst_runtime_key(binding: Json, key: MaybeJson) -> Jv {
    match tunnels::runtime_key(&binding_from(&binding.0), key.0.as_ref()) {
        Ok(k) => Jv::Obj(vec![("ok", s(k))]),
        Err(r) => tunnel_refused(r),
    }
}

#[napi(js_name = "asstChildEnv")]
pub fn asst_child_env(env: Json, key: String, port: MaybeJson, run_as_node: bool, run: bool) -> Json {
    Json(Value::Object(tunnels::child_env(&obj(Some(env.0)), &key, &port.0.unwrap_or(Value::String("undefined".into())), run_as_node, run)))
}

#[napi(js_name = "asstReadyzOf")]
pub fn asst_readyz_of(health: Option<String>) -> Option<String> {
    tunnels::readyz_of(health.as_deref())
}

#[napi(js_name = "asstHealthUrl")]
pub fn asst_health_url(written: String) -> Option<String> {
    tunnels::health_url(&written)
}

#[napi(js_name = "asstCheckLines")]
pub fn asst_check_lines(text: String, home: String) -> Vec<String> {
    tunnels::check_lines(&text, &home)
}

#[napi(js_name = "asstStatusEntry")]
pub fn asst_status_entry(x: Json, binding: Json, key_stored: bool, yours: MaybeJson, run: Json) -> Json {
    Json(tunnels::status_entry(&x.0, &binding_from(&binding.0), key_stored, yours.0.as_ref().filter(|v| !v.is_null()), &run.0))
}

/// The tunnels' runs, by grant.
#[napi(js_name = "AsstTunnels")]
pub struct AsstTunnels {
    inner: tunnels::Tunnels,
}

#[napi]
impl AsstTunnels {
    #[napi(constructor)]
    pub fn new() -> Self {
        AsstTunnels { inner: tunnels::Tunnels::default() }
    }

    #[napi]
    pub fn view(&self, grant: String, pid: Option<f64>) -> Json {
        Json(self.inner.view(&grant, pid))
    }

    #[napi(js_name = "hasRun")]
    pub fn has_run(&self, grant: String) -> bool {
        self.inner.has_run(&grant)
    }

    #[napi]
    pub fn stopping(&self, grant: String) -> bool {
        self.inner.stopping(&grant)
    }

    #[napi]
    pub fn grants(&self) -> Vec<String> {
        self.inner.grants()
    }

    #[napi]
    pub fn launching(&mut self, grant: String, now: f64) -> bool {
        self.inner.launching(&grant, now)
    }

    #[napi]
    pub fn line(&mut self, grant: String, raw: String, now: f64, home: String) -> Option<String> {
        self.inner.line(&grant, &raw, now, &home)
    }

    #[napi]
    pub fn health(&mut self, grant: String, health: MaybeJson, now: f64) -> bool {
        self.inner.health(&grant, health.0.as_ref().filter(|v| !v.is_null()), now)
    }

    /// { changed, retryMs, log }.
    #[napi]
    pub fn exited(&mut self, grant: String, why: String, now: f64) -> Json {
        let a = self.inner.exited(&grant, &why, now);
        Json(json!({ "changed": a.changed, "retryMs": a.retry_ms, "log": a.log }))
    }

    #[napi]
    pub fn failed(&mut self, grant: String, message: String, now: f64) -> bool {
        self.inner.failed(&grant, &message, now)
    }

    #[napi]
    pub fn stop(&mut self, grant: String, reason: Option<String>, has_child: bool, now: f64) -> Option<bool> {
        self.inner.stop(&grant, reason.as_deref(), has_child, now)
    }

    #[napi(js_name = "hasKey")]
    pub fn has_key(&self, target: String, now: f64) -> Option<bool> {
        self.inner.has_key(&target, now)
    }

    #[napi(js_name = "setHas")]
    pub fn set_has(&mut self, target: String, has: bool, now: f64) {
        self.inner.set_has(&target, has, now);
    }
}
