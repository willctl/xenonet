//! This machine in every network it is in (phase 5, commit 13): one node
//! per network (`node::Node`), each with its own chain, roster, feed
//! writer, settings and services, and what they share
//! (`node::Common`): the person here, one link/2 endpoint with one
//! connection per peer, the HLC and the entries taken lately.
//!
//! - Creating or joining a network adds a node; leaving one takes its node
//!   out, and the others go on as they were.
//! - What names a network (`--network <id|name>`, an `Outgoing`'s
//!   network, the network a body arrived in, through `ground::within`)
//!   goes to that network's node alone. With nothing named, a machine in
//!   one network uses it; one in several is asked to say which, except
//!   for what the engine sends with no network: presence goes into every
//!   network, each shown what is shared into it; a claim goes into each
//!   network its project is shared into; a body for a host goes to the
//!   network that host is in, and is refused when the host id names
//!   different machines in two networks; anything else goes to the first.
//! - The endpoint's relays are the ones every network lists (revision 9),
//!   so no network's members route through an operator another network
//!   chose; with none such, the person picks: the relays this machine
//!   adds of its own (`own_relays`) are for its own dials and its home,
//!   and an address record goes into a network carrying the home relay
//!   only when that network lists it.

use crate::files::Dir;
use crate::ground::{self, Facts, Governance};
use crate::identity::Identity;
use crate::node::{link2n, Common, Node, Options, Shared};
use crate::sink::SinkSlot;
use crate::{NetError, R};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, RwLock, Weak};
use tokio::task::JoinHandle;
use xenonet_model::ops::{Host, Kind, Me, Net, Outgoing, Reach, To};
use xenonet_os::sync::{Locked, ReadWrite};

/// The relays a machine whose networks list `lists` makes its home: the
/// ones every network lists, in the first list's order. A relay only some
/// networks list would carry the others' members' traffic through an
/// operator they didn't name, so it isn't taken (revision 9).
pub fn home(lists: &[Vec<String>]) -> Vec<String> {
    let Some((first, rest)) = lists.split_first() else { return Vec::new() };
    let mut out: Vec<String> = first.iter().filter(|u| rest.iter().all(|l| l.contains(u))).cloned().collect();
    out.dedup();
    out
}

/// n0's public relays, as iroh names them: the person's to pick, labelled
/// as n0's, never a default (rate-limited, run by n0).
pub fn n0_relays() -> Vec<String> {
    use iroh::defaults::prod::{AP_RELAY_HOSTNAME, EU_RELAY_HOSTNAME, NA_EAST_RELAY_HOSTNAME, NA_WEST_RELAY_HOSTNAME};
    [NA_EAST_RELAY_HOSTNAME, NA_WEST_RELAY_HOSTNAME, EU_RELAY_HOSTNAME, AP_RELAY_HOSTNAME].iter().map(|h| format!("https://{}", h.trim_end_matches('.'))).collect()
}

/// A relay as the person gives it: `n0` for n0's public relays, or the
/// URL of an iroh-relay (their own, say).
pub fn relay_urls(given: &str) -> R<Vec<String>> {
    let given = given.trim();
    if given.eq_ignore_ascii_case("n0") {
        return Ok(n0_relays());
    }
    let url = given.trim_end_matches('/').to_string();
    if !xenonet_log::network::is_relay_url(&url) {
        return Err(NetError::bad(format!("{given} isn't a relay's URL: give an iroh-relay's https:// address, or n0 for n0's public relays")));
    }
    Ok(vec![url])
}

pub struct Nodes {
    this: Weak<Nodes>,
    opts: Options,
    sink: Arc<SinkSlot>,
    common: Arc<Common>,
    nets: RwLock<Vec<Arc<Node>>>,
    /// A node in no network: what reads of "the network" see while this
    /// machine is in none.
    empty: Arc<Node>,
    /// link/2's task, while it runs.
    link2: Mutex<Option<JoinHandle<()>>>,
}

impl Nodes {
    /// Every network this machine is in, from `<state>/network` and the
    /// store, with a sink that takes what arrives; nothing runs until
    /// `start`.
    pub fn open(opts: Options, sink: Arc<SinkSlot>) -> R<Arc<Nodes>> {
        let dir = Dir::new(opts.dir.clone(), opts.persist.clone());
        let identity = Identity::open(dir.identity_dir(), opts.persist.clone(), opts.vault.clone())?;
        let common = Common::new(identity.clone());
        let mut nets = Vec::new();
        for net in dir.read_networks()? {
            nets.push(Node::open_in(opts.clone(), sink.clone(), common.clone(), Some(net))?);
        }
        let empty = Node::open_in(opts.clone(), sink.clone(), common.clone(), None)?;
        let nodes = Arc::new_cyclic(|this| Nodes { this: this.clone(), opts, sink, common, nets: RwLock::new(nets), empty, link2: Mutex::new(None) });
        nodes.listed();
        // What the person appends here, every network's other machines hear.
        let weak = Arc::downgrade(&nodes);
        identity.on_append(Box::new(move |e| {
            if let Some(n) = weak.upgrade() {
                for s in n.common.shareds() {
                    s.push_person(e.n);
                }
            }
        }));
        Ok(nodes)
    }

    fn listed(&self) {
        self.common.set_nets(self.nets.reading().iter().map(|n| Arc::downgrade(n.shared())).collect());
    }

    pub fn common(&self) -> &Arc<Common> {
        &self.common
    }

    pub fn sink(&self) -> &Arc<SinkSlot> {
        &self.sink
    }

    /// What the engine sends through.
    pub fn net(&self) -> Arc<dyn Net> {
        self.this.upgrade().expect("open") as Arc<dyn Net>
    }

    /// The node of every network this machine is in, first joined first.
    pub fn nodes(&self) -> Vec<Arc<Node>> {
        self.nets.reading().iter().filter(|n| n.shared().network().is_some()).cloned().collect()
    }

    /// The first network's node, or one in none.
    pub fn first(&self) -> Arc<Node> {
        self.nodes().into_iter().next().unwrap_or_else(|| self.empty.clone())
    }

    /// The node of the network `which` names, by its id or its name; with
    /// none named, the one network this machine is in. A machine in
    /// several is asked to say which.
    pub fn pick(&self, which: Option<&str>) -> R<Arc<Node>> {
        let nets = self.nodes();
        let Some(w) = which.map(str::trim).filter(|w| !w.is_empty()) else {
            return match nets.len() {
                0 => Err(NetError::new(409, "no-network", "this machine isn't in a XenoNet network")),
                1 => Ok(nets[0].clone()),
                _ => Err(NetError::new(409, "which-network", format!("this machine is in {} networks ({}): say which with --network <id|name>", nets.len(), names(&nets)))),
            };
        };
        if let Some(n) = nets.iter().find(|n| n.shared().network().is_some_and(|f| f.id == w)) {
            return Ok(n.clone());
        }
        let named: Vec<&Arc<Node>> = nets.iter().filter(|n| n.shared().network().is_some_and(|f| f.name == w)).collect();
        match named.as_slice() {
            [one] => Ok((*one).clone()),
            [] => {
                Err(NetError::new(404, "no-network", format!("this machine isn't in a network named {w} ({})", if nets.is_empty() { "it's in none".into() } else { names(&nets) })))
            }
            _ => Err(NetError::new(409, "which-network", format!("two networks here are named {w}: say which by its id"))),
        }
    }

    /// Starts each network's node, and link/2 for them all.
    pub fn start(&self) -> R<()> {
        for n in self.nodes() {
            n.start()?;
        }
        self.start_link2();
        Ok(())
    }

    pub fn stop(&self) {
        for n in self.nodes() {
            n.stop();
        }
        if let Some(t) = self.link2.locked().take() {
            t.abort();
        }
        link2n::stop(&self.common);
    }

    fn start_link2(&self) {
        if self.nodes().is_empty() {
            return;
        }
        let mut task = self.link2.locked();
        if task.as_ref().is_some_and(|t| !t.is_finished()) {
            self.common.relays_due.notify_one();
            return;
        }
        *task = Some(tokio::spawn(supervise(self.this.clone())));
    }

    /// The relays the endpoint has: the ones every network lists and the
    /// ones this machine adds of its own.
    fn relays_wanted(&self) -> Vec<String> {
        let mut want = home(&self.relay_lists().into_iter().map(|(_, l)| l).collect::<Vec<_>>());
        for r in self.opts.persist.own_relays().unwrap_or_default() {
            if !want.contains(&r) {
                want.push(r);
            }
        }
        want
    }

    fn relay_lists(&self) -> Vec<(String, Vec<String>)> {
        self.common
            .shareds()
            .iter()
            .filter(|s| !s.is_removed())
            .filter_map(|s| Some((s.network()?.id, s.roster().view_ref().map(|v| v.relays.clone()).unwrap_or_default())))
            .collect()
    }

    /// How the home relay stands, as the state shows it: the relays every
    /// network lists (the endpoint's), this machine's own, the ones only
    /// some networks list, which the person may pick from by adding one of
    /// its own, and the networks that list none the endpoint has, which
    /// reach this machine only by a direct path or its own dial.
    pub fn relays_json(&self) -> Value {
        let lists = self.relay_lists();
        let every = home(&lists.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>());
        let own = self.opts.persist.own_relays().unwrap_or_default();
        let mut choose: Vec<String> = lists.iter().flat_map(|(_, l)| l.iter().cloned()).filter(|u| !every.contains(u) && !own.contains(u)).collect();
        choose.sort();
        choose.dedup();
        let has: Vec<String> = every.iter().chain(own.iter()).cloned().collect();
        let direct: Vec<String> = lists.iter().filter(|(_, l)| !l.iter().any(|u| has.contains(u))).map(|(n, _)| n.clone()).collect();
        let url = self.common.link2.reading().as_ref().and_then(|t| t.dial.endpoint().addr().relay_urls().find_map(|u| crate::endpoint::as_listed(u, &has)));
        json!({ "url": url, "every": every, "own": own, "choose": choose, "directOnly": direct, "n0": n0_relays() })
    }

    /// The relays this machine adds of its own, for its dials and its home,
    /// never listed in a chain: `add` puts one in, else it goes. A
    /// setting of this machine's, so yours from your terminal too.
    pub fn own_relay(&self, given: &str, add: bool) -> R<Value> {
        let urls = relay_urls(given)?;
        let mut own = self.opts.persist.own_relays()?;
        if add {
            for u in urls {
                if !own.contains(&u) {
                    own.push(u);
                }
            }
            if own.len() > xenonet_log::network::MAX_RELAYS {
                return Err(NetError::bad(format!("a machine adds at most {} relays of its own", xenonet_log::network::MAX_RELAYS)));
            }
        } else {
            own.retain(|u| !urls.contains(u));
        }
        self.opts.persist.put_own_relays(&own)?;
        self.common.relays_due.notify_one();
        Ok(self.state())
    }

    /// A relay (`given`: a URL, or n0) into the chain of the network
    /// `which` names (`add`, which widens: with the person's presence, so
    /// from the window) or out of it; with `mine`, into or out of the
    /// relays this machine adds of its own.
    pub async fn relay(&self, which: Option<&str>, given: &str, add: bool, mine: bool, at_window: bool) -> R<Value> {
        if mine {
            return self.own_relay(given, add);
        }
        let urls = relay_urls(given)?;
        let node = self.pick(which)?;
        let mut out = node.set_relays(&urls, add, at_window).await?;
        if out.get("proposed") == Some(&json!(true)) {
            let mut state = self.state();
            state["proposed"] = json!(true);
            out = state;
        } else {
            out = self.state();
        }
        Ok(out)
    }

    /// Whether a window host that takes this machine's load is connected
    /// now, so the service reads its load only then.
    pub fn wants_load(&self) -> bool {
        link2n::wants_load(&self.common)
    }

    /// This machine's load (system/v1) to the connected window hosts of
    /// the networks that share it.
    pub fn send_load(&self, reading: &Value) {
        link2n::link2_load(&self.common, reading);
    }

    /// The machine another machine's agent is on, by the agt_ its
    /// presence lists it by.
    pub fn host_of_agent(&self, id: &str) -> Option<String> {
        self.common.shareds().iter().find_map(|s| s.host_of_agent(id))
    }

    /// The latest load each other machine of this one's networks sent.
    pub fn loads(&self) -> Vec<Value> {
        link2n::loads(&self.common)
    }

    /// What the person shares into the network `which` names: `sharing` is
    /// "all" or a list of project ids. Sharing more publishes what another
    /// network's members see, so it's the window's; less is done anywhere.
    /// Each peer is sent this machine's presence whole again at once, shown
    /// what it may see now.
    pub fn share(&self, which: Option<&str>, sharing: &Value, at_window: bool) -> R<Value> {
        let to = crate::share::Sharing::parse(sharing).ok_or_else(|| NetError::bad("sharing is \"all\" or a list of project ids (prj_…)"))?;
        let node = self.pick(which)?;
        let s = node.shared();
        let net = s.network().ok_or_else(|| NetError::new(409, "no-network", "this machine isn't in that network"))?;
        let had = s.sharing(&net.id);
        let widens = match (&had, &to) {
            (_, crate::share::Sharing::Projects(p)) if p.is_empty() => false,
            (crate::share::Sharing::All, _) => false,
            (crate::share::Sharing::Projects(_), crate::share::Sharing::All) => true,
            (crate::share::Sharing::Projects(a), crate::share::Sharing::Projects(b)) => !b.is_subset(a),
        };
        if widens && !at_window {
            return Err(crate::identity::not_at_window("sharing more projects into a network"));
        }
        self.opts.persist.put_sharing(&net.id, &to)?;
        s.log("net.sharing", &format!("what {} sees of this machine changed", net.name));
        link2n::presence_again(&self.common);
        xenonet_model::ops::Sink::gap(self.sink.as_ref());
        Ok(self.state())
    }

    /// `xenonet net create`: a new network, beside any this machine is in
    /// already, with the relay the person names (`relay`: their own
    /// iroh-relay's URL, n0 for n0's public ones, or none).
    pub async fn create(&self, name: &str, host: &str, relay: Option<&str>, at_window: bool) -> R<Value> {
        let relays = relay.map(str::trim).filter(|r| !r.is_empty()).map(relay_urls).transpose()?;
        if self.nodes().iter().any(|n| n.shared().network().is_some_and(|f| f.name == name.trim())) {
            return Err(NetError::new(409, "taken", format!("this machine is in a network named {} already: give the new one another name", name.trim())));
        }
        let fresh = Node::open_in(self.opts.clone(), self.sink.clone(), self.common.clone(), None)?;
        fresh.create(name, host, at_window)?;
        self.added(fresh.clone());
        if let Some(urls) = relays {
            if let Err(e) = fresh.set_relays(&urls, true, at_window).await {
                fresh.shared().log("net.create", &format!("the network was made, without its relay: {}", e.message));
                let mut out = self.state();
                out["relayRefused"] = json!(e.message);
                return Ok(out);
            }
        }
        Ok(self.state())
    }

    /// `xenonet net join <invite> <code>` over join/2: one network more.
    pub async fn join_invite(&self, invite: &str, code: &str, at: Option<&str>, lan: bool) -> R<Value> {
        let inv = crate::join::Invite::parse(invite)?;
        if let Some(n) = self.common.shared_of(&inv.network).and_then(|s| s.network()) {
            return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already, as {}", n.name, n.host)));
        }
        let fresh = Node::open_in(self.opts.clone(), self.sink.clone(), self.common.clone(), None)?;
        fresh.join_invite(invite, code, at, lan).await?;
        self.added(fresh);
        Ok(self.state())
    }

    fn added(&self, node: Arc<Node>) {
        self.nets.writing().push(node.clone());
        self.listed();
        self.start_link2();
        // Its members kept connected now it's listed, where link/2 runs
        // already for another network.
        node.shared().members_changed();
        self.common.take_joined();
    }

    /// `xenonet net leave`: the network `which` names goes; the others stay
    /// as they were.
    pub fn leave(&self, which: Option<&str>, force: bool) -> R<Value> {
        let node = self.pick(which)?;
        let out = node.leave(force)?;
        self.nets.writing().retain(|n| !Arc::ptr_eq(n, &node));
        self.listed();
        if self.nodes().is_empty() {
            if let Some(t) = self.link2.locked().take() {
                t.abort();
            }
            link2n::stop(&self.common);
        } else {
            for n in self.common.shareds() {
                n.members_changed();
            }
        }
        Ok(out)
    }

    /// `network/state/v1`: the first network at the top, as the page and
    /// the command read it, and every network in `networks`, each with
    /// this machine's host id in it and its own members, link and settings.
    pub fn state(&self) -> Value {
        let states: Vec<Value> = self.nodes().iter().map(|n| n.state()).collect();
        let mut out = states.first().cloned().unwrap_or_else(|| self.empty.state());
        out["networks"] = Value::Array(states.iter().map(Node::network_of).collect());
        out["relaysHere"] = self.relays_json();
        out
    }

    // The node a call that names no network goes through: the network the
    // body in hand arrived in, else the first.
    fn current(&self) -> Option<Arc<Shared>> {
        ground::current().and_then(|n| self.common.shared_of(&n)).or_else(|| self.common.shareds().into_iter().next())
    }
}

fn names(nets: &[Arc<Node>]) -> String {
    nets.iter().filter_map(|n| n.shared().network()).map(|f| format!("{} ({})", f.name, f.id)).collect::<Vec<_>>().join(", ")
}

// link/2 for every network, started again when the relays it needs go
// from none to some or back; told of relay changes otherwise.
async fn supervise(nodes: Weak<Nodes>) {
    loop {
        let (common, mut have, task) = {
            let Some(n) = nodes.upgrade() else { return };
            let want = n.relays_wanted();
            (n.common.clone(), want.clone(), tokio::spawn(link2n::run(Arc::downgrade(&n.common), want)))
        };
        let restart = loop {
            common.relays_due.notified().await;
            let Some(n) = nodes.upgrade() else {
                task.abort();
                return;
            };
            let want = n.relays_wanted();
            if want == have {
                continue;
            }
            let Some(t) = common.link2.reading().clone() else { continue };
            let relays = link2n::relays_of(&want);
            match t.dial.set_relays(relays).await {
                Ok(()) => have = want,
                Err(e) if e.code == "restart-needed" => break true,
                Err(e) => {
                    if let Some(s) = common.shareds().into_iter().next() {
                        s.log("net.link2", &format!("the relays weren't taken: {}", e.message));
                    }
                    have = want;
                }
            }
        };
        if restart {
            task.abort();
            link2n::stop(&common);
        }
    }
}

impl Net for Nodes {
    fn networks(&self) -> Vec<Me> {
        self.common.shareds().iter().filter_map(|s| s.network()).map(|n| Me { network: n.id, name: n.name, host: n.host }).collect()
    }

    fn me(&self) -> Option<Me> {
        let s = self.current()?;
        let n = s.network()?;
        Some(Me { network: n.id, name: n.name, host: n.host })
    }

    fn hosts(&self) -> Vec<Host> {
        let mut out: Vec<Host> = Vec::new();
        for s in self.common.shareds() {
            for h in Net::hosts(s.as_ref()) {
                if !out.iter().any(|o| o.id == h.id) {
                    out.push(h);
                }
            }
        }
        out
    }

    fn send(&self, out: Outgoing) -> Result<(), String> {
        let nets = self.common.shareds();
        if nets.is_empty() {
            return Err("this machine isn't in a XenoNet network".into());
        }
        // A network named, or the one the body in hand came in: there alone.
        if let Some(id) = out.network.clone().or_else(ground::current) {
            let s = self.common.shared_of(&id).ok_or_else(|| format!("this machine isn't in {id}"))?;
            let presence = (out.kind == Kind::Presence).then(|| out.body.clone());
            s.send(out.within(id))?;
            if let Some(body) = presence {
                link2n::link2_presence(&self.common, &body);
            }
            return Ok(());
        }
        match out.kind {
            // Into every network, each shown what is shared with its
            // members; once on link/2 for each peer.
            Kind::Presence => {
                let body = out.body.clone();
                let mut sent = 0;
                let mut why = String::new();
                for s in &nets {
                    let Some(id) = s.network().map(|n| n.id) else { continue };
                    match s.send(Outgoing { network: Some(id), ..out.clone() }) {
                        Ok(()) => sent += 1,
                        Err(e) => why = e,
                    }
                }
                link2n::link2_presence(&self.common, &body);
                if sent == 0 {
                    return Err(why);
                }
                Ok(())
            }
            // Into each network its project is shared into.
            Kind::Claims => {
                let project = out.body.get("project").and_then(Value::as_str).unwrap_or("");
                for s in &nets {
                    let Some(id) = s.network().map(|n| n.id) else { continue };
                    if !s.is_removed() && s.sharing(&id).shows_project(project) {
                        s.send(Outgoing { network: Some(id), ..out.clone() })?;
                    }
                }
                Ok(())
            }
            _ => match &out.to {
                To::Host(h) => {
                    let holding: Vec<(&Arc<Shared>, String)> =
                        nets.iter().filter(|s| !s.is_removed()).filter_map(|s| Some((s, s.roster().view_ref()?.member(h)?.machine.id()))).collect();
                    let Some((s, machine)) = holding.first() else { return Err(format!("{h} isn't in a network this machine is in")) };
                    if holding.iter().any(|(_, m)| m != machine) {
                        return Err(format!("{h} names different machines in this machine's networks: the network has to be named"));
                    }
                    let id = s.network().map(|n| n.id).ok_or("this machine is leaving that network")?;
                    s.send(out.within(id))
                }
                To::All => {
                    let s = &nets[0];
                    let id = s.network().map(|n| n.id).ok_or("this machine is leaving that network")?;
                    s.send(out.within(id))
                }
            },
        }
    }

    fn roster_head(&self) -> Option<String> {
        self.current().and_then(|s| Net::roster_head(s.as_ref()))
    }

    fn titles_only(&self) -> bool {
        self.current().is_some_and(|s| Net::titles_only(s.as_ref()))
    }

    fn share_load(&self) -> bool {
        self.current().is_none_or(|s| Net::share_load(s.as_ref()))
    }

    fn is_window_host(&self) -> bool {
        self.current().is_some_and(|s| Net::is_window_host(s.as_ref()))
    }

    fn reach(&self) -> Reach {
        self.current().map(|s| Net::reach(s.as_ref())).unwrap_or_default()
    }

    fn host_can(&self, host: &str, what: &str) -> bool {
        self.common.shareds().iter().any(|s| Net::host_can(s.as_ref(), host, what))
    }

    fn ack(&self, network: &str, host: &str, inc: u64, high: u64) {
        if let Some(s) = self.common.shared_of(network) {
            Net::ack(s.as_ref(), network, host, inc, high);
        }
    }

    fn want(&self, network: &str, host: &str, inc: u64, from: u64) {
        if let Some(s) = self.common.shared_of(network) {
            Net::want(s.as_ref(), network, host, inc, from);
        }
    }
}

/// The governance an act is decided on: the chain of the network it came
/// in (`ground::within`), else the first network's.
impl Governance for Nodes {
    fn facts(&self) -> Facts {
        match self.current() {
            Some(s) => s.facts(),
            None => self.empty.shared().facts(),
        }
    }
    fn identity(&self) -> Arc<Identity> {
        self.common.identity.clone()
    }
    fn want_person(&self, host: &str, person: &str) {
        let s = self.current().filter(|s| s.roster().view_ref().is_some_and(|v| v.member(host).is_some()));
        let s = s.or_else(|| self.common.shareds().into_iter().find(|s| s.roster().view_ref().is_some_and(|v| v.member(host).is_some())));
        if let Some(s) = s {
            s.want_person(host, person);
        }
    }
}

impl Drop for Nodes {
    fn drop(&mut self) {
        if let Some(t) = self.link2.locked().take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_relay_is_one_every_network_lists() {
        let a = "https://relay.a.example".to_string();
        let b = "https://relay.b.example".to_string();
        assert_eq!(home(&[vec![a.clone(), b.clone()], vec![b.clone()]]), vec![b.clone()]);
        assert_eq!(home(&[vec![a.clone()], vec![b.clone()]]), Vec::<String>::new(), "no relay both list: none, never one network's");
        assert_eq!(home(&[vec![a.clone()], vec![]]), Vec::<String>::new(), "a network with no relay keeps the machine off every relay");
        assert_eq!(home(&[vec![a.clone(), b.clone()]]), vec![a, b]);
        assert_eq!(home(&[]), Vec::<String>::new());
    }

    #[test]
    fn a_relay_is_the_persons_own_url_or_n0s_named_as_such() {
        assert_eq!(relay_urls("https://relay.example.net/").unwrap(), vec!["https://relay.example.net".to_string()]);
        assert_eq!(relay_urls("N0").unwrap(), n0_relays());
        assert!(n0_relays().iter().all(|u| u.starts_with("https://") && u.ends_with(".relay.n0.iroh.link")), "{:?}", n0_relays());
        assert!(relay_urls("relay.example.net").is_err(), "a URL with its scheme");
        assert!(relay_urls("https://user@relay.example.net").is_err());
    }
}
