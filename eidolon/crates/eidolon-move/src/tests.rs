//! Two machines in one process, each with its own vault, key store, signing
//! key and X25519 key, joined by `tokio::io::duplex`. The data is made with
//! `Keys::seal` from fixed pseudo-random bytes, so a baseline run tells where
//! every frame starts and ends and the cut tests can stop a side at any of
//! them.

use super::*;
use eidolon_model::{EidolonId, EidolonSpec, Holder, HolderKey, RefusalCode, SeatRecord, SignedSeat, State};
use eidolon_vault::{wrap_key, ChunkName, EidolonKey, KeyRing, Keys, Vault};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wire::{kind, Hello, Iam, Offer, Refused, SeatMsg};
use x25519_dalek::{PublicKey, StaticSecret};
use xenonet_id::codec::{b64, hex, random};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_id::store::MemoryKeyStore;

// ---- machines and data -------------------------------------------------

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        Tmp(std::env::temp_dir().join(format!("eidolon-move-test-{}", hex(&random::<8>().unwrap()))))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Machine {
    name: &'static str,
    signer: SeedSigner,
    key: HolderKey,
    secret: StaticSecret,
    root: Tmp,
    vault: Vault,
    store: MemoryKeyStore,
    _vault_dir: Tmp,
}

impl Machine {
    fn new(name: &'static str, n: u8) -> Machine {
        let signer = SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap();
        let key = HolderKey::from_verifying_key(&signer.public()).unwrap();
        let vault_dir = Tmp::new();
        let vault = Vault::open(&vault_dir.0).unwrap();
        Machine { name, signer, key, secret: StaticSecret::from([n.wrapping_add(100); 32]), root: Tmp::new(), vault, store: MemoryKeyStore::new(), _vault_dir: vault_dir }
    }

    fn ring(&self) -> KeyRing<'_> {
        KeyRing::new(&self.store)
    }

    fn holder(&self) -> Holder {
        Holder { name: self.name.into(), sign: self.key, seal: PublicKey::from(&self.secret).to_bytes() }
    }

    fn holding(&self, eid: &EidolonId) -> Holding {
        Holding::load(&self.root.0, eid).unwrap()
    }

    /// Whether this machine could open the eidolon now: a holding whose seat
    /// is its own, not frozen, and the key to read it with.
    fn can_open(&self, eid: &EidolonId) -> bool {
        Holding::exists(&self.root.0, eid) && self.holding(eid).may_open(&self.key).is_ok() && self.ring().get(&eid.to_string()).unwrap().is_some()
    }
}

fn spec_of(machines: &[&Machine]) -> EidolonSpec {
    EidolonSpec { id: EidolonId::from_bytes([5; 16]), wall: "wall_work".parse().unwrap(), holders: machines.iter().map(|m| m.holder()).collect(), facets: vec![] }
}

fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn eidolon_key() -> EidolonKey {
    EidolonKey::from_bytes([9; 32])
}

fn head_of(chunks: &[ChunkName]) -> Vec<u8> {
    let mut head = b"head ".to_vec();
    for c in chunks {
        head.extend_from_slice(&c.0);
    }
    head
}

/// A new eidolon on `m` with `n` chunks of `size` bytes.
fn originate(m: &Machine, spec: &EidolonSpec, n: usize, size: usize) -> Holding {
    let keys = Keys::new(&eidolon_key()).unwrap();
    let mut names = Vec::new();
    for i in 0..n {
        let (name, sealed) = keys.seal(&bytes(i as u64, size));
        m.vault.put(&name, &sealed).unwrap();
        names.push(name);
    }
    m.ring().put(&spec.id.to_string(), &eidolon_key()).unwrap();
    let seat = seat_first(spec, &m.signer).unwrap();
    Holding::create(&m.root.0, spec.clone(), seat, head_of(&names), names).unwrap()
}

struct World {
    a: Machine,
    b: Machine,
    spec: EidolonSpec,
    holding: Holding,
}

fn world(chunks: usize, size: usize) -> World {
    let (a, b) = (Machine::new("desk", 1), Machine::new("tower", 2));
    let spec = spec_of(&[&a, &b]);
    let holding = originate(&a, &spec, chunks, size);
    World { a, b, spec, holding }
}

// ---- a stream that records, cuts and damages ----------------------------

#[derive(Clone, Copy, Default)]
struct Cut {
    /// The stream fails once this many bytes were written.
    after: Option<usize>,
    /// One byte at this offset of what is written is changed.
    flip: Option<usize>,
}

struct Wire<S> {
    inner: S,
    cut: Cut,
    written: usize,
    tap: Arc<Mutex<Vec<u8>>>,
}

impl<S> Wire<S> {
    fn new(inner: S, cut: Cut) -> (Wire<S>, Arc<Mutex<Vec<u8>>>) {
        let tap = Arc::new(Mutex::new(Vec::new()));
        (Wire { inner, cut, written: 0, tap: tap.clone() }, tap)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Wire<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Wire<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut data = buf;
        if let Some(limit) = this.cut.after {
            if this.written >= limit {
                return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
            }
            data = &buf[..buf.len().min(limit - this.written)];
        }
        let mut data = data.to_vec();
        if let Some(at) = this.cut.flip {
            if (this.written..this.written + data.len()).contains(&at) {
                data[at - this.written] ^= 1;
            }
        }
        match Pin::new(&mut this.inner).poll_write(cx, &data) {
            Poll::Ready(Ok(n)) => {
                this.tap.lock().unwrap().extend_from_slice(&data[..n]);
                this.written += n;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// (type, start, end) of each frame in what one side wrote.
fn frames(written: &[u8]) -> Vec<(u8, usize, usize)> {
    let (mut out, mut at) = (Vec::new(), 0);
    while at + 5 <= written.len() {
        let len = u32::from_be_bytes(written[at..at + 4].try_into().unwrap()) as usize;
        out.push((written[at + 4], at, at + 4 + len));
        at += 4 + len;
    }
    out
}

struct Run {
    sender: Result<MoveStats, MoveError>,
    receiver: Result<MoveStats, MoveError>,
    sent: Vec<u8>,
    answered: Vec<u8>,
}

async fn go(from: &Machine, to: &Machine, holding: &mut Holding, sender_cut: Cut, receiver_cut: Cut) -> Run {
    let (x, y) = tokio::io::duplex(64 * 1024);
    let ((sx, sent), (ry, answered)) = (Wire::new(x, sender_cut), Wire::new(y, receiver_cut));
    let (ring_from, ring_to) = (from.ring(), to.ring());
    let (sender, receiver) = tokio::join!(send(sx, holding, &from.vault, &ring_from, &from.signer), receive(ry, &to.root.0, &to.vault, &ring_to, &to.signer, &to.key, &to.secret));
    let (sent, answered) = (sent.lock().unwrap().clone(), answered.lock().unwrap().clone());
    Run { sender, receiver, sent, answered }
}

async fn plain(from: &Machine, to: &Machine, holding: &mut Holding) -> Run {
    go(from, to, holding, Cut::default(), Cut::default()).await
}

fn refusal_code<T: std::fmt::Debug>(r: &Result<T, MoveError>) -> Option<RefusalCode> {
    match r {
        Err(MoveError::Refused(x) | MoveError::RefusedByPeer(x)) => Some(x.code),
        _ => None,
    }
}

// ---- a sender and a receiver written by hand, for refusals ----------------

fn seat(spec: &EidolonSpec, generation: u64, holder: &Machine, previous: Option<&Machine>, signed_by: &Machine) -> SignedSeat {
    let record = SeatRecord { eidolon: spec.id, generation, holder: holder.key, previous: previous.map(|p| p.key), at: "2026-01-02T03:04:05Z".into() };
    SignedSeat::sign(record, &signed_by.signer).unwrap()
}

/// A sender that offers no chunks and hands over `next` with the key wrapped
/// to `to`. Returns the frame it got back.
async fn fake_send<S: AsyncRead + AsyncWrite + Unpin + Send>(
    mut s: S,
    spec: &EidolonSpec,
    offered: &SignedSeat,
    next: &SignedSeat,
    to: &Machine,
) -> Result<(u8, Vec<u8>), MoveError> {
    let hello = Hello { protocol: wire::PROTOCOL.into(), eidolon: spec.id, challenge: b64(&[3; 32]) };
    wire::write_json(&mut s, kind::HELLO, &hello).await?;
    wire::read_expect(&mut s, kind::IAM).await?;
    wire::write_json(&mut s, kind::OFFER, &Offer { spec: spec.clone(), seat: offered.clone(), head: b64(b"h"), chunks: 0 }).await?;
    wire::write_names(&mut s, &[]).await?;
    wire::read_names(&mut s, 0).await?;
    wire::write_frame(&mut s, kind::SENT, &[]).await?;
    let key = wrap_key(&eidolon_key(), PublicKey::from(&to.secret).to_bytes(), &wire::wrap_context(&spec.id, next.record.generation))?;
    wire::write_json(&mut s, kind::SEAT, &SeatMsg { next: next.clone(), key }).await?;
    let (got, body, _) = wire::read_frame(&mut s).await?;
    Ok((got, body))
}

async fn receive_from_fake(
    to: &Machine,
    fake: impl std::future::Future<Output = Result<(u8, Vec<u8>), MoveError>>,
    stream: tokio::io::DuplexStream,
) -> (Result<MoveStats, MoveError>, Result<(u8, Vec<u8>), MoveError>) {
    let ring = to.ring();
    tokio::join!(receive(stream, &to.root.0, &to.vault, &ring, &to.signer, &to.key, &to.secret), fake)
}

fn refused_frame(frame: &Result<(u8, Vec<u8>), MoveError>) -> Refused {
    let (got, body) = frame.as_ref().unwrap();
    assert_eq!(*got, kind::REFUSED, "the receiver should have refused");
    wire::parse(body).unwrap()
}

// ---- refusals, each before the passing path --------------------------------

#[tokio::test]
async fn a_receiver_off_the_holder_list_is_refused_by_the_sender() {
    let mut w = world(3, 4096);
    let stranger = Machine::new("stranger", 3);
    let run = plain(&w.a, &stranger, &mut w.holding).await;
    assert_eq!(refusal_code(&run.sender), Some(RefusalCode::NotAHolder));
    assert_eq!(refusal_code(&run.receiver), Some(RefusalCode::NotAHolder), "the stranger hears why");
    assert!(w.holding.pending.is_none(), "nothing was handed over");
    assert!(w.a.can_open(&w.spec.id));
    assert!(!Holding::exists(&stranger.root.0, &w.spec.id));
    assert!(stranger.ring().get(&w.spec.id.to_string()).unwrap().is_none());
    assert!(!frames(&run.sent).iter().any(|f| f.0 == kind::SEAT || f.0 == kind::CHUNK), "no seat and no chunk was sent");
}

#[tokio::test]
async fn a_receiver_that_cannot_prove_its_key_is_refused() {
    let mut w = world(3, 4096);
    let stranger = Machine::new("stranger", 3);
    let (x, y) = tokio::io::duplex(64 * 1024);
    let fake = async move {
        let mut y = y;
        let (body, _) = wire::read_expect(&mut y, kind::HELLO).await?;
        let hello: Hello = wire::parse(&body)?;
        let challenge = wire::challenge_of(&hello)?;
        // The listed key, a signature by another machine.
        let proof = wire::attest(&stranger.signer, &challenge, &hello.eidolon, "iam", 0)?;
        wire::write_json(&mut y, kind::IAM, &Iam { holder: w.b.key, proof }).await?;
        wire::read_frame(&mut y).await.map(|(k, b, _)| (k, b))
    };
    let ring = w.a.ring();
    let (sender, frame) = tokio::join!(send(x, &mut w.holding, &w.a.vault, &ring, &w.a.signer), fake);
    assert_eq!(refusal_code(&sender), Some(RefusalCode::NotAHolder));
    assert!(refused_frame(&frame).refusal.message.contains("prove"));
    assert!(w.holding.pending.is_none());
    assert!(w.a.can_open(&w.spec.id));
}

#[tokio::test]
async fn a_repeated_or_skipping_seat_is_refused_as_stale() {
    let mut w = world(3, 4096);
    let moved = plain(&w.a, &w.b, &mut w.holding).await;
    assert!(moved.sender.is_ok() && moved.receiver.is_ok());
    // b holds generation 2. A sender (a, or anyone who kept its key) offers
    // the same seat again, or one that skips ahead.
    for generation in [2, 4, 1] {
        let next = seat(&w.spec, generation, &w.b, Some(&w.a), &w.a);
        let (x, y) = tokio::io::duplex(64 * 1024);
        let offered = seat(&w.spec, 1, &w.a, None, &w.a);
        let (receiver, frame) = receive_from_fake(&w.b, fake_send(x, &w.spec, &offered, &next, &w.b), y).await;
        assert_eq!(refusal_code(&receiver), Some(RefusalCode::StaleGeneration), "generation {generation}");
        let refused = refused_frame(&frame);
        assert_eq!(refused.refusal.code, RefusalCode::StaleGeneration);
        // b signs a refusal only for a generation it does not hold.
        assert_eq!(refused.proof.is_none(), generation <= 2, "generation {generation}");
        assert_eq!(w.b.holding(&w.spec.id).seat.record.generation, 2, "b kept what it had");
    }
    assert!(w.b.can_open(&w.spec.id));
}

#[tokio::test]
async fn a_chunk_changed_in_transit_is_refused_and_the_receiver_keeps_nothing() {
    // The first run only tells where the first chunk frame is.
    let mut probe = world(5, 4096);
    let baseline = plain(&probe.a, &probe.b, &mut probe.holding).await;
    let chunk = frames(&baseline.sent).into_iter().find(|f| f.0 == kind::CHUNK).unwrap();

    let mut w = world(5, 4096);
    let run = go(&w.a, &w.b, &mut w.holding, Cut { flip: Some(chunk.1 + 4 + 1 + 32 + 10), after: None }, Cut::default()).await;
    assert_eq!(refusal_code(&run.receiver), Some(RefusalCode::ChangedChunk));
    assert_eq!(refusal_code(&run.sender), Some(RefusalCode::ChangedChunk));
    assert!(w.b.ring().get(&w.spec.id.to_string()).unwrap().is_none(), "no key kept");
    assert!(!Holding::exists(&w.b.root.0, &w.spec.id), "no seat kept");
    assert!(w.holding.pending.is_none(), "the receiver's signed refusal undoes the pending move");
    assert!(w.a.can_open(&w.spec.id), "the sender still holds it");
    assert!(!w.b.can_open(&w.spec.id));

    // The damaged chunk was dropped, so running again fetches just it.
    assert_eq!(w.b.vault.list().unwrap().len(), 4);
    let again = plain(&w.a, &w.b, &mut w.holding).await;
    assert_eq!(again.receiver.as_ref().unwrap().chunks_sent, 1);
    assert!(again.sender.is_ok());
    assert!(w.b.can_open(&w.spec.id) && !w.a.can_open(&w.spec.id));
}

#[tokio::test]
async fn a_different_spec_for_the_same_eidolon_is_refused() {
    let mut w = world(3, 4096);
    assert!(plain(&w.a, &w.b, &mut w.holding).await.receiver.is_ok());
    let offered = seat(&w.spec, 2, &w.b, Some(&w.a), &w.a);
    let next = seat(&w.spec, 3, &w.b, Some(&w.a), &w.b);

    let mut other_wall = w.spec.clone();
    other_wall.wall = "wall_play".parse().unwrap();
    let mut other_holders = w.spec.clone();
    other_holders.holders.reverse();
    for (spec, code) in [(other_wall, RefusalCode::WrongWall), (other_holders, RefusalCode::NotAHolder)] {
        let (x, y) = tokio::io::duplex(64 * 1024);
        let (receiver, _) = receive_from_fake(&w.b, fake_send(x, &spec, &offered, &next, &w.b), y).await;
        assert_eq!(refusal_code(&receiver), Some(code));
        assert_eq!(w.b.holding(&w.spec.id).spec, w.spec, "b kept its spec");
    }
}

#[tokio::test]
async fn a_sender_that_is_not_the_seat_holder_refuses_to_send() {
    let mut w = world(3, 4096);
    assert!(plain(&w.a, &w.b, &mut w.holding).await.sender.is_ok());
    // a is retired and the seat is b's.
    let run = plain(&w.a, &w.b, &mut w.holding).await;
    assert_eq!(refusal_code(&run.sender), Some(RefusalCode::Retired));
    assert!(run.sent.is_empty(), "nothing was written");

    // Even with the state forgotten, the seat is not its own.
    w.holding.state = State::Sealed;
    let run = plain(&w.a, &w.b, &mut w.holding).await;
    assert_eq!(refusal_code(&run.sender), Some(RefusalCode::SecondSeat));
    assert_eq!(run.sent.len(), 0);
}

#[tokio::test]
async fn a_holding_with_an_unfinished_move_will_not_go_to_another_machine() {
    let (a, b, c) = (Machine::new("desk", 1), Machine::new("tower", 2), Machine::new("lap", 3));
    let spec = spec_of(&[&a, &b, &c]);
    let mut holding = originate(&a, &spec, 3, 4096);
    // b takes it, and its acknowledgement never gets back.
    let ack_start = {
        let mut probe = originate_probe();
        frames(&plain(&probe.a, &probe.b, &mut probe.holding).await.answered).last().unwrap().1
    };
    let run = go(&a, &b, &mut holding, Cut::default(), Cut { after: Some(ack_start), flip: None }).await;
    assert!(run.sender.is_err() && run.receiver.is_err());
    assert_eq!(holding.pending.map(|p| p.generation), Some(2));
    assert!(holding.may_open(&a.key).is_err(), "frozen");
    let to_c = plain(&a, &c, &mut holding).await;
    assert!(matches!(to_c.sender, Err(MoveError::MovePending { .. })), "{:?}", to_c.sender);
    assert!(!Holding::exists(&c.root.0, &spec.id));
}

fn originate_probe() -> World {
    world(3, 4096)
}

// ---- the passing path ------------------------------------------------------

#[tokio::test]
async fn a_move_hands_the_eidolon_to_the_receiver_and_closes_it_on_the_sender() {
    let mut w = world(20, 100 * 1024);
    let eid = w.spec.id;
    let run = plain(&w.a, &w.b, &mut w.holding).await;
    let (sent, got) = (run.sender.unwrap(), run.receiver.unwrap());
    println!("move 1: offered {}, sent {} chunks, {} bytes on the wire, {:?}", sent.chunks_offered, sent.chunks_sent, sent.bytes_sent, sent.elapsed);
    assert_eq!((sent.chunks_offered, sent.chunks_sent, got.chunks_sent), (20, 20, 20));
    assert!(sent.bytes_sent > 20 * 100 * 1024 / 2, "a hundred KiB of random bytes a chunk does not shrink much");

    // The receiver opens everything with the key it unwrapped.
    let key = w.b.ring().get(&eid.to_string()).unwrap().unwrap();
    assert_eq!(key, eidolon_key());
    let keys = Keys::new(&key).unwrap();
    let held = w.b.holding(&eid);
    assert_eq!(held.chunks.len(), 20);
    let opened: BTreeSet<Vec<u8>> = held.chunks.iter().map(|name| keys.open(name, &w.b.vault.get(name).unwrap()).unwrap()).collect();
    assert_eq!(opened, (0..20).map(|i| bytes(i, 100 * 1024)).collect());
    assert_eq!(held.head, w.holding.head);
    assert_eq!(held.state, State::Sealed);
    assert_eq!(held.seat.record.generation, 2);
    assert_eq!((held.seat.record.holder, held.seat.record.previous), (w.b.key, Some(w.a.key)));

    // The sender cannot open it any more.
    assert!(w.a.ring().get(&eid.to_string()).unwrap().is_none());
    let kept = Holding::load(&w.a.root.0, &eid).unwrap();
    assert_eq!(kept.state, State::Retired);
    assert_eq!(kept.seat, held.seat);
    assert!(kept.pending.is_none() && w.holding.pending.is_none());
    assert_eq!(w.holding.may_open(&w.a.key).unwrap_err().code, RefusalCode::Retired);
    assert_eq!(kept.may_open(&w.a.key).unwrap_err().code, RefusalCode::Retired);
    assert!(held.may_open(&w.b.key).is_ok());
    assert!(!w.a.can_open(&eid) && w.b.can_open(&eid));
    // Its chunks stay, closed.
    assert_eq!(w.a.vault.list().unwrap().len(), 20);
}

#[tokio::test]
async fn moving_back_after_one_chunk_changed_sends_one_chunk() {
    let mut w = world(20, 100 * 1024);
    let eid = w.spec.id;
    let first = plain(&w.a, &w.b, &mut w.holding).await;
    let first_sent = first.sender.unwrap();

    // b works on it: one chunk changes.
    let keys = Keys::new(&w.b.ring().get(&eid.to_string()).unwrap().unwrap()).unwrap();
    let (new_name, sealed) = keys.seal(&bytes(999, 100 * 1024));
    w.b.vault.put(&new_name, &sealed).unwrap();
    let mut held = w.b.holding(&eid);
    let mut chunks = held.chunks.clone();
    chunks[7] = new_name;
    held.record_head(head_of(&chunks), chunks).unwrap();

    let back = plain(&w.b, &w.a, &mut held).await;
    let (sent, got) = (back.sender.unwrap(), back.receiver.unwrap());
    println!("move 1 (20 chunks): {} bytes on the wire", first_sent.bytes_sent);
    println!("move 2 (1 of 20 changed): offered {}, sent {} chunk, {} bytes on the wire, {:?}", sent.chunks_offered, sent.chunks_sent, sent.bytes_sent, sent.elapsed);
    assert_eq!((sent.chunks_offered, sent.chunks_sent, got.chunks_sent), (20, 1, 1));
    assert!(sent.bytes_sent < first_sent.bytes_sent / 10);

    assert!(w.a.can_open(&eid) && !w.b.can_open(&eid));
    let home = w.a.holding(&eid);
    assert_eq!((home.seat.record.generation, home.state.clone()), (3, State::Sealed));
    assert_eq!(home.seat.record.previous, Some(w.b.key));
    let keys = Keys::new(&w.a.ring().get(&eid.to_string()).unwrap().unwrap()).unwrap();
    for name in &home.chunks {
        keys.open(name, &w.a.vault.get(name).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn a_cut_transfer_resumes_with_only_the_rest() {
    let mut probe = world(20, 8 * 1024);
    let baseline = frames(&plain(&probe.a, &probe.b, &mut probe.holding).await.sent);
    let chunk_frames: Vec<_> = baseline.iter().filter(|f| f.0 == kind::CHUNK).collect();
    assert_eq!(chunk_frames.len(), 20);

    let mut w = world(20, 8 * 1024);
    let first = go(&w.a, &w.b, &mut w.holding, Cut { after: Some(chunk_frames[8].2), flip: None }, Cut::default()).await;
    assert!(matches!(first.sender, Err(MoveError::Cut)), "{:?}", first.sender);
    assert!(first.receiver.is_err());
    assert_eq!(w.b.vault.list().unwrap().len(), 9, "what arrived is kept");
    assert!(w.a.can_open(&w.spec.id), "nothing was handed over");

    let second = plain(&w.a, &w.b, &mut w.holding).await;
    let (sent, got) = (second.sender.unwrap(), second.receiver.unwrap());
    println!("resume: the first run delivered 9 chunks; the second sent {} of {}", sent.chunks_sent, sent.chunks_offered);
    assert_eq!((sent.chunks_offered, sent.chunks_sent, got.chunks_sent), (20, 11, 11));
    assert!(w.b.can_open(&w.spec.id));
}

// ---- kill points -------------------------------------------------------------

/// Cuts a side at every frame boundary and in the middle of every frame,
/// then checks that at most one machine could open the eidolon, that the
/// person can settle what is left, and that a move after that completes.
async fn kill_matrix(sender_side: bool) {
    let mut probe = world(5, 4096);
    let base = plain(&probe.a, &probe.b, &mut probe.holding).await;
    let theirs = frames(if sender_side { &base.sent } else { &base.answered });
    let mut points = Vec::new();
    for &(k, start, end) in &theirs {
        points.push((k, end));
        points.push((k, start + (end - start) / 2));
    }
    // Every kind of frame is among them.
    let kinds: BTreeSet<u8> = theirs.iter().map(|f| f.0).collect();
    let expect: &[u8] = if sender_side { &[kind::HELLO, kind::OFFER, kind::NAMES, kind::CHUNK, kind::SENT, kind::SEAT] } else { &[kind::IAM, kind::NAMES, kind::ACK] };
    assert!(expect.iter().all(|k| kinds.contains(k)), "{kinds:?}");

    for (k, at) in points {
        let mut w = world(5, 4096);
        let eid = w.spec.id;
        let cut = Cut { after: Some(at), flip: None };
        let run = if sender_side { go(&w.a, &w.b, &mut w.holding, cut, Cut::default()).await } else { go(&w.a, &w.b, &mut w.holding, Cut::default(), cut).await };
        let which = format!("{} cut at {at} in a frame of type {k}", if sender_side { "sender" } else { "receiver" });
        let completed = run.sender.is_ok() && run.receiver.is_ok();
        assert!(completed || run.sender.is_err() || run.receiver.is_err(), "{which}");

        // What is on disk now is all there is: both machines start over.
        let mut a_now = w.a.holding(&eid);
        let (a_open, b_open) = (w.a.can_open(&eid), w.b.can_open(&eid));
        assert!(!(a_open && b_open), "two machines can open it: {which}");

        // A copy frozen with a pending move is not open; the person settles.
        if a_now.pending.is_some() {
            assert!(!a_open, "{which}");
            if Holding::exists(&w.b.root.0, &eid) {
                let taken = w.b.holding(&eid).seat;
                a_now.confirm_moved(&w.a.ring(), &taken).unwrap();
            } else {
                a_now.abandon_pending().unwrap();
            }
        }
        let (a_open, b_open) = (w.a.can_open(&eid), w.b.can_open(&eid));
        assert!(a_open ^ b_open, "exactly one machine can open it after settling: {which} (desk {a_open}, tower {b_open})");

        // A second move, uncut, from whoever holds it.
        let again = if a_open { plain(&w.a, &w.b, &mut a_now).await } else { plain(&w.b, &w.a, &mut w.b.holding(&eid)).await };
        assert!(again.sender.is_ok() && again.receiver.is_ok(), "{which}: {:?} / {:?}", again.sender, again.receiver);
        let (a_after, b_after) = (w.a.can_open(&eid), w.b.can_open(&eid));
        assert!(a_after != a_open && b_after != b_open, "the eidolon moved: {which}");
    }
}

#[tokio::test]
async fn a_sender_killed_at_any_frame_leaves_exactly_one_machine_able_to_open() {
    kill_matrix(true).await;
}

#[tokio::test]
async fn a_receiver_killed_at_any_frame_leaves_exactly_one_machine_able_to_open() {
    kill_matrix(false).await;
}

#[tokio::test]
async fn a_move_run_again_after_a_lost_acknowledgement_is_refused_and_settled_by_the_seat() {
    let mut probe = world(3, 4096);
    let ack = *frames(&plain(&probe.a, &probe.b, &mut probe.holding).await.answered).last().unwrap();
    let mut w = world(3, 4096);
    let eid = w.spec.id;
    let lost = go(&w.a, &w.b, &mut w.holding, Cut::default(), Cut { after: Some(ack.1), flip: None }).await;
    assert!(lost.sender.is_err());
    assert!(w.b.can_open(&eid), "b took it");
    assert!(!w.a.can_open(&eid), "a stays closed");

    // Running it again is stale: b already has the next generation.
    let again = plain(&w.a, &w.b, &mut w.holding).await;
    assert_eq!(refusal_code(&again.receiver), Some(RefusalCode::StaleGeneration));
    assert!(w.holding.pending.is_some(), "a cannot tell, so it stays closed");

    // The wrong seat does not settle it; b's own does.
    let mut a = w.a.holding(&eid);
    let wrong = seat(&w.spec, 2, &w.a, Some(&w.b), &w.a);
    assert!(a.confirm_moved(&w.a.ring(), &wrong).is_err());
    a.confirm_moved(&w.a.ring(), &w.b.holding(&eid).seat).unwrap();
    assert_eq!(a.state, State::Retired);
    assert!(a.pending.is_none());
    assert!(w.a.ring().get(&eid.to_string()).unwrap().is_none());
    assert!(matches!(a.abandon_pending(), Err(MoveError::NoPending)));
}

// ---- small things ------------------------------------------------------------

#[test]
fn a_first_seat_needs_a_listed_machine() {
    let (a, b) = (Machine::new("desk", 1), Machine::new("tower", 2));
    let spec = spec_of(&[&a]);
    assert!(seat_first(&spec, &a.signer).is_ok());
    assert!(matches!(seat_first(&spec, &b.signer), Err(MoveError::Refused(r)) if r.code == RefusalCode::NotAHolder));
}

#[test]
fn a_holding_is_written_to_files_and_read_back() {
    let a = Machine::new("desk", 1);
    let spec = spec_of(&[&a]);
    let held = originate(&a, &spec, 2, 1024);
    let back = Holding::load(&a.root.0, &spec.id).unwrap();
    assert_eq!((&back.spec, &back.seat, &back.head, &back.chunks, &back.state), (&held.spec, &held.seat, &held.head, &held.chunks, &State::Sealed));
    for file in ["spec.json", "seat.json", "head", "chunks.json", "state.json"] {
        assert!(Path::new(&a.root.0).join(spec.id.to_string()).join(file).is_file(), "{file}");
    }
    assert!(!Path::new(&a.root.0).join(spec.id.to_string()).join("pending.json").exists());
    assert!(Holding::create(&a.root.0, back.spec.clone(), back.seat.clone(), vec![], vec![]).is_err(), "not twice");
    assert!(matches!(Holding::load(&a.root.0, &EidolonId::from_bytes([1; 16])), Err(MoveError::BadFile { .. })));
}

#[tokio::test]
async fn a_frame_over_the_limit_is_refused_on_both_ends() {
    let (mut x, mut y) = tokio::io::duplex(1024);
    assert!(matches!(wire::write_frame(&mut x, kind::CHUNK, &vec![0u8; wire::MAX_FRAME]).await, Err(MoveError::Protocol(_))));
    tokio::io::AsyncWriteExt::write_all(&mut x, &((wire::MAX_FRAME as u32) + 1).to_be_bytes()).await.unwrap();
    assert!(matches!(wire::read_frame(&mut y).await, Err(MoveError::Protocol(_))));
}

#[tokio::test]
async fn a_long_list_of_names_goes_in_several_frames() {
    let names: Vec<ChunkName> = (0..wire::NAMES_PER_FRAME * 2 + 5).map(|i| ChunkName([(i % 251) as u8; 32])).collect();
    let (mut x, mut y) = tokio::io::duplex(8 * 1024 * 1024);
    wire::write_names(&mut x, &names).await.unwrap();
    let (back, _) = wire::read_names(&mut y, names.len() as u64).await.unwrap();
    assert_eq!(back, names);
    wire::write_names(&mut x, &[]).await.unwrap();
    assert_eq!(wire::read_names(&mut y, 0).await.unwrap().0.len(), 0);
}
