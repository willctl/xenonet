//! Hosts (core/hosts/): this XenoNet as a host, its hosts.json and ssh
//! config (config.rs), the schemas what hosts send is checked and cut down
//! against (schemas.rs), and the hub that shows other machines' XenoNets on
//! this map, read-only (hub.rs): the hosts in hosts.json over SSH with
//! XENONET_HOSTS=1, and the hosts of this machine's network through its
//! gateway (transport.rs). The relay that serves this host's map to a
//! console on another host in the network is relay.rs.

pub mod config;
pub mod hub;
pub mod relay;
pub mod schemas;
pub mod transport;
