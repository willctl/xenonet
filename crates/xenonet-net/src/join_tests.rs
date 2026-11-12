// join/2: the invite, the proofs, the inviter's door on the simulated
// network (seeded, no iroh hooks) and on real iroh over loopback, and the
// joiner's check of the chain from its genesis.

use super::*;
use crate::admission::Admission;
use crate::dial::{Close, Dial, Side};
use crate::endpoint::{IrohDial, LanLookup, Relays, Settings};
use crate::keys::HostKey;
use crate::roster::{found, HostRef, ADMIT};
use crate::sim::SimDial;
use std::sync::atomic::{AtomicUsize, Ordering};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer};
use xenonet_log::person::{self, DeviceKind, NewDevice, PersonChain};

const A: PeerId = PeerId([0xa1; 32]);
const B: PeerId = PeerId([0xb2; 32]);
const NET: &str = "net_00112233445566778899aabbccddeeff";
const CODE: &str = "KX7Q49MZT2";
const NOW: i64 = 1_792_512_000_000;

fn invite(ticket: TicketId) -> Invite {
    Invite { network: NET.into(), inviter: A, relay: None, ticket }
}

#[test]
fn an_invite_carries_the_network_the_inviter_one_relay_and_a_ticket_id_only() {
    let mut inv = Invite { network: NET.into(), inviter: A, relay: None, ticket: TicketId([7; 16]) };
    let text = inv.encode();
    assert!(text.starts_with("xni2_"), "{text}");
    assert_eq!(text.len(), 5 + 86, "64 bytes in base64url: no address in it");
    assert_eq!(Invite::parse(&format!("  {text}\n")).unwrap(), inv);
    inv.relay = Some("https://relay.example.net/".into());
    let text = inv.encode();
    assert_eq!(Invite::parse(&text).unwrap(), inv);
    for bad in ["", "xni2_", "xni1_AAAA", &text[..40], &format!("{}AAAA", &text[..5]), "KX7Q-49MZ-T2"] {
        assert!(Invite::parse(bad).is_err(), "{bad:?}");
    }
    // A relay that isn't a relay's URL.
    let mut bytes = unb64(&text[5..]).unwrap();
    bytes.truncate(64);
    bytes.extend_from_slice(b"ftp://x@y");
    assert!(Invite::parse(&format!("xni2_{}", b64(&bytes))).is_err());
    assert_ne!(TicketId::new().unwrap(), TicketId::new().unwrap());
}

#[test]
fn both_proofs_cover_the_ticket_the_network_both_endpoints_and_the_add() {
    let hash = ticket::hash(CODE);
    let (t, other) = (TicketId([1; 16]), TicketId([2; 16]));
    let (add, add2) = (sha256(b"add"), sha256(b"another add"));
    let b = Bound { ticket: &t, network: NET, inviter: &A, joiner: &B, add: &add };
    let p = joiner_proof(&hash, &b);
    assert!(check_joiner(&hash, &b, &p));
    assert!(!check_joiner(&ticket::hash("KX7Q49MZT3"), &b, &p), "another code");
    let changed =
        [Bound { ticket: &other, ..b }, Bound { network: "net_ffeeddccbbaa99887766554433221100", ..b }, Bound { inviter: &B, joiner: &A, ..b }, Bound { add: &add2, ..b }];
    for c in &changed {
        assert!(!check_joiner(&hash, c, &p));
    }
    let i = inviter_proof(&hash, &b);
    assert!(check_inviter(&hash, &b, &i));
    assert!(!check_inviter(&hash, &b, &p), "the joiner's proof isn't the inviter's");
    assert!(!check_joiner(&hash, &b, &i));
}

// ---------- another program's join ----------

const JOIN1: Alpn = Alpn::App(b"xenolith/join/1");

// A join to the inviter itself, which names no target: Xenolith's join/1.
const OTHER: JoinTarget = JoinTarget {
    alpn: JOIN1,
    prefix: "xlj1_",
    label: "xenolith-join/1",
    schema: "xenolith/join/v1",
    how: "xenolith invite",
    target_len: 0,
    target_bytes: |id| id.is_empty().then(Vec::new),
    target_of: |_| Some(String::new()),
    relay_ok: crate::roster::is_relay_url,
};

#[test]
fn another_targets_invites_and_proofs_are_its_own_and_never_a_networks() {
    let inv = Invite { network: String::new(), inviter: A, relay: Some("https://relay.example.net/".into()), ticket: TicketId([7; 16]) };
    let text = inv.encode_for(&OTHER).unwrap();
    assert!(text.starts_with("xlj1_"), "{text}");
    assert_eq!(Invite::parse_for(&OTHER, &text).unwrap(), inv);
    assert!(Invite::parse(&text).is_err(), "not a network's invite");
    let e = Invite::parse_for(&OTHER, &invite(TicketId([7; 16])).encode()).unwrap_err();
    assert!(e.message.contains("xenolith invite"), "{}", e.message);
    assert_eq!(Invite { network: NET.into(), ..inv.clone() }.encode_for(&OTHER), None, "a target it can't carry");

    let hash = ticket::hash(CODE);
    let (t, add) = (TicketId([1; 16]), sha256(b"add"));
    let b = Bound { ticket: &t, network: "", inviter: &A, joiner: &B, add: &add };
    let p = OTHER.joiner_proof(&hash, &b);
    assert!(OTHER.check_joiner(&hash, &b, &p));
    assert!(!NETWORK.check_joiner(&hash, &b, &p), "a proof for one target is none for another");
    assert!(!OTHER.check_joiner(&hash, &b, &NETWORK.joiner_proof(&hash, &b)));
    assert!(!OTHER.check_inviter(&hash, &b, &p));
}

#[test]
fn a_machine_joins_another_programs_target_on_its_own_door_and_xenonets_stays_shut() {
    let mut sim = sim(7);
    let slot = Arc::new(Mutex::new(None));
    let served = Arc::new(Mutex::new(Vec::new()));
    let admit = Arc::new(Fixed { answer: json!({ "share": "s" }), calls: AtomicUsize::new(0) });
    let doors = || crate::admission::Doors { members: Vec::new(), join: Some(JOIN1) };
    let (at_a, done) = (slot.clone(), served.clone());
    let fixed = admit.clone();
    sim.host("a", move || {
        let (slot, served, admit) = (at_a.clone(), done.clone(), fixed.clone());
        async move {
            let mesh = Arc::new(Mesh::new(Arc::new(SimDial::bind(A, hosts()).await?), Arc::new(Admission::with(doors()))));
            let door = Door::to(OTHER, mesh.clone());
            let id = door.open("", "laptop", false, CODE, NOW + 600_000)?;
            assert_eq!(mesh.admission().offered(), [JOIN1]);
            *slot.locked() = Some(Invite { network: String::new(), inviter: A, relay: None, ticket: id });
            while let Some(link) = mesh.accept().await {
                assert_eq!(link.alpn(), JOIN1);
                let s = door.serve(&link, admit.as_ref(), NOW).await?;
                served.locked().push(s);
            }
            Ok(())
        }
    });
    let at_b = slot.clone();
    sim.client("b", async move {
        let inv = wait_for(&at_b).await;
        let mesh = Arc::new(Mesh::new(Arc::new(SimDial::bind(B, hosts()).await?), Arc::new(Admission::with(doors()))));
        // XenoNet's join/2 isn't a door of this mesh, nor of the inviter's.
        assert_eq!(ask(&mesh, &inv, CODE, ADD).await.unwrap_err().code, "no-door");
        let answer = ask_to(&OTHER, &mesh, &inv, CODE, ADD).await?;
        assert_eq!(answer, Answer::Joined(json!({ "share": "s" })));
        Ok(())
    });
    sim.run().unwrap();
    assert_eq!(*served.locked(), [Served::Joined { host: "laptop".into() }]);
}

// ---------- the door on the simulated network ----------

fn hosts() -> BTreeMap<PeerId, String> {
    [(A, "a"), (B, "b")].into_iter().map(|(p, h)| (p, h.to_string())).collect()
}

fn sim(seed: u64) -> turmoil::Sim<'static> {
    turmoil::Builder::new()
        .rng_seed(seed)
        .simulation_duration(Duration::from_secs(60))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(Duration::from_millis(40))
        .enable_random_order()
        .build()
}

async fn sim_mesh(me: PeerId) -> R<Arc<Mesh>> {
    Ok(Arc::new(Mesh::new(Arc::new(SimDial::bind(me, hosts()).await?), Arc::new(Admission::new()))))
}

/// Signs anyone in with a fixed answer, and counts the calls.
struct Fixed {
    answer: Value,
    calls: AtomicUsize,
}

impl Admit for Fixed {
    fn admit<'a>(&'a self, ticket: &'a Ticket, joiner: PeerId, add: &'a Value) -> Fut<'a, R<Admitted>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!((ticket.host.as_str(), joiner), ("laptop", B));
            assert_eq!(add["machine"], "m");
            Ok(Admitted::Joined(self.answer.clone()))
        })
    }
}

// The inviter: a ticket for laptop on its door, the invite out through
// `slot`, then every join/2 attempt served until the door closes.
fn inviter(sim: &mut turmoil::Sim<'static>, slot: Arc<Mutex<Option<Invite>>>, served: Arc<Mutex<Vec<Served>>>, admit: Arc<Fixed>) {
    sim.host("a", move || {
        let (slot, served, admit) = (slot.clone(), served.clone(), admit.clone());
        async move {
            let mesh = sim_mesh(A).await?;
            let door = Door::new(mesh.clone());
            assert!(!mesh.admission().tickets_open());
            let id = door.open(NET, "laptop", false, CODE, NOW + 600_000)?;
            assert!(mesh.admission().tickets_open(), "join/2 is offered while the ticket is open");
            *slot.locked() = Some(invite(id));
            while let Some(link) = mesh.accept().await {
                assert_eq!(link.alpn(), Alpn::Join);
                let s = door.serve(&link, admit.as_ref(), NOW).await?;
                served.locked().push(s);
                if door.is_empty() {
                    // The last ticket closed: join/2 is no longer offered.
                    assert!(!mesh.admission().tickets_open());
                }
            }
            Ok(())
        }
    });
}

async fn wait_for(slot: &Mutex<Option<Invite>>) -> Invite {
    loop {
        if let Some(i) = slot.locked().clone() {
            return i;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

const ADD: &str = r#"{"machine":"m","sign":"s","seal":"x"}"#;

#[test]
fn a_machine_joins_over_the_simulated_network() {
    for seed in [1, 0x5eed, 99] {
        let mut sim = sim(seed);
        let slot = Arc::new(Mutex::new(None));
        let served = Arc::new(Mutex::new(Vec::new()));
        let admit = Arc::new(Fixed { answer: json!({ "roster": [], "host": "laptop" }), calls: AtomicUsize::new(0) });
        inviter(&mut sim, slot.clone(), served.clone(), admit.clone());
        let at_b = slot.clone();
        sim.client("b", async move {
            let inv = wait_for(&at_b).await;
            let mesh = sim_mesh(B).await?;
            let answer = ask(&mesh, &inv, "kx7q-49mz-t2", ADD).await?;
            assert_eq!(answer, Answer::Joined(json!({ "roster": [], "host": "laptop" })));
            assert!(mesh.admission().check(&A, Alpn::Join, Side::Dialled).is_err(), "the invite's dial ended with the join");
            // The ticket is used: a second try finds no door.
            let again = ask(&mesh, &inv, CODE, ADD).await.unwrap_err();
            assert_eq!(again.code, "unreachable", "seed {seed}: {again:?}");
            Ok(())
        });
        sim.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(*served.locked(), [Served::Joined { host: "laptop".into() }], "seed {seed}");
        assert_eq!(admit.calls.load(Ordering::SeqCst), 1);
    }
}

// One attempt by hand: what the inviter sends back to `ask`.
async fn attempt(mesh: &Mesh, ask: &Value) -> R<Value> {
    mesh.begin_join(A);
    let link = mesh.dial(A, Alpn::Join).await?;
    let (mut w, mut r) = link.open_bi().await?;
    write(&mut w, ask).await?;
    let _ = w.shutdown().await;
    let v = serde_json::from_slice(&read(&mut r, ANSWER_MAX).await?).unwrap();
    link.close(Close::Done);
    mesh.end_join(&A);
    Ok(v)
}

#[test]
fn wrong_codes_get_no_proof_and_five_retire_the_ticket() {
    let mut sim = sim(3);
    let slot = Arc::new(Mutex::new(None));
    let served = Arc::new(Mutex::new(Vec::new()));
    let admit = Arc::new(Fixed { answer: json!({}), calls: AtomicUsize::new(0) });
    inviter(&mut sim, slot.clone(), served.clone(), admit.clone());
    let at_b = slot.clone();
    sim.client("b", async move {
        let inv = wait_for(&at_b).await;
        let mesh = sim_mesh(B).await?;
        // With no ticket id, and with one that isn't open: refused, and
        // nothing counted against the ticket.
        for ticket in [None, Some(TicketId([9; 16]).to_string())] {
            let mut a = json!({ "schema": NETWORK.schema, "network": NET, "add": ADD, "proof": "x" });
            if let Some(t) = ticket {
                a["ticket"] = json!(t);
            }
            let v = attempt(&mesh, &a).await?;
            assert_eq!(v["refused"], "no-ticket", "{v}");
            assert!(v.get("proof").is_none(), "{v}");
        }
        for left in (0..MAX_WRONG).rev() {
            let e = ask(&mesh, &inv, "KX7Q49MZT3", ADD).await.unwrap_err();
            if left == 0 {
                assert_eq!((e.status, e.code), (410, "retired"), "{e:?}");
            } else {
                assert_eq!((e.status, e.code), (403, "wrong-code"), "{e:?}");
                assert!(e.message.contains(&format!("{left} tries left")), "{}", e.message);
            }
        }
        // Retired: join/2 is withdrawn, and the right code comes too late.
        let e = ask(&mesh, &inv, CODE, ADD).await.unwrap_err();
        assert_eq!(e.code, "unreachable", "retired: join/2 is withdrawn, the right code too late: {e:?}");
        Ok(())
    });
    sim.run().unwrap();
    let s = served.locked().clone();
    assert_eq!(s.len(), 2 + MAX_WRONG as usize, "{s:?}");
    assert_eq!(&s[..2], [Served::Refused("no-ticket"), Served::Refused("no-ticket")]);
    assert_eq!(s.last(), Some(&Served::Refused("retired")));
    assert_eq!(admit.calls.load(Ordering::SeqCst), 0, "never signed in");
}

#[test]
fn a_wrong_codes_answer_carries_no_proof() {
    let mut sim = sim(4);
    let slot = Arc::new(Mutex::new(None));
    let served = Arc::new(Mutex::new(Vec::new()));
    let admit = Arc::new(Fixed { answer: json!({ "roster": [] }), calls: AtomicUsize::new(0) });
    inviter(&mut sim, slot.clone(), served.clone(), admit.clone());
    let at_b = slot.clone();
    sim.client("b", async move {
        let inv = wait_for(&at_b).await;
        let mesh = sim_mesh(B).await?;
        let add_hash = sha256(ADD.as_bytes());
        let bound = Bound { ticket: &inv.ticket, network: NET, inviter: &A, joiner: &B, add: &add_hash };
        let wrong = joiner_proof(&ticket::hash("KX7Q49MZT3"), &bound);
        let v = attempt(&mesh, &json!({ "schema": NETWORK.schema, "ticket": inv.ticket.to_string(), "network": NET, "add": ADD, "proof": wrong })).await?;
        assert_eq!((v["refused"].clone(), v["left"].clone()), (json!("wrong-code"), json!(4)), "{v}");
        assert!(v.get("proof").is_none() && v.get("answer").is_none(), "nothing of the inviter's: {v}");
        // The right code, on the same ticket: the proof comes, and checks.
        let right = joiner_proof(&ticket::hash(CODE), &bound);
        let v = attempt(&mesh, &json!({ "schema": NETWORK.schema, "ticket": inv.ticket.to_string(), "network": NET, "add": ADD, "proof": right })).await?;
        assert!(check_inviter(&ticket::hash(CODE), &bound, v["proof"].as_str().unwrap()), "{v}");
        // The proof binds the add: the same code for another add is wrong.
        Ok(())
    });
    sim.run().unwrap();
    assert_eq!(admit.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn join2_is_refused_with_no_ticket_open() {
    let mut sim = sim(5);
    let refused = Arc::new(AtomicUsize::new(0));
    let count = refused.clone();
    sim.host("a", move || {
        let count = count.clone();
        async move {
            // a holds no ticket: it neither offers join/2 nor hands a
            // join connection on.
            let mesh = sim_mesh(A).await?;
            assert!(!mesh.admission().offered().contains(&Alpn::Join));
            while mesh.accept().await.is_some() {
                count.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    });
    sim.client("b", async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mesh = sim_mesh(B).await?;
        let e = ask(&mesh, &invite(TicketId([1; 16])), CODE, ADD).await.unwrap_err();
        assert_eq!(e.code, "unreachable", "{e:?}");
        // Without an invite naming a, b doesn't even dial it.
        let e = mesh.dial(A, Alpn::Join).await.unwrap_err();
        assert_eq!(e.code, "not-invited");
        Ok(())
    });
    sim.run().unwrap();
    assert_eq!(refused.load(Ordering::SeqCst), 0);
    // And on the accept path itself, whatever the transport offers.
    let a = Admission::new();
    assert_eq!(a.check(&B, Alpn::Join, Side::Accepted), Err(crate::admission::Refused::NoTicket));
}

#[test]
fn tickets_expire_close_and_say_when_one_burned() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let key = HostKey::generate().unwrap();
        let admission = Arc::new(Admission::new());
        let dial = Arc::new(IrohDial::bind(&key, &loopback(), admission.clone()).await.unwrap());
        let mesh = Arc::new(Mesh::new(dial.clone(), admission.clone()));
        let door = Door::new(mesh.clone());
        let t1 = door.open(NET, "laptop", false, CODE, NOW + 10).unwrap();
        let _t2 = door.open(NET, "desk", true, CODE, NOW + 20).unwrap();
        // A new ticket for laptop replaces the old.
        let t3 = door.open(NET, "laptop", false, CODE, NOW + 30).unwrap();
        assert!(door.get(&t1, NOW).is_none());
        assert_eq!(door.open_tickets().len(), 2);
        assert_eq!(door.next_expiry(), Some(NOW + 20));
        assert!(admission.tickets_open());
        assert_eq!(door.expire(NOW + 20).len(), 1);
        assert_eq!(door.burned(), [Burned { host: "desk".into(), at: NOW + 20, retired: false }]);
        assert!(door.get(&t3, NOW + 29).is_some());
        assert!(door.get(&t3, NOW + 30).is_none(), "past its time, even before expire runs");
        for _ in 0..MAX_WRONG {
            door.wrong(&t3, NOW + 25);
        }
        assert_eq!(door.burned().last(), Some(&Burned { host: "laptop".into(), at: NOW + 25, retired: true }));
        assert!(door.is_empty() && !admission.tickets_open(), "join/2 withdrawn with the last ticket");
        dial.close().await;
    });
}

// ---------- real iroh on loopback ----------

fn loopback() -> Settings {
    Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some("127.0.0.1:0".parse().unwrap()), roots: Vec::new() }
}

#[test]
fn a_machine_joins_over_iroh_on_loopback() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let (ka, kb) = (HostKey::generate().unwrap(), HostKey::generate().unwrap());
        let a_adm = Arc::new(Admission::new());
        let a = Arc::new(IrohDial::bind(&ka, &loopback(), a_adm.clone()).await.unwrap());
        let a_mesh = Arc::new(Mesh::new(a.clone(), a_adm));
        let b_adm = Arc::new(Admission::new());
        let b = Arc::new(IrohDial::bind(&kb, &loopback(), b_adm.clone()).await.unwrap());
        let b_mesh = Mesh::new(b.clone(), b_adm);
        b.addresses().put(a.me(), &a.direct_addrs(), None, Duration::from_secs(60)).unwrap();
        let inv = Invite { network: NET.into(), inviter: a.me(), relay: None, ticket: TicketId([0; 16]) };
        // No ticket open: b's dial is refused (the ALPN isn't offered, and
        // a's door and hooks would refuse it if it were).
        assert!(ask(&b_mesh, &inv, CODE, ADD).await.is_err());

        let door = Arc::new(Door::new(a_mesh.clone()));
        let id = door.open(NET, "laptop", false, CODE, i64::MAX).unwrap();
        let inv = Invite { ticket: id, ..inv };
        // b's endpoint id, as QUIC authenticated it, is what the door hands
        // the node to sign in.
        struct Me(PeerId);
        impl Admit for Me {
            fn admit<'a>(&'a self, t: &'a Ticket, joiner: PeerId, add: &'a Value) -> Fut<'a, R<Admitted>> {
                Box::pin(async move {
                    assert_eq!((t.host.as_str(), joiner, &add["machine"]), ("laptop", self.0, &json!("m")));
                    Ok(Admitted::Joined(json!({ "host": "laptop" })))
                })
            }
        }
        let me = Arc::new(Me(b.me()));
        let serve_one = || {
            let (door, me) = (door.clone(), me.clone());
            tokio::spawn(async move {
                let link = door.mesh().accept().await.unwrap();
                door.serve(&link, me.as_ref(), 0).await
            })
        };
        let serving = serve_one();
        let e = ask(&b_mesh, &inv, "KX7Q49MZT3", ADD).await.unwrap_err();
        assert_eq!(e.code, "wrong-code");
        let served = tokio::time::timeout(Duration::from_secs(10), serving).await.unwrap().unwrap().unwrap();
        assert_eq!(served, Served::Refused("wrong-code"));
        let serving = serve_one();
        let answer = tokio::time::timeout(Duration::from_secs(10), ask(&b_mesh, &inv, CODE, ADD)).await.unwrap().unwrap();
        assert_eq!(answer, Answer::Joined(json!({ "host": "laptop" })));
        assert_eq!(serving.await.unwrap().unwrap(), Served::Joined { host: "laptop".into() });
        assert!(door.is_empty());
        b_mesh.close().await;
        a_mesh.close().await;
    });
}

// ---------- the chain, from its genesis ----------

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn signer(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&secret(n), KeyHome::File).unwrap()
}

fn block(device: &SeedSigner, what: &str, act: [u8; 32], at: i64) -> PresenceBlock {
    let mut b = PresenceBlock { device: device.public().id(), what: what.into(), act, nonce: [0x22; 16], at, sig: Vec::new() };
    b.sig = device.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
    b
}

struct Chain {
    roster: Roster,
    owner: PersonChain,
    studio: HostKey,
    laptop: HostKey,
}

// studio founds NET for its owner and adds laptop, with the owner's
// presence at studio's device.
fn chain(network: &str) -> Chain {
    let (root, device) = (signer(5), signer(6));
    let mut owner = PersonChain::new();
    owner.offer(person::found_salted(&root, [0x5a; person::SALT_BYTES], NOW - 2000).unwrap()).unwrap();
    let mut pair = owner.next(person::Body { op: person::Op::AddDevice(NewDevice { key: device.public(), kind: DeviceKind::Weaker, label: "studio".into() }), at: NOW - 1000 });
    let b = block(&device, person::PAIR, pair.digest(), NOW - 1000);
    pair.sign(&root.public().id(), &root, Some(b)).unwrap();
    owner.offer(pair).unwrap();
    let (studio, laptop) = (HostKey::from_secrets(&secret(1), secret(2)).unwrap(), HostKey::from_secrets(&secret(3), secret(4)).unwrap());
    let mut r = Roster::with_policy(NetworkPolicy::with(Arc::new(owner.clone())));
    let me = HostRef { id: "studio".into(), keys: *studio.public(), window: true, device: Some(device.public().id()), machine: signer(7).public() };
    r.offer(found(network, me, &signer(7), owner.id().map(str::to_string), owner.head(), None, NOW).unwrap()).unwrap();
    let mut add = r.draft(Op::Add, HostRef { id: "laptop".into(), keys: *laptop.public(), window: false, device: None, machine: signer(8).public() }, None, NOW + 1);
    add.person_head = owner.head();
    let admit = block(&device, ADMIT, add.digest(), NOW + 1);
    add.sign_with("studio", &signer(7), Some(admit)).unwrap();
    r.offer(add).unwrap();
    Chain { roster: r, owner, studio, laptop }
}

#[test]
fn the_joiner_checks_the_chain_from_its_genesis() {
    let c = chain(NET);
    let policy = || NetworkPolicy::with(Arc::new(c.owner.clone()));
    let inv = Invite { network: NET.into(), inviter: PeerId(c.studio.public().sign), relay: None, ticket: TicketId([1; 16]) };
    let json = c.roster.to_json();
    let r = verify(policy(), &json, &inv, "laptop", c.laptop.public(), &signer(8).public()).unwrap();
    assert_eq!(r.len(), 2);

    let refused = |chain: &Value, inv: &Invite, host: &str, keys: &PublicKeys, machine: &VerifyingKey| {
        let e = verify(policy(), chain, inv, host, keys, machine).unwrap_err();
        assert_eq!(e.code, "bad-answer");
        e.message
    };
    // The genesis gone: the add alone doesn't check.
    let tail = json!([json[1].clone()]);
    assert!(refused(&tail, &inv, "laptop", c.laptop.public(), &signer(8).public()).contains("didn't check"));
    // The genesis changed: its signature, and every link after it, fail.
    let mut forged = json.clone();
    forged[0]["at"] = json!(NOW + 5);
    assert!(refused(&forged, &inv, "laptop", c.laptop.public(), &signer(8).public()).contains("didn't check"));
    // Another network's chain, whole and signed, for this invite.
    let other = chain("net_ffeeddccbbaa99887766554433221100");
    assert!(refused(&other.roster.to_json(), &inv, "laptop", c.laptop.public(), &signer(8).public()).contains("founding of the network the invite names"));
    // An inviter the chain doesn't hold.
    let stranger = Invite { inviter: PeerId([0xee; 32]), ..inv.clone() };
    assert!(refused(&json, &stranger, "laptop", c.laptop.public(), &signer(8).public()).contains("inviter"));
    // This machine added with other keys than it asked for, or not at all.
    assert!(refused(&json, &inv, "laptop", c.studio.public(), &signer(8).public()).contains("doesn't add this machine"));
    assert!(refused(&json, &inv, "laptop", c.laptop.public(), &signer(9).public()).contains("doesn't add this machine"));
    assert!(refused(&json, &inv, "desk", c.laptop.public(), &signer(8).public()).contains("doesn't add this machine"));
}
