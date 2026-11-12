//! join/2 on this machine (`join` has the protocol): `xenonet net invite`,
//! which opens a ticket on the inviter's door, and `xenonet net join
//! <invite> <code>`, which dials the endpoint the invite names.
//!
//! The node isn't on the mesh yet, so each side runs an iroh endpoint of
//! its own for the join alone: the inviter's while any invite is open (it
//! closes with the last), the joiner's for its one attempt. After the
//! join, the new machine links through the network's gateway as before,
//! by the gateway's address the inviter sends, until the gateway goes
//! (phase 5's commit 20).
//!
//! Neither side reaches anyone's infrastructure unless the person chose
//! it: a relay only when the network's chain lists one (the invite names
//! the first), mDNS only with `lan`, and otherwise the address the person
//! gives (`at`), such as a tailnet or LAN address.

use super::{
    identity, is_host_id, json, not_in, ticket, Arc, Duration, HostKey, JoinHandle, Locked, Mutex, NetError, NetFile, NetworkPolicy, Node, Notify, Op, Pin, PublicKeys, Shared,
    Value, Weak, KEY, R,
};
use crate::admission::Admission;
use crate::dial::{Close, Dial, Mesh, PeerId};
use crate::endpoint::{added_roots, IrohDial, LanLookup, Relay, Relays, Settings};
use crate::join::{self, Admit, Admitted, Answer, Burned, Door, Invite, Served, Ticket};
use std::net::SocketAddr;

/// How long an inviter's address, as the person gave it, is kept.
const AT_TTL: Duration = Duration::from_mins(15);
/// The unused tickets the window is told of: the last few, for an hour.
const BURNED_KEPT: usize = 8;
const BURNED_MS: i64 = 3600 * 1000;

struct Open {
    dial: Arc<IrohDial>,
    door: Arc<Door>,
    task: JoinHandle<()>,
}

/// This machine's open invites: its door on join/2, while any is open.
#[derive(Default)]
pub struct Invites {
    open: Mutex<Option<Open>>,
    // Tickets that closed unused, after their door closed.
    burned: Mutex<Vec<Burned>>,
}

impl Invites {
    fn door(&self) -> Option<Arc<Door>> {
        self.open.locked().as_ref().map(|o| o.door.clone())
    }

    /// The invites open here and the ones that closed unused, as the state
    /// document lists them.
    pub fn json(&self, now: i64) -> Value {
        let door = self.door();
        let open: Vec<Value> = door
            .as_ref()
            .map(|d| {
                d.open_tickets().into_iter().filter(|t| t.expires > now).map(|t| json!({ "host": t.host, "window": t.window, "expires": t.expires, "wrong": t.wrong })).collect()
            })
            .unwrap_or_default();
        let mut burned = self.burned.locked().clone();
        if let Some(d) = &door {
            burned.extend(d.burned());
        }
        burned.retain(|b| now - b.at < BURNED_MS);
        let skip = burned.len().saturating_sub(BURNED_KEPT);
        let burned: Vec<Value> = burned.iter().skip(skip).map(|b| json!({ "host": b.host, "at": b.at, "retired": b.retired })).collect();
        json!({ "open": open, "burned": burned })
    }

    /// Every invite closes, and the door with them.
    pub fn close(&self) {
        let Some(o) = self.open.locked().take() else { return };
        o.task.abort();
        o.door.close_all();
        self.keep_burned(&o.door);
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { o.dial.close().await });
        }
    }

    fn keep_burned(&self, door: &Door) {
        let mut b = self.burned.locked();
        b.extend(door.burned());
        let skip = b.len().saturating_sub(BURNED_KEPT);
        b.drain(..skip);
    }

    // The door closes, if it's still the one open and (unless `anyway`, its
    // transport having closed) has no ticket open: under the lock a ticket
    // opens under. Whether it closed, or was closed already.
    fn closed(&self, door: &Arc<Door>, anyway: bool) -> bool {
        let mut open = self.open.locked();
        if !open.as_ref().is_some_and(|o| Arc::ptr_eq(&o.door, door)) {
            return true;
        }
        if !anyway && !door.is_empty() {
            return false;
        }
        let Some(o) = open.take() else { return true };
        drop(open);
        o.door.close_all();
        self.keep_burned(&o.door);
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { o.dial.close().await });
        }
        true
    }
}

fn relays_of(urls: &[String]) -> Relays {
    if urls.is_empty() {
        Relays::None
    } else {
        Relays::Listed(urls.iter().map(|u| Relay { url: u.clone(), auth_token: None }).collect())
    }
}

impl Node {
    /// `xenonet net invite <id>`: a ticket for the machine `host`, and the
    /// invite and the code to give it, apart. The person's alone, at the
    /// window, since signing the machine in asks them when it joins.
    pub async fn invite(self: &Arc<Self>, host: &str, window: bool, lan: bool, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, key) = self.me()?;
        if !at_window {
            return Err(identity::not_at_window("Inviting a machine"));
        }
        if !is_host_id(host) {
            return Err(NetError::bad("a host id is 2 to 32 lowercase letters, digits and dashes, starting with a letter"));
        }
        let view = n.roster().view();
        if view.owner.is_none() || view.owner != n.identity.person_id() || !n.identity.can_act() {
            return Err(NetError::new(
                409,
                "no-person",
                "this machine can't sign a machine in: adding one needs your presence at a device of the network's owner, and this machine has none",
            ));
        }
        if view.member(host).is_some() {
            return Err(NetError::new(409, "taken", format!("{host} is in the network already: Replace it to add a reinstalled machine")));
        }
        if !view.is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "an invite is made on a window host"));
        }
        let code = ticket::make()?;
        let expires = n.now() as i64 + ticket::TICKET_MS as i64;
        let (door, id) = self.open_ticket(&key, lan, &view.relays, |d| d.open(&net.id, host, window, &code, expires)).await?;
        let invite = Invite { network: net.id.clone(), inviter: door.mesh().me(), relay: view.relays.first().cloned(), ticket: id };
        let text = invite.encode();
        // Where the new machine can reach this one, for the person to give
        // it (`--at`): the addresses iroh found, or, before it has, the
        // sockets the endpoint is bound to, but for every-interface ones.
        let open = self.invites.open.locked().as_ref().map(|o| o.dial.clone());
        let at: Vec<String> = open
            .map(|d| {
                let direct = d.direct_addrs();
                let addrs = if direct.is_empty() { d.endpoint().bound_sockets().into_iter().filter(|a| !a.ip().is_unspecified()).collect() } else { direct };
                addrs.iter().map(ToString::to_string).collect()
            })
            .unwrap_or_default();
        n.log("net.invite", &format!("an invite for {host} was made; it works once, for ten minutes"));
        let mut out = json!({
            "host": host,
            "window": window,
            "invite": text,
            "code": ticket::display(&code),
            "line": format!("xenonet net join {text} <code>"),
            "expires": expires,
            "relay": invite.relay,
            "lan": lan,
            "at": at,
        });
        if window && view.founder != net.host {
            out["approval"] = json!(true);
        }
        Ok(out)
    }

    // A ticket on the door on join/2: the one open, or a new endpoint for
    // it. The ticket opens under the lock the door closes under, so a door
    // never closes on a ticket just opened.
    async fn open_ticket(self: &Arc<Self>, key: &HostKey, lan: bool, relays: &[String], ticket: impl Fn(&Door) -> R<join::TicketId>) -> R<(Arc<Door>, join::TicketId)> {
        {
            let open = self.invites.open.locked();
            if let Some(o) = open.as_ref() {
                return Ok((o.door.clone(), ticket(&o.door)?));
            }
        }
        let settings = Settings {
            relays: relays_of(relays),
            lan: if lan { LanLookup::On } else { LanLookup::Off },
            bind: self.shared.opts.bind,
            roots: added_roots(&self.shared.opts.dir).unwrap_or_default(),
        };
        let admission = Arc::new(Admission::new());
        let dial = Arc::new(IrohDial::bind(key, &settings, admission.clone()).await?);
        let door = Arc::new(Door::new(Arc::new(Mesh::new(dial.clone(), admission))));
        let (out, spare) = {
            let mut open = self.invites.open.locked();
            match open.as_ref() {
                // Another invite opened one meanwhile: that one is the door.
                Some(o) => ((o.door.clone(), ticket(&o.door)), Some(dial)),
                None => {
                    let id = ticket(&door);
                    if id.is_ok() {
                        let task = tokio::spawn(run_door(Arc::downgrade(self), door.clone(), Arc::new(Notify::new())));
                        *open = Some(Open { dial: dial.clone(), door: door.clone(), task });
                    }
                    let failed = id.is_err();
                    ((door, id), failed.then_some(dial))
                }
            }
        };
        if let Some(d) = spare {
            d.close().await;
        }
        let (door, id) = out;
        Ok((door, id?))
    }

    /// `xenonet net join <invite> <code>`: joins over join/2, through the
    /// endpoint the invite names, found by the invite's relay, mDNS on the
    /// LAN (`lan`) or the address the person gives (`at`).
    pub async fn join_invite(&self, invite: &str, code: &str, at: Option<&str>, lan: bool) -> R<Value> {
        let n = &self.shared;
        if let Some(net) = n.network() {
            return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already: xenonet net leave first", net.name)));
        }
        let inv = Invite::parse(invite)?;
        let code = ticket::normalize(code).ok_or_else(|| NetError::bad("a code is 10 letters and digits, as KX7Q-49MZ-T2"))?;
        let at = match at.map(str::trim).filter(|a| !a.is_empty()) {
            Some(a) => Some(a.parse::<SocketAddr>().map_err(|_| NetError::bad(format!("{a} isn't an address: say it as <ip>:<port>")))?),
            None => None,
        };
        if at.is_none() && inv.relay.is_none() && !lan {
            return Err(NetError::new(409, "no-path", "the invite names no relay, so say how to find the inviter: its address with --at <ip:port>, or on this LAN with --mdns"));
        }
        let key = HostKey::load_or_create(&n.dir.file(KEY))?;
        let identity = n.identity.clone();
        let machine = identity.machine_key()?.public();
        let mut add = json!({
            "sign": key.public().sign_b64(),
            "seal": key.public().seal_b64(),
            "machine": machine.to_json(),
            "platform": n.opts.platform,
            "version": n.opts.version,
            "desktop": n.opts.desktop,
        });
        // Its device for the inviter to pair into the person, unless it
        // holds a person of its own that other devices share.
        let local = identity.chain().person().is_none_or(|p| p.active_devices().all(|d| Some(&d.id) == identity.device_id().as_ref()));
        if let (Some(d), Some(kind), true) = (identity.device_key()?, identity.device_kind(), local) {
            add["device"] = identity::device_json(&d.signer.public(), kind, &n.opts.platform);
        }
        let add = add.to_string();
        let settings = Settings {
            relays: relays_of(inv.relay.as_slice()),
            lan: if lan { LanLookup::On } else { LanLookup::Off },
            bind: n.opts.bind,
            roots: added_roots(&n.opts.dir).unwrap_or_default(),
        };
        let admission = Arc::new(Admission::new());
        let dial = Arc::new(IrohDial::bind(&key, &settings, admission.clone()).await?);
        if at.is_some() || inv.relay.is_some() {
            dial.addresses().put(inv.inviter, at.as_slice(), inv.relay.as_deref(), AT_TTL)?;
        }
        let mesh = Mesh::new(dial, admission);
        let asked = join::ask(&mesh, &inv, &code, &add).await;
        mesh.close().await;
        let v = match asked {
            Ok(Answer::Joined(v)) => v,
            // The inviter's network waits for an approval: joining again
            // asks again, with the same code.
            Ok(Answer::Waiting(why)) => return Err(NetError::new(409, "waiting", why)),
            Err(e) if e.code == "unreachable" || e.code == "timeout" => {
                let how = if inv.relay.is_some() {
                    "If the relay lets only its members in, the join needs a direct path (--at or --mdns) or the relay's allowlist."
                } else {
                    "Check the address, or that both machines are on the same LAN."
                };
                return Err(NetError::new(502, "unreachable", format!("couldn't reach the machine that made the invite, or it has no invite open: {}. {how}", e.message)));
            }
            Err(e) => return Err(e),
        };
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        // The owner's chain first, which the chain's admits are checked
        // against.
        if let Some(p) = v.get("person").filter(|p| p.is_object()) {
            let person = s(p, "person");
            let entries = p.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
            let chain = xenonet_log::person::PersonChain::from_json(&Value::Array(entries))
                .map_err(|e| NetError::new(502, "bad-answer", format!("the inviter's person chain didn't check: {}", e.message)))?;
            if chain.id() != Some(person.as_str()) {
                return Err(NetError::new(502, "bad-answer", "the inviter's person chain isn't the person it names"));
            }
            if !identity.adopt(&chain) {
                identity.offer(&person, &chain.to_json().as_array().cloned().unwrap_or_default())?;
            }
        }
        let host = s(&v, "host");
        let roster = join::verify(NetworkPolicy::with(identity.clone()), v.get("roster").unwrap_or(&Value::Null), &inv, &host, key.public(), &machine)?;
        let view = roster.view();
        let gw = view.gateway_member().cloned().ok_or_else(|| NetError::new(502, "bad-answer", "the network's chain names no gateway"))?;
        let address = v.get("gateway").map(|g| s(g, "address")).filter(|a| !a.is_empty()).or_else(|| view.gateway_address.clone());
        let net = NetFile {
            id: view.network.clone(),
            name: v.get("network").map(|x| s(x, "name")).unwrap_or_default(),
            host: host.clone(),
            founder: view.founder.clone(),
            gateway: Pin { host: view.gateway.clone(), key_id: gw.keys.key_id(), sign: gw.keys.sign_b64(), seal: gw.keys.seal_b64(), address },
            lan: None,
            titles_only: false,
            share_load: true,
            joined: n.now() as i64,
        };
        let name = net.name.clone();
        let network = net.id.clone();
        self.adopt(net, key, roster)?;
        // Where the inviter and the members it knows are, for link/2.
        let records: Vec<Value> = v.get("addresses").and_then(Value::as_array).cloned().unwrap_or_default();
        n.common.found_at_join(&network, records);
        self.start()?;
        n.log("net.join", &format!("this machine joined the network {name} with an invite, as {host}"));
        Ok(self.state())
    }
}

// The inviter's door: join/2 attempts as they come, and its tickets'
// ten minutes, until the last ticket closes.
async fn run_door(node: Weak<Node>, door: Arc<Door>, served: Arc<Notify>) {
    loop {
        let Some(nd) = node.upgrade() else { return };
        let now = nd.shared.now() as i64;
        for t in door.expire(now) {
            nd.shared.log("net.invite", &format!("the invite for {} ran out unused: make a new one", t.host));
        }
        if door.is_empty() && nd.invites.closed(&door, false) {
            return;
        }
        let wait = door.next_expiry().map_or(Duration::from_secs(60), |e| Duration::from_millis(u64::try_from(e - now).unwrap_or(0).max(50)));
        let attempt = served.notified();
        drop(nd);
        tokio::select! {
            link = door.mesh().accept() => match link {
                Some(link) => {
                    tokio::spawn(serve_one(node.clone(), door.clone(), link, served.clone()));
                }
                None => {
                    if let Some(nd) = node.upgrade() {
                        nd.invites.closed(&door, true);
                    }
                    return;
                }
            },
            () = tokio::time::sleep(wait) => {}
            () = attempt => {}
        }
    }
}

async fn serve_one(node: Weak<Node>, door: Arc<Door>, link: Arc<crate::dial::Link>, done: Arc<Notify>) {
    let Some(nd) = node.upgrade() else { return };
    let n = nd.shared.clone();
    let now = n.now() as i64;
    let served = door.serve(&link, &NodeAdmit(node.clone()), now).await;
    // The joiner closes the connection once it has read the answer: closing
    // it here first could drop the answer still in flight.
    let _ = tokio::time::timeout(join::STEP, link.closed()).await;
    link.close(Close::Done);
    match served {
        Ok(Served::Joined { host }) => n.log("net.invite", &format!("{host} joined the network with its invite")),
        Ok(Served::Waiting { host }) => n.log("net.invite", &format!("{host} had the right code, and waits for an approval")),
        Ok(Served::Refused("retired")) => {
            let host = door.burned().last().map(|b| b.host.clone()).unwrap_or_default();
            n.log("net.invite", &format!("the invite for {host} had five wrong codes, and was retired: make a new one"));
        }
        Ok(Served::Refused("wrong-code")) => n.log("net.invite", "a wrong code was tried with an invite"),
        Ok(Served::Refused(_)) => {}
        Err(e) => n.log("net.invite", &format!("a join with an invite failed: {}", e.message)),
    }
    // A permit, should the door not be waiting yet.
    done.notify_one();
}

/// Signs a joiner in: the node behind the door.
struct NodeAdmit(Weak<Node>);

impl Admit for NodeAdmit {
    fn admit<'a>(&'a self, ticket: &'a Ticket, joiner: PeerId, add: &'a Value) -> crate::dial::Fut<'a, R<Admitted>> {
        Box::pin(async move {
            let node = self.0.upgrade().ok_or_else(|| NetError::new(503, "closing", "this machine's XenoNet is closing"))?;
            let n = &node.shared;
            let s = |k: &str| add.get(k).and_then(Value::as_str).unwrap_or("");
            let keys = PublicKeys::from_b64(s("sign"), s("seal")).ok_or_else(|| NetError::bad("the machine joining sent no keys"))?;
            // The endpoint QUIC authenticated is the transport key it asks
            // to be added with.
            if PeerId(keys.sign) != joiner {
                return Err(NetError::new(403, "wrong-key", "the keys the machine asked for aren't the ones it connected with"));
            }
            let net = n.network().ok_or_else(not_in)?;
            if net.id != ticket.network {
                return Err(NetError::new(409, "no-ticket", "this machine isn't in the invite's network any more"));
            }
            let in_roster = n.roster().view_ref().and_then(|v| v.member(&ticket.host).map(|m| m.keys == keys));
            match in_roster {
                Some(true) => return Ok(Admitted::Joined(answer(n, ticket)?)),
                Some(false) => return Err(NetError::new(409, "taken", format!("{} is in the network already: Replace it, then make a new invite", ticket.host))),
                None => {}
            }
            if n.proposals().iter().any(|p| p.op == Op::Add && p.host.id == ticket.host && p.host.keys == keys) {
                return Ok(Admitted::Waiting(waiting_words(&ticket.host)));
            }
            n.sync_chain().await;
            let e = n.sign_in(&ticket.host, keys, ticket.window, add)?;
            let verdict = n.roster().would_take(&e);
            match verdict {
                Ok(()) => {}
                // A window host added by an admin that isn't the founder
                // waits for a second admin.
                Err(err) if err.code == "needs-approval" => {
                    node.propose(e).await?;
                    return Ok(Admitted::Waiting(waiting_words(&ticket.host)));
                }
                Err(err) => return Err(err.into()),
            }
            node.submit(e).await?;
            Ok(Admitted::Joined(answer(n, ticket)?))
        })
    }
}

fn waiting_words(host: &str) -> String {
    format!(
        "{host} is to be a window host, which needs the founder or a second window host to approve: from one of their consoles, or xenonet net approve {host} --op add. Join again with the same code once it's approved, within the invite's ten minutes."
    )
}

// What the joiner is sent once it's in: the chain, the owner's person
// chain, where the gateway is when the network has one, and the address
// records of this machine and of the members it knows, each signed by its
// member, so the joiner finds them on link/2.
fn answer(n: &Shared, t: &Ticket) -> R<Value> {
    let net = n.network().ok_or_else(not_in)?;
    let mut addresses: Vec<Value> = n.address_record().into_iter().collect();
    addresses.extend(n.records_for(&net.id, &t.host));
    Ok(json!({
        "addresses": addresses,
        "network": { "id": net.id, "name": net.name },
        "host": t.host,
        "window": t.window,
        "founder": net.founder,
        "roster": n.roster().to_json(),
        "person": n.person_json(),
        "gateway": { "host": net.gateway.host, "address": net.gateway.address },
        "now": n.now() as i64,
    }))
}
