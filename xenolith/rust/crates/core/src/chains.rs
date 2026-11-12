//! A chain per space (xenonet-log's `xenolith-space/1`): who reads, appends, writes and curates
//! each space, and what it's about. The chains live in the state folder (`<state>/spaces/`), never
//! in the shared context folder, whose files anyone with the folder can write.
//!
//! - **Founding.** The service founds a chain for each space this Xenolith curates and that has
//!   none yet: at its start, and when a space folder appears. It's founded under its name
//!   (`name:project:atlas`), owned by the standalone owner key and curated by this Xenolith's
//!   context key (keys.rs). A space this machine follows from another (1.x's sync.json `follow`)
//!   is never founded here, since its curator is elsewhere. `move_1x_copies` moves its copy to
//!   `<state>/1x-copies/`, kept, once, and the space comes back by a share from its curator.
//! - **Folders.** `index.json` maps each chain to its folder. The first space of a kind and name
//!   keeps `spaces/<kind>/<name>/`; a later one with that name, from another Xenolith, gets
//!   `<name>~<the first 6 characters of its spc_ after the prefix>` (FORMAT.md), so two spaces
//!   both made as project:x stay apart, shown side by side, never merged.
//! - **Markers.** Each space folder gets `xenolith-space.json`: {space, about, genesis hash,
//!   owner}, signed by the context key, so a reader of the folder (XenoNet's map) can tell a marker
//!   this Xenolith wrote from one an agent wrote. Xenolith itself never reads a marker to decide
//!   anything: what a folder is comes from the index, and a marker that differs is written again.
//! - **Binding.** `bind` ties a space made under a name to the entity it's about (a project's
//!   `prj_`), once. The chain allows it only from a person owner who owns that entity, so a space
//!   the standalone owner key owns binds only after it moves to the person (shared mode).

use crate::paths::AppPaths;
use crate::spaces::{self, SpaceId};
use crate::wiki;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use xenonet_id::codec::{b64, random, unb64};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};
use xenonet_log::space::{Op, Owner, Space, SpacePolicy, found};
pub use xenonet_model::ids::EntityRef;

/// The marker each space folder gets.
pub const MARKER: &str = "xenolith-space.json";
/// What a marker's signature covers, first: so it can't be taken for a chain entry, whose signed
/// bytes are JSON.
const MARKER_TAG: &str = "xenolith-space-marker/1";
/// How many characters of the spc_ a later space of the same name adds to its folder.
pub const SUFFIX_CHARS: usize = 6;

/// The chains' folder.
pub fn dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("spaces")
}

/// Where a 1.x satellite's copies of spaces curated elsewhere go at the first start.
pub fn copies_dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("1x-copies")
}

fn index_file(paths: &AppPaths) -> PathBuf {
    dir(paths).join("index.json")
}

fn chain_file(paths: &AppPaths, id: &str) -> PathBuf {
    dir(paths).join(format!("{id}.json"))
}

fn is_spc(id: &str) -> bool {
    xenonet_model::ids::SpaceId::parse(id).is_ok()
}

/// One chain and the folder it's materialised in, by the folder's address (`project:atlas`,
/// `project:atlas~abc234`, `machine:local`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: String,
    pub address: SpaceId,
}

/// A 1.x copy moved aside at the first start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub address: SpaceId,
    pub to: PathBuf,
}

/// What the state folder says of the spaces: `None` before the first start founded anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub spaces: Vec<Placed>,
    pub moved: Vec<Moved>,
}

impl Index {
    /// index.json; None when there's none yet. An index that's there and can't be read is an
    /// error, never an empty index, so nothing is founded twice.
    pub fn load(paths: &AppPaths) -> Result<Option<Index>, String> {
        let file = index_file(paths);
        let text = match fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", wiki::disp(&file))),
        };
        let v: Value = serde_json::from_str(&text).map_err(|_| format!("{} isn't JSON; it was left as it is", wiki::disp(&file)))?;
        let mut index = Index::default();
        for s in v["spaces"].as_array().into_iter().flatten() {
            let (Some(id), Some(address)) = (s["id"].as_str().filter(|i| is_spc(i)), s["address"].as_str().and_then(|a| SpaceId::parse(a).ok())) else {
                return Err(format!("{} holds an entry that isn't a space; it was left as it is", wiki::disp(&file)));
            };
            index.spaces.push(Placed { id: id.into(), address });
        }
        for m in v["moved"].as_array().into_iter().flatten() {
            if let (Some(address), Some(to)) = (m["address"].as_str().and_then(|a| SpaceId::parse(a).ok()), m["to"].as_str()) {
                index.moved.push(Moved { address, to: PathBuf::from(to) });
            }
        }
        Ok(Some(index))
    }

    fn save(&self, paths: &AppPaths) -> Result<(), String> {
        let v = json!({
            "v": 1,
            "spaces": self.spaces.iter().map(|p| json!({ "id": p.id, "address": p.address.to_string() })).collect::<Vec<_>>(),
            "moved": self.moved.iter().map(|m| json!({ "address": m.address.to_string(), "to": m.to.to_string_lossy() })).collect::<Vec<_>>(),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default());
        wiki::atomic_write(&index_file(paths), &text, None, None).map_err(|e| e.to_string())
    }

    pub fn by_address(&self, address: &SpaceId) -> Option<&Placed> {
        self.spaces.iter().find(|p| p.address == *address)
    }

    pub fn by_id(&self, id: &str) -> Option<&Placed> {
        self.spaces.iter().find(|p| p.id == id)
    }
}

/// A space's chain from its file.
pub fn load_space(paths: &AppPaths, id: &str) -> Result<Space, String> {
    let file = chain_file(paths, id);
    let text = fs::read_to_string(&file).map_err(|e| format!("cannot read {}: {e}", wiki::disp(&file)))?;
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("{} isn't JSON", wiki::disp(&file)))?;
    let space = Space::from_json(SpacePolicy::default(), &v).map_err(|e| format!("{}: {}", wiki::disp(&file), e.message))?;
    if space.id() != id {
        return Err(format!("{} holds another space, {}", wiki::disp(&file), space.id()));
    }
    Ok(space)
}

fn save_space(paths: &AppPaths, space: &Space) -> Result<(), String> {
    let text = format!("{}\n", space.to_json());
    wiki::atomic_write(&chain_file(paths, space.id()), &text, None, None).map_err(|e| e.to_string())
}

/// The space a folder's address names, as the state folder has it.
pub fn of_address(paths: &AppPaths, address: &SpaceId) -> Option<(String, Space)> {
    let index = Index::load(paths).ok()??;
    let id = index.by_address(address)?.id.clone();
    let space = load_space(paths, &id).ok()?;
    Some((id, space))
}

fn owner_text(owner: &Owner) -> String {
    match owner {
        Owner::Person(p) => format!("person:{p}"),
        Owner::Key(k) => format!("key:{}", k.id()),
    }
}

fn marker_message(space: &str, about: &str, genesis: &str, owner: &str, key: &str) -> Vec<u8> {
    format!("{MARKER_TAG}\n{space}\n{about}\n{genesis}\n{owner}\n{key}").into_bytes()
}

/// A space folder's marker, signed by `context`, this Xenolith's context key.
pub fn marker(space: &Space, context: &dyn Signer) -> Result<Value, String> {
    let v = space.view();
    let key = context.public();
    let (about, owner) = (v.about.to_string(), owner_text(&v.owner));
    let sig = context.sign(Purpose::SPACE, &marker_message(&v.id, &about, &v.genesis, &owner, &key.id())).map_err(|e| e.message)?;
    Ok(json!({ "v": 1, "space": v.id, "about": about, "genesis": v.genesis, "owner": owner, "key": key.to_json(), "sig": b64(&sig) }))
}

/// What a marker says, when its signature holds: (space, about, owner, the key that signed). Who
/// that key is, the reader decides (XenoNet: the key its grant for this Xenolith names). Anything
/// else in the file, or a marker that isn't signed by the key it names, is None.
pub fn verify_marker(v: &Value) -> Option<(String, String, String, VerifyingKey)> {
    let o = v.as_object()?;
    let allowed = ["v", "space", "about", "genesis", "owner", "key", "sig"];
    if o.keys().any(|k| !allowed.contains(&k.as_str())) || v["v"] != 1 {
        return None;
    }
    let text = |k: &str| v[k].as_str();
    let (space, about, genesis, owner) = (text("space")?, text("about")?, text("genesis")?, text("owner")?);
    let key = VerifyingKey::parse(&v["key"])?;
    let sig = unb64(text("sig")?)?;
    (is_spc(space) && key.verify(Purpose::SPACE, &marker_message(space, about, genesis, owner, &key.id()), &sig)).then(|| (space.into(), about.into(), owner.into(), key))
}

/// Writes `space`'s marker into its folder when the one there isn't exactly it.
fn mark(root: &Path, address: &SpaceId, space: &Space, context: &dyn Signer) -> Result<bool, String> {
    let file = spaces::space_dir(root, address).join(MARKER);
    let want = marker(space, context)?;
    let have: Option<Value> = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok());
    if have.as_ref() == Some(&want) {
        return Ok(false);
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&want).unwrap_or_default());
    wiki::atomic_write(&file, &text, None, None).map_err(|e| e.to_string())?;
    Ok(true)
}

/// What a space founded here under its folder's name is about.
fn about_of(address: &SpaceId) -> EntityRef {
    EntityRef::Name { kind: address.kind.clone(), name: address.base_name().to_string() }
}

/// The spaces this machine only follows (1.x sync.json): their curator is another machine's.
fn followed(paths: &AppPaths) -> BTreeSet<String> {
    let c = crate::sync::Config::load(paths);
    c.follow.keys().chain(c.curators.keys()).cloned().collect()
}

/// What one pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Spaces founded, by address.
    pub founded: Vec<SpaceId>,
    /// Markers written (new or put right).
    pub marked: Vec<SpaceId>,
    /// 1.x copies moved aside.
    pub moved: Vec<SpaceId>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.founded.is_empty() && self.marked.is_empty() && self.moved.is_empty()
    }

    pub fn to_json(&self) -> Value {
        let s = |v: &[SpaceId]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
        json!({ "founded": s(&self.founded), "marked": s(&self.marked), "moved": s(&self.moved) })
    }
}

fn locked<T>(paths: &AppPaths, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let d = dir(paths);
    fs::create_dir_all(&d).map_err(|e| format!("cannot make {}: {e}", wiki::disp(&d)))?;
    let mut guard = match crate::lock::acquire(&d, "chains", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return Err(b.0),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.to_string()),
    };
    let out = f();
    guard.release();
    out
}

/// Moves a folder aside, keeping it: a rename, or a copy when the state folder is on another
/// volume (the original goes only once the copy is whole).
fn move_aside(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p).map_err(|e| format!("cannot make {}: {e}", wiki::disp(p)))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                copy(&e.path(), &to.join(e.file_name()))?;
            } else if t.is_file() {
                fs::copy(e.path(), to.join(e.file_name()))?;
            }
        }
        Ok(())
    }
    copy(from, to).map_err(|e| format!("cannot copy {} to {}: {e}", wiki::disp(from), wiki::disp(to)))?;
    fs::remove_dir_all(from).map_err(|e| format!("copied {} to {}, but cannot remove it: {e}", wiki::disp(from), wiki::disp(to)))
}

/// A folder under `base` that isn't there yet: `name`, else `name-2`, `name-3`…
fn free_dir(base: &Path, name: &str) -> PathBuf {
    let mut p = base.join(name);
    let mut n = 2;
    while fs::symlink_metadata(&p).is_ok() {
        p = base.join(format!("{name}-{n}"));
        n += 1;
    }
    p
}

/// One pass: at the first start, moves 1.x copies of followed spaces aside; then founds a chain for
/// each space folder this Xenolith curates that has none, and puts every curated space's marker
/// right. `context` is this Xenolith's context key (the curator), `owner` the standalone owner key.
pub fn reconcile(paths: &AppPaths, root: &Path, context: &dyn Signer, owner: &dyn Signer, now: i64) -> Result<Report, String> {
    locked(paths, || {
        let loaded = Index::load(paths)?;
        let first = loaded.is_none();
        let mut index = loaded.unwrap_or_default();
        let followed = followed(paths);
        let mut report = Report::default();
        for address in spaces::list(root) {
            if followed.contains(&address.to_string()) {
                continue;
            }
            let space = match index.by_address(&address) {
                Some(p) => load_space(paths, &p.id)?,
                None => {
                    let g = found(about_of(&address), Owner::Key(owner.public()), context, Some(owner), address.base_name(), random::<16>().map_err(|e| e.message)?, now).map_err(|e| e.message)?;
                    let space = Space::found(SpacePolicy::default(), g).map_err(|e| e.message)?;
                    save_space(paths, &space)?;
                    index.spaces.push(Placed { id: space.id().to_string(), address: address.clone() });
                    report.founded.push(address.clone());
                    space
                }
            };
            if mark(root, &address, &space, context)? {
                report.marked.push(address);
            }
        }
        if first || !report.founded.is_empty() {
            index.save(paths)?;
        }
        Ok(report)
    })
}

/// Moves a 1.x satellite's copies of the spaces it followed (sync.json) out of the folder into
/// `<state>/1x-copies/`, kept, once: the first time it runs, it records what it moved, and later
/// passes move nothing. Those spaces come back by a share from their curator.
pub fn move_1x_copies(paths: &AppPaths, root: &Path) -> Result<Report, String> {
    locked(paths, || {
        let mut index = Index::load(paths)?.unwrap_or_default();
        let mut report = Report::default();
        let file = dir(paths).join("1x-moved");
        if file.exists() {
            return Ok(report);
        }
        let followed = followed(paths);
        for address in spaces::list(root).into_iter().filter(|s| !s.is_local() && followed.contains(&s.to_string())) {
            let from = spaces::space_dir(root, &address);
            let to = free_dir(&copies_dir(paths).join(&address.kind), &address.name);
            move_aside(&from, &to)?;
            index.moved.push(Moved { address: address.clone(), to });
            report.moved.push(address);
        }
        index.save(paths)?;
        wiki::atomic_write(&file, "1\n", None, None).map_err(|e| e.to_string())?;
        Ok(report)
    })
}

/// The kind of folder a space about `about` lives in.
fn kind_of(about: &EntityRef) -> &str {
    match about {
        EntityRef::Name { kind, .. } => kind,
        EntityRef::Project(_) => "project",
        EntityRef::Machine(_) => "machine",
        EntityRef::Actor { .. } => "actor",
        EntityRef::Person(_) | EntityRef::Identity(_) => "identity",
    }
}

/// The folder for a space named `name` of `kind` with id `id`: `<kind>/<name>` while no other
/// space has it, else `<kind>/<name>~<first 6 of the id>`.
pub fn address_for(root: &Path, index: &Index, kind: &str, name: &str, id: &str) -> Result<SpaceId, String> {
    let plain = SpaceId::parse(&format!("{kind}:{name}")).map_err(|e| e.to_string())?;
    if !plain.is_local() && index.by_address(&plain).is_none() && fs::symlink_metadata(spaces::space_dir(root, &plain)).is_err() {
        return Ok(plain);
    }
    let suffix = id.strip_prefix("spc_").and_then(|s| s.get(..SUFFIX_CHARS)).ok_or_else(|| format!("{id} isn't a space's id"))?;
    SpaceId::parse(&format!("{kind}:{name}~{suffix}")).map_err(|e| e.to_string())
}

/// Takes a space's chain from another Xenolith (its whole JSON, as `Space::to_json` writes it,
/// checked from its founding) and gives it a folder here, beside any space of the same name.
/// Returns its address; a space held already keeps the one it has.
pub fn receive(paths: &AppPaths, root: &Path, chain: &Value, context: &dyn Signer) -> Result<SpaceId, String> {
    let space = Space::from_json(SpacePolicy::default(), chain).map_err(|e| e.message)?;
    locked(paths, || {
        let mut index = Index::load(paths)?.unwrap_or_default();
        if let Some(p) = index.by_id(space.id()) {
            return Ok(p.address.clone());
        }
        let v = space.view();
        let address = address_for(root, &index, kind_of(&v.about), &v.name, space.id())?;
        spaces::create(root, &address).map_err(|e| e.to_string())?;
        save_space(paths, &space)?;
        index.spaces.push(Placed { id: space.id().to_string(), address: address.clone() });
        index.save(paths)?;
        mark(root, &address, &space, context)?;
        Ok(address)
    })
}

/// What `xenolith spaces` shows of a space's chain: its id, what it's about, its owner and
/// curator, and whether the owner is the weaker file key. Never a key's secret.
pub fn describe(space: &Space) -> Value {
    let v = space.view();
    json!({
        "id": v.id,
        "about": v.about.to_string(),
        "owner": owner_text(&v.owner),
        "weaker": v.weaker(),
        "curator": v.curator.id(),
        "epoch": v.epoch,
        "entries": space.len(),
    })
}

/// What binding `address` to `to` would do, without doing it: the space, what it's about now,
/// what it would be about, and why it can't be bound when it can't.
pub fn bind_plan(paths: &AppPaths, address: &SpaceId, to: Option<&EntityRef>, repo: Option<&str>) -> Result<Value, String> {
    let (id, space) = of_address(paths, address).ok_or_else(|| format!("{address} has no chain here yet: the service founds one when it starts"))?;
    let v = space.view();
    let remote = repo.map(xenonet_model::ids::normalize_remote).filter(|r| !r.is_empty());
    let why = if v.about.is_bound() {
        Some(format!("{address} is about {} already; a space binds once", v.about))
    } else if to.is_none() {
        Some("a project's id is its owner's (the person's salt, in their person chain), which only shared mode brings: name it with --project prj_… as XenoNet shows it".to_string())
    } else if to.is_some_and(|t| t.kind() == "project") && address.kind != "project" {
        Some(format!("{address} isn't a project's space"))
    } else if v.weaker() {
        Some("the standalone owner key owns this space, so it binds only once it moves to the person (shared mode)".to_string())
    } else {
        None
    };
    Ok(json!({
        "space": address.to_string(),
        "id": id,
        "about": v.about.to_string(),
        "to": to.map(ToString::to_string),
        "repo": remote,
        "canRun": why.is_none(),
        "why": why,
    }))
}

/// Binds `address` to `to`: the owner's signature and the curator's on a `bind` entry, which the
/// chain takes only from a person owner who owns `to`. Returns the space's description.
pub fn bind(paths: &AppPaths, root: &Path, address: &SpaceId, to: EntityRef, owner: &dyn Signer, context: &dyn Signer, now: i64) -> Result<Value, String> {
    locked(paths, || {
        let (_, mut space) = of_address(paths, address).ok_or_else(|| format!("{address} has no chain here yet"))?;
        let mut e = space.next(Op::Bind { about: to }, None, now);
        e.sign(&owner.public().id(), owner, None).map_err(|e| e.message)?;
        if owner.public().id() != context.public().id() {
            e.sign(&context.public().id(), context, None).map_err(|e| e.message)?;
        }
        space.offer(e).map_err(|e| e.message)?;
        save_space(paths, &space)?;
        mark(root, address, &space, context)?;
        Ok(describe(&space))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::random_hex;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    const T: i64 = 1_792_512_000_000;

    struct Home {
        base: PathBuf,
        paths: AppPaths,
        root: PathBuf,
        context: SeedSigner,
        owner: SeedSigner,
    }

    impl Drop for Home {
        fn drop(&mut self) {
            crate::lock::remove_test_dir(&self.base);
        }
    }

    fn home(n: u8) -> Home {
        let base = std::env::temp_dir().join(format!("xenolith-chains-{}", random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), base.join("home").to_string_lossy().to_string())].into_iter().collect();
        let root = base.join("wiki");
        wiki::ensure_wiki(&root).unwrap();
        let key = |m: u8| SeedSigner::from_seed(&[m; 32], KeyHome::File).unwrap();
        Home { paths: AppPaths::from_env(&env), root, base, context: key(n), owner: key(n + 100) }
    }

    impl Home {
        fn reconcile(&self) -> Report {
            reconcile(&self.paths, &self.root, &self.context, &self.owner, T).unwrap()
        }
        fn id(&self, a: &str) -> String {
            of_address(&self.paths, &SpaceId::parse(a).unwrap()).unwrap().0
        }
        fn marker(&self, a: &str) -> Value {
            let file = spaces::space_dir(&self.root, &SpaceId::parse(a).unwrap()).join(MARKER);
            serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
        }
    }

    fn sp(s: &str) -> SpaceId {
        SpaceId::parse(s).unwrap()
    }

    #[test]
    fn each_curated_space_gets_one_chain_at_the_first_start_and_keeps_it() {
        let h = home(1);
        spaces::create(&h.root, &sp("project:atlas")).unwrap();
        spaces::create(&h.root, &sp("actor:hermes")).unwrap();
        let r = h.reconcile();
        let all: Vec<String> = r.founded.iter().map(ToString::to_string).collect();
        assert_eq!(all, ["machine:local", "actor:hermes", "project:atlas"]);
        assert_eq!(r.marked.len(), 3);
        let (atlas, space) = of_address(&h.paths, &sp("project:atlas")).unwrap();
        let v = space.view();
        assert_eq!(v.about.to_string(), "name:project:atlas", "founded under its name");
        assert_eq!(v.owner, Owner::Key(h.owner.public()));
        assert_eq!(v.curator.id(), h.context.public().id());
        assert!(v.weaker());
        let (s, about, owner, key) = verify_marker(&h.marker("project:atlas")).unwrap();
        assert_eq!((s, about.as_str(), owner, key.id()), (atlas.clone(), "name:project:atlas", format!("key:{}", h.owner.public().id()), h.context.public().id()));
        // Again, and again after a restart: nothing new.
        assert!(h.reconcile().is_empty());
        assert_eq!(h.id("project:atlas"), atlas);
        // A space that appears later is founded then.
        spaces::create(&h.root, &sp("project:zephyr")).unwrap();
        assert_eq!(h.reconcile().founded, vec![sp("project:zephyr")]);
        assert_ne!(h.id("project:zephyr"), atlas);
    }

    #[test]
    fn a_hand_edited_marker_changes_nothing_xenolith_decides() {
        let h = home(2);
        spaces::create(&h.root, &sp("project:atlas")).unwrap();
        spaces::create(&h.root, &sp("project:other")).unwrap();
        h.reconcile();
        let (atlas, other) = (h.id("project:atlas"), h.id("project:other"));
        let file = spaces::space_dir(&h.root, &sp("project:atlas")).join(MARKER);
        // Another space's id and a person owner, by hand: unsigned, then with the old signature.
        let mut edited = h.marker("project:atlas");
        edited["space"] = json!(other);
        edited["owner"] = json!("person:prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(verify_marker(&edited).is_none(), "the signature no longer holds");
        fs::write(&file, edited.to_string()).unwrap();
        // The same marker in a folder an agent made, to pass it off as atlas.
        let fake = spaces::create(&h.root, &sp("project:fake")).unwrap();
        fs::write(fake.join(MARKER), h.marker("project:other").to_string()).unwrap();
        let r = h.reconcile();
        assert_eq!(h.id("project:atlas"), atlas, "the folder is still atlas's, by the index");
        assert_eq!(r.founded, vec![sp("project:fake")], "the agent's folder is a new space of its own");
        assert_ne!(h.id("project:fake"), other);
        assert!(r.marked.contains(&sp("project:atlas")) && r.marked.contains(&sp("project:fake")));
        assert_eq!(verify_marker(&h.marker("project:atlas")).unwrap().0, atlas, "written again from the chain");
        assert_eq!(verify_marker(&h.marker("project:fake")).unwrap().0, h.id("project:fake"));
    }

    #[test]
    fn a_1x_satellite_founds_nothing_for_the_spaces_it_follows_and_keeps_their_copies() {
        let h = home(3);
        for s in ["project:atlas", "project:shared", "project:mine"] {
            let d = spaces::create(&h.root, &sp(s)).unwrap();
            fs::write(d.join("pages").join("p.md"), format!("# {s}\n")).unwrap();
        }
        // sync.json as 1.x writes it on a satellite: home, one space followed by hand, one by home.
        fs::create_dir_all(&h.paths.config_dir).unwrap();
        let cfg = json!({
            "machine": "neo", "home": "inix", "peers": { "inix": "https://inix.example" },
            "follow": { "project:atlas": { "peer": "inix", "curator": "0123456789abcdef0123456789abcdef" }, "project:shared": { "peer": "inix", "auto": true } },
        });
        fs::write(crate::sync::config_file(&h.paths), cfg.to_string()).unwrap();
        // Founding first, as a start before the copies moved would: nothing for the followed ones.
        let r = h.reconcile();
        assert_eq!(r.founded, vec![sp("machine:local"), sp("project:mine")], "only what this machine curates");
        assert!(!spaces::space_dir(&h.root, &sp("project:atlas")).join(MARKER).exists(), "no marker in a copy this machine doesn't curate");
        let moved = move_1x_copies(&h.paths, &h.root).unwrap();
        assert_eq!(moved.moved, vec![sp("project:atlas"), sp("project:shared")]);
        assert_eq!(Index::load(&h.paths).unwrap().unwrap().moved.len(), 2, "and the index says where they went");
        for s in ["atlas", "shared"] {
            assert!(!spaces::exists(&h.root, &sp(&format!("project:{s}"))), "{s} moved out of the folder");
            let kept = copies_dir(&h.paths).join("project").join(s).join("pages").join("p.md");
            assert_eq!(fs::read_to_string(kept).unwrap(), format!("# project:{s}\n"), "and kept");
        }
        assert!(of_address(&h.paths, &sp("project:atlas")).is_none());
        // Its copy made again (by 1.x, say): still nothing founded, and moved only once.
        spaces::create(&h.root, &sp("project:atlas")).unwrap();
        assert!(h.reconcile().is_empty());
        assert!(move_1x_copies(&h.paths, &h.root).unwrap().is_empty());
        assert!(spaces::exists(&h.root, &sp("project:atlas")) && of_address(&h.paths, &sp("project:atlas")).is_none());
        assert!(!spaces::space_dir(&h.root, &sp("project:atlas")).join(MARKER).exists(), "no marker in a copy this machine doesn't curate");
    }

    #[test]
    fn two_xenoliths_that_both_make_project_x_keep_both_apart() {
        let (a, b) = (home(4), home(5));
        for h in [&a, &b] {
            spaces::create(&h.root, &sp("project:x")).unwrap();
            h.reconcile();
        }
        let (xa, xb) = (a.id("project:x"), b.id("project:x"));
        assert_ne!(xa, xb, "two spaces, two ids");
        // B gets A's chain (sync does this later): beside its own, never over it.
        let chain = load_space(&a.paths, &xa).unwrap().to_json();
        let at = receive(&b.paths, &b.root, &chain, &b.context).unwrap();
        assert_eq!(at.to_string(), format!("project:x~{}", &xa[4..10]));
        assert_eq!(b.id("project:x"), xb, "B's own keeps the plain name");
        assert_eq!(b.id(&at.to_string()), xa);
        assert_eq!(spaces::list(&b.root).iter().map(ToString::to_string).collect::<Vec<_>>(), ["machine:local".to_string(), "project:x".into(), at.to_string()]);
        assert_eq!(verify_marker(&b.marker(&at.to_string())).unwrap().0, xa);
        // Received again: the same folder. And a reconcile founds nothing for it.
        assert_eq!(receive(&b.paths, &b.root, &chain, &b.context).unwrap(), at);
        assert!(b.reconcile().founded.is_empty());
        // A space of a name nobody here has takes the plain folder.
        spaces::create(&a.root, &sp("project:solo")).unwrap();
        a.reconcile();
        let solo = load_space(&a.paths, &a.id("project:solo")).unwrap().to_json();
        assert_eq!(receive(&b.paths, &b.root, &solo, &b.context).unwrap(), sp("project:solo"));
    }

    #[test]
    fn a_space_the_standalone_key_owns_doesnt_bind_and_the_dry_run_says_why() {
        let h = home(6);
        spaces::create(&h.root, &sp("project:atlas")).unwrap();
        h.reconcile();
        let prj = EntityRef::parse(&format!("prj_{}", "a".repeat(32))).unwrap();
        let plan = bind_plan(&h.paths, &sp("project:atlas"), Some(&prj), Some("git@github.com:willctl/atlas.git")).unwrap();
        assert_eq!(plan["canRun"], false);
        assert_eq!(plan["repo"], "github.com/willctl/atlas");
        assert!(plan["why"].as_str().unwrap().contains("moves to the person"), "{plan}");
        let no_id = bind_plan(&h.paths, &sp("project:atlas"), None, Some("https://github.com/willctl/atlas")).unwrap();
        assert!(no_id["why"].as_str().unwrap().contains("--project"), "{no_id}");
        let before = load_space(&h.paths, &h.id("project:atlas")).unwrap().len();
        let e = bind(&h.paths, &h.root, &sp("project:atlas"), prj, &h.owner, &h.context, T).unwrap_err();
        assert!(e.contains("move-owner"), "{e}");
        assert_eq!(load_space(&h.paths, &h.id("project:atlas")).unwrap().len(), before, "the chain is as it was");
        assert!(bind_plan(&h.paths, &sp("project:nope"), None, None).is_err());
    }
}
