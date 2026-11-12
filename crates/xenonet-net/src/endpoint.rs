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
//! - iroh dials whatever relay a peer's address names, listed or not, so a
//!   relay URL that reaches the endpoint any other way (an address record,
//!   the LAN) is dropped unless the endpoint lists it.
//! - A relay's certificate is checked against the public CAs iroh carries
//!   and the CAs the person added for relays of their own (`relay-ca.pem`
//!   in the network folder: a setting of this machine, never in a chain).
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
use iroh::address_lookup::{AddressLookup, AddressLookupBuilder, AddressLookupBuilderError, EndpointData, EndpointInfo, Item, MemoryLookup};
use iroh::endpoint::{
    AfterHandshakeOutcome, BeforeConnectOutcome, Builder, Connection, ConnectionError, EndpointHooks, PortmapperConfig, RecvStream, SendStream, Side as QuicSide, VarInt,
};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher as _};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::CertificateDer;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
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
    /// The CAs the person added for relays with certificates of their own
    /// (`added_roots`), trusted beside the public ones.
    pub roots: Vec<CertificateDer<'static>>,
}

/// The file in the network folder that holds, as PEM, the CAs the person
/// trusts for their own relays: a setting of this machine alone.
pub const RELAY_CA: &str = "relay-ca.pem";

/// The CAs in `dir`'s `relay-ca.pem`; none when there's no such file.
pub fn added_roots(dir: &Path) -> R<Vec<CertificateDer<'static>>> {
    match std::fs::read(dir.join(RELAY_CA)) {
        Ok(pem) => roots_of(&pem),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(NetError::bad(format!("{RELAY_CA} can't be read: {e}"))),
    }
}

/// The certificates in `pem`: at least one, each one a trust store takes
/// as a CA. A bad one is refused here rather than failing every relay's
/// handshake later.
pub fn roots_of(pem: &[u8]) -> R<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_slice_iter(pem).collect::<Result<Vec<_>, _>>().map_err(|e| NetError::bad(format!("{RELAY_CA} isn't PEM certificates: {e}")))?;
    if certs.is_empty() {
        return Err(NetError::bad(format!("{RELAY_CA} has no certificate in it (-----BEGIN CERTIFICATE-----)")));
    }
    let mut store = rustls::RootCertStore::empty();
    for c in &certs {
        store.add(c.clone()).map_err(|e| NetError::bad(format!("{RELAY_CA} has a certificate that can't be a CA: {e}")))?;
    }
    Ok(certs)
}

/// How the endpoint checks relays' certificates: against the public CAs
/// iroh carries, and the person's own.
pub fn relay_tls(roots: &[CertificateDer<'static>]) -> CaTlsConfig {
    CaTlsConfig::embedded().with_extra_roots(roots.to_vec())
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

/// How `list` spells the relay at `url`, if it lists it. iroh writes a
/// relay's URL with a trailing slash and a chain keeps the person's
/// spelling, which has none, and chains compare relays as written.
pub fn as_listed(url: &RelayUrl, list: &[String]) -> Option<String> {
    let u = url.as_str().trim_end_matches('/');
    list.iter().find(|r| r.trim_end_matches('/') == u).cloned()
}

fn urls_of(relays: &Relays) -> Vec<RelayUrl> {
    match relays {
        Relays::None => Vec::new(),
        Relays::Listed(l) => l.iter().filter_map(|r| relay_url(&r.url).ok()).collect(),
    }
}

/// The relays the endpoint lists, shared with what feeds it addresses.
type Listed = Arc<Mutex<Vec<RelayUrl>>>;

/// `info` without the relays the endpoint doesn't list.
fn only_listed(info: &EndpointInfo, listed: &[RelayUrl]) -> EndpointInfo {
    let mut out = EndpointInfo::new(info.endpoint_id).with_ip_addrs(info.ip_addrs().copied().collect()).with_user_data(info.user_data().cloned());
    for r in info.relay_urls().filter(|r| listed.contains(r)) {
        out = out.with_relay_url(r.clone());
    }
    out
}

/// An address lookup (mDNS) whose answers lose the relays the endpoint
/// doesn't list: otherwise a machine on the LAN could point a member's
/// address at a relay no one chose, and iroh would dial it.
#[derive(Debug)]
struct ListedOnly<B> {
    inner: B,
    listed: Listed,
}

impl<B: AddressLookupBuilder> AddressLookupBuilder for ListedOnly<B> {
    fn into_address_lookup(self, endpoint: &Endpoint) -> Result<impl AddressLookup, AddressLookupBuilderError> {
        Ok(ListedLookup { inner: self.inner.into_address_lookup(endpoint)?, listed: self.listed })
    }
}

#[derive(Debug)]
struct ListedLookup<L> {
    inner: L,
    listed: Listed,
}

type Found = Pin<Box<dyn futures::Stream<Item = Result<Item, iroh::address_lookup::Error>> + Send + 'static>>;

impl<L: AddressLookup> AddressLookup for ListedLookup<L> {
    fn publish(&self, data: &EndpointData) {
        self.inner.publish(data);
    }

    fn resolve(&self, endpoint_id: EndpointId) -> Option<Found> {
        use futures::StreamExt as _;
        let listed = self.listed.clone();
        let found = self.inner.resolve(endpoint_id)?;
        Some(Box::pin(found.map(move |r| r.map(|item| Item::new(only_listed(item.endpoint_info(), &listed.locked()), item.provenance(), item.last_updated())))))
    }
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
    // The relays a member may be reached through: the endpoint's own.
    listed: Listed,
}

impl Addresses {
    /// `peer`'s addresses, in place of what was known, for `ttl`. A relay
    /// the endpoint doesn't list is left out (`lists`); the direct
    /// addresses stay.
    pub fn put(&self, peer: PeerId, direct: &[SocketAddr], relay: Option<&str>, ttl: Duration) -> R<()> {
        let id = endpoint_id(peer)?;
        let mut addr = EndpointAddr::new(id);
        for a in direct {
            addr = addr.with_ip_addr(*a);
        }
        if let Some(r) = relay {
            let url = relay_url(r)?;
            if self.listed.locked().contains(&url) {
                addr = addr.with_relay_url(url);
            }
        }
        self.memory.set_endpoint_info(addr);
        self.until.locked().insert(peer, Instant::now() + ttl);
        Ok(())
    }

    /// Whether the endpoint lists the relay at `url`, so that a member's
    /// address may name it.
    pub fn lists(&self, url: &str) -> bool {
        relay_url(url).is_ok_and(|u| self.listed.locked().contains(&u))
    }

    // The relays the endpoint lists from now on. What was kept of members
    // loses any relay no longer listed.
    fn list(&self, relays: &Relays) {
        let listed = urls_of(relays);
        let peers: Vec<PeerId> = self.until.locked().keys().copied().collect();
        for p in peers {
            let Ok(id) = endpoint_id(p) else { continue };
            if let Some(info) = self.memory.get_endpoint_info(id) {
                if info.relay_urls().any(|r| !listed.contains(r)) {
                    self.memory.set_endpoint_info(only_listed(&info, &listed).into_endpoint_addr());
                }
            }
        }
        *self.listed.locked() = listed;
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

// What the tests change of how an endpoint binds.
#[derive(Clone, Copy)]
struct Knobs {
    // Off: the doors and the mesh refuse what the hooks would have.
    early_cut: bool,
    // Off: no IP transport at all, so every path goes through a relay
    // (iroh's own way to test relay-only paths).
    direct: bool,
}

impl Default for Knobs {
    fn default() -> Knobs {
        Knobs { early_cut: true, direct: true }
    }
}

/// This machine's iroh endpoint, its doors, and what it knows of where
/// members are.
pub struct IrohDial {
    me: PeerId,
    router: Router,
    incoming: tokio::sync::Mutex<mpsc::Receiver<Arc<dyn Wire>>>,
    addresses: Addresses,
    relays: Arc<tokio::sync::Mutex<Relays>>,
}

/// How long the home relay may be gone before the endpoint is told to
/// look for another, and how long it then waits before telling it again.
const HOME_GRACE: Duration = Duration::from_secs(5);
const HOME_AGAIN: Duration = Duration::from_secs(30);

// iroh 1.4 picks a new home relay only in a full report on its relays,
// five minutes apart, or when its relays change. So when the home relay
// has gone and another is listed, the home's own entry goes in again:
// that counts as a change, and the report that follows moves the home to
// a relay that answers. Ends with the endpoint.
async fn keep_home(endpoint: Endpoint, relays: Arc<tokio::sync::Mutex<Relays>>) {
    let mut status = endpoint.home_relay_status();
    let gone = |s: &[iroh::endpoint::RelayStatus]| (!s.is_empty() && !s.iter().any(iroh::endpoint::RelayStatus::is_connected)).then(|| s[0].url().clone());
    let wait = |d: Duration| {
        let closed = endpoint.closed();
        async move {
            tokio::select! {
                () = closed => false,
                () = tokio::time::sleep(d) => true,
            }
        }
    };
    loop {
        let Some(home) = gone(&status.get()) else {
            tokio::select! {
                () = endpoint.closed() => return,
                r = status.updated() => if r.is_err() { return },
            }
            continue;
        };
        if !wait(HOME_GRACE).await {
            return;
        }
        if gone(&status.get()).as_ref() != Some(&home) {
            continue;
        }
        let list = relays.lock().await.clone();
        if urls_of(&list).len() > 1 {
            if let Ok(RelayMode::Custom(map)) = relay_mode(&list) {
                if let Some(config) = map.get(&home) {
                    endpoint.insert_relay(home, config).await;
                }
            }
        }
        if !wait(HOME_AGAIN).await {
            return;
        }
    }
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
        IrohDial::bind_with(key, settings, admission, Knobs::default()).await
    }

    // The knobs are the tests' alone.
    async fn bind_with(key: &HostKey, settings: &Settings, admission: Arc<Admission>, knobs: Knobs) -> R<IrohDial> {
        let addresses = Addresses::default();
        addresses.list(&settings.relays);
        let offered: Vec<Vec<u8>> = admission.offered().iter().map(|a| a.bytes().to_vec()).collect();
        let mut b = Builder::empty()
            .crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .secret_key(secret_key(key))
            .alpns(offered.clone())
            .relay_mode(relay_mode(&settings.relays)?)
            .ca_tls_config(relay_tls(&settings.roots))
            .portmapper_config(PortmapperConfig::Disabled)
            // No session tickets, so no 0-RTT: every connection does the
            // whole handshake.
            .max_tls_tickets(0)
            .address_lookup(addresses.memory.clone());
        if settings.lan == LanLookup::On {
            b = b.address_lookup(ListedOnly { inner: MdnsAddressLookup::builder().service_name(MDNS_SERVICE), listed: addresses.listed.clone() });
        }
        if !knobs.direct {
            b = b.clear_ip_transports();
        } else if let Some(at) = settings.bind {
            b = b.clear_ip_transports().bind_addr(at).map_err(|e| NetError::bad(format!("can't bind {at}: {e}")))?;
        }
        if knobs.early_cut {
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
        let relays = Arc::new(tokio::sync::Mutex::new(settings.relays.clone()));
        if !settings.relays.is_none() {
            tokio::spawn(keep_home(endpoint.clone(), relays.clone()));
        }
        Ok(IrohDial { me: PeerId(*endpoint.id().as_bytes()), router, incoming: tokio::sync::Mutex::new(from), addresses, relays })
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
            self.addresses.list(&relays);
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
        self.addresses.list(&relays);
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

#[cfg(test)]
#[path = "relay_tests.rs"]
mod relay_tests;
