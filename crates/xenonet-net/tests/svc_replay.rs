// A request to a published service goes to it once, however often its
// stream is sent (docs/design/network.md, "Services through the
// network"). studio's node runs svc/2 on its endpoint, on loopback, and
// publishes a made-up HTTP service; laptop is a bare endpoint with
// laptop's host key, a member by studio's chain. The same request on a
// second stream, and on a new connection, is answered 409 and the service
// is called once; a new request id goes; and a stream naming a network
// studio doesn't share with laptop is refused before any service is read.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenonet_net::admission::Admission;
use xenonet_net::dial::{Alpn, Close, Dial, Mesh};
use xenonet_net::endpoint::{peer_id, IrohDial, LanLookup, Relays, Settings};
use xenonet_net::files::{Dir, NetFile, Pin, KEY};
use xenonet_net::identity::Identity;
use xenonet_net::keys::HostKey;
use xenonet_net::node::{Node, Options};
use xenonet_net::nodes::Nodes;
use xenonet_net::persist::{Memory, Persist};
use xenonet_net::roster::{found, HostRef, NetworkPolicy, Op, Roster, ADMIT};
use xenonet_net::sink::{LogSink, SinkSlot};
use xenonet_net::svc::{request_id, Request};
use xenonet_net::svc2;

const NET: &str = "net_00112233445566778899aabbccddeeff";
const T0: i64 = 1_792_512_000_000;
const MIN: i64 = 60_000;

/// A made-up service that counts the requests it gets. Its port.
async fn counting_service(hits: Arc<AtomicUsize>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let hits = hits.clone();
            tokio::spawn(async move {
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                // The head, then as much body as it says.
                loop {
                    let Ok(n) = s.read(&mut buf).await else { return };
                    if n == 0 {
                        return;
                    }
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if got.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
            });
        }
    });
    port
}

/// studio's node, in a network with laptop, on a clock the test sets:
/// its folder, the node, the clock, and both hosts' keys.
fn studio(tag: &str) -> (std::path::PathBuf, Arc<Nodes>, Arc<Node>, Arc<AtomicI64>, HostKey, HostKey) {
    let dir = std::env::temp_dir().join(format!("xenonet-svc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let kept: Arc<dyn Persist> = Arc::new(Memory::default());
    let files = Dir::new(dir.join("network"), kept.clone());
    let studio = HostKey::load_or_create(&files.file(KEY)).unwrap();
    let laptop = HostKey::generate().unwrap();
    // The person, on studio's fake device, who admits laptop.
    let vault: Arc<dyn xenonet_id::vault::Vault> = Arc::new(xenonet_keys::Fake { allow: true });
    let identity = Identity::open(files.identity_dir(), kept.clone(), Some(vault.clone())).unwrap();
    identity.found(true, "studio", T0).unwrap();
    let machine = |seed: u8| xenonet_id::sign::Signer::public(&xenonet_id::sign::SeedSigner::from_seed(&[seed; 32], xenonet_id::sign::KeyHome::File).unwrap());
    let me = HostRef { id: "studio".into(), keys: *studio.public(), window: true, device: identity.device_id(), machine: identity.machine_key().unwrap().public() };
    let mut roster = Roster::with_policy(NetworkPolicy::with(identity.clone()));
    roster.offer(found(NET, me, identity.machine_key().unwrap().as_ref(), identity.person_id(), identity.head(), None, T0).unwrap()).unwrap();
    let mut add = roster.draft(Op::Add, HostRef { id: "laptop".into(), keys: *laptop.public(), window: false, device: None, machine: machine(9) }, None, T0 + 1);
    add.person_head = identity.head();
    let block = identity.prove(true, ADMIT, &add.digest(), "", T0 + 1).unwrap();
    add.sign_with("studio", identity.machine_key().unwrap().as_ref(), Some(block)).unwrap();
    roster.offer(add).unwrap();
    files.write_roster(NET, &roster).unwrap();
    let p = studio.public();
    let pin = Pin { host: "studio".into(), key_id: p.key_id(), sign: p.sign_b64(), seal: p.seal_b64(), address: Some("127.0.0.1:1".into()) };
    files
        .write_network(&NetFile {
            id: NET.into(),
            name: "home".into(),
            host: "studio".into(),
            founder: "studio".into(),
            gateway: pin,
            lan: None,
            titles_only: false,
            share_load: true,
            joined: T0,
        })
        .unwrap();
    let clock = Arc::new(AtomicI64::new(T0 + 10 * MIN));
    let mut o = Options::new(dir.join("network"), kept, "0.0.0-test", "linux", Arc::new(|_: &str, _: &str| {}));
    o.vault = Some(vault);
    o.link = true;
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o.lan = LanLookup::Off;
    let c = clock.clone();
    o.clock = Arc::new(move || c.load(Ordering::SeqCst) as f64);
    let nodes = Nodes::open(o, Arc::new(SinkSlot::new(Arc::new(LogSink::new(Arc::new(|_: &str, _: &str| {})))))).unwrap();
    let node = nodes.first();
    (dir, nodes, node, clock, studio, laptop)
}

const OTHER: &str = "net_ffeeddccbbaa99887766554433221100";

fn loopback() -> Settings {
    Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some("127.0.0.1:0".parse().unwrap()), roots: Vec::new() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_svc_request_is_passed_on_once_however_often_its_stream_is_sent() {
    let (dir, nodes, node, _clock, studio, laptop) = studio("replay");
    let n = node.shared();
    let hits = Arc::new(AtomicUsize::new(0));
    let port = counting_service(hits.clone()).await;
    n.svc_publish("files", &format!("http://127.0.0.1:{port}"), &["/pay/".into()], &[], true).unwrap();
    nodes.start().unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let t = loop {
        if let Some(t) = n.link2() {
            break t;
        }
        assert!(std::time::Instant::now() < end, "studio's endpoint started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    // laptop: its own key's endpoint, studio a member of NET by its chain,
    // and of another network studio isn't in with it.
    let admission = Arc::new(Admission::new());
    let dial = Arc::new(IrohDial::bind(&laptop, &loopback(), admission.clone()).await.unwrap());
    let mesh = Mesh::new(dial.clone(), admission);
    let at = peer_id(studio.public());
    mesh.set_members(NET, [at]);
    mesh.set_members(OTHER, [at]);
    dial.addresses().put(at, &t.dial.direct_addrs(), None, Duration::from_secs(60)).unwrap();

    let req = Request {
        id: request_id().unwrap(),
        name: "files".into(),
        method: "POST".into(),
        path: "/pay/once".into(),
        headers: vec![("authorization".into(), "Bearer t".into())],
        body: b"5 EUR".to_vec(),
    };
    let wait = Duration::from_secs(10);
    let a = svc2::call(&mesh, at, NET, &req.head(NET), &req.body, wait).await;
    assert_eq!((a.status, a.body.as_slice()), (200, b"ok".as_slice()), "{}", a.message());
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    // The same request on a second stream.
    let a = svc2::call(&mesh, at, NET, &req.head(NET), &req.body, wait).await;
    assert_eq!((a.status, a.code.as_deref()), (409, Some("replayed")));
    // And on a new connection.
    mesh.link(at, Alpn::Svc).unwrap().close(Close::Done);
    let a = svc2::call(&mesh, at, NET, &req.head(NET), &req.body, wait).await;
    assert_eq!((a.status, a.code.as_deref()), (409, Some("replayed")));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1, "the service was called once");

    // A new request goes.
    let again = Request { id: request_id().unwrap(), ..req.clone() };
    let a = svc2::call(&mesh, at, NET, &again.head(NET), &again.body, wait).await;
    assert_eq!(a.status, 200);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // A stream naming a network studio's chains don't share with laptop,
    // over the connection the two share: refused at studio.
    let other = Request { id: request_id().unwrap(), ..req.clone() };
    let a = svc2::call(&mesh, at, OTHER, &other.head(OTHER), &other.body, wait).await;
    assert_eq!((a.status, a.code.as_deref()), (403, Some("not-in-network")), "{}", a.message());
    // A head that says nothing it should: refused, and nothing read.
    let a = svc2::call(&mesh, at, NET, &serde_json::json!({ "schema": "network/svc/v2", "op": "request", "network": NET }), &[], wait).await;
    assert_eq!(a.status, 400);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // laptop is no window host, and has no device of the person's: its
    // window reads are refused.
    let key = svc2::WindowKey::make(dial.me(), "prs_x", "xns1_x", T0, |_| Ok(vec![0; 64])).unwrap();
    let read = key.read(NET, at, "/api/session/claude:abc/transcript", xenonet_net::node::now_ms() as i64).unwrap();
    let a = svc2::call(&mesh, at, NET, &read.head(), &[], wait).await;
    assert_eq!((a.status, a.code.as_deref()), (403, Some("not-window")), "{}", a.message());

    mesh.close().await;
    nodes.stop();
    drop(node);
    let _ = std::fs::remove_dir_all(dir);
}
