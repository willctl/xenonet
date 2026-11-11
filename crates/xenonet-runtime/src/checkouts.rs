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
    Found { path: path.to_string(), path_key: key, vcs: None, branch: None, head: None, repo_key: None, kind: CheckoutKind::Folder, parent: None, name: name_of(path) }
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

/// Who this machine's projects are owned by: the person this machine is
/// theirs, with their chain's salt, once the network's governance is
/// plugged in; nobody before.
pub fn owner(w: &crate::runtime::Runtime) -> Owner {
    let Some(g) = crate::ground::governance(w) else { return Owner::default() };
    match g.identity().chain().person() {
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
