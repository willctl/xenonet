//! Other machines of this one's network on its map (docs/design/
//! network.md, section 7). A console subscribes to a window host's map
//! through the network's gateway, sealed between the two machines, and
//! the serving machine's relay answers with frames. Everything another
//! machine sends is untrusted: frames are capped and checked, every id is
//! qualified with the machine's, only the fields the schemas declare are
//! kept, and nothing it sends can make this XenoNet write anywhere. A
//! relay sends only titles, states and counts unless asked for detail,
//! with the home folder (and any folder named for a person) written as ~,
//! and answers only an allowlist of reads. `runtime` runs them in the
//! service: the hub's links, the relay and the transport.

pub mod aliases;
pub mod cache;
pub mod declared;
pub mod frames;
pub mod hub;
pub mod link;
pub mod merge;
pub mod network;
pub mod relay;
pub mod runtime;
