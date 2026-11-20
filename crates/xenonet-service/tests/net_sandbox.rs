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

/// What the `n`th run of the fake on `h` was given, once it has written it.
async fn files_ready(h: &Host, n: usize) -> (Option<String>, Vec<String>) {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        // The fake writes its environment last of the two, and a read of a
        // file it is still writing is empty.
        if let Some(f) = fake_files(&h.dir, n).filter(|f| !f.1.is_empty()) {
            return f;
        }
        assert!(std::time::Instant::now() < end, "waited for run {n}'s files on {}", h.name);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the service's own environment holds on desk in these tests: the
/// sign-in of the person's own Claude Code, a key, and the sockets of the
/// SSH agent and of Docker. None of it is for a sandboxed run.
const SERVICE_ENV: [&str; 5] = ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY", "SSH_AUTH_SOCK", "DOCKER_HOST", "XENONET_TOKEN"];

fn service_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = SERVICE_ENV.iter().map(|k| ((*k).to_string(), "not-for-the-run".to_string())).collect();
    env.push(("PATH".into(), std::env::var("PATH").unwrap_or_default()));
    env
}

fn assert_unreachable_from_the_run(names: &[String]) {
    let leaked: Vec<&String> = names.iter().filter(|n| SERVICE_ENV.contains(&n.as_str())).collect();
    assert!(leaked.is_empty(), "a sandboxed run's environment has {leaked:?}: {names:?}");
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
    let t = two_networks(&root).await;
    t.desk.world.starts.set_environment(service_env());
    desk_takes_starts(&t).await;
    let desk_home = root.join("desk/home");
    let atlas = desk_home.join("atlas").to_string_lossy().into_owned();
    // Desk's person lets runs they didn't start reach github.com.
    let (s, v) = t.desk.window("PUT", "/api/runs/sandbox-domains", json!({ "domains": ["github.com"] })).await;
    assert_eq!(s, 200, "{v}");
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
    let (settings, names) = files_ready(&t.desk, 1).await;
    let settings: Value = serde_json::from_str(&settings.expect("a settings file")).unwrap();
    let sandbox = &settings["sandbox"];
    assert_eq!((&sandbox["enabled"], &sandbox["failIfUnavailable"], &sandbox["allowUnsandboxedCommands"]), (&json!(true), &json!(true), &json!(false)));
    assert_eq!(sandbox["credentials"]["envVars"], json!([{ "name": "CLAUDE_CODE_OAUTH_TOKEN", "mode": "deny" }]), "its commands don't get the run's sign-in");
    assert_eq!(sandbox["autoAllowBashIfSandboxed"], json!(false), "and a command still asks");
    assert_eq!((&sandbox["network"]["allowUnixSockets"], &sandbox["network"]["allowAllUnixSockets"]), (&json!([]), &json!(false)));
    assert_eq!(sandbox["filesystem"]["allowWrite"], json!([atlas]), "it writes only in the folder");
    assert_eq!(sandbox["filesystem"]["denyRead"], json!([desk_home.to_string_lossy()]), "it reads none of the home folder");
    assert!(sandbox["filesystem"]["allowRead"].as_array().unwrap().contains(&json!(atlas)), "but the folder");
    assert_eq!(sandbox["network"]["allowedDomains"], json!(["github.com"]));
    assert_eq!(settings["permissions"]["blockReadsOutsideWorkingDirectories"], true);
    assert_eq!(settings["permissions"]["blockWritesOutsideWorkingDirectories"], true);
    assert!(settings["permissions"]["allow"].as_array().unwrap().contains(&json!("WebFetch(domain:github.com)")), "{settings}");
    assert!(settings["hooks"]["Stop"].is_array(), "XenoNet's own hooks stay: {settings}");
    // Its environment: the scrubbed one, and nothing of the service's secrets.
    assert_unreachable_from_the_run(&names);
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_start_with_no_domains_allows_the_network_none_and_a_persons_own_run_stays_unsandboxed() {
    let root = root("sandbox-local");
    let t = two_networks(&root).await;
    t.desk.world.starts.set_environment(service_env());
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // The person's own console start on this machine, which has the switch off.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas" })).await;
    assert_eq!(s, 200, "{v}");
    let (_, prompt) = ran(&t.desk, 1).await;
    assert!(!prompt.to_lowercase().contains("sandbox"), "{prompt}");
    let (settings, names) = files_ready(&t.desk, 1).await;
    assert!(!settings.unwrap_or_default().contains("\"sandbox\""));
    assert!(
        ["ANTHROPIC_API_KEY", "SSH_AUTH_SOCK", "DOCKER_HOST"].iter().all(|k| names.iter().any(|n| n == k)),
        "an unsandboxed run keeps the service's environment, as before: {names:?}"
    );
    // The same, sandboxed, with the switch on and no domains.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas again", "sandbox": true })).await;
    assert_eq!(s, 200, "{v}");
    let _ = ran(&t.desk, 2).await;
    let (settings, names) = files_ready(&t.desk, 2).await;
    let settings: Value = serde_json::from_str(&settings.unwrap()).unwrap();
    assert_eq!(settings["sandbox"]["network"]["allowedDomains"], json!([]));
    assert!(settings["permissions"]["deny"].as_array().unwrap().contains(&json!("WebFetch")), "{settings}");
    assert_unreachable_from_the_run(&names);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sandbox_is_never_the_home_folder_whoever_asks_and_wherever_it_is_checked() {
    let root = root("sandbox-home");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let refused = |v: &Value| v["error"].as_str().is_some_and(|e| e.contains("project folder") && e.contains("home folder"));
    // The person's own sandboxed start at desk, in the default folder and in `~`.
    for (n, folder) in ["~", ""].into_iter().enumerate() {
        let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": folder, "prompt": format!("tidy {n}"), "sandbox": true })).await;
        assert!(s == 409 && refused(&v), "{folder:?}: {s} {v}");
    }
    // A session on contractor-vm asking for `~`: desk's home is not a project
    // it shows, so that is refused at the door before the sandbox's check, and
    // nothing runs.
    let (s, v) = as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": "~", "prompt": "tidy", "domains": [] })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let told = told(&t, "home").await;
    assert!(told.contains("No session was started"), "{told}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The person's unsandboxed start in home is theirs to make, as before.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": "~", "prompt": "tidy up" })).await;
    assert_eq!(s, 200, "{v}");
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
    let t = two_networks_ended(&root).await;
    t.desk.world.starts.set_environment(service_env());
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new() };
    xenonet_service::compose::runtime::sandbox::keep(&t.desk.world, DESK_ATLAS_ENDED, &record).unwrap();
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
    let (settings, names) = files_ready(&t.desk, 1).await;
    let settings: Value = serde_json::from_str(&settings.expect("a settings file")).unwrap();
    assert_eq!(settings["sandbox"]["filesystem"]["allowWrite"], json!([atlas]));
    assert_unreachable_from_the_run(&names);
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// The file XenoNet keeps a sandboxed session's record in.
fn record_file(t: &TwoNetworks, session: &str) -> std::path::PathBuf {
    std::path::Path::new(&t.desk.world.setup.state_dir).join("sandboxed").join(format!("{}.json", session.replace(':', "_")))
}

fn long_ago(file: &std::path::Path) {
    let old = std::time::SystemTime::now() - Duration::from_hours(720);
    std::fs::File::options().write(true).open(file).unwrap().set_modified(old).unwrap();
}

/// The message that wakes `DESK_ATLAS_ENDED` at desk, from contractor-vm's session.
async fn wake_atlas(t: &TwoNetworks, n: u32) {
    let (s, v) = t.desk.window("PUT", "/api/net/settings", json!({ "network": "work", "host": "desk", "wake": "network" })).await;
    assert_eq!(s, 200, "{v}");
    until("desk knows contractor-vm's session", 20, || async { t.desk.peers(DESK_ATLAS_ENDED).await.contains(&t.contractor.agt(VM_ATLAS)) }).await;
    let frame = json!({
        "schema": "network/send/v1", "mid": format!("msg_00000000-0000-4000-8000-{:012}", 80 + n), "sender": VM_ATLAS, "senderKind": "agent", "agent": t.contractor.agt(VM_ATLAS),
        "target": t.desk.agt(DESK_ATLAS_ENDED), "body": "The fixtures moved.", "replyTo": null, "scope": null, "kind": "message", "hops": 0, "at": now_ms() as i64,
    });
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: frame }).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_record_that_is_old_still_holds_its_session_sandboxed_and_a_turn_renews_it() {
    let root = root("sandbox-old");
    let t = two_networks_ended(&root).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new() };
    let w = &t.desk.world;
    xenonet_service::compose::runtime::sandbox::keep(w, DESK_ATLAS_ENDED, &record).unwrap();
    long_ago(&record_file(&t, DESK_ATLAS_ENDED));
    // Another session being kept sweeps the folder, as it did by age.
    xenonet_service::compose::runtime::sandbox::keep(w, "claude:0190d3e0-0000-7000-8000-0000000d0002", &record).unwrap();
    assert_eq!(xenonet_service::compose::runtime::sandbox::of(w, DESK_ATLAS_ENDED), Some(record.clone()), "a month old, and still sandboxed");
    // A turn for it runs sandboxed, and the record is as new as the turn.
    wake_atlas(&t, 1).await;
    let (args, _) = ran(&t.desk, 1).await;
    assert!(args.iter().any(|a| a == "--strict-mcp-config"), "{args:?}");
    until("the record is renewed", 10, || async {
        std::fs::metadata(record_file(&t, DESK_ATLAS_ENDED)).and_then(|m| m.modified()).unwrap().elapsed().map_or(true, |e| e < Duration::from_secs(3600))
    })
    .await;
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_session_started_for_a_delegation_with_no_record_is_not_resumed_bare() {
    let root = root("sandbox-lost");
    let t = two_networks_ended(&root).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // The store says desk started it for contractor-vm under a delegation;
    // its sandbox record is gone.
    let d = xenonet_store::delivery::Delegated { cwd: atlas, from: Some("contractor-vm".into()), at: now_ms(), intent: None };
    t.desk.world.store.0.db().put_delegated(DESK_ATLAS_ENDED, &d, 7.0 * 24.0 * 3600e3).unwrap();
    assert!(!record_file(&t, DESK_ATLAS_ENDED).exists());
    wake_atlas(&t, 2).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "no turn ran, bare or otherwise");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sandboxed_start_whose_record_cannot_be_written_does_not_run() {
    let root = root("sandbox-unwritable");
    let t = two_networks(&root).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // The folder the records go in is a file.
    std::fs::create_dir_all(&t.desk.world.setup.state_dir).unwrap();
    std::fs::write(std::path::Path::new(&t.desk.world.setup.state_dir).join("sandboxed"), "in the way").unwrap();
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas", "sandbox": true })).await;
    assert_eq!(s, 409, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("sandbox")), "{v}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_copy_forked_from_a_sandboxed_session_stays_sandboxed_across_a_restart() {
    use xenonet_service::compose::runtime::{sandbox, starts};
    let root = root("sandbox-fork");
    let t = two_networks_ended(&root).await;
    let w = &t.desk.world;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let (parent, child) = (DESK_ATLAS_ENDED, "claude:0190d3e0-0000-7000-8000-0000000f0f01");
    let record = sandbox::Record { folder: atlas, domains: vec!["github.com".into()] };
    sandbox::keep(w, parent, &record).unwrap();
    // The copy is named by its stream: kept as sandboxed in its own right.
    starts::note_fork(w, child, parent);
    sandbox::fork_named(w, child, &record).unwrap();
    // A restart forgets whose copy it is; it is still sandboxed, and a turn for it is too.
    starts::forget_forks(w);
    assert_eq!(sandbox::of(w, child), Some(record.clone()));
    assert_eq!(sandbox::for_turn(w, child), Ok(Some(record)));
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_run_you_didnt_start_reaches_only_the_domains_you_allowed() {
    let root = root("sandbox-domains");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let ask = |n: u32, domain: &str| {
        let body = json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": format!("tidy atlas {n}"), "domains": [domain] });
        let t = &t;
        async move { as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", body).await }
    };
    // By default such a run reaches no domain at all.
    let (s, v) = ask(1, "github.com").await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let said = told(&t, "a domain nobody allowed").await;
    assert!(said.contains("github.com") && said.contains("xenonet runs domains"), "{said}");
    // The list is the person's (the route is YOU's, as the sign-in's is: a
    // session's shell is refused by its process, which this harness has not).
    let (s, v) = t.desk.window("PUT", "/api/runs/sandbox-domains", json!({ "domains": ["localhost"] })).await;
    assert_eq!(s, 400, "a bad name is refused: {v}");
    let (s, v) = t.desk.window("PUT", "/api/runs/sandbox-domains", json!({ "domains": ["GitHub.com", "*.npmjs.org"] })).await;
    assert_eq!((s, v["domains"].clone()), (200, json!(["*.npmjs.org", "github.com"])), "{v}");
    let (s, v) = t.desk.window("GET", "/api/runs/sandbox-domains", Value::Null).await;
    assert_eq!((s, v["domains"].clone()), (200, json!(["*.npmjs.org", "github.com"])), "{v}");
    // Inside the list it runs; outside it is refused, in words.
    let (s, v) = ask(2, "registry.npmjs.org").await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    assert!(told(&t, "an allowed domain").await.contains("Started"));
    let (s, v) = ask(3, "evil.example").await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let said = told(&t, "a domain outside the list").await;
    assert!(said.contains("evil.example") && said.contains("github.com"), "{said}");
    let _ = ran(&t.desk, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 1, "only the allowed one ran");
    // The person's own start names any domain.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas 4", "domains": ["evil.example"] })).await;
    assert_eq!(s, 200, "{v}");
    // And the list can be cleared.
    let (s, v) = t.desk.window("DELETE", "/api/runs/sandbox-domains", Value::Null).await;
    assert_eq!((s, v["domains"].clone()), (200, json!([])), "{v}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_codex_start_is_sandboxed_for_the_person_alone_and_closes_its_own_config() {
    let root = root("sandbox-codex");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // Another machine's session: Codex's sandbox reads the whole disk, so it isn't started.
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "app": "codex", "prompt": "tidy atlas" })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let said = told(&t, "a codex start").await;
    assert!(said.contains("Codex") && said.contains("anyone but you"), "{said}");
    assert_eq!(fake_runs(&t.desk).len(), 0);
    // The person's own, sandboxed: with the user's config left out.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "app": "codex", "prompt": "tidy atlas again", "sandbox": true })).await;
    assert_eq!(s, 200, "{v}");
    let (args, _) = ran(&t.desk, 1).await;
    assert_eq!(args[..3], ["exec", "--sandbox", "workspace-write"], "{args:?}");
    assert!(args.iter().any(|a| a == "--ignore-user-config") && args.iter().any(|a| a == "notify=[]"), "{args:?}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_session_cannot_ask_for_no_sandbox_not_even_of_a_peer_that_would_not_know_it() {
    let root = root("sandbox-off");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // Sent to desk, an older XenoNet there would take "sandbox": false as no word on it.
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "tidy atlas", "sandbox": false })).await;
    assert!(s >= 400 && v["error"].as_str().is_some_and(|e| e.contains("sandbox") && e.contains("you")), "{s} {v}");
    // And on this machine.
    let (s, v) = as_agent(&t.desk, DESK_ATLAS, "POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas", "sandbox": false })).await;
    assert!(s >= 400 && v["error"].as_str().is_some_and(|e| e.contains("sandbox")), "{s} {v}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0);
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}
