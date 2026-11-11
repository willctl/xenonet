//! The transport on a simulated network, for the tests alone (turmoil is a
//! dev-dependency, so none of this ships): a `Dial` over turmoil's TCP,
//! where hosts, partitions, loss and latency run on one seeded clock, and
//! the in-memory `Persist` and `Sink` a machine keeps beside it.
//!
//! iroh can't run on turmoil's sockets, so this `Dial` stands in for it
//! with what the `Mesh` needs of a transport: a connection to a peer by
//! its endpoint id, any number of bi and uni streams on it, a close code
//! both ends see, and ALPNs offered or not. It authenticates nothing (the
//! hello names its sender) and runs none of iroh's hooks, which is what
//! the mesh's own admission is tested against. Real iroh is tested on
//! loopback (`endpoint`) and in the VM lab.
//!
//! On the wire, each connection is a TCP stream that opens with a hello
//! (`conn`) and carries only its close; each QUIC-like stream is a TCP
//! stream of its own whose hello names the connection.

use crate::dial::{Alpn, Close, Dial, Fut, PeerId, Reader, Wire, Writer};
use crate::persist;
use crate::{NetError, R};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use turmoil::net::{TcpListener, TcpStream};
use xenonet_model::ops::{Incoming, Sink};
use xenonet_os::sync::Locked;

/// The port every simulated machine listens on.
pub const PORT: u16 = 4919;

const CONN: u8 = 1;
const BI: u8 = 2;
const UNI: u8 = 3;
// The hello: kind, the connection's dialler, its ALPN, its number.
const HELLO: usize = 1 + 32 + 1 + 8;

fn alpn_byte(a: Alpn) -> u8 {
    match a {
        Alpn::Link => 0,
        Alpn::Join => 1,
        Alpn::Svc => 2,
    }
}

fn hello(kind: u8, dialler: PeerId, alpn: Alpn, id: u64) -> [u8; HELLO] {
    let mut h = [0u8; HELLO];
    h[0] = kind;
    h[1..33].copy_from_slice(&dialler.0);
    h[33] = alpn_byte(alpn);
    h[34..].copy_from_slice(&id.to_be_bytes());
    h
}

fn read_hello(h: &[u8; HELLO]) -> Option<(u8, PeerId, Alpn, u64)> {
    let mut dialler = [0u8; 32];
    dialler.copy_from_slice(&h[1..33]);
    let alpn = Alpn::ALL.into_iter().find(|a| alpn_byte(*a) == h[33])?;
    let mut id = [0u8; 8];
    id.copy_from_slice(&h[34..]);
    Some((h[0], PeerId(dialler), alpn, u64::from_be_bytes(id)))
}

fn io(e: std::io::Error) -> NetError {
    NetError::new(502, "unreachable", format!("the simulated network: {e}"))
}

fn ended() -> NetError {
    NetError::new(502, "link-closed", "the connection ended")
}

// A connection's key on both ends: who dialled it, and its number there.
type Key = (PeerId, u64);

/// A connection on the simulated network.
#[derive(Debug)]
pub struct SimWire {
    peer: PeerId,
    host: String,
    alpn: Alpn,
    key: Key,
    bi: tokio::sync::Mutex<mpsc::UnboundedReceiver<(Writer, Reader)>>,
    uni: tokio::sync::Mutex<mpsc::UnboundedReceiver<Reader>>,
    control: Mutex<Option<turmoil::net::tcp::OwnedWriteHalf>>,
    state: watch::Sender<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Open,
    /// Closed, with the code either end closed it with, if any.
    Closed(Option<Close>),
}

impl State {
    fn is_closed(self) -> bool {
        self != State::Open
    }
}

#[derive(Default)]
struct Streams {
    bi: Option<mpsc::UnboundedSender<(Writer, Reader)>>,
    uni: Option<mpsc::UnboundedSender<Reader>>,
}

impl SimWire {
    fn ended(&self, why: Option<Close>) {
        self.state.send_if_modified(|s| {
            if s.is_closed() {
                return false;
            }
            *s = State::Closed(why);
            true
        });
    }

    async fn open(&self, kind: u8) -> R<TcpStream> {
        if self.is_closed() {
            return Err(ended());
        }
        let mut s = TcpStream::connect((self.host.as_str(), PORT)).await.map_err(io)?;
        s.write_all(&hello(kind, self.key.0, self.alpn, self.key.1)).await.map_err(io)?;
        Ok(s)
    }
}

impl Wire for SimWire {
    fn peer(&self) -> PeerId {
        self.peer
    }

    fn alpn(&self) -> Alpn {
        self.alpn
    }

    fn open_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move {
            let (r, w) = self.open(BI).await?.into_split();
            Ok((Box::new(w) as Writer, Box::new(r) as Reader))
        })
    }

    fn accept_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move {
            let mut bi = self.bi.lock().await;
            let mut state = self.state.subscribe();
            tokio::select! {
                s = bi.recv() => s.ok_or_else(ended),
                _ = state.wait_for(|s| s.is_closed()) => Err(ended()),
            }
        })
    }

    fn open_uni(&self) -> Fut<'_, R<Writer>> {
        Box::pin(async move { Ok(Box::new(self.open(UNI).await?) as Writer) })
    }

    fn accept_uni(&self) -> Fut<'_, R<Reader>> {
        Box::pin(async move {
            let mut uni = self.uni.lock().await;
            let mut state = self.state.subscribe();
            tokio::select! {
                s = uni.recv() => s.ok_or_else(ended),
                _ = state.wait_for(|s| s.is_closed()) => Err(ended()),
            }
        })
    }

    fn close(&self, why: Close) {
        if let Some(mut w) = self.control.locked().take() {
            // The close code, then the end of the control stream.
            tokio::spawn(async move {
                let _ = w.write_all(&why.code().to_be_bytes()).await;
                let _ = w.shutdown().await;
            });
        }
        self.ended(Some(why));
    }

    fn closed(&self) -> Fut<'_, Option<Close>> {
        Box::pin(async move {
            let mut state = self.state.subscribe();
            let _ = state.wait_for(|s| s.is_closed()).await;
            match *self.state.borrow() {
                State::Closed(why) => why,
                State::Open => None,
            }
        })
    }

    fn is_closed(&self) -> bool {
        self.state.borrow().is_closed()
    }
}

/// What one machine's dial shares with its listener.
#[derive(Default)]
struct Shared {
    offered: Vec<Alpn>,
    wires: BTreeMap<Key, (Weak<SimWire>, Streams)>,
    // Streams that came before their connection's hello was taken in.
    early: BTreeMap<Key, VecDeque<(u8, TcpStream)>>,
}

/// A machine's `Dial` on the simulated network. `hosts` names each
/// machine's simulated host by its endpoint id.
pub struct SimDial {
    me: PeerId,
    hosts: BTreeMap<PeerId, String>,
    shared: Arc<Mutex<Shared>>,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<Arc<dyn Wire>>>,
    next: AtomicU64,
    listening: tokio::task::JoinHandle<()>,
}

impl SimDial {
    /// Listens on `PORT` of the host this runs on.
    pub async fn bind(me: PeerId, hosts: BTreeMap<PeerId, String>) -> R<SimDial> {
        let listener = TcpListener::bind(("0.0.0.0", PORT)).await.map_err(io)?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (to, from) = mpsc::unbounded_channel();
        let listening = tokio::spawn(listen(listener, hosts.clone(), shared.clone(), to));
        Ok(SimDial { me, hosts, shared, incoming: tokio::sync::Mutex::new(from), next: AtomicU64::new(1), listening })
    }
}

impl Drop for SimDial {
    fn drop(&mut self) {
        self.listening.abort();
    }
}

// A new connection's wire, registered so its streams find it.
fn wire(peer: PeerId, host: String, alpn: Alpn, key: Key, control: TcpStream, shared: &Arc<Mutex<Shared>>) -> Arc<SimWire> {
    let (bi_to, bi) = mpsc::unbounded_channel();
    let (uni_to, uni) = mpsc::unbounded_channel();
    let (r, w) = control.into_split();
    let w = Arc::new(SimWire {
        peer,
        host,
        alpn,
        key,
        bi: tokio::sync::Mutex::new(bi),
        uni: tokio::sync::Mutex::new(uni),
        control: Mutex::new(Some(w)),
        state: watch::channel(State::Open).0,
    });
    let early = {
        let mut s = shared.locked();
        s.wires.insert(key, (Arc::downgrade(&w), Streams { bi: Some(bi_to), uni: Some(uni_to) }));
        s.early.remove(&key).unwrap_or_default()
    };
    for (kind, stream) in early {
        route(shared, key, kind, stream);
    }
    // The control stream says how the other end closed, or just ends.
    let watching = Arc::downgrade(&w);
    let shared = shared.clone();
    tokio::spawn(async move {
        let mut r = r;
        let mut code = [0u8; 4];
        let why = match r.read_exact(&mut code).await {
            Ok(_) => Close::from_code(u64::from(u32::from_be_bytes(code))),
            Err(_) => None,
        };
        if let Some(w) = watching.upgrade() {
            w.control.locked().take();
            w.ended(why);
        }
        shared.locked().wires.remove(&key);
    });
    w
}

// A stream to the connection it names: to its accept queue, or held until
// the connection's hello has been taken in.
fn route(shared: &Arc<Mutex<Shared>>, key: Key, kind: u8, stream: TcpStream) {
    let mut s = shared.locked();
    let Some((w, streams)) = s.wires.get(&key) else {
        s.early.entry(key).or_default().push_back((kind, stream));
        return;
    };
    if w.upgrade().is_none_or(|w| w.is_closed()) {
        return;
    }
    match kind {
        BI => {
            let (r, w) = stream.into_split();
            if let Some(to) = &streams.bi {
                let _ = to.send((Box::new(w) as Writer, Box::new(r) as Reader));
            }
        }
        _ => {
            if let Some(to) = &streams.uni {
                let _ = to.send(Box::new(stream) as Reader);
            }
        }
    }
}

async fn listen(listener: TcpListener, hosts: BTreeMap<PeerId, String>, shared: Arc<Mutex<Shared>>, to: mpsc::UnboundedSender<Arc<dyn Wire>>) {
    while let Ok((mut stream, _)) = listener.accept().await {
        let mut h = [0u8; HELLO];
        if stream.read_exact(&mut h).await.is_err() {
            continue;
        }
        let Some((kind, dialler, alpn, id)) = read_hello(&h) else { continue };
        let key = (dialler, id);
        match kind {
            CONN => {
                // An ALPN not offered fails as iroh's handshake would.
                let offered = shared.locked().offered.contains(&alpn);
                let Some(host) = hosts.get(&dialler).cloned() else { continue };
                if !offered || stream.write_all(&[1]).await.is_err() {
                    continue;
                }
                let w = wire(dialler, host, alpn, key, stream, &shared);
                if to.send(w as Arc<dyn Wire>).is_err() {
                    return;
                }
            }
            BI | UNI => route(&shared, key, kind, stream),
            _ => {}
        }
    }
}

impl Dial for SimDial {
    fn me(&self) -> PeerId {
        self.me
    }

    fn connect(&self, peer: PeerId, alpn: Alpn) -> Fut<'_, R<Arc<dyn Wire>>> {
        Box::pin(async move {
            let host = self.hosts.get(&peer).cloned().ok_or_else(|| NetError::new(502, "unreachable", format!("no address for {peer}")))?;
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let mut s = TcpStream::connect((host.as_str(), PORT)).await.map_err(io)?;
            s.write_all(&hello(CONN, self.me, alpn, id)).await.map_err(io)?;
            let mut ok = [0u8; 1];
            s.read_exact(&mut ok).await.map_err(|_| NetError::new(502, "unreachable", format!("{peer} doesn't answer {alpn:?}")))?;
            Ok(wire(peer, host, alpn, (self.me, id), s, &self.shared) as Arc<dyn Wire>)
        })
    }

    fn incoming(&self) -> Fut<'_, Option<Arc<dyn Wire>>> {
        Box::pin(async move { self.incoming.lock().await.recv().await })
    }

    fn offer(&self, alpns: &[Alpn]) {
        self.shared.locked().offered = alpns.to_vec();
    }

    fn close(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            self.listening.abort();
            let wires: Vec<Arc<SimWire>> = self.shared.locked().wires.values().filter_map(|(w, _)| w.upgrade()).collect();
            for w in wires {
                w.close(Close::Done);
            }
        })
    }
}

/// What reached a machine's engine, in order: the in-memory `Sink`.
#[derive(Default)]
pub struct Heard(Mutex<Vec<Incoming>>);

impl Heard {
    pub fn all(&self) -> Vec<Incoming> {
        self.0.locked().clone()
    }
}

impl Sink for Heard {
    fn receive(&self, incoming: Incoming) {
        self.0.locked().push(incoming);
    }
    fn linked(&self, _up: bool) {}
    fn removed(&self, _host: &str) {}
    fn gap(&self) {}
}

/// What a simulated machine keeps: the in-memory store and engine.
#[derive(Default)]
pub struct Kept {
    pub persist: persist::Memory,
    pub sink: Heard,
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
