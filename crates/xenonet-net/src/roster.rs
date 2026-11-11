//! The roster (docs/design/network.md, section 3): the signed chain of a
//! network's hosts and their keys (`network/roster-entry/v2`), and who may
//! sign what. It is `xenonet-log`'s chain under the network's policy
//! (`xenonet_log::network`, where the table of rules is); this module names
//! it for the rest of the network, and its errors are `NetError`s through
//! `From`.

pub use xenonet_log::chain::{ChainError, Offered, Signature, MAX_SIGNERS};
pub use xenonet_log::network::{
    found, is_relay_url, needs_approval, Body, Entry, GatewayAt, HostRef, Member, NetworkPolicy, Op, Removal, Roster, Settings, Start, View, Wake, ADDRESS_MAX, ADMIT,
    MAX_MAILBOXES, MAX_RELAYS, ROSTER_TAG, WIDEN,
};

#[cfg(test)]
mod tests {
    use crate::codec::is_network_id;
    use xenonet_model::ids::is_host_id;

    #[test]
    fn the_roster_reads_ids_as_the_rest_of_the_network_does() {
        // xenonet-log keeps its own copies of these two rules, since it
        // doesn't depend on the model or the network crate.
        for id in ["studio", "s", "a-b", "a1", "1a", "Studio", "a_b", "a--b", "a-", &"a".repeat(32), &"a".repeat(33), ""] {
            assert_eq!(xenonet_log::network::is_host_id(id), is_host_id(id), "{id:?}");
        }
        for id in ["net_0123456789abcdef0123456789abcdef", "net_0123456789ABCDEF0123456789abcdef", "net_0123", "nett0123456789abcdef0123456789abcdef"] {
            assert_eq!(xenonet_log::network::is_network_id(id), is_network_id(id), "{id:?}");
        }
    }
}
