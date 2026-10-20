#![no_main]

// HTTP requests as the service gets them: its API's routes (the hooks
// Claude Code and Codex post to among them), with cores that refuse but a
// process check that lets every caller through, so a request gets as far
// as its route reads it; and the actor gateway, which a tunnel can put on
// the internet, with stand-in cores like its tests'. Then the pieces both
// parse requests with. The input is a request as written on the wire
// ("METHOD target", header lines, a blank line, the body); the first byte
// picks the router or the gateway.

use async_trait::async_trait;
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use xenonet_service::cores::{self, Cores, R};
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::actor_gateway::{self, ActorGateway, GatewayCores, GatewayHooks, GatewayRequest};
use xenonet_service::gateway::tunnel;
use xenonet_service::http::{self, Peer, Req};
use xenonet_service::routes::{self, api};
use xenonet_service::{borrowed, serve, upstream};

// What the router may be sent here: the routes that read requests from
// sessions, hooks, actors and pages. Routes that open apps, run
// terminals or change this machine's settings stay out, whatever the
// cores would say.
const ROUTES: [&str; 16] = [
    "/api/hook/",
    "/api/engine/",
    "/api/delivery",
    "/api/actors",
    "/api/assistant",
    "/api/hosts",
    "/api/refusals",
    "/api/drift",
    "/api/search",
    "/api/stats",
    "/api/graph",
    "/api/state",
    "/api/live",
    "/api/health",
    "/api/layout",
    "/api/integrations",
];

struct Permit;

#[async_trait]
impl cores::Verify for Permit {
    async fn check(&self, _: Peer, _: &str, _: bool) -> Value {
        json!({ "ok": true, "root": { "pid": 4242 } })
    }
    async fn person(&self, _: Peer) -> Value {
        json!({ "ok": true })
    }
    async fn window(&self, _: Peer) -> Value {
        json!({ "ok": true })
    }
}

struct Stand;

#[async_trait]
impl cores::Engine for Stand {
    async fn whoami(&self, _: &str) -> R<Value> {
        Ok(json!({ "you": "actor:muse" }))
    }
    async fn peers(&self, _: &str, scope: &str) -> R<Value> {
        Ok(json!([{ "id": "claude:x", "scope": scope }]))
    }
    async fn send(&self, _: &str, msg: Value, _: Option<Value>) -> R<Value> {
        Ok(json!({ "id": 7, "to": msg.get("to") }))
    }
    async fn inbox(&self, _: &str, wait: Duration) -> R<Value> {
        Ok(json!({ "messages": [], "waitMs": wait.as_millis() as u64 }))
    }
}

impl cores::Registration for Stand {
    fn authenticate_gateway(&self, token: &str) -> Value {
        if token == format!("xenonet_gateway_{}", "a".repeat(64)) {
            json!({ "ok": true, "id": "actor:muse", "slug": "muse", "state": "allowed" })
        } else {
            json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that token" })
        }
    }
    fn me(&self, _: &Value, _: Option<Value>) -> R<Value> {
        Ok(json!({ "schema": "actor/me/v1", "id": "actor:muse", "unread": 2 }))
    }
}

impl cores::Actors for Stand {
    fn info(&self, _: &str) -> Option<Value> {
        Some(json!({ "name": "Muse", "product": { "id": "muse" } }))
    }
    fn can(&self, _: &str, _: &str) -> bool {
        true
    }
}

impl cores::Reports for Stand {
    fn take_heartbeat(&self, _: &str, _: &Value) -> R<()> {
        Ok(())
    }
}

impl cores::Runs for Stand {
    fn report(&self, _: &str, run: Value) -> Value {
        json!({ "ok": true, "id": format!("run:{}", run["run"].as_str().unwrap_or("")) })
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime"))
}

fn service() -> &'static Arc<Ctx> {
    static CTX: OnceLock<Arc<Ctx>> = OnceLock::new();
    CTX.get_or_init(|| {
        runtime().block_on(async {
            let mut c = Cores::pending(true, true, true);
            c.verify = Arc::new(Permit);
            Ctx::for_tests(c)
        })
    })
}

// "METHOD target", "name: value" lines, a blank line, then the body.
fn request(data: &[u8]) -> (String, String, Vec<(String, String)>, Vec<u8>) {
    let (mut method, mut target, mut headers) = (String::new(), String::new(), Vec::new());
    let mut at = 0;
    for (n, line) in data.split(|&b| b == b'\n').enumerate() {
        at += line.len() + 1;
        let line = String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line)).into_owned();
        if n == 0 {
            let (m, t) = line.split_once(' ').unwrap_or(("GET", &line));
            (method, target) = (m.to_string(), t.to_string());
        } else if line.is_empty() {
            break;
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (method, target, headers, data.get(at..).unwrap_or_default().to_vec())
}

fn gateway() -> ActorGateway {
    let s = Arc::new(Stand);
    let cores = GatewayCores { engine: s.clone(), registration: s.clone(), actors: s.clone(), reports: s.clone(), runs: Some(s) };
    ActorGateway::with_clock(cores, GatewayHooks::default(), 47999, Some("https://mac.example.ts.net/"), "fuzz", Arc::new(|| 1_791_230_000_000.0))
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let (method, target, headers, body) = request(rest);
    // The pieces the router and the gateway read a request with.
    let (path, query) = http::parse_target(&target);
    let _ = http::decode_uri_component(&target);
    let _ = http::percent_decode(target.as_bytes());
    let _ = routes::integrations::params_object(&query);
    let _ = (routes::assistant::grant_id(&path), routes::themes::theme_id(&path), borrowed::parse_selector(&path));
    let _ = xenonet_actors::tokens::token_in_url(query.iter().map(|(k, v)| (k.as_str(), v.as_str())), &path);
    for (k, v) in &headers {
        let _ = xenonet_actors::tokens::bearer_of(v);
        let _ = actor_gateway::normal_host(Some(v));
        if k == "x-upstream" {
            let _ = upstream::upstream_of(Some(v));
        }
    }
    let text = String::from_utf8_lossy(&body);
    let _ = (tunnel::version_in(&text), tunnel::tailscale_self(&text), tunnel::read_line("tailscale", &text), tunnel::read_line("cloudflared", &text));
    if let Some(v) = xenonet_fuzz::json(&body) {
        let _ = tunnel::funnel_of(&v, "mac.example.ts.net", 47999);
        let _ = borrowed::from_opencode_id(&v);
        if let (Some(schema), Some(args)) = (v.get("schema"), v.get("args")) {
            let _ = actor_gateway::validate(schema, args);
        }
    }
    xenonet_fuzz::run(rest, || {
        if pick & 1 == 0 {
            if !ROUTES.iter().any(|r| path.starts_with(r)) {
                return;
            }
            let ctx = service();
            let hs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let mut req = Req::fake(&method, &target, &hs, None);
            req.body = http::Body::Bytes(body.clone());
            runtime().block_on(async {
                let answer = tokio::time::timeout(Duration::from_secs(2), api(ctx, req)).await;
                if let Ok(answer) = answer {
                    let _ = serve::answered(ctx, &method, &path, answer);
                }
            });
        } else {
            let req = GatewayRequest { method: method.clone(), target: target.clone(), headers: headers.clone(), body: body.clone() };
            runtime().block_on(async {
                let _ = tokio::time::timeout(Duration::from_secs(2), gateway().serve(req, pick & 2 != 0)).await;
            });
        }
    });
});
