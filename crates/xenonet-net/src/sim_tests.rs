// Seeded runs on the simulated network. Each seed is one run, the same
// every time: a failure names its seed, and that seed fails again.

use super::*;
use crate::admission::Admission;
use crate::dial::{Link, Mesh};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use xenonet_model::ops::Kind;

const A: PeerId = PeerId([0xa1; 32]);
const B: PeerId = PeerId([0xb2; 32]);
const C: PeerId = PeerId([0xc3; 32]);

fn hosts() -> BTreeMap<PeerId, String> {
    [(A, "a"), (B, "b"), (C, "c")].into_iter().map(|(p, h)| (p, h.to_string())).collect()
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

async fn mesh(me: PeerId, members: &[PeerId]) -> R<Mesh> {
    let mesh = Mesh::new(Arc::new(SimDial::bind(me, hosts()).await?), Arc::new(Admission::new()));
    mesh.set_members("n1", members.iter().copied());
    Ok(mesh)
}

// Records, each a u32 length and its bytes.
async fn put(w: &mut Writer, record: &[u8]) -> std::io::Result<()> {
    w.write_all(&u32::try_from(record.len()).unwrap().to_be_bytes()).await?;
    w.write_all(record).await
}

async fn take(r: &mut Reader) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut record = vec![0u8; u32::from_be_bytes(len) as usize];
    r.read_exact(&mut record).await?;
    Ok(Some(record))
}

fn messages() -> Vec<Vec<u8>> {
    (0..200).map(|i| serde_json::to_vec(&json!({ "n": i, "text": "x".repeat(i * 7 % 300) })).unwrap()).collect()
}

// b takes a's messages to its engine, in order, and sends each back.
async fn answer(link: &Link, kept: &Kept) -> R<()> {
    let (mut w, mut r) = link.accept_bi().await?;
    while let Some(m) = take(&mut r).await.map_err(io)? {
        link.check("n1")?;
        let body = serde_json::from_slice(&m).map_err(|e| NetError::bad(e.to_string()))?;
        kept.sink.receive(Incoming { from: link.peer().to_string(), kind: Kind::Message, body, at: 0.0, from_window_host: false });
        put(&mut w, &m).await.map_err(io)?;
    }
    w.shutdown().await.map_err(io)
}

fn round_trip(seed: u64) {
    let mut sim = sim(seed);
    let kept = Arc::new(Kept::default());
    let at_b = kept.clone();
    sim.host("b", move || {
        let kept = at_b.clone();
        async move {
            let mesh = mesh(B, &[A, B]).await?;
            let link = mesh.accept().await.ok_or("b's mesh closed")?;
            answer(&link, &kept).await?;
            link.closed().await;
            Ok(())
        }
    });
    sim.client("a", async move {
        let mesh = mesh(A, &[A, B]).await?;
        let link = mesh.dial(B, Alpn::Link).await?;
        assert_eq!((link.dialled_by(), link.networks()), (A, ["n1".to_string()].into()));
        let (mut w, mut r) = link.open_bi().await?;
        let sent = messages();
        let reading = tokio::spawn(async move {
            let mut back = Vec::new();
            while let Some(m) = take(&mut r).await? {
                back.push(m);
            }
            std::io::Result::Ok(back)
        });
        for m in &sent {
            put(&mut w, m).await?;
        }
        w.shutdown().await?;
        let back = reading.await??;
        assert_eq!(back, sent, "seed {seed}: every message back, whole and in order");
        link.close(Close::Done);
        Ok(())
    });
    sim.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    let heard = kept.sink.all();
    assert_eq!(heard.len(), messages().len(), "seed {seed}");
    for (i, m) in heard.iter().enumerate() {
        assert_eq!((m.kind, m.from.as_str(), &m.body["n"]), (Kind::Message, A.to_string().as_str(), &json!(i)), "seed {seed}");
    }
}

#[test]
fn a_seed_round_trips_every_message_losslessly() {
    round_trip(0x5eed);
}

#[test]
fn the_mesh_refuses_a_non_member_on_a_dial_with_no_hooks() {
    // The simulated network runs none of iroh's hooks and checks nothing:
    // the mesh's accept path is all that stands between c and b.
    let mut sim = sim(7);
    let handed = Arc::new(AtomicUsize::new(0));
    let count = handed.clone();
    sim.host("b", move || {
        let count = count.clone();
        async move {
            let mesh = mesh(B, &[A, B]).await?;
            while let Some(_link) = mesh.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    });
    sim.client("c", async {
        // c holds a chain that has b in it; b's has no c.
        let mesh = mesh(C, &[B, C]).await?;
        let link = mesh.dial(B, Alpn::Link).await?;
        assert_eq!(link.closed().await, Some(Close::Refused));
        Ok(())
    });
    let at_a = handed.clone();
    sim.client("a", async move {
        let mesh = mesh(A, &[A, B]).await?;
        let link = mesh.dial(B, Alpn::Link).await?;
        // b handed out a's connection, and only a's.
        while at_a.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!link.is_closed());
        Ok(())
    });
    sim.run().unwrap();
    assert_eq!(handed.load(Ordering::SeqCst), 1, "c's connection never reached b's protocols");
}
