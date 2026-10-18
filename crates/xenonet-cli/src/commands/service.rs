//! xenonet service (cli/commands/service.js): the background service,
//! started, asked how it is, or stopped, through the component service's
//! socket (core/components/runtime.js, install.js).

use crate::args::Flags;
use crate::cores;
use crate::out::{print, Done, Fail};
use serde_json::{json, Map, Value};
use xenonet_base::json::stringify;

pub const HELP: &str = include_str!("help/service.txt");

pub fn run(args: &[String], flags: &Flags) -> Done {
    let action = args.first().map(String::as_str).filter(|a| !a.is_empty()).unwrap_or("status");
    // The environment's XENONET_PORT, or the one --port names.
    let mut env = Map::new();
    if flags.on("port") {
        env.insert("XENONET_PORT".into(), json!(xenonet_base::text::js_string(&flags.get("port").cloned().unwrap_or(Value::Null))));
    }
    let env = Value::Object(env);
    if action == "start" {
        cores::recover_service_update(&env)?;
        let status = cores::ensure_service(&cores::installed_runtime(&env)?)?;
        print(stringify(&json!({ "status": status }), 0));
        return Ok(0);
    }
    if action != "status" && action != "stop" {
        return Err(Fail::error(HELP));
    }
    let answer = cores::service_request(&env, action, &json!({ "force": flags.on("force") }))?;
    print(stringify(&answer, 0));
    Ok(0)
}
