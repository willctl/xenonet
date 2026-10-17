// The actor gateway's switch (server/gateway-control.js) with its settings
// in a made-up engine database: on with a tunnel of the owner's own, the
// address checked on this computer alone (as XENONET_GATEWAY_PROBE=local
// checks it), the settings kept, a tunnel it doesn't know refused, and off.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_service::cores::{self, R};
use xenonet_service::gateway::actor_gateway::GatewayCores;
use xenonet_service::gateway::control::{ControlParts, GatewayControl};
use xenonet_service::gateway::reach::SystemNetwork;
use xenonet_service::gateway::tunnel::{RunOut, Spawned, TunnelHost};

#[derive(Default)]
struct Store(Mutex<HashMap<String, String>>);

impl cores::EngineStore for Store {
    fn meta(&self, key: &str) -> R<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set_meta(&self, key: &str, value: &str) -> R<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn actor_token_times(&self) -> R<Vec<Value>> {
        Ok(vec![json!({ "slug": "muse", "hasToken": false, "at": null, "hasGateway": true, "gatewayAt": 1000 })])
    }
}

struct Stand;
impl cores::Engine for Stand {}
impl cores::Registration for Stand {}
impl cores::Reports for Stand {}
impl cores::Actors for Stand {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Muse", "product": { "id": "muse" } }))
    }
}

struct NoTools;
#[async_trait]
impl TunnelHost for NoTools {
    fn exists(&self, _f: &str) -> bool {
        false
    }
    async fn run(&self, _f: &str, _a: &[String], _t: Duration) -> RunOut {
        RunOut::default()
    }
    fn spawn(&self, _f: &str, _a: &[String]) -> Result<Spawned, String> {
        Err("ENOENT".into())
    }
    fn now(&self) -> f64 {
        5000.0
    }
    fn path_var(&self) -> String {
        String::new()
    }
    fn platform(&self) -> &'static str {
        "darwin"
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn on_with_a_tunnel_of_your_own_checked_here_kept_and_off() {
    let store = Arc::new(Store::default());
    let s = Arc::new(Stand);
    let port = free_port();
    let c = GatewayControl::new(ControlParts {
        cores: GatewayCores { engine: s.clone(), registration: s.clone(), actors: s.clone(), reports: s, runs: None },
        store: store.clone(),
        on_run: Arc::new(|_, _| {}),
        port,
        version: "test".into(),
        dir: None,
        home: "/home/nova".into(),
        probe_local: true,
        log: Arc::new(|_, _| {}),
        tunnel_host: Arc::new(NoTools),
        network: Arc::new(SystemNetwork),
        now: Arc::new(|| 5000.0),
    });
    c.init().await;
    let off = c.status().await.unwrap();
    assert_eq!((off["on"].clone(), off["listening"].clone(), off["tunnel"]["kind"].clone()), (json!(false), json!(false), Value::Null));

    let bad = c.set(None, Some(json!({ "kind": "carrier-pigeon" })), false, false).await.unwrap_err();
    assert_eq!((bad.status, bad.code.as_deref()), (Some(400), Some("bad-request")));
    assert!(bad.message.starts_with("a tunnel is one of tailscale, cloudflare, cloudflare-quick, ngrok, own"));
    let named = c.set(None, Some(json!({ "kind": "cloudflare", "host": "x.example.com" })), false, false).await.unwrap_err();
    assert_eq!(named.message, "say the tunnel's name, as cloudflared tunnel list shows it");
    let host = c.set(None, Some(json!({ "kind": "own", "host": "not a host" })), false, false).await.unwrap_err();
    assert_eq!(host.message, "that isn't a host name, such as xenonet.example.com");

    let on = c.set(Some(true), Some(json!({ "kind": "own", "host": "https://Mac.Example.ts.net/mcp" })), true, false).await.unwrap();
    assert_eq!(
        (on["on"].clone(), on["listening"].clone(), on["host"].clone(), on["url"].clone()),
        (json!(true), json!(true), json!("mac.example.ts.net"), json!("https://mac.example.ts.net/mcp"))
    );
    assert_eq!((on["tunnel"]["kind"].clone(), on["tunnel"]["state"].clone(), on["tunnel"]["label"].clone()), (json!("own"), json!("yours"), json!("Your own")));
    assert_eq!((on["reach"]["state"].clone(), on["reach"]["stage"].clone(), on["reach"]["local"].clone()), (json!("ok"), json!("ok"), json!(true)), "{}", on["reach"]);
    assert_eq!(on["problem"], Value::Null);
    assert_eq!(on["kinds"].as_array().unwrap().len(), 5);
    assert_eq!(on["actors"][0]["id"], json!("actor:muse"));
    assert_eq!(on["actors"][0]["expiresAt"], json!(1000u64 + 30 * 24 * 3600 * 1000), "its gateway token lapses unused 30 days on");
    let kept: Value = serde_json::from_str(&store.0.lock().unwrap()["gateway"]).unwrap();
    assert_eq!(kept, json!({ "on": true, "host": "mac.example.ts.net", "tunnel": { "kind": "own", "name": null, "host": "mac.example.ts.net" } }));
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(), "its port is open");

    let off = c.set(Some(false), None, false, false).await.unwrap();
    assert_eq!((off["on"].clone(), off["listening"].clone(), off["reach"]["state"].clone()), (json!(false), json!(false), json!("off")));
    c.stop().await;
}
