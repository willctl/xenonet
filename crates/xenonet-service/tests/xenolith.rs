// Xenolith's shared mode over the forward (phase 6, commit 16), end to
// end: three XenoNets in this process and three Xenoliths beside them, the
// real `xenolith-service` programs (XENOLITH_RUST_BIN names `xenolith`,
// with `xenolith-service` beside it; else xenolith/rust/target's). laptop
// founds N1 with studio and N2 with desk, so laptop alone is in both. Each
// XenoNet forwards xenolith/sync/2 for its Xenolith (the registration in
// compose/forward.rs, the owner-only socket) and hands out its chains.
//
// The gates: a space shared into N2 reaches desk's Xenolith and never
// studio's, though studio's is bound and takes a space shared into N1; a
// Xenolith whose grant names no context key (no key(id)) is refused by its
// peers, and takes the space once its grant does. The person's device
// signs the move from the owner key and the shares, with presence, as the
// window would. Nothing here touches an installed XenoNet or Xenolith:
// temp homes, loopback, ports from the OS. NET_TEST_LOG=1 prints what the
// XenoNets log, XENOLITH_TEST_LOG=1 what the Xenoliths do.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_api::cores::{self, Cores};
use xenonet_authz::{Action, ActorGrant, ActorSlug, Binding, Ground, Schema, Token, ACTOR_PREFIX};
use xenonet_model::ops::{Kind, Outgoing, To};
use xenonet_net::forward::{Forwardable, Forwards, Grant, Grants};
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_net::{NetError, R};
use xenonet_os::log::Log;
use xenonet_service::config::Config;
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

const ALPN: &str = "xenolith/sync/2";

struct You;

#[async_trait]
impl cores::Verify for You {
    async fn person(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
    async fn window(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
}

/// `xenolith` and `xenolith-service`: XENOLITH_RUST_BIN (and
/// XENOLITH_SERVICE_BIN), else xenolith/rust/target/{release,debug}. A
/// missing binary fails the test: it never skips.
fn bins() -> (PathBuf, PathBuf) {
    let exe = std::env::consts::EXE_SUFFIX;
    let cli = std::env::var("XENOLITH_RUST_BIN").ok().filter(|b| !b.is_empty()).map(PathBuf::from).or_else(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xenolith/rust/target");
        ["release", "debug"].iter().map(|p| root.join(p).join(format!("xenolith{exe}"))).find(|f| f.exists())
    });
    let cli = cli.filter(|c| c.exists());
    assert!(
        cli.is_some(),
        "this test runs the Xenolith programs: build them (cargo build --release in xenolith/rust, with its pinned toolchain) or set XENOLITH_RUST_BIN to the built `xenolith`"
    );
    let cli = cli.unwrap();
    let service = std::env::var("XENOLITH_SERVICE_BIN").ok().filter(|b| !b.is_empty()).map_or_else(|| cli.with_file_name(format!("xenolith-service{exe}")), PathBuf::from);
    assert!(service.exists(), "xenolith-service isn't beside {}: build the Xenolith workspace, or set XENOLITH_SERVICE_BIN", cli.display());
    (cli, service)
}

/// A forward is decided on the machine's own grants, as the engine's are:
/// this machine's actor's token, in its time, forwarding exactly `alpn`.
struct Machine(Arc<NetService>);

impl Grants for Machine {
    fn decide(&self, token: &str, alpn: &str) -> R<Grant> {
        let facts = self.0.node().shared().facts();
        let t = Token::parse(token, ACTOR_PREFIX, &facts, now_ms() as i64).map_err(|e| NetError::new(403, "no-grant", e.message))?;
        if &t.bearer().machine != facts.own_machine() {
            return Err(NetError::new(403, "no-grant", "another machine's grant"));
        }
        if t.forward() != Some(alpn) {
            return Err(NetError::new(403, "other-alpn", "that grant forwards another ALPN"));
        }
        Ok(Grant { actor: t.bearer().slug.as_str().to_string(), ids: vec![t.id()] })
    }
}

struct Host {
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
    forwards: Arc<Forwards>,
    socket: PathBuf,
    dir: PathBuf,
}

fn options(dir: &Path, forwards: Arc<Forwards>) -> Options {
    let who = dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let mut o = Options::new(
        dir.join("state").join("network"),
        common::kept(&dir.join("state")),
        "0.0.0-test",
        "linux",
        Arc::new(move |code: &str, words: &str| {
            if std::env::var("NET_TEST_LOG").is_ok() {
                eprintln!("[{who}] {code}: {words}");
            }
        }),
    );
    o.clock = Arc::new(now_ms);
    o.timing = Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, answer_ms: 4000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 };
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o.link = true;
    o.forwards = Some(forwards);
    o
}

impl Host {
    async fn new(name: &str) -> Host {
        // Short: a socket's path has room for about a hundred bytes.
        let dir = std::env::temp_dir().join(format!("xl16-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let home = dir.join("home").to_string_lossy().into_owned();
        let state = dir.join("state").to_string_lossy().into_owned();
        let env = move |k: &str| match k {
            "HOME" | "USERPROFILE" => Some(home.clone()),
            "XENONET_STATE_DIR" => Some(state.clone()),
            _ => None,
        };
        let config = Config::from(&[], &env);
        let mut cores = Cores::pending(false, false);
        xenonet_service::serve::wire_root(&mut cores);
        cores.verify = Arc::new(You);
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        // The forwards as the service makes them: the ALPNs the composition
        // root registers, and the machine's grants deciding.
        let slot: Arc<Mutex<Option<Arc<NetService>>>> = Arc::default();
        struct Late(Arc<Mutex<Option<Arc<NetService>>>>);
        impl Grants for Late {
            fn decide(&self, token: &str, alpn: &str) -> R<Grant> {
                let svc = self.0.lock().unwrap().clone().ok_or_else(|| NetError::new(503, "no-network", "not yet"))?;
                Machine(svc).decide(token, alpn)
            }
        }
        let forwards = Forwards::new(Forwardable::new(xenonet_service::compose::forward::FORWARDED).unwrap(), Arc::new(Late(slot.clone())));
        let svc = NetService::open(options(&dir, forwards.clone())).unwrap();
        *slot.lock().unwrap() = Some(svc.clone());
        let nodes = Arc::downgrade(svc.nodes());
        forwards.set_chains(Arc::new(move || nodes.upgrade().map_or(Value::Null, |n| xenonet_service::compose::forward::chains_of(&n))));
        let _ = ctx.net.set(svc.clone());
        svc.start().await.unwrap();
        let socket = dir.join("f.sock");
        xenonet_service::compose::forward::listen(forwards.clone(), &socket, |_| {}).unwrap();
        Host { ctx, svc, forwards, socket, dir }
    }

    async fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        let headers: Vec<(&str, &str)> = vec![("x-xenonet-window", key.as_str())];
        let text = body.to_string();
        let req = Req::fake(method, path, &headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
        (r.status, r.body_json())
    }

    /// An actor grant of this machine's for its Xenolith: forward(sync/2),
    /// the ctx rights, and key(id) when `key` names one.
    fn grant(&self, key: Option<&str>) -> String {
        let identity = self.svc.nodes().common().identity.clone();
        let facts = self.svc.node().shared().facts();
        let now = now_ms() as i64;
        let root = identity.token_root(facts.own_machine(), now).unwrap();
        let g = ActorGrant {
            actor: ActorSlug::new("xenolith").unwrap(),
            network: None,
            rights: vec![Action::CtxRead, Action::CtxAppend, Action::CtxWrite],
            until: now + 30 * 86_400_000,
        };
        root.mint_actor_with(&g, &Schema::XENONET, &Binding::new(key, Some(ALPN)).unwrap()).unwrap().encode(ACTOR_PREFIX).unwrap()
    }

    /// The person's device signs a drafted space entry (`draft`: the entry,
    /// which may carry no signature yet, and its act) with their presence
    /// for it, as XenoNet's window would.
    fn sign_as_person(&self, draft: &Value) -> Value {
        let identity = self.svc.nodes().common().identity.clone();
        let device = identity.device_key().unwrap().unwrap();
        let mut unsigned = draft["entry"].as_object().unwrap().clone();
        let mut signers = unsigned.remove("signers").and_then(|s| s.as_array().cloned()).unwrap_or_default();
        let sig = device.signer.sign(xenonet_id::sign::Purpose::SPACE, Value::Object(unsigned.clone()).to_string().as_bytes()).unwrap();
        let act: [u8; 32] = xenonet_id::codec::unb64_n(draft["act"].as_str().unwrap()).unwrap();
        let block = identity.prove(true, xenonet_log::space::WIDEN, &act, "share a space", now_ms() as i64).unwrap();
        signers.push(json!({ "key": device.signer.public().id(), "sig": xenonet_id::codec::b64(&sig), "presence": block.to_json() }));
        unsigned.insert("signers".into(), Value::Array(signers));
        // What the chain will check: it reads back as an entry, and its act is the draft's.
        let e = xenonet_log::space::Entry::parse(&Value::Object(unsigned.clone())).unwrap();
        assert_eq!(e.digest(), act);
        Value::Object(unsigned)
    }

    fn person(&self) -> (String, String) {
        let identity = &self.svc.nodes().common().identity;
        (identity.person_id().unwrap(), identity.head().unwrap())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.svc.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `founder` makes a network named `name`; `joiner` joins it as `host`
/// with an invite from the founder's window, finding it at the invite's
/// door on loopback.
async fn network(founder: &Host, joiner: &Host, name: &str, me: &str, host: &str) -> String {
    let (s, v) = founder.call("POST", "/api/net/create", json!({ "name": name, "host": me })).await;
    assert_eq!(s, 200, "{v}");
    let id = v["networks"].as_array().unwrap().iter().find(|n| n["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
    let (s, t) = founder.call("POST", "/api/net/invite", json!({ "host": host, "network": name })).await;
    assert_eq!(s, 200, "{t}");
    let at = t["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
    let (s, v) = joiner.call("POST", "/api/net/join", json!({ "invite": t["invite"], "code": t["code"], "at": at })).await;
    assert_eq!(s, 200, "{v}");
    id
}

/// A Xenolith beside a XenoNet: its own temp home and folder, its service
/// running until dropped, and what it logs.
struct Lith {
    name: &'static str,
    cli: PathBuf,
    env: Vec<(String, String)>,
    wiki: PathBuf,
    child: Child,
    log: Arc<Mutex<String>>,
}

impl Lith {
    fn start(name: &'static str, host: &Host) -> Lith {
        let (cli, service) = bins();
        let home = host.dir.join("xl");
        let wiki = host.dir.join("wiki");
        std::fs::create_dir_all(&home).unwrap();
        let config = json!({ "sync": { "bind": "127.0.0.1:0", "xenonet": { "socket": host.socket.to_string_lossy() } } });
        std::fs::write(home.join("config.json"), config.to_string()).unwrap();
        let env = vec![("XENOLITH_HOME".to_string(), home.to_string_lossy().into_owned()), ("XENOLITH_DIR".to_string(), wiki.to_string_lossy().into_owned())];
        let mut cmd = Command::new(&service);
        cmd.args(["serve", "--http", "--port", "0", "--parent-stdin"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).env_remove("XENOLITH_TOKEN");
        for (k, v) in &env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let log = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let (err, l) = (child.stderr.take().unwrap(), log.clone());
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                if std::env::var("XENOLITH_TEST_LOG").is_ok() {
                    eprintln!("[xenolith {name}] {line}");
                }
                if line.contains("listening on http://127.0.0.1:") {
                    let _ = tx.send(());
                }
                let mut l = l.lock().unwrap();
                l.push_str(&line);
                l.push('\n');
            }
        });
        let started = rx.recv_timeout(Duration::from_secs(30)).is_ok();
        assert!(started, "{name}'s Xenolith didn't start:\n{}", log.lock().unwrap());
        Lith { name, cli, env, wiki, child, log }
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Value {
        let mut cmd = Command::new(&self.cli);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let mut c = cmd.spawn().unwrap();
        if let Some(s) = stdin {
            c.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        }
        drop(c.stdin.take());
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "{}: xenolith {}: {}{}", self.name, args.join(" "), String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    fn status(&self) -> Value {
        self.run(&["sync", "status", "--json"], None)["sync"].clone()
    }

    fn key(&self) -> String {
        self.status()["key"].as_str().unwrap().to_string()
    }

    fn page(&self, space: &str, slug: &str) -> Option<String> {
        std::fs::read_to_string(self.wiki.join("spaces").join("project").join(space).join("pages").join(format!("{slug}.md"))).ok()
    }

    fn holds(&self, id: &str) -> bool {
        self.status()["spaces"].as_array().is_some_and(|s| s.iter().any(|x| x["id"] == id))
    }

    fn logged(&self, what: &str) -> bool {
        self.log.lock().unwrap().contains(what)
    }
}

impl Drop for Lith {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

async fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn page(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\ntype: project\n---\n\n# {title}\n\n{body}\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_space_shared_into_one_network_reaches_its_members_alone_and_a_grant_without_its_key_binds_nothing() {
    let studio = Host::new("studio").await;
    let laptop = Host::new("laptop").await;
    let desk = Host::new("desk").await;
    let n1 = network(&laptop, &studio, "home", "laptop", "studio").await;
    let n2 = network(&laptop, &desk, "work", "laptop", "desk").await;
    let (person, _) = laptop.person();
    // link/2 up between laptop and each of the others, by a direct path: the
    // endpoint the forwards ride knows where they are.
    for h in [&studio, &laptop, &desk] {
        let body = json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [], "removed": [], "you": false, "roster": "" });
        h.svc.net().send(Outgoing { network: None, authority: None, to: To::All, kind: Kind::Presence, body }).unwrap();
    }
    let path = |s: &Value, net: usize, of: &str| s["networks"][net]["hosts"].as_array().and_then(|h| h.iter().find(|x| x["id"] == of).map(|x| x["path"].clone()));
    let mut st = Value::Null;
    for _ in 0..300 {
        st = laptop.call("GET", "/api/net/state", Value::Null).await.1;
        if path(&st, 0, "studio") == Some(json!("direct")) && path(&st, 1, "desk") == Some(json!("direct")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!((path(&st, 0, "studio"), path(&st, 1, "desk")), (Some(json!("direct")), Some(json!("direct"))), "{st}");

    let (l, s, d) = (Lith::start("laptop", &laptop), Lith::start("studio", &studio), Lith::start("desk", &desk));
    let (lk, sk, dk) = (l.key(), s.key(), d.key());
    // desk's grant names no context key: what its Xenolith says binds to nothing.
    l.run(&["sync", "grant"], Some(&laptop.grant(Some(&lk))));
    s.run(&["sync", "grant"], Some(&studio.grant(Some(&sk))));
    d.run(&["sync", "grant"], Some(&desk.grant(None)));
    for x in [&l, &s, &d] {
        until(&format!("{}'s Xenolith attached to its XenoNet", x.name), 60, || x.status()["xenonet"]["mode"] == "attached").await;
    }
    assert_eq!(laptop.forwards.list()["forwards"][0]["actor"], "xenolith");
    until("desk's Xenolith sees its grant names no key", 20, || d.status()["xenonet"]["grant"]["problem"].as_str().is_some_and(|p| p.contains("no context key"))).await;

    // laptop's Xenolith founds two spaces under its owner key; the person
    // takes them (move-owner: the owner key and the person's device), and
    // shares atlas into N2 and notes into N1, each with their presence.
    l.run(&["spaces", "add", "project:atlas"], None);
    l.run(&["spaces", "add", "project:notes"], None);
    std::fs::write(l.wiki.join("spaces/project/atlas/pages/plan.md"), page("Atlas plan", "Ship the sync.")).unwrap();
    std::fs::write(l.wiki.join("spaces/project/notes/pages/home.md"), page("Home notes", "Water the plants.")).unwrap();
    let id_of = |name: &str| {
        let addr = format!("project:{name}");
        let list = l.run(&["spaces", "--json"], None);
        list["spaces"].as_array().and_then(|a| a.iter().find(|x| x["space"] == addr.as_str()).and_then(|x| x["chain"]["id"].as_str().map(String::from)))
    };
    until("laptop's Xenolith founded its spaces", 20, || id_of("atlas").is_some() && id_of("notes").is_some()).await;
    let (atlas, notes) = (id_of("atlas").unwrap(), id_of("notes").unwrap());
    for (space, net) in [("project:atlas", &n2), ("project:notes", &n1)] {
        let (_, head) = laptop.person();
        let moved = l.run(&["spaces", "draft", space, "--move-owner", &person, "--person-head", &head], None);
        l.run(&["spaces", "sequence", space], Some(&laptop.sign_as_person(&moved).to_string()));
        let (_, head) = laptop.person();
        let share = l.run(&["spaces", "draft", space, "--share-network", net, "--level", "append", "--person-head", &head], None);
        l.run(&["spaces", "sequence", space], Some(&laptop.sign_as_person(&share).to_string()));
    }
    // Shares need the person's device: one only the owner key signed is refused.
    let (_, head) = laptop.person();
    let unsigned = l.run(&["spaces", "draft", "project:atlas", "--share-network", &n1, "--person-head", &head], None);
    let mut c = Command::new(&l.cli);
    c.args(["spaces", "sequence", "project:atlas"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in &l.env {
        c.env(k, v);
    }
    let mut c = c.spawn().unwrap();
    c.stdin.take().unwrap().write_all(unsigned["entry"].to_string().as_bytes()).unwrap();
    assert!(!c.wait_with_output().unwrap().status.success(), "a share into a network without the person's device is refused");

    // studio, in N1 alone, takes notes and never atlas.
    until("notes at studio", 60, || s.page("notes", "home").is_some_and(|p| p.contains("Water the plants."))).await;
    assert!(s.holds(&notes) && !s.holds(&atlas));
    // desk, in N2, gets nothing while its grant binds no key: laptop's
    // Xenolith refuses its hello.
    until("laptop refused desk's key-less grant", 30, || l.logged("names no context key (key(id))")).await;
    assert!(!d.holds(&atlas) && d.page("atlas", "plan").is_none(), "nothing reaches a Xenolith its grant doesn't bind");
    let bound: Vec<Value> = l.status()["xenonet"]["peers"].as_array().cloned().unwrap_or_default();
    assert!(bound.iter().any(|p| p["key"] == sk.as_str()) && !bound.iter().any(|p| p["key"] == dk.as_str()), "{bound:?}");

    // desk's grant names its key now: atlas reaches it, and its later pages.
    d.run(&["sync", "grant"], Some(&desk.grant(Some(&dk))));
    until("atlas at desk", 60, || d.page("atlas", "plan").is_some_and(|p| p.contains("Ship the sync."))).await;
    std::fs::write(l.wiki.join("spaces/project/atlas/pages/plan.md"), page("Atlas plan", "Ship the sync, then the docs.")).unwrap();
    until("atlas's new version at desk", 30, || d.page("atlas", "plan").is_some_and(|p| p.contains("then the docs"))).await;
    assert!(!d.holds(&notes), "desk is in N2 alone: notes is N1's");
    // studio never heard of atlas, then or after.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!s.holds(&atlas) && s.page("atlas", "plan").is_none(), "a space shared into N2 never reaches an N1-only member");
    assert!(!s.logged(&atlas), "studio was never told of atlas");
}
