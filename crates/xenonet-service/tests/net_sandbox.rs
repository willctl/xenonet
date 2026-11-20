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
use xenonet_service::http::Req;
use xenonet_service::routes::api;

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

/// What the `n`th run of the fake on the machine whose folder is `dir` read
/// from the descriptor it was told to: the token's length and cksum. None if
/// it was told of none.
fn fake_fd_token(dir: &std::path::Path, n: usize) -> Option<(usize, String)> {
    let got = std::fs::read_to_string(dir.join("bin").join(format!("fdtoken-{}", n - 1))).ok()?;
    let (len, sum) = got.trim().split_once(' ')?;
    Some((len.parse().ok()?, sum.to_string()))
}

/// The length and cksum of `token`, as `fake_fd_token` reports them.
fn token_mark(token: &str) -> (usize, String) {
    use std::io::Write;
    let mut c = std::process::Command::new("cksum").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(token.as_bytes()).unwrap();
    let out = String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap();
    (token.len(), out.split_whitespace().next().unwrap().to_string())
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
fn older_start(t: &TwoNetworks, n: u32, folder: &str, sandbox: Option<Value>) -> Value {
    let mid = format!("msg_00000000-0000-4000-8000-{n:012}");
    let mut request = json!({
        "folder": folder, "app": "claude", "mode": null, "prompt": "Tidy atlas", "request": mid,
        "by": { "kind": "agent", "id": VM_ATLAS, "title": "Fix the login form" },
    });
    if let Some(sandbox) = sandbox {
        request["sandbox"] = sandbox;
    }
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
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: older_start(&t, 1, &atlas, None) }).unwrap();
    until("desk logs the refusal", 20, || async { t.desk.logged("start").iter().any(|l| l.contains("was refused")) }).await;
    let refused = t.desk.logged("start").join("\n");
    assert!(
        refused.contains("a start from contractor-vm was refused: a run started by anyone but the person runs in its app's sandbox: this start asked for none"),
        "the refusal says why: {refused}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The control: the same frame with the sandbox in it runs.
    t.contractor
        .svc
        .net()
        .send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: older_start(&t, 2, &atlas, Some(json!({ "domains": [] }))) })
        .unwrap();
    let (args, _) = ran(&t.desk, 1).await;
    assert!(args.iter().any(|a| a == "--strict-mcp-config"), "it ran, sandboxed: {args:?}");
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
async fn a_runs_sign_in_comes_on_a_descriptor_and_is_in_no_environment() {
    let root = root("sandbox-fd");
    let t = two_networks(&root).await;
    let token = "made-up-token-for-the-ledger-run";
    t.desk.world.starts.set_sign_in(token);
    t.desk.world.starts.set_environment(service_env());
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // A sandboxed start from another machine, then the person's own start.
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "tidy atlas", "sandbox": true })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let _ = ran(&t.desk, 1).await;
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas again" })).await;
    assert_eq!(s, 200, "{v}");
    let _ = ran(&t.desk, 2).await;
    for n in [1, 2] {
        let (_, names) = files_ready(&t.desk, n).await;
        assert!(!names.iter().any(|k| k == "CLAUDE_CODE_OAUTH_TOKEN"), "run {n}: the token is in no environment: {names:?}");
        assert!(names.iter().any(|k| k == "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR"), "run {n} is told where to read it: {names:?}");
        assert_eq!(fake_fd_token(&t.desk.dir, n), Some(token_mark(token)), "run {n} read its token from the descriptor");
    }
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

/// The temp folder the `n`th run of the fake on `h` was told to use
/// (`CLAUDE_CODE_TMPDIR`), empty if none.
fn run_tmpdir(h: &Host, n: usize) -> String {
    std::fs::read_to_string(h.dir.join("bin").join(format!("tmpdir-{}", n - 1))).unwrap_or_default()
}

/// Claude Code lets a sandboxed command write anywhere in its temp folder,
/// which is one for every session of the user (`/tmp/claude-<uid>`, seen
/// on 2.1.327): other sessions' scratch files and scripts are there, and
/// they run outside any sandbox. So each sandboxed run gets a temp folder
/// of its own, private, made for it, whatever the service's environment
/// says; a run that isn't sandboxed gets none made.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sandboxed_run_has_a_temp_folder_of_its_own_and_no_other_sessions() {
    use std::os::unix::fs::PermissionsExt;
    let root = root("sandbox-tmp");
    let t = two_networks(&root).await;
    let mut env = service_env();
    env.push(("CLAUDE_CODE_TMPDIR".into(), "/tmp/shared-by-every-session".into()));
    t.desk.world.starts.set_environment(env);
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas" })).await;
    assert_eq!(s, 200, "{v}");
    let _ = files_ready(&t.desk, 1).await;
    let own = run_tmpdir(&t.desk, 1);
    assert!(own.is_empty() || own == "/tmp/shared-by-every-session", "an unsandboxed run gets no folder made for it: {own:?}");
    let mut seen = Vec::new();
    for n in [2, 3] {
        let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": format!("tidy atlas {n}"), "sandbox": true })).await;
        assert_eq!(s, 200, "{v}");
        let _ = files_ready(&t.desk, n).await;
        let dir = run_tmpdir(&t.desk, n);
        assert!(!dir.is_empty() && dir != "/tmp/shared-by-every-session", "run {n} gets a temp folder of its own: {dir:?}");
        let meta = std::fs::symlink_metadata(&dir).unwrap_or_else(|e| panic!("run {n}'s temp folder {dir} exists: {e}"));
        assert!(meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700, "run {n}'s temp folder is a private folder: {:o}", meta.permissions().mode());
        assert!(!std::path::Path::new(&dir).starts_with(&atlas), "and not in the folder the run works in");
        assert!(dir.starts_with("/tmp/") && !dir.starts_with("/tmp/claude-"), "under /tmp, where Claude Code's sandbox honours it, and not the shared one: {dir}");
        seen.push(dir);
    }
    assert_ne!(seen[0], seen[1], "each run has its own");
    let (settings, _) = files_ready(&t.desk, 3).await;
    let settings: Value = serde_json::from_str(&settings.unwrap()).unwrap();
    let deny = settings["sandbox"]["filesystem"]["denyWrite"].as_array().unwrap();
    assert!(deny.contains(&json!("/tmp/claude-*")) && deny.contains(&json!("/private/tmp/claude-*")), "the shared one is denied besides: {deny:?}");
    for d in &seen {
        let _ = std::fs::remove_dir_all(d);
    }
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
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new(), for_others: None };
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
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new(), for_others: None };
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
    // The control: the same session with its record resumes, sandboxed.
    let record = xenonet_service::compose::runtime::sandbox::Record { folder: atlas.clone(), domains: Vec::new(), for_others: None };
    xenonet_service::compose::runtime::sandbox::keep(&t.desk.world, DESK_ATLAS_ENDED, &record).unwrap();
    wake_atlas(&t, 2).await;
    let (args, _) = ran(&t.desk, 1).await;
    assert!(args.iter().any(|a| a == "--strict-mcp-config"), "{args:?}");
    // The store says desk started it for contractor-vm under a delegation;
    // its sandbox record is gone.
    let d = xenonet_store::delivery::Delegated { cwd: atlas, from: Some("contractor-vm".into()), at: now_ms(), intent: None };
    t.desk.world.store.0.db().put_delegated(DESK_ATLAS_ENDED, &d, 7.0 * 24.0 * 3600e3).unwrap();
    std::fs::remove_file(record_file(&t, DESK_ATLAS_ENDED)).unwrap();
    wake_atlas(&t, 3).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(fake_runs(&t.desk).len(), 1, "no second turn ran, bare or otherwise");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// A start from another machine that is no delegation (the start setting is
/// `network`) is marked in the store, for as long as the session exists:
/// with its record file gone its next turn is refused, and with the store
/// unreadable it is too.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_session_started_for_another_machine_is_never_resumed_bare_even_with_its_record_gone() {
    use xenonet_service::compose::runtime::sandbox;
    let root = root("sandbox-marked");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "tidy atlas", "domains": [] })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let (args, _) = ran(&t.desk, 1).await;
    let session = format!("claude:{}", args[2]);
    assert!(t.desk.world.store.0.db().delegated(&session).unwrap().is_none(), "no delegation: the start setting is network");
    let w = &t.desk.world;
    // The control: with its record, a turn for it is sandboxed as it was.
    let rec = sandbox::for_turn(w, &session).unwrap().expect("a record");
    assert_eq!((rec.folder.as_str(), rec.for_others.is_some()), (atlas.as_str(), true));
    assert!(sandbox::confined_for(w, &session).unwrap().is_some_and(|who| who.contains("contractor-vm")), "marked in the store, for whom");
    // Its record file is gone: the mark holds it.
    std::fs::remove_file(record_file(&t, &session)).unwrap();
    assert_eq!(sandbox::of(w, &session), None);
    let why = sandbox::for_turn(w, &session).unwrap_err();
    assert!(why.contains("started for a session on contractor-vm") && why.contains("no turn is run for it"), "{why}");
    // A session XenoNet never started confined has no mark and is as before.
    assert_eq!(sandbox::for_turn(w, "claude:0190d3e0-0000-7000-8000-0000000d0099"), Ok(None));
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
    assert!(v["error"].as_str().is_some_and(|e| e.contains("couldn't be kept") && e.contains("a session that can't be held to its sandbox later isn't started")), "{v}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_copy_forked_from_a_sandboxed_session_stays_sandboxed_across_a_restart() {
    use xenonet_service::compose::runtime::{sandbox, starts};
    let root = root("sandbox-fork");
    let t = two_networks_in(&root, false, false, false, true).await;
    let w = &t.desk.world;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let record = sandbox::Record { folder: atlas, domains: vec!["github.com".into()], for_others: Some("a session on contractor-vm".into()) };
    sandbox::keep(w, DESK_ATLAS_ENDED, &record).unwrap();
    // A message for the session wakes it, and it is open, so XenoNet runs a copy.
    let (s, v) = t.desk.window("PUT", "/api/net/settings", json!({ "network": "work", "host": "desk", "wake": "network" })).await;
    assert_eq!(s, 200, "{v}");
    until("desk knows contractor-vm's session", 20, || async { t.desk.peers(DESK_ATLAS_ENDED).await.contains(&t.contractor.agt(VM_ATLAS)) }).await;
    let frame = json!({
        "schema": "network/send/v1", "mid": "msg_00000000-0000-4000-8000-000000000091", "sender": VM_ATLAS, "senderKind": "agent", "agent": t.contractor.agt(VM_ATLAS),
        "target": t.desk.agt(DESK_ATLAS_ENDED), "body": "The fixtures moved.", "replyTo": null, "scope": null, "kind": "message", "hops": 0, "at": now_ms() as i64,
    });
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: frame }).unwrap();
    let (args, _) = ran(&t.desk, 1).await;
    assert!(args.iter().any(|a| a == "--fork-session") && args.iter().any(|a| a == "--strict-mcp-config"), "a sandboxed copy: {args:?}");
    // The copy's id is chosen before the run, and the copy is kept as sandboxed and marked in the
    // store before it starts: the fake's stream never names it (a restart mid-turn), so nothing
    // after the start could have.
    let id = args.iter().position(|a| a == "--session-id").and_then(|i| args.get(i + 1)).expect("the copy's id is given to the run");
    let child = &format!("claude:{id}");
    assert_eq!(sandbox::of(w, child), Some(record.clone()), "the copy is sandboxed from the start");
    assert_eq!(sandbox::confined_for(w, child), Ok(Some("a session on contractor-vm".to_string())), "and marked in the store");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_cell_start_is_yours_alone_and_not_yet_wired() {
    let root = root("confine-cell");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    // A session, here or on another machine, is refused in words that say why, and nothing goes out.
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "tidy atlas", "confine": "cell" })).await;
    assert!(s >= 400 && v["error"].as_str().is_some_and(|e| e.contains("cell") && e.contains("yours alone")), "{s} {v}");
    let (s, v) = as_agent(&t.desk, DESK_ATLAS, "POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas", "confine": "cell" })).await;
    assert!(s >= 400 && v["error"].as_str().is_some_and(|e| e.contains("cell") && e.contains("yours alone")), "{s} {v}");
    // A frame that claims a cell, sent by a machine's session with no person's signature, is refused at desk's door.
    let mid = "msg_00000000-0000-4000-8000-000000000031";
    let request = json!({
        "folder": atlas, "app": "claude", "mode": null, "prompt": "Tidy atlas", "request": mid, "confine": "cell",
        "by": { "kind": "agent", "id": VM_ATLAS, "title": "Fix the login form" },
    });
    let frame = json!({
        "schema": "network/send/v1", "mid": mid, "sender": VM_ATLAS, "senderKind": "agent", "agent": t.contractor.agt(VM_ATLAS),
        "target": "xenonet:start", "body": request.to_string(), "replyTo": null, "scope": "direct", "kind": "message", "hops": 0, "at": now_ms() as i64,
    });
    t.contractor.svc.net().send(Outgoing { network: None, authority: None, to: To::Host("desk".into()), kind: Kind::Message, body: frame }).unwrap();
    until("desk logs the refusal", 20, || async { t.desk.logged("start").iter().any(|l| l.contains("was refused") && l.contains("cell")) }).await;
    // The person's own, on this machine: cells aren't wired to session starts, and it says so.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas", "confine": "cell" })).await;
    assert_eq!(s, 409, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("cell starts aren't wired yet")), "{v}");
    // A request never names a wall.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas", "confine": "cell", "wall": "wall_nova_0123456789abcdef" })).await;
    assert_eq!(s, 400, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("names no wall")), "{v}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The control: the person's own vendor start still runs.
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas once more", "confine": "vendor" })).await;
    assert_eq!(s, 200, "{v}");
    let _ = ran(&t.desk, 1).await;
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// A delegated, confined run's file tools: a read within its folder is
/// answered by the delegation and any other, however the path is written,
/// comes to the person, who is the only one who can let it be sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_delegated_confined_run_reads_in_its_folder_and_nowhere_else_without_asking_you() {
    const ASKER: &str = "claude:0190d3e0-0000-7000-8000-00000000b001";
    let root = root("sandbox-delegated-reads");
    for h in ["studio", "laptop", "desk"] {
        std::fs::create_dir_all(root.join(h).join("home").join("work")).unwrap();
    }
    let home = root.join("desk/home");
    let work = home.join("work");
    std::fs::create_dir_all(home.join("other")).unwrap();
    std::fs::create_dir_all(home.join(".config/gh")).unwrap();
    std::fs::write(work.join("a.txt"), "in the folder").unwrap();
    std::fs::write(home.join("other/file"), "elsewhere").unwrap();
    std::fs::write(home.join(".config/gh/hosts.yml"), "oauth_token: made-up").unwrap();
    std::os::unix::fs::symlink(home.join("other/file"), work.join("link")).unwrap();
    let at = |h: &str| root.join(h).join("home").join("work").to_string_lossy().into_owned();
    let (a, b, c) = three(&root, [vec![], vec![sess(ASKER, "Fix the login form", &at("laptop"))], vec![sess(DESK_ATLAS, "Ledger fixtures", &at("desk"))]]).await;
    until("desk takes starts, as studio sees it", 10, || async {
        a.state().await["hosts"].as_array().unwrap().iter().any(|h| h["id"] == "desk" && h["can"].as_array().is_some_and(|c| c.contains(&json!("start"))))
    })
    .await;
    let (s, v) = a.window("PUT", "/api/net/settings", json!({ "host": "desk", "start": "delegate" })).await;
    assert_eq!(s, 200, "{v}");
    until("desk is on delegate", 10, || async { c.state().await["start"] == "delegate" }).await;
    let (s, v) = as_agent(&b, ASKER, "POST", "/api/start", json!({ "machine": "desk", "folder": at("desk"), "prompt": "hold-on: read around", "delegate": true })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let (args, _) = ran(&c, 1).await;
    let run = format!("claude:{}", args[2]);
    let ask = |tool: &'static str, input: Value| {
        let ctx = c.ctx.clone();
        let run = run.clone();
        tokio::spawn(async move {
            let req = Req::fake(
                "POST",
                "/api/runs/permission",
                &[("x-xenonet-agent", run.as_str())],
                Some(&json!({ "tool_name": tool, "input": input, "tool_use_id": "toolu_1" }).to_string()),
            );
            let r = api(&ctx, common::local(&ctx, req)).await.unwrap();
            (r.status, r.body_json())
        })
    };
    let shown = || {
        let b = b.ctx.clone();
        async move {
            let req = Req::fake("GET", "/api/runs/asks", &[("x-xenonet-window", b.window_key.reveal())], None);
            api(&b, req).await.unwrap().body_json()["asks"].as_array().cloned().unwrap_or_default()
        }
    };
    // The control: a file in the folder is answered at once.
    let input = json!({ "file_path": work.join("a.txt").to_string_lossy() });
    let (s, answer) = tokio::time::timeout(Duration::from_secs(3), ask("Read", input.clone())).await.expect("the read in the folder was held, not allowed").unwrap();
    assert_eq!((s, answer), (200, json!({ "behavior": "allow", "updatedInput": input })));
    assert!(shown().await.is_empty(), "and asks nobody");
    // The rest are held for the person, each saying why: elsewhere in home, a
    // sign-in file, the process's own environment, and a link in the folder
    // that points out of it.
    for (n, (tool, input)) in [
        ("Read", json!({ "file_path": home.join("other/file").to_string_lossy() })),
        ("Read", json!({ "file_path": home.join(".config/gh/hosts.yml").to_string_lossy() })),
        ("Read", json!({ "file_path": "/proc/self/environ" })),
        ("Grep", json!({ "pattern": "token", "path": home.join(".config").to_string_lossy() })),
        ("Glob", json!({ "pattern": format!("{}/.config/**/*", home.display()) })),
        ("Read", json!({ "file_path": work.join("link").to_string_lossy() })),
    ]
    .into_iter()
    .enumerate()
    {
        let held = ask(tool, input.clone());
        let waiting = until_value(&format!("the {tool} {n} at laptop"), 10, || async { shown().await.into_iter().find(|x| x["host"] == "desk") }).await;
        assert!(waiting["input"].as_str().unwrap().starts_with("[outside what you delegated: it reads "), "{tool} {input}: {waiting}");
        let (s, v) = b.window("POST", "/api/runs/answer", json!({ "ask": waiting["id"], "decision": "deny" })).await;
        assert_eq!((s, v["host"].clone()), (200, json!("desk")), "{v}");
        let (s, answer) = tokio::time::timeout(Duration::from_secs(10), held).await.unwrap().unwrap();
        assert_eq!((s, answer["behavior"].clone()), (200, json!("deny")), "{tool} {input}: {answer}");
    }
    // A file that hooks tools later is the person's to allow, in the folder too.
    for file in [".git/hooks/pre-commit", ".claude/settings.json", ".mcp.json", ".vscode/tasks.json"] {
        let held = ask("Write", json!({ "file_path": work.join(file).to_string_lossy(), "content": "x" }));
        let waiting = until_value(&format!("the write of {file} at laptop"), 10, || async { shown().await.into_iter().find(|x| x["host"] == "desk") }).await;
        assert!(waiting["input"].as_str().unwrap().starts_with("[outside what you delegated: it changes a file that hooks"), "{file}: {waiting}");
        b.window("POST", "/api/runs/answer", json!({ "ask": waiting["id"], "decision": "deny" })).await;
        let _ = tokio::time::timeout(Duration::from_secs(10), held).await.unwrap();
    }
    let input = json!({ "file_path": work.join("notes.md").to_string_lossy(), "content": "x" });
    let (s, answer) = tokio::time::timeout(Duration::from_secs(3), ask("Write", input.clone())).await.expect("an ordinary write in the folder was held").unwrap();
    assert_eq!((s, answer), (200, json!({ "behavior": "allow", "updatedInput": input })));
    drop((a, b, c));
    let _ = std::fs::remove_dir_all(root);
}

async fn until_value<F: std::future::Future<Output = Option<Value>>>(what: &str, secs: u64, mut get: impl FnMut() -> F) -> Value {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = get().await {
            return v;
        }
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The run of an intent is made here for the person's own id, but it is
/// unattended and its last words go out as the outcome: it is checked as
/// a run that isn't the person's, whoever's id it carries.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn an_intents_run_is_checked_as_a_run_that_is_not_the_persons() {
    use std::sync::Arc;
    use xenonet_engine::delivery::starts::{ConfineAsk, Request};
    use xenonet_service::compose::runtime::intents::Intended;
    use xenonet_service::compose::runtime::starts::{run_here, Answer, Apart, Kind};
    let root = root("sandbox-intent");
    let t = two_networks(&root).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let by = json!({ "kind": "you", "id": xenonet_model::ids::YOU, "title": "you" });
    let answer: Answer = Arc::new(|_, _, _| {});
    let intent = |n: u32, app: &str, domains: Vec<String>| Apart {
        request: Request { folder: atlas.clone(), app: app.into(), mode: None, prompt: String::new(), delegate: true, confine: ConfineAsk::Vendor(domains) },
        prompt: xenonet_authz::Information::new(format!("Tidy atlas {n}")),
        intent: Some(Intended { id: "int_nova".into(), delegation: "dlg_0123456789abcdef".into(), from: None }),
    };
    let run = |a: &Apart| run_here(&t.desk.world, &by, None, Kind::You, xenonet_model::ids::YOU, a, answer.clone());
    // Codex's sandbox reads the whole disk: not for a run that reports its last words.
    let why = run(&intent(1, "codex", Vec::new())).unwrap_err();
    assert!(why.contains("Codex") && why.contains("anyone but you"), "{why}");
    // The domains a run reaches are the ones the person allowed on this machine.
    let why = run(&intent(2, "claude", vec!["evil.example".into()])).unwrap_err();
    assert!(why.contains("evil.example") && why.contains("xenonet runs domains"), "{why}");
    // An intent that names no level, or off, carries the person's id and is still not theirs:
    // it is not run bare.
    for (n, confine) in [(4, ConfineAsk::Off), (5, ConfineAsk::Unset)] {
        let mut a = intent(n, "claude", Vec::new());
        a.request.confine = confine;
        let why = run(&a).unwrap_err();
        assert!(why.contains("anyone but the person runs in its app's sandbox"), "{why}");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The control: the intent that asks for no domain, in Claude Code, runs.
    run(&intent(3, "claude", Vec::new())).unwrap();
    let (args, _) = ran(&t.desk, 1).await;
    assert!(args.iter().any(|a| a == "--strict-mcp-config"), "it ran, sandboxed: {args:?}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// Claude Code loads a folder's own skills, agents and commands whatever
/// settings sources it is given, and their hooks run outside its sandbox: a
/// confined start in a folder that has some isn't made.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_confined_claude_start_is_refused_in_a_folder_whose_skills_or_agents_carry_hooks() {
    let root = root("sandbox-hooks");
    let t = two_networks(&root).await;
    let atlas = root.join("desk/home/atlas");
    let agent = atlas.join(".claude/agents/guard.md");
    std::fs::create_dir_all(agent.parent().unwrap()).unwrap();
    std::fs::write(&agent, "---\nname: guard\nhooks:\n  Stop: []\n---\nGuard.\n").unwrap();
    let folder = atlas.to_string_lossy().into_owned();
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": folder, "prompt": "tidy atlas", "sandbox": true })).await;
    assert_eq!(s, 409, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains(".claude/agents/guard.md") && e.contains("outside the sandbox")), "{v}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The control: without the hooks the same start runs, and the person's own unconfined start never mattered.
    std::fs::write(&agent, "---\nname: guard\n---\nGuard.\n").unwrap();
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": folder, "prompt": "tidy atlas again", "sandbox": true })).await;
    assert_eq!(s, 200, "{v}");
    let _ = ran(&t.desk, 1).await;
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// The folder's hooks are looked for on every turn of a confined Claude
/// session, not only at its start: a skill or command added since would
/// otherwise run outside the sandbox on the next turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn hooks_added_to_the_folder_between_turns_refuse_the_next_turn() {
    use xenonet_service::compose::runtime::sandbox;
    let root = root("sandbox-hooks-later");
    let t = two_networks_ended(&root).await;
    let w = &t.desk.world;
    let atlas = root.join("desk/home/atlas");
    std::fs::create_dir_all(atlas.join("sub/.claude/commands")).unwrap();
    let folder = atlas.to_string_lossy().into_owned();
    let record = sandbox::Record { folder, domains: Vec::new(), for_others: None };
    sandbox::keep(w, DESK_ATLAS_ENDED, &record).unwrap();
    // The control: no hooks, and the turn's arguments are made.
    assert!(sandbox::turn(w, "claude", &record, "run-before", Vec::new()).is_ok());
    std::fs::write(atlas.join("sub/.claude/commands/ship.md"), "---\nhooks:\n  Stop: []\n---\nShip.\n").unwrap();
    let why = sandbox::turn(w, "claude", &record, "run-after", Vec::new()).unwrap_err();
    assert!(why.contains("sub/.claude/commands/ship.md") && why.contains("outside the sandbox"), "{why}");
    // And a message that wakes the session runs nothing while they are there.
    wake_atlas(&t, 1).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "no turn ran");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// In acceptEdits, auto and dontAsk Claude Code approves edits and commands
/// without calling the permission tool, so the delegation's judge never sees
/// them: only the person starts a run with a mode of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_start_that_is_not_the_persons_may_not_pick_a_permission_mode() {
    let root = root("sandbox-mode");
    let t = two_networks(&root).await;
    desk_takes_starts(&t).await;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    for (n, mode) in ["acceptEdits", "auto", "dontAsk", "plan"].into_iter().enumerate() {
        let body = json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": format!("tidy atlas {n}"), "sandbox": true, "mode": mode });
        let (s, v) = as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", body).await;
        assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{mode}: {v}");
        until("desk logs the refusal", 20, || async { t.desk.logged("start").iter().filter(|l| l.contains("permission mode")).count() > n }).await;
    }
    let refused = t.desk.logged("start").join("\n");
    assert!(refused.contains("only the person picks a permission mode"), "the refusal says why: {refused}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake_runs(&t.desk).len(), 0, "nothing ran");
    // The controls: the same start without a mode runs, and the person's own with one does too.
    let (s, v) =
        as_agent(&t.contractor, VM_ATLAS, "POST", "/api/start", json!({ "machine": "desk", "network": "work", "folder": atlas, "prompt": "tidy atlas", "sandbox": true })).await;
    assert_eq!((s, v["state"].clone()), (200, json!("sent")), "{v}");
    let _ = ran(&t.desk, 1).await;
    let (s, v) = t.desk.window("POST", "/api/start", json!({ "folder": atlas, "prompt": "tidy atlas again", "mode": "acceptEdits" })).await;
    assert_eq!(s, 200, "{v}");
    let (args, _) = ran(&t.desk, 2).await;
    assert!(args.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]), "{args:?}");
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}

/// A record kept before the store marked who a session was for has no `for`
/// field: read as a start that wasn't the person's, the narrowest reading, so
/// it can never be resumed bare. The person's own records say `for: null`
/// and stay theirs.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_record_from_before_the_marks_is_marked_once_as_not_the_persons() {
    use xenonet_service::compose::runtime::sandbox;
    let root = root("sandbox-backfill");
    let t = two_networks_ended(&root).await;
    let w = &t.desk.world;
    let atlas = root.join("desk/home/atlas").to_string_lossy().into_owned();
    let (old, own) = ("claude:0190d3e0-0000-7000-8000-0000000d0101", "claude:0190d3e0-0000-7000-8000-0000000d0102");
    sandbox::keep(w, own, &sandbox::Record { folder: atlas.clone(), domains: Vec::new(), for_others: None }).unwrap();
    let dir = std::path::Path::new(&w.setup.state_dir).join("sandboxed");
    let legacy = dir.join(format!("{}.json", old.replace(':', "_")));
    std::fs::write(&legacy, json!({ "folder": atlas, "domains": ["github.com"] }).to_string()).unwrap();
    assert_eq!(sandbox::confined_for(w, old), Ok(None), "no mark yet");
    assert_eq!(sandbox::backfill_marks(w), 1, "one record was marked");
    let who = sandbox::confined_for(w, old).unwrap().expect("marked");
    assert!(who.contains("before"), "it says why it is marked: {who}");
    assert_eq!(sandbox::of(w, old).map(|r| r.for_others.is_some()), Some(true), "and its record says so too");
    assert_eq!(sandbox::confined_for(w, own), Ok(None), "the person's own record stays theirs");
    // Once: a second start of the service changes nothing.
    assert_eq!(sandbox::backfill_marks(w), 0);
    // And a turn for the marked session with its record gone is refused, never run bare.
    std::fs::remove_file(&legacy).unwrap();
    assert!(sandbox::for_turn(w, old).is_err());
    drop(t);
    let _ = std::fs::remove_dir_all(root);
}
