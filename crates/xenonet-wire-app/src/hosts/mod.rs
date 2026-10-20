//! Hosts (core/hosts/): this XenoNet as a host, its hosts.json and ssh
//! config (config.rs), the schemas what hosts send is checked and cut down
//! against (schemas.rs), and with XENONET_HOSTS=1 the hub that shows other
//! machines' XenoNets on this map, read-only (hub.rs).

pub mod config;
pub mod hub;
pub mod schemas;
