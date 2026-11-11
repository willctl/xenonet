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

/// A request as a program of this account sends it: with the API's
/// credential from its file, unless it brings a credential of its own (the
/// window's key, an actor's token, the tray's key). A call to another
/// host's service (`/api/svc/…`) brings that service's `Authorization`,
/// which isn't one of XenoNet's.
pub fn local(ctx: &xenonet_service::ctx::Ctx, mut req: xenonet_service::http::Req) -> xenonet_service::http::Req {
    let svc = req.path.starts_with("/api/svc/");
    let own = ["x-xenonet-window", "authorization", "x-xenonet-credential", "x-xenonet-tray", "x-xenonet-self"];
    if !req.headers.iter().any(|(k, _)| own.contains(&k.as_str()) && !(svc && k == "authorization")) {
        req.headers.push(("x-xenonet-credential".into(), ctx.api_key.reveal().to_string()));
    }
    req
}
