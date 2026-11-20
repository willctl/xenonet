// Sandboxed starts, end to end (docs/design/daily-use.md): a start from
// another machine that isn't the person's own runs in the app's own
// sandbox or doesn't run. Three services in this process, as in
// net_engine.rs, with the fake CLI keeping the settings file and the
// names of the environment each run was given.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]
#![expect(dead_code, reason = "the shared engine harness does more than this file uses")]

mod common;
#[path = "common/engine.rs"]
mod engine;
mod ports;

use engine::*;

use serde_json::{json, Value};
use std::time::Duration;
use xenonet_model::ops::{Kind, Outgoing, To};
use xenonet_net::node::now_ms;

fn root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("xenonet-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

/// `desk` takes starts from the network, and `contractor-vm` knows it.
async fn desk_takes_starts(t: &TwoNetworks) {
    let (s, v) = t.desk.window("PUT", "/api/net/settings", json!({ "network": "work", "host": "desk", "start": "network" })).await;
    assert_eq!(s, 200, "{v}");
    until("contractor-vm knows desk takes starts from the network", 20, || async {
        let r = t.contractor.state().await["hosts"].as_array().and_then(|h| h.iter().find(|x| x["id"] == "desk").cloned()).unwrap_or(Value::Null);
        r["reach"]["start"] == "network" && r["can"].as_array().is_some_and(|c| c.contains(&json!("start")))
    })
    .await;
}

/// What contractor-vm's session is told next that is a XenoNet note.
async fn told(t: &TwoNetworks, what: &str) -> String {
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let got = texts(&t.contractor.ctx.cores.engine.inbox(VM_ATLAS, Duration::ZERO).await.unwrap());
        if let Some(i) = got.into_iter().find(|m| m.contains("[XenoNet]")) {
            return i;
        }
        assert!(std::time::Instant::now() < end, "waited for the answer to {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The frame an older XenoNet on `contractor-vm` sends for a start in
/// `folder`: the request as it wrote it, which has no `sandbox` in it.
fn older_start(t: &TwoNetworks, n: u32, folder: &str) -> Value {
    let mid = format!("msg_00000000-0000-4000-8000-{n:012}");
    let request = json!({
        "folder": folder, "app": "claude", "mode": null, "prompt": "Tidy atlas", "request": mid,
        "by": { "kind": "agent", "id": VM_ATLAS, "title": "Fix the login form" },
    });
    json!({
        "schema": "network/send/v1", "mid": mid, "sender": VM_ATLAS, "senderKind": "agent", "agent": t.contractor.agt(VM_ATLAS),
        "target": "xenonet:start", "body": request.to_string(), "replyTo": null, "scope": "direct", "kind": "message", "hops": 0, "at": now_ms() as i64,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_start_from_another_machine_without_the_sandbox_is_refused() {
    let root = root("sandbox-missing");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: older_start(&t, 1, &atlas) }).unwrap();
    until("desk logs the refusal", 20, || async { t.desk.logged("start").iter().any(|l| l.contains("was refused")) }).await;
    let refused = t.desk.logged("start").join("\n");
    assert!(refused.contains("sandbox"), "the refusal says why: {refused}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sandboxed_start_from_another_machine_runs_in_the_folder_and_nowhere_else() {
    let root = root("sandbox-on");
    // A secret in the service's environment, which a sandboxed run never gets.
    std::env::set_var("SANDBOX_TEST_API_KEY", "not-for-the-run");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let desk_home = root.join("desk/home");
    let atlas = desk_home.join("atlas").to_string_lossy().into_owned();
    let (s, v) = as_agent(
        &t.contractor,
        VM_ATLAS,
        "POST",
        "/api/start",
        json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "hold-on: tidy atlas", "sandbox": true, "domains": ["github.com"] }),
    )
    .await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let first = told(&t, "atlas").await;
    assert!(first.contains("Started"), "{first}");
    assert!(first.to_lowercase().contains("sandbox") && first.contains("github.com"), "the answer says the run is sandboxed and what it reaches: {first}");
    let (args, prompt) = ran(&t.desk, 1).await;
    assert!(prompt.to_lowercase().contains("sandbox") && prompt.contains("github.com"), "the first prompt says so too: {prompt}");
    assert!(args.iter().any(|a| a == "--strict-mcp-config") && args.iter().any(|a| a == "--setting-sources="), "the repo's own settings and servers aren't loaded: {args:?}");
    let (settings, names) = fake_files(&t.desk.dir, 1).unwrap();
    let settings: Value = serde_json::from_str(&settings.expect("a settings file")).unwrap();
    let sandbox = &settings["sandbox"];
    assert_eq!((&sandbox["enabled"], &sandbox["failIfUnavailable"], &sandbox["allowUnsandboxedCommands"]), (&json!(true), &json!(true), &json!(false)));
    assert_eq!(sandbox["filesystem"]["allowWrite"], json!([atlas]), "it writes only in the folder");
    assert_eq!(sandbox["filesystem"]["denyRead"], json!([desk_home.to_string_lossy()]), "it reads none of the home folder");
    assert!(sandbox["filesystem"]["allowRead"].as_array().unwrap().contains(&json!(atlas)), "but the folder");
    assert_eq!(sandbox["network"]["allowedDomains"], json!(["github.com"]));
    assert_eq!(settings["permissions"]["blockReadsOutsideWorkingDirectories"], true);
    assert_eq!(settings["permissions"]["blockWritesOutsideWorkingDirectories"], true);
    assert!(settings["permissions"]["allow"].as_array().unwrap().contains(&json!("WebFetch(domain:github.com)")), "{settings}");
    assert!(settings["hooks"]["Stop"].is_array(), "XenoNet's own hooks stay: {settings}");
    // Its environment: the scrubbed one, and nothing of the service's secrets.
    assert!(!names.iter().any(|n| n == "SANDBOX_TEST_API_KEY" || n.starts_with("XENONET_")), "{names:?}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_start_with_no_domains_allows_the_network_none_and_a_persons_own_run_stays_unsandboxed() {
    let root = root("sandbox-local");
    std::env::set_var("SANDBOX_TEST_API_KEY", "not-for-the-run");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // The person's own console start on this machine, which has the switch off.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas" })).await;
    assert_eq!(s, 200, "{v}");
    let (_, prompt) = ran(&t.desk, 1).await;
    assert!(!prompt.to_lowercase().contains("sandbox"), "{prompt}");
    let (settings, names) = fake_files(&t.desk.dir, 1).unwrap();
    assert!(!settings.unwrap_or_default().contains("\"sandbox\""));
    assert!(names.iter().any(|n| n == "SANDBOX_TEST_API_KEY"), "an unsandboxed run keeps the service's environment, as before");
    // The same, sandboxed, with the switch on and no domains.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas again", "sandbox": true })).await;
    assert_eq!(s, 200, "{v}");
    let _ = ran(&t.desk, 2).await;
    let (settings, names) = fake_files(&t.desk.dir, 2).unwrap();
    let settings: Value = serde_json::from_str(&settings.unwrap()).unwrap();
    assert_eq!(settings["sandbox"]["network"]["allowedDomains"], json!([]));
    assert!(settings["permissions"]["deny"].as_array().unwrap().contains(&json!("WebFetch")), "{settings}");
    assert!(!names.iter().any(|n| n == "SANDBOX_TEST_API_KEY"));
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn an_app_with_no_sandbox_refuses_a_sandboxed_start_in_words() {
    let root = root("sandbox-grok");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "app": "grok", "prompt": "tidy atlas", "sandbox": true })).await;
    assert_eq!(s, 409, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("Grok Build") && e.contains("sandbox")), "{v}");
    assert_eq!(fake_runs(&t.desk).len(), 0);
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// A session that was started sandboxed is resumed sandboxed: the turn a
/// message wakes it for loads the run's settings and none of the repo's, and
/// has the scrubbed environment, never the bare turn a session without a
/// sandbox gets.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_turn_resumed_for_a_sandboxed_session_is_sandboxed_too() {
    let root = root("sandbox-resume");
    std::env::set_var("SANDBOX_TEST_API_KEY", "not-for-the-run");
    let t = two_networks_ended(&root).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new() };
    xenonet_service::compose::runtime::sandbox::keep(&t.desk.world, DESK_ATLAS_ENDED, &record);
    until("desk knows contractor-vm's session", 20, || async { t.desk.peers(DESK_ATLAS_ENDED).await.contains(&t.contractor.agt(VM_ATLAS)) }).await;
    let (s, v) = t.desk.window("PUT", "/api/net/settings", json!({ "network": "work", "host": "desk", "wake": "network" })).await;
    assert_eq!(s, 200, "{v}");
    let frame = json!({
        "schema": "network/send/v1", "mid": "msg_00000000-0000-4000-8000-000000000071", "sender": VM_ATLAS, "senderKind": "agent", "agent": t.contractor.agt(VM_ATLAS),
        "target": t.desk.agt(DESK_ATLAS_ENDED), "body": "The fixtures moved.", "replyTo": null, "scope": null, "kind": "message", "hops": 0, "at": now_ms() as i64,
    });
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: frame }).unwrap();
    let (args, _) = ran(&t.desk, 1).await;
    assert_eq!(&args[..2], ["-p", "--resume"], "{args:?}");
    assert!(args.iter().any(|a| a == "--strict-mcp-config") && args.iter().any(|a| a == "--setting-sources="), "{args:?}");
    let (settings, names) = fake_files(&t.desk.dir, 1).unwrap();
    let settings: Value = serde_json::from_str(&settings.expect("a settings file")).unwrap();
    assert_eq!(settings["sandbox"]["filesystem"]["allowWrite"], json!([atlas]));
    assert!(!names.iter().any(|n| n == "SANDBOX_TEST_API_KEY"), "{names:?}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}
