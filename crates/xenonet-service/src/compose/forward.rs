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

/// A forward decided on the engine's grants.
struct Decided(Arc<dyn GrantsCore>);

impl Grants for Decided {
    fn decide(&self, token: &str, alpn: &str) -> R<Grant> {
        match self.0.forward(token, alpn) {
            Ok((actor, ids)) => Ok(Grant { actor, ids }),
            Err(code @ "other-alpn") => Err(NetError::new(403, code, format!("that grant forwards another ALPN than {alpn}"))),
            Err(code @ ("revoked" | "expired")) => Err(NetError::new(403, code, "that grant was revoked or is past its time")),
            Err(_) => Err(NetError::new(403, "no-grant", format!("no grant of this machine's actors forwards {alpn}"))),
        }
    }
}

struct Core(Arc<Forwards>);

impl ForwardsCore for Core {
    fn list(&self) -> Result<Value, CoreError> {
        Ok(self.0.list())
    }

    fn net(&self) -> Option<Arc<Forwards>> {
        Some(self.0.clone())
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
    cores.forwards = Arc::new(Core(forwards));
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
            fn forward(&self, _token: &str, _alpn: &str) -> Result<(String, Vec<String>), &'static str> {
                Err(self.0)
            }
        }
        for (said, code) in [("other-alpn", "other-alpn"), ("revoked", "revoked"), ("expired", "expired"), ("no-grant", "no-grant"), ("invalid", "no-grant")] {
            assert_eq!(Decided(Arc::new(Says(said))).decide("t", "xenolith/sync/2").unwrap_err().code, code);
        }
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
