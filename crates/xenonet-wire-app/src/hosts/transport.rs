//! How the hub's links reach a host that isn't a program (docs/design/
//! network.md, section 7): the `network` transport, beside `ssh` and
//! `local`. A link writes the relay's console frames (get, time) and reads
//! the relay's frames back, whatever carries them; here they're the asks
//! and answers of `network/relay/v1`, through the network's gateway.
//!
//! The hub starts a subscription for a link's run, writes to it, renews it
//! and ends it through a `Transport`; what comes back reaches the hub's
//! relay sink as `{ sub, seq, frames }`, and each frame goes to the link as
//! a line would from ssh.

use serde_json::Value;
use std::sync::{Arc, Weak};
use xenonet_base::network::{Host, Incoming, Kind, Me, Net, Outgoing, RelaySink, To};
use xenonet_hosts::frames::{parse_frame, FROM_CONSOLE};
use xenonet_hosts::network::{self as wire, Op};

/// A link's way to a host's map.
pub trait Transport: Send + Sync {
    /// Starts a subscription to `host`'s map: its id.
    fn start(&self, host: &str) -> Result<String, String>;
    /// A frame the link writes (a get, or the time), for the subscription.
    fn write(&self, host: &str, sub: &str, text: &str);
    /// Still watching; with `fresh`, a new snapshot, since frames were lost.
    fn renew(&self, host: &str, sub: &str, fresh: bool);
    /// Ends it.
    fn end(&self, host: &str, sub: &str);
}

/// A new subscription's id.
pub fn new_sub() -> Result<String, String> {
    let b = xenonet_net::codec::random::<16>().map_err(|e| e.message)?;
    Ok(format!("sub_{}", xenonet_base::jsstr::hex(&b)))
}

/// What a link wrote, as the ask that carries it: a get or the time, and
/// nothing else a console sends.
pub fn op_of(text: &str) -> Option<Op> {
    let f = parse_frame(text.trim_end(), Some(&FROM_CONSOLE)).ok()?;
    let id = f.get("id").and_then(Value::as_u64).filter(|n| *n >= 1)?;
    match f.get("t").and_then(Value::as_str)? {
        "get" => Some(Op::Get { id, path: f.get("path").and_then(Value::as_str)?.to_string() }),
        "time" => Some(Op::Time { id }),
        _ => None,
    }
}

/// The network: asks go to the host through the gateway, sealed for it.
pub struct NetTransport {
    pub net: Arc<dyn Net>,
}

impl NetTransport {
    fn send(&self, host: &str, sub: &str, op: &Op) -> Result<(), String> {
        self.net.send(Outgoing { to: To::Host(host.into()), kind: Kind::Relay, body: wire::ask(sub, op) })
    }
}

impl Transport for NetTransport {
    fn start(&self, host: &str) -> Result<String, String> {
        let sub = new_sub()?;
        self.send(host, &sub, &Op::Open)?;
        Ok(sub)
    }
    fn write(&self, host: &str, sub: &str, text: &str) {
        if let Some(op) = op_of(text) {
            let _ = self.send(host, sub, &op);
        }
    }
    fn renew(&self, host: &str, sub: &str, fresh: bool) {
        let _ = self.send(host, sub, &Op::Renew { fresh });
    }
    fn end(&self, host: &str, sub: &str) {
        let _ = self.send(host, sub, &Op::Close);
    }
}

// ---------- a host in the same process, for XenoNet Dev ----------

/// The network between this machine's hub and a relay of its own, for
/// XenoNet Dev's net-relay simulation: the relay serves this machine's own
/// map as if it were `host`, to a console on `console`, a window host.
pub struct LoopNet {
    /// The host the relay plays.
    pub host: String,
    /// The console's host.
    pub console: String,
    /// The hub's relay sink, which the relay's answers reach.
    pub hub: Weak<dyn RelaySink>,
}

impl Net for LoopNet {
    fn me(&self) -> Option<Me> {
        Some(Me { network: "net_00000000000000000000000000000000".into(), name: "made up".into(), host: self.host.clone() })
    }
    fn hosts(&self) -> Vec<Host> {
        vec![Host { id: self.console.clone(), window: true, online: true, seen: None }]
    }
    fn send(&self, out: Outgoing) -> Result<(), String> {
        let Some(hub) = self.hub.upgrade() else { return Err("the hub stopped".into()) };
        let from = self.host.clone();
        // Apart from the caller, which may hold the hub's lock.
        tokio::spawn(async move { hub.relay(Incoming { from, kind: out.kind, body: out.body, at: crate::fx::now_ms(), from_window_host: false }) });
        Ok(())
    }
}

/// The hub's side of that: asks go straight to the relay, from the console.
pub struct LoopTransport {
    pub relay: Arc<dyn RelaySink>,
    pub console: String,
}

impl LoopTransport {
    fn send(&self, sub: &str, op: &Op) {
        let (relay, from, body) = (self.relay.clone(), self.console.clone(), wire::ask(sub, op));
        tokio::spawn(async move { relay.relay(Incoming { from, kind: Kind::Relay, body, at: crate::fx::now_ms(), from_window_host: true }) });
    }
}

impl Transport for LoopTransport {
    fn start(&self, _host: &str) -> Result<String, String> {
        let sub = new_sub()?;
        self.send(&sub, &Op::Open);
        Ok(sub)
    }
    fn write(&self, _host: &str, sub: &str, text: &str) {
        if let Some(op) = op_of(text) {
            self.send(sub, &op);
        }
    }
    fn renew(&self, _host: &str, sub: &str, fresh: bool) {
        self.send(sub, &Op::Renew { fresh });
    }
    fn end(&self, _host: &str, sub: &str) {
        self.send(sub, &Op::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_writes_gets_and_the_time_and_nothing_else() {
        assert_eq!(op_of("{\"t\":\"get\",\"id\":3,\"path\":\"/api/graph\"}\n"), Some(Op::Get { id: 3, path: "/api/graph".into() }));
        assert_eq!(op_of("{\"t\":\"time\",\"id\":4}\n"), Some(Op::Time { id: 4 }));
        for other in ["{\"t\":\"exec\",\"id\":1}", "{\"t\":\"get\",\"id\":0,\"path\":\"/x\"}", "{\"t\":\"get\",\"id\":1}", "nope"] {
            assert_eq!(op_of(other), None, "{other}");
        }
        let sub = new_sub().unwrap();
        assert!(wire::is_sub(&sub), "{sub}");
    }
}
