// A real self-hosted relay: iroh's own relay server (iroh-relay, at
// iroh's version), with TLS from a test CA that the endpoints trust the
// way a person's machine does (`relay-ca.pem`). Nothing leaves this
// machine: every relay, endpoint and tap binds 127.0.0.1, and mDNS stays
// off.
//
// Direct paths are turned off the way iroh tests relay-only paths: the
// endpoint has no IP transport at all (`Knobs::direct`), so the only way
// between two of them is a relay.

use super::*;
use crate::dial::{Link, Mesh};
use crate::doctor::RelayProbe;
use futures::StreamExt as _;
use iroh_relay::server::{CertConfig, RelayConfig as Serve, Server, ServerConfig, TlsConfig};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WAIT: Duration = Duration::from_secs(15);
const TTL: Duration = Duration::from_secs(60);
// What one machine sends another, which no relay should ever see.
const MARKER: &[u8] = b"xenonet-plaintext-marker-6c1f3e9a";

async fn within<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, f).await.expect("in time")
}

// ---------- a CA of the person's own ----------

struct Ca {
    issuer: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
}

impl Ca {
    fn new() -> Ca {
        let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.distinguished_name.push(rcgen::DnType::CommonName, "XenoNet test relay CA");
        p.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];
        Ca { issuer: rcgen::CertifiedIssuer::self_signed(p, rcgen::KeyPair::generate().unwrap()).unwrap() }
    }

    fn der(&self) -> CertificateDer<'static> {
        self.issuer.der().clone()
    }

    // A relay's TLS: a certificate for 127.0.0.1 this CA issued.
    fn server(&self) -> rustls::ServerConfig {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut p = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()]).unwrap();
        p.distinguished_name.push(rcgen::DnType::CommonName, "relay.test");
        let cert = p.signed_by(&key, &self.issuer).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::from(rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()));
        rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key)
            .unwrap()
    }
}

// ---------- relays, and a tap in front of one ----------

struct OwnRelay {
    server: Server,
    url: String,
    at: SocketAddr,
}

// iroh's relay server on loopback: over TLS with `tls`, else plain HTTP.
async fn own_relay(tls: Option<rustls::ServerConfig>) -> OwnRelay {
    let https = tls.is_some();
    let mut relay = Serve::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = tls.map(|server_config| TlsConfig::new((Ipv4Addr::LOCALHOST, 0), CertConfig::Manual { server_config }));
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    let server = Server::spawn(config).await.unwrap();
    let at = if https { server.https_addr() } else { server.http_addr() }.unwrap();
    let url = format!("{}://{at}", if https { "https" } else { "http" });
    OwnRelay { server, url, at }
}

// A TCP tap in front of a relay: what goes each way, and how many
// connections came.
struct Tap {
    url: String,
    up: Arc<Mutex<Vec<u8>>>,
    down: Arc<Mutex<Vec<u8>>>,
    connections: Arc<AtomicUsize>,
}

async fn tap(to: SocketAddr, scheme: &str) -> Tap {
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url = format!("{scheme}://{}", l.local_addr().unwrap());
    let (up, down, connections) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicUsize::new(0)));
    let (u, d, n) = (up.clone(), down.clone(), connections.clone());
    tokio::spawn(async move {
        while let Ok((client, _)) = l.accept().await {
            n.fetch_add(1, Ordering::SeqCst);
            let Ok(relay) = TcpStream::connect(to).await else { continue };
            let (cr, cw) = client.into_split();
            let (rr, rw) = relay.into_split();
            tokio::spawn(pump(cr, rw, u.clone()));
            tokio::spawn(pump(rr, cw, d.clone()));
        }
    });
    Tap { url, up, down, connections }
}

async fn pump(mut from: tokio::net::tcp::OwnedReadHalf, mut to: tokio::net::tcp::OwnedWriteHalf, log: Arc<Mutex<Vec<u8>>>) {
    let mut buf = vec![0u8; 16 << 10];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        log.locked().extend_from_slice(&buf[..n]);
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ---------- machines ----------

struct Machine {
    id: PeerId,
    dial: Arc<IrohDial>,
    mesh: Arc<Mesh>,
}

async fn machine(relays: &[&str], roots: &[CertificateDer<'static>], direct: bool) -> Machine {
    let key = HostKey::generate().unwrap();
    let admission = Arc::new(Admission::new());
    let settings = Settings {
        relays: Relays::Listed(relays.iter().map(|u| Relay { url: (*u).to_string(), auth_token: None }).collect()),
        lan: LanLookup::Off,
        bind: Some("127.0.0.1:0".parse().unwrap()),
        roots: roots.to_vec(),
    };
    let dial = Arc::new(IrohDial::bind_with(&key, &settings, admission.clone(), Knobs { direct, ..Knobs::default() }).await.unwrap());
    let mesh = Arc::new(Mesh::new(dial.clone(), admission));
    Machine { id: dial.me(), dial, mesh }
}

fn members(all: &[&Machine]) {
    for m in all {
        m.mesh.set_members("n1", all.iter().map(|x| x.id));
    }
}

fn home(m: &Machine) -> Option<RelayUrl> {
    m.dial.endpoint().addr().relay_urls().next().cloned()
}

fn url(u: &str) -> RelayUrl {
    u.parse().unwrap()
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

// A link from `from` to `to`, and `to`'s end of it.
async fn linked(from: &Machine, to: &Machine, theirs: &mut mpsc::UnboundedReceiver<Arc<Link>>) -> (Arc<Link>, Arc<Link>) {
    let ours = within(from.mesh.dial(to.id, Alpn::Link)).await.unwrap();
    let theirs = within(theirs.recv()).await.unwrap();
    (ours, theirs)
}

// `what` there and back over a new stream of `link`, `theirs` echoing it.
async fn round_trip(link: &Link, theirs: &Link, what: &[u8]) -> Vec<u8> {
    let ask = async {
        let (mut w, mut r) = link.open_bi().await.unwrap();
        w.write_all(what).await.unwrap();
        w.shutdown().await.unwrap();
        let mut back = Vec::new();
        r.read_to_end(&mut back).await.unwrap();
        back
    };
    let echo = async {
        let (mut w, mut r) = theirs.accept_bi().await.unwrap();
        let mut got = Vec::new();
        r.read_to_end(&mut got).await.unwrap();
        w.write_all(&got).await.unwrap();
        w.shutdown().await.unwrap();
    };
    within(async { tokio::join!(ask, echo) }).await.0
}

async fn close(all: &[&Machine]) {
    for m in all {
        m.mesh.close().await;
    }
}

// ---------- the tests ----------

#[test]
fn the_cas_you_add_are_pem_certificates_kept_beside_the_network() {
    let ca = Ca::new();
    let pem = ca.issuer.pem();
    assert_eq!(roots_of(pem.as_bytes()).unwrap(), [ca.der()]);
    assert_eq!(roots_of(format!("{pem}{}", Ca::new().issuer.pem()).as_bytes()).unwrap().len(), 2);
    assert_eq!(roots_of(b"").unwrap_err().code, "bad-request", "no certificate in it");
    assert_eq!(roots_of(b"-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydA==\n-----END CERTIFICATE-----\n").unwrap_err().code, "bad-request", "not a certificate");
    let dir = std::env::temp_dir().join(format!("xenonet-net-relay-ca-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(added_roots(&dir).unwrap().is_empty(), "no file, no CA added");
    std::fs::write(dir.join(RELAY_CA), &pem).unwrap();
    assert_eq!(added_roots(&dir).unwrap(), [ca.der()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_home_relay_matches_the_chains_spelling_of_it() {
    // iroh writes the URL with a trailing slash; a chain keeps the
    // person's spelling, without.
    let chain = vec!["https://relay.example.test".to_string(), "https://203.0.113.7:3443".to_string()];
    assert_eq!(url("https://relay.example.test").to_string(), "https://relay.example.test/");
    assert_eq!(as_listed(&url("https://relay.example.test"), &chain).as_deref(), Some("https://relay.example.test"));
    assert_eq!(as_listed(&url("https://203.0.113.7:3443/"), &chain).as_deref(), Some("https://203.0.113.7:3443"));
    assert_eq!(as_listed(&url("https://other.example.test"), &chain), None);
}

#[tokio::test]
async fn the_lans_answers_lose_the_relays_the_endpoint_doesnt_list() {
    // A machine on the LAN announces a member at a relay no one listed.
    #[derive(Debug)]
    struct Says(EndpointInfo);
    impl AddressLookup for Says {
        fn resolve(&self, _: EndpointId) -> Option<Found> {
            Some(Box::pin(futures::stream::iter([Ok(Item::new(self.0.clone(), "lan", None))])))
        }
    }
    let id = SecretKey::from_bytes(&[9; 32]).public();
    let at: SocketAddr = "192.0.2.7:4433".parse().unwrap();
    let said = EndpointInfo::new(id).with_ip_addrs(vec![at]).with_relay_url(url("https://listed.example.test")).with_relay_url(url("https://elsewhere.example.test"));
    let lookup = ListedLookup { inner: Says(said), listed: Arc::new(Mutex::new(vec![url("https://listed.example.test")])) };
    let found: Vec<_> = lookup.resolve(id).unwrap().collect().await;
    let info = found[0].as_ref().unwrap().endpoint_info().clone();
    assert_eq!(info.relay_urls().cloned().collect::<Vec<_>>(), [url("https://listed.example.test")]);
    assert_eq!(info.ip_addrs().copied().collect::<Vec<_>>(), [at], "direct addresses stay");
}

#[tokio::test]
async fn two_machines_with_no_direct_path_link_through_a_relay_with_your_own_ca() {
    let ca = Ca::new();
    let r = own_relay(Some(ca.server())).await;
    let (a, b) = (machine(&[&r.url], &[ca.der()], false).await, machine(&[&r.url], &[ca.der()], false).await);
    within(a.dial.online()).await;
    within(b.dial.online()).await;
    assert_eq!(home(&b), Some(url(&r.url)));
    assert!(a.dial.direct_addrs().is_empty() && b.dial.direct_addrs().is_empty(), "no direct path at all");
    members(&[&a, &b]);
    a.dial.addresses().put(b.id, &[], Some(&r.url), TTL).unwrap();
    let mut at_b = accepting(&b);
    let (link, theirs) = linked(&a, &b, &mut at_b).await;
    assert_eq!(round_trip(&link, &theirs, MARKER).await, MARKER);
    assert_eq!(link.path(), Some("relay"));

    // The doctor reaches it and says whose CA vouched for it.
    let ok = RelayProbe::new(&r.url, vec!["home".into()], vec![ca.der()]).check().await;
    assert_eq!(ok["status"], "ok", "{ok}");
    let detail = ok["detail"].as_str().unwrap();
    assert!(detail.contains("listed by home") && detail.contains("a CA you added") && detail.contains("iroh-relay 1.4.1") && detail.contains("ms away"), "{ok}");

    // Without the CA, the certificate doesn't check: the doctor says so,
    // and an endpoint never makes the relay its home.
    let bad = RelayProbe::new(&r.url, vec!["home".into()], Vec::new()).check().await;
    assert_eq!(bad["status"], "fail", "{bad}");
    assert!(bad["detail"].as_str().unwrap().contains("no CA this machine trusts issued it"), "{bad}");
    assert!(bad["fix"].as_str().unwrap().contains(RELAY_CA), "{bad}");
    let c = machine(&[&r.url], &[], false).await;
    assert!(tokio::time::timeout(Duration::from_secs(3), c.dial.online()).await.is_err(), "no home relay without the CA");
    assert!(!c.dial.has_home_relay());
    close(&[&a, &b, &c]).await;
}

#[tokio::test]
async fn a_relay_sees_who_talks_never_what() {
    // A relay over plain HTTP behind a tap: the tap sees what the relay's
    // operator would, its TLS taken off.
    let r = own_relay(None).await;
    let t = tap(r.at, "http").await;
    let (a, b) = (machine(&[&t.url], &[], false).await, machine(&[&t.url], &[], false).await);
    within(a.dial.online()).await;
    within(b.dial.online()).await;
    members(&[&a, &b]);
    a.dial.addresses().put(b.id, &[], Some(&t.url), TTL).unwrap();
    let mut at_b = accepting(&b);
    let (link, theirs) = linked(&a, &b, &mut at_b).await;
    let said = MARKER.repeat(64);
    assert_eq!(round_trip(&link, &theirs, &said).await, said);
    assert_eq!(link.path(), Some("relay"));
    let (up, down) = (t.up.locked().clone(), t.down.locked().clone());
    // The tap does see the relay's own protocol in the clear: the upgrade
    // to it, and the endpoint ids it routes by.
    assert!(holds(&up, b"GET /relay"), "the tap sees the relay's requests");
    assert!(holds(&down, &a.id.0) || holds(&down, &b.id.0), "the relay routes by endpoint id, in the clear");
    // What the machines said crossed only as QUIC ciphertext.
    assert!(!holds(&up, MARKER) && !holds(&down, MARKER), "no plaintext crossed the relay");
    close(&[&a, &b]).await;
}

#[tokio::test]
async fn a_relay_the_network_doesnt_list_is_never_used() {
    let ca = Ca::new();
    let listed = own_relay(Some(ca.server())).await;
    let other = own_relay(Some(ca.server())).await;
    let t = tap(other.at, "https").await;
    // `b` makes the unlisted relay its home: it's in another network, say.
    let (a, b) = (machine(&[&listed.url], &[ca.der()], false).await, machine(&[&t.url], &[ca.der()], false).await);
    within(a.dial.online()).await;
    within(b.dial.online()).await;
    members(&[&a, &b]);
    let before = t.connections.load(Ordering::SeqCst);
    assert!(before >= 1, "b reached its home through the tap");

    // b's address names the unlisted relay: it's left out.
    a.dial.addresses().put(b.id, &[], Some(&t.url), TTL).unwrap();
    assert!(!a.dial.addresses().lists(&t.url));
    assert_eq!(a.dial.addresses().get(&b.id).unwrap().relay_urls().count(), 0);
    assert_eq!(within(a.mesh.dial(b.id, Alpn::Link)).await.unwrap_err().code, "unreachable");
    assert_eq!(t.connections.load(Ordering::SeqCst), before, "a never contacted the unlisted relay");

    // Why it's left out: told the same address past `Addresses`, iroh
    // dials the unlisted relay at once.
    a.dial.addresses().memory.set_endpoint_info(EndpointAddr::new(endpoint_id(b.id).unwrap()).with_relay_url(url(&t.url)));
    let mut at_b = accepting(&b);
    let (link, theirs) = linked(&a, &b, &mut at_b).await;
    assert_eq!(round_trip(&link, &theirs, MARKER).await, MARKER);
    assert!(t.connections.load(Ordering::SeqCst) > before, "iroh itself would have used it");
    close(&[&a, &b]).await;
}

#[tokio::test]
async fn a_relay_going_down_leaves_direct_paths_and_the_doctor_names_it() {
    let ca = Ca::new();
    let r = own_relay(Some(ca.server())).await;
    let (a, b) = (machine(&[&r.url], &[ca.der()], true).await, machine(&[&r.url], &[ca.der()], true).await);
    within(a.dial.online()).await;
    within(b.dial.online()).await;
    members(&[&a, &b]);
    a.dial.addresses().put(b.id, &b.dial.direct_addrs(), Some(&r.url), TTL).unwrap();
    let mut at_b = accepting(&b);
    let (link, theirs) = linked(&a, &b, &mut at_b).await;
    assert_eq!(round_trip(&link, &theirs, MARKER).await, MARKER);

    r.server.shutdown().await.unwrap();
    // The link goes on, on its direct path.
    assert_eq!(round_trip(&link, &theirs, MARKER).await, MARKER);
    within(async {
        while link.path() != Some("direct") {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    // And a new one goes direct from the start.
    let c = machine(&[&r.url], &[ca.der()], true).await;
    members(&[&a, &b, &c]);
    c.dial.addresses().put(b.id, &b.dial.direct_addrs(), Some(&r.url), TTL).unwrap();
    let (fresh, theirs) = linked(&c, &b, &mut at_b).await;
    assert_eq!(round_trip(&fresh, &theirs, MARKER).await, MARKER);
    assert_eq!(fresh.path(), Some("direct"));

    let down = RelayProbe::new(&r.url, vec!["home".into()], vec![ca.der()]).check().await;
    assert_eq!(down["status"], "warn", "{down}");
    let detail = down["detail"].as_str().unwrap();
    assert!(detail.starts_with(&r.url) && detail.contains("doesn't answer"), "the relay names itself: {down}");
    assert!(down["fix"].as_str().unwrap().contains("other relays, or direct paths"), "{down}");
    close(&[&a, &b, &c]).await;
}

#[tokio::test]
async fn a_relay_going_down_moves_relay_only_machines_to_another_listed_one() {
    let ca = Ca::new();
    let (r1, r2) = (own_relay(Some(ca.server())).await, own_relay(Some(ca.server())).await);
    let (u1, u2) = (r1.url.clone(), r2.url.clone());
    let both = [u1.as_str(), u2.as_str()];
    let (a, b) = (machine(&both, &[ca.der()], false).await, machine(&both, &[ca.der()], false).await);
    within(a.dial.online()).await;
    within(b.dial.online()).await;
    members(&[&a, &b]);
    let was = home(&b).unwrap();
    a.dial.addresses().put(b.id, &[], Some(was.as_str()), TTL).unwrap();
    let mut at_b = accepting(&b);
    let (link, theirs) = linked(&a, &b, &mut at_b).await;
    assert_eq!(round_trip(&link, &theirs, MARKER).await, MARKER);

    let (gone, left) = if was == url(&r1.url) { (r1, r2) } else { (r2, r1) };
    gone.server.shutdown().await.unwrap();
    // The endpoint has iroh look for a home again once its home has been
    // gone a few seconds (`keep_home`), and takes the one that answers;
    // left to itself, iroh 1.4 would wait for its next full report, up to
    // five minutes.
    tokio::time::timeout(Duration::from_secs(20), async {
        while home(&b).as_ref() != Some(&url(&left.url)) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("b moved to the relay that's left");
    // b's next address record names its new home; a new link takes it.
    let c = machine(&both, &[ca.der()], false).await;
    members(&[&a, &b, &c]);
    c.dial.addresses().put(b.id, &[], Some(&left.url), TTL).unwrap();
    let (fresh, theirs) = linked(&c, &b, &mut at_b).await;
    assert_eq!(round_trip(&fresh, &theirs, MARKER).await, MARKER);
    assert_eq!(fresh.path(), Some("relay"));

    let down = RelayProbe::new(&gone.url, vec!["home".into()], vec![ca.der()]).check().await;
    assert_eq!(down["status"], "warn", "{down}");
    assert!(down["detail"].as_str().unwrap().starts_with(&gone.url), "{down}");
    assert_eq!(RelayProbe::new(&left.url, vec!["home".into()], vec![ca.der()]).check().await["status"], "ok");
    close(&[&a, &b, &c]).await;
}
