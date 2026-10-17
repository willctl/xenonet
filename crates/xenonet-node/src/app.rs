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
