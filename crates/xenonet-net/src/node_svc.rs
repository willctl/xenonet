//! svc/2 on this machine (`svc2` has the protocol): calling another
//! member's service, the window's reads of another machine's transcripts,
//! and answering both.
//!
//! The streams come on the machine's one endpoint (link/2's), on its own
//! ALPN; each names the network it's in, and is answered by that network's
//! node, which takes the caller to be the member whose endpoint id the
//! connection authenticated in its own chain.

use super::{Arc, Common, Locked, Shared, Weak};
use crate::dial::{Fut, PeerId};
use crate::endpoint::peer_id;
use crate::svc::{self, Answer};
use crate::svc2::{self, WindowCheck, WindowKey, WindowRead};
use std::time::Duration;
use xenonet_model::ids::is_host_id;

/// How long the window waits for another machine's transcript.
const WINDOW_MS: u64 = 30_000;

/// The node's answers to svc/2's streams, by the network each names.
pub(crate) struct NodeServed(pub Weak<Common>);

impl NodeServed {
    fn of(&self, network: &str) -> Option<Arc<Shared>> {
        self.0.upgrade()?.shared_of(network)
    }
}

impl svc2::Served for NodeServed {
    fn service(&self, network: &str, peer: PeerId, req: svc::Request) -> Fut<'_, Answer> {
        let node = self.of(network);
        Box::pin(async move {
            match node {
                Some(n) => n.serve_service(peer, req).await,
                None => Answer::refused(404, "no-network", "this machine isn't in that network"),
            }
        })
    }

    fn window(&self, network: &str, peer: PeerId, read: WindowRead) -> Fut<'_, Answer> {
        let node = self.of(network);
        Box::pin(async move {
            match node {
                Some(n) => n.serve_window(peer, read).await,
                None => Answer::refused(404, "no-network", "this machine isn't in that network"),
            }
        })
    }
}

impl Shared {
    /// A program here asks for `name` on `host`: the answer, or why there's
    /// none. 503 when `host` can't be reached on svc/2 now; 504 when no
    /// answer comes in time.
    pub async fn svc_call(&self, host: &str, name: &str, call: svc::Call) -> Answer {
        let Some(net) = self.network() else { return Answer::refused(503, "no-network", "this machine isn't in a XenoNet network") };
        if self.is_removed() {
            return Answer::refused(503, "removed", "this machine was removed from its network");
        }
        if !is_host_id(host) || !svc::is_name(name) {
            return Answer::refused(400, "bad-request", "reach a service at /api/svc/<host>/<name>/<path>");
        }
        if host == net.host {
            return Answer::refused(400, "this-host", "that's this machine: reach the service at its own address");
        }
        let Some(member) = self.roster().view_ref().and_then(|v| v.member(host).cloned()) else {
            return Answer::refused(404, "no-host", format!("{host} isn't in the network"));
        };
        if !svc::METHODS.contains(&call.method.as_str()) {
            return Answer::refused(405, "method", format!("{} isn't passed on: a service is reached with {}", call.method, svc::METHODS.join(", ")));
        }
        if call.body.len() > svc::REQUEST_BODY {
            return Answer::refused(413, "too-big", "a request's body is at most 8 MiB");
        }
        let Some(t) = self.link2() else { return Answer::refused(503, "offline", "this machine's endpoint isn't running, so it reaches no other machine") };
        // A member with no link/2 connection that presence says is offline is
        // away: said now, rather than after a dial's wait.
        let peer = peer_id(&member.keys);
        if !t.links.is_connected(peer) && !self.peers.locked().get(host).is_some_and(|p| p.online) {
            return Answer::refused(503, "offline", format!("{host} is offline: this machine has no connection to it now"));
        }
        // This host stops waiting when `_wait` goes: with the answer, at
        // the timeout, or when the program that asked goes away, which
        // drops this call.
        let _wait = match self.svc.wait(host) {
            Ok(w) => w,
            Err(a) => return a,
        };
        let Ok(id) = svc::request_id() else { return Answer::refused(500, "internal", "no randomness for a request's id") };
        let req = svc::Request { id, name: name.to_string(), method: call.method, path: call.path, headers: svc::passed(&call.headers), body: call.body };
        let a = svc2::call(&t.mesh, peer, &net.id, &req.head(&net.id), &req.body, Duration::from_millis(self.opts.svc_ms)).await;
        if a.code.as_deref() == Some("offline") {
            return Answer::refused(503, "offline", format!("{host} can't be reached now: {}", a.message()));
        }
        a
    }

    /// A request from `peer` for a service here, in this network: checked,
    /// passed to the service, and answered.
    pub(crate) async fn serve_service(&self, peer: PeerId, req: svc::Request) -> Answer {
        let Some(from) = self.member_at(peer).map(|m| m.id) else { return Answer::refused(403, "not-in-network", "that machine isn't a member of this network") };
        let now = self.now();
        let name = req.name.clone();
        let answer = self.svc.serve(&from, req, now).await;
        if let Some(code) = answer.code.as_deref() {
            if self.svc.say(&format!("{from}|{name}|{code}"), now) {
                self.log("net.svc", &format!("a request from {from} to {name} was answered {}: {}", answer.status, answer.message()));
            }
        }
        answer
    }

    /// The window's read of `path` on `host` (a session's transcript or
    /// stats), over svc/2 under the window's key: None when this machine's
    /// endpoint isn't running, so the caller reads some other way. The
    /// first read of a launch asks the person's device once, for the key's
    /// cert; the routes let only the window here call this.
    pub async fn window_read(&self, host: &str, path: &str) -> Option<Answer> {
        let t = self.link2()?;
        let net = self.network()?;
        let Some(member) = self.roster().view_ref().and_then(|v| v.member(host).cloned()) else {
            return Some(Answer::refused(404, "no-host", format!("{host} isn't in the network")));
        };
        let now = self.now() as i64;
        let key = match self.window_key(t.mesh.me(), now) {
            Ok(k) => k,
            Err(e) => return Some(Answer::refused(e.status, e.code, e.message)),
        };
        let peer = peer_id(&member.keys);
        let read = match key.read(&net.id, peer, path, now) {
            Ok(r) => r,
            Err(e) => return Some(Answer::refused(500, "internal", e.message)),
        };
        Some(svc2::call(&t.mesh, peer, &net.id, &read.head(), &[], Duration::from_millis(WINDOW_MS)).await)
    }

    /// The window's key, certified for this machine's endpoint `me`: the
    /// one made this launch while it holds a while yet, or a new one, which
    /// the person's device certifies (it asks them once).
    fn window_key(&self, me: PeerId, now: i64) -> crate::R<Arc<WindowKey>> {
        let mut held = self.common.window.locked();
        if let Some(k) = held.as_ref().filter(|k| k.fresh(now) && k.cert.endpoint == me) {
            return Ok(k.clone());
        }
        let id = &self.identity;
        let person = id.person_id().ok_or_else(|| crate::NetError::new(409, "no-person", "this machine has no identity of yours, so its window reads no other machine"))?;
        let device = id.device_id().ok_or_else(|| crate::NetError::new(409, "no-device", "this machine has no device of yours, so its window reads no other machine"))?;
        let key =
            WindowKey::make(me, &person, &device, now, |msg| id.sign_as_person(true, svc2::CERT, msg, "read your other machines' transcripts in this window").map(|(_, sig)| sig))?;
        let key = Arc::new(key);
        *held = Some(key.clone());
        Ok(key)
    }

    /// A window read from `peer`, in this network: the person's window on
    /// a window host, under a key their device on it certified, read from
    /// this machine's own API.
    pub(crate) async fn serve_window(&self, peer: PeerId, read: WindowRead) -> Answer {
        let Some(net) = self.network() else { return Answer::refused(404, "no-network", "this machine isn't in that network") };
        let Some(from) = self.member_at(peer) else { return Answer::refused(403, "not-in-network", "that machine isn't a member of this network") };
        if !self.decide_read(&from.id) {
            return Answer::refused(403, "not-window", format!("{} isn't a window host here: only a window host's window reads this machine's transcripts", from.id));
        }
        let path = match svc2::window_path(&read.path, net.titles_only) {
            Ok(p) => p,
            Err(a) => return a,
        };
        let me = match self.link2() {
            Some(t) => t.mesh.me(),
            None => return Answer::refused(503, "offline", "this machine's endpoint isn't running"),
        };
        let now = self.now();
        {
            let view = self.roster().view();
            let owner = view.owner.clone();
            let device_key = owner.as_deref().and_then(|o| self.identity.active_device(o, &read.cert.device)).map(|d| d.key);
            let check = WindowCheck { me, peer, owner: owner.as_deref(), device_there: from.device.as_deref(), device_key, now: now as i64 };
            if let Err(a) = check.check(&read, &mut self.common.nonces.locked()) {
                if self.svc.say(&format!("{}|window|{}", from.id, a.code.as_deref().unwrap_or("")), now) {
                    self.log("net.svc", &format!("a window read from {} was refused: {}", from.id, a.message()));
                }
                return a;
            }
        }
        if let Err(a) = self.svc.rate(&from.id, "window", now) {
            return a;
        }
        let Some(reads) = self.common.reads.read().ok().and_then(|r| r.clone()) else {
            return Answer::refused(503, "unreadable", "this machine's API isn't open to window reads yet");
        };
        match reads.get(&path).await {
            Ok((status, body)) => Answer::json(status, &body),
            Err((why, status)) => Answer::refused(status, "unreadable", why),
        }
    }
}
