//! Hosts (core/hosts/, docs/design/hosts.md): the other machines whose
//! XenoNets this one shows. Everything a host sends is untrusted: frames
//! are capped and checked, every id is qualified with the host's, only the
//! fields the schemas declare are kept, and nothing a host sends can make
//! this XenoNet write anywhere but the relay's standard input. A relay
//! sends only titles, states and counts unless asked for detail, with the
//! home folder (and any folder named for a person) written as ~, and
//! answers only an allowlist of reads. ssh runs with XenoNet's own config,
//! never a prompt, and a host key that must be known and unchanged.

pub mod aliases;
pub mod cache;
pub mod config;
pub mod declared;
pub mod doctor;
pub mod frames;
pub mod hub;
pub mod link;
pub mod merge;
pub mod relay;
