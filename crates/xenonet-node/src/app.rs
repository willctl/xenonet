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
