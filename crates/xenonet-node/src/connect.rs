//! xenonet-engine's connect for the twin of core/engine/connect.js: what
//! XenoNet adds to Claude Code's and Codex's settings, and takes out. The
//! files and the CLIs are called through the JS objects the twin was given.

use crate::args::JsArg;
use crate::js::Json;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::json;
use std::sync::Arc;
use xenonet_engine::connect::{self, Connect, Instance, Options};
use xenonet_engine::peer::list;
use xenonet_engine::sql::Arg;
use xenonet_engine::store::js_text;

fn instances(a: Arg) -> Vec<Instance> {
    list(a).iter().map(|i| Instance { id: js_text(&i.field("id")), name: js_text(&i.field("name")), port: i.field("port") }).collect()
}

fn text(a: Arg) -> Option<String> {
    (!a.is_nullish()).then(|| js_text(&a))
}

#[napi(js_name = "connectServerNames")]
pub fn server_names(port: JsArg, all: JsArg) -> Json {
    let (xenonet, console) = connect::server_names(&port.0, &instances(all.0));
    Json(json!({ "xenonet": xenonet, "console": console }))
}

/// mergeHooks with the caller's own `ours(hook)`.
#[napi(js_name = "connectMerge")]
pub fn merge(env: Env, deps: Object<'_>, settings: JsArg, add: JsArg) -> Result<Json> {
    let mut c = SyncCalls::new(&env, &deps);
    let add = (!add.0.is_nullish()).then(|| add.0.to_json());
    let out = connect::merge_hooks(&mut c, &settings.0.to_json(), &mut |c, h| Ok(c.call("", "ours", vec![crate::args::value_to_arg(h)])?.truthy()), add.as_ref());
    let out = out.map_err(|t| rethrow(&env, t));
    settle(&env);
    out.map(Json)
}

#[napi]
pub struct ConnectCore {
    inner: Arc<Connect>,
}

#[napi]
impl ConnectCore {
    #[napi(constructor)]
    pub fn new(o: JsArg) -> Self {
        let o = o.0;
        let options = Options {
            port: o.field("port"),
            demo: o.field("demo"),
            bin: js_text(&o.field("bin")),
            home: js_text(&o.field("home")),
            claude_config_dir: text(o.field("claudeConfigDir")),
            codex_home: text(o.field("codexHome")),
            grok_home: text(o.field("grokHome")),
            console_mcp: o.field("consoleMcp").truthy(),
            platform: js_text(&o.field("platform")),
            instances: instances(o.field("instances")),
        };
        ConnectCore { inner: Arc::new(Connect::new(options)) }
    }

    #[napi]
    pub fn preview(&self) -> Machine {
        let c = self.inner.clone();
        start(move |asker| c.preview(&mut AskCalls(asker)).map(|v| crate::args::value_to_arg(&v)))
    }
    /// `apps`: the ids to connect; null for every app that's here.
    #[napi]
    pub fn apply(&self, apps: JsArg) -> Machine {
        let c = self.inner.clone();
        let apps = (!apps.0.is_nullish()).then(|| list(apps.0));
        start(move |asker| {
            let mut calls = AskCalls(asker);
            let apps = match apps {
                Some(a) => a,
                None => c.installed_ids(&mut calls)?,
            };
            c.apply(&mut calls, &apps).map(|v| crate::args::value_to_arg(&v))
        })
    }
    #[napi(js_name = "hookConnected")]
    pub fn hook_connected(&self, event: JsArg) -> Machine {
        let c = self.inner.clone();
        let event = js_text(&event.0);
        start(move |asker| c.hook_connected(&mut AskCalls(asker), &event).map(Arg::Bool))
    }
    #[napi]
    pub fn remove(&self) -> Machine {
        let c = self.inner.clone();
        start(move |asker| c.remove(&mut AskCalls(asker)).map(|v| crate::args::value_to_arg(&v)))
    }
}
