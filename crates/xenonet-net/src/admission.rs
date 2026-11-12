//! Who may hold a connection to this machine, and on which protocol
//! (docs/design/network.md; phase 5's admission). A peer is its endpoint
//! id, the Ed25519 public key of its host.key. It is admitted on
//! `xenonet/link/2` and `xenonet/svc/2` while it is a member of at least
//! one network this machine holds the chain of, and the networks it shares
//! with this machine are what a connection to it may carry.
//! `xenonet/join/2` has no members yet: a machine dials it only to the
//! endpoint id an invite names, and answers it only while one of its own
//! tickets is open.
//!
//! **Another program's doors.** Which protocols admit members and which
//! one a machine joins by are the admission's `Doors`: XenoNet's are
//! link/2 and svc/2, and join/2 (`Doors::xenonet`). Another program that
//! runs a mesh of its own (Xenolith on its own endpoint: `xenolith/sync/2`
//! for the members of the spaces it holds the chains of, `xenolith/join/1`
//! while one of its tickets is open) brings its own, and its groups are
//! what it calls `set_members` with: the same rules, for spaces instead of
//! networks. XenoNet's own doors also admit the members of its networks on
//! the protocols it forwards for an actor (`forward`).
//!
//! The same `check` runs at every place a connection can come through: the
//! iroh endpoint's hooks (an early cut, before a protocol sees it), each
//! protocol handler's accept, and the `Mesh`'s dial and accept paths, which
//! any `Dial` goes through (the turmoil one in the tests included, which
//! never runs iroh's hooks).

use crate::dial::{Alpn, PeerId, Side};
use crate::NetError;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;
use xenonet_os::sync::ReadWrite;

/// Why a connection was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The peer is in no network whose chain this machine holds.
    NotAMember,
    /// A join dial to an endpoint no invite names.
    NotInvited,
    /// A join with no ticket open here.
    NoTicket,
    /// A protocol this machine has no door for.
    NoDoor,
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Refused::NotAMember => "not-a-member",
            Refused::NotInvited => "not-invited",
            Refused::NoTicket => "no-ticket",
            Refused::NoDoor => "no-door",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Refused::NotAMember => "that machine is in none of this machine's networks",
            Refused::NotInvited => "no invite on this machine names that machine",
            Refused::NoTicket => "this machine has no join ticket open",
            Refused::NoDoor => "this machine doesn't speak that protocol",
        }
    }
}

impl From<Refused> for NetError {
    fn from(r: Refused) -> NetError {
        NetError::new(403, r.code(), r.words())
    }
}

/// The protocols an admission answers: the ones whose connections carry
/// groups and admit their members, and the one a machine joins by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doors {
    pub members: Vec<Alpn>,
    pub join: Option<Alpn>,
}

impl Doors {
    /// XenoNet's: link/2 and svc/2 for the members of its networks, join/2
    /// to join one.
    pub fn xenonet() -> Doors {
        Doors { members: vec![Alpn::Link, Alpn::Svc], join: Some(Alpn::Join) }
    }

    /// XenoNet's, and the members of its networks on `forwarded` too: the
    /// protocols it carries for an actor (`forward`). XenoNet's own are
    /// never among them.
    pub fn xenonet_forwarding(forwarded: &[Alpn]) -> Doors {
        let mut d = Doors::xenonet();
        for a in forwarded {
            if !a.is_xenonet() && !d.members.contains(a) {
                d.members.push(*a);
            }
        }
        d
    }
}

impl Default for Doors {
    fn default() -> Doors {
        Doors::xenonet()
    }
}

#[derive(Default, Debug)]
struct State {
    // Each group (a network, or another program's own, such as a space)
    // this machine is in, and its members' endpoint ids, as this machine's
    // own copy of the group's chain has them.
    networks: BTreeMap<String, BTreeSet<PeerId>>,
    // The inviters this machine is joining through, by the endpoint id
    // each invite names, and how many joins name each.
    joining: BTreeMap<PeerId, usize>,
    // How many of this machine's own tickets are open.
    tickets: usize,
}

/// The membership a connection is checked against.
#[derive(Default, Debug)]
pub struct Admission {
    doors: Doors,
    state: RwLock<State>,
}

impl Admission {
    /// XenoNet's admission (`Doors::xenonet`).
    pub fn new() -> Admission {
        Admission::default()
    }

    /// An admission with other doors: another program's mesh, or XenoNet's
    /// with the protocols it forwards.
    pub fn with(doors: Doors) -> Admission {
        Admission { doors, state: RwLock::default() }
    }

    pub fn doors(&self) -> &Doors {
        &self.doors
    }

    /// Every protocol this machine has a door for, open now or not: the
    /// members' and the join's.
    pub fn protocols(&self) -> Vec<Alpn> {
        self.doors.members.iter().copied().chain(self.doors.join).collect()
    }

    /// The protocol `alpn`'s bytes name, among the ones there's a door for.
    pub fn alpn(&self, alpn: &[u8]) -> Option<Alpn> {
        self.protocols().into_iter().find(|a| a.bytes() == alpn)
    }

    /// Whether connections on `alpn` carry groups and admit their members
    /// (rather than a join, which carries none).
    pub fn is_members(&self, alpn: Alpn) -> bool {
        self.doors.members.contains(&alpn)
    }

    /// A network's members, in place of what was known of it: the members
    /// of this machine's copy of its chain.
    pub fn set_members(&self, network: &str, members: impl IntoIterator<Item = PeerId>) {
        self.state.writing().networks.insert(network.to_string(), members.into_iter().collect());
    }

    /// A network this machine left: none of its members is admitted for it.
    pub fn leave(&self, network: &str) {
        self.state.writing().networks.remove(network);
    }

    /// The networks `peer` shares with this machine.
    pub fn shared(&self, peer: &PeerId) -> BTreeSet<String> {
        self.state.reading().networks.iter().filter(|(_, m)| m.contains(peer)).map(|(n, _)| n.clone()).collect()
    }

    /// A join through the inviter an invite names begins: dials to it on
    /// join/2 are let through until `end_join`.
    pub fn begin_join(&self, inviter: PeerId) {
        *self.state.writing().joining.entry(inviter).or_default() += 1;
    }

    pub fn end_join(&self, inviter: &PeerId) {
        let mut s = self.state.writing();
        if let Some(n) = s.joining.get_mut(inviter) {
            *n -= 1;
            if *n == 0 {
                s.joining.remove(inviter);
            }
        }
    }

    /// One of this machine's tickets opened. Returns whether join/2 has to
    /// be offered now (it wasn't before).
    pub fn open_ticket(&self) -> bool {
        let mut s = self.state.writing();
        s.tickets += 1;
        s.tickets == 1
    }

    /// One of this machine's tickets closed (used, expired or retired).
    /// Returns whether join/2 has to be withdrawn now.
    pub fn close_ticket(&self) -> bool {
        let mut s = self.state.writing();
        if s.tickets == 0 {
            return false;
        }
        s.tickets -= 1;
        s.tickets == 0
    }

    pub fn tickets_open(&self) -> bool {
        self.state.reading().tickets > 0
    }

    /// The protocols this machine answers now: the join's only while a
    /// ticket is open.
    pub fn offered(&self) -> Vec<Alpn> {
        let mut alpns = self.doors.members.clone();
        if self.tickets_open() {
            alpns.extend(self.doors.join);
        }
        alpns
    }

    /// Whether a connection with `peer` on `alpn`, dialled by this machine
    /// or by the peer (`side`), is admitted, and the groups it may carry
    /// (none on a join).
    pub fn check(&self, peer: &PeerId, alpn: Alpn, side: Side) -> Result<BTreeSet<String>, Refused> {
        if self.is_members(alpn) {
            let shared = self.shared(peer);
            return if shared.is_empty() { Err(Refused::NotAMember) } else { Ok(shared) };
        }
        if self.doors.join != Some(alpn) {
            return Err(Refused::NoDoor);
        }
        match side {
            Side::Dialled => {
                if self.state.reading().joining.contains_key(peer) {
                    Ok(BTreeSet::new())
                } else {
                    Err(Refused::NotInvited)
                }
            }
            Side::Accepted => {
                if self.tickets_open() {
                    Ok(BTreeSet::new())
                } else {
                    Err(Refused::NoTicket)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(b: u8) -> PeerId {
        PeerId([b; 32])
    }

    #[test]
    fn link_and_svc_admit_members_of_any_shared_network_whichever_side_dialled() {
        let a = Admission::new();
        a.set_members("n1", [peer(1), peer(2)]);
        a.set_members("n2", [peer(2), peer(3)]);
        for side in [Side::Dialled, Side::Accepted] {
            for alpn in [Alpn::Link, Alpn::Svc] {
                assert_eq!(a.check(&peer(1), alpn, side), Ok(["n1".to_string()].into()));
                assert_eq!(a.check(&peer(2), alpn, side), Ok(["n1".to_string(), "n2".to_string()].into()));
                assert_eq!(a.check(&peer(9), alpn, side), Err(Refused::NotAMember));
            }
        }
        a.set_members("n1", [peer(1)]);
        assert_eq!(a.shared(&peer(2)), ["n2".to_string()].into(), "removed from n1, still in n2");
        a.leave("n2");
        assert_eq!(a.check(&peer(2), Alpn::Link, Side::Accepted), Err(Refused::NotAMember));
    }

    #[test]
    fn join_is_dialled_only_to_an_invites_inviter_and_answered_only_with_a_ticket_open() {
        let a = Admission::new();
        assert_eq!(a.check(&peer(7), Alpn::Join, Side::Dialled), Err(Refused::NotInvited));
        a.begin_join(peer(7));
        a.begin_join(peer(7));
        assert_eq!(a.check(&peer(7), Alpn::Join, Side::Dialled), Ok(BTreeSet::new()));
        assert_eq!(a.check(&peer(8), Alpn::Join, Side::Dialled), Err(Refused::NotInvited));
        a.end_join(&peer(7));
        assert!(a.check(&peer(7), Alpn::Join, Side::Dialled).is_ok(), "one join still names it");
        a.end_join(&peer(7));
        assert_eq!(a.check(&peer(7), Alpn::Join, Side::Dialled), Err(Refused::NotInvited));

        // Being an inviter's target says nothing about answering joins.
        assert_eq!(a.check(&peer(7), Alpn::Join, Side::Accepted), Err(Refused::NoTicket));
        assert_eq!(a.offered(), [Alpn::Link, Alpn::Svc]);
        assert!(a.open_ticket(), "the first ticket offers join/2");
        assert!(!a.open_ticket());
        assert_eq!(a.offered(), [Alpn::Link, Alpn::Svc, Alpn::Join]);
        assert_eq!(a.check(&peer(5), Alpn::Join, Side::Accepted), Ok(BTreeSet::new()));
        assert!(!a.close_ticket());
        assert!(a.close_ticket(), "the last ticket withdraws it");
        assert!(!a.close_ticket(), "no ticket left to close");
        assert_eq!(a.check(&peer(5), Alpn::Join, Side::Accepted), Err(Refused::NoTicket));
        // A join is never a way in to the members' protocols.
        a.open_ticket();
        assert_eq!(a.check(&peer(5), Alpn::Link, Side::Accepted), Err(Refused::NotAMember));
    }

    const SYNC: Alpn = Alpn::App(b"xenolith/sync/2");
    const JOIN1: Alpn = Alpn::App(b"xenolith/join/1");
    const SPACE: &str = "spc_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn another_programs_doors_admit_the_members_of_its_own_groups_and_nothing_of_xenonets() {
        let a = Admission::with(Doors { members: vec![SYNC], join: Some(JOIN1) });
        a.set_members(SPACE, [peer(1)]);
        assert_eq!(a.protocols(), [SYNC, JOIN1]);
        assert_eq!(a.offered(), [SYNC], "its join only while a ticket is open");
        assert_eq!(a.alpn(b"xenolith/sync/2"), Some(SYNC));
        assert_eq!(a.alpn(b"xenonet/link/2"), None);
        assert_eq!(a.check(&peer(1), SYNC, Side::Accepted), Ok([SPACE.to_string()].into()));
        assert_eq!(a.check(&peer(2), SYNC, Side::Dialled), Err(Refused::NotAMember));
        for alpn in Alpn::ALL {
            assert_eq!(a.check(&peer(1), alpn, Side::Accepted), Err(Refused::NoDoor), "a member of a space is no way in to {alpn:?}");
        }
        assert_eq!(a.check(&peer(3), JOIN1, Side::Accepted), Err(Refused::NoTicket));
        a.open_ticket();
        assert_eq!(a.offered(), [SYNC, JOIN1]);
        assert_eq!(a.check(&peer(3), JOIN1, Side::Accepted), Ok(BTreeSet::new()));
        assert!(!a.is_members(JOIN1) && a.is_members(SYNC));
    }

    #[test]
    fn xenonets_forwarded_protocols_admit_its_networks_members_and_never_replace_its_own() {
        let a = Admission::with(Doors::xenonet_forwarding(&[SYNC, Alpn::Link, Alpn::Join]));
        assert_eq!(a.protocols(), [Alpn::Link, Alpn::Svc, SYNC, Alpn::Join]);
        a.set_members("n1", [peer(1)]);
        assert_eq!(a.check(&peer(1), SYNC, Side::Dialled), Ok(["n1".to_string()].into()));
        assert_eq!(a.check(&peer(9), SYNC, Side::Accepted), Err(Refused::NotAMember));
        assert_eq!(a.check(&peer(1), JOIN1, Side::Accepted), Err(Refused::NoDoor));
    }
}
