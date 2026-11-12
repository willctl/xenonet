//! Connections between machines (phase 5's transport, docs/design/network.md):
//! the `Dial` a transport implements, and the `Mesh` every connection goes
//! through on its way in or out.
//!
//! A machine is its endpoint id (`PeerId`), the Ed25519 public key of its
//! host.key. It holds at most one connection per peer and protocol, shared
//! by every network both are in, and the `Mesh` keeps it that way:
//!
//! - Every connection is checked against `Admission`, dialled or accepted,
//!   whatever the `Dial` underneath (iroh, or turmoil in the tests).
//! - When two machines dial each other at once, both keep the connection
//!   the lower endpoint id dialled and close the other, so they agree
//!   without a word. A peer that dials while an older connection to it
//!   stands (one more than `CROSSING` old) has lost that one, having
//!   restarted or moved, so its new one stays, whoever dialled the old:
//!   the old would otherwise block it until QUIC gave up on it.
//! - A connection carries the networks both machines are in by this
//!   machine's chains. When a member is removed from one of them, that
//!   network's traffic stops on the connection (`Link::check`), and the
//!   connection closes only when no shared network is left.
//!
//! The implementation over iroh is in `endpoint`.

use crate::admission::Admission;
use crate::{NetError, R};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncRead, AsyncWrite};
use xenonet_os::sync::{Locked, ReadWrite};

/// A future a `Dial` or a `Wire` returns.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// One direction of a stream.
pub type Reader = Box<dyn AsyncRead + Send + Unpin>;
pub type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// A machine on the wire: its endpoint id, the Ed25519 public key of its
/// host.key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub [u8; 32]);

impl fmt::Display for PeerId {
    // The first five bytes, as the log names a machine.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0[..5] {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({self})")
    }
}

/// The protocols machines speak to each other, by their ALPN: XenoNet's
/// three, and another program's (`App`), which whoever runs a mesh names
/// in its `Admission` (Xenolith's `xenolith/sync/2` on its own endpoint,
/// or forwarded over XenoNet's).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Alpn {
    /// Members' feeds, messages and presence.
    Link,
    /// A machine joining a network with an invite.
    Join,
    /// Requests to the services members publish.
    Svc,
    /// Another program's protocol, by its bytes. Made with `Alpn::named`,
    /// so XenoNet's own never come as `App`.
    App(&'static [u8]),
}

impl Alpn {
    /// XenoNet's own protocols.
    pub const ALL: [Alpn; 3] = [Alpn::Link, Alpn::Join, Alpn::Svc];

    pub const fn bytes(self) -> &'static [u8] {
        match self {
            Alpn::Link => b"xenonet/link/2",
            Alpn::Join => b"xenonet/join/2",
            Alpn::Svc => b"xenonet/svc/2",
            Alpn::App(b) => b,
        }
    }

    /// One of XenoNet's own protocols, by its bytes.
    pub fn parse(alpn: &[u8]) -> Option<Alpn> {
        Alpn::ALL.into_iter().find(|a| a.bytes() == alpn)
    }

    /// The protocol `alpn` names: XenoNet's own when it is one of them,
    /// another program's otherwise.
    pub fn named(alpn: &'static [u8]) -> Alpn {
        Alpn::parse(alpn).unwrap_or(Alpn::App(alpn))
    }

    /// Whether this is one of XenoNet's own protocols.
    pub fn is_xenonet(self) -> bool {
        !matches!(self, Alpn::App(_))
    }

    /// The protocol's name, for the log and errors.
    pub fn name(self) -> &'static str {
        std::str::from_utf8(self.bytes()).unwrap_or("?")
    }
}

/// Which machine dialled a connection, seen from this one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// This machine dialled it.
    Dialled,
    /// The peer dialled it.
    Accepted,
}

/// Why a connection was closed, as the code both ends see.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Close {
    /// Closed when done with.
    Done,
    /// Refused on admission.
    Refused,
    /// The other of two crossing connections was kept.
    Crossed,
    /// No network is shared any more.
    Removed,
}

impl Close {
    pub const fn code(self) -> u32 {
        match self {
            Close::Done => 0,
            Close::Refused => 1,
            Close::Crossed => 2,
            Close::Removed => 3,
        }
    }

    pub fn from_code(code: u64) -> Option<Close> {
        [Close::Done, Close::Refused, Close::Crossed, Close::Removed].into_iter().find(|c| u64::from(c.code()) == code)
    }

    pub const fn reason(self) -> &'static [u8] {
        match self {
            Close::Done => b"done",
            Close::Refused => b"refused",
            Close::Crossed => b"crossed",
            Close::Removed => b"removed",
        }
    }
}

/// One connection, as a transport has it.
pub trait Wire: Send + Sync + fmt::Debug {
    /// The machine at the other end, as the handshake authenticated it.
    fn peer(&self) -> PeerId;
    fn alpn(&self) -> Alpn;
    fn open_bi(&self) -> Fut<'_, R<(Writer, Reader)>>;
    fn accept_bi(&self) -> Fut<'_, R<(Writer, Reader)>>;
    fn open_uni(&self) -> Fut<'_, R<Writer>>;
    fn accept_uni(&self) -> Fut<'_, R<Reader>>;
    fn close(&self, why: Close);
    /// Waits for the connection to end, and says why if either end closed
    /// it with one of `Close`'s codes.
    fn closed(&self) -> Fut<'_, Option<Close>>;
    fn is_closed(&self) -> bool;
    /// The path the connection takes now: "direct" or "relay", when the
    /// transport knows.
    fn path(&self) -> Option<&'static str> {
        None
    }
    /// 32 bytes both ends derive from this connection's TLS secrets under
    /// `label` (RFC 5705), when the transport has them: what binds a
    /// signature to one connection, so it can't be replayed on another.
    fn exporter(&self, _label: &[u8]) -> Option<[u8; 32]> {
        None
    }
}

/// A transport: dials machines by endpoint id and hands over the
/// connections they dial in. It decides nothing about who may connect: the
/// `Mesh` above it does, whatever the transport.
pub trait Dial: Send + Sync {
    /// This machine's endpoint id.
    fn me(&self) -> PeerId;
    /// A connection to `peer`, by the addresses the transport knows for it.
    fn connect(&self, peer: PeerId, alpn: Alpn) -> Fut<'_, R<Arc<dyn Wire>>>;
    /// The next connection a peer dialled in, or None once the transport
    /// is closed.
    fn incoming(&self) -> Fut<'_, Option<Arc<dyn Wire>>>;
    /// The protocols this machine answers from now on.
    fn offer(&self, alpns: &[Alpn]);
    fn close(&self) -> Fut<'_, ()>;
}

/// An admitted connection: the networks it carries, and its streams.
pub struct Link {
    wire: Arc<dyn Wire>,
    peer: PeerId,
    alpn: Alpn,
    dialler: PeerId,
    networks: RwLock<BTreeSet<String>>,
    opened: tokio::time::Instant,
}

/// How long after a connection opened another to the same peer is taken
/// as their two dials crossing: past it, the newer one is the peer coming
/// back.
pub const CROSSING: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a connection to a peer that shares no network any more stays
/// open: long enough for what was sent it last (the chain entry that
/// removes it) to go, never longer. It carries nothing of the network's
/// after this machine took the removal: every receiver decides what comes
/// by its own chain, which no longer lists the peer.
pub const REMOVED_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

// A connection that's no longer admitted, closed after REMOVED_GRACE, or
// at once with no runtime to wait on.
fn close_soon(l: Arc<Link>) {
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            rt.spawn(async move {
                tokio::time::sleep(REMOVED_GRACE).await;
                l.close(Close::Removed);
            });
        }
        Err(_) => l.close(Close::Removed),
    }
}

/// Whether a connection that stands (dialled by `old`, `age` old) stays
/// when another comes, dialled by `new`: only when the two crossed, and
/// the one the lower endpoint id dialled is the old one.
fn keeps_old(old: PeerId, new: PeerId, me: PeerId, peer: PeerId, age: std::time::Duration) -> bool {
    old != new && new != me.min(peer) && age < CROSSING
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Link({} {:?}, dialled by {})", self.peer, self.alpn, self.dialler)
    }
}

impl Link {
    pub fn peer(&self) -> PeerId {
        self.peer
    }

    pub fn alpn(&self) -> Alpn {
        self.alpn
    }

    /// The machine that dialled this connection.
    pub fn dialled_by(&self) -> PeerId {
        self.dialler
    }

    /// The networks this connection carries now.
    pub fn networks(&self) -> BTreeSet<String> {
        self.networks.reading().clone()
    }

    pub fn carries(&self, network: &str) -> bool {
        self.networks.reading().contains(network)
    }

    /// Refuses anything of `network` on this connection once the peer is no
    /// member of it: what's sent and what arrives for a network goes
    /// through here first.
    pub fn check(&self, network: &str) -> R<()> {
        if self.carries(network) {
            Ok(())
        } else {
            Err(NetError::new(403, "not-in-network", format!("{} isn't in {network} for this machine", self.peer)))
        }
    }

    pub async fn open_bi(&self) -> R<(Writer, Reader)> {
        self.wire.open_bi().await
    }

    pub async fn accept_bi(&self) -> R<(Writer, Reader)> {
        self.wire.accept_bi().await
    }

    pub async fn open_uni(&self) -> R<Writer> {
        self.wire.open_uni().await
    }

    pub async fn accept_uni(&self) -> R<Reader> {
        self.wire.accept_uni().await
    }

    pub fn close(&self, why: Close) {
        self.wire.close(why);
    }

    pub async fn closed(&self) -> Option<Close> {
        self.wire.closed().await
    }

    pub fn is_closed(&self) -> bool {
        self.wire.is_closed()
    }

    /// "direct" or "relay", when the transport says.
    pub fn path(&self) -> Option<&'static str> {
        self.wire.path()
    }

    /// The connection's keying material under `label`, when the transport
    /// has it (`Wire::exporter`).
    pub fn exporter(&self, label: &[u8]) -> Option<[u8; 32]> {
        self.wire.exporter(label)
    }
}

type Key = (PeerId, Alpn);

/// What became of a connection offered to the table.
enum Adopted {
    /// It is the connection for its peer and protocol now.
    New(Arc<Link>),
    /// It crossed one the other machine keeps too, and was closed.
    Kept(Arc<Link>),
}

/// The connections of this machine: admitted, one per peer and protocol,
/// and narrowed as members leave networks.
pub struct Mesh {
    dial: Arc<dyn Dial>,
    admission: Arc<Admission>,
    links: Mutex<BTreeMap<Key, Arc<Link>>>,
    // One dial at a time to each peer and protocol, so two callers share a
    // connection rather than replacing each other's.
    dialling: Mutex<BTreeMap<Key, Arc<tokio::sync::Mutex<()>>>>,
}

impl Mesh {
    /// The mesh over `dial`. `admission` is the one the transport's own
    /// early checks read (the iroh endpoint's hooks and handlers), if any.
    pub fn new(dial: Arc<dyn Dial>, admission: Arc<Admission>) -> Mesh {
        dial.offer(&admission.offered());
        Mesh { dial, admission, links: Mutex::new(BTreeMap::new()), dialling: Mutex::new(BTreeMap::new()) }
    }

    pub fn me(&self) -> PeerId {
        self.dial.me()
    }

    pub fn admission(&self) -> &Admission {
        &self.admission
    }

    /// A network's members as this machine's copy of its chain has them,
    /// applied to every connection at once.
    pub fn set_members(&self, network: &str, members: impl IntoIterator<Item = PeerId>) {
        self.admission.set_members(network, members);
        self.refresh();
    }

    /// A network this machine left.
    pub fn leave(&self, network: &str) {
        self.admission.leave(network);
        self.refresh();
    }

    /// One of this machine's join tickets opened: join/2 is answered while
    /// any is open.
    pub fn open_ticket(&self) {
        if self.admission.open_ticket() {
            self.dial.offer(&self.admission.offered());
        }
    }

    pub fn close_ticket(&self) {
        if self.admission.close_ticket() {
            self.dial.offer(&self.admission.offered());
        }
    }

    /// A join through the inviter an invite names begins: join/2 may be
    /// dialled to that endpoint id alone, until `end_join`.
    pub fn begin_join(&self, inviter: PeerId) {
        self.admission.begin_join(inviter);
    }

    pub fn end_join(&self, inviter: &PeerId) {
        self.admission.end_join(inviter);
    }

    /// The live connection to `peer` on `alpn`, if there is one.
    pub fn link(&self, peer: PeerId, alpn: Alpn) -> Option<Arc<Link>> {
        self.links.locked().get(&(peer, alpn)).filter(|l| !l.is_closed()).cloned()
    }

    /// Every live connection.
    pub fn links(&self) -> Vec<Arc<Link>> {
        self.links.locked().values().filter(|l| !l.is_closed()).cloned().collect()
    }

    /// The connection to `peer` on `alpn`: the one there is, or a new one.
    /// Refused before anything is sent when the peer isn't admitted.
    pub async fn dial(&self, peer: PeerId, alpn: Alpn) -> R<Arc<Link>> {
        self.admission.check(&peer, alpn, Side::Dialled)?;
        if let Some(l) = self.link(peer, alpn) {
            return Ok(l);
        }
        let gate = self.dialling.locked().entry((peer, alpn)).or_default().clone();
        let _one = gate.lock().await;
        if let Some(l) = self.link(peer, alpn) {
            return Ok(l);
        }
        let wire = self.dial.connect(peer, alpn).await?;
        if wire.peer() != peer || wire.alpn() != alpn {
            wire.close(Close::Refused);
            return Err(NetError::new(502, "wrong-peer", format!("dialled {peer} and reached {}", wire.peer())));
        }
        // The membership may have changed while dialling.
        let networks = match self.admission.check(&peer, alpn, Side::Dialled) {
            Ok(n) => n,
            Err(r) => {
                wire.close(Close::Refused);
                return Err(r.into());
            }
        };
        match self.adopt(wire, Side::Dialled, networks) {
            Adopted::New(l) | Adopted::Kept(l) => Ok(l),
        }
    }

    /// The next connection a peer dialled in that is admitted and kept;
    /// None once the transport is closed. A refused one is closed with
    /// `Close::Refused`, and one that crossed a kept connection with
    /// `Close::Crossed`, and neither is handed out.
    pub async fn accept(&self) -> Option<Arc<Link>> {
        loop {
            let wire = self.dial.incoming().await?;
            let Ok(networks) = self.admission.check(&wire.peer(), wire.alpn(), Side::Accepted) else {
                wire.close(Close::Refused);
                continue;
            };
            if let Adopted::New(l) = self.adopt(wire, Side::Accepted, networks) {
                return Some(l);
            }
        }
    }

    pub async fn close(&self) {
        for l in self.links() {
            l.close(Close::Done);
        }
        self.dial.close().await;
    }

    // Puts a connection in the table, or keeps the one there when both
    // machines dialled: the one the lower endpoint id dialled stays. Two
    // connections dialled by the same machine mean it dialled again (it
    // restarted, or the old path died), so the newer one stays.
    fn adopt(&self, wire: Arc<dyn Wire>, side: Side, networks: BTreeSet<String>) -> Adopted {
        let me = self.me();
        let peer = wire.peer();
        let alpn = wire.alpn();
        let dialler = match side {
            Side::Dialled => me,
            Side::Accepted => peer,
        };
        let link = Arc::new(Link { wire, peer, alpn, dialler, networks: RwLock::new(networks), opened: tokio::time::Instant::now() });
        let mut links = self.links.locked();
        if let Some(old) = links.get(&(peer, alpn)).filter(|l| !l.is_closed()).cloned() {
            if keeps_old(old.dialler, dialler, me, peer, old.opened.elapsed()) {
                link.close(Close::Crossed);
                return Adopted::Kept(old);
            }
            old.close(if old.dialler == dialler { Close::Done } else { Close::Crossed });
        }
        links.insert((peer, alpn), link.clone());
        Adopted::New(link)
    }

    // Applies the membership to every connection: each carries the networks
    // its peer still shares, and one that shares none closes. A join
    // connection carries no network and is left as it is.
    fn refresh(&self) {
        let mut links = self.links.locked();
        links.retain(|_, l| !l.is_closed());
        links.retain(|(peer, alpn), l| {
            if !self.admission.is_members(*alpn) {
                return true;
            }
            let shared = self.admission.shared(peer);
            let none = shared.is_empty();
            // What it carries is what's shared now: nothing, for one that
            // closes after the grace.
            *l.networks.writing() = shared;
            if none {
                close_soon(l.clone());
                return false;
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_dials_keep_the_lower_ids_and_a_peer_coming_back_gets_its_new_one() {
        let (lo, hi) = (PeerId([1; 32]), PeerId([2; 32]));
        let soon = std::time::Duration::from_millis(200);
        // hi dialled first, lo's dial crossed it: lo's stays, on both.
        assert!(!keeps_old(hi, lo, hi, lo, soon), "at hi");
        assert!(!keeps_old(hi, lo, lo, hi, soon), "at lo");
        // lo dialled first, hi's dial crossed it: lo's stays.
        assert!(keeps_old(lo, hi, lo, hi, soon));
        assert!(keeps_old(lo, hi, hi, lo, soon));
        // hi restarted and dials again, long after lo's connection opened:
        // the old one is dead to hi, and its new one stays.
        assert!(!keeps_old(lo, hi, lo, hi, CROSSING + soon));
    }

    #[test]
    fn alpns_and_close_codes_read_back() {
        for a in Alpn::ALL {
            assert_eq!(Alpn::parse(a.bytes()), Some(a));
        }
        assert_eq!(Alpn::parse(b"xenonet/link/1"), None);
        assert_eq!(Alpn::parse(b"xenolith/sync/2"), None, "another program's, not XenoNet's");
        assert_eq!(Alpn::named(b"xenonet/link/2"), Alpn::Link);
        assert_eq!(Alpn::named(b"xenolith/sync/2"), Alpn::App(b"xenolith/sync/2"));
        assert!(!Alpn::named(b"xenolith/sync/2").is_xenonet() && Alpn::Svc.is_xenonet());
        assert_eq!(Alpn::App(b"xenolith/sync/2").name(), "xenolith/sync/2");
        for c in [Close::Done, Close::Refused, Close::Crossed, Close::Removed] {
            assert_eq!(Close::from_code(u64::from(c.code())), Some(c));
        }
        assert_eq!(Close::from_code(99), None);
        assert_eq!(PeerId([0xab; 32]).to_string(), "ababababab");
    }
}
