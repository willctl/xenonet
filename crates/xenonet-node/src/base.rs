//! xenonet-base for the twins of core/pricing.js and core/paths.js.

use crate::js::Json;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use xenonet_base::{paths, pricing, time, units};

// ----- pricing -----

#[napi(js_name = "pricesChecked")]
pub fn prices_checked() -> &'static str {
    pricing::PRICES_CHECKED
}

#[napi(js_name = "priorityTier")]
pub fn priority_tier(tier: String) -> bool {
    pricing::priority_tier(&tier)
}

#[napi(js_name = "plainModel")]
pub fn plain_model(model: String) -> String {
    pricing::plain_model(&model)
}

// A price row as core/pricing.js builds it: its rates, and longContext and
// fast only when the row has them.
fn row_value(p: &pricing::Price) -> Value {
    let mut o = Map::new();
    o.insert("provider".into(), json!(p.provider.as_str()));
    o.insert("ids".into(), json!(p.ids));
    o.insert("from".into(), json!(p.from));
    o.insert("input".into(), json!(p.input));
    o.insert("output".into(), json!(p.output));
    o.insert("write5m".into(), json!(p.write5m));
    o.insert("write1h".into(), json!(p.write1h));
    o.insert("read".into(), json!(p.read));
    if let Some(l) = &p.long_context {
        let mut lc = Map::new();
        lc.insert("over".into(), json!(l.over));
        if let Some(f) = l.from {
            lc.insert("from".into(), json!(f));
        }
        if let Some(u) = l.until {
            lc.insert("until".into(), json!(u));
        }
        lc.insert("input".into(), json!(l.input));
        lc.insert("output".into(), json!(l.output));
        if let Some(s) = l.scale_cache {
            lc.insert("scaleCache".into(), json!(s));
        }
        o.insert("longContext".into(), Value::Object(lc));
    }
    if let Some(f) = p.fast {
        o.insert("fast".into(), json!({ "input": f.input, "output": f.output }));
    }
    Value::Object(o)
}

#[napi(js_name = "priceOf")]
pub fn price_of(model: String, at: Option<f64>) -> Option<Value> {
    pricing::price_of(&model, at).map(row_value)
}

#[napi(object)]
pub struct Usage {
    pub input: Option<f64>,
    #[napi(js_name = "cacheRead")]
    pub cache_read: Option<f64>,
    #[napi(js_name = "cacheWrite5m")]
    pub cache_write5m: Option<f64>,
    #[napi(js_name = "cacheWrite1h")]
    pub cache_write1h: Option<f64>,
    pub output: Option<f64>,
}

#[napi(object)]
pub struct CostParts {
    pub input: f64,
    #[napi(js_name = "cacheRead")]
    pub cache_read: f64,
    #[napi(js_name = "cacheWrite")]
    pub cache_write: f64,
    pub output: f64,
    pub total: f64,
}

#[napi(js_name = "costParts")]
pub fn cost_parts(model: String, at: Option<f64>, u: Usage, fast: bool, priority: bool) -> Option<CostParts> {
    let tokens = pricing::Tokens { input: u.input, cache_read: u.cache_read, cache_write5m: u.cache_write5m, cache_write1h: u.cache_write1h, output: u.output };
    pricing::cost_parts(&model, at, &tokens, fast, priority).map(|c| CostParts {
        input: c.input,
        cache_read: c.cache_read,
        cache_write: c.cache_write,
        output: c.output,
        total: c.total,
    })
}

// ----- time -----

#[napi(js_name = "parseIso")]
pub fn parse_iso(text: String) -> f64 {
    time::parse_iso(&text).unwrap_or(f64::NAN)
}

// ----- paths -----

#[napi(js_name = "pathKey")]
pub fn path_key(p: String) -> Option<String> {
    paths::path_key(&p)
}

#[napi(js_name = "isUnder")]
pub fn is_under(child: String, parent: String) -> bool {
    paths::is_under(&child, &parent)
}

#[napi]
pub fn tilde(text: String, home: String) -> String {
    paths::tilde(&text, &home)
}

#[napi(js_name = "baseName")]
pub fn base_name(p: String) -> String {
    paths::base_name(&p)
}

/// "win32", "posix" or "native": which of Node's path modules a path keeps
/// its syntax in.
#[napi(js_name = "pathStyle")]
pub fn path_style(p: String) -> &'static str {
    match paths::path_style(&p) {
        paths::PathStyle::Windows => "win32",
        paths::PathStyle::Posix => "posix",
        paths::PathStyle::Native => "native",
    }
}

#[napi(object)]
pub struct ZoneInfo {
    pub id: String,
    pub name: String,
    pub app: Option<String>,
}

#[napi(js_name = "scratchZones")]
pub fn scratch_zones() -> Vec<ZoneInfo> {
    paths::SCRATCH_ZONES.iter().map(|z| ZoneInfo { id: z.id.into(), name: z.name.into(), app: z.app.map(Into::into) }).collect()
}

/// The index of the zone a key belongs to, or null.
#[napi(js_name = "scratchZone")]
pub fn scratch_zone(key: String) -> Option<u32> {
    let z = paths::scratch_zone(&key)?;
    paths::SCRATCH_ZONES.iter().position(|s| s == z).map(|i| i as u32)
}

/// Whether a key falls in zone `i`, whatever the zones before it say.
#[napi(js_name = "inScratchZone")]
pub fn in_scratch_zone(i: u32, key: String) -> bool {
    paths::in_scratch_zone(i as usize, &key)
}

/// The first `cmd` on `path_var` that `can_run` accepts (it throws for a
/// file it can't run), or that this process may run when there's none.
#[napi(js_name = "onPath")]
pub fn on_path(cmd: String, path_var: String, pathext: Option<String>, windows: bool, can_run: Option<Function<'_, String, Unknown<'_>>>) -> Option<String> {
    let mut check = |file: &str| match &can_run {
        Some(f) => f.call(file.to_string()).is_ok(),
        None => paths::can_run(file),
    };
    paths::on_path(&cmd, &path_var, pathext.as_deref(), windows, &mut check)
}

#[napi(js_name = "listFiles")]
pub fn list_files(dir: String, pick: Function<'_, String, bool>, depth: u32) -> Result<Vec<String>> {
    // Once `pick` throws, JS would read no further: it isn't called again,
    // and its error is thrown.
    let mut failed = None;
    let mut keep = |name: &str| {
        if failed.is_some() {
            return false;
        }
        match pick.call(name.to_string()) {
            Ok(v) => v,
            Err(e) => {
                failed = Some(e);
                false
            }
        }
    };
    let out = paths::list_files(&dir, &mut keep, depth);
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

// ----- ids -----

use xenonet_base::ids;

#[napi(object)]
pub struct IdPatterns {
    #[napi(js_name = "schemaBase")]
    pub schema_base: String,
    #[napi(js_name = "hostId")]
    pub host_id: String,
    #[napi(js_name = "actorSlug")]
    pub actor_slug: String,
    #[napi(js_name = "sessionApps")]
    pub session_apps: String,
    pub you: String,
}

#[napi(js_name = "idPatterns")]
pub fn id_patterns() -> IdPatterns {
    IdPatterns {
        schema_base: ids::SCHEMA_BASE.into(),
        host_id: ids::HOST_ID.into(),
        actor_slug: ids::ACTOR_SLUG.into(),
        session_apps: ids::session_app_alternation(),
        you: ids::YOU.into(),
    }
}

#[napi(js_name = "sessionAppNames")]
pub fn session_app_names() -> Vec<Vec<&'static str>> {
    ids::SESSION_APPS.iter().map(|(a, n)| vec![*a, *n]).collect()
}

#[napi(js_name = "isSchemaPath")]
pub fn is_schema_path(path: String) -> bool {
    ids::is_schema_path(&path)
}

#[napi(js_name = "shortId")]
pub fn short_id(path: String, major: f64) -> Result<String> {
    ids::short_id(&path, major).map_err(|e| Error::new(Status::InvalidArg, e))
}

#[napi(object)]
pub struct ParsedSchemaId {
    pub id: String,
    pub path: String,
    pub major: f64,
}

#[napi(js_name = "parseSchemaId")]
pub fn parse_schema_id(id: String) -> Option<ParsedSchemaId> {
    ids::parse_schema_id(&id).map(|p| ParsedSchemaId { id: p.id, path: p.path, major: p.major as f64 })
}

#[napi(js_name = "schemaUri")]
pub fn schema_uri(short: String) -> Option<String> {
    ids::schema_uri(&short).ok()
}

#[napi(js_name = "isHostId")]
pub fn is_host_id(host: String) -> bool {
    ids::is_host_id(&host)
}

#[napi(object)]
pub struct Qualified {
    pub host: Option<String>,
    pub local: String,
}

#[napi(js_name = "parseQualified")]
pub fn parse_qualified(id: String) -> Option<Qualified> {
    ids::parse_qualified(&id).map(|(h, l)| Qualified { host: h.map(Into::into), local: l.into() })
}

/// The id qualified, or which of the two isn't one: { id } or { error }.
#[napi(object)]
pub struct QualifyAnswer {
    pub id: Option<String>,
    pub error: Option<String>,
}

#[napi]
pub fn qualify(id: String, host: Option<String>, self_host: Option<String>) -> QualifyAnswer {
    match ids::qualify(&id, host.as_deref(), self_host.as_deref()) {
        Ok(q) => QualifyAnswer { id: Some(q), error: None },
        Err(ids::QualifyError::NotAnId) => QualifyAnswer { id: None, error: Some("id".into()) },
        Err(ids::QualifyError::NotAHost) => QualifyAnswer { id: None, error: Some("host".into()) },
    }
}

#[napi(js_name = "isSessionApp")]
pub fn is_session_app(app: String) -> bool {
    ids::is_session_app(&app)
}

#[napi(js_name = "isSessionId")]
pub fn is_session_id(id: String) -> bool {
    ids::is_session_id(&id)
}

#[napi(js_name = "appOfSession")]
pub fn app_of_session(id: String) -> Option<&'static str> {
    ids::app_of_session(&id)
}

#[napi(js_name = "isActorSlug")]
pub fn is_actor_slug(slug: String) -> bool {
    ids::is_actor_slug(&slug)
}

#[napi(object)]
pub struct ActorRef {
    pub host: Option<String>,
    pub slug: String,
}

#[napi(js_name = "parseActorId")]
pub fn parse_actor_id(id: String) -> Option<ActorRef> {
    ids::parse_actor_id(&id).map(|(h, s)| ActorRef { host: h.map(Into::into), slug: s.into() })
}

#[napi(js_name = "normalizeRemote")]
pub fn normalize_remote(url: String) -> String {
    ids::normalize_remote(&url)
}

/// A remote's host and repo path, [host, repo], or null.
#[napi(js_name = "remoteParts")]
pub fn remote_parts(url: String) -> Option<Vec<String>> {
    ids::remote_parts(&url).map(|(h, r)| vec![h, r])
}

#[napi(js_name = "parseRepoId")]
pub fn parse_repo_id(id: String) -> Option<String> {
    ids::parse_repo_id(&id).map(Into::into)
}

// ----- dirs -----

use xenonet_base::dirs::{self, Look};

/// A twin's way of looking at disk: the JS functions it was given.
struct JsLook<'a> {
    exists: Function<'a, String, bool>,
    readdir: Function<'a, String, Vec<String>>,
    mtime: Option<Function<'a, String, f64>>,
}

impl Look for JsLook<'_> {
    fn exists(&mut self, path: &str) -> bool {
        self.exists.call(path.to_string()).unwrap_or(false)
    }
    fn readdir(&mut self, path: &str) -> std::result::Result<Vec<String>, dirs::Unreadable> {
        self.readdir.call(path.to_string()).map_err(|_| dirs::Unreadable)
    }
    fn mtime(&mut self, path: &str) -> f64 {
        self.mtime.as_ref().and_then(|m| m.call(path.to_string()).ok()).unwrap_or(f64::NAN)
    }
}

#[napi(object)]
pub struct DirsEnv {
    #[napi(js_name = "claudeDesktopDir")]
    pub claude_desktop_dir: Option<String>,
    #[napi(js_name = "localAppData")]
    pub local_app_data: Option<String>,
    #[napi(js_name = "appData")]
    pub app_data: Option<String>,
    #[napi(js_name = "xdgConfigHome")]
    pub xdg_config_home: Option<String>,
}

fn env_of(e: DirsEnv) -> dirs::Env {
    dirs::Env { claude_desktop_dir: e.claude_desktop_dir, local_app_data: e.local_app_data, app_data: e.app_data, xdg_config_home: e.xdg_config_home }
}

#[napi(js_name = "appDir")]
pub fn app_dir(kind: String, xdg_state: Option<String>, xdg_cache: Option<String>, local_app_data: Option<String>, platform: String, home: String, name: String) -> String {
    let xdg = if kind == "state" { xdg_state } else { xdg_cache };
    dirs::app_dir(&kind, xdg.as_deref(), local_app_data.as_deref(), &platform, &home, &name)
}

#[napi(js_name = "findClaudeDesktopDir")]
pub fn find_claude_desktop_dir(env: DirsEnv, platform: String, home: String, exists: Function<'_, String, bool>, readdir: Function<'_, String, Vec<String>>) -> Option<String> {
    dirs::find_claude_desktop_dir(&env_of(env), &platform, &home, &mut JsLook { exists, readdir, mtime: None })
}

#[napi(js_name = "bundledCodexBinaries")]
pub fn bundled_codex_binaries(
    env: DirsEnv,
    platform: String,
    home: String,
    exists: Function<'_, String, bool>,
    readdir: Function<'_, String, Vec<String>>,
    mtime: Function<'_, String, f64>,
) -> Vec<String> {
    dirs::bundled_codex_binaries(&env_of(env), &platform, &home, &mut JsLook { exists, readdir, mtime: Some(mtime) })
}

// ----- the log -----

use xenonet_base::log::{self as xlog, ErrInfo, Log};

#[napi(object)]
pub struct LogConstants {
    pub levels: Vec<String>,
    pub components: Vec<String>,
    #[napi(js_name = "maxBytes")]
    pub max_bytes: f64,
}

#[napi(js_name = "logConstants")]
pub fn log_constants() -> LogConstants {
    LogConstants {
        levels: xlog::LEVELS.iter().map(ToString::to_string).collect(),
        components: xlog::COMPONENTS.iter().map(ToString::to_string).collect(),
        max_bytes: xlog::MAX_BYTES as f64,
    }
}

#[napi]
pub fn scrub(text: String, home: String) -> String {
    xlog::scrub(&text, &home)
}

#[napi(js_name = "unsafeText")]
pub fn unsafe_text(text: String, home: String) -> Option<&'static str> {
    xlog::unsafe_text(&text, &home)
}

#[napi(object)]
pub struct LogErr {
    pub message: Option<String>,
    pub stack: Option<String>,
    pub code: Option<String>,
}

#[napi(js_name = "LogCore")]
pub struct JsLog {
    log: Log,
}

#[napi]
// On the impl, not the methods: napi-derive copies a method's attributes
// onto the two-argument wrapper it generates for it, and an expectation
// there goes unfulfilled.
#[expect(clippy::too_many_arguments, reason = "write takes the twin's arguments one by one, as LogCore's JS method did")]
impl JsLog {
    #[napi(constructor)]
    pub fn new(dir: Option<String>, component: String, instance: Option<String>, version: Option<String>, home: String, max_bytes: f64) -> Self {
        JsLog {
            log: Log::new(dir.as_deref(), &component, instance.as_deref(), version.as_deref(), &home, if max_bytes.is_finite() { max_bytes.max(0.0) as u64 } else { u64::MAX }),
        }
    }

    /// The entry written, or null when the same came too soon.
    #[napi]
    pub fn write(&mut self, level: String, code: String, message: Option<String>, err: Option<LogErr>, fix: Option<String>, ctx: Option<Json>, now: f64) -> Option<Json> {
        let err = err.map(|e| ErrInfo { message: e.message, stack: e.stack, code: e.code });
        self.log.write(&level, &code, message.as_deref(), err.as_ref(), fix.as_deref(), ctx.as_ref().map(|c| &c.0), now).map(Json)
    }
}

#[napi(js_name = "readLogs")]
pub fn read_logs(dir: String, since: f64, level: String, component: Option<String>, code: Option<String>, limit: f64) -> Json {
    let limit = if limit.is_finite() { limit.max(0.0) as usize } else { usize::MAX };
    Json(Value::Array(xlog::read_logs(std::path::Path::new(&dir), since, &level, component.as_deref(), code.as_deref(), limit)))
}

#[napi]
pub fn summarize(entries: Json) -> Json {
    Json(Value::Array(xlog::summarize(entries.0.as_array().map(Vec::as_slice).unwrap_or_default())))
}

// ----- units -----

#[napi(js_name = "unitsMoney")]
pub fn units_money(n: Option<f64>) -> String {
    units::money(n)
}

#[napi(js_name = "unitsTokens")]
pub fn units_tokens(n: Option<f64>) -> String {
    units::tokens(n)
}

#[napi(js_name = "unitsDuration")]
pub fn units_duration(ms: Option<f64>) -> String {
    units::duration(ms)
}

// The twin says whether n was 1 itself, since "1" isn't.
#[napi(js_name = "unitsPlural")]
pub fn units_plural(n: f64, one: bool, word: String, many: String) -> String {
    format!("{} {}", units::locale(n, 0, 3), if one { word } else { many })
}

// The twin divides, as JS would whatever it was given.
#[napi(js_name = "unitsShare")]
pub fn units_share(p: f64) -> String {
    units::share_of(p)
}

#[napi(js_name = "unitsExcerpt")]
pub fn units_excerpt(s: String, n: f64) -> String {
    units::excerpt(&s, n as usize)
}

// ----- private files -----

use crate::js::Jv;
use xenonet_base::private_file::{self, Error as PrivateError};

/// What the twin's `run` answers: a command's output, or its error's
/// message (the twin keeps the error, to throw again if it ends the call).
#[napi(object)]
pub struct RunAnswer {
    pub out: Option<String>,
    pub error: Option<String>,
}

/// A JS `run`, called with [file, ...args], as private_file's Run.
pub(crate) fn runner<'a, 'f: 'a>(f: &'a Function<'f, Vec<String>, RunAnswer>) -> Box<private_file::Run<'a>> {
    Box::new(move |file, args| {
        let argv = std::iter::once(file).chain(args.iter().copied()).map(String::from).collect();
        match f.call(argv) {
            Ok(RunAnswer { out: Some(out), .. }) => Ok(out),
            Ok(RunAnswer { error, .. }) => Err(error.unwrap_or_default()),
            Err(e) => Err(e.reason.clone()),
        }
    })
}

/// { ok } or { error: { kind, message, code } }: "run" when a command
/// failed (the twin throws its error), "said" or "fs" for an Error of these
/// words (and code).
pub(crate) fn outcome(r: std::result::Result<Jv, PrivateError>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", v)]),
        Err(e) => {
            let (kind, code) = match &e {
                PrivateError::Run(_) => ("run", None),
                PrivateError::Said(_) => ("said", None),
                PrivateError::Fs(f) => ("fs", f.code.clone()),
            };
            let code = code.map_or(Jv::Undef, |c| Jv::Json(Value::String(c)));
            Jv::Obj(vec![("error", Jv::Obj(vec![("kind", Jv::Json(json!(kind))), ("message", Jv::Json(json!(e.message()))), ("code", code)]))])
        }
    }
}

pub(crate) fn text(s: String) -> Jv {
    Jv::Json(Value::String(s))
}

#[napi(js_name = "userSid")]
pub fn user_sid(run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    outcome(private_file::user_sid(&mut runner(&run)).map(text))
}

#[napi(js_name = "aclOf")]
pub fn acl_of(file: String, run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    outcome(private_file::acl_of(&file, &mut runner(&run)).map(text))
}

#[napi(js_name = "othersIn")]
pub fn others_in(sddl: String, sid: String) -> Vec<String> {
    private_file::others_in(&sddl, &sid)
}

#[napi(js_name = "otherReaders")]
pub fn other_readers(file: String, windows: bool, run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    outcome(private_file::other_readers(&file, windows, &mut runner(&run)).map(|v| Jv::Json(json!(v))))
}

#[napi(js_name = "protectFile")]
pub fn protect_file(file: String, windows: bool, run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    outcome(private_file::protect_file(&file, windows, &mut runner(&run)).map(|()| Jv::Undef))
}

// ----- the client -----

use crate::js::MaybeJson;
use xenonet_base::client;

#[napi(object)]
pub struct ClientConstants {
    pub xenonet: u32,
    pub demo: u32,
    pub dev: u32,
    pub health_ms: u32,
    pub named_health_ms: u32,
}

#[napi(js_name = "clientConstants")]
pub fn client_constants() -> ClientConstants {
    ClientConstants {
        xenonet: client::XENONET_PORT.into(),
        demo: client::DEMO_PORT.into(),
        dev: client::DEV_PORT.into(),
        health_ms: client::HEALTH_MS as u32,
        named_health_ms: client::NAMED_HEALTH_MS as u32,
    }
}

/// Which XenoNet, as the twin reads the options: a port when one is named
/// (Number() of it), and XENONET_PORT when it's set.
#[napi(object)]
pub struct ClientPick {
    pub port: Option<f64>,
    pub demo: bool,
    pub dev: bool,
    pub env_port: Option<String>,
}

fn pick_of(p: ClientPick) -> client::Pick {
    client::Pick { port: p.port, demo: p.demo, dev: p.dev, env_port: p.env_port }
}

#[napi(js_name = "clientCandidates")]
pub fn client_candidates(p: ClientPick) -> Vec<f64> {
    client::candidates(&pick_of(p))
}

#[napi(js_name = "clientHealthMs")]
pub fn client_health_ms(p: ClientPick) -> f64 {
    client::health_ms(&pick_of(p)) as f64
}

#[napi(js_name = "clientNotRunning")]
pub fn client_not_running(ports: Vec<f64>) -> String {
    client::not_running(&ports)
}

#[napi(js_name = "clientDefaultName")]
pub fn client_default_name(port: f64) -> &'static str {
    client::default_name(port)
}

// The calls wait on the network in a thread of their own, not in one of
// libuv's few, which the files share: a long poll would hold one for its
// whole wait.

/// A promise of the first XenoNet of these ports that answers: { port,
/// health }, or null.
#[napi(js_name = "clientFind")]
pub fn client_find(env: &Env, ports: Vec<f64>, answer_ms: f64) -> Result<Object<'_>> {
    let (deferred, promise) = env.create_deferred()?;
    std::thread::spawn(move || {
        let found = client::find(&ports, std::time::Duration::from_millis(answer_ms.max(0.0) as u64));
        deferred.resolve(move |_| Ok(found.map_or(Jv::Json(Value::Null), |s| Jv::Obj(vec![("port", Jv::Num(s.port.into())), ("health", Jv::Json(s.health))]))));
    });
    Ok(promise)
}

/// A promise of { ok } (the answer) or { error: { message, status } }.
#[napi(js_name = "clientRequest")]
pub fn client_request(env: &Env, port: f64, name: String, method: String, route: String, body: MaybeJson, headers: Vec<Vec<String>>) -> Result<Object<'_>> {
    let (deferred, promise) = env.create_deferred()?;
    let headers: Vec<(String, String)> = headers.into_iter().filter_map(|h| Some((h.first()?.clone(), h.get(1)?.clone()))).collect();
    std::thread::spawn(move || {
        let answer = client::request(port, &name, &method, &route, body.0.as_ref(), &headers);
        deferred.resolve(move |_| {
            Ok(match answer {
                Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
                Err(e) => Jv::Obj(vec![("error", Jv::Obj(vec![("message", text(e.message)), ("status", e.status.map_or(Jv::Undef, |s| Jv::Num(s.into())))]))]),
            })
        });
    });
    Ok(promise)
}
