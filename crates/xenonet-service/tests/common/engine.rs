// The engine harness: machines with the real engine and a fake agent CLI,
// joined into networks as the window does it. A test file declares it with
// `#[path = "common/engine.rs"] mod engine;` beside `mod common;`, and uses
// all of it: a file that uses part would warn about the rest.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use xenonet_api::cores::NetLink;
use xenonet_api::cores::{self, Cores, State, R};
use xenonet_api::wire::WireEnv;
use xenonet_net::node::{now_ms, Options};
use xenonet_os::log::Log;
use xenonet_service::a2a::{A2a, A2aCores, A2aParts, Limits};
use xenonet_service::compose::runtime::Runtime;
use xenonet_service::config::Config;
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::actor_gateway::{ActorGateway, GatewayCores, GatewayHooks};
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

/// The person, at the window and at their own terminal; and any session
/// is its own process, but for those in SPOOFED.
pub struct You;

/// Ids whose calls come from a process that isn't that session, as the
/// real verifier tells by the calling process's tree.
pub static SPOOFED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[async_trait]
impl cores::Verify for You {
    async fn check(&self, _peer: Peer, id: &str, _hook: bool) -> Value {
        if SPOOFED.lock().unwrap().iter().any(|s| s == id) {
            return json!({ "ok": false, "reason": "the calling process isn't that session" });
        }
        json!({ "ok": true })
    }
    async fn person(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
    async fn window(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
}

/// A made-up session: its id, title, folder and the files it edits, and
/// what its live state adds (a waiting question's text, its last message).
#[derive(Clone)]
pub struct Sess {
    pub id: &'static str,
    pub title: &'static str,
    pub cwd: String,
    pub edits: Vec<String>,
    pub live: Value,
}

struct Map {
    at: f64,
    sessions: Vec<Sess>,
}

#[async_trait]
impl State for Map {
    fn graph(&self) -> R<Option<Value>> {
        Ok(Some(json!({})))
    }
    fn graph_version(&self) -> u64 {
        1
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok(self.sessions.iter().map(|s| json!({ "id": s.id, "kind": "session", "title": s.title, "cwd": s.cwd, "updatedAt": self.at })).collect())
    }
    fn node(&self, id: &str) -> R<Option<Value>> {
        Ok(self.nodes()?.into_iter().find(|n| n["id"] == id))
    }
    fn home_of(&self, _id: &str) -> R<Value> {
        Ok(Value::Null)
    }
    fn live_ids(&self) -> R<Vec<(String, Value)>> {
        Ok(self.sessions.iter().map(|s| (s.id.to_string(), json!(self.at))).collect())
    }
    fn live_info(&self, id: &str, _now: f64) -> R<Option<Value>> {
        Ok(self.sessions.iter().find(|s| s.id == id).map(|s| {
            let mut info = json!({ "lastEventAt": self.at, "endedAt": null, "status": "tool", "needsYou": null, "edits": s.edits.iter().map(|p| json!({ "path": p, "ts": self.at - 60e3 })).collect::<Vec<_>>() });
            for (k, v) in s.live.as_object().into_iter().flatten() {
                info[k] = v.clone();
            }
            info
        }))
    }
    fn observed_live(&self, _id: &str) -> R<bool> {
        Ok(false)
    }
    fn identities(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
    fn add_block(&self, _health: Arc<dyn Fn() -> Value + Send + Sync>, _source: Option<String>) -> R<()> {
        Ok(())
    }
}

/// The program every run on a test machine starts, in place of the app's
/// CLI: it keeps its arguments and its prompt (standard input, or Grok
/// Build's prompt file) beside itself, says it isn't signed in when the
/// prompt asks it to, waits 25 seconds when it says hold-on, and
/// otherwise takes a second and ends well. With
/// --fork-session it names its copy as Claude Code's stream does. It also
/// keeps the file --settings names as `settings-N` and the names (never the
/// values) of the variables it was given, sorted, as `env-N` (`fake_files`
/// reads both), without the ones its own shell adds.
pub fn fake_cli(dir: &Path) -> String {
    let bin = dir.join("bin").join("fake-cli");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let script = r#"#!/bin/sh
here=$(dirname "$0")
prompt=$(cat)
file=""
settings=""
prev=""
for a in "$@"; do
  [ "$prev" = "--prompt-file" ] && file="$a"
  [ "$prev" = "--settings" ] && settings="$a"
  prev="$a"
done
[ -n "$file" ] && prompt=$(cat "$file")
n=$(ls "$here" | grep -c '^args-')
printf '%s\n' "$@" > "$here/args-$n"
printf '%s' "$prompt" > "$here/prompt-$n"
printf '%s' "$MCP_TOOL_TIMEOUT" > "$here/timeout-$n"
[ -n "$settings" ] && cat "$settings" > "$here/settings-$n"
env | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' | grep -v -x -e PWD -e OLDPWD -e SHLVL -e _ | sort > "$here/env-$n"
case "$*" in *--fork-session*) echo '{"type":"system","subtype":"init","session_id":"0190d3e0-0000-7000-8000-0000000f0f01"}' ;; esac
case "$prompt" in *signed-out*) echo "Not logged in · Please run /login"; exit 1 ;; esac
case "$prompt" in
  *ask-first*) echo '{"type":"result","subtype":"success","is_error":false,"result":"Want me to go ahead with the update?","permission_denials":[]}' ;;
esac
case "$prompt" in *hold-on*) sleep 25 ;; esac
sleep 1
case "$prompt" in *ask-first*|*hold-on*) ;; *) echo '{"type":"result","subtype":"success","is_error":false,"result":"Done: ledger checked.","permission_denials":[]}' ;; esac
exit 0
"#;
    std::fs::write(&bin, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin.to_string_lossy().into_owned()
}

/// The `n`th run of the fake on `h`, once it has its prompt.
pub async fn ran(h: &Host, n: usize) -> (Vec<String>, String) {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let runs = fake_runs(h);
        if let Some(r) = runs.get(n - 1).filter(|r| !r.1.is_empty()) {
            return r.clone();
        }
        assert!(std::time::Instant::now() < end, "waited for run {n} on {}: {runs:?}", h.name);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the fake ran on `h`: each run's arguments and prompt, in order.
pub fn fake_runs(h: &Host) -> Vec<(Vec<String>, String)> {
    let bin = h.dir.join("bin");
    let mut out = Vec::new();
    for n in 0.. {
        let Ok(args) = std::fs::read_to_string(bin.join(format!("args-{n}"))) else { break };
        let prompt = std::fs::read_to_string(bin.join(format!("prompt-{n}"))).unwrap_or_default();
        out.push((args.lines().map(str::to_string).collect(), prompt));
    }
    out
}

/// What the `n`th run of the fake (counting from 1, as `ran` does) on the
/// machine whose folder is `dir` was given: the contents of the file it
/// got with --settings, if any, and the sorted names of its environment.
/// None until the run has written them.
pub fn fake_files(dir: &Path, n: usize) -> Option<(Option<String>, Vec<String>)> {
    let bin = dir.join("bin");
    let env = std::fs::read_to_string(bin.join(format!("env-{}", n - 1))).ok()?;
    let settings = std::fs::read_to_string(bin.join(format!("settings-{}", n - 1))).ok();
    Some((settings, env.lines().map(str::to_string).collect()))
}

pub fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A clone of `remote` on `branch`, with one commit.
pub fn clone(dir: &Path, remote: &str, branch: &str) -> String {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", branch]);
    git(dir, &["remote", "add", "origin", remote]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", "start"]);
    dir.to_string_lossy().into_owned()
}

pub fn options(dir: &Path, who: &str, kept: Arc<dyn xenonet_net::persist::Persist>) -> Options {
    let who = who.to_string();
    let mut o = Options::new(
        dir.join("state").join("network"),
        kept,
        "0.0.0-test",
        "linux",
        Arc::new(move |code: &str, words: &str| {
            if std::env::var("NET_TEST_LOG").is_ok() {
                eprintln!("[{who}] {code}: {words}");
            }
        }),
    );
    // Fake keys: a test never touches the machine's own, or prompts.
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    // link/2's and join/2's endpoints on loopback alone.
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o
}

/// One machine: its service as `xenonet serve` puts it together, with the
/// map made up and the engine, the network and, on the founder, the
/// actor gateway with A2A, as they are.
pub struct Host {
    pub name: &'static str,
    pub ctx: Arc<Ctx>,
    pub svc: Arc<NetService>,
    pub world: Arc<Runtime>,
    pub dir: PathBuf,
    pub sessions: Vec<Sess>,
    pub door: Option<(ActorGateway, Arc<A2a>, u16)>,
}

impl Host {
    pub async fn open(name: &'static str, dir: PathBuf, sessions: Vec<Sess>, door: Option<u16>) -> Host {
        let actors = door.is_some();
        Host::open_with(name, dir, sessions, door, actors).await
    }

    /// A machine with actors on or off; the founder always has them.
    pub async fn open_with(name: &'static str, dir: PathBuf, sessions: Vec<Sess>, door: Option<u16>, actors: bool) -> Host {
        let home = dir.join("home").to_string_lossy().into_owned();
        let state = dir.join("state").to_string_lossy().into_owned();
        let on = actors || door.is_some();
        let actors = if on { "on" } else { "off" };
        // Every run this machine starts runs the fake (`fake_cli`), never
        // Claude Code, Codex or Grok Build.
        let fake = fake_cli(&dir);
        let vars = [
            ("HOME", home.as_str()),
            ("XENONET_STATE_DIR", state.as_str()),
            ("XENONET_CACHE_DIR", state.as_str()),
            ("XENONET_ACTORS", actors),
            ("XENONET_PORT", "47997"),
            ("HERMES_HOME", home.as_str()),
            ("XENONET_RUNS_BIN", fake.as_str()),
            // A permission prompt waits six seconds for an answer here.
            ("XENONET_PERMISSION_HOLD_MS", "6000"),
        ];
        let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let config = Config::from(&[], &get);
        let mut cores = Cores::pending(on, false);
        xenonet_service::serve::wire_root(&mut cores);
        cores.state = Arc::new(Map { at: now_ms(), sessions: sessions.clone() });
        let env = WireEnv::new("0.0.0-test");
        // As the binary's serve closure gives the engine its settings.
        let setup = xenonet_service::compose::runtime::Setup {
            demo: config.demo,
            actors: config.actors,
            port: config.port,
            home: config.home.clone(),
            state_dir: config.state_dir.clone(),
            projects_dir: config.projects_dir.clone(),
            version: env.version.clone(),
            root: config.root.clone(),
            runs_bin: config.runs_bin.clone(),
            permission_hold_ms: config.permission_hold_ms,
            launcher_exec: config.launcher_exec.clone(),
            made_up: xenonet_service::compose::runtime::actors::MadeUp::none(),
            db: Some(Arc::new(xenonet_store::Db::open(Path::new(&config.state_dir)).expect("the test machine's store"))),
        };
        // The real adapters, as the binary installs them at start.
        xenonet_platforms::install();
        let world = xenonet_service::compose::runtime::wire(&mut cores, setup, &env).expect("the engine's parts");
        // As the binary's serve closure plugs it.
        let plugged = world.clone();
        cores.on_net = Some(Arc::new(move |svc: &dyn NetLink| {
            xenonet_service::compose::runtime::network::plug(&plugged, svc);
        }));
        cores.verify = Arc::new(You);
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        // Its engine's events reach its console's stream, as they do in
        // `xenonet serve`.
        env.emitter.bind(&ctx);
        // The network kept in the engine's store, as `xenonet serve` keeps it.
        let kept = xenonet_service::serve::kept_network(&ctx.cores).expect("the engine's store");
        let svc = NetService::open(options(&dir, name, kept)).unwrap();
        if let Some(plug) = &ctx.cores.on_net {
            plug(&*svc);
        }
        let _ = ctx.net.set(svc.clone());
        // The founder: its actor gateway, with A2A, on the port of
        // its own that a tunnel would forward to.
        let door = match door {
            Some(port) => {
                let c = &ctx.cores;
                let g = ActorGateway::new(
                    GatewayCores {
                        engine: c.engine.clone(),
                        registration: c.registration.clone().unwrap(),
                        actors: c.actors.clone().unwrap(),
                        reports: c.reports.clone(),
                        runs: c.runs.clone(),
                    },
                    GatewayHooks::default(),
                    port,
                    None,
                    "0.0.0-test",
                );
                let a2a = A2a::new(A2aParts {
                    cores: A2aCores { engine: c.engine.clone(), actors: c.actors.clone().unwrap() },
                    db: xenonet_service::serve::store_of(&ctx.cores),
                    version: "0.0.0-test".into(),
                    home: home.clone(),
                    name: Arc::new(|| Some("home".into())),
                    now: Arc::new(now_ms),
                    limits: Limits { block: Duration::from_secs(3), ..Limits::default() },
                })
                .unwrap();
                g.set_a2a(a2a.clone());
                g.start().await.unwrap();
                Some((g, a2a, port))
            }
            None => None,
        };
        svc.start().await.unwrap();
        Host { name, ctx, svc, world, dir, sessions, door }
    }

    /// The same machine started again: its files as they were.
    pub async fn restart(self) -> Host {
        let (name, dir, sessions, port) = (self.name, self.dir.clone(), self.sessions.clone(), self.door.as_ref().map(|d| d.2));
        drop(self);
        Host::open(name, dir, sessions, port).await
    }

    pub async fn call(&self, method: &str, path: &str, body: Value, headers: &[(&str, &str)]) -> (u16, Value) {
        let text = body.to_string();
        let req = Req::fake(method, path, headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, crate::common::local(&self.ctx, req)).await.unwrap();
        (r.status, r.body_json())
    }

    /// The window's own call.
    pub async fn window(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        self.call(method, path, body, &[("x-xenonet-window", key.as_str())]).await
    }

    /// A session's call to the engine, as its hooks and MCP server make it.
    pub async fn agent(&self, me: &str, op: &str, body: Value) -> Value {
        let (s, v) = self.call("POST", &format!("/api/engine/{op}"), body, &[("x-xenonet-agent", me)]).await;
        assert_eq!(s, 200, "{} {me} {op}: {v}", self.name);
        v
    }

    pub async fn agent_fails(&self, me: &str, op: &str, body: Value) -> (u16, Value) {
        self.call("POST", &format!("/api/engine/{op}"), body, &[("x-xenonet-agent", me)]).await
    }

    /// The agt_ this machine names its session or actor `local` by, as
    /// the other machines do.
    pub fn agt(&self, local: &str) -> String {
        xenonet_service::serve::store_of(&self.ctx.cores).unwrap().agent_of(local, None, 0).unwrap().as_str().to_string()
    }

    pub async fn state(&self) -> Value {
        self.window("GET", "/api/net/state", Value::Null).await.1
    }

    pub async fn peers(&self, me: &str) -> Vec<String> {
        ids(&self.agent(me, "peers", json!({ "scope": "machine" })).await)
    }

    pub async fn engine_state(&self) -> Value {
        self.call("GET", "/api/engine/state", Value::Null, &[]).await.1
    }

    /// The receipts this host has for the messages it sent, by agent.
    pub fn receipts(&self) -> Vec<(String, Option<String>)> {
        self.world.store.with(|s| {
            s.recent(0.0)
                .unwrap()
                .iter()
                .flat_map(|m| s.net_deliveries(m.num("id").unwrap()).unwrap())
                .filter(|d| d.text("host").is_some())
                .map(|d| (d.text("agent").unwrap().to_string(), d.text("receipt").map(str::to_string)))
                .collect()
        })
    }

    /// What a PreToolUse hook tells a session about to edit `file`.
    pub async fn before_edit(&self, me: &str, cwd: &str, file: &str) -> String {
        let session = me.split_once(':').unwrap().1;
        let input = json!({ "session_id": session, "hook_event_name": "PreToolUse", "cwd": cwd, "tool_name": "Edit", "tool_input": { "file_path": format!("{cwd}/{file}") } });
        let out = self.ctx.cores.engine.hook(me.split(':').next().unwrap(), Some("PreToolUse"), &input).await.unwrap();
        out.and_then(|h| h["hookSpecificOutput"]["additionalContext"].as_str().map(str::to_string)).unwrap_or_default()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some((g, _, _)) = &self.door {
            g.stop();
        }
        self.svc.stop();
    }
}

pub fn ids(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|a| a["id"].as_str().map(str::to_string)).collect()
}

pub fn texts(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|m| m["text"].as_str().map(str::to_string)).collect()
}

pub async fn until<F: std::future::Future<Output = bool>>(what: &str, secs: u64, mut ok: impl FnMut() -> F) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !ok().await {
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// An invite made at `maker`'s window for `host`: the invite, its code,
/// and the loopback address of the invite's door, which the person gives
/// the joining machine.
pub async fn invite(maker: &Host, host: &str, window: bool) -> (String, String, String) {
    invite_to(maker, host, window, None).await
}

/// The same for the network named `network`, which a machine in two
/// networks must say.
pub async fn invite_to(maker: &Host, host: &str, window: bool, network: Option<&str>) -> (String, String, String) {
    let mut body = json!({ "host": host, "window": window });
    if let Some(n) = network {
        body["network"] = json!(n);
    }
    let (s, t) = maker.window("POST", "/api/net/invite", body).await;
    assert_eq!(s, 200, "an invite at {}: {t}", maker.name);
    assert_eq!((t["host"].as_str(), t["window"].as_bool()), (Some(host), Some(window)), "{t}");
    let at = t["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
    (t["invite"].as_str().unwrap().to_string(), t["code"].as_str().unwrap().to_string(), at)
}

/// Joins with an invite as the window does, again while the join waits
/// for an approval.
pub async fn join(h: &Host, inv: &(String, String, String)) {
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (s, v) = h.window("POST", "/api/net/join", json!({ "invite": inv.0, "code": inv.1, "at": inv.2 })).await;
        if s == 202 && v["waiting"] == true && std::time::Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        assert_eq!(s, 200, "{} joins: {v}", h.name);
        assert_eq!(v["self"]["id"], h.name);
        return;
    }
}

pub async fn live(h: &Host, hosts: usize) {
    until(&format!("{} live with {hosts} hosts", h.name), 20, || async {
        let s = h.state().await;
        s["link"]["state"] == "live" && s["hosts"].as_array().map_or(0, Vec::len) == hosts && s["hosts"].as_array().unwrap().iter().all(|x| x["online"] == true)
    })
    .await;
}

pub fn sess(id: &'static str, title: &'static str, cwd: &str) -> Sess {
    Sess { id, title, cwd: cwd.to_string(), edits: Vec::new(), live: Value::Null }
}

/// studio founds a network (a window host), laptop joins as a window host
/// and desk as a server, each with `sessions`, with invites from studio's
/// window; all three connected on link/2.
pub async fn three(root: &Path, sessions: [Vec<Sess>; 3]) -> (Host, Host, Host) {
    let [sa, sb, sc] = sessions;
    let a = Host::open("studio", root.join("studio"), sa, None).await;
    let (s, v) = a.window("POST", "/api/net/create", json!({ "name": "home", "host": "studio" })).await;
    assert_eq!(s, 200, "{v}");
    let inv = invite(&a, "laptop", true).await;
    let b = Host::open("laptop", root.join("laptop"), sb, None).await;
    join(&b, &inv).await;
    live(&b, 2).await;
    let inv = invite(&a, "desk", false).await;
    let c = Host::open("desk", root.join("desk"), sc, None).await;
    join(&c, &inv).await;
    for h in [&a, &b, &c] {
        live(h, 3).await;
    }
    (a, b, c)
}

/// A session's own call, by its header, to any route.
pub async fn as_agent(h: &Host, me: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    h.call(method, path, body, &[("x-xenonet-agent", me)]).await
}

/// The project (`prj_…`) `h` holds the folder in, as its presence names it.
pub fn project_of(h: &Host, folder: &str) -> Option<String> {
    xenonet_service::compose::runtime::checkouts::project_of(h.world.setup.db.as_ref()?, folder)
}

/// The session of Nova's on `desk` in atlas, and in ledger; the one on
/// `contractor-vm`, in its own clone of atlas.
pub const DESK_ATLAS: &str = "claude:dddddddd-0001";
pub const DESK_LEDGER: &str = "claude:dddddddd-0002";
pub const VM_ATLAS: &str = "claude:eeeeeeee-0001";

/// `desk` in two networks at once, as the daily-use plan has Nova's: N1 is
/// `home`, with `tower`; N2 is `work`, with `contractor-vm`, a machine she
/// doesn't own. Every machine runs the real engine and the fake CLI.
pub struct TwoNetworks {
    pub desk: Host,
    pub tower: Host,
    pub contractor: Host,
    /// N1's and N2's ids.
    pub n1: String,
    pub n2: String,
    /// `atlas` (shared into N2) and `ledger` (not), as `desk` names them.
    pub atlas: String,
    pub ledger: String,
}

/// `desk` founds N1 and N2, `tower` joins N1 and `contractor-vm` N2, each
/// as a server. `desk` holds atlas (a session in it) and ledger (a session
/// in it); `contractor-vm` has its own clone of atlas and a session in it.
/// N2 starts sharing nothing; only atlas is shared into it, so the network
/// is waited on until `desk` and `contractor-vm` are in contact.
pub async fn two_networks(root: &Path) -> TwoNetworks {
    two_networks_with(root, false).await
}

/// `two_networks`, with `contractor-vm` a window host of N2 when `guest_window`.
pub async fn two_networks_with(root: &Path, guest_window: bool) -> TwoNetworks {
    let desk_atlas = clone(&root.join("desk/home/atlas"), "git@github.com:acme/atlas.git", "main");
    let desk_ledger = clone(&root.join("desk/home/ledger"), "git@github.com:acme/ledger.git", "main");
    let vm_atlas = clone(&root.join("contractor-vm/home/atlas"), "https://github.com/acme/atlas", "main");
    let desk = Host::open("desk", root.join("desk"), vec![sess(DESK_ATLAS, "Add partial refunds", &desk_atlas), sess(DESK_LEDGER, "Reconcile March", &desk_ledger)], None).await;
    let n1 = founded(&desk, "home").await;
    let tower = Host::open("tower", root.join("tower"), Vec::new(), None).await;
    join(&tower, &invite_to(&desk, "tower", true, Some("home")).await).await;
    let n2 = founded(&desk, "work").await;
    let contractor = Host::open("contractor-vm", root.join("contractor-vm"), vec![sess(VM_ATLAS, "Fix the login form", &vm_atlas)], None).await;
    join(&contractor, &invite_to(&desk, "contractor-vm", guest_window, Some("work")).await).await;
    // The projects are named once the person is known (their ids are keyed
    // into it), so after the networks.
    let (atlas, ledger) = (project_of(&desk, &desk_atlas).expect("desk's atlas"), project_of(&desk, &desk_ledger).expect("desk's ledger"));
    assert_ne!(atlas, ledger);
    let (s, v) = desk.window("PUT", "/api/net/sharing", json!({ "network": "work", "sharing": [atlas] })).await;
    assert_eq!(s, 200, "{v}");
    live(&tower, 2).await;
    until("desk and contractor-vm in contact in N2", 20, || async {
        let (d, c) = (desk.state().await, contractor.state().await);
        let online = |hosts: &Value| hosts.as_array().is_some_and(|h| h.len() == 2 && h.iter().all(|x| x["online"] == true));
        online(&d["networks"][1]["hosts"]) && c["link"]["state"] == "live" && online(&c["hosts"])
    })
    .await;
    TwoNetworks { desk, tower, contractor, n1, n2, atlas, ledger }
}

/// `h` founds a network named `name`; its id.
async fn founded(h: &Host, name: &str) -> String {
    let (s, v) = h.window("POST", "/api/net/create", json!({ "name": name, "host": h.name })).await;
    assert_eq!(s, 200, "{v}");
    v["networks"].as_array().and_then(|n| n.iter().find(|n| n["name"] == name)).expect("the new network in the state")["id"].as_str().unwrap().to_string()
}
