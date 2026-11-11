//! Projects and their checkouts (docs/design/direction.md, "Domain
//! model"): a project is a minted id, never a path; a checkout is one
//! folder of it on one machine. A machine holds many checkouts, and a
//! project has many, on many machines.
//!
//! - **Found, not configured.** Checkouts come from where sessions ran and
//!   from the folders the person names as roots; every session's folder
//!   gets one, a folder that isn't a repo too (kind `folder`). The map's
//!   reconcile diffs what it found into the rows (`reconcile`), and the
//!   engine makes the one a claim needs when the map hasn't yet
//!   (`ensure`).
//! - **One repo, one project.** A repo with an origin is the project its
//!   owner's salt makes of the normalized remote (`ProjectId::of_repo`),
//!   the same on each of the person's machines; one with no person yet, or
//!   a folder, gets a random id. When the person appears, an ownerless
//!   project of a repo is re-keyed once: its row points at the owned one
//!   (`merged_into`), and `resolve` follows it.
//! - **Joining is the person's.** A checkout joins a project that already
//!   has one only when the person confirms it at the window (`confirm`);
//!   the first checkout of a project founds it. Until then it's a
//!   candidate: it names the project it would join, and gives no rights in
//!   it, so a folder that copies a repo's remote can't take its claims,
//!   its grants or its intents.
//!
//! This machine's own rows have `machine` = `LOCAL`; another machine's
//! carry its signing key's id.

use crate::{Db, StoreError, R};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::sync::Arc;
use xenonet_model::ids::{CheckoutId, Ids, ProjectId, ProjectSalt};

/// Who this machine's projects are owned by, as the runtime answers it.
pub type OwnerFn = Arc<dyn Fn() -> Owner + Send + Sync>;

/// What this machine's own rows name as their machine.
pub const LOCAL: &str = "local";

/// The longest chain of merges `resolve` follows.
const MERGES_FOLLOWED: usize = 16;

pub(crate) const TABLES: &[&str] = &[
    // A project: its owner (the person whose salt made its id, or none),
    // the normalized remote it was made from (none for a folder), a name
    // to show, and the project it was merged into.
    "CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        owner TEXT,
        repo_key TEXT,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        merged_into TEXT,
        UNIQUE (owner, repo_key)
    ) STRICT",
    // A checkout: one folder of a project on one machine (its path none on
    // another machine), what its version control says of it, the checkout
    // it belongs to (a worktree's main one, a submodule's superproject),
    // whether the person confirmed it in its project, and when it was
    // found, last seen and gone.
    "CREATE TABLE checkouts (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        machine TEXT NOT NULL,
        path TEXT,
        path_key TEXT NOT NULL,
        vcs TEXT,
        branch TEXT,
        head TEXT,
        parent TEXT,
        kind TEXT NOT NULL,
        confirmed INTEGER NOT NULL,
        found_at INTEGER NOT NULL,
        seen_at INTEGER NOT NULL,
        gone_at INTEGER,
        UNIQUE (machine, path_key)
    ) STRICT",
    "CREATE INDEX checkouts_by_project ON checkouts (project)",
];

/// What a project is made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    /// A repo, by its remote.
    Repo,
    /// A folder with no remote.
    Folder,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Repo => "repo",
            ProjectKind::Folder => "folder",
        }
    }
    fn parse(s: &str) -> ProjectKind {
        if s == "repo" {
            ProjectKind::Repo
        } else {
            ProjectKind::Folder
        }
    }
}

/// What a checkout is to its project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckoutKind {
    /// A repo's main working tree.
    Repo,
    /// A linked worktree of a repo.
    Worktree,
    /// A submodule inside its superproject.
    Submodule,
    /// A folder that isn't a repo.
    Folder,
}

impl CheckoutKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckoutKind::Repo => "repo",
            CheckoutKind::Worktree => "worktree",
            CheckoutKind::Submodule => "submodule",
            CheckoutKind::Folder => "folder",
        }
    }
    pub fn parse(s: &str) -> Option<CheckoutKind> {
        [CheckoutKind::Repo, CheckoutKind::Worktree, CheckoutKind::Submodule, CheckoutKind::Folder].into_iter().find(|k| k.as_str() == s)
    }
}

/// A project as the store keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub owner: Option<String>,
    pub repo_key: Option<String>,
    pub name: String,
    pub kind: ProjectKind,
    pub created_at: i64,
    pub merged_into: Option<ProjectId>,
}

/// A checkout as the store keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    pub id: CheckoutId,
    /// The project it's in, or for a candidate, the one it would join.
    pub project: ProjectId,
    pub machine: String,
    pub path: Option<String>,
    pub path_key: String,
    pub vcs: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub parent: Option<CheckoutId>,
    pub kind: CheckoutKind,
    /// In its project: the founding checkout, or one the person confirmed.
    pub confirmed: bool,
    pub found_at: i64,
    pub seen_at: i64,
    pub gone_at: Option<i64>,
}

/// A checkout as a reader of the disk found it, on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The folder, spelled as on disk.
    pub path: String,
    /// Its path key (`xenonet_os::paths::path_key`).
    pub path_key: String,
    pub vcs: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    /// origin normalized, for a repo that has one.
    pub repo_key: Option<String>,
    pub kind: CheckoutKind,
    /// The path key of the checkout it belongs to.
    pub parent: Option<String>,
    /// A name to give its project, when it founds one.
    pub name: String,
}

/// Whose this machine's projects are: the person, with the salt their
/// chain holds, when this machine has one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    pub person: Option<String>,
    pub salt: Option<ProjectSalt>,
}

/// What a reconcile changed, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// A new checkout, founding its project or (candidate) waiting to join
    /// one.
    Found { checkout: CheckoutId, path: String, project: ProjectId, candidate: bool },
    /// One seen again after it was gone.
    Back { checkout: CheckoutId, path: String },
    /// Its branch or head moved.
    Moved { checkout: CheckoutId, path: String, branch: Option<String> },
    /// It's in another project now (its remote changed, or a folder got
    /// one), confirmed only if it founds that one.
    Rejoined { checkout: CheckoutId, path: String, from: ProjectId, to: ProjectId, candidate: bool },
    /// Not found where it was.
    Gone { checkout: CheckoutId, path: String },
    /// An ownerless project re-keyed into its owner's id.
    Rekeyed { from: ProjectId, to: ProjectId },
}

impl Fix {
    /// The fix in words.
    pub fn words(&self) -> String {
        match self {
            Fix::Found { checkout, path, project, candidate: false } => format!("found {path} ({checkout}), in {project}"),
            Fix::Found { checkout, path, project, candidate: true } => {
                format!("found {path} ({checkout}), a checkout of {project}, which already has one: it joins once you confirm it in XenoNet's window")
            }
            Fix::Back { checkout, path } => format!("{path} ({checkout}) is back"),
            Fix::Moved { checkout, path, branch } => format!("{path} ({checkout}) moved to {}", branch.as_deref().unwrap_or("another head")),
            Fix::Rejoined { checkout, path, from, to, candidate } => {
                format!("{path} ({checkout}) is a checkout of {to} now, not {from}{}", if *candidate { ": it joins once you confirm it" } else { "" })
            }
            Fix::Gone { checkout, path } => format!("{path} ({checkout}) is gone"),
            Fix::Rekeyed { from, to } => format!("{from} is {to} now, its owner's id"),
        }
    }
}

/// Where a path is: the checkout with the longest path key at or above
/// it, and the path inside it (with `/` between parts, empty for its top).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub checkout: Checkout,
    pub rel: String,
}

fn project_of(r: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: ProjectId::parse(&r.get::<_, String>("id")?).map_err(|e| rusqlite::Error::InvalidColumnName(e.0))?,
        owner: r.get("owner")?,
        repo_key: r.get("repo_key")?,
        name: r.get("name")?,
        kind: ProjectKind::parse(&r.get::<_, String>("kind")?),
        created_at: r.get("created_at")?,
        merged_into: r.get::<_, Option<String>>("merged_into")?.and_then(|s| ProjectId::parse(&s).ok()),
    })
}

fn checkout_of(r: &Row<'_>) -> rusqlite::Result<Checkout> {
    let bad = |s: String| rusqlite::Error::InvalidColumnName(s);
    Ok(Checkout {
        id: CheckoutId::parse(&r.get::<_, String>("id")?).map_err(|e| bad(e.0))?,
        project: ProjectId::parse(&r.get::<_, String>("project")?).map_err(|e| bad(e.0))?,
        machine: r.get("machine")?,
        path: r.get("path")?,
        path_key: r.get("path_key")?,
        vcs: r.get("vcs")?,
        branch: r.get("branch")?,
        head: r.get("head")?,
        parent: r.get::<_, Option<String>>("parent")?.and_then(|s| CheckoutId::parse(&s).ok()),
        kind: CheckoutKind::parse(&r.get::<_, String>("kind")?).unwrap_or(CheckoutKind::Folder),
        confirmed: r.get::<_, i64>("confirmed")? != 0,
        found_at: r.get("found_at")?,
        seen_at: r.get("seen_at")?,
        gone_at: r.get("gone_at")?,
    })
}

fn one_project(c: &Connection, id: &str) -> R<Option<Project>> {
    Ok(c.prepare_cached("SELECT * FROM projects WHERE id = ?1")?.query_row(params![id], project_of).optional()?)
}

fn one_checkout(c: &Connection, id: &str) -> R<Option<Checkout>> {
    Ok(c.prepare_cached("SELECT * FROM checkouts WHERE id = ?1")?.query_row(params![id], checkout_of).optional()?)
}

fn checkout_at(c: &Connection, machine: &str, path_key: &str) -> R<Option<Checkout>> {
    Ok(c.prepare_cached("SELECT * FROM checkouts WHERE machine = ?1 AND path_key = ?2")?.query_row(params![machine, path_key], checkout_of).optional()?)
}

// The project `id` stands for now: itself, or what it was merged into.
fn resolved(c: &Connection, id: &ProjectId) -> R<ProjectId> {
    let mut at = id.clone();
    for _ in 0..MERGES_FOLLOWED {
        match one_project(c, at.as_str())?.and_then(|p| p.merged_into) {
            Some(next) if next != at => at = next,
            _ => return Ok(at),
        }
    }
    Err(StoreError::new("bad-entry", format!("{id} was merged into more projects in a row than the store follows")))
}

// Whether a project has any checkout yet, on any machine.
fn has_checkouts(c: &Connection, project: &ProjectId) -> R<bool> {
    Ok(c.prepare_cached("SELECT 1 FROM checkouts WHERE project = ?1 LIMIT 1")?.query_row(params![project.as_str()], |_| Ok(())).optional()?.is_some())
}

// The key and its parents, longest first: `/a/b/c`, `/a/b`, `/a`.
fn ancestors(key: &str) -> Vec<&str> {
    let mut out = vec![key];
    let mut k = key;
    while let Some(i) = k.rfind('/') {
        k = &k[..i];
        if k.is_empty() {
            break;
        }
        out.push(k);
    }
    out
}

impl Db {
    /// Where the store's random ids come from from now on: a seeded source
    /// for the demo and the golden files.
    pub fn use_ids(&self, ids: Ids) {
        *self.ids.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(ids);
    }

    /// The store's source of random ids.
    pub fn ids(&self) -> Arc<Ids> {
        self.ids.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Who this machine's projects are owned by, asked of `f` each time it
    /// matters: the runtime puts the person's chain behind it.
    pub fn set_owner(&self, f: OwnerFn) {
        *self.owner.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(f);
    }

    /// Whether this machine's person is known: projects made before they
    /// were are re-keyed into theirs when next found.
    pub fn owner_known(&self) -> bool {
        self.owner().person.is_some()
    }

    fn owner(&self) -> Owner {
        let f = self.owner.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        f.map(|f| f()).unwrap_or_default()
    }

    pub fn project(&self, id: &ProjectId) -> R<Option<Project>> {
        self.read(|c| one_project(c, id.as_str()))
    }

    /// The project `id` stands for now, following merges; None for one the
    /// store doesn't hold.
    pub fn resolve(&self, id: &ProjectId) -> R<Option<ProjectId>> {
        self.read(|c| if one_project(c, id.as_str())?.is_some() { resolved(c, id).map(Some) } else { Ok(None) })
    }

    pub fn checkout(&self, id: &CheckoutId) -> R<Option<Checkout>> {
        self.read(|c| one_checkout(c, id.as_str()))
    }

    /// This machine's checkouts, gone ones too, by path.
    pub fn checkouts(&self) -> R<Vec<Checkout>> {
        self.read(|c| Ok(c.prepare_cached("SELECT * FROM checkouts WHERE machine = ?1 ORDER BY path_key")?.query_map(params![LOCAL], checkout_of)?.collect::<Result<_, _>>()?))
    }

    /// Every project the store holds, by id.
    pub fn projects(&self) -> R<Vec<Project>> {
        self.read(|c| Ok(c.prepare_cached("SELECT * FROM projects ORDER BY id")?.query_map([], project_of)?.collect::<Result<_, _>>()?))
    }

    /// Where `path_key` is on this machine: the checkout, not gone, with
    /// the longest path key at or above it. A candidate is answered as one;
    /// what it gives rights in is the caller's to say (none).
    pub fn place(&self, path_key: &str) -> R<Option<Place>> {
        self.read(|c| {
            for k in ancestors(path_key) {
                if let Some(ch) = checkout_at(c, LOCAL, k)?.filter(|ch| ch.gone_at.is_none()) {
                    let rel = path_key[k.len()..].trim_start_matches('/').to_string();
                    return Ok(Some(Place { checkout: ch, rel }));
                }
            }
            Ok(None)
        })
    }

    /// The projects this machine has a confirmed checkout of, not gone,
    /// whose name or checkout folder is `name` (any case; a trailing `*`
    /// for any that starts so); with None, every one. What a scope's
    /// `project:` terms name, resolved when the person sets it: a
    /// candidate checkout names nothing.
    pub fn confirmed_projects(&self, name: Option<&str>) -> R<Vec<ProjectId>> {
        let want = name.map(str::to_lowercase);
        self.read(|c| {
            let mut q = c.prepare_cached(
                "SELECT DISTINCT p.id AS id, p.name AS name, k.path_key AS key FROM checkouts k JOIN projects p ON p.id = k.project WHERE k.machine = ?1 AND k.confirmed = 1 AND k.gone_at IS NULL ORDER BY p.id",
            )?;
            let rows: Vec<(String, String, String)> = q.query_map(params![LOCAL], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            let mut out: Vec<ProjectId> = Vec::new();
            for (id, pname, key) in rows {
                let folder = key.rsplit('/').next().unwrap_or("").to_string();
                let pname = pname.to_lowercase();
                let named = want.as_ref().is_none_or(|w| match w.strip_suffix('*') {
                    Some(pre) => pname.starts_with(pre) || folder.starts_with(pre),
                    None => pname == *w || folder == *w,
                });
                if let (true, Ok(id)) = (named, ProjectId::parse(&id)) {
                    if !out.contains(&id) {
                        out.push(id);
                    }
                }
            }
            Ok(out)
        })
    }

    /// The person confirms a candidate in the project it would join, from
    /// the window: the route that calls this is the window's alone. The
    /// checkout as it is then; an error for one the store doesn't hold.
    pub fn confirm(&self, id: &CheckoutId) -> R<Checkout> {
        self.write(|c| {
            let Some(ch) = one_checkout(c, id.as_str())? else {
                return Err(StoreError::new("not-found", format!("no checkout {id} here")));
            };
            let project = resolved(c, &ch.project)?;
            c.execute("UPDATE checkouts SET confirmed = 1, project = ?2 WHERE id = ?1", params![id.as_str(), project.as_str()])?;
            one_checkout(c, id.as_str())?.ok_or_else(|| StoreError::new("not-found", format!("no checkout {id} here")))
        })
    }

    /// One checkout found on this machine, made or brought up to date, and
    /// nothing else changed: what the engine needs before a claim when the
    /// map hasn't reconciled the folder yet.
    pub fn ensure(&self, found: &Found, now: i64) -> R<(Checkout, Vec<Fix>)> {
        let owner = self.owner();
        let ids = self.ids();
        self.write(|c| {
            let mut fixes = Vec::new();
            let mut by_key = std::collections::HashMap::new();
            let ch = upsert(c, &ids, &owner, found, &mut by_key, now, &mut fixes)?;
            Ok((ch, fixes))
        })
    }

    /// What the map found on this machine, diffed into the rows: new
    /// checkouts made, the rest brought up to date, and this machine's
    /// checkouts it didn't find marked gone. What changed, for the log.
    pub fn reconcile(&self, found: &[Found], now: i64) -> R<Vec<Fix>> {
        let owner = self.owner();
        let ids = self.ids();
        // Main checkouts before their worktrees and submodules, and
        // shallower before deeper, so the one that founds a project is the
        // repo's own and a parent is in place before what's in it.
        let mut order: Vec<&Found> = found.iter().collect();
        order.sort_by(|a, b| (a.kind, a.path_key.len(), &a.path_key).cmp(&(b.kind, b.path_key.len(), &b.path_key)));
        order.dedup_by(|a, b| a.path_key == b.path_key);
        self.write(|c| {
            let mut fixes = Vec::new();
            let mut by_key = std::collections::HashMap::new();
            for f in &order {
                upsert(c, &ids, &owner, f, &mut by_key, now, &mut fixes)?;
            }
            let mut q = c.prepare_cached("SELECT * FROM checkouts WHERE machine = ?1 AND gone_at IS NULL ORDER BY path_key")?;
            let gone: Vec<Checkout> =
                q.query_map(params![LOCAL], checkout_of)?.collect::<Result<Vec<_>, _>>()?.into_iter().filter(|ch| !by_key.contains_key(&ch.path_key)).collect();
            for ch in gone {
                c.execute("UPDATE checkouts SET gone_at = ?2 WHERE id = ?1", params![ch.id.as_str(), now])?;
                fixes.push(Fix::Gone { checkout: ch.id, path: ch.path.unwrap_or(ch.path_key) });
            }
            Ok(fixes)
        })
    }

    /// Merges project `from` into `into`: `from` stands for `into` from now
    /// on, its checkouts move, and each stays confirmed as it was. Both
    /// are the person's (owners of each, once two persons share).
    pub fn merge(&self, from: &ProjectId, into: &ProjectId) -> R<()> {
        self.write(|c| {
            let target = resolved(c, into)?;
            if resolved(c, from)? == target {
                return Ok(());
            }
            if one_project(c, from.as_str())?.is_none() || one_project(c, target.as_str())?.is_none() {
                return Err(StoreError::new("not-found", "a merge is of two projects the store holds"));
            }
            c.execute("UPDATE projects SET merged_into = ?2 WHERE id = ?1", params![from.as_str(), target.as_str()])?;
            c.execute("UPDATE checkouts SET project = ?2 WHERE project = ?1", params![from.as_str(), target.as_str()])?;
            Ok(())
        })
    }
}

// The project a found checkout belongs to, made when there's none: a
// repo's by its owner and remote, re-keying an ownerless one once the
// owner is known; a folder's own random one, or the one it had.
fn project_for(c: &Connection, ids: &Ids, owner: &Owner, f: &Found, had: Option<&Checkout>, now: i64, fixes: &mut Vec<Fix>) -> R<ProjectId> {
    let Some(key) = &f.repo_key else {
        if let Some(p) = had.and_then(|h| one_project(c, h.project.as_str()).ok().flatten()).filter(|p| p.kind == ProjectKind::Folder) {
            return resolved(c, &p.id);
        }
        let id = ids.project();
        c.execute("INSERT INTO projects (id, owner, repo_key, name, kind, created_at) VALUES (?1, ?2, NULL, ?3, 'folder', ?4)", params![id.as_str(), owner.person, f.name, now])?;
        return Ok(id);
    };
    let by = |who: Option<&str>| -> R<Option<Project>> {
        Ok(c.prepare_cached("SELECT * FROM projects WHERE owner IS ?1 AND repo_key = ?2")?.query_row(params![who, key], project_of).optional()?)
    };
    if let Some(p) = by(owner.person.as_deref())? {
        return resolved(c, &p.id);
    }
    let id = match &owner.salt {
        Some(salt) if owner.person.is_some() => ProjectId::of_repo(salt, key),
        _ => ids.project(),
    };
    c.execute("INSERT INTO projects (id, owner, repo_key, name, kind, created_at) VALUES (?1, ?2, ?3, ?4, 'repo', ?5)", params![id.as_str(), owner.person, key, f.name, now])?;
    // A project of this repo made before its owner was known is this one
    // now: once, by pointing at it, its checkouts with it.
    if owner.person.is_some() {
        if let Some(old) = by(None)?.filter(|p| p.merged_into.is_none()) {
            c.execute("UPDATE projects SET merged_into = ?2 WHERE id = ?1", params![old.id.as_str(), id.as_str()])?;
            c.execute("UPDATE checkouts SET project = ?2 WHERE project = ?1", params![old.id.as_str(), id.as_str()])?;
            fixes.push(Fix::Rekeyed { from: old.id, to: id.clone() });
        }
    }
    Ok(id)
}

fn upsert(c: &Connection, ids: &Ids, owner: &Owner, f: &Found, by_key: &mut std::collections::HashMap<String, CheckoutId>, now: i64, fixes: &mut Vec<Fix>) -> R<Checkout> {
    let had = checkout_at(c, LOCAL, &f.path_key)?;
    let project = project_for(c, ids, owner, f, had.as_ref(), now, fixes)?;
    let parent = match &f.parent {
        Some(k) => match by_key.get(k) {
            Some(id) => Some(id.clone()),
            None => checkout_at(c, LOCAL, k)?.map(|p| p.id),
        },
        None => None,
    };
    let parent = parent.as_ref().map(CheckoutId::as_str);
    let id = match had {
        None => {
            // The first checkout of a project founds it; any other waits
            // for the person.
            let founds = !has_checkouts(c, &project)?;
            let id = ids.checkout();
            c.execute(
                "INSERT INTO checkouts (id, project, machine, path, path_key, vcs, branch, head, parent, kind, confirmed, found_at, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![id.as_str(), project.as_str(), LOCAL, f.path, f.path_key, f.vcs, f.branch, f.head, parent, f.kind.as_str(), founds, now],
            )?;
            fixes.push(Fix::Found { checkout: id.clone(), path: f.path.clone(), project, candidate: !founds });
            id
        }
        Some(h) => {
            let was = resolved(c, &h.project)?;
            let rejoined = was != project;
            let confirmed = if rejoined {
                let founds = !has_checkouts(c, &project)?;
                // A folder's own project, once it has a remote, is the
                // repo's: its id points there.
                let alone = c
                    .prepare_cached("SELECT 1 FROM checkouts WHERE project = ?1 AND id != ?2 LIMIT 1")?
                    .query_row(params![was.as_str(), h.id.as_str()], |_| Ok(()))
                    .optional()?
                    .is_none();
                if alone && one_project(c, was.as_str())?.is_some_and(|p| p.kind == ProjectKind::Folder) {
                    c.execute("UPDATE projects SET merged_into = ?2 WHERE id = ?1", params![was.as_str(), project.as_str()])?;
                }
                fixes.push(Fix::Rejoined { checkout: h.id.clone(), path: f.path.clone(), from: was, to: project.clone(), candidate: !founds });
                founds
            } else {
                h.confirmed
            };
            if h.gone_at.is_some() {
                fixes.push(Fix::Back { checkout: h.id.clone(), path: f.path.clone() });
            } else if !rejoined && (h.branch != f.branch || h.head != f.head) {
                fixes.push(Fix::Moved { checkout: h.id.clone(), path: f.path.clone(), branch: f.branch.clone() });
            }
            c.execute(
                "UPDATE checkouts SET project = ?2, path = ?3, vcs = ?4, branch = ?5, head = ?6, parent = ?7, kind = ?8, confirmed = ?9, seen_at = ?10, gone_at = NULL WHERE id = ?1",
                params![h.id.as_str(), project.as_str(), f.path, f.vcs, f.branch, f.head, parent, f.kind.as_str(), confirmed, now],
            )?;
            h.id
        }
    };
    by_key.insert(f.path_key.clone(), id.clone());
    one_checkout(c, id.as_str())?.ok_or_else(|| StoreError::new("sqlite", "a checkout just written isn't there"))
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
