//! xenonet script (cli/commands/script.js): what you did in XenoNet Dev,
//! saved as a script (the console logs it as steps; src/oplog.js), and
//! scripts run again, in any instance, shown or hidden (core/script.js).

use super::console::said;
use crate::args::Flags;
use crate::js::{list, t};
use crate::out::{c, json, print, Done, Fail};
use crate::pending;
use crate::sys;
use serde_json::{json, Map, Value};

pub const HELP: &str = include_str!("help/script.txt");

/// brief(state) (core/script.js): what a script leaves the page at, in
/// short.
pub fn brief(st: &Value) -> Value {
    let selection = st.get("selection").and_then(|s| crate::js::have(s, "id")).cloned().unwrap_or(Value::Null);
    let gathered = crate::js::some(st, "gathered").map_or(Value::Null, |g| json!(format!("{} around {}", list(g, "members").len(), t(g, "root"))));
    let overlays: Vec<Value> =
        st.get("overlays").and_then(Value::as_object).into_iter().flatten().filter(|(_, v)| xenonet_base::text::truthy(Some(v))).map(|(k, _)| json!(k)).collect();
    let mut m = Map::new();
    m.insert("selection".into(), selection);
    m.insert("gathered".into(), gathered);
    m.insert("overlays".into(), Value::Array(overlays));
    m.insert("alerts".into(), json!(list(st, "alerts").len()));
    m.insert("settled".into(), st.get("motion").and_then(|mo| mo.get("settled")).cloned().unwrap_or(Value::Null));
    Value::Object(m)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let instance = flags.text("instance").or_else(|| sys::var("XENONET_INSTANCE")).unwrap_or_else(|| "demo".into());
    let quiet = flags.on("json");
    let mut say = |v: &Value| {
        if !quiet {
            print(c::dim(said(v)));
        }
    };
    let verb = args.first().map(String::as_str);
    if verb == Some("record") {
        let ops = pending::control_driver(&instance, "ops", &Value::Null)?;
        let ops = ops.as_array().cloned().unwrap_or_default();
        let last = flags.number("last");
        let n = if last == 0.0 || last.is_nan() { ops.len() as f64 } else { last.trunc() };
        // ops.slice(-n)
        let from = if n > 0.0 { ops.len().saturating_sub(n as usize) } else { (-n as usize).min(ops.len()) };
        let steps: Vec<Value> =
            ops[from..].iter().map(|o| json!({ "op": o.get("op").cloned().unwrap_or(Value::Null), "args": o.get("args").cloned().unwrap_or(Value::Null) })).collect();
        if flags.on("clear") {
            pending::control_driver(&instance, "clearOps", &Value::Null)?;
        }
        json(&Value::Array(steps));
        return Ok(0);
    }
    let file = args.get(1).filter(|f| !f.is_empty());
    let (Some("run"), Some(file)) = (verb, file) else { return Err(Fail::new("xenonet script record, or xenonet script run <file>; xenonet script --help says more", 2)) };
    let full = sys::resolve(file);
    if !std::path::Path::new(&full).exists() {
        return Err(Fail::new(format!("there's no {file}"), 1));
    }
    pending::control_launch(&instance, &json!({ "show": if flags.on("hidden") { "hidden" } else { "inactive" } }))?;
    let module = [".js", ".mjs", ".cjs"].iter().any(|ext| full.ends_with(ext));
    if module {
        pending::run_module(&full, &instance)?;
    } else {
        let text = std::fs::read_to_string(&full).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &full).message))?;
        let steps = xenonet_base::json::parse(&text).map_err(|e| Fail::new(format!("{file} isn't JSON: {e}"), 1))?;
        let Some(steps) = steps.as_array() else { return Err(Fail::new(format!("{file} should be a list of steps"), 1)) };
        pending::run_script(&instance, steps, &mut say)?;
    }
    let st = brief(&pending::control_driver(&instance, "state", &Value::Null)?);
    if flags.on("json") {
        json(&st);
        return Ok(0);
    }
    print(format!("{} {} ran {}", c::green("✓"), sys::base_name(file), c::dim(xenonet_base::json::stringify(&st, 0))));
    Ok(0)
}
