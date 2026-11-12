//! `forward(alpn)`: an actor's own protocol carried over this machine's
//! endpoint (phase 6, revision 17). An actor whose grant names
//! `forward(<alpn>)` (xenonet-authz's `Token::forward`) may have XenoNet
//! take its connections on that one ALPN from the members of this
//! machine's networks, and dial them for it: Xenolith's sync/2 in shared
//! mode rides XenoNet's endpoint instead of one of its own.
//!
//! **What's refused, and where.**
//!
//! - *Forwardable ALPNs only.* The composition root registers the ALPNs
//!   that may be forwarded (`Forwardable`), as data; XenoNet's own
//!   (link/2, join/2, svc/2) are never among them. An attach naming
//!   another is refused before its grant is read.
//! - *The grant.* An attach is decided by `Grants`: this machine's actor's
//!   token, in its time, not revoked, whose forward names exactly that
//!   ALPN. A token for another ALPN, or none, is refused.
//! - *One actor per ALPN.* A second attach for an ALPN that's held is
//!   refused until the first ends, so no stream goes to whichever came
//!   last.
//! - *Members only.* Every connection in and out goes through the mesh
//!   and its admission (`Doors::xenonet_forwarding`): the peer is a member
//!   of a network this machine is in, by its own chains, before anything
//!   is forwarded, and the mesh closes the connection when it leaves the
//!   last one. A dial goes to that ALPN alone, so the grant is no way to
//!   XenoNet's own protocols or to any machine outside its networks.
//! - *Revocation ends it.* A revoked grant (`revoked`, which the service
//!   calls as the store records a revocation) ends the actor's session
//!   and closes every connection forwarded for it; each new connection,
//!   dial and stream re-decides the grant as well.
//!
//! XenoNet terminates QUIC, so it reads the forwarded protocol's bytes:
//! the same machine and account, which reads the actor's folder anyway.
//! The actor decides again on what it receives (Xenolith: its space
//! chains and the signed hello).
//!
//! **The local side.** The actor reaches XenoNet over a local stream (the
//! service's owner-only socket in its runtime folder), in records as
//! link/2's: a u32 length and that much JSON. The first record of a
//! connection says what it is:
//!
//! - `attach {token, alpn}`: the actor's session. The answer is
//!   `attached {me, session}` (this machine's endpoint id and the
//!   session's secret) or `refused {code, error}`. Then, from XenoNet:
//!   `incoming {conn, peer, networks, exporter}`, `dialled {req, conn,
//!   networks, exporter}`, `failed {req, code, error}`, `stream {conn,
//!   stream, bi}`, `opened {req, stream}`, `closed {conn, code}`,
//!   `chains {req, chains}`, `revoked`; from the actor: `dial {req,
//!   peer}`, `open {req, conn, bi}`, `close {conn, code}`, `chains {req}`.
//!   The header of a connection names the remote's endpoint id, the
//!   networks both are in, by this machine's chains, and the connection's
//!   keying material under `EXPORTER` (both ends derive the same bytes),
//!   so what the actor signs is bound to the QUIC connection XenoNet
//!   terminates and can't be spliced onto another (phase 6, revision 16d).
//! - `chains`: copies of the chains this machine holds (its networks', the
//!   persons' that own them, its token roots' certificates), as the
//!   composition root gives them (`Forwards::set_chains`). The actor
//!   verifies them from genesis itself: XenoNet's word counts only for a
//!   connection's remote endpoint id (revision 22).
//! - `take {session, stream}`: one stream, by the number `stream` or
//!   `opened` gave it, answered `taken` and then its bytes, as they are,
//!   both ways (one way for a uni stream).
//!
//! `ForwardDial` is the actor's end: a `Dial` over that socket, so the
//! actor runs its protocol over a `Mesh` as it would over its own
//! endpoint.

use crate::codec::{b64, random, unb64_n};
use crate::dial::{Alpn, Close, Dial, Fut, Link, Mesh, PeerId, Reader, Wire, Writer};
use crate::link2::{read_record, write_record};
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, oneshot, watch};
use xenonet_os::sync::{Locked, ReadWrite};

/// The record version of the local side.
pub const VERSION: u64 = 1;
/// How long a stream XenoNet holds for the actor waits to be taken.
pub const TAKE_WAIT: Duration = Duration::from_secs(10);
/// The most streams waiting to be taken, per session: a remote that opens
/// more has them reset.
pub const MAX_WAITING: usize = 256;
/// The label a forwarded connection's keying material is derived under,
/// for its header.
pub const EXPORTER: &[u8] = b"xenonet/forward/1";

/// What the composition root says of this machine's chains, for the
/// actors (`chains`).
pub type Chains = Arc<dyn Fn() -> Value + Send + Sync>;

fn exporter_of(link: &Link) -> Value {
    link.exporter(EXPORTER).map_or(Value::Null, |e| Value::String(b64(&e)))
}

fn refused(code: &'static str, words: impl Into<String>) -> NetError {
    NetError::new(403, code, words)
}

fn bad(what: &str) -> NetError {
    NetError::bad(format!("a forward record's {what} isn't right"))
}

// ---------- what may be forwarded, and for whom ----------

/// The ALPNs this machine forwards, as the composition root registers
/// them. XenoNet's own are never among them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forwardable(Vec<Alpn>);

impl Forwardable {
    /// `alpns`, each an ALPN as a grant names one (xenonet-authz's
    /// `is_alpn`) and none of XenoNet's own.
    pub fn new(alpns: &[&'static [u8]]) -> R<Forwardable> {
        let mut out = Vec::new();
        for a in alpns {
            let alpn = Alpn::named(a);
            let text = std::str::from_utf8(a).ok().filter(|t| xenonet_authz::is_alpn(t));
            if alpn.is_xenonet() || text.is_none() {
                return Err(NetError::bad(format!("{} can't be forwarded", String::from_utf8_lossy(a))));
            }
            if !out.contains(&alpn) {
                out.push(alpn);
            }
        }
        Ok(Forwardable(out))
    }

    pub fn alpns(&self) -> &[Alpn] {
        &self.0
    }

    /// The forwardable ALPN `name` names.
    pub fn get(&self, name: &str) -> Option<Alpn> {
        self.0.iter().copied().find(|a| a.bytes() == name.as_bytes())
    }
}

/// A grant that may forward: whose it is, and its revocation ids (hex), any
/// of which revoked ends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub actor: String,
    pub ids: Vec<String>,
}

/// What decides a forward: the service's, over xenonet-authz. It decides
/// the attach, and again at each connection, dial and stream.
pub trait Grants: Send + Sync {
    /// The grant `token` is, when it's this machine's actor's, in its
    /// time, not revoked, and its forward names `alpn` exactly; why not
    /// otherwise (`no-grant`, `other-alpn`, `revoked`).
    fn decide(&self, token: &str, alpn: &str) -> R<Grant>;
}

// ---------- XenoNet's side ----------

enum Waiting {
    Bi(Writer, Reader),
    /// A uni stream from the remote: read here, written to the actor.
    In(Reader),
    /// A uni stream to the remote: the actor writes it.
    Out(Writer),
}

struct Session {
    alpn: Alpn,
    grant: Grant,
    /// The grant's text, decided again as the session goes on; never
    /// logged or listed.
    token: String,
    secret: [u8; 16],
    events: mpsc::UnboundedSender<Value>,
    conns: Mutex<BTreeMap<u64, Arc<Link>>>,
    /// Ended (the actor went, or its grant was revoked): true once.
    ended: watch::Sender<bool>,
}

impl Session {
    fn send(&self, v: Value) {
        let _ = self.events.send(v);
    }

    fn conn_of(&self, link: &Arc<Link>) -> Option<u64> {
        self.conns.locked().iter().find(|(_, l)| Arc::ptr_eq(l, link)).map(|(c, _)| *c)
    }
}

#[derive(Default)]
struct State {
    sessions: BTreeMap<Alpn, Arc<Session>>,
    /// Streams waiting to be taken, by number, with their session.
    waiting: BTreeMap<u64, (Weak<Session>, Waiting)>,
}

/// The actors' forwards on this machine.
pub struct Forwards {
    this: Weak<Forwards>,
    forwardable: Forwardable,
    grants: Arc<dyn Grants>,
    mesh: RwLock<Option<Arc<Mesh>>>,
    state: Mutex<State>,
    next: AtomicU64,
    /// Where the actors reach these forwards (the local socket), or why
    /// they can't.
    socket: Mutex<Option<Result<String, String>>>,
    chains: RwLock<Option<Chains>>,
}

impl Forwards {
    pub fn new(forwardable: Forwardable, grants: Arc<dyn Grants>) -> Arc<Forwards> {
        Arc::new_cyclic(|this| Forwards {
            this: this.clone(),
            forwardable,
            grants,
            mesh: RwLock::new(None),
            state: Mutex::default(),
            next: AtomicU64::new(1),
            socket: Mutex::new(None),
            chains: RwLock::new(None),
        })
    }

    pub fn forwardable(&self) -> &Forwardable {
        &self.forwardable
    }

    /// The mesh connections go through, once the network's endpoint runs;
    /// None when it stops. Sessions are ended when it changes.
    pub fn set_mesh(&self, mesh: Option<Arc<Mesh>>) {
        *self.mesh.writing() = mesh;
        let sessions: Vec<Arc<Session>> = self.state.locked().sessions.values().cloned().collect();
        for s in sessions {
            self.end(&s, Close::Done, None);
        }
    }

    fn id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// The actors forwarding now: each ALPN, its actor and its connections.
    pub fn list(&self) -> Value {
        let sessions: Vec<Arc<Session>> = self.state.locked().sessions.values().cloned().collect();
        let rows: Vec<Value> = sessions
            .iter()
            .map(|s| {
                let conns: Vec<Value> = s
                    .conns
                    .locked()
                    .values()
                    .filter(|l| !l.is_closed())
                    .map(|l| json!({ "peer": b64(&l.peer().0), "networks": l.networks(), "dialledByThis": l.dialled_by() != l.peer() }))
                    .collect();
                json!({ "alpn": s.alpn.name(), "actor": s.grant.actor, "connections": conns })
            })
            .collect();
        let socket = match &*self.socket.locked() {
            Some(Ok(at)) => json!({ "at": at }),
            Some(Err(why)) => json!({ "error": why }),
            None => Value::Null,
        };
        json!({ "forwardable": self.forwardable.alpns().iter().map(|a| a.name()).collect::<Vec<_>>(), "socket": socket, "forwards": rows })
    }

    /// Where the actors reach these forwards, or why they can't: what the
    /// list says, so a socket that couldn't be opened shows where the
    /// person looks for forwards.
    pub fn set_socket(&self, at: Result<String, String>) {
        *self.socket.locked() = Some(at);
    }

    /// Where the copies of this machine's chains the actors ask for come
    /// from: the composition root's.
    pub fn set_chains(&self, chains: Chains) {
        *self.chains.writing() = Some(chains);
    }

    fn chains_now(&self) -> Value {
        self.chains.reading().as_ref().map_or_else(|| json!({ "networks": [], "persons": [], "roots": [] }), |c| c())
    }

    /// A grant was revoked, by one of its revocation ids (hex): every
    /// session on it ends, and its connections close.
    pub fn revoked(&self, id: &str) {
        let hit: Vec<Arc<Session>> = self.state.locked().sessions.values().filter(|s| s.grant.ids.iter().any(|i| i == id)).cloned().collect();
        for s in hit {
            self.end(&s, Close::Refused, Some("revoked"));
        }
    }

    // Ends a session: its connections close with `why`, what waits for it
    // goes, and the actor hears `said` before its socket closes.
    fn end(&self, s: &Arc<Session>, why: Close, said: Option<&str>) {
        {
            let mut st = self.state.locked();
            if st.sessions.get(&s.alpn).is_some_and(|x| Arc::ptr_eq(x, s)) {
                st.sessions.remove(&s.alpn);
            }
            st.waiting.retain(|_, (w, _)| w.upgrade().is_some_and(|w| !Arc::ptr_eq(&w, s)));
        }
        for l in std::mem::take(&mut *s.conns.locked()).into_values() {
            l.close(why);
        }
        if let Some(t) = said {
            s.send(json!({ "t": t }));
        }
        s.ended.send_replace(true);
    }

    // Whether a session's grant still stands: decided again, as at attach.
    fn stands(&self, s: &Session) -> bool {
        self.grants.decide(&s.token, s.alpn.name()).is_ok_and(|g| g.actor == s.grant.actor)
    }

    // The session on `alpn`, if its grant still stands; one whose grant
    // doesn't is ended here.
    fn session(&self, alpn: Alpn) -> Option<Arc<Session>> {
        let s = self.state.locked().sessions.get(&alpn).cloned()?;
        if self.stands(&s) {
            Some(s)
        } else {
            self.end(&s, Close::Refused, Some("revoked"));
            None
        }
    }

    /// A connection a member dialled in on a forwardable ALPN (the mesh
    /// admitted it): to the actor attached for it, or closed.
    pub fn incoming(&self, link: Arc<Link>) {
        let Some(s) = self.session(link.alpn()) else {
            link.close(Close::Refused);
            return;
        };
        let conn = self.adopt(&s, &link);
        s.send(json!({ "t": "incoming", "conn": conn, "peer": b64(&link.peer().0), "networks": link.networks(), "exporter": exporter_of(&link) }));
    }

    // The connection's number in the session, and the task that hands its
    // streams to the actor, once.
    fn adopt(&self, s: &Arc<Session>, link: &Arc<Link>) -> u64 {
        if let Some(c) = s.conn_of(link) {
            return c;
        }
        let conn = self.id();
        s.conns.locked().insert(conn, link.clone());
        let (me, s, link) = (self.this.clone(), s.clone(), link.clone());
        tokio::spawn(async move {
            let mut ended = s.ended.subscribe();
            loop {
                let next = tokio::select! {
                    r = link.accept_bi() => r.map(|(w, r)| Waiting::Bi(w, r)),
                    r = link.accept_uni() => r.map(Waiting::In),
                    _ = ended.wait_for(|e| *e) => break,
                };
                let Some(f) = me.upgrade() else { break };
                match next {
                    Ok(w) => {
                        if !f.stands(&s) {
                            f.end(&s, Close::Refused, Some("revoked"));
                            break;
                        }
                        let bi = matches!(w, Waiting::Bi(..));
                        if let Some(stream) = f.wait(&s, w) {
                            s.send(json!({ "t": "stream", "conn": conn, "stream": stream, "bi": bi }));
                        }
                    }
                    Err(_) => break,
                }
            }
            let code = link.closed().await.map(Close::code);
            s.conns.locked().remove(&conn);
            s.send(json!({ "t": "closed", "conn": conn, "code": code }));
        });
        conn
    }

    // Holds a stream for the actor to take: its number, or None when the
    // session has too many waiting (the stream is dropped, so reset).
    fn wait(&self, s: &Arc<Session>, w: Waiting) -> Option<u64> {
        let stream = self.id();
        {
            let mut st = self.state.locked();
            let mine = st.waiting.values().filter(|(x, _)| x.upgrade().is_some_and(|x| Arc::ptr_eq(&x, s))).count();
            if mine >= MAX_WAITING {
                return None;
            }
            st.waiting.insert(stream, (Arc::downgrade(s), w));
        }
        let me = self.this.clone();
        tokio::spawn(async move {
            tokio::time::sleep(TAKE_WAIT).await;
            if let Some(f) = me.upgrade() {
                f.state.locked().waiting.remove(&stream);
            }
        });
        Some(stream)
    }

    /// Serves one local connection: an actor's session, or one stream it
    /// takes. Returns when it ends.
    pub async fn serve(self: Arc<Self>, mut r: Reader, mut w: Writer) {
        let Ok(Some(first)) = tokio::time::timeout(TAKE_WAIT, read_record(&mut r)).await.unwrap_or(Ok(None)) else { return };
        let v: Value = serde_json::from_slice(&first).unwrap_or(Value::Null);
        match v.get("t").and_then(Value::as_str) {
            Some("attach") => self.attached(&v, r, w).await,
            Some("take") => self.take(&v, r, w).await,
            _ => {
                let _ = say_refused(&mut w, &bad("kind")).await;
            }
        }
    }

    fn attach(&self, v: &Value) -> R<(Arc<Session>, mpsc::UnboundedReceiver<Value>)> {
        if v.get("v").and_then(Value::as_u64) != Some(VERSION) {
            return Err(bad("version"));
        }
        let name = v.get("alpn").and_then(Value::as_str).ok_or_else(|| bad("alpn"))?;
        let alpn = self.forwardable.get(name).ok_or_else(|| refused("not-forwardable", format!("{name} isn't forwarded by this machine")))?;
        let token = v.get("token").and_then(Value::as_str).ok_or_else(|| refused("no-grant", "an attach comes with the actor's grant"))?;
        let grant = self.grants.decide(token, name)?;
        if self.mesh.reading().is_none() {
            return Err(NetError::new(503, "no-network", "this machine's network endpoint isn't running"));
        }
        let (events, rx) = mpsc::unbounded_channel();
        let s = Arc::new(Session { alpn, grant, token: token.to_string(), secret: random::<16>()?, events, conns: Mutex::default(), ended: watch::Sender::new(false) });
        {
            let mut st = self.state.locked();
            if st.sessions.contains_key(&alpn) {
                return Err(NetError::new(409, "taken", format!("another actor forwards {name} here: it ends first")));
            }
            st.sessions.insert(alpn, s.clone());
        }
        Ok((s, rx))
    }

    async fn attached(self: Arc<Self>, v: &Value, mut r: Reader, mut w: Writer) {
        let (s, mut rx) = match self.attach(v) {
            Ok(s) => s,
            Err(e) => {
                let _ = say_refused(&mut w, &e).await;
                return;
            }
        };
        let me = self.mesh.reading().as_ref().map_or(PeerId([0; 32]), |m| m.me());
        if write_json(&mut w, &json!({ "t": "attached", "me": b64(&me.0), "session": b64(&s.secret) })).await.is_err() {
            self.end(&s, Close::Done, None);
            return;
        }
        let mut ended = s.ended.subscribe();
        let writer = async {
            while let Some(e) = rx.recv().await {
                let last = e["t"] == "revoked";
                if write_json(&mut w, &e).await.is_err() || last {
                    break;
                }
            }
            let _ = w.shutdown().await;
        };
        let reader = async {
            loop {
                let rec = tokio::select! {
                    rec = read_record(&mut r) => rec,
                    _ = ended.wait_for(|e| *e) => break,
                };
                let Ok(Some(rec)) = rec else { break };
                let Ok(c) = serde_json::from_slice::<Value>(&rec) else { continue };
                self.command(&s, &c);
            }
            self.end(&s, Close::Done, None);
        };
        tokio::join!(writer, reader);
    }

    // One of the actor's commands on its session.
    fn command(&self, s: &Arc<Session>, c: &Value) {
        let req = c.get("req").and_then(Value::as_u64).unwrap_or(0);
        let fail = |e: &NetError| s.send(json!({ "t": "failed", "req": req, "code": e.code, "error": e.message }));
        if !self.stands(s) {
            self.end(s, Close::Refused, Some("revoked"));
            return;
        }
        match c.get("t").and_then(Value::as_str) {
            Some("dial") => {
                let Some(peer) = c.get("peer").and_then(Value::as_str).and_then(unb64_n::<32>).map(PeerId) else { return fail(&bad("peer")) };
                let Some(mesh) = self.mesh.reading().clone() else { return fail(&NetError::new(503, "no-network", "this machine's network endpoint isn't running")) };
                let (me, s) = (self.this.clone(), s.clone());
                tokio::spawn(async move {
                    // The mesh's admission refuses a machine that's in none
                    // of this machine's networks before anything goes out,
                    // and the ALPN is the session's alone.
                    match mesh.dial(peer, s.alpn).await {
                        Ok(link) => {
                            let Some(f) = me.upgrade() else { return };
                            let conn = f.adopt(&s, &link);
                            s.send(json!({ "t": "dialled", "req": req, "conn": conn, "networks": link.networks(), "exporter": exporter_of(&link) }));
                        }
                        Err(e) => s.send(json!({ "t": "failed", "req": req, "code": e.code, "error": e.message })),
                    }
                });
            }
            Some("open") => {
                let conn = c.get("conn").and_then(Value::as_u64).unwrap_or(0);
                let bi = c.get("bi").and_then(Value::as_bool).unwrap_or(true);
                let Some(link) = s.conns.locked().get(&conn).cloned() else { return fail(&NetError::new(404, "no-connection", "no such connection")) };
                let (me, s) = (self.this.clone(), s.clone());
                tokio::spawn(async move {
                    let opened = if bi { link.open_bi().await.map(|(w, r)| Waiting::Bi(w, r)) } else { link.open_uni().await.map(Waiting::Out) };
                    let Some(f) = me.upgrade() else { return };
                    match opened.map(|w| f.wait(&s, w)) {
                        Ok(Some(stream)) => s.send(json!({ "t": "opened", "req": req, "stream": stream })),
                        Ok(None) => s.send(json!({ "t": "failed", "req": req, "code": "busy", "error": "too many streams wait to be taken" })),
                        Err(e) => s.send(json!({ "t": "failed", "req": req, "code": e.code, "error": e.message })),
                    }
                });
            }
            Some("chains") => s.send(json!({ "t": "chains", "req": req, "chains": self.chains_now() })),
            Some("close") => {
                let conn = c.get("conn").and_then(Value::as_u64).unwrap_or(0);
                let why = c.get("code").and_then(Value::as_u64).and_then(Close::from_code).unwrap_or(Close::Done);
                if let Some(l) = s.conns.locked().remove(&conn) {
                    l.close(why);
                }
            }
            _ => {}
        }
    }

    // A stream the actor takes: its bytes, both ways, until either end.
    async fn take(self: Arc<Self>, v: &Value, r: Reader, mut w: Writer) {
        let secret = v.get("session").and_then(Value::as_str).and_then(unb64_n::<16>);
        let stream = v.get("stream").and_then(Value::as_u64);
        let found = {
            let mut st = self.state.locked();
            match (secret, stream) {
                (Some(secret), Some(n)) if st.waiting.get(&n).and_then(|(s, _)| s.upgrade()).is_some_and(|s| s.secret == secret) => st.waiting.remove(&n),
                _ => None,
            }
        };
        let Some((_, waiting)) = found else {
            let _ = say_refused(&mut w, &refused("no-stream", "no such stream waits for this session")).await;
            return;
        };
        if write_json(&mut w, &json!({ "t": "taken" })).await.is_err() {
            return;
        }
        pump(waiting, r, w).await;
    }
}

// Copies a taken stream's bytes: the actor's socket `(r, w)` and the QUIC
// stream, each way to its end.
async fn pump(waiting: Waiting, mut r: Reader, mut w: Writer) {
    async fn copy(from: &mut Reader, to: &mut Writer) {
        let _ = tokio::io::copy(from, to).await;
        let _ = to.shutdown().await;
    }
    match waiting {
        Waiting::Bi(mut qw, mut qr) => {
            tokio::join!(copy(&mut r, &mut qw), copy(&mut qr, &mut w));
        }
        Waiting::In(mut qr) => copy(&mut qr, &mut w).await,
        Waiting::Out(mut qw) => copy(&mut r, &mut qw).await,
    }
}

async fn write_json(w: &mut Writer, v: &Value) -> R<()> {
    write_record(w, v.to_string().as_bytes()).await
}

async fn say_refused(w: &mut Writer, e: &NetError) -> R<()> {
    write_json(w, &json!({ "t": "refused", "code": e.code, "status": e.status, "error": e.message })).await?;
    w.shutdown().await.map_err(|e| NetError::new(502, "link-closed", e.to_string()))
}

fn refusal(v: &Value) -> NetError {
    let code = match v.get("code").and_then(Value::as_str) {
        Some("not-forwardable") => "not-forwardable",
        Some("no-grant") => "no-grant",
        Some("other-alpn") => "other-alpn",
        Some("revoked") => "revoked",
        Some("expired") => "expired",
        Some("taken") => "taken",
        Some("no-network") => "no-network",
        Some("not-a-member") => "not-a-member",
        Some("no-stream") => "no-stream",
        Some("no-connection") => "no-connection",
        Some("busy") => "busy",
        Some("unreachable") => "unreachable",
        _ => "refused",
    };
    let status = v.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()).unwrap_or(403);
    NetError::new(status, code, v.get("error").and_then(Value::as_str).unwrap_or("XenoNet refused the forward").to_string())
}

// ---------- the actor's side ----------

/// How the actor reaches XenoNet's local side: a new stream each call.
pub type Connect = Arc<dyn Fn() -> Fut<'static, R<(Reader, Writer)>> + Send + Sync>;

/// XenoNet's owner-only socket at `path`, as a `Connect`.
#[cfg(unix)]
pub fn unix(path: std::path::PathBuf) -> Connect {
    Arc::new(move || {
        let path = path.clone();
        Box::pin(async move {
            let s = tokio::net::UnixStream::connect(&path).await.map_err(|e| NetError::new(503, "unreachable", format!("XenoNet's forward socket: {e}")))?;
            let (r, w) = s.into_split();
            Ok((Box::new(r) as Reader, Box::new(w) as Writer))
        })
    })
}

/// Two ends of an in-memory stream, as a `Connect` hands one out: for a
/// XenoNet in the same process (the tests).
pub fn duplex(forwards: Arc<Forwards>) -> Connect {
    Arc::new(move || {
        let f = forwards.clone();
        Box::pin(async move {
            let (a, b) = tokio::io::duplex(64 << 10);
            let (ar, aw): (ReadHalf<_>, WriteHalf<_>) = tokio::io::split(a);
            let (br, bw) = tokio::io::split(b);
            tokio::spawn(f.serve(Box::new(br), Box::new(bw)));
            Ok((Box::new(ar) as Reader, Box::new(aw) as Writer))
        })
    })
}

struct ConnState {
    bi: tokio::sync::Mutex<mpsc::UnboundedReceiver<u64>>,
    uni: tokio::sync::Mutex<mpsc::UnboundedReceiver<u64>>,
    to_bi: mpsc::UnboundedSender<u64>,
    to_uni: mpsc::UnboundedSender<u64>,
    closed: watch::Sender<Shut>,
}

/// Whether a forwarded connection is open, or closed and with which of
/// `Close`'s codes, if one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shut {
    Open,
    Closed(Option<Close>),
}

impl Shut {
    fn is_closed(self) -> bool {
        self != Shut::Open
    }
}

impl ConnState {
    fn new() -> Arc<ConnState> {
        let (to_bi, bi) = mpsc::unbounded_channel();
        let (to_uni, uni) = mpsc::unbounded_channel();
        Arc::new(ConnState { bi: tokio::sync::Mutex::new(bi), uni: tokio::sync::Mutex::new(uni), to_bi, to_uni, closed: watch::Sender::new(Shut::Open) })
    }

    fn close(&self, why: Option<Close>) {
        self.closed.send_if_modified(|c| {
            if *c == Shut::Open {
                *c = Shut::Closed(why);
                true
            } else {
                false
            }
        });
    }
}

struct Client {
    me: PeerId,
    alpn: Alpn,
    session: [u8; 16],
    connect: Connect,
    to: mpsc::UnboundedSender<Value>,
    asks: Mutex<BTreeMap<u64, oneshot::Sender<Value>>>,
    next: AtomicU64,
    conns: Mutex<BTreeMap<u64, Arc<ConnState>>>,
    /// What XenoNet's header said of each peer's latest connection: the
    /// networks both are in, by XenoNet's chains.
    headers: Mutex<BTreeMap<PeerId, BTreeSet<String>>>,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<Arc<dyn Wire>>>,
    /// The session ended (the socket closed, or the grant was revoked).
    ended: watch::Sender<bool>,
}

impl Client {
    async fn ask(&self, mut v: Value) -> R<Value> {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        v["req"] = json!(req);
        let (tx, rx) = oneshot::channel();
        self.asks.locked().insert(req, tx);
        self.to.send(v).map_err(|_| ended())?;
        let a = rx.await.map_err(|_| ended())?;
        if a["t"] == "failed" {
            return Err(refusal(&a));
        }
        Ok(a)
    }

    // One stream, taken by its number on a socket connection of its own.
    async fn take(&self, stream: u64) -> R<(Reader, Writer)> {
        let (mut r, mut w) = (self.connect)().await?;
        write_json(&mut w, &json!({ "t": "take", "v": VERSION, "session": b64(&self.session), "stream": stream })).await?;
        let a = read_record(&mut r).await?.ok_or_else(ended)?;
        let a: Value = serde_json::from_slice(&a).map_err(|_| bad("answer"))?;
        if a["t"] != "taken" {
            return Err(refusal(&a));
        }
        Ok((r, w))
    }
}

fn ended() -> NetError {
    NetError::new(502, "link-closed", "XenoNet's forward ended")
}

/// The actor's end of a forward: a `Dial` over XenoNet's local side, on
/// the one ALPN its grant names.
pub struct ForwardDial(Arc<Client>);

impl ForwardDial {
    /// Attaches with `token` for `alpn`: refused unless XenoNet forwards
    /// that ALPN and the grant names it.
    pub async fn attach(connect: Connect, token: &str, alpn: Alpn) -> R<ForwardDial> {
        let (mut r, mut w) = connect().await?;
        write_json(&mut w, &json!({ "t": "attach", "v": VERSION, "token": token, "alpn": alpn.name() })).await?;
        let first = read_record(&mut r).await?.ok_or_else(ended)?;
        let a: Value = serde_json::from_slice(&first).map_err(|_| bad("answer"))?;
        if a["t"] != "attached" {
            return Err(refusal(&a));
        }
        let me = a.get("me").and_then(Value::as_str).and_then(unb64_n::<32>).map(PeerId).ok_or_else(|| bad("me"))?;
        let session = a.get("session").and_then(Value::as_str).and_then(unb64_n::<16>).ok_or_else(|| bad("session"))?;
        let (to, mut out) = mpsc::unbounded_channel::<Value>();
        let (to_incoming, incoming) = mpsc::unbounded_channel();
        let client = Arc::new(Client {
            me,
            alpn,
            session,
            connect,
            to,
            asks: Mutex::default(),
            next: AtomicU64::new(1),
            conns: Mutex::default(),
            headers: Mutex::default(),
            incoming: tokio::sync::Mutex::new(incoming),
            ended: watch::Sender::new(false),
        });
        tokio::spawn(async move {
            while let Some(v) = out.recv().await {
                if write_json(&mut w, &v).await.is_err() {
                    break;
                }
            }
            let _ = w.shutdown().await;
        });
        let c = Arc::downgrade(&client);
        tokio::spawn(async move {
            while let Ok(Some(rec)) = read_record(&mut r).await {
                let Some(c) = c.upgrade() else { return };
                let Ok(v) = serde_json::from_slice::<Value>(&rec) else { continue };
                c.event(&v, &to_incoming);
            }
            if let Some(c) = c.upgrade() {
                c.end();
            }
        });
        Ok(ForwardDial(client))
    }

    /// Whether the session ended: the socket closed, or the grant was
    /// revoked.
    pub fn is_ended(&self) -> bool {
        *self.0.ended.borrow()
    }

    /// Waits for the session to end.
    pub async fn ended(&self) {
        let mut e = self.0.ended.subscribe();
        let _ = e.wait_for(|e| *e).await;
    }

    /// The networks XenoNet's header named for `peer`'s latest connection:
    /// the ones both machines are in, by XenoNet's chains.
    pub fn networks(&self, peer: PeerId) -> BTreeSet<String> {
        self.0.headers.locked().get(&peer).cloned().unwrap_or_default()
    }

    /// Copies of the chains XenoNet holds now (`chains`), for the actor to
    /// verify.
    pub async fn chains(&self) -> R<Value> {
        let a = self.0.ask(json!({ "t": "chains" })).await?;
        Ok(a.get("chains").cloned().unwrap_or(Value::Null))
    }
}

impl Client {
    fn wire(self: &Arc<Self>, conn: u64, peer: PeerId, header: &Value) -> Arc<ForwardWire> {
        let state = ConnState::new();
        self.conns.locked().insert(conn, state.clone());
        let exporter = header.get("exporter").and_then(Value::as_str).and_then(unb64_n::<32>);
        Arc::new(ForwardWire { conn, peer, alpn: self.alpn, client: self.clone(), state, exporter })
    }

    fn event(self: &Arc<Self>, v: &Value, incoming: &mpsc::UnboundedSender<Arc<dyn Wire>>) {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        match v.get("t").and_then(Value::as_str) {
            Some("incoming") => {
                let Some(peer) = v.get("peer").and_then(Value::as_str).and_then(unb64_n::<32>).map(PeerId) else { return };
                self.headers.locked().insert(peer, networks_of(v));
                let _ = incoming.send(self.wire(n("conn"), peer, v) as Arc<dyn Wire>);
            }
            Some("dialled" | "failed" | "opened" | "chains") => {
                if let Some(tx) = self.asks.locked().remove(&n("req")) {
                    let _ = tx.send(v.clone());
                }
            }
            Some("stream") => {
                if let Some(c) = self.conns.locked().get(&n("conn")) {
                    let _ = if v["bi"] == true { c.to_bi.send(n("stream")) } else { c.to_uni.send(n("stream")) };
                }
            }
            Some("closed") => {
                if let Some(c) = self.conns.locked().remove(&n("conn")) {
                    c.close(v.get("code").and_then(Value::as_u64).and_then(Close::from_code));
                }
            }
            Some("revoked") => self.end(),
            _ => {}
        }
    }

    fn end(&self) {
        for c in std::mem::take(&mut *self.conns.locked()).into_values() {
            c.close(Some(Close::Refused));
        }
        self.asks.locked().clear();
        self.ended.send_replace(true);
    }
}

/// A forwarded connection, at the actor's end.
pub struct ForwardWire {
    conn: u64,
    peer: PeerId,
    alpn: Alpn,
    client: Arc<Client>,
    state: Arc<ConnState>,
    /// The connection's keying material, as XenoNet's header gave it.
    exporter: Option<[u8; 32]>,
}

impl std::fmt::Debug for ForwardWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ForwardWire({} {:?} #{})", self.peer, self.alpn, self.conn)
    }
}

impl ForwardWire {
    async fn open(&self, bi: bool) -> R<(Reader, Writer)> {
        if self.is_closed() {
            return Err(ended());
        }
        let a = self.client.ask(json!({ "t": "open", "conn": self.conn, "bi": bi })).await?;
        self.client.take(a.get("stream").and_then(Value::as_u64).ok_or_else(|| bad("stream"))?).await
    }

    async fn next(&self, bi: bool) -> R<(Reader, Writer)> {
        let mut closed = self.state.closed.subscribe();
        let stream = {
            let mut q = if bi { self.state.bi.lock().await } else { self.state.uni.lock().await };
            tokio::select! {
                s = q.recv() => s.ok_or_else(ended)?,
                _ = closed.wait_for(|s| s.is_closed()) => return Err(ended()),
            }
        };
        self.client.take(stream).await
    }
}

impl Wire for ForwardWire {
    fn peer(&self) -> PeerId {
        self.peer
    }

    fn alpn(&self) -> Alpn {
        self.alpn
    }

    fn open_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move { self.open(true).await.map(|(r, w)| (w, r)) })
    }

    fn accept_bi(&self) -> Fut<'_, R<(Writer, Reader)>> {
        Box::pin(async move { self.next(true).await.map(|(r, w)| (w, r)) })
    }

    fn open_uni(&self) -> Fut<'_, R<Writer>> {
        Box::pin(async move { self.open(false).await.map(|(_, w)| w) })
    }

    fn accept_uni(&self) -> Fut<'_, R<Reader>> {
        Box::pin(async move { self.next(false).await.map(|(r, _)| r) })
    }

    fn close(&self, why: Close) {
        let _ = self.client.to.send(json!({ "t": "close", "conn": self.conn, "code": why.code() }));
        self.client.conns.locked().remove(&self.conn);
        self.state.close(Some(why));
    }

    fn closed(&self) -> Fut<'_, Option<Close>> {
        Box::pin(async move {
            let mut c = self.state.closed.subscribe();
            match c.wait_for(|s| s.is_closed()).await.map(|v| *v) {
                Ok(Shut::Closed(why)) => why,
                _ => None,
            }
        })
    }

    fn is_closed(&self) -> bool {
        self.state.closed.borrow().is_closed()
    }

    // Derived from the header's, under `label`: both actors derive the
    // same bytes from the one QUIC connection their XenoNets terminate.
    fn exporter(&self, label: &[u8]) -> Option<[u8; 32]> {
        let x = self.exporter?;
        let mut m = b"xenonet forward exporter\n".to_vec();
        m.extend_from_slice(label);
        m.push(b'\n');
        m.extend_from_slice(&x);
        Some(xenonet_id::codec::sha256(&m))
    }
}

impl Dial for ForwardDial {
    fn me(&self) -> PeerId {
        self.0.me
    }

    fn connect(&self, peer: PeerId, alpn: Alpn) -> Fut<'_, R<Arc<dyn Wire>>> {
        Box::pin(async move {
            if alpn != self.0.alpn {
                return Err(refused("other-alpn", format!("this forward carries {} alone", self.0.alpn.name())));
            }
            let a = self.0.ask(json!({ "t": "dial", "peer": b64(&peer.0) })).await?;
            let conn = a.get("conn").and_then(Value::as_u64).ok_or_else(|| bad("conn"))?;
            self.0.headers.locked().insert(peer, networks_of(&a));
            Ok(self.0.wire(conn, peer, &a) as Arc<dyn Wire>)
        })
    }

    fn incoming(&self) -> Fut<'_, Option<Arc<dyn Wire>>> {
        Box::pin(async move {
            let mut ended = self.0.ended.subscribe();
            let mut rx = self.0.incoming.lock().await;
            tokio::select! {
                w = rx.recv() => w,
                _ = ended.wait_for(|e| *e) => None,
            }
        })
    }

    // What the actor's own mesh offers is XenoNet's to decide: it admits
    // the members of its networks on the forwarded ALPN.
    fn offer(&self, _alpns: &[Alpn]) {}

    fn close(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            self.0.end();
        })
    }
}

fn networks_of(v: &Value) -> BTreeSet<String> {
    v.get("networks").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;
