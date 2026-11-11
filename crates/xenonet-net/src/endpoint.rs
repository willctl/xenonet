//! The `Dial` over iroh (docs/design/network.md; phase 5's transport): QUIC
//! with hole punching, one endpoint per machine, whose endpoint id is
//! host.key's Ed25519 key.
//!
//! Nothing here reaches anyone's infrastructure unless the person chose it:
//!
//! - The endpoint is built from `Builder::empty()`, never iroh's `N0`
//!   preset, which would turn on n0's relays and n0's lookup services.
//! - Relays are the person's choice, `Relays::None` (LAN and direct paths
//!   only) or a list they name. There is no default relay. `online` never
//!   waits on a relay when there is none (iroh's own would wait forever).
//! - Addresses come from `Addresses` (iroh's `MemoryLookup`, which keeps
//!   entries forever, so each gets a time to live here), and from mDNS on
//!   the LAN only when the machine's setting says so. The setting is asked
//!   at the first network setup (`LanLookup::Unasked` is off): mDNS
//!   announces this machine's endpoint id, which stays the same, on every
//!   LAN it joins.
//! - No port mapping (UPnP, NAT-PMP), and no 0-RTT: the client keeps no
//!   session tickets, so it never sends early data, and a stream that came
//!   in as 0-RTT is refused.
//!
//! Admission (`admission`) is checked by the endpoint's hooks as an early
//! cut, before a connection reaches a protocol: an outgoing dial before any
//! packet is sent, and every connection once its handshake is done. Each
//! protocol's handler (one per ALPN, on iroh's `Router`) checks it again,
//! and so does the `Mesh` above.

use crate::admission::Admission;
use crate::dial::{Alpn, Close, Dial, Fut, PeerId, Reader, Side, Wire, Writer};
use crate::keys::{HostKey, PublicKeys};
use crate::{NetError, R};
use iroh::address_lookup::MemoryLookup;
use iroh::endpoint::{
    AfterHandshakeOutcome, BeforeConnectOutcome, Builder, Connection, ConnectionError, EndpointHooks, PortmapperConfig, RecvStream, SendStream, Side as QuicSide, VarInt,
};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher as _};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use xenonet_os::sync::Locked;

/// How long a dial may take before it is given up.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The service name mDNS announces XenoNet's endpoints under, so that other
/// iroh programs on the LAN don't see them and they don't see those.
pub const MDNS_SERVICE: &str = "xenonet";

/// The endpoint's secret key: host.key's Ed25519 key, so the endpoint id is
/// host.key's public signing key.
pub fn secret_key(key: &HostKey) -> SecretKey {
    SecretKey::from_bytes(&key.transport_secret())
}

/// A machine's endpoint id, from the public keys its chain entry carries.
pub fn peer_id(keys: &PublicKeys) -> PeerId {
    PeerId(keys.sign)
}

/// A relay the person named, and the token it asks for, if any. The token
/// is a setting of this machine, never part of a chain.
#[derive(Clone, PartialEq, Eq)]
pub struct Relay {
    pub url: String,
    pub auth_token: Option<String>,
}

impl fmt::Debug for Relay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Relay({}{})", self.url, if self.auth_token.is_some() { ", with a token" } else { "" })
    }
}

/// The relays this endpoint uses: the person's choice, with no default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relays {
    /// None at all: the LAN and direct paths only.
    None,
    /// The relays the person named.
    Listed(Vec<Relay>),
}

impl Relays {
    pub fn is_none(&self) -> bool {
        match self {
            Relays::None => true,
            Relays::Listed(l) => l.is_empty(),
        }
    }
}

/// The machine's setting for finding members on the LAN by mDNS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanLookup {
    /// Not asked yet: off until the person answers at the first network
    /// setup.
    Unasked,
    On,
    Off,
}

/// How the endpoint is set up.
#[derive(Clone, Debug)]
pub struct Settings {
    pub relays: Relays,
    pub lan: LanLookup,
    /// The address to bind, or None for every interface, IPv4 and IPv6.
    pub bind: Option<SocketAddr>,
}

/// iroh's relay mode for the person's relays: disabled when there are
/// none, their list alone otherwise.
pub fn relay_mode(relays: &Relays) -> R<RelayMode> {
    let Relays::Listed(list) = relays else {
        return Ok(RelayMode::Disabled);
    };
    if list.is_empty() {
        return Ok(RelayMode::Disabled);
    }
    let map = RelayMap::empty();
    for r in list {
        let url = relay_url(&r.url)?;
        let mut config = RelayConfig::from(url.clone());
        if let Some(t) = &r.auth_token {
            config = config.with_auth_token(t.clone());
        }
        map.insert(url, Arc::new(config));
    }
    Ok(RelayMode::Custom(map))
}

fn relay_url(url: &str) -> R<RelayUrl> {
    url.parse().map_err(|_| NetError::bad(format!("{url} isn't a relay's URL")))
}

/// The address lookups the endpoint has, by name: memory always, mDNS when
/// the machine's setting is on. Nothing else (no DNS, no pkarr).
pub fn lookups(lan: LanLookup) -> Vec<&'static str> {
    let mut l = vec![MemoryLookup::PROVENANCE];
    if lan == LanLookup::On {
        l.push(iroh_mdns_address_lookup::NAME);
    }
    l
}

/// Where members can be reached: iroh's memory lookup, with a time to live
/// on each entry, which iroh doesn't keep.
#[derive(Clone, Default)]
pub struct Addresses {
    memory: MemoryLookup,
    until: Arc<Mutex<BTreeMap<PeerId, Instant>>>,
}

impl Addresses {
    /// `peer`'s addresses, in place of what was known, for `ttl`.
    pub fn put(&self, peer: PeerId, direct: &[SocketAddr], relay: Option<&str>, ttl: Duration) -> R<()> {
        let id = endpoint_id(peer)?;
        let mut addr = EndpointAddr::new(id);
        for a in direct {
            addr = addr.with_ip_addr(*a);
        }
        if let Some(r) = relay {
            addr = addr.with_relay_url(relay_url(r)?);
        }
        self.memory.set_endpoint_info(addr);
        self.until.locked().insert(peer, Instant::now() + ttl);
        Ok(())
    }

    pub fn forget(&self, peer: &PeerId) {
        self.until.locked().remove(peer);
        if let Ok(id) = endpoint_id(*peer) {
            self.memory.remove_endpoint_info(id);
        }
    }

    /// Drops what has outlived its time. Run before every dial.
    pub fn expire(&self) {
        let now = Instant::now();
        let gone: Vec<PeerId> = {
            let mut until = self.until.locked();
            let gone = until.iter().filter(|(_, t)| **t <= now).map(|(p, _)| *p).collect::<Vec<_>>();
            for p in &gone {
                until.remove(p);
            }
            gone
        };
        for p in gone {
            if let Ok(id) = endpoint_id(p) {
                self.memory.remove_endpoint_info(id);
            }
        }
    }

    /// What is known of `peer` now, expired entries left out.
    pub fn get(&self, peer: &PeerId) -> Option<EndpointAddr> {
        self.expire();
        let id = endpoint_id(*peer).ok()?;
        self.memory.get_endpoint_info(id).map(|i| i.to_endpoint_addr())
    }
}

fn endpoint_id(peer: PeerId) -> R<EndpointId> {
    EndpointId::from_bytes(&peer.0).map_err(|_| NetError::bad(format!("{peer} isn't an Ed25519 public key")))
}

/// The endpoint's early cut: refuses a dial to a machine that isn't
/// admitted before any packet goes out, and closes any connection that
/// isn't once its handshake has named the peer.
#[derive(Debug)]
struct EarlyCut {
    admission: Arc<Admission>,
}

impl EarlyCut {
    fn admits(&self, peer: &EndpointId, alpn: &[u8], side: Side) -> bool {
        Alpn::parse(alpn).is_some_and(|a| self.admission.check(&PeerId(*peer.as_bytes()), a, side).is_ok())
    }
}

// Both answers are known at once: the futures iroh asks for are ready.
impl EndpointHooks for EarlyCut {
    fn before_connect<'a>(&'a self, remote: &'a EndpointAddr, alpn: &'a [u8]) -> impl Future<Output = BeforeConnectOutcome> + Send + 'a {
        let outcome = if self.admits(&remote.id, alpn, Side::Dialled) { BeforeConnectOutcome::Accept } else { BeforeConnectOutcome::Reject };
        std::future::ready(outcome)
    }

    fn after_handshake<'a>(&'a self, conn: &'a Connection) -> impl Future<Output = AfterHandshakeOutcome> + Send + 'a {
        let side = match conn.side() {
            QuicSide::Client => Side::Dialled,
            QuicSide::Server => Side::Accepted,
        };
        let outcome = if self.admits(&conn.remote_id(), conn.alpn(), side) {
            AfterHandshakeOutcome::Accept
        } else {
            AfterHandshakeOutcome::Reject { error_code: VarInt::from_u32(Close::Refused.code()), reason: Close::Refused.reason().to_vec() }
        };
        std::future::ready(outcome)
    }
}

/// One protocol's door: checks each connection that comes in on its ALPN
/// and hands the admitted ones to the `Mesh` (through `incoming`).
#[derive(Debug)]
struct Door {
    alpn: Alpn,
    admission: Arc<Admission>,
    to: mpsc::Sender<Arc<dyn Wire>>,
}

impl ProtocolHandler for Door {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = PeerId(*conn.remote_id().as_bytes());
        if Alpn::parse(conn.alpn()) != Some(self.alpn) || self.admission.check(&peer, self.alpn, Side::Accepted).is_err() {
            conn.close(VarInt::from_u32(Close::Refused.code()), Close::Refused.reason());
            return Ok(());
        }
        let wire: Arc<dyn Wire> = Arc::new(IrohWire::new(conn, peer, self.alpn));
        // The mesh holds the connection from here; once it has stopped
        // taking any, the connection goes with this one.
        let _ = self.to.send(wire).await;
        Ok(())
    }
}

/// A connection over iroh.
#[derive(Debug)]
pub struct IrohWire {
    conn: Connection,
    peer: PeerId,
    alpn: Alpn,
    // How this end closed it, which iroh reports only as "locally closed".
    ours: Mutex<Option<Close>>,
}

impl IrohWire {
    fn new(conn: Connection, peer: PeerId, alpn: Alpn) -> IrohWire {
        IrohWire { conn, peer, alpn, ours: Mutex::new(None) }
    }
}

fn ended(e: impl fmt::Display) -> NetError {
    NetError::new(502, "link-closed", format!("the connection ended: {e}"))
}

fn boxed(send: SendStream, recv: RecvStream) -> (Writer, Reader) {
    (Box::new(send), Box::new(recv))
}

// A stream that arrived as 0-RTT early data: none should, since no client
// here sends it, and early data can be replayed.
fn refuse_early(send: Option<&mut SendStream>, recv: &mut RecvStream) {
    let code = VarInt::from_u32(Close::Refused.code());
    let _ = recv.stop(code);
    if let Some(s) = send {
        let _ = s.reset(code);
    }
}

impl Wire for IrohWire {
    fn peer(&self) -> PeerId {
        self.peer
    }

    fn alpn(&self) -> Alpn {
        self.alpn
    }

    fn open_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move {
            let (send, recv) = self.conn.open_bi().await.map_err(ended)?;
            Ok(boxed(send, recv))
        })
    }

    fn accept_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move {
            loop {
                let (mut send, mut recv) = self.conn.accept_bi().await.map_err(ended)?;
                if recv.is_0rtt() {
                    refuse_early(Some(&mut send), &mut recv);
                    continue;
                }
                return Ok(boxed(send, recv));
            }
        })
    }

    fn open_uni(&self) -> Fut<'_, R<Writer>> {
        Box::pin(async move {
            let send = self.conn.open_uni().await.map_err(ended)?;
            Ok(Box::new(send) as Writer)
        })
    }

    fn accept_uni(&self) -> Fut<'_, R<Reader>> {
        Box::pin(async move {
            loop {
                let mut recv = self.conn.accept_uni().await.map_err(ended)?;
                if recv.is_0rtt() {
                    refuse_early(None, &mut recv);
                    continue;
                }
                return Ok(Box::new(recv) as Reader);
            }
        })
    }

    fn close(&self, why: Close) {
        self.ours.locked().get_or_insert(why);
        self.conn.close(VarInt::from_u32(why.code()), why.reason());
    }

    fn closed(&self) -> Fut<'_, Option<Close>> {
        Box::pin(async move {
            match self.conn.closed().await {
                ConnectionError::ApplicationClosed(c) => Close::from_code(c.error_code.into_inner()),
                ConnectionError::LocallyClosed => *self.ours.locked(),
                _ => None,
            }
        })
    }

    fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }

    fn path(&self) -> Option<&'static str> {
        let paths = self.conn.paths();
        let selected = paths.iter().find(iroh::endpoint::Path::is_selected)?;
        if selected.is_relay() {
            Some("relay")
        } else if selected.is_ip() {
            Some("direct")
        } else {
            None
        }
    }
}

/// This machine's iroh endpoint, its doors, and what it knows of where
/// members are.
pub struct IrohDial {
    me: PeerId,
    router: Router,
    incoming: tokio::sync::Mutex<mpsc::Receiver<Arc<dyn Wire>>>,
    addresses: Addresses,
    relays: tokio::sync::Mutex<Relays>,
}

impl fmt::Debug for IrohDial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IrohDial({})", self.me)
    }
}

impl IrohDial {
    /// The endpoint for this machine, keyed by host.key, bound as
    /// `settings` say, with its doors open for the protocols `admission`
    /// offers.
    pub async fn bind(key: &HostKey, settings: &Settings, admission: Arc<Admission>) -> R<IrohDial> {
        IrohDial::bind_with(key, settings, admission, true).await
    }

    // `early_cut` off is for the tests alone: they check that the doors and
    // the mesh refuse what the hooks would have.
    async fn bind_with(key: &HostKey, settings: &Settings, admission: Arc<Admission>, early_cut: bool) -> R<IrohDial> {
        let addresses = Addresses::default();
        let offered: Vec<Vec<u8>> = admission.offered().iter().map(|a| a.bytes().to_vec()).collect();
        let mut b = Builder::empty()
            .crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .secret_key(secret_key(key))
            .alpns(offered.clone())
            .relay_mode(relay_mode(&settings.relays)?)
            .portmapper_config(PortmapperConfig::Disabled)
            // No session tickets, so no 0-RTT: every connection does the
            // whole handshake.
            .max_tls_tickets(0)
            .address_lookup(addresses.memory.clone());
        if settings.lan == LanLookup::On {
            b = b.address_lookup(MdnsAddressLookup::builder().service_name(MDNS_SERVICE));
        }
        if let Some(at) = settings.bind {
            b = b.clear_ip_transports().bind_addr(at).map_err(|e| NetError::bad(format!("can't bind {at}: {e}")))?;
        }
        if early_cut {
            b = b.hooks(EarlyCut { admission: admission.clone() });
        }
        let endpoint = b.bind().await.map_err(|e| NetError::internal(format!("the network's endpoint didn't start: {e}")))?;
        let (to, from) = mpsc::channel(64);
        let mut router = Router::builder(endpoint.clone());
        for alpn in Alpn::ALL {
            router = router.accept(alpn.bytes(), Door { alpn, admission: admission.clone(), to: to.clone() });
        }
        let router = router.spawn();
        // The router offers every protocol it has a door for; join/2 is
        // offered only while a ticket is open. Its door and the hooks refuse
        // a join with none open in the moment between the two.
        endpoint.set_alpns(offered);
        Ok(IrohDial { me: PeerId(*endpoint.id().as_bytes()), router, incoming: tokio::sync::Mutex::new(from), addresses, relays: tokio::sync::Mutex::new(settings.relays.clone()) })
    }

    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    pub fn addresses(&self) -> &Addresses {
        &self.addresses
    }

    /// The addresses this endpoint is reachable at directly.
    pub fn direct_addrs(&self) -> Vec<SocketAddr> {
        self.endpoint().addr().ip_addrs().copied().collect()
    }

    /// How many address lookups the endpoint has (`lookups` names them).
    pub fn lookup_count(&self) -> usize {
        self.endpoint().address_lookup().map_or(0, iroh::address_lookup::AddressLookupServices::len)
    }

    /// Waits until the endpoint reaches a home relay, when it has relays to
    /// reach. With none, there is nothing to wait for.
    pub async fn online(&self) {
        if self.relays.lock().await.is_none() {
            return;
        }
        self.endpoint().online().await;
    }

    /// Whether the endpoint has a home relay now.
    pub fn has_home_relay(&self) -> bool {
        self.endpoint().home_relay_status().get().iter().any(iroh::endpoint::RelayStatus::is_connected)
    }

    /// The person's relays, in place of the ones before. Going from none to
    /// some, or back, needs the endpoint started again: iroh sets up its
    /// relay transport when it binds.
    pub async fn set_relays(&self, relays: Relays) -> R<()> {
        let mut had = self.relays.lock().await;
        if had.is_none() != relays.is_none() {
            return Err(NetError::new(409, "restart-needed", "the network's endpoint has to start again to change between relays and none"));
        }
        let old = match &*had {
            Relays::Listed(l) => l.clone(),
            Relays::None => Vec::new(),
        };
        let new = match &relays {
            Relays::Listed(l) => l.clone(),
            Relays::None => Vec::new(),
        };
        let RelayMode::Custom(map) = relay_mode(&relays)? else {
            *had = relays;
            return Ok(());
        };
        for r in &old {
            if !new.iter().any(|n| n.url == r.url) {
                self.endpoint().remove_relay(&relay_url(&r.url)?).await;
            }
        }
        for r in &new {
            if !old.contains(r) {
                let url = relay_url(&r.url)?;
                if let Some(config) = map.get(&url) {
                    self.endpoint().insert_relay(url, config).await;
                }
            }
        }
        *had = relays;
        Ok(())
    }
}

impl Dial for IrohDial {
    fn me(&self) -> PeerId {
        self.me
    }

    fn connect(&self, peer: PeerId, alpn: Alpn) -> Fut<'_, R<Arc<dyn Wire>>> {
        Box::pin(async move {
            self.addresses.expire();
            let id = endpoint_id(peer)?;
            let conn = tokio::time::timeout(CONNECT_TIMEOUT, self.endpoint().connect(id, alpn.bytes()))
                .await
                .map_err(|_| NetError::new(504, "unreachable", format!("{peer} didn't answer")))?
                .map_err(|e| NetError::new(502, "unreachable", format!("couldn't reach {peer}: {e}")))?;
            Ok(Arc::new(IrohWire::new(conn, peer, alpn)) as Arc<dyn Wire>)
        })
    }

    fn incoming(&self) -> Fut<'_, Option<Arc<dyn Wire>>> {
        Box::pin(async move { self.incoming.lock().await.recv().await })
    }

    fn offer(&self, alpns: &[Alpn]) {
        self.endpoint().set_alpns(alpns.iter().map(|a| a.bytes().to_vec()).collect());
    }

    fn close(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            let _ = self.router.shutdown().await;
        })
    }
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
