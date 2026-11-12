// join/1 on loopback, in one process: a curator invites, a member joins with the code; and what
// goes on the wire, which test/join.mjs can't see: a wrong code is refused with no proof of the
// inviter's in the answer, a proof caught by a fake inviter fails at the real one, and an invite
// past its time is refused.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenolith_core::chains;
use xenolith_core::keys::{Keys, Signer};
use xenolith_core::paths::AppPaths;
use xenolith_core::spaces::SpaceId;
use xenolith_core::sync::Settings;
use xenolith_net::join::{self, Choice, TARGET};
use xenolith_net::{Config, Engine};
use xenonet_net::admission::{Admission, Doors};
use xenonet_net::dial::{Mesh, PeerId};
use xenonet_net::endpoint::{IrohDial, LanLookup, Relays};
use xenonet_net::join::{Bound, Invite};

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

impl X {
    fn new(name: &str) -> X {
        let home = std::env::temp_dir().join(format!("xenolith-join-{name}-{}", xenolith_core::text::random_hex(6)));
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

    /// The owner key a running service would have made, and the space founded with it.
    fn curate(&mut self, space: &str) {
        let real = xenolith_core::owner::create(&self.paths, 47999, &[]).unwrap();
        let owner = self.keys.owner.signer(&real, &real.key).unwrap();
        xenolith_core::spaces::create(&self.root, &SpaceId::parse(space).unwrap()).unwrap();
        let ctx: &dyn Signer = &self.keys.context;
        xenolith_core::upgrade::reconcile(&self.paths, &self.root, ctx, owner, 1).unwrap();
        self.owner = Some(real);
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

    fn invite(&self, ttl_ms: i64) -> Value {
        let atlas = SpaceId::parse("project:atlas").unwrap();
        self.engine().invite(&[atlas], chains::Level::Append, &self.owner.as_ref().unwrap().key, ttl_ms).unwrap()
    }

    fn at(&self) -> Vec<std::net::SocketAddr> {
        vec![format!("127.0.0.1:{}", self.port).parse().unwrap()]
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

/// A bare endpoint that speaks join/1 by hand: what a fake inviter, or a joiner that writes its
/// own records, can do.
struct Raw {
    rt: tokio::runtime::Runtime,
    mesh: Arc<Mesh>,
    dial: Arc<IrohDial>,
}

impl Raw {
    fn new() -> Raw {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let seed: [u8; 32] = xenonet_id::codec::random().unwrap();
        let key = xenonet_net::keys::HostKey::from_secrets(&seed, [7; 32]).unwrap();
        let (mesh, dial) = rt.block_on(async {
            let settings = xenonet_net::endpoint::Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some("127.0.0.1:0".parse().unwrap()), roots: vec![] };
            let admission = Arc::new(Admission::with(Doors { members: vec![], join: Some(TARGET.alpn) }));
            let dial = Arc::new(IrohDial::bind(&key, &settings, admission.clone()).await.unwrap());
            (Arc::new(Mesh::new(dial.clone(), admission)), dial)
        });
        Raw { rt, mesh, dial }
    }

    fn me(&self) -> PeerId {
        self.mesh.me()
    }

    /// One join/1 ask, written by hand: the record sent, and the inviter's answer as it came.
    fn ask(&self, to: &X, invite: &Invite, record: &Value) -> Value {
        self.try_ask(to, invite, record).unwrap()
    }

    fn try_ask(&self, to: &X, invite: &Invite, record: &Value) -> Result<Value, String> {
        let at = to.at();
        self.dial.addresses().put(invite.inviter, &at, None, Duration::from_secs(600)).unwrap();
        self.rt.block_on(async {
            self.mesh.begin_join(invite.inviter);
            let dialled = self.mesh.dial(invite.inviter, TARGET.alpn).await;
            self.mesh.end_join(&invite.inviter);
            let link = dialled.map_err(|e| e.message)?;
            let (mut w, mut r) = link.open_bi().await.unwrap();
            let bytes = record.to_string().into_bytes();
            w.write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes()).await.unwrap();
            w.write_all(&bytes).await.unwrap();
            w.flush().await.unwrap();
            let _ = w.shutdown().await;
            let mut len = [0u8; 4];
            r.read_exact(&mut len).await.unwrap();
            let mut out = vec![0u8; u32::from_be_bytes(len) as usize];
            r.read_exact(&mut out).await.unwrap();
            link.close(xenonet_net::dial::Close::Done);
            Ok(serde_json::from_slice(&out).unwrap())
        })
    }
}

fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The joiner's record for `invite`, proved with `code` over `inviter` and `joiner` as the
/// proof's endpoint ids.
fn record(invite: &Invite, code: &str, inviter: PeerId, joiner: PeerId, add: &str) -> Value {
    let code = join::normalize(code).unwrap();
    let hash = xenonet_id::codec::sha256(code.as_bytes());
    let add_hash = xenonet_id::codec::sha256(add.as_bytes());
    let b = Bound { ticket: &invite.ticket, network: &invite.network, inviter: &inviter, joiner: &joiner, add: &add_hash };
    json!({ "schema": TARGET.schema, "ticket": invite.ticket.to_string(), "network": invite.network, "add": add, "proof": TARGET.joiner_proof(&hash, &b) })
}

#[test]
fn a_member_joins_with_the_code_and_the_wire_gives_nothing_to_a_wrong_one() {
    let mut a = X::new("curator");
    let mut b = X::new("member");
    a.curate("project:atlas");
    fs::write(a.root.join("spaces/project/atlas/pages/db.md"), "---\ntitle: Atlas\ntype: project\n---\n\n# Atlas\n\nPostgres 16.\n").unwrap();
    a.start();
    b.start();

    // A wrong code: refused, and the answer holds no proof of the inviter's.
    let v = a.invite(join::TICKET_MS);
    let inv = Invite::parse_for(&TARGET, v["invite"].as_str().unwrap()).unwrap();
    let code = v["code"].as_str().unwrap().to_string();
    let raw = Raw::new();
    let add = json!({ "key": "xns1_raw", "endpoint": hex(raw.me().0) }).to_string();
    let wrong = format!("{}{}", if code.starts_with('A') { "B" } else { "A" }, &code[1..]);
    let answer = raw.ask(&a, &inv, &record(&inv, &wrong, inv.inviter, raw.me(), &add));
    assert_eq!(answer["refused"], "wrong-code", "{answer}");
    assert!(answer.get("proof").is_none(), "no inviter proof to a wrong code: {answer}");

    // A proof the joiner made for the endpoint it dialled (a fake inviter's, from a swapped
    // invite), sent on by that fake to the real inviter: the right code, and still refused.
    let fake = PeerId([9; 32]);
    let caught = record(&inv, &code, fake, b_endpoint(&b), &json!({ "key": b.key(), "endpoint": hex(b_endpoint(&b).0) }).to_string());
    let answer = raw.ask(&a, &inv, &caught);
    assert_eq!(answer["refused"], "wrong-code", "a proof caught by a fake inviter fails at the real one: {answer}");
    assert!(answer.get("proof").is_none());

    // The member joins with the right code: it follows the space, and the page comes.
    let out = b.engine().join(v["invite"].as_str().unwrap(), &code, &Choice { at: a.at(), ..Choice::default() }).unwrap();
    assert_eq!(out["followed"][0]["address"], "project:atlas", "{out}");
    let (_, space) = chains::of_address(&a.paths, &SpaceId::parse("project:atlas").unwrap()).unwrap();
    assert!(chains::members(&space).iter().any(|m| m.key == b.key() && m.endpoint == hex(b_endpoint(&b).0) && m.level == chains::Level::Append));
    let copy = b.root.join("spaces/project/atlas/pages/db.md");
    until("the page reached the member", 20, || copy.exists());

    // A second join with the same invite: with no invite open, join/1 isn't even offered.
    assert!(raw.try_ask(&a, &inv, &record(&inv, &code, inv.inviter, raw.me(), &add)).unwrap_err().contains("protocol"));

    // An invite past its time is refused, the right code or not; another invite keeps join/1
    // open, so the refusal is the door's own.
    let short = a.invite(1000);
    let _open = a.invite(join::TICKET_MS);
    let inv2 = Invite::parse_for(&TARGET, short["invite"].as_str().unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let late = raw.ask(&a, &inv2, &record(&inv2, short["code"].as_str().unwrap(), inv2.inviter, raw.me(), &add));
    assert_eq!(late["refused"], "no-ticket", "{late}");
    let again = raw.ask(&a, &inv, &record(&inv, &code, inv.inviter, raw.me(), &add));
    assert_eq!(again["refused"], "no-ticket", "the used invite: {again}");
}

fn b_endpoint(x: &X) -> PeerId {
    let k = xenonet_net::keys::HostKey::from_secrets(&x.keys.transport.secret(), [0; 32]).unwrap();
    PeerId(k.public().sign)
}
