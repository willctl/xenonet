// forward(alpn): what an attach is refused for, and a forwarded protocol
// between two machines on the simulated network, each XenoNet carrying it
// for its actor: members only, one actor per ALPN, and a revoked grant
// closing what it forwards.

use super::*;
use crate::admission::{Admission, Doors};
use crate::sim::SimDial;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SYNC: Alpn = Alpn::App(b"xenolith/sync/2");
const N1: &str = "net_00000000000000000000000000000001";
const A: PeerId = PeerId([0xa1; 32]);
const B: PeerId = PeerId([0xb2; 32]);
const C: PeerId = PeerId([0xc3; 32]);

/// Grants by their text: whose, which ALPN they forward, and which are
/// revoked.
#[derive(Default)]
struct Fake {
    revoked: Mutex<BTreeSet<String>>,
    /// The context keys the person approved a grant for.
    approved: Mutex<BTreeSet<String>>,
}

impl Grants for Fake {
    fn decide(&self, token: &str, alpn: &str) -> R<Grant> {
        let (actor, forwards) = match token {
            "tok-a" => ("lith-a", "xenolith/sync/2"),
            "tok-a2" => ("lith-a2", "xenolith/sync/2"),
            "tok-b" | "tok-b-n2" => ("lith-b", "xenolith/sync/2"),
            "tok-other" => ("lith-a", "other/alpn/1"),
            _ => return Err(refused("no-grant", "no such grant")),
        };
        if self.revoked.locked().contains(token) {
            return Err(refused("revoked", "that grant was revoked"));
        }
        if forwards != alpn {
            return Err(refused("other-alpn", "that grant forwards another ALPN"));
        }
        let networks = (token == "tok-b-n2").then(|| [N2.to_string()].into());
        Ok(Grant { actor: actor.into(), ids: vec![format!("id-{token}")], networks, until: i64::MAX })
    }

    fn collect(&self, alpn: &str, key: &str) -> R<String> {
        if alpn != "xenolith/sync/2" {
            return Err(refused("no-grant", "not that ALPN"));
        }
        if self.approved.locked().contains(key) {
            Ok(format!("tok-for-{key}"))
        } else {
            Err(refused("unapproved", "the person hasn't approved one"))
        }
    }
}

const N2: &str = "net_00000000000000000000000000000002";

#[test]
fn a_collect_needs_the_keys_proof_waits_for_the_person_and_takes_their_answer() {
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let grants = Arc::new(Fake::default());
        let f = forwards(grants.clone());
        let connect = duplex(f.clone());
        let lith = SeedSigner::from_seed(&[7; 32], KeyHome::File).unwrap();
        let other = SeedSigner::from_seed(&[8; 32], KeyHome::File).unwrap();
        let key = lith.public();
        let id = key.id();
        // Signed by another key than the one named: refused before any ask.
        let forged = ForwardDial::collect(&connect, SYNC, &key, |m| other.sign(xenonet_id::sign::Purpose::FORWARD_COLLECT, m).ok(), || {}).await;
        assert_eq!(forged.err().map(|e| e.code), Some("not-the-key"));
        // The right key's signature, but for another purpose: refused too.
        let wrong = ForwardDial::collect(&connect, SYNC, &key, |m| lith.sign(xenonet_id::sign::Purpose::SYNC_HELLO, m).ok(), || {}).await;
        assert_eq!(wrong.err().map(|e| e.code), Some("not-the-key"));
        assert!(f.asks().is_empty(), "no ask without the key's proof");
        let not = ForwardDial::collect(&connect, Alpn::Link, &key, |m| lith.sign(xenonet_id::sign::Purpose::FORWARD_COLLECT, m).ok(), || {}).await;
        assert_eq!(not.err().map(|e| e.code), Some("not-forwardable"));

        // Unapproved: it waits, the ask shows, and the person's approval
        // sends the grant down the same connection.
        let waited = Arc::new(AtomicU64::new(0));
        let w = waited.clone();
        let (g, ff) = (grants.clone(), f.clone());
        let k = id.clone();
        let approve = async move {
            until("the ask shows", || ff.asks().iter().any(|a| a.key == k && a.alpn == "xenolith/sync/2")).await.unwrap();
            g.approved.locked().insert(k.clone());
            assert!(ff.answered("xenolith/sync/2", &k));
        };
        let collect = ForwardDial::collect(
            &connect,
            SYNC,
            &key,
            |m| lith.sign(xenonet_id::sign::Purpose::FORWARD_COLLECT, m).ok(),
            move || {
                w.fetch_add(1, Ordering::SeqCst);
            },
        );
        let (got, ()) = tokio::join!(collect, approve);
        assert_eq!(got.unwrap(), format!("tok-for-{id}"));
        assert_eq!(waited.load(Ordering::SeqCst), 1);
        assert!(f.asks().is_empty(), "answered, the ask goes");
        // Approved, the next collect gets one at once.
        assert_eq!(ForwardDial::collect(&connect, SYNC, &key, |m| lith.sign(xenonet_id::sign::Purpose::FORWARD_COLLECT, m).ok(), || {}).await.unwrap(), format!("tok-for-{id}"));

        // Turned down: the waiting collect is refused.
        let okey = other.public();
        let oid = okey.id();
        let ff = f.clone();
        let o = oid.clone();
        let deny = async move {
            until("the second ask shows", || ff.asks().iter().any(|a| a.key == o)).await.unwrap();
            ff.answered("xenolith/sync/2", &o);
        };
        let collect = ForwardDial::collect(&connect, SYNC, &okey, |m| other.sign(xenonet_id::sign::Purpose::FORWARD_COLLECT, m).ok(), || {});
        let (got, ()) = tokio::join!(collect, deny);
        assert_eq!(got.err().map(|e| e.code), Some("denied"));
    });
}

#[test]
fn a_grant_reaches_the_networks_it_names_alone() {
    let g = Grant { actor: "lith".into(), ids: vec![], networks: Some([N2.to_string()].into()), until: 0 };
    let both: BTreeSet<String> = [N1.to_string(), N2.to_string()].into();
    assert_eq!(g.reached(&both), [N2.to_string()].into());
    assert!(g.reached(&[N1.to_string()].into()).is_empty());
    let any = Grant { networks: None, ..g };
    assert_eq!(any.reached(&both), both);
}

#[test]
fn a_member_of_a_network_the_grant_doesnt_name_is_refused_its_connection() {
    let mut s = sim(3);
    let seen = Arc::new(AtomicU64::new(0));
    let sb = seen.clone();
    s.host("b", move || {
        let sb = sb.clone();
        async move {
            // B's actor's grant reaches N2 alone, and A is B's peer in N1.
            let (_, f) = xenonet(B, &[A, B], Arc::default()).await?;
            let dial = ForwardDial::attach(duplex(f), "tok-b-n2", SYNC).await?;
            while dial.incoming().await.is_some() {
                sb.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    });
    s.client("a", async move {
        let (mesh, _) = xenonet(A, &[A, B], Arc::default()).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let link = loop {
            match mesh.dial(B, SYNC).await {
                Ok(l) => break l,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        assert_eq!(link.closed().await, Some(Close::Refused));
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 0, "B's actor never heard of A's connection");
}

fn forwards(grants: Arc<Fake>) -> Arc<Forwards> {
    Forwards::new(Forwardable::new(&[b"xenolith/sync/2"]).unwrap(), grants)
}

#[test]
fn only_registered_alpns_are_forwardable_and_never_xenonets_own() {
    for own in [&b"xenonet/link/2"[..], b"xenonet/join/2", b"xenonet/svc/2", b"Not An ALPN"] {
        assert!(Forwardable::new(&[own]).is_err(), "{}", String::from_utf8_lossy(own));
    }
    let f = Forwardable::new(&[b"xenolith/sync/2", b"xenolith/sync/2"]).unwrap();
    assert_eq!(f.alpns(), [SYNC]);
    assert_eq!(f.get("xenolith/sync/2"), Some(SYNC));
    assert_eq!(f.get("xenonet/link/2"), None);
    let d = Doors::xenonet_forwarding(f.alpns());
    assert_eq!(d.members, [Alpn::Link, Alpn::Svc, SYNC]);
}

#[test]
fn an_attach_without_a_grant_or_for_another_alpn_is_refused() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let f = forwards(Arc::default());
        let at = |token: &'static str, alpn: Alpn| {
            let f = f.clone();
            async move { ForwardDial::attach(duplex(f), token, alpn).await.err().map(|e| e.code) }
        };
        assert_eq!(at("tok-a", Alpn::Link).await, Some("not-forwardable"), "XenoNet's own protocols are never forwarded");
        assert_eq!(at("tok-a", Alpn::App(b"other/alpn/1")).await, Some("not-forwardable"), "nor one the root didn't register");
        assert_eq!(at("nothing", SYNC).await, Some("no-grant"));
        assert_eq!(at("tok-other", SYNC).await, Some("other-alpn"), "a grant forwards the one ALPN it names");
        assert_eq!(at("tok-a", SYNC).await, Some("no-network"), "and nothing goes before the network's endpoint runs");
    });
}

fn hosts() -> BTreeMap<PeerId, String> {
    [(A, "a"), (B, "b"), (C, "c")].into_iter().map(|(p, h)| (p, h.to_string())).collect()
}

fn sim(seed: u64) -> turmoil::Sim<'static> {
    turmoil::Builder::new()
        .rng_seed(seed)
        .simulation_duration(Duration::from_secs(120))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(Duration::from_millis(20))
        .build()
}

// A machine's XenoNet: its mesh, admitting `members` of N1 on link/2 and
// the forwarded ALPN, and its forwards taking what comes in on it.
async fn xenonet(me: PeerId, members: &[PeerId], grants: Arc<Fake>) -> R<(Arc<Mesh>, Arc<Forwards>)> {
    let f = forwards(grants);
    let mesh = Arc::new(Mesh::new(Arc::new(SimDial::bind(me, hosts()).await?), Arc::new(Admission::with(Doors::xenonet_forwarding(f.forwardable().alpns())))));
    mesh.set_members(N1, members.iter().copied());
    f.set_mesh(Some(mesh.clone()));
    let (m, fw) = (mesh.clone(), f.clone());
    tokio::spawn(async move {
        while let Some(l) = m.accept().await {
            if l.alpn().is_xenonet() {
                l.close(Close::Done);
            } else {
                fw.incoming(l);
            }
        }
    });
    Ok((mesh, f))
}

async fn until(what: &str, ok: impl Fn() -> bool) -> R<()> {
    for _ in 0..5_000 {
        if ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(NetError::internal(format!("never: {what}")))
}

#[test]
fn a_forwarded_protocol_reaches_members_only_one_actor_holds_an_alpn_and_revoking_closes_it() {
    let mut s = sim(1);
    let seen = Arc::new(AtomicU64::new(0));
    let b_forwards: Arc<Mutex<Option<Arc<Forwards>>>> = Arc::default();
    let (seen_b, at_b) = (seen.clone(), b_forwards.clone());
    // B: its XenoNet, and an actor that echoes each stream it's opened.
    s.host("b", move || {
        let (seen, at_b) = (seen_b.clone(), at_b.clone());
        async move {
            let (_, f) = xenonet(B, &[A, B], Arc::default()).await?;
            *at_b.locked() = Some(f.clone());
            let dial = ForwardDial::attach(duplex(f), "tok-b", SYNC).await?;
            while let Some(wire) = dial.incoming().await {
                seen.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    while let Ok((mut w, mut r)) = wire.accept_bi().await {
                        tokio::spawn(async move {
                            let _ = tokio::io::copy(&mut r, &mut w).await;
                            let _ = w.shutdown().await;
                        });
                    }
                });
            }
            Ok(())
        }
    });
    // C: a machine B's chain doesn't list, whatever C's own says.
    let refused_c = Arc::new(Mutex::new(None));
    let at_c = refused_c.clone();
    s.client("c", async move {
        let (mesh, _) = xenonet(C, &[B, C], Arc::default()).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let r = match mesh.dial(B, SYNC).await {
            Ok(l) => l.closed().await,
            Err(_) => Some(Close::Refused),
        };
        *at_c.locked() = Some(r);
        Ok(())
    });
    let grants = Arc::new(Fake::default());
    let (seen_a, b_at_a) = (seen.clone(), b_forwards.clone());
    s.client("a", async move {
        let (_, f) = xenonet(A, &[A, B], grants.clone()).await?;
        let dial = ForwardDial::attach(duplex(f.clone()), "tok-a", SYNC).await?;
        assert_eq!(dial.me(), A);
        // The chains the composition root hands out, as asked for.
        assert_eq!(dial.chains().await?, json!({ "networks": [], "persons": [], "roots": [] }), "none set: empty lists");
        f.set_chains(Arc::new(|| json!({ "networks": [{ "id": N1 }], "persons": [], "roots": [] })));
        assert_eq!(dial.chains().await?["networks"][0]["id"], N1);
        let second = ForwardDial::attach(duplex(f.clone()), "tok-a2", SYNC).await.err().map(|e| e.code);
        assert_eq!(second, Some("taken"), "one actor per ALPN");
        // Refused before anything is forwarded: C is in none of A's networks.
        assert_eq!(dial.connect(C, SYNC).await.err().map(|e| e.code), Some("not-a-member"));
        assert_eq!(dial.connect(B, Alpn::Link).await.err().map(|e| e.code), Some("other-alpn"), "the forward's own ALPN alone");
        let wire = loop {
            match dial.connect(B, SYNC).await {
                Ok(w) => break w,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        assert_eq!(dial.networks(B), [N1.to_string()].into(), "the header names the networks both are in");
        assert_eq!(wire.exporter(b"x"), None, "the simulated transport has no keying material, so the header has none");
        let (mut w, mut r) = wire.open_bi().await?;
        w.write_all(b"hello over the forward").await.unwrap();
        w.shutdown().await.unwrap();
        let mut back = Vec::new();
        r.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, b"hello over the forward");
        assert_eq!(f.list()["forwards"][0]["actor"], "lith-a");
        assert_eq!(f.list()["forwards"][0]["connections"].as_array().map(Vec::len), Some(1));
        until("b's actor saw a's connection", || seen_a.load(Ordering::SeqCst) == 1).await?;
        let on_b = b_at_a.locked().clone().unwrap();
        until("b lists it", || on_b.list()["forwards"][0]["connections"].as_array().is_some_and(|c| c.len() == 1)).await?;

        // Revoked: the session ends, and what it forwarded closes, both ends.
        grants.revoked.locked().insert("tok-a".into());
        f.revoked("id-tok-a");
        tokio::time::timeout(Duration::from_secs(5), dial.ended()).await.map_err(|_| NetError::internal("the session didn't end"))?;
        assert!(wire.is_closed());
        until("b's end closed", || on_b.list()["forwards"][0]["connections"].as_array().is_some_and(Vec::is_empty)).await?;
        assert_eq!(f.list()["forwards"], json!([]));
        let again = ForwardDial::attach(duplex(f.clone()), "tok-a", SYNC).await.err().map(|e| e.code);
        assert_eq!(again, Some("revoked"));
        // The ALPN is free for another actor's grant.
        assert!(ForwardDial::attach(duplex(f), "tok-a2", SYNC).await.is_ok());
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(*refused_c.locked(), Some(Some(Close::Refused)), "b refused c");
    assert_eq!(seen.load(Ordering::SeqCst), 1, "only a's connection reached b's actor");
}

#[test]
fn a_grant_revoked_without_word_is_caught_at_the_next_connection_or_stream() {
    let mut s = sim(2);
    let grants_b = Arc::new(Fake::default());
    let gb = grants_b.clone();
    let ended = Arc::new(AtomicU64::new(0));
    let e = ended.clone();
    s.host("b", move || {
        let (gb, e) = (gb.clone(), e.clone());
        async move {
            let (_, f) = xenonet(B, &[A, B], gb).await?;
            let dial = ForwardDial::attach(duplex(f), "tok-b", SYNC).await?;
            dial.ended().await;
            e.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            Ok(())
        }
    });
    s.client("a", async move {
        let (mesh, _) = xenonet(A, &[A, B], Arc::default()).await?;
        // B's grant is revoked, and B's XenoNet isn't told: the next
        // connection to it re-decides, and is refused.
        tokio::time::sleep(Duration::from_millis(500)).await;
        grants_b.revoked.locked().insert("tok-b".into());
        let link = loop {
            match mesh.dial(B, SYNC).await {
                Ok(l) => break l,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        assert_eq!(link.closed().await, Some(Close::Refused));
        until("b's session ended", || ended.load(Ordering::SeqCst) == 1).await?;
        Ok(())
    });
    s.run().unwrap();
}
