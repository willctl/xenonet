//! svc/2 (`xenonet/svc/2`): requests between members of a network, one
//! QUIC bi stream for each, on the connection the mesh admitted.
//!
//! **A stream** carries one head, a link/2 record (a u32 length and that
//! much JSON, `network/svc/v2`), then as many bytes of body as the head's
//! `length` says, at most 8 MiB, and the caller finishes its side. The
//! answer comes back the same way on the same stream: its head, then its
//! body. Two kinds of head go out:
//!
//! - `request`: a call to a service a member published (`svc`), by name.
//! - `window`: a read of this machine's own API for the person's window on
//!   another machine (a session's transcript or stats), with no body.
//!
//! **Who is calling** is the connection's endpoint id, as the handshake
//! authenticated it, and nothing in the head: a head names the network it
//! is in, and the connection must carry that network by this machine's
//! chains (`Link::check`), so a service published in N1 never answers a
//! member of N2 alone over a connection the two share. The answering side
//! maps the endpoint id to the member in that network's chain (`Served`).
//!
//! **Window reads** ([`WindowCert`], [`WindowRead`]). A read of another
//! machine's transcript is the person's, from their window, so it carries
//! their device's word for it. Asking the device for every read would ask
//! Touch ID for every page of a transcript, so the window's machine makes
//! a key per launch (in memory, never written) and has the device certify
//! it once: the person, the device, the key, the endpoint id of the
//! machine it's for, and 12 hours. Each read is signed by that key over
//! the network, both endpoint ids, a nonce, its time and the path. The
//! answering machine takes it only over a connection from the endpoint
//! the cert names, within two minutes of its time, and once by its nonce.
//! The cert is no good to another machine (its connection is from another
//! endpoint), and a read recorded off a connection is no good twice.

use crate::codec::{b64, random, unb64};
use crate::dial::{Alpn, Fut, Link, Mesh, PeerId, Reader, Writer};
use crate::link2::{read_record, write_record};
use crate::svc::{Answer, Request, ANSWER_BODY, REQUEST_BODY, SCHEMA};
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenonet_id::codec::hex;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer, VerifyingKey};

/// How long a stream's head may take to come.
pub const HEAD_WAIT: Duration = Duration::from_secs(10);
/// How long a body may take to come once its head did.
pub const BODY_WAIT: Duration = Duration::from_secs(60);
/// How long a dial to the publishing machine may take.
pub const DIAL_WAIT: Duration = Duration::from_secs(10);
/// Streams one connection may have open at once on the answering side.
pub const STREAMS: usize = 64;
/// How long a window key's cert holds.
pub const CERT_MS: i64 = 12 * 3600 * 1000;
/// A cert is made again when less than this is left of it.
pub const CERT_RENEW_MS: i64 = 10 * 60 * 1000;
/// How far a window read's time may be from the answering machine's
/// clock.
pub const READ_SKEW_MS: i64 = 2 * 60 * 1000;
/// Nonces kept at most: past this the oldest go, which is safe once they
/// are past their read's time.
const NONCES: usize = 65_536;

/// A window read's cert, signed by the person's device (`Purpose::WINDOW`).
pub const CERT: Purpose = Purpose::WINDOW;
/// A window read, signed by the window key (`Purpose::WINDOW_READ`).
pub const READ: Purpose = Purpose::WINDOW_READ;

fn bad(what: &str) -> NetError {
    NetError::bad(format!("a svc/2 {what} isn't right"))
}

fn io(e: std::io::Error) -> NetError {
    NetError::new(502, "stream-closed", format!("a svc/2 stream ended: {e}"))
}

// ---------- the stream ----------

/// Writes a head and its body, and finishes this side of the stream.
async fn send(w: &mut Writer, head: &Value, body: &[u8]) -> R<()> {
    write_record(w, head.to_string().as_bytes()).await?;
    w.write_all(body).await.map_err(io)?;
    w.shutdown().await.map_err(io)
}

/// Reads a head: its JSON.
async fn head_of(r: &mut Reader) -> R<Value> {
    let bytes = read_record(r).await?.ok_or_else(|| bad("head"))?;
    serde_json::from_slice(&bytes).map_err(|_| bad("head"))
}

/// Reads exactly `length` bytes of body.
async fn body_of(r: &mut Reader, length: usize) -> R<Vec<u8>> {
    let mut body = vec![0u8; length];
    r.read_exact(&mut body).await.map_err(io)?;
    Ok(body)
}

/// What answers the requests that come on svc/2: the node, for each
/// network this machine is in.
pub trait Served: Send + Sync {
    /// A request for a service published in `network`, from `peer`, which
    /// the connection carries `network` for.
    fn service(&self, network: &str, peer: PeerId, req: Request) -> Fut<'_, Answer>;
    /// A window read in `network`, from `peer`, which the connection
    /// carries `network` for.
    fn window(&self, network: &str, peer: PeerId, read: WindowRead) -> Fut<'_, Answer>;
}

/// Answers the streams `link` brings, each in a task of its own, until it
/// closes.
pub async fn serve(link: Arc<Link>, served: Arc<dyn Served>) {
    let open = Arc::new(tokio::sync::Semaphore::new(STREAMS));
    loop {
        let Ok((w, r)) = link.accept_bi().await else { return };
        let Ok(permit) = open.clone().try_acquire_owned() else {
            // One peer holds every slot: this stream is answered busy.
            tokio::spawn(async move {
                let mut w = w;
                let _ = send(&mut w, &Answer::refused(503, "busy", format!("this machine answers {STREAMS} svc/2 streams from one machine at once, at most")).head(), &[]).await;
            });
            continue;
        };
        let (link, served) = (link.clone(), served.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let mut w = w;
            let a = one(&link, r, served.as_ref()).await;
            let _ = send(&mut w, &a.head(), &a.body).await;
        });
    }
}

/// One stream's answer.
async fn one(link: &Link, mut r: Reader, served: &dyn Served) -> Answer {
    let head = match tokio::time::timeout(HEAD_WAIT, head_of(&mut r)).await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return Answer::refused(400, "bad-request", e.message),
        Err(_) => return Answer::refused(408, "timeout", "the request's head didn't come in time"),
    };
    if head.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Answer::refused(400, "bad-request", format!("a svc/2 head is {SCHEMA}"));
    }
    let Some(network) = crate::svc::network_of(&head).map(str::to_string) else { return Answer::refused(400, "bad-request", "a svc/2 head names its network") };
    // The caller is the connection's peer, in the network the head names
    // by this machine's chain.
    if let Err(e) = link.check(&network) {
        return Answer::refused(403, "not-in-network", e.message);
    }
    match head.get("op").and_then(Value::as_str) {
        Some("request") => {
            let (mut req, length) = match Request::parse_head(&head) {
                Ok(x) => x,
                Err(why) => return Answer::refused(400, "bad-request", why),
            };
            if length > REQUEST_BODY {
                return Answer::refused(413, "too-big", "a request's body is at most 8 MiB");
            }
            req.body = match tokio::time::timeout(BODY_WAIT, body_of(&mut r, length)).await {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => return Answer::refused(400, "bad-request", e.message),
                Err(_) => return Answer::refused(408, "timeout", "the request's body didn't come in time"),
            };
            served.service(&network, link.peer(), req).await
        }
        Some("window") => match WindowRead::parse(&head) {
            Ok(read) => served.window(&network, link.peer(), read).await,
            Err(why) => Answer::refused(400, "bad-request", why),
        },
        _ => Answer::refused(400, "bad-request", "a svc/2 head is a request or a window read"),
    }
}

/// Sends `head` and `body` to `peer` on svc/2, over `mesh`, and waits up to
/// `wait` for the answer. Every way it fails is an answer: 503 when the
/// peer can't be reached.
pub async fn call(mesh: &Mesh, peer: PeerId, network: &str, head: &Value, body: &[u8], wait: Duration) -> Answer {
    let link = match tokio::time::timeout(DIAL_WAIT, mesh.dial(peer, Alpn::Svc)).await {
        Ok(Ok(l)) => l,
        Ok(Err(e)) if e.status == 403 => return Answer::refused(403, "not-in-network", e.message),
        Ok(Err(e)) => return Answer::refused(503, "offline", format!("that machine can't be reached now: {}", e.message)),
        Err(_) => return Answer::refused(503, "offline", "that machine didn't answer"),
    };
    if let Err(e) = link.check(network) {
        return Answer::refused(403, "not-in-network", e.message);
    }
    let exchange = async {
        let (mut w, mut r) = link.open_bi().await?;
        send(&mut w, head, body).await?;
        let h = head_of(&mut r).await?;
        let (mut a, length) = Answer::parse_head(&h).map_err(NetError::bad)?;
        if length > ANSWER_BODY {
            return Ok(Answer::refused(413, "answer-too-big", "the service's answer is over 8 MiB"));
        }
        a.body = body_of(&mut r, length).await?;
        Ok::<Answer, NetError>(a)
    };
    match tokio::time::timeout(wait, exchange).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => Answer::refused(502, "dropped", format!("the request was dropped before its answer came: {}", e.message)),
        Err(_) => Answer::refused(504, "timeout", format!("no answer within {} s", wait.as_secs())),
    }
}

// ---------- window reads ----------

/// What a window read asks for: a session's transcript (with `cursor` and
/// `limit`) or its stats, on the answering machine's own API. The path to
/// read, or why not. A machine that shares titles only with the network
/// answers a transcript with 403.
pub fn window_path(path: &str, titles_only: bool) -> Result<String, Answer> {
    let refused = |why: &str| Answer::refused(403, "path-refused", why.to_string());
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let rest = route.strip_prefix("/api/session/").ok_or_else(|| refused("a window reads a session's transcript or stats, and nothing else"))?;
    let (session, what) = rest.split_once('/').ok_or_else(|| refused("a window reads a session's transcript or stats, and nothing else"))?;
    // A session by its agent's agt_, which this machine maps to its own.
    if !(is_session(session) || xenonet_model::ids::AgentId::parse(session).is_ok()) {
        return Err(Answer::refused(400, "bad-path", "that isn't a session's id"));
    }
    match what {
        "transcript" => {
            if titles_only {
                return Err(Answer::refused(403, "titles-only", "this machine shares titles only with the network: its transcripts stay on it"));
            }
            for pair in query.split('&').filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                let ok = match k {
                    "limit" => (1..=4).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()),
                    "cursor" => (1..=200).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-')),
                    _ => false,
                };
                if !ok {
                    return Err(Answer::refused(400, "bad-path", format!("a transcript doesn't take {k} that way")));
                }
            }
            Ok(path.to_string())
        }
        "stats" if query.is_empty() => Ok(route.to_string()),
        _ => Err(refused("a window reads a session's transcript or stats, and nothing else")),
    }
}

// A session's id on the machine read: an app's id, a colon, and up to 128
// letters, digits, `.`, `_` and `-`. Which apps there are is the
// adapters', not this crate's: the answering machine's own routes refuse
// a session it doesn't know.
fn is_session(local: &str) -> bool {
    let Some((app, rest)) = local.split_once(':') else { return false };
    let b = rest.as_bytes();
    xenonet_model::ids::is_app_id(app)
        && (1..=128).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// The person's device's word that a window key reads for them, from one
/// machine's endpoint, for 12 hours at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCert {
    pub person: String,
    /// The device's key id, in the person's chain.
    pub device: String,
    /// The window key.
    pub key: VerifyingKey,
    /// The endpoint id of the machine whose window it is.
    pub endpoint: PeerId,
    pub at: i64,
    pub until: i64,
    pub sig: Vec<u8>,
}

impl WindowCert {
    /// What the device signs.
    pub fn signed(person: &str, device: &str, key: &VerifyingKey, endpoint: PeerId, at: i64, until: i64) -> Vec<u8> {
        format!("{person}\n{device}\n{}\n{}\n{at}\n{until}", b64(key.bytes()), hex(&endpoint.0)).into_bytes()
    }

    pub fn to_json(&self) -> Value {
        json!({ "person": self.person, "device": self.device, "key": self.key.to_json(), "endpoint": hex(&self.endpoint.0), "at": self.at, "until": self.until, "sig": b64(&self.sig) })
    }

    pub fn parse(v: &Value) -> Option<WindowCert> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let endpoint = text("endpoint").filter(|h| h.len() == 64).and_then(|h| (0..32).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect::<Option<Vec<u8>>>())?;
        Some(WindowCert {
            person: text("person").filter(|p| p.len() <= 128)?.to_string(),
            device: text("device").filter(|d| d.len() <= 128)?.to_string(),
            key: VerifyingKey::parse(v.get("key")?)?,
            endpoint: PeerId(endpoint.try_into().ok()?),
            at: v.get("at")?.as_i64()?,
            until: v.get("until")?.as_i64()?,
            sig: unb64(text("sig")?)?,
        })
    }

    /// Whether `device` signed it.
    pub fn verify(&self, device: &VerifyingKey) -> bool {
        device.verify(CERT, &WindowCert::signed(&self.person, &self.device, &self.key, self.endpoint, self.at, self.until), &self.sig)
    }
}

/// One read for the window, signed by its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRead {
    pub network: String,
    pub path: String,
    pub nonce: String,
    pub at: i64,
    pub cert: WindowCert,
    pub sig: Vec<u8>,
}

impl WindowRead {
    /// What the window key signs: the network, the caller's and the
    /// answerer's endpoint ids, the nonce, the time and the path.
    pub fn signed(network: &str, from: PeerId, to: PeerId, nonce: &str, at: i64, path: &str) -> Vec<u8> {
        format!("{network}\n{}\n{}\n{nonce}\n{at}\n{path}", hex(&from.0), hex(&to.0)).into_bytes()
    }

    pub fn head(&self) -> Value {
        json!({ "schema": SCHEMA, "op": "window", "network": self.network, "path": self.path, "nonce": self.nonce, "at": self.at, "cert": self.cert.to_json(), "sig": b64(&self.sig), "length": 0 })
    }

    pub fn parse(v: &Value) -> Result<WindowRead, String> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        if text("schema") != Some(SCHEMA) || text("op") != Some("window") {
            return Err("that isn't a window read".into());
        }
        Ok(WindowRead {
            network: crate::svc::network_of(v).ok_or("a window read names its network")?.to_string(),
            path: text("path").filter(|p| p.len() <= 4096).ok_or("a window read's path is at most 4 KiB")?.to_string(),
            nonce: text("nonce").filter(|n| (16..=64).contains(&n.len()) && crate::codec::is_b64(n)).ok_or("a window read's nonce is 16 to 64 base64url characters")?.to_string(),
            at: v.get("at").and_then(Value::as_i64).ok_or("a window read says when it was made")?,
            cert: v.get("cert").and_then(WindowCert::parse).ok_or("a window read carries its key's cert")?,
            sig: text("sig").and_then(unb64).ok_or("a window read is signed")?,
        })
    }
}

/// The window's key on this machine: made once a launch, certified once by
/// the person's device, kept in memory only.
pub struct WindowKey {
    signer: SeedSigner,
    pub cert: WindowCert,
}

impl WindowKey {
    /// A new key for the window of the machine at `endpoint`, certified by
    /// `sign` (the person's device, over `WindowCert::signed`, for `CERT`)
    /// for 12 hours from `now`.
    pub fn make(endpoint: PeerId, person: &str, device: &str, now: i64, sign: impl FnOnce(&[u8]) -> R<Vec<u8>>) -> R<WindowKey> {
        let signer = SeedSigner::from_seed(&random::<32>()?, KeyHome::File)?;
        let key = signer.public();
        let until = now + CERT_MS;
        let sig = sign(&WindowCert::signed(person, device, &key, endpoint, now, until))?;
        Ok(WindowKey { signer, cert: WindowCert { person: person.into(), device: device.into(), key, endpoint, at: now, until, sig } })
    }

    /// Whether it holds a while yet at `now`.
    pub fn fresh(&self, now: i64) -> bool {
        self.cert.until - now > CERT_RENEW_MS
    }

    /// A read of `path` on `to`, in `network`, at `now`.
    pub fn read(&self, network: &str, to: PeerId, path: &str, now: i64) -> R<WindowRead> {
        let nonce = b64(&random::<18>()?);
        let sig = self.signer.sign(READ, &WindowRead::signed(network, self.cert.endpoint, to, &nonce, now, path))?;
        Ok(WindowRead { network: network.into(), path: path.into(), nonce, at: now, cert: self.cert.clone(), sig })
    }
}

/// The nonces of the window reads this machine took lately, and their
/// times.
#[derive(Default)]
pub struct Nonces(HashMap<String, i64>);

impl Nonces {
    /// Takes `nonce` of a read made `at`, once.
    fn take(&mut self, nonce: &str, at: i64, now: i64) -> bool {
        if self.0.len() >= NONCES {
            self.0.retain(|_, t| (now - *t).abs() <= READ_SKEW_MS);
            if self.0.len() >= NONCES {
                return false;
            }
        }
        self.0.insert(nonce.to_string(), at).is_none()
    }
}

/// What the answering machine knows to check a window read against: its
/// own chains, for the network the read is in.
pub struct WindowCheck<'a> {
    /// This machine's endpoint id.
    pub me: PeerId,
    /// The connection's peer.
    pub peer: PeerId,
    /// The person the network is theirs (`found` named them).
    pub owner: Option<&'a str>,
    /// The person's device on the calling machine, by the network's chain.
    pub device_there: Option<&'a str>,
    /// The device's key, when it is one of the person's active devices.
    pub device_key: Option<VerifyingKey>,
    pub now: i64,
}

impl WindowCheck<'_> {
    /// Whether `read` stands: its cert is the owner's device's, for the
    /// endpoint the read came from, in its time; the read is signed by
    /// the cert's key, for this machine, within two minutes, and new.
    pub fn check(&self, read: &WindowRead, nonces: &mut Nonces) -> Result<(), Answer> {
        let refused = |code: &str, why: &str| Err(Answer::refused(403, code, why.to_string()));
        let c = &read.cert;
        if c.endpoint != self.peer {
            return refused("cert-elsewhere", "that window key's cert is for another machine than the one that sent it");
        }
        if self.owner != Some(c.person.as_str()) {
            return refused("not-the-person", "that window key isn't certified by the person this network is theirs");
        }
        if self.device_there != Some(c.device.as_str()) {
            return refused("not-its-device", "that window key isn't certified by the person's device on the machine that sent it");
        }
        let Some(device) = &self.device_key else { return refused("not-its-device", "the device that certified that window key isn't one of the person's now") };
        if !c.verify(device) {
            return refused("bad-cert", "that window key's cert isn't signed by the device it names");
        }
        if c.until <= c.at || c.until - c.at > CERT_MS || self.now < c.at - READ_SKEW_MS || self.now >= c.until {
            return refused("cert-expired", "that window key's cert isn't in its time");
        }
        if (self.now - read.at).abs() > READ_SKEW_MS {
            return refused("clock-skew", "that window read's time is more than two minutes from this machine's clock");
        }
        if !c.key.verify(READ, &WindowRead::signed(&read.network, self.peer, self.me, &read.nonce, read.at, &read.path), &read.sig) {
            return refused("bad-signature", "that window read isn't signed by its key, for this machine");
        }
        if !nonces.take(&read.nonce, read.at, self.now) {
            return Err(Answer::refused(409, "replayed", "that window read came before: each is taken once"));
        }
        Ok(())
    }
}

/// This machine's own API, as a window read reads it: the composition root
/// gives one (the service reading itself, as the relay did).
pub trait LocalReads: Send + Sync {
    /// The status and JSON of `path`, or why there was none and the status
    /// to answer with.
    fn get(&self, path: &str) -> Fut<'_, Result<(u16, Value), (String, u16)>>;
}

#[cfg(test)]
#[path = "svc2_tests.rs"]
mod tests;
