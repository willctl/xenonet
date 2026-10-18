//! Actors as the binary drives them (test/actors.test.js,
//! actor-registration.test.js and observers.test.js, where they check what
//! the driver adds): the demo's made-up actors and its simulations,
//! registering an actor and allowing it, a report taken, the events the
//! page hears, and the observers outside the demo.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_service::config::Config;
use xenonet_service::cores::{Actors, Cores, Engine, Registration, Reports, State, Wakes, R};
use xenonet_service::wire::{Emitter, WireEnv};
use xenonet_wire_engine::world::World;

/// A map with nothing on it yet, and the blocks the drivers list in it.
#[derive(Default)]
struct Empty {
    blocks: Mutex<Vec<Arc<dyn Fn() -> Value + Send + Sync>>>,
}

#[async_trait]
impl State for Empty {
    fn graph(&self) -> R<Option<Value>> {
        Ok(Some(json!({})))
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
    fn node(&self, _id: &str) -> R<Option<Value>> {
        Ok(None)
    }
    fn add_block(&self, health: Arc<dyn Fn() -> Value + Send + Sync>, _source: Option<String>) -> R<()> {
        self.blocks.lock().expect("the blocks the test keeps").push(health);
        Ok(())
    }
}

fn world(name: &str, demo: bool) -> (Arc<World>, Arc<Empty>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xenonet-wire-actors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp folder");
    let home = dir.to_string_lossy().into_owned();
    let state = dir.join("state").to_string_lossy().into_owned();
    let projects = dir.join("Projects").to_string_lossy().into_owned();
    let vars = [
        ("HOME", home.as_str()),
        ("XENONET_STATE_DIR", state.as_str()),
        ("XENONET_CACHE_DIR", state.as_str()),
        ("XENONET_PROJECTS_DIR", projects.as_str()),
        ("XENONET_PORT", "47997"),
        ("HERMES_HOME", home.as_str()),
    ];
    let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    let args: Vec<String> = if demo { vec!["--demo".into()] } else { Vec::new() };
    let mut config = Config::from(&args, &get);
    // The demo's world is the map's to make: here, its folders.
    if demo {
        config.use_world(&json!({ "home": home, "projectsDir": projects, "stateDir": state, "cacheDir": state }));
    }
    let empty = Arc::new(Empty::default());
    let mut cores = Cores::pending(true, false, false);
    cores.state = empty.clone();
    let env = WireEnv { version: "0.0.0".into(), emitter: Emitter::new(), drift: Arc::default() };
    let w = xenonet_wire_engine::world(&cores, &mut config, &env).expect("the area's parts");
    (w, empty, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demos_actors_their_simulations_and_its_made_up_request() {
    let (w, empty, dir) = world("demo", true);
    let actors = xenonet_wire_engine::actors::ActorsDriver(w.clone());
    let list = actors.list().unwrap();
    assert_eq!(list["schema"], "actor/list/v1");
    let ids: Vec<&str> = list["actors"].as_array().unwrap().iter().filter_map(|a| a["id"].as_str()).collect();
    assert_eq!(ids, vec!["actor:you", "actor:wren", "actor:kestrel", "actor:moth"]);
    assert_eq!(actors.snapshot("actor:kestrel", None, true).unwrap().unwrap()["name"], "Kestrel");
    assert!(actors.snapshot("actor:nobody", None, true).unwrap().is_none());
    assert_ne!(actors.events(Some(0.0), None, None, 1000.0).unwrap()["events"].as_array().unwrap(), &[] as &[Value]);
    assert!(actors.is_demo());
    for kind in ["actor-spawn", "actor-edit", "actor-message", "actor-offline", "actor-online"] {
        assert_eq!(actors.simulate(kind, Some("actor:moth".into()), None).unwrap().unwrap()["kind"], kind);
    }
    assert!(actors.simulate("actor-spawn", Some("actor:nobody".into()), None).unwrap().is_none());
    assert_eq!(actors.simulate("actor-needs-you", Some("actor:wren".into()), None).unwrap().unwrap()["kind"], "actor-needs-you");
    actors.simulate("clear", None, None).unwrap();
    // The demo's made-up request, and its token, which waits for you.
    let reg = xenonet_wire_engine::registration::RegistrationDriver(w.clone());
    let sim = reg.simulate("actor-request", None).unwrap().unwrap();
    assert_eq!((sim["ok"].clone(), sim["a"].clone()), (json!(true), json!("actor:heron")));
    let token = sim["token"].as_str().unwrap();
    let auth = reg.authenticate(token);
    assert_eq!((auth["ok"].clone(), auth["state"].clone()), (json!(true), json!("waiting")), "{auth}");
    assert!(reg.waiting() >= 1);
    assert_eq!(reg.authenticate(&format!("xenonet_actor_{}", "0".repeat(64)))["code"], "bad-token");
    // No observers in the demo; the engine's store is listed with the map's.
    assert_eq!(actors.observers_status(), json!([]));
    let names: Vec<Value> = empty.blocks.lock().unwrap().iter().map(|h| h()["name"].clone()).collect();
    assert_eq!(names, vec![json!("engine")]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_an_actor_allowing_it_and_taking_what_it_reports() {
    let (w, empty, dir) = world("register", false);
    let reg = xenonet_wire_engine::registration::RegistrationDriver(w.clone());
    let actors = xenonet_wire_engine::actors::ActorsDriver(w.clone());
    let asked = reg.register(&json!({ "name": "Kite", "capabilities": ["run-commands"] }), &json!({ "ok": true, "checked": false })).unwrap();
    assert_eq!((asked["state"].clone(), asked["id"].clone()), (json!("waiting"), json!("actor:kite")), "{asked}");
    let token = asked["token"].as_str().unwrap().to_string();
    let requests = reg.requests().unwrap();
    let request = requests["requests"][0]["id"].as_str().unwrap().to_string();
    let allowed = reg.decide(&request, &json!("allow")).unwrap();
    assert_eq!(allowed["decision"], "allow", "{allowed}");
    let auth = reg.authenticate(&token);
    assert_eq!((auth["ok"].clone(), auth["id"].clone()), (json!(true), json!("actor:kite")), "{auth}");
    assert!(actors.ids().contains(&"actor:kite".to_string()));
    // A heartbeat and events, through the reports' driver.
    let reports = xenonet_wire_engine::reports::ReportsDriver(w.clone());
    reports.take_heartbeat("actor:kite", &json!({ "presence": "working", "status": "Watching the build" })).unwrap();
    assert_eq!(actors.snapshot("actor:kite", None, true).unwrap().unwrap()["status"], "Watching the build");
    let event = reports.parse_event(&json!({ "kind": "ran", "count": 2 })).unwrap();
    let out = reports.take_events("actor:kite", vec![event, Value::Null], None, vec![1]).await.unwrap();
    assert_eq!(out["accepted"], 1, "{out}");
    assert_eq!(reports.status_line(&json!("a status"))["ok"], true);
    let me = reg.me(&auth, None).unwrap();
    assert_eq!(me["schema"], "actor/me/v1");
    // A program of its own holds a stream open, and hears at once that a
    // message waits: how many, never what it says.
    let wakes = xenonet_wire_engine::wakes::WakesDriver(w.clone());
    let mut stream = wakes.subscribe("actor:kite").unwrap();
    let engine = xenonet_wire_engine::engine::EngineDriver(w.clone());
    let sent = engine.send("claude:0190d3e0-0000-7000-8000-00000000000b", json!({ "to": "actor:kite", "text": "the build is red" }), None).await.unwrap();
    assert_eq!(sent["to"][0]["id"], "actor:kite", "{sent}");
    let (event, data) = tokio::time::timeout(std::time::Duration::from_secs(5), stream.recv()).await.unwrap().unwrap();
    assert_eq!((event.as_str(), data), ("inbox", json!({ "unread": 1 })));
    let inbox = engine.inbox("actor:kite", std::time::Duration::ZERO).await.unwrap();
    assert_eq!(inbox[0]["text"], "the build is red");
    // Taken out again: its token stops working.
    reg.remove("kite", None).unwrap();
    assert!(!actors.ids().contains(&"actor:kite".to_string()));
    assert_eq!(reg.authenticate(&token)["ok"], false);
    // Outside the demo, the observers of the three products, each on, and
    // their blocks listed after the engine's store.
    let status = actors.observers_status();
    let products: Vec<(Value, Value)> = status.as_array().unwrap().iter().map(|o| (o["product"].clone(), o["on"].clone())).collect();
    assert_eq!(products, vec![(json!("hermes-agent"), json!(true)), (json!("grok-bot"), json!(true)), (json!("muse"), json!(true))]);
    let names: Vec<Value> = empty.blocks.lock().unwrap().iter().map(|h| h()["name"].clone()).collect();
    assert_eq!(names, vec![json!("engine"), json!("hermes-observer"), json!("grok-bot-observer"), json!("muse-observer")]);
    std::fs::remove_dir_all(dir).unwrap();
}
