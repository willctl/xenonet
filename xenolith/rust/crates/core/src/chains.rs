//! A chain per space (xenonet-log's `xenolith-space/1`): who reads, appends, writes and curates
//! each space, and what it's about. The chains live in the state folder (`<state>/spaces/`), never
//! in the shared context folder, whose files anyone with the folder can write.
//!
//! - **Founding.** Once the upgrade from 1.x has run (upgrade.rs: `xenolith upgrade --run`, or a
//!   folder 2.0 made itself), the service founds a chain for each space this Xenolith curates and
//!   that has none yet: at its start, and when a space folder appears. It's founded under its name
//!   (`name:project:atlas`), owned by the standalone owner key and curated by this Xenolith's
//!   context key (keys.rs). A space this machine follows from another (1.x's sync.json `follow`)
//!   is never founded here, since its curator is elsewhere. `move_1x_copies` (part of the upgrade)
//!   moves its copy to `<state>/1x-copies/`, kept, once, and the space comes back by a share from
//!   its curator.
//! - **Changes.** Founding, marking and moving say each change to a `Record` before making it
//!   (`Change`), so the upgrade's manifest can take back exactly what was made.
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
pub use xenonet_log::space::Level;
use xenonet_log::space::{Op, Owner, Space, SpacePolicy, Subject, found};
pub use xenonet_model::ids::EntityRef;

/// The marker each space folder gets.
pub const MARKER: &str = "xenolith-space.json";
/// What a marker's signature covers, first: so it can't be taken for a chain entry, whose signed
/// bytes are JSON.
const MARKER_TAG: &str = "xenolith-space-marker/1";
/// How many characters of the spc_ a later space of the same name adds to its folder.
pub const SUFFIX_CHARS: usize = 6;

/// The file that says 1.x's copies were moved aside, in the chains' folder.
const MOVED_FLAG: &str = "1x-moved";

/// One change to the shared folder or the state folder, said before it's made, so the upgrade
/// (upgrade.rs) can take back exactly what was made and nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A folder that wasn't there.
    Dir { path: PathBuf },
    /// A file written, with what it held before: None when it wasn't there.
    File { path: PathBuf, before: Option<Vec<u8>> },
    /// A space folder moved aside, kept.
    Moved { address: SpaceId, from: PathBuf, to: PathBuf },
}

/// Where changes are said before they're made. An error stops the change.
pub type Record<'a> = &'a mut dyn FnMut(Change) -> Result<(), String>;

/// A Record that keeps nothing, for changes no upgrade takes back (a bind, a received space).
pub fn unrecorded(_: Change) -> Result<(), String> {
    Ok(())
}

/// Makes `path` and any folder above it that's missing, saying each first.
fn make_dir(path: &Path, rec: Record) -> Result<(), String> {
    let mut missing = vec![];
    let mut at = Some(path);
    while let Some(d) = at.filter(|d| !d.as_os_str().is_empty() && fs::symlink_metadata(d).is_err()) {
        missing.push(d.to_path_buf());
        at = d.parent();
    }
    for d in missing.into_iter().rev() {
        rec(Change::Dir { path: d.clone() })?;
        fs::create_dir(&d).or_else(|e| if d.is_dir() { Ok(()) } else { Err(e) }).map_err(|e| format!("cannot make {}: {e}", wiki::disp(&d)))?;
    }
    Ok(())
}

/// Writes `text` to `file` unless it holds exactly that, saying the change (with what was there)
/// first. Returns whether it wrote.
fn write(file: &Path, text: &str, rec: Record) -> Result<bool, String> {
    if let Some(p) = file.parent() {
        make_dir(p, rec)?;
    }
    let before = match fs::read(file) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", wiki::disp(file))),
    };
    if before.as_deref() == Some(text.as_bytes()) {
        return Ok(false);
    }
    rec(Change::File { path: file.to_path_buf(), before })?;
    wiki::atomic_write(file, text, None, None).map_err(|e| e.to_string())?;
    Ok(true)
}

/// The chains' folder.
pub fn dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("spaces")
}

/// Where a 1.x satellite's copies of spaces curated elsewhere go when the upgrade runs (upgrade.rs).
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

/// A 1.x copy moved aside by the upgrade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub address: SpaceId,
    pub to: PathBuf,
}

/// What the state folder says of the spaces: `None` before anything was founded.
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

    fn save(&self, paths: &AppPaths, rec: Record) -> Result<(), String> {
        let v = json!({
            "v": 1,
            "spaces": self.spaces.iter().map(|p| json!({ "id": p.id, "address": p.address.to_string() })).collect::<Vec<_>>(),
            "moved": self.moved.iter().map(|m| json!({ "address": m.address.to_string(), "to": m.to.to_string_lossy() })).collect::<Vec<_>>(),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default());
        write(&index_file(paths), &text, rec).map(|_| ())
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

fn save_space(paths: &AppPaths, space: &Space, rec: Record) -> Result<(), String> {
    let text = format!("{}\n", space.to_json());
    write(&chain_file(paths, space.id()), &text, rec).map(|_| ())
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
fn mark(root: &Path, address: &SpaceId, space: &Space, context: &dyn Signer, rec: Record) -> Result<bool, String> {
    let file = spaces::space_dir(root, address).join(MARKER);
    let want = marker(space, context)?;
    let have: Option<Value> = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok());
    if have.as_ref() == Some(&want) {
        return Ok(false);
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&want).unwrap_or_default());
    write(&file, &text, rec)
}

/// Whether a space folder's marker is a good one for the chain `id` (the dry run's guess at
/// whether a pass would write it: which key signed isn't checked here).
fn marked_for(root: &Path, address: &SpaceId, id: &str) -> bool {
    let file = spaces::space_dir(root, address).join(MARKER);
    fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| verify_marker(&v)).is_some_and(|(s, ..)| s == id)
}

/// What a space founded here under its folder's name is about.
fn about_of(address: &SpaceId) -> EntityRef {
    EntityRef::Name { kind: address.kind.clone(), name: address.base_name().to_string() }
}

/// 1.x's sync settings, in the config folder: read for the spaces it followed, never written
/// (2.0 deletes no 1.x file).
pub fn sync_1x_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("sync.json")
}

/// The spaces this machine followed under 1.x (sync.json's `follow`): their curator is another
/// machine's, so they're never founded or curated here.
pub fn followed(paths: &AppPaths) -> BTreeSet<String> {
    let v: Value = fs::read_to_string(sync_1x_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v["follow"].as_object().into_iter().flatten().filter_map(|(s, _)| SpaceId::parse(s).ok().filter(|s| !s.is_local())).map(|s| s.to_string()).collect()
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

/// Runs `f` holding the chains' lock. It's taken in the state folder itself, not the chains'
/// folder, so an undo can remove that folder while it holds the lock.
pub(crate) fn locked<T>(paths: &AppPaths, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let d = &paths.state_dir;
    fs::create_dir_all(d).map_err(|e| format!("cannot make {}: {e}", wiki::disp(d)))?;
    let mut guard = match crate::lock::acquire(d, "chains", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return Err(b.0),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.to_string()),
    };
    let out = f();
    guard.release();
    out
}

/// Moves a folder aside, keeping it: a rename, or a copy when the state folder is on another
/// volume (the original goes only once the copy is whole). `to`'s parent must be there.
pub(crate) fn move_aside(from: &Path, to: &Path) -> Result<(), String> {
    if wiki::retrying(|| fs::rename(from, to)).is_ok() {
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

/// One pass: founds a chain for each space folder this Xenolith curates that has none, and puts
/// every curated space's marker right, saying each change to `rec` first. `context` is this
/// Xenolith's context key (the curator), `owner` the standalone owner key. The caller holds
/// `locked` and has checked that the upgrade ran (upgrade.rs).
pub(crate) fn reconcile(paths: &AppPaths, root: &Path, context: &dyn Signer, owner: &dyn Signer, now: i64, rec: Record) -> Result<Report, String> {
    let mut index = Index::load(paths)?.unwrap_or_default();
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
                save_space(paths, &space, &mut *rec)?;
                index.spaces.push(Placed { id: space.id().to_string(), address: address.clone() });
                report.founded.push(address.clone());
                space
            }
        };
        if mark(root, &address, &space, context, &mut *rec)? {
            report.marked.push(address);
        }
    }
    index.save(paths, rec)?;
    Ok(report)
}

/// Moves a 1.x satellite's copies of the spaces it followed (sync.json) out of the folder into
/// `<state>/1x-copies/`, kept, once: the first time it runs, it records what it moved, and later
/// passes move nothing. Those spaces come back by a share from their curator. The caller holds
/// `locked`.
pub(crate) fn move_1x_copies(paths: &AppPaths, root: &Path, rec: Record) -> Result<Report, String> {
    let mut index = Index::load(paths)?.unwrap_or_default();
    let mut report = Report::default();
    let flag = dir(paths).join(MOVED_FLAG);
    if flag.exists() {
        return Ok(report);
    }
    for (address, to) in copies_to_move(paths, root) {
        let from = spaces::space_dir(root, &address);
        if let Some(p) = to.parent() {
            make_dir(p, &mut *rec)?;
        }
        rec(Change::Moved { address: address.clone(), from: from.clone(), to: to.clone() })?;
        move_aside(&from, &to)?;
        index.moved.push(Moved { address: address.clone(), to });
        report.moved.push(address);
    }
    index.save(paths, &mut *rec)?;
    write(&flag, "1\n", rec)?;
    Ok(report)
}

/// The 1.x copies the first move would take aside, and where each would go.
fn copies_to_move(paths: &AppPaths, root: &Path) -> Vec<(SpaceId, PathBuf)> {
    let followed = followed(paths);
    spaces::list(root)
        .into_iter()
        .filter(|s| !s.is_local() && followed.contains(&s.to_string()))
        .map(|a| {
            let to = free_dir(&copies_dir(paths).join(&a.kind), &a.name);
            (a, to)
        })
        .collect()
}

/// What the upgrade would do now, without doing it: the copies it would move aside (and where),
/// the spaces it would found, and the space folders whose marker it would write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub moved: Vec<(SpaceId, PathBuf)>,
    pub founded: Vec<SpaceId>,
    pub marked: Vec<SpaceId>,
}

/// The plan, read only: the choices `move_1x_copies` and `reconcile` would make.
pub fn plan(paths: &AppPaths, root: &Path) -> Result<Plan, String> {
    let index = Index::load(paths)?.unwrap_or_default();
    let followed = followed(paths);
    let mut plan = Plan::default();
    if !dir(paths).join(MOVED_FLAG).exists() {
        plan.moved = copies_to_move(paths, root);
    }
    for address in spaces::list(root).into_iter().filter(|a| !followed.contains(&a.to_string())) {
        match index.by_address(&address) {
            None => {
                plan.founded.push(address.clone());
                plan.marked.push(address);
            }
            Some(p) if !marked_for(root, &address, &p.id) => plan.marked.push(address),
            Some(_) => {}
        }
    }
    Ok(plan)
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
        save_space(paths, &space, &mut unrecorded)?;
        index.spaces.push(Placed { id: space.id().to_string(), address: address.clone() });
        index.save(paths, &mut unrecorded)?;
        mark(root, &address, &space, context, &mut unrecorded)?;
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
    let (id, space) = of_address(paths, address).ok_or_else(|| format!("{address} has no chain here yet: the service founds one once the upgrade has run (`xenolith upgrade`)"))?;
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
        save_space(paths, &space, &mut unrecorded)?;
        mark(root, address, &space, context, &mut unrecorded)?;
        Ok(describe(&space))
    })
}

// ---------------------------------------------------------------- sync (xenolith/sync/2)

/// A member of a space as own mode admits it: its context key's id, the endpoint id its share
/// names, and its level. Only a key subject with an endpoint is one: the others (persons,
/// networks, machines) are shared mode's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub key: String,
    pub endpoint: String,
    pub level: Level,
}

/// The members of a space by its chain as this Xenolith holds it, unshared ones left out.
pub fn members(space: &Space) -> Vec<Member> {
    space
        .view()
        .shares
        .values()
        .filter_map(|(subject, row)| match (subject, &row.endpoint) {
            (Subject::Key(k), Some(e)) => space.level(subject).map(|level| Member { key: k.clone(), endpoint: e.clone(), level }),
            _ => None,
        })
        .collect()
}

/// What a holder has of a space, as two holders compare it: the ids of every entry and side entry
/// it holds, in any order, hashed. Equal digests, nothing to send.
pub fn digest(space: &Space) -> String {
    let mut ids: Vec<String> = space.entries().iter().chain(space.held()).map(|e| e.hash()).chain(space.sides().map(xenonet_log::space::Side::id)).collect();
    ids.sort();
    ids.dedup();
    b64(&xenonet_id::codec::sha256(ids.join("\n").as_bytes()))
}

/// Every space this Xenolith holds a chain of, by id, with its folder's address.
pub fn held(paths: &AppPaths) -> Vec<(Placed, Space)> {
    let Ok(Some(index)) = Index::load(paths) else { return vec![] };
    index.spaces.into_iter().filter_map(|p| load_space(paths, &p.id).ok().map(|s| (p, s))).collect()
}

/// The spaces whose curator is elsewhere, by folder address: the ones 1.x followed, and the ones
/// whose chain names another context key than this Xenolith's as curator. Their folders are
/// copies; this machine's curator leaves them alone.
pub fn curated_elsewhere(paths: &AppPaths) -> BTreeSet<String> {
    let mine = crate::keys::context_id(paths);
    let mut out = followed(paths);
    for (p, s) in held(paths) {
        if mine.as_deref() != Some(s.view().curator.id().as_str()) {
            out.insert(p.address.to_string());
        }
    }
    out
}

/// Shares `address` with another Xenolith: its context key's id and the endpoint id own mode
/// admits it from, at `level`. Signed as the owner and the curator (this Xenolith), so only the
/// curator's machine can do it, with the owner key.
#[allow(clippy::too_many_arguments)]
pub fn share(paths: &AppPaths, root: &Path, address: &SpaceId, key: &str, endpoint: &str, level: Level, owner: &dyn Signer, context: &dyn Signer, now: i64) -> Result<Value, String> {
    let subject = Subject::parse(&format!("key:{key}")).ok_or_else(|| format!("{key} isn't a Xenolith's context key id (xns1_…)"))?;
    if !xenonet_log::space::is_endpoint_id(endpoint) {
        return Err(format!("{endpoint} isn't an endpoint id (64 hex digits)"));
    }
    locked(paths, || {
        let (_, mut space) = of_address(paths, address).ok_or_else(|| format!("{address} has no chain here yet"))?;
        if space.view().curator.id() != context.public().id() {
            return Err(format!("{address} is curated by another Xenolith ({}): share it there", space.view().curator.id()));
        }
        let after = space.unshares_of(&subject);
        let mut e = space.next(Op::Share { subject, level, endpoint: Some(endpoint.to_string()), after }, None, now);
        e.sign(&owner.public().id(), owner, None).map_err(|e| e.message)?;
        if owner.public().id() != context.public().id() {
            e.sign(&context.public().id(), context, None).map_err(|e| e.message)?;
        }
        space.offer(e).map_err(|e| e.message)?;
        save_space(paths, &space, &mut unrecorded)?;
        mark(root, address, &space, context, &mut unrecorded)?;
        let mut d = describe(&space);
        d["members"] = json!(members(&space).iter().map(|m| json!({ "key": m.key, "endpoint": m.endpoint, "level": m.level.as_str() })).collect::<Vec<_>>());
        Ok(d)
    })
}

/// Unshares `address` from another Xenolith's context key: a side entry, valid at any place in the
/// chain and in force from where each holder stores it (review revision 10b), so narrowing never
/// waits on anyone. Signed as the owner, on the curator's machine.
pub fn unshare(paths: &AppPaths, root: &Path, address: &SpaceId, key: &str, owner: &dyn Signer, context: &dyn Signer, now: i64) -> Result<Value, String> {
    let subject = Subject::parse(&format!("key:{key}")).ok_or_else(|| format!("{key} isn't a Xenolith's context key id (xns1_…)"))?;
    locked(paths, || {
        let (_, mut space) = of_address(paths, address).ok_or_else(|| format!("{address} has no chain here yet"))?;
        if space.view().curator.id() != context.public().id() {
            return Err(format!("{address} is curated by another Xenolith ({}): unshare it there", space.view().curator.id()));
        }
        // The owner's alone: a side entry has no curator's signature.
        let mut side = space.unshare(subject, None, now);
        side.sign(&owner.public().id(), owner, None).map_err(|e| e.message)?;
        space.offer_side(side).map_err(|e| e.message)?;
        save_space(paths, &space, &mut unrecorded)?;
        mark(root, address, &space, context, &mut unrecorded)?;
        Ok(describe(&space))
    })
}

/// Takes what another holder sent of a space this Xenolith holds (`chain`, as `Space::to_json`
/// writes it): each entry and side entry it doesn't hold, by the chain's own rules, any order.
/// One the rules refuse is left out; the rest stand. Returns whether anything was taken.
pub fn merge(paths: &AppPaths, id: &str, chain: &Value) -> Result<bool, String> {
    locked(paths, || {
        let mut space = load_space(paths, id)?;
        let before = digest(&space);
        let entries: Vec<xenonet_log::space::Entry> = chain["entries"].as_array().into_iter().flatten().filter_map(|e| xenonet_log::space::Entry::parse(e).ok()).collect();
        // Twice: an entry can follow one that comes after it in the list.
        for _ in 0..2 {
            for e in &entries {
                let _ = space.offer(e.clone());
            }
        }
        for s in chain["sides"].as_array().into_iter().flatten() {
            if let Ok(s) = xenonet_log::space::Side::parse(s) {
                let _ = space.offer_side(s);
            }
        }
        if digest(&space) == before {
            return Ok(false);
        }
        save_space(paths, &space, &mut unrecorded)?;
        Ok(true)
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
            locked(&self.paths, || reconcile(&self.paths, &self.root, &self.context, &self.owner, T, &mut unrecorded)).unwrap()
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
        fs::write(sync_1x_file(&h.paths), cfg.to_string()).unwrap();
        // Founding first, as a start before the copies moved would: nothing for the followed ones.
        let r = h.reconcile();
        assert_eq!(r.founded, vec![sp("machine:local"), sp("project:mine")], "only what this machine curates");
        assert!(!spaces::space_dir(&h.root, &sp("project:atlas")).join(MARKER).exists(), "no marker in a copy this machine doesn't curate");
        let moved = locked(&h.paths, || move_1x_copies(&h.paths, &h.root, &mut unrecorded)).unwrap();
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
        assert!(locked(&h.paths, || move_1x_copies(&h.paths, &h.root, &mut unrecorded)).unwrap().is_empty());
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
