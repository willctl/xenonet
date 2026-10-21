//! xenonet-app for the twins of core/config.js, instances.js, version.js,
//! environments.js, demo-root.js and the other small modules of core/.

// On the module, not the items: napi-derive copies an item's attributes
// onto the two-argument wrapper it generates for the export, and an
// expectation there goes unfulfilled.
#![expect(clippy::too_many_arguments, reason = "appConfig, reading and createWorld take the twin's arguments one by one, as their JS functions did")]

use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use xenonet_app::{config, demo_root, environments, instances, nodepath, version};

/// { ok } or { error: message }, for a twin to throw as JS's Error.
pub(crate) fn answer(r: std::result::Result<Jv, String>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", v)]),
        Err(e) => Jv::Obj(vec![("error", Jv::Json(Value::String(e)))]),
    }
}

pub(crate) fn s(text: impl Into<String>) -> Jv {
    Jv::Json(Value::String(text.into()))
}

// ----- version -----

#[napi(js_name = "appVersion")]
pub fn app_version(file: String) -> Jv {
    version::version_of(std::path::Path::new(&file)).into()
}

// ----- environments -----

#[napi(js_name = "appEnvironments")]
pub fn app_environments() -> Json {
    Json(Value::Array(environments::ENVIRONMENTS.iter().map(|e| json!({ "id": e.id, "name": e.name, "about": e.about, "shape": e.shape, "index": e.index })).collect()))
}

#[napi(js_name = "appDefaultEnvironment")]
pub fn app_default_environment() -> &'static str {
    environments::DEFAULT_ENVIRONMENT
}

/// The index in ENVIRONMENTS of the place with this id, or of the first.
#[napi(js_name = "appEnvironmentIndex")]
pub fn app_environment_index(id: Option<String>) -> u32 {
    environments::environment_of(id.as_deref()).index
}

// ----- instances -----

fn instance_fields(i: &instances::Instance, id: bool, port: f64) -> Vec<(&'static str, Jv)> {
    let mut f = Vec::new();
    if id {
        f.push(("id", s(i.id)));
    }
    f.push(("name", s(i.name)));
    f.push(("port", Jv::Num(port)));
    f.push(("demo", Jv::Bool(i.demo)));
    if i.frozen {
        f.push(("frozen", Jv::Bool(true)));
    }
    f
}

#[napi(js_name = "appInstances")]
pub fn app_instances() -> Jv {
    Jv::Obj(instances::INSTANCES.iter().map(|i| (i.id, Jv::Obj(instance_fields(i, false, f64::from(i.port))))).collect())
}

#[napi(js_name = "appInstanceFrom")]
pub fn app_instance_from(instance: Option<String>, demo: Option<String>, port: Option<String>) -> Jv {
    answer(instances::instance_from(instance.as_deref(), demo.as_deref(), port.as_deref()).map(|(i, port)| Jv::Obj(instance_fields(i, true, port))))
}

// ----- demo-root -----

#[napi(object)]
pub struct DemoRootPlan {
    pub retries: u32,
    pub retry_delay: u32,
    pub sibling: String,
}

#[napi(js_name = "appDemoRootPlan")]
pub fn app_demo_root_plan(root: String) -> DemoRootPlan {
    DemoRootPlan { retries: demo_root::REMOVE_RETRIES, retry_delay: demo_root::REMOVE_RETRY_DELAY_MS, sibling: demo_root::sibling_prefix(&root) }
}

#[napi(js_name = "appDemoRootUseSibling")]
pub fn app_demo_root_use_sibling(platform: String, code: Option<String>) -> bool {
    demo_root::use_sibling(&platform, code.as_deref())
}

// ----- config -----

struct JsFinds<'a> {
    world: Function<'a, (), Json>,
    context: Function<'a, (), Json>,
    context_server: Function<'a, Option<String>, Json>,
    claude_desktop_dir: Function<'a, (), Option<String>>,
    failed: Option<Error>,
}

impl JsFinds<'_> {
    fn keep<T: Default>(&mut self, r: Result<T>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => {
                self.failed.get_or_insert(e);
                T::default()
            }
        }
    }
}

impl config::Finds for JsFinds<'_> {
    fn world(&mut self) -> Value {
        let r = self.world.call(()).map(|j| j.0);
        self.keep(r)
    }
    fn context(&mut self) -> Value {
        let r = self.context.call(()).map(|j| j.0);
        self.keep(r)
    }
    fn context_server(&mut self, dir: Option<&str>) -> Value {
        let r = self.context_server.call(dir.map(String::from)).map(|j| j.0);
        self.keep(r)
    }
    fn claude_desktop_dir(&mut self) -> Option<String> {
        let r = self.claude_desktop_dir.call(());
        self.keep(r)
    }
}

/// The settings, with readers and hiddenResources as arrays for the twin
/// to make Sets of.
#[napi(js_name = "appConfig")]
pub fn app_config(
    argv: Vec<String>,
    env: Json,
    home: String,
    platform: String,
    world: Function<'_, (), Json>,
    context: Function<'_, (), Json>,
    context_server: Function<'_, Option<String>, Json>,
    claude_desktop_dir: Function<'_, (), Option<String>>,
) -> Result<Jv> {
    let env = match env.0 {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let mut finds = JsFinds { world, context, context_server, claude_desktop_dir, failed: None };
    let c = config::config(&argv, &env, &home, &platform, &mut finds);
    if let Some(e) = finds.failed {
        return Err(e);
    }
    Ok(Jv::Obj(
        c.fields
            .into_iter()
            .map(|(k, f)| {
                let v = match f {
                    config::Field::Num(n) => Jv::Num(n),
                    config::Field::Json(v) => Jv::Json(v),
                    config::Field::Set(items) => Jv::Json(json!(items)),
                    config::Field::Nums(nums) => Jv::Obj(nums.into_iter().map(|(k, n)| (k, Jv::Num(n))).collect()),
                };
                (k, v)
            })
            .collect(),
    ))
}

// ----- Node's path, for checking it against Node's own -----

#[napi(js_name = "appPath")]
pub fn app_path(style: String, op: String, args: Vec<String>, extra: MaybeJson) -> Result<String> {
    let st = nodepath::Style::of(&style);
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let one = || a.first().copied().unwrap_or("");
    Ok(match op.as_str() {
        "join" => st.join(&a),
        "resolve" => st.resolve(&a),
        "normalize" => st.normalize(one()),
        "dirname" => st.dirname(one()),
        "basename" => st.basename(one(), extra.0.as_ref().and_then(Value::as_str)),
        "extname" => st.extname(one()),
        "relative" => st.relative(one(), a.get(1).copied().unwrap_or("")),
        "isAbsolute" => st.is_absolute(one()).to_string(),
        _ => return Err(Error::from_reason(format!("no path.{op}"))),
    })
}

// ----- errors, as Node's fs words them -----

use xenonet_app::fsx::FsError;

/// { ok } or { error: { message, code? } }, for a twin to throw as JS's
/// Error with that code.
pub(crate) fn fs_answer(r: std::result::Result<Jv, FsError>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", v)]),
        Err(e) => Jv::Obj(vec![("error", fs_error(&e))]),
    }
}

pub(crate) fn fs_error(e: &FsError) -> Jv {
    let mut f = vec![("message", s(e.message.clone()))];
    if let Some(c) = &e.code {
        f.push(("code", s(c.clone())));
    }
    Jv::Obj(f)
}

// ----- command -----

use xenonet_app::command;

fn state_jv(st: command::State) -> Jv {
    Jv::Json(json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path }))
}

#[napi(js_name = "appCommandDir")]
pub fn app_command_dir(home: String, platform: String, local_app_data: Option<String>) -> String {
    command::command_dir(&home, &platform, local_app_data.as_deref())
}

#[napi(js_name = "appCommandScript")]
pub fn app_command_script(bin: String, platform: String) -> Jv {
    answer(command::command_script(&bin, &platform).map(s))
}

#[napi(js_name = "appCommandState")]
pub fn app_command_state(dir: Option<String>, path_var: Option<String>, platform: String) -> Jv {
    state_jv(command::command_state(dir.as_deref(), path_var.as_deref(), &platform))
}

#[napi(js_name = "appSamePlace")]
pub fn app_same_place(a: String, b: String, platform: String) -> bool {
    command::same_place(&a, &b, &platform)
}

fn env_map(env: Json) -> serde_json::Map<String, Value> {
    match env.0 {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// reg.exe and the arguments of each query, in the order they're asked.
#[napi(js_name = "appSavedPathQueries")]
pub fn app_saved_path_queries(env: Json) -> Json {
    let env = env_map(env);
    Json(json!({ "reg": command::reg_exe(&env), "queries": command::PATH_KEYS.iter().map(|k| command::reg_query(k)).collect::<Vec<_>>() }))
}

#[napi(js_name = "appRegPath")]
pub fn app_reg_path(stdout: String, env: Json) -> Vec<String> {
    command::reg_path(&stdout, &env_map(env))
}

#[napi(js_name = "appInstallCommand")]
pub fn app_install_command(dir: String, bin: String, force: bool, path_var: Option<String>, platform: String) -> Jv {
    fs_answer(command::install_command(&dir, &bin, force, path_var.as_deref(), &platform).map(state_jv))
}

#[napi(js_name = "appUninstallCommand")]
pub fn app_uninstall_command(dir: String, path_var: Option<String>, platform: String) -> Jv {
    fs_answer(command::uninstall_command(&dir, path_var.as_deref(), &platform).map(state_jv))
}

// ----- open -----

use xenonet_app::open;

#[napi(js_name = "appActionsFor")]
pub fn app_actions_for(node: MaybeJson) -> Json {
    Json(Value::Array(open::actions_for(node.0.as_ref())))
}

#[napi(js_name = "appUrlFor")]
pub fn app_url_for(node: Json, action: String) -> Jv {
    open::url_for(&node.0, &action).into()
}

#[napi(js_name = "appVscodeUrl")]
pub fn app_vscode_url(folder: String) -> String {
    open::vscode_url(&folder)
}

#[napi(js_name = "appHandoffUrl")]
pub fn app_handoff_url(target: String, folder: Option<String>, brief: String) -> Option<String> {
    open::handoff_url(&target, folder.as_deref(), &brief)
}

#[napi(js_name = "appOpenConstants")]
pub fn app_open_constants() -> Json {
    Json(json!({ "maxBrief": open::MAX_BRIEF, "needsInputUrl": open::NEEDS_INPUT_URL }))
}

#[napi(js_name = "appLaunchCommand")]
pub fn app_launch_command(target: String, platform: String) -> Json {
    Json(json!(open::launch_command(&target, &platform)))
}

#[napi(js_name = "appRevealCommand")]
pub fn app_reveal_command(file: String, platform: String) -> Json {
    Json(json!(open::reveal_command(&file, &platform)))
}

// ----- decide -----

use xenonet_app::decide;

fn decision_error(e: &decide::DecisionError) -> Jv {
    Jv::Obj(vec![("status", Jv::Num(f64::from(e.status))), ("code", s(e.code)), ("message", s(e.message.clone()))])
}

fn log_line(l: &decide::LogLine) -> Jv {
    Jv::Obj(vec![("level", s(l.level)), ("code", s(l.code.clone())), ("message", s(l.message.clone()))])
}

#[napi(js_name = "appDecideConstants")]
pub fn app_decide_constants() -> Json {
    Json(
        json!({ "url": decide::DECISIONS_URL, "model": decide::DEFAULT_MODEL, "keyTarget": decide::KEY_TARGET, "key": decide::KEY, "keyComment": decide::KEY_COMMENT, "timeoutMs": decide::TIMEOUT_MS }),
    )
}

#[napi(js_name = "appDecideSettings")]
pub fn app_decide_settings(url: Option<String>, model: Option<String>) -> Json {
    let (url, model) = decide::settings(url.as_deref(), model.as_deref());
    Json(json!({ "url": url, "model": model }))
}

#[napi(js_name = "appDecideCheckKey")]
pub fn app_decide_check_key(key: String) -> Jv {
    match decide::check_key(&key) {
        Ok(k) => Jv::Obj(vec![("ok", s(k))]),
        Err(e) => Jv::Obj(vec![("error", decision_error(&e))]),
    }
}

#[napi(js_name = "appDecideNoKey")]
pub fn app_decide_no_key() -> Jv {
    decision_error(&decide::no_key())
}

#[napi(js_name = "appDecideRequest")]
pub fn app_decide_request(model: String, key: String, state: MaybeJson, questions: MaybeJson) -> Jv {
    let (headers, body) = decide::request(&model, &key, state.0.as_ref(), questions.0.as_ref());
    Jv::Obj(vec![("headers", Jv::Obj(headers.into_iter().map(|(k, v)| (k, s(v))).collect())), ("body", s(body))])
}

#[napi(js_name = "appDecideUnreachable")]
pub fn app_decide_unreachable(url: String, kind: String, timed_out: bool, message: String) -> Jv {
    let (line, e) = decide::unreachable(&url, &kind, timed_out, &message);
    Jv::Obj(vec![("log", log_line(&line)), ("error", decision_error(&e))])
}

#[napi(js_name = "appDecideRefused")]
pub fn app_decide_refused(status: u32, body: MaybeJson, kind: String) -> Jv {
    let (line, e) = decide::refused(status as u16, body.0.as_ref(), &kind);
    Jv::Obj(vec![("log", log_line(&line)), ("error", decision_error(&e))])
}

#[napi(js_name = "appDecideAnswered")]
pub fn app_decide_answered(body: MaybeJson, model: String, kind: String, questions: u32, ms: f64) -> Jv {
    let (line, d) = decide::answered(body.0.as_ref(), &model, &kind, questions as usize, ms);
    Jv::Obj(vec![("log", log_line(&line)), ("decision", Jv::Json(d))])
}

// ----- mcp-info -----

use xenonet_app::mcp_info;

/// A JS read(file), or none, for Rust to read the file itself; the first
/// error it throws is kept, to throw when the call is over.
struct Reader<'a> {
    f: Option<Function<'a, Option<String>, Json>>,
    failed: Option<Error>,
}

impl Reader<'_> {
    fn read(&mut self, file: Option<&str>) -> Value {
        match &self.f {
            None => file.and_then(|f| std::fs::read(f).ok()).and_then(|b| xenonet_base::json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or_else(|| json!({})),
            Some(f) => match f.call(file.map(String::from)) {
                Ok(j) => j.0,
                Err(e) => {
                    self.failed.get_or_insert(e);
                    json!({})
                }
            },
        }
    }
}

fn next_jv(c: &mcp_info::Collect, next: mcp_info::Next) -> Jv {
    let next = match next {
        mcp_info::Next::Request { method, params } => Jv::Obj(vec![("request", Jv::Obj(vec![("method", s(method)), ("params", Jv::Json(params))]))]),
        mcp_info::Next::Discover(spec) => Jv::Obj(vec![("discover", Jv::Json(spec))]),
        mcp_info::Next::Done(r) => Jv::Obj(vec![("done", Jv::Json(r))]),
    };
    Jv::Obj(vec![("state", Jv::Json(c.to_value())), ("next", next)])
}

#[napi(js_name = "appMcpPublicUrl")]
pub fn app_mcp_public_url(value: Option<String>, origin_only: bool) -> Option<String> {
    mcp_info::public_url(value.as_deref(), origin_only)
}

#[napi(js_name = "appMcpDisplaySchema")]
pub fn app_mcp_display_schema(value: MaybeJson, depth: f64) -> Json {
    Json(mcp_info::display_schema(value.0.as_ref(), if depth.is_finite() && depth > 0.0 { depth as usize } else { 0 }))
}

#[napi(js_name = "appMcpDisplayCatalog")]
pub fn app_mcp_display_catalog(raw: MaybeJson) -> Json {
    Json(mcp_info::display_catalog(raw.0.as_ref()))
}

#[napi(js_name = "appMcpBegin")]
pub fn app_mcp_begin(server: String, sessions: Json, settings: Json, now: f64, read: Option<Function<'_, Option<String>, Json>>) -> Result<Jv> {
    let sessions = match sessions.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let mut r = Reader { f: read, failed: None };
    let (c, next) = mcp_info::begin(&server, &sessions, &settings.0, now, &mut |f| r.read(f));
    match r.failed {
        Some(e) => Err(e),
        None => Ok(next_jv(&c, next)),
    }
}

#[napi(js_name = "appMcpStep")]
pub fn app_mcp_step(state: Json, answer: Json, settings: Json, read: Option<Function<'_, Option<String>, Json>>) -> Result<Jv> {
    let answer = match answer.0.get("error") {
        Some(e) => mcp_info::Answer::Failed { name: e.get("name").and_then(Value::as_str).map(String::from), code: e.get("code").filter(|c| !c.is_null()).cloned() },
        None => mcp_info::Answer::Ok(answer.0.get("ok").cloned().unwrap_or(Value::Null)),
    };
    let mut r = Reader { f: read, failed: None };
    let (c, next) = mcp_info::step(mcp_info::Collect::from_value(state.0), answer, &settings.0, &mut |f| r.read(f));
    match r.failed {
        Some(e) => Err(e),
        None => Ok(next_jv(&c, next)),
    }
}

#[napi(js_name = "appMcpFresh")]
pub fn app_mcp_fresh(checked_at: f64, now: f64) -> bool {
    mcp_info::fresh(checked_at, now)
}

#[napi(js_name = "appDemoMcpInfo")]
pub fn app_demo_mcp_info(server: String, now: f64) -> Json {
    Json(mcp_info::demo_mcp_info(&server, now))
}

// ----- mcp-discovery -----

use xenonet_app::mcp_discovery;

#[napi(js_name = "appMcpDiscoveryConstants")]
pub fn app_mcp_discovery_constants() -> Json {
    Json(json!({ "timeoutMs": mcp_discovery::TIMEOUT_MS, "signalMs": mcp_discovery::SIGNAL_MS }))
}

#[napi(js_name = "appMcpTransport")]
pub fn app_mcp_transport(spec: Json, env: Json) -> Json {
    Json(mcp_discovery::transport(&spec.0, &env_map(env)))
}

#[napi(js_name = "appMcpAnotherPage")]
pub fn app_mcp_another_page(pages: u32, rows: u32, cursor: MaybeJson, next: MaybeJson) -> bool {
    mcp_discovery::another_page(pages as usize, rows as usize, cursor.0.as_ref().filter(|c| !c.is_null()), next.0.as_ref().filter(|c| !c.is_null()))
}

#[napi(js_name = "appMcpFound")]
pub fn app_mcp_found(server_info: MaybeJson, capabilities: MaybeJson, instructions: MaybeJson, tools: Json, resources: Json, templates: Json, by_url: bool) -> Json {
    Json(mcp_discovery::found(server_info.0.as_ref(), capabilities.0.as_ref(), instructions.0.as_ref(), tools.0, resources.0, templates.0, by_url))
}

// ----- diagnostics -----

use xenonet_app::diagnostics;

#[napi(js_name = "appDiagnosticsSchema")]
pub fn app_diagnostics_schema() -> &'static str {
    diagnostics::SCHEMA
}

#[napi(js_name = "appMakeBundle")]
pub fn app_make_bundle(g: Json) -> Jv {
    let v = g.0;
    let errors = match v.get("errors") {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let env = match v.get("env") {
        Some(Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    let text = |k: &str| v.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    let (home, platform, arch, release, node) = (text("home"), text("platform"), text("arch"), text("release"), text("node"));
    let electron = v.get("electron").and_then(Value::as_str).map(String::from);
    let gathered = diagnostics::Gathered {
        xenonet: v.get("xenonet"),
        doctor: v.get("doctor"),
        errors: &errors,
        connect: v.get("connect"),
        launcher: v.get("launcher"),
        env: &env,
        home: &home,
        now: v.get("now").and_then(Value::as_f64).unwrap_or(f64::NAN),
        platform: &platform,
        arch: &arch,
        release: &release,
        node: &node,
        electron: electron.as_deref(),
    };
    answer(diagnostics::make_bundle(&gathered).map(Jv::Json))
}

#[napi(js_name = "appBundleMarkdown")]
pub fn app_bundle_markdown(b: Json) -> String {
    diagnostics::bundle_markdown(&b.0)
}

#[napi(js_name = "appAgentBrief")]
pub fn app_agent_brief(sources: Vec<String>) -> String {
    diagnostics::agent_brief(&sources)
}

#[napi(js_name = "appWriteLook")]
pub fn app_write_look(state_dir: String, bundle: Json, sources: Vec<String>, now: f64) -> Jv {
    fs_answer(diagnostics::write_look(&state_dir, &bundle.0, &sources, now).map(s))
}

#[napi(js_name = "appAskCommand")]
pub fn app_ask_command(app: String, model: Option<String>, dir: MaybeJson, sources: Vec<String>) -> Json {
    Json(diagnostics::ask_command(&app, model.as_deref(), &dir.0.unwrap_or(Value::Null), &sources))
}

// ----- script -----

use xenonet_app::script;

fn arg_jv(a: script::Arg) -> Jv {
    match a {
        script::Arg::V(v) => Jv::Json(v),
        script::Arg::Undef => Jv::Undef,
        script::Arg::Obj(fields) => Jv::Obj(fields.into_iter().map(|(k, a)| (k, arg_jv(a))).collect()),
    }
}

fn script_jv(state: script::State, out: script::Out) -> Jv {
    let then = match out.then {
        script::Then::Call { target, args } => Jv::Obj(vec![("call", Jv::Obj(vec![("target", s(target)), ("args", Jv::Arr(args.into_iter().map(arg_jv).collect()))]))]),
        script::Then::Sleep(ms) => Jv::Obj(vec![("sleep", Jv::Num(ms))]),
        script::Then::Done => Jv::Obj(vec![("done", Jv::Bool(true))]),
        script::Then::Rethrow => Jv::Obj(vec![("rethrow", Jv::Bool(true))]),
        script::Then::Fail { message, type_error } => Jv::Obj(vec![("fail", Jv::Obj(vec![("message", s(message)), ("typeError", Jv::Bool(type_error))]))]),
    };
    Jv::Obj(vec![("state", Jv::Json(state)), ("out", Jv::Obj(vec![("say", Jv::Arr(out.say.into_iter().map(arg_jv).collect())), ("then", then)]))])
}

#[napi(js_name = "appScriptSteps")]
pub fn app_script_steps() -> &'static str {
    script::STEPS
}

#[napi(js_name = "appScriptBrief")]
pub fn app_script_brief(st: MaybeJson) -> Jv {
    match st.0 {
        None => answer(Err("Cannot read properties of undefined (reading 'selection')".into())),
        Some(v) => answer(script::brief(&v).map(Jv::Json)),
    }
}

#[napi(js_name = "appScriptBegin")]
pub fn app_script_begin(step: MaybeJson, name: String, film_seconds: f64) -> Jv {
    let (state, out) = script::begin(&step.0.unwrap_or(Value::String("undefined".into())), &script::Context { name, film_seconds });
    script_jv(state, out)
}

/// `threw` is the message of the error the call threw, if it did; else
/// `ok` is what it answered.
#[napi(js_name = "appScriptResume")]
pub fn app_script_resume(state: Json, ok: MaybeJson, threw: Option<String>) -> Jv {
    let answer = match (threw, ok.0) {
        (Some(m), _) => script::Answer::Threw(m),
        (None, Some(v)) => script::Answer::Ok(v),
        (None, None) => script::Answer::Undefined,
    };
    let (state, out) = script::resume(state.0, answer);
    script_jv(state, out)
}

// ----- spotify -----

use xenonet_app::spotify;

fn spotify_error(e: &spotify::SpotifyError) -> Jv {
    Jv::Obj(vec![("status", Jv::Num(f64::from(e.status))), ("code", s(e.code)), ("message", s(e.message.clone()))])
}

fn ask_jv(job: spotify::Job, ask: spotify::Ask) -> Jv {
    let ask = match ask {
        spotify::Ask::SecretGet(t) => Jv::Obj(vec![("secret", Jv::Obj(vec![("op", s("get")), ("target", s(t))]))]),
        spotify::Ask::SecretHas(t) => Jv::Obj(vec![("secret", Jv::Obj(vec![("op", s("has")), ("target", s(t))]))]),
        spotify::Ask::SecretRemove(t) => Jv::Obj(vec![("secret", Jv::Obj(vec![("op", s("remove")), ("target", s(t))]))]),
        spotify::Ask::SecretSet { target, value, comment } => {
            Jv::Obj(vec![("secret", Jv::Obj(vec![("op", s("set")), ("target", s(target)), ("value", s(value)), ("comment", s(comment))]))])
        }
        spotify::Ask::Fetch { url, method, headers, body, timeout_ms, fallback, skip } => {
            let mut f = vec![("url", s(url)), ("method", s(method)), ("headers", Jv::Obj(headers.into_iter().map(|(k, v)| (k, s(v))).collect()))];
            if let Some(b) = body {
                f.push(("body", s(b)));
            }
            f.extend([("timeoutMs", Jv::Num(timeout_ms)), ("fallback", Jv::Json(fallback)), ("skip", Jv::Json(json!(skip)))]);
            Jv::Obj(vec![("fetch", Jv::Obj(f))])
        }
        spotify::Ask::Log { level, code, message } => Jv::Obj(vec![("log", Jv::Obj(vec![("level", s(level)), ("code", s(code)), ("message", s(message))]))]),
        spotify::Ask::Done(v) => Jv::Obj(vec![("done", Jv::Json(v))]),
        spotify::Ask::Fail(e) => Jv::Obj(vec![("fail", spotify_error(&e))]),
        spotify::Ask::Rethrow => Jv::Obj(vec![("rethrow", Jv::Bool(true))]),
    };
    Jv::Obj(vec![("job", Jv::Json(job)), ("ask", ask)])
}

#[napi(js_name = "appSpotifyConstants")]
pub fn app_spotify_constants() -> Json {
    Json(json!({ "scopes": spotify::SCOPES, "refreshTarget": spotify::REFRESH_TARGET, "clientId": "^[0-9a-f]{32}$", "redirect": spotify::REDIRECT_SHOWN }))
}

#[napi(js_name = "appSpotifyChallenge")]
pub fn app_spotify_challenge(verifier: String) -> String {
    spotify::challenge_of(&verifier)
}

#[napi(js_name = "appSpotifyNowPlaying")]
pub fn app_spotify_now_playing(p: MaybeJson) -> Json {
    Json(spotify::now_playing(p.0.as_ref()))
}

#[napi(js_name = "appSpotifyDuck")]
pub fn app_spotify_duck(waiting: bool, player: MaybeJson, device: MaybeJson, held: MaybeJson, dip: f64) -> Jv {
    let d = spotify::duck_plan(waiting, player.0.as_ref(), device.0.as_ref(), held.0.as_ref(), dip);
    let held = d.held.map_or(Jv::Undef, Jv::Json);
    match d.volume {
        Some(v) => Jv::Obj(vec![("op", s("volume")), ("volume", Jv::Num(v)), ("held", held)]),
        None => Jv::Obj(vec![("op", Jv::Json(Value::Null)), ("held", held)]),
    }
}

#[napi(js_name = "appSpotifyDemoPlayer")]
pub fn app_spotify_demo_player() -> Json {
    Json(spotify::demo_player())
}

#[napi(js_name = "appSpotifyDemoSettings")]
pub fn app_spotify_demo_settings() -> Json {
    Json(spotify::demo_settings())
}

#[napi(js_name = "appSpotifyDemoCommand")]
pub fn app_spotify_demo_command(p: Json, op: MaybeJson, value: MaybeJson) -> Json {
    let mut p = p.0;
    spotify::demo_command(&mut p, op.0.as_ref().and_then(Value::as_str), value.0.as_ref());
    Json(p)
}

/// One Spotify connection, for the twin's createSpotify.
#[napi(js_name = "AppSpotify")]
pub struct AppSpotify {
    inner: spotify::Spotify,
}

#[napi]
impl AppSpotify {
    #[napi(constructor)]
    pub fn new(file: String, port: MaybeJson) -> Self {
        AppSpotify { inner: spotify::Spotify::new(&file, port.0.unwrap_or(Value::String("undefined".into()))) }
    }

    #[napi]
    pub fn settings(&self) -> Json {
        Json(self.inner.settings())
    }

    /// { ok }, or { error: { spotify } } or { error: { fs } }.
    #[napi(js_name = "setApp")]
    pub fn set_app(&mut self, client_id: String) -> Jv {
        match self.inner.set_app(&client_id) {
            Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
            Err(spotify::Failed::Spotify(e)) => Jv::Obj(vec![("error", Jv::Obj(vec![("spotify", spotify_error(&e))]))]),
            Err(spotify::Failed::Fs(e)) => Jv::Obj(vec![("error", Jv::Obj(vec![("fs", fs_error(&e))]))]),
        }
    }

    #[napi]
    pub fn set(&mut self, device: MaybeJson, duck: MaybeJson) -> Jv {
        match self.inner.set(device.0.as_ref(), duck.0.as_ref()) {
            Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
            Err(e) => Jv::Obj(vec![("error", Jv::Obj(vec![("fs", fs_error(&e))]))]),
        }
    }

    #[napi]
    pub fn begin(&mut self, now: f64) -> Jv {
        match self.inner.begin(now) {
            Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
            Err(e) => Jv::Obj(vec![("error", Jv::Obj(vec![("spotify", spotify_error(&e))]))]),
        }
    }

    #[napi]
    pub fn start(&mut self, kind: String, args: Json, now: f64) -> Jv {
        let (job, ask) = self.inner.start(&kind, &args.0, now);
        ask_jv(job, ask)
    }

    /// `got` is { value }, { threw: message } or { res: { ok, status, body, retryAfter } }.
    #[napi]
    pub fn resume(&mut self, job: Json, got: Json, now: f64) -> Jv {
        let g = got.0;
        let got = if let Some(m) = g.get("threw") {
            spotify::Got::Threw(xenonet_base::text::js_string(m))
        } else if let Some(r) = g.get("res") {
            spotify::Got::Response {
                ok: r.get("ok") == Some(&Value::Bool(true)),
                status: r.get("status").and_then(Value::as_f64).unwrap_or(0.0) as u16,
                body: r.get("body").cloned().unwrap_or(Value::Null),
                retry_after: r.get("retryAfter").filter(|x| !x.is_null()).map(xenonet_base::text::js_string),
            }
        } else {
            spotify::Got::Value(g.get("value").cloned().unwrap_or(Value::Null))
        };
        let (job, ask) = self.inner.resume(job.0, got, now);
        ask_jv(job, ask)
    }
}

// ----- tasks: async Rust whose waiting the twin does -----

use xenonet_app::task;

fn failure_jv(f: task::Failure) -> Jv {
    match f {
        task::Failure::Rethrow => Jv::Obj(vec![("rethrow", Jv::Bool(true))]),
        task::Failure::Error { message, fields, kind } => {
            let mut m = serde_json::Map::new();
            m.insert("message".into(), Value::String(message));
            m.insert("kind".into(), Value::String(kind.into()));
            for (k, v) in fields {
                m.insert(k, v);
            }
            Jv::Json(Value::Object(m))
        }
    }
}

/// A Rust task for the twin's drive() (js/task.js): next() runs it until it
/// asks for something ({ ask }) or ends ({ done } or { fail }).
#[napi(js_name = "AppTask")]
pub struct AppTask {
    task: task::Task,
    started: bool,
}

impl AppTask {
    pub(crate) fn of(task: task::Task) -> Self {
        AppTask { task, started: false }
    }
}

#[napi]
impl AppTask {
    /// `threw` describes the error the last request threw; else `ok` is
    /// what it answered.
    #[napi]
    pub fn next(&mut self, ok: MaybeJson, threw: Option<Json>) -> Jv {
        let reply = if self.started {
            Some(match threw {
                Some(e) => Err(e.0),
                None => Ok(ok.0.unwrap_or(Value::Null)),
            })
        } else {
            None
        };
        self.started = true;
        match self.task.step(reply) {
            task::Outcome::Ask(q) => Jv::Obj(vec![("ask", Jv::Json(q))]),
            task::Outcome::Done(v) => Jv::Obj(vec![("done", Jv::Json(v))]),
            task::Outcome::Failed(f) => Jv::Obj(vec![("fail", failure_jv(f))]),
        }
    }
}

// ----- control -----

use xenonet_app::control;

#[napi(js_name = "appUserDataDir")]
pub fn app_user_data_dir(name: String, platform: String, appdata: Option<String>, xdg_config_home: Option<String>, home: String) -> String {
    control::user_data_dir(&name, &platform, appdata.as_deref(), xdg_config_home.as_deref(), &home)
}

#[napi(js_name = "appParseProcesses")]
pub fn app_parse_processes(text: String) -> Json {
    Json(control::parse_processes(&text))
}

#[napi(js_name = "appElectronCommand")]
pub fn app_electron_command(root: String, home: String, platform: String, software_rendering: bool) -> Json {
    let (command, args) = control::electron_command(&root, &home, &platform, software_rendering);
    Json(json!({ "command": command, "args": args }))
}

#[napi(js_name = "appLockHolder")]
pub fn app_lock_holder(dir: String) -> AppTask {
    AppTask::of(task::Task::new(move |io| async move { control::lock_holder(&io, &dir).await }))
}

#[napi(js_name = "appControlCheck")]
pub fn app_control_check(id: String) -> Option<String> {
    let o = control::Opts {
        port: None,
        user_data: Some(""),
        log_dir: Some(""),
        settle_ms: 0.0,
        term_ms: 0.0,
        root: "",
        tmpdir: "",
        platform: "",
        appdata: None,
        xdg_config_home: None,
        home: "",
    };
    control::Instance::new(&id, &o).err()
}

/// One instance of XenoNet Dev, for the twin's xenonet().
#[napi(js_name = "AppControl")]
pub struct AppControl {
    inst: control::Instance,
}

#[napi]
impl AppControl {
    #[napi(constructor)]
    pub fn new(id: String, opts: Json) -> Result<Self> {
        let o = opts.0;
        let text = |k: &str| o.get(k).and_then(Value::as_str).map(String::from);
        let (user_data, log_dir, root, tmpdir, platform, appdata, xdg, home) = (
            text("userData"),
            text("logDir"),
            text("root").unwrap_or_default(),
            text("tmpdir").unwrap_or_default(),
            text("platform").unwrap_or_default(),
            text("appdata"),
            text("xdgConfigHome"),
            text("home").unwrap_or_default(),
        );
        let num = |k: &str, d: f64| o.get(k).and_then(Value::as_f64).unwrap_or(d);
        let opts = control::Opts {
            port: o.get("port").cloned(),
            user_data: user_data.as_deref(),
            log_dir: log_dir.as_deref(),
            settle_ms: num("settleMs", 15000.0),
            term_ms: num("termMs", 5000.0),
            root: &root,
            tmpdir: &tmpdir,
            platform: &platform,
            appdata: appdata.as_deref(),
            xdg_config_home: xdg.as_deref(),
            home: &home,
        };
        control::Instance::new(&id, &opts).map(|inst| AppControl { inst }).map_err(Error::from_reason)
    }

    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inst.name.clone()
    }

    #[napi(getter)]
    pub fn base(&self) -> String {
        self.inst.base.clone()
    }

    /// A task for one of the instance's methods, with its arguments.
    #[napi]
    pub fn task(&self, kind: String, args: Json) -> AppTask {
        let i = self.inst.clone();
        let a = args.0;
        let num = move |a: &Value, k: &str, d: f64| a.get(k).and_then(Value::as_f64).unwrap_or(d);
        AppTask::of(task::Task::new(move |io| async move {
            let opts = a.get("opts");
            match kind.as_str() {
                "health" => Ok(Value::Bool(i.health(&io).await)),
                "call" => i.call(&io, a.get("body").unwrap_or(&Value::Null), num(&a, "wait", 20000.0)).await,
                "ping" => i.ping(&io).await,
                "ready" => i.ready(&io, num(&a, "timeout", 60000.0)).await,
                "launch" => i.launch(&io, a.get("show"), a.get("restart") == Some(&Value::Bool(true)), num(&a, "timeout", 120000.0)).await,
                "quit" => i.quit(&io, num(&a, "timeout", 20000.0)).await,
                "driver" => {
                    let args = match a.get("args") {
                        Some(Value::Array(x)) => x.clone(),
                        _ => Vec::new(),
                    };
                    i.driver(&io, a.get("method").unwrap_or(&Value::Null), &args).await
                }
                "simulate" => i.simulate(&io, a.get("kind").unwrap_or(&Value::Null), a.get("a"), a.get("b")).await,
                "screenshot" | "input" | "window" | "record" | "logs" => i.call(&io, &control::Instance::body(&kind, None, opts), 20000.0).await,
                "eval" => i.call(&io, &control::Instance::body("eval", Some(("expression", a.get("expression").cloned().unwrap_or(Value::Null))), opts), 20000.0).await,
                "api" => i.api(&io, a.get("route").and_then(Value::as_str).unwrap_or("")).await,
                _ => Err(task::Failure::msg(format!("no control task {kind}"))),
            }
        }))
    }
}

// ----- system-stats -----

use xenonet_app::system_stats as stats;

#[napi(js_name = "appSystemConstants")]
pub fn app_system_constants() -> Json {
    Json(
        json!({ "history": stats::HISTORY, "freshMs": stats::FRESH_MS, "idleMs": stats::IDLE_MS, "staleMs": stats::STALE_MS, "nvidiaQuery": stats::NVIDIA_QUERY, "nvidiaArgs": stats::nvidia_args() }),
    )
}

#[napi(js_name = "appCpuModel")]
pub fn app_cpu_model(model: String) -> Option<String> {
    stats::cpu_model(&model)
}

#[napi(js_name = "appParseNvidia")]
pub fn app_parse_nvidia(line: String) -> Option<Json> {
    stats::parse_nvidia(&line).map(Json)
}

#[napi(js_name = "appParseIoreg")]
pub fn app_parse_ioreg(text: String, total_mem: Option<f64>) -> Option<Json> {
    stats::parse_ioreg(&text, total_mem).map(Json)
}

fn adapters_of(v: &Value) -> Vec<(String, f64)> {
    match v {
        Value::Array(a) => a.iter().map(|x| (x.get("name").map(xenonet_base::text::js_string).unwrap_or_default(), x.get("mem").and_then(Value::as_f64).unwrap_or(0.0))).collect(),
        _ => Vec::new(),
    }
}

/// What a line changed: { adapters } or { latest }, or nothing.
#[napi(js_name = "appParseWindowsGpu")]
pub fn app_parse_windows_gpu(line: String, adapters: Json) -> Json {
    Json(match stats::parse_windows_gpu(&line, &adapters_of(&adapters.0)) {
        stats::WindowsLine::Adapters(a) => {
            json!({ "adapters": a.into_iter().map(|(name, mem)| json!({ "name": name, "mem": xenonet_base::json::number(mem) })).collect::<Vec<_>>() })
        }
        stats::WindowsLine::Latest(l) => json!({ "latest": l }),
        stats::WindowsLine::Nothing => json!({}),
    })
}

#[napi(js_name = "appWindowsGpuScript")]
pub fn app_windows_gpu_script(parent: f64) -> String {
    stats::windows_gpu_script(parent)
}

#[napi(js_name = "appWindowsGpuCommand")]
pub fn app_windows_gpu_command(system_root: Option<String>, parent: f64) -> Json {
    Json(json!({ "file": stats::windows_powershell(system_root.as_deref()), "args": stats::windows_args(parent) }))
}

#[napi(js_name = "appFindNvidiaSmi")]
pub fn app_find_nvidia_smi(
    platform: String,
    path_var: String,
    pathext: Option<String>,
    system_root: Option<String>,
    program_files: Option<String>,
    exists: Function<'_, String, bool>,
) -> Option<String> {
    stats::find_nvidia_smi(&platform, &path_var, pathext.as_deref(), system_root.as_deref(), program_files.as_deref(), &mut |f| exists.call(f.to_string()).unwrap_or(false))
}

#[napi(js_name = "appDrmCards")]
pub fn app_drm_cards(names: Vec<String>) -> Vec<String> {
    stats::drm_cards(&names)
}

#[napi(js_name = "appAmdCard")]
pub fn app_amd_card(index: u32, busy: Option<String>, used: Option<String>, total: Option<String>, label: Option<String>) -> Json {
    Json(stats::amd_card(index as usize, busy.as_deref(), used.as_deref(), total.as_deref(), label.as_deref()))
}

#[napi(js_name = "appDemoSystemReading")]
pub fn app_demo_system_reading(at: f64) -> Json {
    Json(stats::demo_reading(at))
}

/// A program kept running for GPU readings: nvidia-smi ("nvidia") or
/// Windows' counter loop ("windows").
#[napi(js_name = "AppGpuStream")]
pub struct AppGpuStream {
    windows: bool,
    proc: stats::Streamed,
    nvidia: stats::NvidiaCards,
    counters: stats::WindowsReadings,
}

#[napi]
impl AppGpuStream {
    #[napi(constructor)]
    pub fn new(kind: String) -> Self {
        AppGpuStream { windows: kind == "windows", proc: stats::Streamed::default(), nvidia: stats::NvidiaCards::default(), counters: stats::WindowsReadings::default() }
    }

    #[napi(getter)]
    pub fn dead(&self) -> bool {
        self.proc.dead()
    }

    /// The generation to start the program as, or null to leave it.
    #[napi]
    pub fn start(&mut self, now: f64) -> Option<u32> {
        self.proc.start(now)
    }

    #[napi(js_name = "spawnFailed")]
    pub fn spawn_failed(&mut self, now: f64) {
        self.proc.spawn_failed(now);
    }

    #[napi]
    pub fn data(&mut self, chunk: String, now: f64) {
        for line in self.proc.data(&chunk) {
            if self.windows {
                self.counters.line(&line, now);
            } else {
                self.nvidia.line(&line, now);
            }
        }
    }

    #[napi]
    pub fn gone(&mut self, generation: u32, now: f64) {
        self.proc.gone(generation, now);
    }

    /// Whether there was a program to kill.
    #[napi]
    pub fn stop(&mut self) -> bool {
        self.proc.stop()
    }

    #[napi]
    pub fn read(&self, now: f64) -> Json {
        Json(Value::Array(if self.windows { self.counters.read(now) } else { self.nvidia.read(now) }))
    }
}

/// A reading run each time it's asked for, at most every couple of seconds.
#[napi(js_name = "AppPolled")]
pub struct AppPolled {
    inner: stats::Polled,
}

#[napi]
impl AppPolled {
    #[napi(constructor)]
    pub fn new() -> Self {
        AppPolled { inner: stats::Polled::default() }
    }

    /// The last reading while it's fresh, else undefined.
    #[napi]
    pub fn fresh(&self, now: f64) -> Jv {
        self.inner.fresh(now).map_or(Jv::Undef, Jv::Json)
    }

    #[napi]
    pub fn set(&mut self, v: MaybeJson, now: f64) {
        self.inner.set(v.0.unwrap_or(Value::Null), now);
    }
}

#[napi(object)]
pub struct CpuTimes {
    pub user: Option<f64>,
    pub nice: Option<f64>,
    pub sys: Option<f64>,
    pub irq: Option<f64>,
    pub idle: Option<f64>,
}

fn times_of(t: Vec<CpuTimes>) -> Vec<stats::Times> {
    let n = |x: Option<f64>| x.unwrap_or(f64::NAN);
    t.into_iter().map(|c| stats::Times { user: n(c.user), nice: n(c.nice), sys: n(c.sys), irq: n(c.irq), idle: n(c.idle) }).collect()
}

/// The CPU and memory side of the system panel.
#[napi(js_name = "AppSystemStats")]
pub struct AppSystemStats {
    inner: stats::Stats,
}

#[napi]
impl AppSystemStats {
    #[napi(constructor)]
    pub fn new() -> Self {
        AppSystemStats { inner: stats::Stats::default() }
    }

    #[napi(js_name = "hasPrev")]
    pub fn has_prev(&self) -> bool {
        self.inner.has_prev()
    }

    #[napi(js_name = "setPrev")]
    pub fn set_prev(&mut self, times: Vec<CpuTimes>) {
        self.inner.set_prev(&times_of(times));
    }

    #[napi]
    pub fn fresh(&self, now: f64) -> bool {
        self.inner.fresh(now)
    }

    #[napi]
    pub fn cpu(&mut self, model: Option<String>, speed: Option<f64>, threads: u32, times: Vec<CpuTimes>) -> Json {
        Json(self.inner.cpu(model.as_deref(), speed, threads as usize, &times_of(times)))
    }

    #[napi]
    pub fn reading(&mut self, at: f64, platform: String, arch: Option<String>, uptime: f64, cpu: Json, total: f64, free: f64, gpus: MaybeJson, reader: Option<String>) -> Json {
        let gpus = match gpus.0 {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        };
        Json(self.inner.reading(at, &platform, arch.as_deref(), uptime, cpu.0, total, free, gpus, reader.as_deref()))
    }
}

// ----- themes -----

use xenonet_app::{themefiles, themes};

#[napi(js_name = "appDefaultTheme")]
pub fn app_default_theme() -> &'static str {
    themes::DEFAULT_THEME
}

#[napi(js_name = "appBuiltInThemes")]
pub fn app_built_in_themes() -> Vec<String> {
    themes::built_in_ids()
}

#[napi(js_name = "appThemeStyles")]
pub fn app_theme_styles() -> Json {
    Json(Value::Object(themes::styles().into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect()))
}

#[napi(js_name = "appThemeMerge")]
pub fn app_theme_merge(base: MaybeJson, over: MaybeJson) -> Json {
    Json(themes::merge(base.0.as_ref(), over.0.as_ref()))
}

#[napi(js_name = "appThemeConform")]
pub fn app_theme_conform(base: MaybeJson, over: MaybeJson, at: String) -> Jv {
    match base.0 {
        // conform(undefined, over): JS spread nothing in, and gave back
        // undefined itself when over isn't an object.
        None if !matches!(over.0, Some(Value::Object(_))) => Jv::Undef,
        b => Jv::Json(themes::conform(&b.unwrap_or(Value::Null), over.0.as_ref(), &at)),
    }
}

#[napi(js_name = "appThemeComplete")]
pub fn app_theme_complete(theme: MaybeJson) -> Json {
    Json(themes::complete(theme.0.as_ref()))
}

#[napi(js_name = "appSonarGap")]
pub fn app_sonar_gap(every: MaybeJson) -> Option<f64> {
    themes::sonar_gap(every.0.as_ref())
}

#[napi(js_name = "appThemes")]
pub fn app_themes(custom: MaybeJson) -> Json {
    let custom = match custom.0 {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    Json(Value::Object(themes::themes(&custom)))
}

#[napi(js_name = "appContrast")]
pub fn app_contrast(a: String, b: String) -> f64 {
    themes::contrast(&a, &b)
}

#[napi(js_name = "appColorDistance")]
pub fn app_color_distance(a: String, b: String) -> f64 {
    themes::color_distance(&a, &b)
}

#[napi(js_name = "appRingColors")]
pub fn app_ring_colors(theme: Json) -> Vec<String> {
    themes::ring_colors(&theme.0)
}

#[napi(js_name = "appMathCbrt")]
pub fn app_math_cbrt(x: f64) -> f64 {
    themes::js_cbrt(x)
}

#[napi(js_name = "appReadThemes")]
pub fn app_read_themes(dir: String) -> Json {
    let (all, problems) = themefiles::read_themes(&dir);
    Json(json!({ "themes": Value::Object(all), "problems": problems }))
}

#[napi(js_name = "appCopyTheme")]
pub fn app_copy_theme(dir: String, from: Option<String>, id: String, name: MaybeJson) -> Jv {
    fs_answer(themefiles::copy_theme(&dir, from.as_deref(), &id, name.0.as_ref()).map(s))
}

// ----- demo -----

use xenonet_app::demo;

/// What the demo asks of JS: [gitAvailable, local, claudeId, githubId,
/// gitId, random, randomUUID, now], each a function.
struct JsAround<'a> {
    git: Option<Function<'a, (), bool>>,
    local: Option<Function<'a, f64, Vec<f64>>>,
    claude: Option<Function<'a, FnArgs<(String, String)>, String>>,
    github: Option<Function<'a, FnArgs<(String, String)>, String>>,
    git_id: Option<Function<'a, String, String>>,
    random: Function<'a, (), f64>,
    uuid: Function<'a, (), String>,
    now: Function<'a, (), f64>,
    failed: Option<Error>,
}

impl JsAround<'_> {
    fn keep<T: Default>(&mut self, r: Result<T>) -> T {
        r.unwrap_or_else(|e| {
            self.failed.get_or_insert(e);
            T::default()
        })
    }
}

fn missing<T>() -> Result<T> {
    Err(Error::from_reason("the demo needs that only when it makes its world"))
}

impl demo::Around for JsAround<'_> {
    fn git_available(&mut self) -> bool {
        let r = self.git.as_ref().map_or_else(missing, |f| f.call(()));
        self.keep(r)
    }
    fn claude_id(&mut self, account: &str, org: &str) -> String {
        let r = self.claude.as_ref().map_or_else(missing, |f| f.call((account.to_string(), org.to_string()).into()));
        self.keep(r)
    }
    fn github_id(&mut self, host: &str, login: &str) -> String {
        let r = self.github.as_ref().map_or_else(missing, |f| f.call((host.to_string(), login.to_string()).into()));
        self.keep(r)
    }
    fn git_id(&mut self, email: &str) -> String {
        let r = self.git_id.as_ref().map_or_else(missing, |f| f.call(email.to_string()));
        self.keep(r)
    }
    fn local(&mut self, ms: f64) -> [i64; 5] {
        let r = self.local.as_ref().map_or_else(missing, |f| f.call(ms));
        let v = self.keep(r);
        let at = |i: usize| v.get(i).copied().unwrap_or(0.0) as i64;
        [at(0), at(1), at(2), at(3), at(4)]
    }
    fn random(&mut self) -> f64 {
        let r = self.random.call(());
        self.keep(r)
    }
    fn uuid(&mut self) -> String {
        let r = self.uuid.call(());
        self.keep(r)
    }
    fn now(&mut self) -> f64 {
        let r = self.now.call(());
        self.keep(r)
    }
}

/// The demo's made-up world and sessions, for the twin of core/demo.js.
#[napi(js_name = "AppDemo")]
pub struct AppDemo {
    inner: demo::Demo,
}

#[napi]
impl AppDemo {
    #[napi(constructor)]
    pub fn new(tmpdir: String, port: String, today: String) -> Self {
        AppDemo { inner: demo::Demo::new(&tmpdir, &port, &today) }
    }

    #[napi]
    pub fn world(&self) -> Json {
        Json(self.inner.world.to_value())
    }

    #[napi(js_name = "createWorld")]
    pub fn create_world(
        &mut self,
        now: f64,
        platform: String,
        git: Function<'_, (), bool>,
        local: Function<'_, f64, Vec<f64>>,
        claude: Function<'_, FnArgs<(String, String)>, String>,
        github: Function<'_, FnArgs<(String, String)>, String>,
        git_id: Function<'_, String, String>,
        random: Function<'_, (), f64>,
        uuid: Function<'_, (), String>,
        clock: Function<'_, (), f64>,
    ) -> Result<Jv> {
        let mut around = JsAround { git: Some(git), local: Some(local), claude: Some(claude), github: Some(github), git_id: Some(git_id), random, uuid, now: clock, failed: None };
        let r = self.inner.create_world(now, &mut around, &platform);
        if let Some(e) = around.failed {
            return Err(e);
        }
        Ok(fs_answer(r.map(Jv::Json)))
    }

    /// The second folder's repos, made the first time something asks:
    /// [{ dir, steps: [git's arguments…], dirty: [[file, text]…] }].
    #[napi(js_name = "clientRepos")]
    pub fn client_repos(&self) -> Json {
        let steps = demo::repo_steps();
        Json(Value::Array(
            self.inner
                .client_repos
                .iter()
                .map(|r| json!({ "dir": r.dir, "steps": steps, "dirty": r.dirty.iter().map(|(rel, t)| json!([nodepath::NATIVE.join(&[&r.dir, rel]), t])).collect::<Vec<_>>() }))
                .collect(),
        ))
    }

    #[napi]
    pub fn sessions(&self, app: String) -> Jv {
        fs_answer(self.inner.sessions_of(&app).map(|v| Jv::Json(Value::Array(v))))
    }

    #[napi(js_name = "sessionList")]
    pub fn session_list(&self) -> Json {
        Json(Value::Array(self.inner.session_list()))
    }

    #[napi(js_name = "nativeProjects")]
    pub fn native_projects(&self) -> Json {
        Json(self.inner.native_projects())
    }

    #[napi]
    pub fn transcript(&self, raw_id: String) -> Jv {
        answer(self.inner.transcript(&raw_id).map(Jv::Json))
    }

    #[napi(js_name = "stepLive")]
    pub fn step_live(&mut self, k: u32, now: f64, random: Function<'_, (), f64>, uuid: Function<'_, (), String>, clock: Function<'_, (), f64>) -> Result<()> {
        let mut around = JsAround { git: None, local: None, claude: None, github: None, git_id: None, random, uuid, now: clock, failed: None };
        self.inner.step_live(k as usize, now, &mut around);
        around.failed.map_or(Ok(()), Err)
    }
}

#[napi(js_name = "appDemoClaudeAccount")]
pub fn app_demo_claude_account(project: Option<String>) -> Json {
    Json(demo::demo_claude_account(project.as_deref()))
}

#[napi(js_name = "appDemoPicks")]
pub fn app_demo_picks(outline: MaybeJson) -> Json {
    Json(demo::demo_picks(&outline.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "appDemoNarration")]
pub fn app_demo_narration() -> &'static str {
    demo::NARRATION
}
