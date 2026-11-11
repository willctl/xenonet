//! Signed hash chains (docs/design/direction.md, "Domain model"): a
//! group's history as entries, each naming the one before it by hash and
//! signed by whoever had the right to add it. Every holder checks the
//! chain whole and takes only entries that extend its own head, so a relay
//! can withhold an entry but never add or change one.
//!
//! - `chain`: the chain itself, `Chain<P: Policy>`: indexes and `prev`
//!   hashes, gaps, forks and duplicates, signers (each once, four at
//!   most), their signatures, entries signed rightly by too few
//!   (`needs-approval`), and the JSON a chain is kept and sent as.
//! - `network`: a network's chain of machines, the roster
//!   (`xenonet-roster/2`), as the network crate runs it, with its
//!   members' settings and the presence its admits and widenings need.
//! - `person`: a person's chain of device keys (`xenonet-person/1`): the
//!   root key, the devices it or two devices pair, and what they revoke.
//! - `feed`: a machine's feed in a network (`xenonet-feed/1`), what it
//!   tells the other members, applied once and in order by each receiver.
//!
//! Why `Policy` is a trait rather than one chain with a kind field: each
//! chain kind (network, person, and later org, project, space) has its own
//! entry shape, its own byte-exact canonical form, and its own table of who
//! may sign what, and those are the only parts that differ. The hash
//! chain, the signer rules and the JSON envelope are the same for all, so
//! they're written once in `chain`, and an org chain is one more `Policy`
//! with no change here. A policy is a value, not just a type, so one that
//! has to look outside its own chain (the network chain checking a
//! signer's device against the person chain at the head an entry names)
//! holds what it consults, such as an `Arc<dyn person::Devices>`.

pub mod chain;
pub mod feed;
pub mod network;
pub mod person;
