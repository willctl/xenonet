//! xenonet svc (docs/guide/network.md, "Services through the network"): a
//! local HTTP service this machine publishes to its network, through the
//! local service's /api/net/svc routes. Publishing and taking a service
//! off are the person's: the service refuses them from under an agent, as
//! it does creating or joining a network. Other hosts' programs reach a
//! published service through their own XenoNet, at
//! `/api/svc/<host>/<name>/<path>`.

use crate::args::Flags;
use crate::js::{list, str, t};
use crate::out::{c, json, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Value};
use xenonet_base::client::Server;

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
        print("This machine publishes no services. xenonet svc publish <name> http://127.0.0.1:<port> --paths /<path>/ publishes one.");
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
        "publish" => {
            let name = arg(args, 1, "the service's name and address: xenonet svc publish <name> http://127.0.0.1:<port> --paths /<path>/")?;
            let target = arg(args, 2, "the service's address, as http://127.0.0.1:<port>")?;
            let paths = flags.list("paths");
            if paths.is_empty() {
                return Err(Fail::new("say the paths other hosts may reach, as --paths /sync/v1/: nothing else of the service is reachable", 2));
            }
            let body = json!({ "name": name, "target": target, "paths": paths, "hosts": flags.list("hosts") });
            let v = call(&s, "POST", "/api/net/svc/publish", Some(&body))?;
            if flags.on("json") {
                json(&v);
                return Ok(0);
            }
            let host = str(&v, "host").unwrap_or("<this host>");
            print(format!("{} published {name}: other hosts reach it at /api/svc/{host}/{name}/<path> on their own XenoNet", c::green("✓")));
            print(c::dim("  A service with no sign-in of its own is open to every program on every host allowed to call it."));
            Ok(0)
        }
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
