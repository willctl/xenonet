//! xenonet-components for the twins of core/components/.

use crate::app::{answer, fs_answer, fs_error, s, AppTask};
use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use xenonet_app::task::Task;
use xenonet_components::{console, console_policy, install, packages, protocol, rpc, runtime, updates};

fn env_of(env: Json) -> Map<String, Value> {
    match env.0 {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// { platform, arch, home }: the machine's own.
fn machine_of(m: &Json) -> runtime::Machine {
    let t = |k: &str| m.0.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    runtime::Machine { platform: t("platform"), arch: t("arch"), home: t("home") }
}

/// The machine, the launcher's source and process.env's LOCALAPPDATA.
fn ctx_of(c: &Json) -> install::Ctx {
    install::Ctx {
        machine: machine_of(c),
        launcher_source: c.0.get("launcherSource").map(xenonet_base::text::js_string).unwrap_or_default(),
        local_app_data: c.0.get("localAppData").and_then(Value::as_str).map(String::from),
    }
}

#[napi(js_name = "compConstants")]
pub fn comp_constants() -> Json {
    Json(
        json!({ "serviceApi": protocol::api_value(protocol::SERVICE_API), "consoleApi": protocol::api_value(protocol::CONSOLE_API), "capabilities": protocol::CAPABILITIES, "components": packages::COMPONENTS, "maxLine": rpc::MAX_LINE }),
    )
}

#[napi(js_name = "compNegotiate")]
pub fn comp_negotiate(local: MaybeJson, remote: MaybeJson) -> Jv {
    answer(protocol::negotiate(local.0.as_ref(), remote.0.as_ref()).map(|v| Jv::Num(v as f64)))
}

#[napi(js_name = "compConsolePolicy")]
pub fn comp_console_policy(op: MaybeJson, method: MaybeJson, args: MaybeJson, production: bool) -> Option<String> {
    console_policy::console_policy(op.0.as_ref(), method.0.as_ref(), args.0.as_ref(), production).err()
}

/// The framing and rules of one connection, for the twin's rpc().
#[napi(js_name = "CompRpc")]
pub struct CompRpc {
    inner: rpc::Rpc,
}

#[napi]
impl CompRpc {
    #[napi(constructor)]
    pub fn new(max_line: f64, events: Vec<String>) -> Self {
        CompRpc { inner: rpc::Rpc::new(if max_line.is_finite() && max_line >= 0.0 { max_line as usize } else { rpc::MAX_LINE }, events) }
    }

    #[napi(js_name = "nextId")]
    pub fn next_id(&mut self) -> f64 {
        self.inner.next_id()
    }

    #[napi]
    pub fn data(&mut self, chunk: String) -> Jv {
        Jv::Arr(
            self.inner
                .data(&chunk)
                .into_iter()
                .map(|m| match m {
                    rpc::Incoming::Reply { id, result, error } => Jv::Obj(vec![("reply", Jv::Json(id)), ("result", Jv::Json(result)), ("error", error.map_or(Jv::Undef, s))]),
                    rpc::Incoming::Request { id, method, params } => Jv::Obj(vec![("id", Jv::Num(id)), ("method", s(method)), ("params", Jv::Json(params))]),
                    rpc::Incoming::Event { name, value } => Jv::Obj(vec![("event", s(name)), ("value", Jv::Json(value))]),
                    rpc::Incoming::Destroy => Jv::Obj(vec![("destroy", Jv::Bool(true))]),
                })
                .collect(),
        )
    }

    #[napi]
    pub fn line(&self, value: Json, gone: bool, writable: f64) -> Jv {
        answer(self.inner.line(&value.0, gone, writable).map(s).map_err(String::from))
    }
}

#[napi(js_name = "compRuntimeDir")]
pub fn comp_runtime_dir(env: Json, machine: Json) -> String {
    runtime::runtime_dir(&env_of(env), &machine_of(&machine))
}

#[napi(js_name = "compBinName")]
pub fn comp_bin_name(platform: String) -> String {
    runtime::bin_name(&platform).into()
}

#[napi(js_name = "compLauncherBin")]
pub fn comp_launcher_bin(env: Json, machine: Json) -> String {
    runtime::launcher_bin(&env_of(env), &machine_of(&machine))
}

#[napi(js_name = "compXenonetBin")]
pub fn comp_xenonet_bin(env: Json, machine: Json, this: String) -> String {
    runtime::xenonet_bin(&env_of(env), &machine_of(&machine), &this)
}

#[napi(js_name = "compDescriptorFile")]
pub fn comp_descriptor_file(env: Json, machine: Json) -> String {
    runtime::descriptor_file(&env_of(env), &machine_of(&machine))
}

#[napi(js_name = "compReadDescriptor")]
pub fn comp_read_descriptor(env: Json, machine: Json) -> Jv {
    fs_answer(runtime::read_descriptor(&env_of(env), &machine_of(&machine)).map(|(port, token)| Jv::Obj(vec![("port", Jv::Num(port)), ("token", s(token))])))
}

#[napi(js_name = "compHello")]
pub fn comp_hello(token: String, role: String) -> Json {
    Json(runtime::hello(&token, &role))
}

#[napi(js_name = "compEnsureService")]
pub fn comp_ensure_service(root: String, exec: String, env: Json, timeout: f64, machine: Json) -> AppTask {
    let (env, m) = (env_of(env), machine_of(&machine));
    AppTask::of(Task::new(move |io| async move { runtime::ensure_service(&io, &root, &exec, &env, timeout, &m).await }))
}

#[napi(js_name = "compDigest")]
pub fn comp_digest(data: Buffer) -> String {
    packages::digest(&data)
}

#[napi(js_name = "compNewer")]
pub fn comp_newer(a: MaybeJson, b: MaybeJson) -> Jv {
    answer(packages::newer(a.0.as_ref(), b.0.as_ref()).map(Jv::Bool))
}

#[napi(js_name = "compPackageInfo")]
pub fn comp_package_info(value: MaybeJson, platform: String, arch: String) -> Jv {
    answer(packages::package_info(value.0.as_ref(), &platform, &arch).map(Jv::Json))
}

#[napi(js_name = "compComponentsDir")]
pub fn comp_components_dir(env: Json, machine: Json) -> String {
    packages::components_dir(&env_of(env), &machine_of(&machine))
}

#[napi(js_name = "compCurrent")]
pub fn comp_current(component: String, env: Json, machine: Json) -> Jv {
    fs_answer(packages::current_component(&component, &env_of(env), &machine_of(&machine)).map(|v| Jv::Json(v.unwrap_or(Value::Null))))
}

#[napi(js_name = "compSelect")]
pub fn comp_select(component: String, value: Json, env: Json, machine: Json) -> Jv {
    fs_answer(packages::select_component(&component, &value.0, &env_of(env), &machine_of(&machine)).map(|()| Jv::Undef))
}

#[napi(js_name = "compStage")]
pub fn comp_stage(file: String, expected: Json, env: Json, machine: Json) -> AppTask {
    let (expected, env, m) = (expected.0, env_of(env), machine_of(&machine));
    AppTask::of(Task::new(move |io| async move { packages::stage_package(&io, &file, &expected, &env, &m).await }))
}

#[napi(js_name = "compLock")]
pub fn comp_lock(env: Json, machine: Json) -> AppTask {
    let (env, m) = (env_of(env), machine_of(&machine));
    AppTask::of(Task::new(move |io| async move { packages::lock(&io, &env, &m).await.map(Value::String) }))
}

#[napi(js_name = "compUnlock")]
pub fn comp_unlock(file: String) -> Jv {
    fs_answer(packages::unlock(&file).map(|()| Jv::Undef))
}

#[napi(js_name = "compLauncherVersion")]
pub fn comp_launcher_version(text: Option<String>) -> f64 {
    install::launcher_version(text.as_deref())
}

#[napi(js_name = "compWriteLauncher")]
pub fn comp_write_launcher(entry: String, source: String) -> Jv {
    fs_answer(install::write_launcher(&entry, &source).map(Jv::Bool))
}

fn failed(f: xenonet_app::task::Failure) -> Jv {
    match f {
        xenonet_app::task::Failure::Error { message, fields, .. } => {
            let mut e = vec![("message", s(message))];
            if let Some(c) = fields.get("code") {
                e.push(("code", Jv::Json(c.clone())));
            }
            Jv::Obj(vec![("error", Jv::Obj(e))])
        }
        xenonet_app::task::Failure::Rethrow => Jv::Obj(vec![("error", Jv::Obj(vec![("message", s("a request failed"))]))]),
    }
}

fn settled(r: std::result::Result<Jv, xenonet_app::task::Failure>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", v)]),
        Err(f) => failed(f),
    }
}

#[napi(js_name = "compInstallLauncher")]
pub fn comp_install_launcher(env: Json, ctx: Json) -> Jv {
    settled(install::install_launcher(&env_of(env), &ctx_of(&ctx)).map(|v| Jv::Json(v.unwrap_or(Value::Null))))
}

#[napi(js_name = "compCheckRuntime")]
pub fn comp_check_runtime(staged: Json, platform: String) -> Jv {
    settled(install::check_runtime(&staged.0, &platform).map(|()| Jv::Undef))
}

#[napi(js_name = "compInstalledRuntime")]
pub fn comp_installed_runtime(env: Json, ctx: Json) -> Jv {
    settled(install::installed_runtime(&env_of(env), &ctx_of(&ctx)).map(Jv::Json))
}

#[napi(js_name = "compRecover")]
pub fn comp_recover(env: Json, ctx: Json, stop_timeout: f64) -> AppTask {
    let (env, ctx) = (env_of(env), ctx_of(&ctx));
    AppTask::of(Task::new(move |io| async move { install::recover_service_update(&io, &env, &ctx, &install::Life { stop_timeout }).await }))
}

#[napi(js_name = "compInstallBundle")]
pub fn comp_install_bundle(bundle: String, env: Json, command: bool, ctx: Json, stop_timeout: f64) -> AppTask {
    let (env, ctx) = (env_of(env), ctx_of(&ctx));
    AppTask::of(Task::new(move |io| async move { install::install_bundle(&io, &bundle, &env, command, &ctx, &install::Life { stop_timeout }).await }))
}

#[napi(js_name = "compActivate")]
pub fn comp_activate(file: String, item: Json, env: Json, ctx: Json, stop_timeout: f64) -> AppTask {
    let (item, env, ctx) = (item.0, env_of(env), ctx_of(&ctx));
    AppTask::of(Task::new(move |io| async move { install::activate_package(&io, &file, &item, &env, &ctx, &install::Life { stop_timeout }).await }))
}

#[napi(js_name = "compUpdates")]
pub fn comp_updates(env: Json, machine: Json) -> AppTask {
    let (env, m) = (env_of(env), machine_of(&machine));
    AppTask::of(Task::new(move |io| async move { updates::component_updates(&io, &env, &m).await }))
}

#[napi(js_name = "compDownloadPlan")]
pub fn comp_download_plan(item: Json, env: Json, machine: Json) -> Jv {
    fs_answer(updates::download_plan(&item.0, &env_of(env), &machine_of(&machine)).map(|(file, have)| Jv::Obj(vec![("file", s(file)), ("have", Jv::Bool(have))])))
}

/// An update that isn't one: { component, status: "current", version }
/// when the item isn't newer, else null.
#[napi(js_name = "compUpdateSkip")]
pub fn comp_update_skip(item: Json) -> Json {
    let i = &item.0;
    if xenonet_app::jsv::truthy(i.get("available")) {
        return Json(Value::Null);
    }
    let mut o = Map::new();
    if let Some(c) = i.get("component") {
        o.insert("component".into(), c.clone());
    }
    o.insert("status".into(), json!("current"));
    if let Some(v) = i.get("current") {
        o.insert("version".into(), v.clone());
    }
    Json(Value::Object(o))
}

/// A component's download under way, for the twin's downloadComponent.
#[napi(js_name = "CompDownload")]
pub struct CompDownload {
    file: String,
    size: f64,
    sha256: String,
    inner: Option<updates::Download>,
}

#[napi]
impl CompDownload {
    #[napi(constructor)]
    pub fn new(file: String, size: f64, sha256: String) -> Self {
        CompDownload { file, size, sha256, inner: None }
    }

    /// Makes the temporary file; { error } if it can't.
    #[napi]
    pub fn open(&mut self) -> Jv {
        match updates::Download::new(&self.file, self.size, &self.sha256) {
            Ok(d) => {
                self.inner = Some(d);
                Jv::Obj(vec![("ok", Jv::Bool(true))])
            }
            Err(e) => Jv::Obj(vec![("error", fs_error(&e))]),
        }
    }

    /// Writes a chunk; the error when it can't, or when it's past the size.
    #[napi]
    pub fn write(&mut self, chunk: Buffer) -> Jv {
        match self.inner.as_mut().map(|d| d.write(&chunk)) {
            Some(Ok(())) | None => Jv::Undef,
            Some(Err(e)) => fs_error(&e),
        }
    }

    #[napi]
    pub fn finish(&mut self) -> Jv {
        match self.inner.as_mut() {
            None => Jv::Obj(vec![("error", Jv::Obj(vec![("message", s("the download wasn't opened"))]))]),
            Some(d) => fs_answer(d.finish().map(s)),
        }
    }

    #[napi]
    pub fn discard(&mut self) {
        if let Some(d) = self.inner.as_mut() {
            d.discard();
        }
    }
}

/// The installed XenoNet's console, for the twin's installedConsole.
#[napi(js_name = "CompConsole")]
pub struct CompConsole {
    env: Map<String, Value>,
    machine: runtime::Machine,
}

#[napi]
impl CompConsole {
    #[napi(constructor)]
    pub fn new(env: Json, machine: Json) -> Self {
        CompConsole { env: env_of(env), machine: machine_of(&machine) }
    }

    #[napi(getter)]
    pub fn port(&self) -> f64 {
        xenonet_base::text::js_number(Some(&Value::String(runtime::port_of(&self.env))))
    }

    #[napi]
    pub fn task(&self, kind: String, args: Json) -> AppTask {
        let c = console::Console { env: self.env.clone(), machine: self.machine.clone() };
        let a = args.0;
        AppTask::of(Task::new(move |io| async move {
            let opts = a.get("opts");
            // { op, ...opts }, as the JS spread them.
            let with = |op: &str| {
                let mut b = Map::new();
                b.insert("op".into(), json!(op));
                if let Some(Value::Object(o)) = opts {
                    for (k, v) in o {
                        b.insert(k.clone(), v.clone());
                    }
                }
                Value::Object(b)
            };
            match kind.as_str() {
                "call" => c.call(&io, a.get("body").unwrap_or(&Value::Null)).await,
                "ping" => c.ping(&io).await,
                "ready" => c.ready(&io, a.get("timeout").and_then(Value::as_f64).unwrap_or(60000.0)).await,
                "quit" => c.quit(&io).await,
                "launch" => {
                    let show = a.get("show").map_or_else(|| "inactive".to_string(), xenonet_base::text::js_string);
                    c.launch(&io, &show, xenonet_app::jsv::truthy(a.get("restart")), a.get("timeout").and_then(Value::as_f64).unwrap_or(60000.0)).await
                }
                "driver" => {
                    c.call(&io, &json!({ "op": "driver", "method": a.get("method").cloned().unwrap_or(Value::Null), "args": a.get("args").cloned().unwrap_or(json!([])) })).await
                }
                "screenshot" | "input" | "window" | "logs" | "record" => c.call(&io, &with(&kind)).await,
                "api" => c.api(&io, a.get("route").and_then(Value::as_str).unwrap_or("")).await,
                _ => Err(xenonet_app::task::Failure::msg(format!("no console task {kind}"))),
            }
        }))
    }
}

#[napi(js_name = "compOnlyDev")]
pub fn comp_only_dev() -> &'static str {
    "This action is available only in development instances."
}
