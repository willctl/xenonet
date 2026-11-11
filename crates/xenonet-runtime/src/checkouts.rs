//! Checkouts as this machine finds them on its disk (xenonet-store's
//! `projects`): what the version control the binary installed
//! (`xenonet_adapter::repo`) says of a folder, as a row the store diffs in,
//! and who this machine's projects are owned by.

use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Weak};
use xenonet_os::paths::path_key;
use xenonet_store::projects::{Checkout, CheckoutKind, Found, Owner};
use xenonet_store::Db;

/// How a folder came to be looked at: a project the map's scan found
/// (a repo, or a folder that isn't one), or where a session ran, which is
/// the checkout it's in, or the folder itself outside any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Repo,
    Folder,
    Session,
}

impl Why {
    pub fn parse(s: &str) -> Option<Why> {
        match s {
            "repo" => Some(Why::Repo),
            "folder" => Some(Why::Folder),
            "session" => Some(Why::Session),
            _ => None,
        }
    }
}

fn name_of(path: &str) -> String {
    let t = path.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(t).to_string()
}

fn folder(path: &str, key: String) -> Found {
    Found {
        path: path.to_string(),
        path_key: key,
        vcs: None,
        branch: None,
        head: None,
        repo_key: None,
        common: None,
        kind: CheckoutKind::Folder,
        parent: None,
        name: name_of(path),
    }
}

/// What's at `path`, as a checkout to keep: a repo's working tree, or a
/// folder. None for a path with no key.
pub fn found_of(path: &str, why: Why) -> Option<Found> {
    let key = path_key(path)?;
    let reader = xenonet_adapter::repo::checkouts();
    let ch = match why {
        Why::Repo => reader.open(Path::new(path)),
        Why::Session => reader.find(Path::new(path)),
        Why::Folder => None,
    };
    let Some(ch) = ch else { return Some(folder(path, key)) };
    let top = ch.top.to_string_lossy().into_owned();
    let (kind, parent) = match (&ch.worktree_of, &ch.submodule_of) {
        (Some(main), _) => (CheckoutKind::Worktree, path_key(&main.to_string_lossy())),
        (None, Some(sup)) => (CheckoutKind::Submodule, path_key(&sup.to_string_lossy())),
        (None, None) => (CheckoutKind::Repo, None),
    };
    Some(Found {
        path_key: path_key(&top).unwrap_or(key),
        name: name_of(&top),
        path: top,
        vcs: Some(ch.vcs.to_string()),
        branch: ch.branch,
        head: ch.head,
        repo_key: ch.key,
        common: path_key(&ch.common.to_string_lossy()),
        kind,
        parent,
    })
}

/// A checkout as the map and the API show it.
pub fn json_of(ch: &Checkout) -> Value {
    json!({
        "id": ch.id.as_str(),
        "project": ch.project.as_str(),
        "path": ch.path,
        "pathKey": ch.path_key,
        "kind": ch.kind.as_str(),
        "branch": ch.branch,
        "parent": ch.parent.as_ref().map(xenonet_model::ids::CheckoutId::as_str),
        "confirmed": ch.confirmed,
        "gone": ch.gone_at.is_some(),
    })
}

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
pub fn may_claim(w: &crate::runtime::Runtime, me: &str, project: &str, token: Option<&str>) -> Result<(), String> {
    let Some((None, slug)) = xenonet_model::ids::parse_actor_id(me) else { return Ok(()) };
    let token = token.ok_or("an actor claims with its own token")?;
    let g = crate::ground::ground(w, None).ok_or("this machine's governance isn't plugged in")?;
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
pub fn author(w: &crate::runtime::Runtime) -> String {
    crate::ground::governance(w).and_then(|g| g.identity().machine_key().ok()).map_or_else(|| xenonet_store::projects::LOCAL.to_string(), |k| k.public().id())
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
pub fn owner(w: &crate::runtime::Runtime) -> Owner {
    let Some(g) = crate::ground::governance(w) else { return Owner::default() };
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
pub fn own(db: &Db, w: Weak<crate::runtime::Runtime>) {
    db.set_owner(Arc::new(move || w.upgrade().map(|w| owner(&w)).unwrap_or_default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_outside_any_repo_is_a_folder_checkout() {
        let dir = std::env::temp_dir().join(format!("xenonet-checkouts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.to_string_lossy().into_owned();
        let f = found_of(&p, Why::Session).unwrap();
        assert_eq!((f.kind, f.repo_key.as_deref(), f.path_key), (CheckoutKind::Folder, None, path_key(&p).unwrap()));
        assert_eq!(found_of("", Why::Folder), None);
        assert_eq!(name_of("/w/atlas/"), "atlas");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
