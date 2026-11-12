//! The network's chain on link/2: how its entries and the ones waiting for
//! a second admin reach every member, machine to machine.
//!
//! - **New entries go on.** An entry a machine appends, its own or one a
//!   member sent it, goes to every other member it's connected to, as a
//!   Chain record with the owner's person chain beside it, which the
//!   chain's admits are checked against. A machine that already holds it
//!   takes it as a duplicate and sends it no further, so an entry reaches
//!   each member from any other, whoever it's connected to.
//! - **Heads on connect.** Each hello names the chain's head. Where the
//!   peer's isn't this machine's, it asks for every entry past its own
//!   length (ChainWant); the one that holds more answers, a batch a
//!   record, and the one that holds less answers with nothing. A gap (an
//!   entry past what this machine holds) asks for the rest the same way.
//! - **Proposals** (entries signed by too few) go the same way, and each
//!   machine keeps one only when its own chain would take it but for the
//!   second signer, so a member can't put an entry in another's window
//!   that its signers didn't make.
//! - **A removal** goes to the machine it removes before it's taken here,
//!   while their connection still carries the network: that machine hears
//!   it was removed, then the connection closes.
//! - **Before an entry is drafted** here (a removal, a setting, a relay, an
//!   approval, a machine signed in), this machine asks every member it's
//!   connected to for what it holds past its own chain, and waits for them
//!   a moment, so the person acting at one window after another builds on
//!   what the first one did. Two admins writing at the same moment can
//!   still both take the same place: the members then hold different
//!   heads, which the state shows (`rosterMismatch`).
//!
//! Every entry is decided by the receiver's own copy of the chain, under
//! the network's policy, whoever passed it on: a member can withhold an
//! entry, never forge or change one.

use super::{json, Entry, Locked, NetError, Shared, Value};
use crate::dial::PeerId;
use crate::endpoint::peer_id;
use crate::link2::{Control, Out};

/// Entries in one Chain record, at most.
const BATCH: usize = 64;
/// How long a machine waits for the members' chains before it drafts.
const SYNC: std::time::Duration = std::time::Duration::from_millis(1500);

impl Shared {
    /// The members of this network connected on link/2 now, but `except`.
    fn connected_members(&self, except: Option<PeerId>) -> Vec<PeerId> {
        let (Some(t), Some(net)) = (self.link2(), self.network()) else { return Vec::new() };
        t.links.peers().into_iter().filter(|(p, s)| Some(*p) != except && s.networks.contains(&net.id)).map(|(p, _)| p).collect()
    }

    fn chain_record(&self, entries: &[Entry], proposals: &[Entry]) -> Option<Out> {
        let net = self.network()?;
        let body = json!({
            "entries": entries.iter().map(Entry::to_json).collect::<Vec<_>>(),
            "person": self.person_json(),
            "proposals": proposals.iter().map(Entry::to_json).collect::<Vec<_>>(),
        });
        Some(Out::Control(Control::Chain { network: net.id, body }))
    }

    /// Entries this machine took (and ones waiting for approval), to every
    /// member connected on link/2 but `except`, the one they came from.
    pub(super) fn chain_out(&self, entries: &[Entry], proposals: &[Entry], except: Option<PeerId>) {
        if entries.is_empty() && proposals.is_empty() {
            return;
        }
        let (Some(t), Some(out)) = (self.link2(), self.chain_record(entries, proposals)) else { return };
        for p in self.connected_members(except) {
            let _ = t.links.send(p, out.clone());
        }
    }

    /// A removal, to the machine it removes, while it's still a member here.
    pub(super) fn chain_to_removed(&self, e: &Entry) {
        let Some(t) = self.link2() else { return };
        let Some(peer) = self.roster().view_ref().and_then(|v| v.member(&e.host.id).map(|m| peer_id(&m.keys))) else { return };
        if let Some(out) = self.chain_record(std::slice::from_ref(e), &[]) {
            let _ = t.links.send(peer, out);
        }
    }

    /// The chain from entry `from` on, a batch a record, and what waits
    /// for approval here: what a peer that holds less is sent.
    pub(super) fn chain_since(&self, from: u64) -> Vec<Out> {
        let entries: Vec<Entry> = self.roster().since(from).to_vec();
        let mut out: Vec<Out> = entries.chunks(BATCH).filter_map(|c| self.chain_record(c, &[])).collect();
        let waiting = self.proposals();
        if !waiting.is_empty() {
            out.extend(self.chain_record(&[], &waiting));
        }
        out
    }

    /// What a hello's chain head asks of this machine: when it isn't this
    /// machine's, every entry the peer holds past this machine's length.
    pub(super) fn chain_hello(&self, head: Option<&str>) -> Option<Out> {
        let (net, r) = (self.network()?, self.roster());
        (r.head().as_deref() != head).then(|| Out::Control(Control::ChainWant { network: net.id, from: r.len() }))
    }

    /// A Chain record from `peer` (the member `host`): the owner's person
    /// chain first, then each entry in order, decided by this machine's own
    /// copy of the chain; a gap asks the peer for what comes before. Then
    /// what waits for approval.
    pub(super) fn chain_in(&self, peer: PeerId, host: &str, body: &Value) {
        *self.chain_heard.locked() += 1;
        self.chain_wake.notify_waiters();
        if let Some(p) = body.get("person").filter(|p| p.is_object()) {
            self.offer_person(p);
        }
        let mut gap = false;
        for e in body.get("entries").and_then(Value::as_array).into_iter().flatten() {
            match Entry::parse(e).map_err(NetError::from).and_then(|e| self.append_from(e, Some(peer))) {
                Ok(_) => {}
                Err(e) if e.code == "roster-gap" => {
                    gap = true;
                    break;
                }
                Err(e) => self.log("net.roster", &format!("a chain entry from {host} was refused: {}", e.message)),
            }
        }
        if gap {
            if let (Some(t), Some(net)) = (self.link2(), self.network()) {
                let _ = t.links.send(peer, Out::Control(Control::ChainWant { network: net.id, from: self.roster().len() }));
            }
        }
        let proposed: Vec<Entry> = body.get("proposals").and_then(Value::as_array).into_iter().flatten().filter_map(|e| Entry::parse(e).ok()).take(MAX_IN).collect();
        if !proposed.is_empty() {
            self.note_proposals_from(proposed, Some(peer));
        }
    }

    /// A ChainWant from `peer`: what this machine holds past `from`, or an
    /// empty record when it holds nothing more, so the asker knows.
    pub(super) fn chain_want_in(&self, peer: PeerId, from: u64) {
        let Some(t) = self.link2() else { return };
        let mut out = self.chain_since(from);
        if out.is_empty() {
            out.extend(self.chain_record(&[], &[]));
        }
        for o in out {
            let _ = t.links.send(peer, o);
        }
    }

    /// The chain brought up to date with the members connected now, before
    /// this machine drafts an entry: each is asked for what it holds past
    /// this machine's length, and this machine waits for every answer, or
    /// `SYNC`.
    pub(crate) async fn sync_chain(&self) {
        let (Some(t), Some(net)) = (self.link2(), self.network()) else { return };
        let peers = self.connected_members(None);
        if peers.is_empty() {
            return;
        }
        let want = *self.chain_heard.locked() + peers.len() as u64;
        let from = self.roster().len();
        for p in &peers {
            let _ = t.links.send(*p, Out::Control(Control::ChainWant { network: net.id.clone(), from }));
        }
        let end = tokio::time::Instant::now() + SYNC;
        loop {
            let woke = self.chain_wake.notified();
            if *self.chain_heard.locked() >= want {
                return;
            }
            if tokio::time::timeout_at(end, woke).await.is_err() {
                return;
            }
        }
    }
}

/// Proposals read from one record, at most.
const MAX_IN: usize = super::MAX_PROPOSALS;
