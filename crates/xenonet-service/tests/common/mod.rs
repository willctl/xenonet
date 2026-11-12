// What the network tests share: a request as a local program makes it,
// and the store a test machine keeps its network in.

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

/// What a test machine keeps of its network and person: the store in its
/// state folder, which outlives a restart of the machine.
pub fn kept(state: &std::path::Path) -> std::sync::Arc<dyn xenonet_net::persist::Persist> {
    let db = xenonet_store::Db::open(state).expect("the test machine's store");
    std::sync::Arc::new(xenonet_service::compose::net::StorePersist(std::sync::Arc::new(db)))
}
