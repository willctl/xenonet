//! xenonet intent: the person's intents, work asked for at one of their
//! devices and worked on the machine it names under one of their
//! delegations, through the local service's /api/intents routes. Listing
//! them is the person's from their terminal. Making one and cancelling one
//! sign as the person: XenoNet's window signs them with the device, and
//! from anywhere else they need the person's presence, which XenoNet asks
//! for only in its window, so the service refuses them from here and the
//! command says where they're done.

use crate::args::Flags;
use crate::js::{list, str, t};
use crate::out::{ago, c, json, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Map, Value};
use xenonet_api::client::Server;

pub const HELP: &str = include_str!("help/intent.txt");

fn call(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    s.request(method, route, body, &[]).map_err(|r| Fail::new(r.message, 1))
}

fn show(v: &Value) {
    let intents = list(v, "intents");
    if intents.is_empty() {
        print("No intents yet. Make one in XenoNet's window: Intents, in the command palette.");
        return;
    }
    let now = xenonet_text::time::now_ms();
    let wide = intents.iter().map(|i| t(i, "state").len()).max().unwrap_or(0);
    for i in intents {
        let state = t(i, "state");
        let shown = match state.as_str() {
            "done" => c::green(pad(&state, wide)),
            "failed" | "cancelled" | "expired" => c::dim(pad(&state, wide)),
            _ => c::yellow(pad(&state, wide)),
        };
        let at = i.get("madeAt").and_then(Value::as_f64).map_or_else(String::new, |m| ago(m, now));
        print(format!("  {}  {}  {} → {}  {}", shown, t(i, "id"), t(i, "maker"), t(i, "target"), c::dim(at)));
        print(format!("      {}", t(i, "goal").lines().next().unwrap_or("")));
        if let Some(o) = str(i, "outcome").or_else(|| str(i, "note")) {
            print(c::dim(format!("      {}", o.lines().next().unwrap_or(""))));
        }
    }
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let what = args.first().map_or("list", String::as_str);
    let s = need(flags)?;
    match what {
        "list" => {
            let v = call(&s, "GET", "/api/intents", None)?;
            if flags.on("json") {
                json(&v);
            } else {
                show(&v);
            }
            Ok(0)
        }
        "make" => {
            let goal = args[1..].join(" ");
            let mut body = Map::new();
            body.insert("goal".into(), json!(goal));
            for (flag, key) in [("on", "target"), ("project", "project"), ("app", "app"), ("delegation", "delegation"), ("arbiter", "arbiter")] {
                if let Some(v) = flags.text(flag) {
                    body.insert(key.into(), json!(v));
                }
            }
            if let Some(h) = flags.text("hours").and_then(|h| h.parse::<f64>().ok()) {
                body.insert("hours".into(), json!(h));
            }
            let v = call(&s, "POST", "/api/intents", Some(&Value::Object(body)))?;
            if flags.on("json") {
                json(&v);
            } else {
                print(format!("{} {} for {}: {}", c::green("✓"), t(&v, "id"), t(&v, "target"), t(&v, "state")));
            }
            Ok(0)
        }
        "cancel" => {
            let id = args.get(1).filter(|a| !a.is_empty()).ok_or_else(|| Fail::new("say which intent: xenonet intent cancel <int_…>", 2))?;
            let v = call(&s, "POST", &format!("/api/intents/{id}/cancel"), Some(&json!({})))?;
            if flags.on("json") {
                json(&v);
            } else {
                print(format!("{} {id} is cancelled", c::green("✓")));
            }
            Ok(0)
        }
        w => Err(Fail::new(format!("xenonet intent doesn't know {w}: list, make or cancel"), 2)),
    }
}
