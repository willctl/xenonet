//! Who is on the other end of a request to the service, as the process
//! checks (the Verify core) are given it.

use std::net::SocketAddr;

/// The two ends of a request's connection: what core/engine/verify.js reads
/// off `req.socket` to find the process that sent it; and the connection
/// itself (`conn`, never given to another), so what's found for it can be
/// kept for its next requests, as verify.js keeps it on the socket.
#[derive(Clone, Copy, Debug)]
pub struct Peer {
    pub remote: SocketAddr,
    pub local: SocketAddr,
    pub conn: u64,
}

impl Peer {
    /// A new connection's two ends.
    pub fn new(remote: SocketAddr, local: SocketAddr) -> Peer {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Peer { remote, local, conn: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }
}
