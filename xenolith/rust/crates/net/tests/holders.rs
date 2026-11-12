// Fetching a space's curator feed from any member that holds it (review revision 14), on loopback
// in one process: with the curator away, one member fills another; a holder that claims more and
// never answers is skipped after its ten seconds; a holder that holds an Unshare serves the removed
// member nothing; and a member's Want for a note feed, or for a space it isn't in, gets nothing
// (revision 12). The rogue members here are bare link/2 endpoints with a member's keys, speaking
// sync/2 by hand: what a member's Xenolith could do if it were changed to.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenolith_core::chains::{self, Level};
use xenolith_core::inbox::{self, By, NoteInput};
use xenolith_core::keys::{Keys, Signer};
use xenolith_core::paths::AppPaths;
use xenolith_core::spaces::SpaceId;
use xenolith_core::sync::Settings;
use xenolith_net::join::Choice;
use xenolith_net::{Config, Engine};
use xenonet_log::feed::Pos;
use xenonet_net::admission::{Admission, Doors};
use xenonet_net::dial::{Link, Mesh, PeerId};
use xenonet_net::endpoint::{IrohDial, LanLookup, Relays};
use xenonet_net::envelope::Envelope;
use xenonet_net::link2::{Control, Links, Machine, NetHello, Out};

struct X {
    home: PathBuf,
    paths: AppPaths,
    root: PathBuf,
    keys: Arc<Keys>,
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
    engine: Option<Engine>,
    owner: Option<xenolith_core::owner::OwnerKey>,
}

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl X {
    fn new(name: &str) -> X {
        let home = std::env::temp_dir().join(format!("xenolith-holders-{name}-{}", xenolith_core::text::random_hex(6)));
        let env = [("XENOLITH_HOME".to_string(), home.join("home").to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        let root = home.join("wiki");
        xenolith_core::upgrade::open_folder(&paths, &root).unwrap();
        let keys = Arc::new(Keys::load_or_create(&paths, &[]).unwrap());
        X { home, paths, root, keys, port: free_port(), log: Arc::default(), engine: None, owner: None }
    }

    fn key(&self) -> String {
        self.keys.context.public().id()
    }

    fn peer(&self) -> PeerId {
        PeerId(xenonet_net::keys::HostKey::from_secrets(&self.keys.transport.secret(), [0; 32]).unwrap().public().sign)
    }

    fn endpoint(&self) -> String {
        hex(self.peer().0)
    }

    fn owner(&self) -> &dyn Signer {
        let real = self.owner.as_ref().unwrap();
        self.keys.owner.signer(real, &real.key).unwrap()
    }

    fn curate(&mut self, spaces: &[&str]) {
        self.owner = Some(xenolith_core::owner::create(&self.paths, 47999, &[]).unwrap());
        for s in spaces {
            xenolith_core::spaces::create(&self.root, &SpaceId::parse(s).unwrap()).unwrap();
        }
        let ctx: &dyn Signer = &self.keys.context;
        xenolith_core::upgrade::reconcile(&self.paths, &self.root, ctx, self.owner(), 1).unwrap();
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

    fn engine(&self) -> &Engine {
        self.engine.as_ref().unwrap()
    }

    fn at(&self) -> Vec<std::net::SocketAddr> {
        vec![format!("127.0.0.1:{}", self.port).parse().unwrap()]
    }

    /// `member` joins `spaces` (curated here) at `level`, with an invite.
    fn invite_in(&self, member: &X, spaces: &[&str], level: Level) {
        let spaces: Vec<SpaceId> = spaces.iter().map(|s| SpaceId::parse(s).unwrap()).collect();
        let v = self.engine().invite(&spaces, level, &self.owner.as_ref().unwrap().key, 600_000).unwrap();
        member.engine().join(v["invite"].as_str().unwrap(), v["code"].as_str().unwrap(), &Choice { at: self.at(), ..Choice::default() }).unwrap();
    }

    fn page(&self, space: &str, slug: &str) -> PathBuf {
        let (kind, name) = space.split_once(':').unwrap();
        self.root.join("spaces").join(kind).join(name).join("pages").join(format!("{slug}.md"))
    }

    fn logged(&self, what: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(what))
    }

    fn space(&self, address: &str) -> (String, xenonet_log::space::Space) {
        chains::of_address(&self.paths, &SpaceId::parse(address).unwrap()).unwrap()
    }
}

impl Drop for X {
    fn drop(&mut self) {
        self.engine = None;
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

// ---------------------------------------------------------------- a rogue member

/// What a rogue member saw come back.
#[derive(Default)]
struct Seen {
    envelopes: Vec<(String, String)>,
    lacks: Vec<(String, String, String)>,
    wants: usize,
}

/// A member's keys on a bare sync/2 endpoint: it says in its hello that it holds `claims` of a
/// feed, never answers a Want, and sends the Wants the test gives it.
struct Rogue {
    links: Arc<Links>,
    dial: Arc<IrohDial>,
    seen: Arc<Mutex<Seen>>,
    members: Mutex<std::collections::BTreeMap<String, Vec<PeerId>>>,
    // Last: the endpoint closes on it.
    rt: tokio::runtime::Runtime,
}

struct RogueSide {
    keys: Arc<Keys>,
    me: [u8; 32],
    claims: Vec<(String, String, Pos)>,
    seen: Arc<Mutex<Seen>>,
}

impl Machine for RogueSide {
    fn now_ms(&self) -> u64 {
        u64::try_from(xenolith_core::text::now_ms()).unwrap()
    }
    fn hello(&self, _peer: PeerId, network: &str) -> Option<NetHello> {
        let feeds = self.claims.iter().filter(|(g, _, _)| g == network).map(|(_, a, p)| (a.clone(), *p)).collect();
        Some(NetHello { network: network.to_string(), chain: None, feeds })
    }
    fn hello_in(&self, _peer: PeerId, _h: &NetHello) -> Vec<Out> {
        vec![]
    }
    fn envelope_in(&self, _peer: PeerId, env: Envelope) -> Vec<Out> {
        self.seen.lock().unwrap().envelopes.push((env.network.clone(), env.author.clone()));
        vec![]
    }
    fn want_in(&self, _peer: PeerId, _network: &str, _author: &str, _from: Pos) -> Vec<Out> {
        // Withholds: never answers.
        self.seen.lock().unwrap().wants += 1;
        vec![]
    }
    fn ack_in(&self, _peer: PeerId, _network: &str, _author: &str, _at: Pos) {}
    fn other_in(&self, _peer: PeerId, record: Control) {
        if let Control::DontHave { network, author, why, .. } = record {
            self.seen.lock().unwrap().lacks.push((network, author, why.as_str().to_string()));
        }
    }
    fn hello_ext(&self, link: &Link) -> Option<Value> {
        xenolith_net::hello::make(
            &self.keys.context,
            &self.me,
            &link.peer().0,
            &[],
            &xenolith_net::hello::Where::default(),
            None,
            link.exporter(xenolith_net::hello::EXPORTER).as_ref(),
            xenolith_core::text::now_ms(),
        )
    }
    fn hello_ext_in(&self, _link: &Link, _ext: Option<&Value>) -> xenonet_net::R<()> {
        Ok(())
    }
}

impl Rogue {
    /// `x`'s keys, saying it holds `claims` (space, author, place).
    fn new(x: &X, claims: Vec<(String, String, Pos)>) -> Rogue {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let key = xenonet_net::keys::HostKey::from_secrets(&x.keys.transport.secret(), [0; 32]).unwrap();
        let side = Arc::new(RogueSide { keys: x.keys.clone(), me: key.public().sign, claims, seen: seen.clone() });
        let port = free_port();
        let (links, dial) = rt.block_on(async {
            let settings = xenonet_net::endpoint::Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some(format!("127.0.0.1:{port}").parse().unwrap()), roots: vec![] };
            let admission = Arc::new(Admission::with(Doors { members: vec![xenolith_net::protocol().alpn], join: None }));
            let dial = Arc::new(IrohDial::bind(&key, &settings, admission.clone()).await.unwrap());
            let mesh = Arc::new(Mesh::new(dial.clone(), admission));
            let links = Links::over(xenolith_net::protocol(), mesh, side);
            tokio::spawn(links.clone().accept());
            (links, dial)
        });
        Rogue { links, dial, seen, members: Mutex::default(), rt }
    }

    /// Connects to `to`, as a member of `groups` there.
    fn connect(&self, to: &X, groups: &[String]) {
        let _rt = self.rt.enter();
        for g in groups {
            let mut m = self.members.lock().unwrap();
            let list = m.entry(g.clone()).or_default();
            list.push(to.peer());
            self.links.mesh().set_members(g, list.clone());
        }
        self.dial.addresses().put(to.peer(), &to.at(), None, Duration::from_secs(600)).unwrap();
        self.rt.block_on(async {
            let t = Instant::now();
            while self.links.connect(to.peer()).await.is_err() {
                assert!(t.elapsed() < Duration::from_secs(15), "the rogue couldn't connect");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }

    fn want(&self, to: &X, group: &str, author: &str) {
        let _rt = self.rt.enter();
        let _ = self.links.send(to.peer(), Out::Control(Control::Want { network: group.to_string(), author: author.to_string(), inc: 0, from: 1 }));
    }

    /// Lets the runtime run for `ms`.
    fn run(&self, ms: u64) {
        self.rt.block_on(async move { tokio::time::sleep(Duration::from_millis(ms)).await });
    }
}

// ---------------------------------------------------------------- the gates

#[test]
fn with_the_curator_away_a_member_fills_another_and_a_withholding_holder_is_skipped() {
    let mut a = X::new("curator");
    let mut b = X::new("holder");
    let mut c = X::new("reader");
    let w = X::new("withholder");
    a.curate(&["project:atlas"]);
    fs::write(a.page("project:atlas", "db"), page("Atlas database", "Postgres 16.")).unwrap();
    a.start();
    b.start();
    c.start();
    // b joins before c, so the curator's hello to c tells it where b is.
    a.invite_in(&b, &["project:atlas"], Level::Append);
    until("b got the page", 20, || b.page("project:atlas", "db").exists());
    a.invite_in(&c, &["project:atlas"], Level::Read);
    until("c got the page", 20, || c.page("project:atlas", "db").exists());
    let (space, _) = a.space("project:atlas");
    // w is a member at read too, shared by hand: it never runs a Xenolith, only the rogue.
    chains::share(&a.paths, &a.root, &SpaceId::parse("project:atlas").unwrap(), &w.key(), &w.endpoint(), Level::Read, a.owner(), &a.keys.context, 2).unwrap();
    a.engine().changed();
    until("c heard of w's share", 20, || chains::members(&c.space("project:atlas").1).iter().any(|m| m.key == w.key()));
    until("b connected to c", 20, || b.engine().status()["endpoint"]["peers"].as_array().is_some_and(|p| p.iter().any(|x| x["endpoint"] == c.endpoint())));

    // c is away while the curator writes a page; b takes it; then the curator goes too.
    c.engine = None;
    b.engine = None;
    std::thread::sleep(Duration::from_millis(300));
    b.start();
    fs::write(a.page("project:atlas", "fresh"), page("Fresh", "made while c was away")).unwrap();
    until("b got the fresh page", 20, || b.page("project:atlas", "fresh").exists());
    b.engine = None;
    a.engine = None;

    // c comes back to w alone, which says it holds far more of the curator's feed and never
    // answers; then b comes back. c asks w first, and b once w's ten seconds are up.
    c.start();
    let rogue = Rogue::new(&w, vec![(space.clone(), a.key(), Pos::new(u64::MAX / 2, 999))]);
    rogue.connect(&c, std::slice::from_ref(&space));
    let asked = Instant::now();
    while rogue.seen.lock().unwrap().wants == 0 {
        assert!(asked.elapsed() < Duration::from_secs(15), "c never asked the withholder");
        rogue.run(50);
    }
    let asked = Instant::now();
    b.start();
    let fresh = c.page("project:atlas", "fresh");
    while !fresh.exists() {
        assert!(asked.elapsed() < Duration::from_secs(40), "c never got the fresh page from b");
        rogue.run(100);
    }
    assert!(asked.elapsed() >= Duration::from_secs(8), "c waited out the withholder's ten seconds first ({:?})", asked.elapsed());
    assert_eq!(fs::read_to_string(&fresh).unwrap(), fs::read_to_string(b.page("project:atlas", "fresh")).unwrap());
    assert!(c.logged("from a holder"), "{:?}", c.log.lock().unwrap());
}

#[test]
fn a_holder_serves_no_one_it_holds_an_unshare_of_no_note_feed_and_no_space_its_asker_isnt_in() {
    let mut a = X::new("curator");
    let mut b = X::new("holder");
    let mut c = X::new("removed");
    let r = X::new("rogue");
    a.curate(&["project:atlas", "project:beta"]);
    fs::write(a.page("project:atlas", "db"), page("Atlas database", "Postgres 16.")).unwrap();
    fs::write(a.page("project:beta", "plan"), page("Beta plan", "Start small.")).unwrap();
    a.start();
    b.start();
    c.start();
    a.invite_in(&b, &["project:atlas", "project:beta"], Level::Append);
    a.invite_in(&c, &["project:atlas"], Level::Read);
    until("c got the page", 20, || c.page("project:atlas", "db").exists());
    until("b got both spaces", 20, || b.page("project:beta", "plan").exists() && b.page("project:atlas", "db").exists());
    let (atlas, _) = a.space("project:atlas");
    let (beta, _) = a.space("project:beta");
    chains::share(&a.paths, &a.root, &SpaceId::parse("project:atlas").unwrap(), &r.key(), &r.endpoint(), Level::Read, a.owner(), &a.keys.context, 3).unwrap();
    a.engine().changed();
    until("b heard of r's share", 20, || chains::members(&b.space("project:atlas").1).iter().any(|m| m.key == r.key()));

    // b leaves a note in atlas: it goes to the curator alone.
    let n = NoteInput { kind: "log".into(), source: "agent".into(), app: "codex".into(), title: "A note from b".into(), body: "for the curator".into(), ..Default::default() };
    inbox::submit_note(&b.root.join("spaces/project/atlas"), &n, "stdio", "test", &By::Local("test")).unwrap();
    until("the note reached the curator", 20, || {
        fs::read_dir(a.root.join("spaces/project/atlas/inbox")).unwrap().flatten().any(|e| fs::read_to_string(e.path()).unwrap_or_default().contains("A note from b"))
    });

    // r, a member of atlas at read, asks the curator and b for b's note feed, and b for beta,
    // which it isn't in: nothing of either. Asked for the curator's feed in atlas, b serves it.
    let rogue = Rogue::new(&r, vec![]);
    rogue.connect(&a, std::slice::from_ref(&atlas));
    rogue.connect(&b, &[atlas.clone(), beta.clone()]);
    rogue.want(&a, &atlas, &b.key());
    rogue.want(&b, &atlas, &b.key());
    rogue.want(&b, &beta, &a.key());
    rogue.run(1500);
    {
        let seen = rogue.seen.lock().unwrap();
        assert!(!seen.envelopes.iter().any(|(_, author)| *author == b.key()), "no note of b's reached a read member: {:?}", seen.envelopes);
        assert!(!seen.envelopes.iter().any(|(g, _)| *g == beta), "nothing of beta reached r: {:?}", seen.envelopes);
    }
    let before = rogue.seen.lock().unwrap().envelopes.len();
    rogue.want(&b, &atlas, &a.key());
    let t = Instant::now();
    while rogue.seen.lock().unwrap().envelopes.len() == before {
        assert!(t.elapsed() < Duration::from_secs(10), "b never served the curator's feed to a member");
        rogue.run(100);
    }
    assert!(rogue.seen.lock().unwrap().envelopes[before..].iter().all(|(g, author)| *g == atlas && *author == a.key()));
    drop(rogue);

    // The curator unshares c; b holds the Unshare. The curator writes a page after it and goes
    // away. c, which never learned of the Unshare, asks b: b serves it nothing.
    chains::unshare(&a.paths, &a.root, &SpaceId::parse("project:atlas").unwrap(), &c.key(), a.owner(), &a.keys.context, 4).unwrap();
    a.engine().changed();
    until("b holds the unshare", 20, || !chains::members(&b.space("project:atlas").1).iter().any(|m| m.key == c.key()));
    assert!(chains::members(&c.space("project:atlas").1).iter().any(|m| m.key == c.key()), "c's own copy is stale");
    fs::write(a.page("project:atlas", "after"), page("After", "written after c was removed")).unwrap();
    until("b got the page written after", 20, || b.page("project:atlas", "after").exists());
    a.engine = None;
    c.engine = None;
    c.start();
    std::thread::sleep(Duration::from_secs(4));
    assert!(!c.page("project:atlas", "after").exists(), "the removed member got nothing stored after the unshare");
    let peers = b.engine().status()["endpoint"]["peers"].clone();
    assert!(!peers.as_array().unwrap().iter().any(|p| p["endpoint"] == c.endpoint()), "b refused c's connection: {peers}");
}
