// What the network tests share: loopback ports from the OS. A fixed port
// can already be taken, by another test binary or by an outgoing
// connection, since the ephemeral range those draw from covers most high
// ports, and then a listener fails with "Address already in use".

use std::net::TcpListener;

/// XenoNet's own ports, the person's XenoNet's: never handed to a test.
const XENONET: std::ops::RangeInclusive<u16> = 47840..=47842;

/// `N` free ports on 127.0.0.1, each different. The OS picks each one for
/// a listener on port 0; every listener is held until all the ports are
/// read, so no two are the same, then released for the test to listen on.
pub fn free_ports<const N: usize>() -> [u16; N] {
    let mut held = Vec::new();
    let mut ports = Vec::new();
    while ports.len() < N {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port on loopback");
        let port = listener.local_addr().expect("a bound listener's address").port();
        if !XENONET.contains(&port) {
            ports.push(port);
        }
        held.push(listener);
    }
    drop(held);
    std::array::from_fn(|i| ports[i])
}
