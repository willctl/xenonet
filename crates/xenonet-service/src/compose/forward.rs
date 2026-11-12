//! The actors' forwards (`xenonet_net::forward`), at the composition root:
//! which ALPNs this machine forwards (registered here, as data, so no
//! library crate names another program's protocol), how a grant to
//! forward one is decided (the engine's grants, over xenonet-authz), the
//! revocations that end them (the store's), and the owner-only socket in
//! the runtime folder the actors reach them on.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xenonet_api::cores::{CoreError, Cores, Forwards as ForwardsCore, Grants as GrantsCore};
use xenonet_net::forward::{Forwardable, Forwards, Grant, Grants};
use xenonet_net::{NetError, R};

/// The protocols this machine carries for an actor whose grant names one:
/// Xenolith's sync/2, for its shared mode.
pub const FORWARDED: &[&[u8]] = &[b"xenolith/sync/2"];

/// Whose a forwarded ALPN is, and the rights its grant holds: Xenolith's
/// sync/2 is the actor `xenolith`'s, reading, appending to and writing the
/// spaces it's in.
pub fn actor_of(alpn: &str) -> Option<(&'static str, &'static [&'static str])> {
    match alpn {
        "xenolith/sync/2" => Some(("xenolith", &["ctx-read", "ctx-append", "ctx-write"])),
        _ => None,
    }
}

/// A forward decided on the engine's grants.
struct Decided(Arc<dyn GrantsCore>);

impl Grants for Decided {
    fn decide(&self, token: &str, alpn: &str) -> R<Grant> {
        match self.0.forward(token, alpn) {
            Ok(f) => Ok(Grant { actor: f.actor, ids: f.ids, networks: f.networks.map(|n| n.into_iter().collect()), until: f.until }),
            Err(code @ "other-alpn") => Err(NetError::new(403, code, format!("that grant forwards another ALPN than {alpn}"))),
            Err(code @ ("revoked" | "expired")) => Err(NetError::new(403, code, "that grant was revoked or is past its time")),
            Err(_) => Err(NetError::new(403, "no-grant", format!("no grant of this machine's actors forwards {alpn}"))),
        }
    }

    fn collect(&self, alpn: &str, key: &str) -> R<String> {
        let (actor, _) = actor_of(alpn).ok_or_else(|| NetError::new(403, "no-grant", format!("{alpn} has no actor here")))?;
        match self.0.collect_forward(actor, alpn, key) {
            Ok(text) => Ok(text),
            // Away past its grant's end, it asks the person again.
            Err("lapsed") => Err(NetError::new(403, "unapproved", "the last grant for that key ended unrenewed: the person approves it again")),
            Err(_) => Err(NetError::new(403, "unapproved", "the person hasn't approved a grant for that key here")),
        }
    }
}

struct Core {
    forwards: Arc<Forwards>,
    grants: Arc<dyn GrantsCore>,
    context: Arc<crate::compose::context::Context>,
}

fn refused(status: u16, code: &str, message: impl Into<String>) -> CoreError {
    CoreError { status: Some(status), code: Some(code.to_string()), ..CoreError::new(message) }
}

#[async_trait::async_trait]
impl ForwardsCore for Core {
    fn list(&self) -> Result<Value, CoreError> {
        let mut v = self.forwards.list();
        v["grants"] = self.grants.forward_grants();
        // The spaces Xenolith curates, with what the window offers for each.
        v["spaces"] = self.context.view();
        // Each ask with the actor it's for.
        if let Some(asks) = v["asks"].as_array_mut() {
            for a in asks {
                a["actor"] = a["alpn"].as_str().and_then(actor_of).map_or(Value::Null, |(actor, _)| Value::from(actor));
            }
        }
        Ok(v)
    }

    fn approve(&self, alpn: &str, key: &str, networks: &[String], window: bool) -> Result<Value, CoreError> {
        if !window {
            return Err(refused(403, "window", "a grant is approved at XenoNet's window, where it asks you"));
        }
        let (actor, rights) = actor_of(alpn).ok_or_else(|| refused(404, "not-found", format!("{alpn} isn't forwarded here")))?;
        // The key proved it's held here (its collect's proof) before the
        // person is asked for it.
        if !self.forwards.asks().iter().any(|a| a.alpn == alpn && a.key == key) {
            return Err(refused(404, "not-found", "no ask for a grant for that key waits"));
        }
        let rights: Vec<String> = rights.iter().map(|r| (*r).to_string()).collect();
        let reason = format!("let {actor} on this machine sync over XenoNet's network");
        let out = self.grants.approve_forward(actor, alpn, key, networks, &rights, &reason).map_err(|e| refused(409, "refused", e))?;
        self.forwards.answered(alpn, key);
        Ok(out)
    }

    fn deny(&self, alpn: &str, key: &str) -> Result<Value, CoreError> {
        Ok(serde_json::json!({ "denied": self.forwards.answered(alpn, key) }))
    }

    fn revoke(&self, alpn: &str) -> Result<Value, CoreError> {
        let (actor, _) = actor_of(alpn).ok_or_else(|| refused(404, "not-found", format!("{alpn} isn't forwarded here")))?;
        match self.grants.revoke_forward(actor).map_err(|e| refused(500, "error", e))? {
            Some(v) => Ok(serde_json::json!({ "revoked": v })),
            None => Err(refused(404, "not-found", format!("{actor} has no grant here"))),
        }
    }

    async fn sign(&self, body: &Value, window: bool) -> Result<Value, CoreError> {
        let asked = crate::compose::context::asked_of(body).map_err(|e| refused(400, "bad-request", e))?;
        let space = body.get("space").and_then(Value::as_str).filter(|s| s.starts_with("spc_")).ok_or_else(|| refused(400, "bad-request", "name the space (spc_…)"))?;
        let out = self.context.sign(space, &asked, window).await.map_err(|e| refused(409, "refused", e))?;
        Ok(serde_json::json!({ "space": out }))
    }

    fn on_net(&self, nodes: &Arc<xenonet_net::nodes::Nodes>) {
        self.context.set_nodes(nodes);
    }

    fn changed(&self) {
        self.context.changed();
    }

    fn net(&self) -> Option<Arc<Forwards>> {
        Some(self.forwards.clone())
    }
}

/// The forwards, in `cores` once the engine's grants are: a grant revoked
/// in the store ends what it forwards.
pub fn wire(cores: &mut Cores) {
    let Ok(forwardable) = Forwardable::new(FORWARDED) else { return };
    let forwards = Forwards::new(forwardable, Arc::new(Decided(cores.grants.clone())));
    if let Some(store) = cores.engine_store.native().and_then(|a| a.downcast_ref::<xenonet_store::engine::Shared>().cloned()) {
        let weak = Arc::downgrade(&forwards);
        store.0.on_revoke(Arc::new(move |id: &str| {
            if let Some(f) = weak.upgrade() {
                f.revoked(id);
            }
        }));
    }
    let context = crate::compose::context::Context::new(&forwards);
    cores.forwards = Arc::new(Core { forwards, grants: cores.grants.clone(), context });
}

/// Copies of this machine's chains for its actors (`Forwards::set_chains`):
/// each network's roster, the person chains of their owners and this
/// machine's own person, and its token roots' certificates. Public
/// records, each verified by the actor from genesis; nothing secret.
pub fn chains_of(nodes: &xenonet_net::nodes::Nodes) -> Value {
    let common = nodes.common();
    let identity = &common.identity;
    let mut networks = Vec::new();
    let mut owners: Vec<String> = identity.person_id().into_iter().collect();
    for s in common.shareds() {
        let Some(n) = s.network() else { continue };
        let roster = s.roster();
        if let Some(o) = roster.view_ref().and_then(|v| v.owner.clone()) {
            if !owners.contains(&o) {
                owners.push(o);
            }
        }
        networks.push(serde_json::json!({ "id": n.id, "chain": roster.to_json() }));
    }
    let persons: Vec<Value> = owners.iter().filter_map(|p| identity.chain_of(p).map(|c| serde_json::json!({ "person": p, "chain": c.to_json() }))).collect();
    let roots: Vec<Value> = identity.all_token_roots().iter().map(xenonet_authz::RootCertificate::to_json).collect();
    serde_json::json!({ "networks": networks, "persons": persons, "roots": roots })
}

/// Where the actors reach the forwards of the service on `port`.
pub fn socket_path(runtime_dir: &Path, port: u16) -> PathBuf {
    runtime_dir.join(format!("forward-{port}.sock"))
}

/// Listens on the owner-only socket for the actors, until the service
/// stops; only a peer of this account is served.
#[cfg(unix)]
pub fn listen(forwards: Arc<Forwards>, path: &Path, log: impl Fn(&str) + Send + Sync + 'static) -> std::io::Result<tokio::task::JoinHandle<()>> {
    use std::os::unix::fs::PermissionsExt;
    // A socket left by an earlier start; anything else there stays.
    if std::fs::symlink_metadata(path).is_ok_and(|m| std::os::unix::fs::FileTypeExt::is_socket(&m.file_type())) {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let me = xenonet_os::paths::own_uid();
    Ok(tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            if !stream.peer_cred().is_ok_and(|c| c.uid() == me) {
                log("a connection to the forward socket from another account was refused");
                continue;
            }
            let (r, w) = stream.into_split();
            tokio::spawn(forwards.clone().serve(Box::new(r), Box::new(w)));
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_another_programs_alpn_is_forwarded_and_the_grants_say_why_not() {
        assert!(Forwardable::new(FORWARDED).is_ok());
        struct Says(&'static str);
        impl GrantsCore for Says {
            fn forward(&self, _token: &str, _alpn: &str) -> Result<xenonet_api::cores::Forwarding, &'static str> {
                Err(self.0)
            }
            fn collect_forward(&self, _actor: &str, _alpn: &str, _key: &str) -> Result<String, &'static str> {
                Err(self.0)
            }
        }
        for (said, code) in [("other-alpn", "other-alpn"), ("revoked", "revoked"), ("expired", "expired"), ("no-grant", "no-grant"), ("invalid", "no-grant")] {
            assert_eq!(Decided(Arc::new(Says(said))).decide("t", "xenolith/sync/2").unwrap_err().code, code);
        }
        // Lapsed or never approved, a collect waits for the person.
        for said in ["unapproved", "lapsed"] {
            assert_eq!(Decided(Arc::new(Says(said))).collect("xenolith/sync/2", "k").unwrap_err().code, "unapproved");
        }
        assert_eq!(Decided(Arc::new(Says("unapproved"))).collect("other/alpn/1", "k").unwrap_err().code, "no-grant");
    }

    #[test]
    fn approving_a_grant_is_the_windows_and_needs_a_proved_ask() {
        let forwards = Forwards::new(Forwardable::new(FORWARDED).unwrap(), Arc::new(Decided(Arc::new(xenonet_api::cores::Pending))));
        let core = Core { forwards: forwards.clone(), grants: Arc::new(xenonet_api::cores::Pending), context: crate::compose::context::Context::new(&forwards) };
        let nets = ["net_00000000000000000000000000000001".to_string()];
        assert_eq!(core.approve("xenolith/sync/2", "xns1_k", &nets, false).unwrap_err().code.as_deref(), Some("window"));
        assert_eq!(core.approve("xenolith/sync/2", "xns1_k", &nets, true).unwrap_err().code.as_deref(), Some("not-found"), "no ask waits for that key");
        forwards.ask("xenolith/sync/2", "xns1_k").unwrap();
        assert_eq!(core.list().unwrap()["asks"][0]["actor"], "xenolith");
        assert_eq!(core.approve("other/alpn/1", "xns1_k", &nets, true).unwrap_err().code.as_deref(), Some("not-found"));
        assert_eq!(core.deny("xenolith/sync/2", "xns1_k").unwrap()["denied"], true);
        assert_eq!(core.list().unwrap()["asks"], serde_json::json!([]));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_socket_is_this_accounts_alone_and_an_attach_on_it_is_decided() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xn-fwd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = socket_path(&dir, 4);
        let forwards = Forwards::new(Forwardable::new(FORWARDED).unwrap(), Arc::new(Decided(Arc::new(xenonet_api::cores::Pending))));
        let task = listen(forwards, &path, |_| {}).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let alpn = xenonet_net::dial::Alpn::App(b"xenolith/sync/2");
        let refused = xenonet_net::forward::ForwardDial::attach(xenonet_net::forward::unix(path.clone()), "xenonet_actor_none", alpn).await.err().map(|e| e.code);
        assert_eq!(refused, Some("no-grant"));
        // A start after a crash finds its old socket and takes the place.
        task.abort();
        let again = Forwards::new(Forwardable::new(FORWARDED).unwrap(), Arc::new(Decided(Arc::new(xenonet_api::cores::Pending))));
        assert!(listen(again, &path, |_| {}).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
