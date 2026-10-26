// A request to a published service is passed on once, however it comes
// back (docs/design/network.md, section 14). studio publishes a made-up
// HTTP service; laptop's svc request, dated four minutes ahead as a
// gateway could let it be, reaches studio's node. It's sent again three
// minutes later, when a host that kept ids only two minutes from their
// arrival would take it as new, and again after the gateway sets studio's
// clock back. The service is called once. And a svc frame from a batch
// the gateway refused for this host's time goes again only while its own
// date is right.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenonet_net::codec::frame_id;
use xenonet_net::files::{Dir, NetFile, Pin, KEY};
use xenonet_net::frame::{host_frame, Draft};
use xenonet_net::keys::HostKey;
use xenonet_net::node::{Batch, Node, Options};
use xenonet_net::roster::{found, HostRef, Op, Roster};
use xenonet_net::sink::{LogSink, SinkSlot};
use xenonet_net::svc::{request_id, Request};

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
fn studio(tag: &str) -> (std::path::PathBuf, Arc<Node>, Arc<AtomicI64>, HostKey, HostKey) {
    let dir = std::env::temp_dir().join(format!("xenonet-svc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let files = Dir::new(&dir);
    let studio = HostKey::load_or_create(&files.file(KEY)).unwrap();
    let laptop = HostKey::generate().unwrap();
    let mut roster = Roster::new();
    roster.offer(found(NET, "studio", &studio, None, T0)).unwrap();
    let mut add = roster.draft(Op::Add, HostRef { id: "laptop".into(), keys: *laptop.public(), window: false }, None, T0 + 1);
    add.sign_by("studio", &studio);
    roster.offer(add).unwrap();
    files.write_roster(&roster).unwrap();
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
    let mut o = Options::new(&dir, "0.0.0-test", "linux", Arc::new(|_: &str, _: &str| {}));
    let c = clock.clone();
    o.clock = Arc::new(move || c.load(Ordering::SeqCst) as f64);
    let node = Node::open(o, Arc::new(SinkSlot::new(Arc::new(LogSink::new(Arc::new(|_: &str, _: &str| {})))))).unwrap();
    (dir, node, clock, studio, laptop)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_svc_request_is_passed_on_once_however_it_comes_back() {
    let (dir, node, clock, studio, laptop) = studio("replay");
    let n = node.shared();

    let hits = Arc::new(AtomicUsize::new(0));
    let port = counting_service(hits.clone()).await;
    n.svc_publish("files", &format!("http://127.0.0.1:{port}"), &["/pay/".into()], &[]).unwrap();

    // laptop's request: a POST with its bearer, dated four minutes ahead.
    let now = T0 + 10 * MIN;
    let req = Request {
        id: request_id().unwrap(),
        name: "files".into(),
        method: "POST".into(),
        path: "/pay/once".into(),
        headers: vec![("authorization".into(), "Bearer t".into())],
        body: b"5 EUR".to_vec(),
    };
    let d = Draft { network: NET, id: frame_id().unwrap(), kind: "svc", from: "laptop", to: "studio", at: now + 4 * MIN };
    let f = host_frame(&d, &laptop, studio.public(), &req.to_json(), None).unwrap();

    let _ = n.frame(&f);
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while hits.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < end, "the first request reached the service");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Three minutes on: past two minutes from its arrival, still within
    // its own time.
    clock.store(now + 3 * MIN, Ordering::SeqCst);
    let _ = n.frame(&f);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1, "three minutes on");

    // Ten minutes on it's stale; then the gateway sets studio's clock back
    // to when it came.
    clock.store(now + 10 * MIN, Ordering::SeqCst);
    let _ = n.frame(&f);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1, "ten minutes on");
    n.learn_offset((now + 4 * MIN) as f64);
    assert_eq!(n.gateway_now() as i64, now + 4 * MIN);
    let _ = n.frame(&f);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1, "with the clock set back");

    // The clock right again, a new request goes.
    n.learn_offset((now + 10 * MIN) as f64);
    let req2 = Request { id: request_id().unwrap(), ..req };
    let d = Draft { network: NET, id: frame_id().unwrap(), kind: "svc", from: "laptop", to: "studio", at: now + 10 * MIN };
    let _ = n.frame(&host_frame(&d, &laptop, studio.public(), &req2.to_json(), None).unwrap());
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while hits.load(Ordering::SeqCst) == 1 {
        assert!(std::time::Instant::now() < end, "a new request reached the service");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(node);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_svc_frame_dated_by_a_wrong_clock_isnt_sent_again() {
    // The gateway refused this host's time, and it learned the right one:
    // a svc frame from the refused batch goes again only while its own
    // date, sealed in, is within a minute of that time.
    let (dir, node, _clock, studio, laptop) = studio("clock");
    let n = node.shared();
    let now = T0 + 10 * MIN;
    let answer = |at: i64| {
        let d = Draft { network: NET, id: frame_id().unwrap(), kind: "svc", from: "studio", to: "laptop", at };
        host_frame(&d, &studio, laptop.public(), &serde_json::json!({ "op": "answer" }), None).unwrap()
    };
    let batch = |f| Batch { ids: Vec::new(), body: String::new(), relay: true, svc: Some(f) };
    let off = answer(now - 10 * MIN);
    let mut w = n.services().wait("svq_00000000000000000000000000000001", "laptop", &off.id).unwrap();
    n.batch_failed(&batch(off), "", true);
    let a = w.answer.try_recv().unwrap();
    assert_eq!((a.status, a.code.as_deref()), (503, Some("clock")));
    assert!(n.next_batch().is_none(), "not sent again");
    let right = answer(now - 20_000);
    let id = right.id.clone();
    n.batch_failed(&batch(right), "", true);
    assert_eq!(n.next_batch().and_then(|b| b.svc).map(|f| f.id), Some(id), "sent again");
    drop(w);
    drop(node);
    let _ = std::fs::remove_dir_all(dir);
}
