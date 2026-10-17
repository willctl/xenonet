//! xenonet-app for the twins of core/config.js, instances.js, version.js,
//! environments.js, demo-root.js and the other small modules of core/.

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
    context_dir: Function<'a, (), Option<String>>,
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
    fn context_dir(&mut self) -> Option<String> {
        let r = self.context_dir.call(());
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
#[allow(clippy::too_many_arguments)]
pub fn app_config(
    argv: Vec<String>,
    env: Json,
    home: String,
    platform: String,
    world: Function<(), Json>,
    context_dir: Function<(), Option<String>>,
    context_server: Function<Option<String>, Json>,
    claude_desktop_dir: Function<(), Option<String>>,
) -> Result<Jv> {
    let env = match env.0 {
        Value::Object(m) => m,
        _ => Default::default(),
    };
    let mut finds = JsFinds { world, context_dir, context_server, claude_desktop_dir, failed: None };
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

#[napi(js_name = "appCurrentRuntime")]
pub fn app_current_runtime(xenonet_exec: Option<String>, exec_path: String, electron: bool, path_var: Option<String>) -> Json {
    let r = command::current_runtime(xenonet_exec.as_deref(), &exec_path, electron, path_var.as_deref());
    Json(json!({ "runtime": r.runtime, "runAsNode": r.run_as_node }))
}

#[napi(js_name = "appCommandScript")]
pub fn app_command_script(runtime: String, entry: String, run_as_node: bool, platform: String) -> Jv {
    answer(command::command_script(&runtime, &entry, run_as_node, &platform).map(s))
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
        _ => Default::default(),
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
pub fn app_install_command(dir: String, runtime: String, entry: String, run_as_node: bool, force: bool, path_var: Option<String>, platform: String) -> Jv {
    fs_answer(command::install_command(&dir, &runtime, &entry, run_as_node, force, path_var.as_deref(), &platform).map(state_jv))
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
    Json(json!({ "url": decide::DECISIONS_URL, "model": decide::DEFAULT_MODEL, "keyTarget": decide::KEY_TARGET, "key": decide::KEY, "keyComment": decide::KEY_COMMENT, "timeoutMs": decide::TIMEOUT_MS }))
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
pub fn app_mcp_begin(server: String, sessions: Json, settings: Json, now: f64, read: Option<Function<Option<String>, Json>>) -> Result<Jv> {
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
pub fn app_mcp_step(state: Json, answer: Json, settings: Json, read: Option<Function<Option<String>, Json>>) -> Result<Jv> {
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
        _ => Default::default(),
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
        spotify::Ask::SecretSet { target, value, comment } => Jv::Obj(vec![("secret", Jv::Obj(vec![("op", s("set")), ("target", s(target)), ("value", s(value)), ("comment", s(comment))]))]),
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
    let o = control::Opts { port: None, user_data: Some(""), log_dir: Some(""), settle_ms: 0.0, term_ms: 0.0, root: "", tmpdir: "", platform: "", appdata: None, xdg_config_home: None, home: "" };
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
        let (user_data, log_dir, root, tmpdir, platform, appdata, xdg, home) = (text("userData"), text("logDir"), text("root").unwrap_or_default(), text("tmpdir").unwrap_or_default(), text("platform").unwrap_or_default(), text("appdata"), text("xdgConfigHome"), text("home").unwrap_or_default());
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
