//! Checkouts on this machine (xenonet-store's `projects`): where a path
//! is, the project a folder is in, who may claim in a project, and who
//! this machine's projects are owned by. What a folder is, as a checkout to
//! keep, is xenonet-saved's (`checkouts`), with the Store core that keeps
//! them.

use std::sync::{Arc, Weak};
use xenonet_os::paths::path_key;
use xenonet_store::projects::Owner;
use xenonet_store::Db;

pub use xenonet_saved::checkouts::{found_of, json_of, Why};

/// Where a path (or a glob's folder) is: the checkout with the longest
/// path key at or above it, its project resolved, and the path inside it,
/// as a path key has it (lowercase, / between parts), a folder's trailing
/// / kept.
pub fn place(db: &Db, path: &str) -> Result<Option<xenonet_engine::host::Place>, xenonet_store::StoreError> {
    let head = path.split('*').next().unwrap_or("");
    let folder = if path.contains('*') { head.rsplit_once(['/', '\\']).map_or(head, |(d, _)| d) } else { path };
    let Some(key) = path_key(folder).filter(|k| !k.is_empty()) else { return Ok(None) };
    let Some(at) = db.place(&key)? else { return Ok(None) };
    let full = path_key(path).unwrap_or_default();
    let mut rel = full.get(at.checkout.path_key.len()..).unwrap_or("").trim_start_matches('/').to_string();
    if (path.ends_with('/') || path.ends_with('\\')) && !rel.is_empty() {
        rel.push('/');
    }
    let project = db.resolve(&at.checkout.project)?.unwrap_or(at.checkout.project.clone());
    Ok(Some(xenonet_engine::host::Place {
        checkout: at.checkout.id.as_str().to_string(),
        top: at.checkout.path.clone().unwrap_or(at.checkout.path_key.clone()),
        project: project.as_str().to_string(),
        rel,
        confirmed: at.checkout.confirmed,
    }))
}

/// Whether `me` may claim in `project`, decided with xenonet-authz: an
/// agent here may; an actor only in the projects its grant names, with
/// the token it called with.
pub fn may_claim(w: &crate::compose::runtime::Runtime, me: &str, project: &str, token: Option<&str>) -> Result<(), String> {
    let Some(slug) = xenonet_model::ids::parse_actor_id(me) else { return Ok(()) };
    let token = token.ok_or("an actor claims with its own token")?;
    let g = crate::compose::runtime::ground::ground(w, None).ok_or("this machine's governance isn't plugged in")?;
    let token = xenonet_authz::Token::parse(token, xenonet_authz::ACTOR_PREFIX, &g, g.facts.now).map_err(|e| e.message)?;
    let project = xenonet_authz::ProjectId::new(project).map_err(|e| e.message)?;
    let slug = xenonet_authz::ActorSlug::new(slug).map_err(|e| e.message)?;
    let request = xenonet_authz::Request::new(
        xenonet_authz::Principal::Actor { slug, machine: g.facts.machine.clone() },
        xenonet_authz::Resource::Project(project.clone()),
        xenonet_authz::Act::Claim { project },
        xenonet_authz::Door::Local,
        g.facts.now,
        None,
    );
    match xenonet_authz::decide(Some(&token), &request, &g, None) {
        d if d.is_allow() => Ok(()),
        d => Err(d.reason().map_or_else(|| "not allowed".to_string(), |r| r.text().to_string())),
    }
}

/// Whom this machine's claims are authored by: its signing key's id, or
/// `local` before it has one.
pub fn author(w: &crate::compose::runtime::Runtime) -> String {
    crate::compose::runtime::ground::governance(w).and_then(|g| g.identity().machine_key().ok()).map_or_else(|| xenonet_store::projects::LOCAL.to_string(), |k| k.public().id())
}

/// The project a folder is in here, through a checkout the person
/// confirmed in it (made from the folder when there's none): what this
/// machine's agents are matched across machines by.
pub fn project_of(db: &Db, folder: &str) -> Option<String> {
    let key = path_key(folder)?;
    // Made when there's none; found again when its project was made before
    // the person was known, so it's re-keyed into theirs.
    let ownerless = || db.place(&key).ok().flatten().and_then(|p| db.project(&p.checkout.project).ok().flatten()).is_some_and(|p| p.owner.is_none() && p.repo_key.is_some());
    if db.place(&key).ok()?.is_none() || (db.owner_known() && ownerless()) {
        let found = found_of(folder, Why::Session)?;
        db.ensure(&found, xenonet_text::time::now_ms() as i64).ok()?;
    }
    let at = db.place(&key).ok()??;
    if !at.checkout.confirmed {
        return None;
    }
    db.resolve(&at.checkout.project).ok()?.map(|p| p.as_str().to_string())
}

/// Who this machine's projects are owned by: the person this machine is
/// theirs (a device of theirs is here), else the person who owns its
/// network (a machine of theirs with no device of theirs, a server), with
/// their chain's salt, once the network's governance is plugged in; nobody
/// before.
pub fn owner(w: &crate::compose::runtime::Runtime) -> Owner {
    let Some(g) = crate::compose::runtime::ground::governance(w) else { return Owner::default() };
    let identity = g.identity();
    let chain = match identity.chain().person() {
        Some(_) => Some(identity.chain()),
        None => g.facts().view.and_then(|v| v.owner.clone()).and_then(|o| identity.chain_of(&o)),
    };
    match chain.as_ref().and_then(|c| c.person()) {
        Some(p) => Owner { person: Some(p.id.clone()), salt: Some(p.salt) },
        None => Owner::default(),
    }
}

/// Puts the runtime's person behind the store's owner of projects.
pub fn own(db: &Db, w: Weak<crate::compose::runtime::Runtime>) {
    db.set_owner(Arc::new(move || w.upgrade().map(|w| owner(&w)).unwrap_or_default()));
}
