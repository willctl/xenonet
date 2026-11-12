// Real iroh endpoints on loopback, with no relays and the memory lookup
// alone: nothing leaves this machine, and mDNS never runs (it would
// announce on the LAN).

use super::*;
use crate::dial::{Link, Mesh, REMOVED_GRACE};
use std::future::Future;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(10);

struct Machine {
    id: PeerId,
    dial: Arc<IrohDial>,
    mesh: Arc<Mesh>,
}

fn loopback() -> Settings {
    Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some("127.0.0.1:0".parse().unwrap()), roots: Vec::new() }
}

async fn machine_with(early_cut: bool) -> Machine {
    let key = HostKey::generate().unwrap();
    let admission = Arc::new(Admission::new());
    let dial = Arc::new(IrohDial::bind_with(&key, &loopback(), admission.clone(), Knobs { early_cut, ..Knobs::default() }).await.unwrap());
    let mesh = Arc::new(Mesh::new(dial.clone(), admission));
    assert_eq!(dial.me(), peer_id(key.public()));
    Machine { id: dial.me(), dial, mesh }
}

async fn machine() -> Machine {
    machine_with(true).await
}

// `from` learns where `to` is.
fn introduce(from: &Machine, to: &Machine) {
    from.dial.addresses().put(to.id, &to.dial.direct_addrs(), None, Duration::from_secs(60)).unwrap();
}

// The mesh's admitted connections, as they come, on a channel.
fn accepting(m: &Machine) -> mpsc::UnboundedReceiver<Arc<Link>> {
    let (to, from) = mpsc::unbounded_channel();
    let mesh = m.mesh.clone();
    tokio::spawn(async move {
        while let Some(l) = mesh.accept().await {
            if to.send(l).is_err() {
                return;
            }
        }
    });
    from
}

async fn within<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, f).await.expect("in time")
}

// Sends `what` on a new stream of `link` and returns what came back.
async fn ask(link: &Link, what: &[u8]) -> Vec<u8> {
    let (mut w, mut r) = link.open_bi().await.unwrap();
    w.write_all(what).await.unwrap();
    w.shutdown().await.unwrap();
    let mut back = Vec::new();
    r.read_to_end(&mut back).await.unwrap();
    back
}

// Answers one stream of `link` with what it said.
async fn echo(link: &Link) {
    let (mut w, mut r) = link.accept_bi().await.unwrap();
    let mut got = Vec::new();
    r.read_to_end(&mut got).await.unwrap();
    w.write_all(&got).await.unwrap();
    w.shutdown().await.unwrap();
}

#[test]
fn the_endpoint_id_is_host_keys_ed25519_public_key() {
    // RFC 8032's first Ed25519 test vector: ring (host.key) and iroh
    // (ed25519-dalek) derive the same public key from the seed.
    let unhex = |s: &str| -> [u8; 32] {
        let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
        v.try_into().unwrap()
    };
    let seed = unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    let public = unhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let key = HostKey::from_secrets(&seed, [7; 32]).unwrap();
    assert_eq!(key.public().sign, public);
    assert_eq!(*secret_key(&key).public().as_bytes(), public);
    assert_eq!(peer_id(key.public()), PeerId(public));
    let random = HostKey::generate().unwrap();
    assert_eq!(*secret_key(&random).public().as_bytes(), random.public().sign);
}

#[tokio::test]
async fn two_endpoints_on_loopback_talk_over_link_2() {
    let (a, b) = (machine().await, machine().await);
    for m in [&a, &b] {
        m.mesh.set_members("n1", [a.id, b.id]);
    }
    introduce(&a, &b);
    let mut at_b = accepting(&b);
    let link = within(a.mesh.dial(b.id, Alpn::Link)).await.unwrap();
    assert_eq!((link.peer(), link.alpn(), link.dialled_by()), (b.id, Alpn::Link, a.id));
    assert_eq!(link.networks(), ["n1".to_string()].into());
    let (asked, ()) = tokio::join!(ask(&link, b"hello"), async {
        let theirs = within(at_b.recv()).await.unwrap();
        assert_eq!((theirs.peer(), theirs.dialled_by()), (a.id, a.id));
        echo(&theirs).await;
    });
    assert_eq!(asked, b"hello");
    // A second dial shares the connection.
    assert!(Arc::ptr_eq(&link, &a.mesh.dial(b.id, Alpn::Link).await.unwrap()));
    a.mesh.close().await;
    b.mesh.close().await;
}

// The peer that isn't a member: `b` holds a chain without `a`, and `a`
// thinks itself a member.
async fn refused_by(b: &Machine) {
    let a = machine().await;
    a.mesh.set_members("n1", [a.id, b.id]);
    b.mesh.set_members("n1", [b.id]);
    introduce(&a, b);
    let mut at_b = accepting(b);
    for alpn in [Alpn::Link, Alpn::Svc] {
        match within(a.mesh.dial(b.id, alpn)).await {
            Err(_) => {}
            Ok(l) => assert_eq!(within(l.closed()).await, Some(Close::Refused), "{alpn:?}"),
        }
    }
    assert!(tokio::time::timeout(Duration::from_millis(500), at_b.recv()).await.is_err(), "b hands out nothing");
    a.mesh.close().await;
}

#[tokio::test]
async fn an_unknown_key_is_refused_by_the_hooks_and_by_the_doors_without_them() {
    let b = machine().await;
    refused_by(&b).await;
    b.mesh.close().await;
    let b = machine_with(false).await;
    refused_by(&b).await;
    b.mesh.close().await;
}

#[tokio::test]
async fn nothing_is_dialled_to_a_machine_in_no_network() {
    let (a, c) = (machine().await, machine().await);
    a.mesh.set_members("n1", [a.id]);
    introduce(&a, &c);
    let e = a.mesh.dial(c.id, Alpn::Link).await.unwrap_err();
    assert_eq!((e.status, e.code), (403, "not-a-member"));
    // The endpoint's own hook refuses it before a packet, mesh or not.
    let e = a.dial.connect(c.id, Alpn::Link).await.unwrap_err();
    assert_eq!(e.code, "unreachable");
    a.mesh.close().await;
    c.mesh.close().await;
}

#[tokio::test]
async fn removal_from_n1_while_still_in_n2_stops_n1_and_n2_still_flows() {
    let (a, b) = (machine().await, machine().await);
    for m in [&a, &b] {
        m.mesh.set_members("n1", [a.id, b.id]);
        m.mesh.set_members("n2", [a.id, b.id]);
    }
    introduce(&b, &a);
    let mut at_a = accepting(&a);
    let bs = within(b.mesh.dial(a.id, Alpn::Link)).await.unwrap();
    let ask_first = tokio::spawn({
        let bs = bs.clone();
        async move { ask(&bs, b"before").await }
    });
    let at = within(at_a.recv()).await.unwrap();
    echo(&at).await;
    assert_eq!(ask_first.await.unwrap(), b"before");
    assert_eq!(at.networks(), ["n1".to_string(), "n2".to_string()].into());

    // a applies b's removal from n1.
    a.mesh.set_members("n1", [a.id]);
    assert_eq!(at.networks(), ["n2".to_string()].into());
    assert_eq!(at.check("n1").unwrap_err().code, "not-in-network");
    assert!(at.check("n2").is_ok());
    assert!(!at.is_closed(), "n2 is still shared");
    let (asked, ()) = tokio::join!(ask(&bs, b"n2 still"), echo(&at));
    assert_eq!(asked, b"n2 still");

    // And from n2: nothing is shared, so the connection leaves the mesh at
    // once, carries no network, and closes after the grace that lets what
    // was sent b last go; b is told why.
    a.mesh.set_members("n2", [a.id]);
    assert!(a.mesh.link(b.id, Alpn::Link).is_none());
    assert_eq!(at.check("n2").unwrap_err().code, "not-in-network");
    let started = Instant::now();
    assert_eq!(within(bs.closed()).await, Some(Close::Removed));
    assert!(at.is_closed());
    assert!(started.elapsed() >= REMOVED_GRACE.saturating_sub(Duration::from_millis(100)), "closed after the grace, not at once");
    a.mesh.close().await;
    b.mesh.close().await;
}

#[tokio::test]
async fn crossing_dials_keep_the_connection_the_lower_endpoint_id_dialled() {
    let (a, b) = (machine().await, machine().await);
    for m in [&a, &b] {
        m.mesh.set_members("n1", [a.id, b.id]);
    }
    introduce(&a, &b);
    introduce(&b, &a);
    let (_at_a, _at_b) = (accepting(&a), accepting(&b));
    let (ab, ba) = tokio::join!(a.mesh.dial(b.id, Alpn::Link), b.mesh.dial(a.id, Alpn::Link));
    ab.unwrap();
    ba.unwrap();
    let lower = a.id.min(b.id);
    let settled = || {
        let (x, y) = (a.mesh.link(b.id, Alpn::Link), b.mesh.link(a.id, Alpn::Link));
        matches!((&x, &y), (Some(x), Some(y)) if x.dialled_by() == lower && y.dialled_by() == lower)
    };
    within(async {
        while !settled() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let (x, y) = (a.mesh.link(b.id, Alpn::Link).unwrap(), b.mesh.link(a.id, Alpn::Link).unwrap());
    let (asked, ()) = tokio::join!(ask(&x, b"one connection"), echo(&y));
    assert_eq!(asked, b"one connection");
    a.mesh.close().await;
    b.mesh.close().await;
}

#[tokio::test]
async fn join_is_dialled_only_to_the_invites_endpoint_and_answered_only_with_a_ticket_open() {
    // The doors alone (no hooks), so the closed ALPN is what turns the
    // joiner away.
    let (inviter, joiner, other) = (machine_with(false).await, machine().await, machine().await);
    inviter.mesh.set_members("n1", [inviter.id]);
    introduce(&joiner, &inviter);
    introduce(&joiner, &other);
    let mut at_inviter = accepting(&inviter);

    let e = joiner.mesh.dial(inviter.id, Alpn::Join).await.unwrap_err();
    assert_eq!(e.code, "not-invited");
    joiner.mesh.begin_join(inviter.id);
    assert_eq!(joiner.mesh.dial(other.id, Alpn::Join).await.unwrap_err().code, "not-invited");
    assert!(within(joiner.mesh.dial(inviter.id, Alpn::Join)).await.is_err(), "no ticket open: join/2 isn't offered");

    inviter.mesh.open_ticket();
    let link = within(joiner.mesh.dial(inviter.id, Alpn::Join)).await.unwrap();
    assert!(link.networks().is_empty(), "a join carries no network");
    let (asked, ()) = tokio::join!(ask(&link, b"join"), async {
        let theirs = within(at_inviter.recv()).await.unwrap();
        assert_eq!((theirs.alpn(), theirs.peer()), (Alpn::Join, joiner.id));
        echo(&theirs).await;
    });
    assert_eq!(asked, b"join");
    // A joiner is no member: the members' protocols stay shut to it.
    joiner.mesh.set_members("n1", [inviter.id]);
    match within(joiner.mesh.dial(inviter.id, Alpn::Link)).await {
        Err(_) => {}
        Ok(l) => assert_eq!(within(l.closed()).await, Some(Close::Refused)),
    }
    inviter.mesh.close_ticket();
    link.close(Close::Done);
    joiner.mesh.end_join(&inviter.id);
    for m in [inviter, joiner, other] {
        m.mesh.close().await;
    }
}

#[tokio::test]
async fn with_relays_none_the_endpoint_reaches_nothing_outside_its_lists() {
    let m = machine().await;
    assert_eq!(m.dial.lookup_count(), 1, "the memory lookup alone");
    assert_eq!(lookups(LanLookup::Off), [MemoryLookup::PROVENANCE]);
    assert_eq!(lookups(LanLookup::Unasked), [MemoryLookup::PROVENANCE], "mDNS stays off until asked");
    assert_eq!(lookups(LanLookup::On), [MemoryLookup::PROVENANCE, iroh_mdns_address_lookup::NAME]);
    assert_eq!(m.dial.endpoint().addr().relay_urls().count(), 0);
    assert!(!m.dial.has_home_relay());
    within(m.dial.online()).await;
    assert!(m.dial.direct_addrs().iter().all(|a| a.ip().is_loopback()));
    assert!(matches!(relay_mode(&Relays::None).unwrap(), RelayMode::Disabled));
    assert!(matches!(relay_mode(&Relays::Listed(Vec::new())).unwrap(), RelayMode::Disabled));
    assert_eq!(m.dial.set_relays(Relays::Listed(vec![Relay { url: "https://relay.example.test".into(), auth_token: None }])).await.unwrap_err().code, "restart-needed");

    // A member with no known address: the dial fails at once, since there
    // is no lookup to ask anywhere else.
    let unknown = PeerId(*SecretKey::from_bytes(&[3; 32]).public().as_bytes());
    m.mesh.set_members("n1", [m.id, unknown]);
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), m.mesh.dial(unknown, Alpn::Link)).await.expect("fails fast").unwrap_err().code, "unreachable");
    m.mesh.close().await;
}

#[test]
fn listed_relays_are_exactly_the_persons_with_their_tokens() {
    let relays = Relays::Listed(vec![
        Relay { url: "https://relay.example.test".into(), auth_token: Some("t0ken".into()) },
        Relay { url: "https://other.example.test".into(), auth_token: None },
    ]);
    let RelayMode::Custom(map) = relay_mode(&relays).unwrap() else { panic!("a custom map") };
    let urls: Vec<String> = map.urls::<Vec<RelayUrl>>().iter().map(ToString::to_string).collect();
    assert_eq!(urls, ["https://other.example.test/", "https://relay.example.test/"]);
    assert_eq!(map.get(&relay_url("https://relay.example.test").unwrap()).unwrap().auth_token.as_deref(), Some("t0ken"));
    assert_eq!(map.get(&relay_url("https://other.example.test").unwrap()).unwrap().auth_token, None);
    assert!(!format!("{relays:?}").contains("t0ken"), "a token never reaches the log");
    assert_eq!(relay_mode(&Relays::Listed(vec![Relay { url: "not a url".into(), auth_token: None }])).unwrap_err().code, "bad-request");
}

#[test]
fn addresses_outlive_their_time_to_live_nowhere() {
    let a = Addresses::default();
    let peer = PeerId(*SecretKey::from_bytes(&[5; 32]).public().as_bytes());
    let at: SocketAddr = "127.0.0.1:4433".parse().unwrap();
    a.put(peer, &[at], None, Duration::from_secs(60)).unwrap();
    assert_eq!(a.get(&peer).unwrap().ip_addrs().copied().collect::<Vec<_>>(), [at]);
    a.put(peer, &[at], None, Duration::ZERO).unwrap();
    assert!(a.get(&peer).is_none(), "expired");
    a.put(peer, &[at], Some("https://relay.example.test"), Duration::from_secs(60)).unwrap();
    assert_eq!(a.get(&peer).unwrap().relay_urls().count(), 0, "a relay the endpoint doesn't list is left out");
    assert_eq!(a.get(&peer).unwrap().ip_addrs().count(), 1, "the direct address stays");
    a.list(&Relays::Listed(vec![Relay { url: "https://relay.example.test".into(), auth_token: None }]));
    assert!(a.lists("https://relay.example.test") && !a.lists("https://other.example.test"));
    a.put(peer, &[at], Some("https://relay.example.test"), Duration::from_secs(60)).unwrap();
    assert_eq!(a.get(&peer).unwrap().relay_urls().count(), 1);
    a.list(&Relays::None);
    assert_eq!(a.get(&peer).unwrap().relay_urls().count(), 0, "a relay taken off the list goes from what was kept");
    assert_eq!(a.get(&peer).unwrap().ip_addrs().count(), 1);
    a.forget(&peer);
    assert!(a.get(&peer).is_none());
}
