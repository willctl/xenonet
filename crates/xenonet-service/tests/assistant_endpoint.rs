// The assistant's private endpoint with stand-ins for the connections and
// the reads (server/assistant-endpoint.js): a hello with the grant's
// credential and nothing else, then assistant.read and nothing else; its
// descriptor written while it listens, and gone when it stops.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use xenonet_service::assistant_endpoint::Endpoint;
use xenonet_service::cores::{self, R};

const GRANT: &str = "gr_0a1b2c3d4e5f60718293a4b5";

#[derive(Default)]
struct Grants {
    bridged: Mutex<Vec<i32>>,
    audit: Mutex<Vec<Value>>,
}

impl cores::Connections for Grants {
    fn authenticate(&self, grant: &Value, token: &Value) -> Value {
        if grant == GRANT && token == "secret" {
            json!({ "ok": true, "grant": { "grant_id": GRANT }, "actorId": "actor:dot", "connection": { "connection_id": "cn_1" }, "revision": 3 })
        } else {
            json!({ "ok": false, "code": "GRANT_REVOKED", "reason": "revoked" })
        }
    }
    fn bridge(&self, _c: &str, delta: i32) {
        self.bridged.lock().unwrap().push(delta);
    }
    fn policy(&self, _s: &Value) -> R<Value> {
        Ok(json!({ "deviceId": "dev_1", "permissions": ["read"] }))
    }
    fn any_active(&self) -> bool {
        true
    }
    fn recheck(&self, _g: &str) -> Value {
        json!({ "ok": true })
    }
    fn dir(&self) -> String {
        "/state".into()
    }
}

impl cores::Audit for Grants {
    fn access(&self, record: Value) {
        self.audit.lock().unwrap().push(record);
    }
}

struct Reads;

#[async_trait]
impl cores::AssistantService for Reads {
    async fn read(&self, session: &Value, params: &Value) -> R<Value> {
        Ok(json!({ "session": session, "op": params["operation"] }))
    }
}

async fn ask(lines: &mut tokio::io::Lines<BufReader<TcpStream>>, id: u64, method: &str, params: Value) -> Value {
    let line = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
    lines.get_mut().get_mut().write_all(line.as_bytes()).await.unwrap();
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn an_adapter_says_which_grant_and_then_may_only_read() {
    let dir = std::env::temp_dir().join(format!("xenonet-endpoint-{}", std::process::id()));
    let file = dir.join("assistant-47999.json");
    let grants = Arc::new(Grants::default());
    let e = Endpoint::new(grants.clone(), Arc::new(Reads), Some(grants.clone()), file.clone(), Arc::new(|_: &str| {}));
    e.sync().await;
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!((d["state"].clone(), d["api"].clone(), d["pid"].as_u64()), (json!("/state"), json!({ "min": 1, "max": 1 }), Some(u64::from(std::process::id()))));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let port = d["port"].as_u64().unwrap() as u16;

    let mut c = BufReader::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap()).lines();
    assert_eq!(ask(&mut c, 1, "assistant.read", json!({})).await["error"], json!("XenoNet refused this connection."), "nothing before hello");
    let hello = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": GRANT, "token": "wrong" });
    assert_eq!(ask(&mut c, 2, "hello", hello).await["error"], json!("GRANT_REVOKED"));
    let extra = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": GRANT, "token": "secret", "more": 1 });
    assert_eq!(ask(&mut c, 3, "hello", extra).await["error"], json!("NOT_AUTHORIZED"), "nothing else in a hello");
    let old = json!({ "role": "assistant", "api": { "min": 2, "max": 2 }, "grant_id": GRANT, "token": "secret" });
    assert_eq!(ask(&mut c, 4, "hello", old).await["error"], json!("INCOMPATIBLE_VERSION"));
    let good = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": GRANT, "token": "secret" });
    let answer = ask(&mut c, 5, "hello", good).await;
    assert_eq!(
        answer["result"],
        json!({ "assistant_api": 1, "device_id": "dev_1", "actor_id": "actor:dot", "connection_id": "cn_1", "grant_revision": 3, "permissions": ["read"] })
    );
    assert_eq!(e.info()["sessions"], json!(1));
    let read = ask(&mut c, 6, "assistant.read", json!({ "request_id": "r1", "operation": "projects", "arguments": {} })).await;
    assert_eq!(read["result"], json!({ "session": { "grantId": GRANT, "actorId": "actor:dot", "connectionId": "cn_1" }, "op": "projects" }));
    assert_eq!(ask(&mut c, 7, "status", json!({})).await["error"], json!("Unsupported request."));
    let audit = grants.audit.lock().unwrap().clone();
    let results: Vec<Value> = audit.iter().map(|a| a["result"].clone()).collect();
    assert_eq!(results, vec![json!("revoked"), json!("bad-hello"), json!("version"), json!("ok")]);
    drop(c);
    for _ in 0..50 {
        if e.info()["sessions"] == json!(0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*grants.bridged.lock().unwrap(), vec![1, -1], "the adapter's connection counted, then let go");
    e.stop();
    assert!(!file.exists(), "its descriptor goes with it");
    let _ = std::fs::remove_dir_all(&dir);
}

const OTHER: &str = "gr_ffeeddccbbaa998877665544";

/// Two grants whose events the test sends, and whose standing it sets:
/// the ones lapsed (a tombstone written without the service), and whether
/// any could be used at all.
struct Live {
    events: Mutex<Option<tokio::sync::mpsc::UnboundedSender<(String, Value)>>>,
    lapsed: Mutex<Vec<String>>,
    active: Mutex<bool>,
    forgotten: Mutex<Vec<String>>,
}

impl Live {
    fn new() -> Arc<Live> {
        Arc::new(Live { events: Mutex::new(None), lapsed: Mutex::new(vec![]), active: Mutex::new(true), forgotten: Mutex::new(vec![]) })
    }
    fn tell(&self, event: &str, grant: &str) {
        self.events.lock().unwrap().as_ref().unwrap().send((event.into(), json!({ "grant": grant }))).unwrap();
    }
}

impl cores::Connections for Live {
    fn authenticate(&self, grant: &Value, token: &Value) -> Value {
        match (grant.as_str(), token.as_str()) {
            (Some(g), Some("secret")) if g == GRANT || g == OTHER => {
                json!({ "ok": true, "grant": { "grant_id": g }, "actorId": "actor:dot", "connection": { "connection_id": format!("cn_{g}") }, "revision": 1 })
            }
            _ => json!({ "ok": false, "code": "NOT_AUTHORIZED", "reason": "bad-token" }),
        }
    }
    fn policy(&self, _s: &Value) -> R<Value> {
        Ok(json!({ "deviceId": "dev_1", "permissions": ["read"] }))
    }
    fn any_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
    fn recheck(&self, grant: &str) -> Value {
        json!({ "ok": !self.lapsed.lock().unwrap().iter().any(|g| g == grant) })
    }
    fn subscribe(&self) -> Option<cores::assistant::ConnectionEvents> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.events.lock().unwrap() = Some(tx);
        Some(rx)
    }
}

#[async_trait]
impl cores::AssistantService for Live {
    async fn read(&self, _session: &Value, params: &Value) -> R<Value> {
        Ok(json!({ "op": params["operation"] }))
    }
    fn forget(&self, grant: &str) {
        self.forgotten.lock().unwrap().push(grant.into());
    }
}

type Adapter = tokio::io::Lines<BufReader<TcpStream>>;

/// An adapter let in for `grant`.
async fn adapter(port: u16, grant: &str) -> Adapter {
    let mut c = BufReader::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap()).lines();
    let hello = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": grant, "token": "secret" });
    assert_eq!(ask(&mut c, 1, "hello", hello).await["result"]["actor_id"], json!("actor:dot"));
    c
}

/// Whether XenoNet let go of an adapter: its connection ends within a few
/// seconds (the sweep runs every two).
async fn let_go(c: &mut Adapter) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(5), c.next_line()).await, Ok(Ok(None) | Err(_)))
}

fn port_of(file: &std::path::Path) -> u16 {
    let d: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    d["port"].as_u64().unwrap() as u16
}

#[tokio::test]
async fn a_grant_revoked_rotated_lapsed_or_turned_off_lets_go_of_its_adapters_at_once() {
    let dir = std::env::temp_dir().join(format!("xenonet-endpoint-live-{}", std::process::id()));
    let file = dir.join("assistant-47998.json");
    let live = Live::new();
    let e = Endpoint::new(live.clone(), live.clone(), None, file.clone(), Arc::new(|_: &str| {}));
    e.sync().await;
    let port = port_of(&file);
    // Revoked: its adapter goes, the other grant's stays.
    let (mut a, mut b) = (adapter(port, GRANT).await, adapter(port, OTHER).await);
    live.tell("revoked", GRANT);
    assert!(let_go(&mut a).await, "revoked");
    assert_eq!(ask(&mut b, 2, "assistant.read", json!({ "operation": "projects" })).await["result"], json!({ "op": "projects" }));
    assert_eq!(*live.forgotten.lock().unwrap(), vec![GRANT.to_string()], "and what was read for it is forgotten");
    // Given a new credential: the adapter with the old one goes.
    live.tell("rotated", OTHER);
    assert!(let_go(&mut b).await, "rotated");
    // Lapsed while XenoNet's service wasn't the one that wrote it down.
    let mut c = adapter(port, OTHER).await;
    live.lapsed.lock().unwrap().push(OTHER.into());
    assert!(let_go(&mut c).await, "lapsed");
    // Turned off: nothing listens, until it's on again.
    let mut d = adapter(port, GRANT).await;
    live.tell("disabled", "");
    assert!(let_go(&mut d).await, "disabled");
    for _ in 0..50 {
        if !file.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!file.exists() && e.info()["listening"] == json!(false));
    *live.active.lock().unwrap() = false;
    live.tell("enabled", "");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(e.info()["listening"], json!(false), "no grant could be used");
    *live.active.lock().unwrap() = true;
    live.tell("changed", "");
    for _ in 0..50 {
        if file.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut again = adapter(port_of(&file), GRANT).await;
    assert_eq!(ask(&mut again, 2, "assistant.read", json!({ "operation": "projects" })).await["result"], json!({ "op": "projects" }));
    e.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn after_twenty_refused_hellos_none_is_let_in_for_a_minute() {
    let dir = std::env::temp_dir().join(format!("xenonet-endpoint-held-{}", std::process::id()));
    let file = dir.join("assistant-47997.json");
    let live = Live::new();
    let e = Endpoint::new(live.clone(), live.clone(), None, file.clone(), Arc::new(|_: &str| {}));
    e.sync().await;
    let mut c = BufReader::new(TcpStream::connect(("127.0.0.1", port_of(&file))).await.unwrap()).lines();
    let wrong = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": GRANT, "token": "wrong" });
    for i in 0..21 {
        assert_eq!(ask(&mut c, i, "hello", wrong.clone()).await["error"], json!("NOT_AUTHORIZED"), "{i}");
    }
    let good = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": GRANT, "token": "secret" });
    assert_eq!(ask(&mut c, 21, "hello", good).await["error"], json!("RATE_LIMITED"), "not even the right credential");
    e.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
