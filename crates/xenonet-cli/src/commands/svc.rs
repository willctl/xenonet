//! xenonet svc (docs/guide/network.md, "Services through the network"): the
//! local HTTP services this machine publishes to its network, through the
//! local service's /api/net/svc routes. Publishing widens what the network
//! reaches, so it asks the person and is the window's (Network settings ›
//! Publish a service): refused here before anything is sent. Taking a
//! service off is the person's from their terminal too; the service
//! refuses it from under an agent. Other hosts' programs reach a published
//! service through their own XenoNet, at `/api/svc/<host>/<name>/<path>`.

use crate::args::Flags;
use crate::js::{list, str, t};
use crate::out::{c, json, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Value};
use xenonet_api::client::Server;

pub const HELP: &str = include_str!("help/svc.txt");

fn call(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    s.request(method, route, body, &[]).map_err(|r| Fail::new(r.message, 1))
}

fn arg(args: &[String], i: usize, what: &str) -> Result<String, Fail> {
    args.get(i).filter(|a| !a.is_empty()).cloned().ok_or_else(|| Fail::new(format!("say {what}: xenonet svc --help"), 2))
}

fn words(list_of: &[Value]) -> Vec<String> {
    list_of.iter().filter_map(Value::as_str).map(str::to_string).collect()
}

fn show(v: &Value) {
    let services = list(v, "services");
    if services.is_empty() {
        print("This machine publishes no services. Publish one in XenoNet's window: Network settings › Publish a service.");
        return;
    }
    let host = str(v, "host").unwrap_or("<this host>");
    let wide = services.iter().map(|s| t(s, "name").len()).max().unwrap_or(0);
    for s in services {
        let hosts = words(list(s, "hosts"));
        let who = if hosts.is_empty() { "every host".to_string() } else { hosts.join(", ") };
        print(format!("  {}  {}  {}  {}", pad(&t(s, "name"), wide), t(s, "target"), words(list(s, "paths")).join(" "), c::dim(format!("for {who}"))));
    }
    print(c::dim(format!("  Other hosts reach a service at /api/svc/{host}/<name>/<path> on their own XenoNet.")));
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let what = args.first().map_or("list", String::as_str);
    // Publishing asks the person, which only the window does.
    if what == "publish" {
        return Err(Fail::new("Publishing a service asks you, so it's done in XenoNet's window: Network settings › Publish a service", 1));
    }
    let s = need(flags)?;
    let out = |v: &Value| {
        if flags.on("json") {
            json(v);
        } else {
            show(v);
        }
        Ok(0)
    };
    match what {
        "list" => out(&call(&s, "GET", "/api/net/svc", None)?),
        "unpublish" => {
            let name = arg(args, 1, "which service: its name")?;
            let v = call(&s, "POST", "/api/net/svc/unpublish", Some(&json!({ "name": name })))?;
            if flags.on("json") {
                json(&v);
                return Ok(0);
            }
            print(format!("{} {name} is off the network", c::green("✓")));
            Ok(0)
        }
        w => Err(Fail::new(format!("xenonet svc doesn't know {w}: publish, unpublish or list"), 2)),
    }
}
