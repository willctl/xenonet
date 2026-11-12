// Xenoliths syncing a space on loopback, in one process: each with its own temp home and folder,
// its own keys and its own iroh endpoint on 127.0.0.1. The curator founds project:atlas and
// shares it with a member, which follows it; a third that isn't shared, and a fourth that copied
// the member's transport key but not its context key, get nothing. test/sync.mjs runs the same
// with the real programs (the curator's process filing notes, the partitions).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenolith_core::chains;
use xenolith_core::inbox::{self, By, NoteInput};
use xenolith_core::keys::Keys;
use xenolith_core::paths::AppPaths;
use xenolith_core::sync::{self as cs, Follow, Settings};
use xenolith_net::{Config, Engine};

struct X {
    home: PathBuf,
    paths: AppPaths,
    root: PathBuf,
    keys: Arc<Keys>,
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
    engine: Option<Engine>,
}

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

impl X {
    fn new(name: &str) -> X {
        let home = std::env::temp_dir().join(format!("xenolith-net-{name}-{}", xenolith_core::text::random_hex(6)));
        let env = [("XENOLITH_HOME".to_string(), home.join("home").to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        let root = home.join("wiki");
        xenolith_core::upgrade::open_folder(&paths, &root).unwrap();
        let keys = Arc::new(Keys::load_or_create(&paths, &[]).unwrap());
        X { home, paths, root, keys, port: free_port(), log: Arc::default(), engine: None }
    }

    fn endpoint(&self) -> String {
        let k = xenonet_net::keys::HostKey::from_secrets(&self.keys.transport.secret(), [0; 32]).unwrap();
        k.public().sign.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn key(&self) -> String {
        use xenolith_core::keys::Signer;
        self.keys.context.public().id()
    }

    fn start(&mut self) {
        let log = self.log.clone();
        let cfg = Config {
            paths: self.paths.clone(),
            root: self.root.clone(),
            keys: self.keys.clone(),
            settings: Settings { bind: Some(format!("127.0.0.1:{}", self.port)), ..Settings::default() },
            log: Arc::new(move |w: &str| {
                if std::env::var_os("XENOLITH_TEST_LOG").is_some() {
                    eprintln!("{w}");
                }
                log.lock().unwrap().push(w.to_string());
            }),
        };
        self.engine = Some(Engine::start(cfg).unwrap());
    }

    fn atlas(&self) -> PathBuf {
        self.root.join("spaces").join("project").join("atlas")
    }

    fn logged(&self, what: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(what))
    }
}

impl Drop for X {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < Duration::from_secs(secs), "{what}: not within {secs} s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn page(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\ntype: project\n---\n\n# {title}\n\n{body}\n")
}

fn md(dir: &Path) -> Vec<String> {
    fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).map(|p| fs::read_to_string(p).unwrap_or_default()).collect()).unwrap_or_default()
}

fn note(dir: &Path, title: &str, body: &str) {
    let n = NoteInput { kind: "log".into(), source: "user".into(), app: "codex".into(), title: title.into(), body: body.into(), ..Default::default() };
    inbox::submit_note(dir, &n, "stdio", "test", &By::Local("test")).unwrap();
}

#[test]
fn a_member_and_its_curator_sync_a_space_and_strangers_get_nothing() {
    let mut a = X::new("curator");
    let mut b = X::new("member");
    let mut c = X::new("stranger");
    let mut d = X::new("thief");
    // The curator's space, with a page, founded and shared with b at write.
    xenolith_core::spaces::create(&a.root, &xenolith_core::spaces::SpaceId::parse("project:atlas").unwrap()).unwrap();
    fs::write(a.atlas().join("pages").join("db.md"), page("Atlas database", "Postgres 16.")).unwrap();
    // The standalone owner key signs only for an owner-key request in the service; the test makes
    // that key, as the service does once a request presented it.
    let keys = a.keys.clone();
    let ctx: &dyn xenolith_core::keys::Signer = &keys.context;
    let real = xenolith_core::owner::create(&a.paths, 47999, &[]).unwrap();
    let owner = keys.owner.signer(&real, &real.key).unwrap();
    xenolith_core::upgrade::reconcile(&a.paths, &a.root, ctx, owner, 1).unwrap();
    let address = xenolith_core::spaces::SpaceId::parse("project:atlas").unwrap();
    let (id, _) = chains::of_address(&a.paths, &address).unwrap();
    chains::share(&a.paths, &a.root, &address, &b.key(), &b.endpoint(), chains::Level::Write, owner, ctx, 2).unwrap();
    let f = Follow { space: id.clone(), curator: a.key(), endpoint: a.endpoint(), addrs: vec![format!("127.0.0.1:{}", a.port)], all: false };
    // b follows whatever else the curator shares with it too.
    cs::follow(&b.paths, Follow { all: true, ..f.clone() }).unwrap();
    cs::follow(&c.paths, f.clone()).unwrap();
    // d holds b's transport key, so its endpoint is b's, and its own context key.
    fs::copy(xenolith_core::keys::Use::Transport.file(&b.paths), xenolith_core::keys::Use::Transport.file(&d.paths)).unwrap();
    d.keys = Arc::new(Keys::load_or_create(&d.paths, &[]).unwrap());
    cs::follow(&d.paths, f.clone()).unwrap();
    a.start();
    b.start();

    // The curator's page reaches the member's copy, in a folder of its own.
    let copy = b.atlas().join("pages").join("db.md");
    until("the page reached the member", 20, || fs::read_to_string(&copy).ok() == fs::read_to_string(a.atlas().join("pages").join("db.md")).ok());

    // A note left on the member reaches the curator's inbox once, and leaves the member's.
    note(&b.atlas(), "Atlas moves to Postgres 17", "Logical replication.");
    until("the note reached the curator", 10, || md(&a.atlas().join("inbox")).iter().any(|n| n.contains("Atlas moves to Postgres 17")));
    let arrived = md(&a.atlas().join("inbox"));
    let n = arrived.iter().find(|n| n.contains("Atlas moves to Postgres 17")).unwrap();
    assert!(n.contains("transport: sync") && n.contains(&format!("key:{}", b.key())), "{n}");
    assert!(n.contains("source: agent"), "a peer's note never speaks as the person: {n}");
    until("the note left the member's inbox", 5, || md(&b.atlas().join("inbox")).is_empty());

    // An edit to the member's copy reaches the curator as a note within 2 s.
    fs::write(&copy, page("Atlas database", "Postgres 16, tuned on the laptop.")).unwrap();
    let t = Instant::now();
    until("the edit reached the curator", 2, || md(&a.atlas().join("inbox")).iter().any(|n| n.contains("tuned on the laptop")));
    assert!(t.elapsed() < Duration::from_secs(2));

    // An edit against a version the curator moved past waits there as a proposal.
    fs::write(a.atlas().join("pages").join("db.md"), page("Atlas database", "Postgres 16 with a replica.")).unwrap();
    until("the curator's change came back", 10, || fs::read_to_string(&copy).unwrap_or_default().contains("with a replica"));
    assert!(fs::read_dir(b.atlas().join(".history").join("pages").join("db")).unwrap().count() >= 1, "the copy keeps what it replaced");
    assert!(chains::of_address(&b.paths, &address).is_some(), "the member holds the chain");

    // A space shared with the member while they're connected reaches it, though it held nothing
    // of it (revision 6).
    let beta = xenolith_core::spaces::SpaceId::parse("project:beta").unwrap();
    xenolith_core::spaces::create(&a.root, &beta).unwrap();
    fs::write(a.root.join("spaces").join("project").join("beta").join("pages").join("plan.md"), page("Beta plan", "Start small.")).unwrap();
    xenolith_core::upgrade::reconcile(&a.paths, &a.root, ctx, owner, 3).unwrap();
    chains::share(&a.paths, &a.root, &beta, &b.key(), &b.endpoint(), chains::Level::Read, owner, ctx, 4).unwrap();
    a.engine.as_ref().unwrap().changed();
    let plan = b.root.join("spaces").join("project").join("beta").join("pages").join("plan.md");
    until("the space shared later reached the member", 20, || plan.exists());
    assert!(chains::of_address(&b.paths, &beta).is_some());
    // The member's edit of a version that's gone: held on the curator for the person's OK.
    fs::write(a.atlas().join("pages").join("db.md"), page("Atlas database", "Postgres 16, two replicas.")).unwrap();
    fs::write(&copy, page("Atlas database", "Postgres 17 on the laptop.")).unwrap();
    until("the stale edit is held on the curator", 10, || xenolith_core::held::list(&a.atlas()).iter().any(|h| h["kind"] == "sync" && h["slug"] == "db"));
    until("and the copy shows the curator's page", 10, || fs::read_to_string(&copy).unwrap_or_default().contains("two replicas"));

    // A partition: the curator goes away; a note waits on the member and its own pages change;
    // after the heal both arrive.
    a.engine = None;
    note(&b.atlas(), "Written while apart", "the curator was away");
    fs::write(a.atlas().join("pages").join("db.md"), page("Atlas database", "Postgres 16, changed while apart.")).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(!md(&a.atlas().join("inbox")).iter().any(|n| n.contains("Written while apart")));
    a.start();
    until("the note written apart reached the curator", 40, || md(&a.atlas().join("inbox")).iter().any(|n| n.contains("Written while apart")));
    until("the page changed apart reached the member", 20, || fs::read_to_string(&copy).unwrap_or_default().contains("changed while apart"));

    // A removal and a day of the log reach the member.
    fs::write(a.atlas().join("pages").join("old.md"), page("Old", "gone soon")).unwrap();
    until("old reached the member", 10, || b.atlas().join("pages").join("old.md").exists());
    fs::remove_file(a.atlas().join("pages").join("old.md")).unwrap();
    until("old was removed on the member", 10, || !b.atlas().join("pages").join("old.md").exists());
    xenolith_core::wiki::append_log(&a.atlas(), "curator", "Filed the database notes", "", &Value::Null, &json!(["db"])).unwrap();
    until("the log reached the member", 10, || cs::recent_logs(&b.atlas()).iter().any(|(_, t)| t.contains("Filed the database notes")));

    // A stranger the space isn't shared with is refused before its hello, and a member's
    // transport key without its context key is refused at its hello.
    c.start();
    std::thread::sleep(Duration::from_secs(2));
    assert!(!c.atlas().exists(), "the stranger got nothing");
    b.engine = None;
    d.start();
    until("the thief's hello was refused", 15, || a.logged("which no space here binds to that endpoint"));
    assert!(!d.atlas().exists(), "the thief got nothing");
    assert!(a.engine.as_ref().unwrap().status()["spaces"][0]["members"].as_array().unwrap().iter().all(|m| m["connected"] == false));
}
