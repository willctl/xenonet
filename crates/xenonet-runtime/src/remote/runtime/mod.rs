//! The network's maps on this machine, as the service runs them: the
//! schemas what another machine sends is checked and cut down against
//! (schemas.rs), the hub (the Hub core) that shows the other machines of
//! this one's network on its map, read-only, through its gateway (hub.rs,
//! transport.rs), and the relay that serves this machine's map to a
//! console on another machine of the network (relay.rs).

// A core's error carries what the routes answer (a status, a code, extra
// fields), and is rare: boxing it would only make each core wordier.
#![expect(clippy::result_large_err, reason = "a core's error is the routes' own, and rare")]

pub mod hub;
pub mod relay;
pub mod schemas;
pub mod transport;
