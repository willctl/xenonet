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
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Refused::NotAMember => "not-a-member",
            Refused::NotInvited => "not-invited",
            Refused::NoTicket => "no-ticket",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Refused::NotAMember => "that machine is in none of this machine's networks",
            Refused::NotInvited => "no invite on this machine names that machine",
            Refused::NoTicket => "this machine has no join ticket open",
        }
    }
}

impl From<Refused> for NetError {
    fn from(r: Refused) -> NetError {
        NetError::new(403, r.code(), r.words())
    }
}

#[derive(Default, Debug)]
struct State {
    // Each network this machine is in, and its members' endpoint ids, as
    // this machine's own copy of the chain has them.
    networks: BTreeMap<String, BTreeSet<PeerId>>,
    // The inviters this machine is joining through, by the endpoint id
    // each invite names, and how many joins name each.
    joining: BTreeMap<PeerId, usize>,
    // How many of this machine's own tickets are open.
    tickets: usize,
}

/// The membership a connection is checked against.
#[derive(Default, Debug)]
pub struct Admission(RwLock<State>);

impl Admission {
    pub fn new() -> Admission {
        Admission::default()
    }

    /// A network's members, in place of what was known of it: the members
    /// of this machine's copy of its chain.
    pub fn set_members(&self, network: &str, members: impl IntoIterator<Item = PeerId>) {
        self.0.writing().networks.insert(network.to_string(), members.into_iter().collect());
    }

    /// A network this machine left: none of its members is admitted for it.
    pub fn leave(&self, network: &str) {
        self.0.writing().networks.remove(network);
    }

    /// The networks `peer` shares with this machine.
    pub fn shared(&self, peer: &PeerId) -> BTreeSet<String> {
        self.0.reading().networks.iter().filter(|(_, m)| m.contains(peer)).map(|(n, _)| n.clone()).collect()
    }

    /// A join through the inviter an invite names begins: dials to it on
    /// join/2 are let through until `end_join`.
    pub fn begin_join(&self, inviter: PeerId) {
        *self.0.writing().joining.entry(inviter).or_default() += 1;
    }

    pub fn end_join(&self, inviter: &PeerId) {
        let mut s = self.0.writing();
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
        let mut s = self.0.writing();
        s.tickets += 1;
        s.tickets == 1
    }

    /// One of this machine's tickets closed (used, expired or retired).
    /// Returns whether join/2 has to be withdrawn now.
    pub fn close_ticket(&self) -> bool {
        let mut s = self.0.writing();
        if s.tickets == 0 {
            return false;
        }
        s.tickets -= 1;
        s.tickets == 0
    }

    pub fn tickets_open(&self) -> bool {
        self.0.reading().tickets > 0
    }

    /// The protocols this machine answers now: join/2 only while a ticket
    /// is open.
    pub fn offered(&self) -> Vec<Alpn> {
        let mut alpns = vec![Alpn::Link, Alpn::Svc];
        if self.tickets_open() {
            alpns.push(Alpn::Join);
        }
        alpns
    }

    /// Whether a connection with `peer` on `alpn`, dialled by this machine
    /// or by the peer (`side`), is admitted, and the networks it may carry
    /// (none on join/2).
    pub fn check(&self, peer: &PeerId, alpn: Alpn, side: Side) -> Result<BTreeSet<String>, Refused> {
        match (alpn, side) {
            (Alpn::Link | Alpn::Svc, _) => {
                let shared = self.shared(peer);
                if shared.is_empty() {
                    Err(Refused::NotAMember)
                } else {
                    Ok(shared)
                }
            }
            (Alpn::Join, Side::Dialled) => {
                if self.0.reading().joining.contains_key(peer) {
                    Ok(BTreeSet::new())
                } else {
                    Err(Refused::NotInvited)
                }
            }
            (Alpn::Join, Side::Accepted) => {
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
}
