//! Sync between Xenoliths (`xenolith/sync/2`): what this Xenolith keeps of it in its state folder,
//! and what a space's feed entries do to the folder. The wire, the endpoint and the threads are
//! xenolith-net's; everything here is files, so the hook and the tray could read it without tokio.
//!
//! - **Who's who.** A space's members are the subjects its chain shares it with (chains.rs), each a
//!   context key with the endpoint id own mode admits it from (xenonet-log's `Share{key,
//!   endpoint}`). A Xenolith that follows a space it holds no chain of yet knows its curator from a
//!   follow record (`Follow`), which joining (`xenolith join`, xenolith-net's join/1) writes.
//! - **Feeds.** Each Xenolith writes one feed per space (`Outbox`, append-only, `<state>/sync/out/`):
//!   a member's notes (`ctx.note`) and its edits of the pages it holds (`ctx.propose`), sent to the
//!   curator alone (review revision 12); the curator's pages, removals and log days (`ctx.page`,
//!   `ctx.remove`, `ctx.log`), sent to every member. What it took of other feeds is `Feeds`, a
//!   xenonet-log `FeedStore` per space, so each entry is applied once, in its author's order.
//! - **Holders.** A member keeps the curator's entries it took (`Relay`, `<state>/sync/held/`) and
//!   where the other members were said to be (`Addrs`), so with the curator away one member fills
//!   another (review revision 14; xenolith-net's engine decides who is served what).
//! - **Texts apart.** An entry names its text by SHA-256 (`text: {sha256, bytes}`) and the text
//!   travels beside it; texts are kept in `<state>/sync/blobs/` by that hash, so `forget` can drop
//!   them and the entries still serve (revision 13).
//! - **On the curator,** a note or an edit becomes what 1.x's sync made of it: a note in the
//!   space's inbox, for its curator to file; a replace or removal against the page as it is now, a
//!   note the curator applies (a removal, applied at once); against an older version, a held
//!   change of kind "sync" for the person's OK (`take`).
//! - **On a member,** the curator's pages are written into the copy, what they replace kept in
//!   `.history/`, and the copy's own edits go back as proposals (`changes_in_copy`), each once.

use crate::curator::{HeldOp, PageWrite};
use crate::inbox::{self, By, NoteInput, PageInput, curator_paths, is_base, is_note_id};
use crate::paths::AppPaths;
use crate::text::{hash_text, history_stamp, local_iso, now, now_ms, one_line, random_hex};
use crate::wiki::{self, DATE_RE, SLUG_RE, wiki_err};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use xenonet_id::codec::{b64, sha256, unb64_n};
use xenonet_log::feed::{FeedEntry, FeedStore, Fork, Head, Header, Kind, Link, Open, Pos, Writer};
use xenonet_log::space::Level;

/// The ALPN, as the endpoint offers it.
pub const ALPN: &[u8] = b"xenolith/sync/2";
/// How many days of the curator's log a member keeps a copy of.
pub const LOG_DAYS: i64 = 7;
/// The largest text an entry carries beside it: an envelope (1 MiB of JSON, the text in base64)
/// with room for its head and entry. A page past it stays on its machine, and the log says so.
pub const MAX_TEXT: usize = 700 * 1024;

pub fn dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("sync")
}

fn read_json(file: &Path) -> Option<Value> {
    fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Written whole (a temp file renamed) and readable by the owner alone: what sync keeps names who
/// the members are and where they're reached.
fn write_json(file: &Path, v: &Value) -> Result<(), String> {
    if let Some(p) = file.parent() {
        fs::create_dir_all(p).map_err(|e| format!("cannot make {}: {e}", wiki::disp(p)))?;
    }
    wiki::write_private(file, &format!("{v}\n"), &[]).map_err(|e| format!("cannot write {}: {e}", wiki::disp(file)))
}

fn is_spc(id: &str) -> bool {
    xenonet_model::ids::SpaceId::parse(id).is_ok()
}

/// Whether `e` is an endpoint id as a share names one: 64 lowercase hex digits.
pub fn is_endpoint(e: &str) -> bool {
    xenonet_log::space::is_endpoint_id(e)
}

// ---------------------------------------------------------------- settings

/// How this Xenolith's own endpoint is set up: config.json's `sync`. With no relays and no mDNS
/// (the defaults), it's reached only at the addresses its peers were given: no relay of anyone's,
/// and no id announced on the LAN (review revision 19).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `ip:port` to bind; None: every IPv4 interface on DEFAULT_PORT.
    pub bind: Option<String>,
    /// The relays the person named; none by default, and never n0's unless named.
    pub relays: Vec<String>,
    /// Whether to find members on the LAN by mDNS.
    pub mdns: bool,
    /// `sync.endpoint: "off"` keeps the endpoint closed even with peers.
    pub off: bool,
    /// Shared mode's socket (`sync.xenonet.socket`): XenoNet's forward socket; None: the one in
    /// XenoNet's runtime folder for its default port (xenolith-net finds it).
    pub xenonet_socket: Option<String>,
    /// `sync.xenonet: "off"`: no shared mode, whatever XenoNet runs here.
    pub xenonet_off: bool,
}

/// The UDP port the endpoint binds when `sync.bind` names none.
pub const DEFAULT_PORT: u16 = 47851;

impl Settings {
    pub fn from_config(config: &Value) -> Settings {
        let s = &config["sync"];
        Settings {
            bind: s["bind"].as_str().filter(|b| b.parse::<std::net::SocketAddr>().is_ok()).map(String::from),
            relays: s["relays"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|u| u.starts_with("https://")).map(String::from).collect(),
            mdns: s["mdns"].as_bool().unwrap_or(false),
            off: s["endpoint"].as_str() == Some("off"),
            xenonet_socket: s["xenonet"]["socket"].as_str().filter(|p| !p.is_empty()).map(String::from),
            xenonet_off: s["xenonet"].as_str() == Some("off"),
        }
    }
}

// ---------------------------------------------------------------- follow records

/// A space this Xenolith follows, as a join named it: its
/// id, and its curator's context key and endpoint, so the curator is admitted before the chain is
/// here. The chain, once it comes, says who curates from then on; a follow only lets it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Follow {
    pub space: String,
    pub curator: String,
    pub endpoint: String,
    /// Where the curator's endpoint is reached (`ip:port`).
    pub addrs: Vec<String>,
    /// Follow the other spaces this curator shares with this Xenolith too.
    pub all: bool,
}

impl Follow {
    pub fn to_json(&self) -> Value {
        json!({ "space": self.space, "curator": self.curator, "endpoint": self.endpoint, "addrs": self.addrs, "all": self.all })
    }

    pub fn parse(v: &Value) -> Option<Follow> {
        let f = Follow {
            space: v["space"].as_str().filter(|s| is_spc(s))?.into(),
            curator: v["curator"].as_str().filter(|k| xenonet_model::ids::is_signing_key_id(k))?.into(),
            endpoint: v["endpoint"].as_str().filter(|e| is_endpoint(e))?.into(),
            addrs: v["addrs"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|a| a.parse::<std::net::SocketAddr>().is_ok()).map(String::from).collect(),
            all: v["all"].as_bool().unwrap_or(false),
        };
        Some(f)
    }
}

fn follow_file(paths: &AppPaths) -> PathBuf {
    dir(paths).join("follow.json")
}

pub fn follows(paths: &AppPaths) -> Vec<Follow> {
    read_json(&follow_file(paths)).and_then(|v| v["follows"].as_array().cloned()).unwrap_or_default().iter().filter_map(Follow::parse).collect()
}

/// Adds a follow record, in place of one for the same space.
pub fn follow(paths: &AppPaths, f: Follow) -> Result<(), String> {
    let mut all = follows(paths);
    all.retain(|x| x.space != f.space);
    all.push(f);
    write_json(&follow_file(paths), &json!({ "v": 1, "follows": all.iter().map(Follow::to_json).collect::<Vec<_>>() }))
}

// ---------------------------------------------------------------- texts

/// A text's id: the base64url SHA-256 of its bytes, as an entry's `text.sha256` names it.
pub fn text_id(bytes: &[u8]) -> String {
    b64(&sha256(bytes))
}

/// What an entry's body says of its text.
pub fn text_ref(bytes: &[u8]) -> Value {
    json!({ "sha256": text_id(bytes), "bytes": bytes.len() })
}

/// The text id an entry's body names, if any.
pub fn named_text(body: &Value) -> Option<String> {
    body["text"]["sha256"].as_str().filter(|h| unb64_n::<32>(h).is_some()).map(String::from)
}

fn blob_file(paths: &AppPaths, id: &str) -> Option<PathBuf> {
    unb64_n::<32>(id)?;
    Some(dir(paths).join("blobs").join(id))
}

/// Keeps a text by its id; one already kept stays as it is.
pub fn put_text(paths: &AppPaths, bytes: &[u8]) -> Result<String, String> {
    let id = text_id(bytes);
    let file = blob_file(paths, &id).ok_or("a text's id")?;
    if file.exists() {
        return Ok(id);
    }
    if let Some(p) = file.parent() {
        fs::create_dir_all(p).map_err(|e| format!("cannot make {}: {e}", wiki::disp(p)))?;
    }
    let tmp = file.with_extension(format!("{}.tmp", random_hex(6)));
    fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&tmp)))?;
    fs::rename(&tmp, &file).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&file)))?;
    Ok(id)
}

pub fn text(paths: &AppPaths, id: &str) -> Option<Vec<u8>> {
    fs::read(blob_file(paths, id)?).ok()
}

/// Drops texts by id (a forget): the entries that name them still serve, without them.
pub fn drop_texts(paths: &AppPaths, ids: &[String]) -> usize {
    ids.iter().filter_map(|id| blob_file(paths, id)).filter(|f| fs::remove_file(f).is_ok()).count()
}

/// The texts that hold `needle` (a forget's text), by id.
pub fn texts_with(paths: &AppPaths, needle: &str, ignore_case: bool) -> Vec<String> {
    if needle.is_empty() {
        return vec![];
    }
    let low = needle.to_lowercase();
    let Ok(rd) = fs::read_dir(dir(paths).join("blobs")) else { return vec![] };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            unb64_n::<32>(&name)?;
            let t = String::from_utf8_lossy(&fs::read(e.path()).ok()?).to_string();
            let hit = if ignore_case { t.to_lowercase().contains(&low) } else { t.contains(needle) };
            hit.then_some(name)
        })
        .collect()
}

// ---------------------------------------------------------------- what this Xenolith took

fn pos_key(p: Pos) -> String {
    format!("{}:{}", p.inc, p.seq)
}

fn pos_of(s: &str) -> Option<Pos> {
    let (a, b) = s.split_once(':')?;
    Some(Pos::new(a.parse().ok()?, b.parse().ok()?))
}

fn pos_json(p: Pos) -> Value {
    json!({ "inc": p.inc, "seq": p.seq })
}

fn pos_parse(v: &Value) -> Option<Pos> {
    Some(Pos::new(v["inc"].as_u64()?, v["seq"].as_u64()?))
}

#[derive(Clone, Debug, Default)]
struct Author {
    head: Head,
    kept: BTreeMap<Pos, [u8; 32]>,
    held: BTreeMap<Pos, FeedEntry>,
}

/// What this Xenolith took of every author's feed in one space: a xenonet-log `FeedStore` in a file
/// of its own (`<state>/sync/feeds/<spc>.json`), written whole after each change (`save`).
#[derive(Clone, Debug)]
pub struct Feeds {
    file: PathBuf,
    space: String,
    authors: BTreeMap<String, Author>,
}

impl Feeds {
    pub fn load(paths: &AppPaths, space: &str) -> Feeds {
        let file = dir(paths).join("feeds").join(format!("{space}.json"));
        let mut authors = BTreeMap::new();
        if let Some(v) = read_json(&file) {
            for (who, a) in v["authors"].as_object().into_iter().flatten() {
                let mut au = Author::default();
                au.head.latest = pos_parse(&a["head"]).unwrap_or(Pos::ORIGIN);
                let f = &a["frozen"];
                let id = |k: &str| f[k].as_str().and_then(unb64_n::<32>);
                if let (Some(at), Some(held), Some(offered)) = (pos_parse(f), id("held"), id("offered")) {
                    au.head.frozen = Some(Fork { at, held, offered });
                }
                for (k, id) in a["kept"].as_object().into_iter().flatten() {
                    if let (Some(p), Some(id)) = (pos_of(k), id.as_str().and_then(unb64_n::<32>)) {
                        au.kept.insert(p, id);
                    }
                }
                for e in a["held"].as_array().into_iter().flatten() {
                    if let Ok(e) = FeedEntry::parse(e) {
                        au.held.insert(e.pos(), e);
                    }
                }
                authors.insert(who.clone(), au);
            }
        }
        Feeds { file, space: space.to_string(), authors }
    }

    pub fn save(&self) -> Result<(), String> {
        let mut m = Map::new();
        for (who, a) in &self.authors {
            let frozen = a.head.frozen.map_or(Value::Null, |f| json!({ "inc": f.at.inc, "seq": f.at.seq, "held": b64(&f.held), "offered": b64(&f.offered) }));
            m.insert(
                who.clone(),
                json!({
                    "head": pos_json(a.head.latest),
                    "frozen": frozen,
                    "kept": a.kept.iter().map(|(p, id)| (pos_key(*p), json!(b64(id)))).collect::<Map<_, _>>(),
                    "held": a.held.values().map(FeedEntry::to_json).collect::<Vec<_>>(),
                }),
            );
        }
        write_json(&self.file, &json!({ "v": 1, "space": self.space, "authors": m }))
    }

    /// Where this Xenolith stands with each author's feed.
    pub fn heads(&self) -> Vec<(String, Pos)> {
        self.authors.iter().map(|(a, s)| (a.clone(), s.head.latest)).collect()
    }

    pub fn head_of(&self, author: &str) -> Pos {
        self.authors.get(author).map_or(Pos::ORIGIN, |a| a.head.latest)
    }

    fn au(&mut self, author: &str) -> &mut Author {
        self.authors.entry(author.to_string()).or_default()
    }
}

impl FeedStore for Feeds {
    type Error = String;
    fn head(&self, author: &str, _group: &str) -> Result<Head, String> {
        Ok(self.authors.get(author).map(|a| a.head).unwrap_or_default())
    }
    fn set_head(&mut self, author: &str, _group: &str, head: Head) -> Result<(), String> {
        self.au(author).head = head;
        Ok(())
    }
    fn kept(&self, author: &str, _group: &str, at: Pos) -> Result<Option<[u8; 32]>, String> {
        Ok(self.authors.get(author).and_then(|a| a.kept.get(&at).copied()))
    }
    fn keep(&mut self, e: &FeedEntry) -> Result<(), String> {
        self.au(&e.author).kept.insert(e.pos(), e.hash());
        Ok(())
    }
    fn know(&mut self, h: &Header) -> Result<(), String> {
        self.au(&h.author).kept.insert(h.pos(), h.hash());
        Ok(())
    }
    // A space's kinds are never compacted (their retention is Forever), so nothing is open for a
    // compact entry to keep or end.
    fn open_upto(&self, _author: &str, _group: &str, _upto: Pos) -> Result<Vec<Open>, String> {
        Ok(vec![])
    }
    fn close(&mut self, _author: &str, _group: &str, _at: Pos) -> Result<(), String> {
        Ok(())
    }
    // A space's kinds are all kept: there are no loose entries.
    fn has_loose(&self, _author: &str, _group: &str, _id: &[u8; 32]) -> Result<bool, String> {
        Ok(false)
    }
    fn take_loose(&mut self, _e: &FeedEntry) -> Result<(), String> {
        Ok(())
    }
    fn prune_loose(&mut self, _before: u64) -> Result<(), String> {
        Ok(())
    }
    fn held(&self, author: &str, _group: &str, at: Pos) -> Result<Option<FeedEntry>, String> {
        Ok(self.authors.get(author).and_then(|a| a.held.get(&at).cloned()))
    }
    fn held_in(&self, author: &str, _group: &str, inc: u64, upto: u64) -> Result<Vec<FeedEntry>, String> {
        Ok(self.authors.get(author).map(|a| a.held.range(Pos::new(inc, 0)..=Pos::new(inc, upto)).map(|(_, e)| e.clone()).collect()).unwrap_or_default())
    }
    fn hold(&mut self, e: FeedEntry) -> Result<(), String> {
        let a = self.au(&e.author);
        // Anyone admitted may send entries ahead of a gap; a few hundred are enough to wait on.
        if a.held.len() < 512 {
            a.held.insert(e.pos(), e);
        }
        Ok(())
    }
    fn unhold(&mut self, author: &str, _group: &str, at: Pos) -> Result<(), String> {
        self.au(author).held.remove(&at);
        Ok(())
    }
    fn held_firsts(&self, author: &str, _group: &str, after: u64) -> Result<Vec<FeedEntry>, String> {
        Ok(self.authors.get(author).map(|a| a.held.values().filter(|e| e.seq == 1 && e.inc > after).cloned().collect()).unwrap_or_default())
    }
    fn drop_held_before(&mut self, author: &str, _group: &str, inc: u64) -> Result<(), String> {
        self.au(author).held.retain(|p, _| p.inc >= inc);
        Ok(())
    }
}

// ---------------------------------------------------------------- what this Xenolith writes

/// This Xenolith's own feed in one space: every entry it signed, in order, in an append-only file
/// (`<state>/sync/out/<spc>.jsonl`), and its incarnation beside it (`.inc`, fsynced before the
/// first entry of it), kept for good so a member that comes later, or a curator that took over,
/// gets them all (texts dropped by a forget aside).
pub struct Outbox {
    file: PathBuf,
    writer: Writer,
    entries: Vec<FeedEntry>,
}

impl Outbox {
    pub fn load(paths: &AppPaths, space: &str, author: &str, now: u64) -> Result<Outbox, String> {
        let base = dir(paths).join("out");
        let file = base.join(format!("{space}.jsonl"));
        let inc_file = base.join(format!("{space}.inc"));
        let entries: Vec<FeedEntry> = fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| FeedEntry::parse(&v).ok())
            .filter(|e| e.author == author)
            .collect();
        let inc = match fs::read_to_string(&inc_file).ok().and_then(|t| t.trim().parse::<u64>().ok()) {
            Some(i) => i,
            None => {
                fs::create_dir_all(&base).map_err(|e| format!("cannot make {}: {e}", wiki::disp(&base)))?;
                let i = xenonet_log::feed::next_incarnation(entries.last().map_or(0, |e| e.inc), now);
                wiki::write_synced(&inc_file, &format!("{i}\n"), false).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&inc_file)))?;
                i
            }
        };
        let mine: Vec<&FeedEntry> = entries.iter().filter(|e| e.inc == inc).collect();
        let writer = match mine.last() {
            Some(last) => {
                let link = mine[0].prev.unwrap_or(Link::ORIGIN);
                Writer::resume(author, space, inc, last.seq, 0, link, Link::to(last.pos(), last.hash()))
            }
            // A new incarnation links at the last entry this file still holds.
            None => {
                let held = entries.last().map_or(Link::ORIGIN, |e| Link::to(e.pos(), e.hash()));
                Writer::resume(author, space, inc, 0, 0, held, held)
            }
        };
        Ok(Outbox { file, writer, entries })
    }

    /// Signs the next entry and appends it, fsynced, before anyone is sent it.
    pub fn write(&mut self, key: &dyn xenonet_id::sign::Signer, kind: Kind, body: Value) -> Result<FeedEntry, String> {
        let e = self.writer.write(key, kind, u64::try_from(now_ms()).unwrap_or(0), body).map_err(|e| e.message)?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(&self.file).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&self.file)))?;
        f.write_all(format!("{}\n", e.to_json()).as_bytes()).and_then(|()| f.sync_all()).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&self.file)))?;
        self.entries.push(e.clone());
        Ok(e)
    }

    /// The entries a receiver standing at `at` hasn't got: everything after it in its incarnation,
    /// and every later incarnation.
    pub fn after(&self, at: Pos) -> Vec<FeedEntry> {
        self.entries.iter().filter(|e| e.pos() > at).cloned().collect()
    }

    pub fn entries(&self) -> &[FeedEntry] {
        &self.entries
    }

    pub fn head(&self) -> Pos {
        self.entries.last().map_or(Pos::ORIGIN, FeedEntry::pos)
    }
}

// ---------------------------------------------------------------- what a member holds

/// The curator's entries a member took, kept so it can serve them to another member while the
/// curator is away (review revision 14): `<state>/sync/held/<spc>.jsonl`, one entry a line, each
/// signed by the curator that wrote it. Only the curator's kinds are kept here, never a note
/// (revision 12); their texts stay in the blobs, where a forget drops them.
pub struct Relay {
    file: PathBuf,
    entries: BTreeMap<(String, Pos), FeedEntry>,
}

/// The kinds only a curator writes: what members take and hold for each other.
pub fn is_curators(kind: Kind) -> bool {
    matches!(kind, Kind::CtxPage | Kind::CtxRemove | Kind::CtxLog | Kind::CtxForget | Kind::CtxSnapshot)
}

impl Relay {
    pub fn load(paths: &AppPaths, space: &str) -> Relay {
        let file = dir(paths).join("held").join(format!("{space}.jsonl"));
        let entries = fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| FeedEntry::parse(&v).ok())
            .filter(|e| e.group == space && is_curators(e.kind))
            .map(|e| ((e.author.clone(), e.pos()), e))
            .collect();
        Relay { file, entries }
    }

    /// Keeps `e`, appended to the file; one held already, or not a curator's kind, is left.
    pub fn keep(&mut self, e: &FeedEntry) -> Result<(), String> {
        let key = (e.author.clone(), e.pos());
        if !is_curators(e.kind) || self.entries.contains_key(&key) {
            return Ok(());
        }
        if let Some(p) = self.file.parent() {
            fs::create_dir_all(p).map_err(|err| format!("cannot make {}: {err}", wiki::disp(p)))?;
        }
        let mut f = fs::OpenOptions::new().create(true).append(true).open(&self.file).map_err(|err| format!("cannot write {}: {err}", wiki::disp(&self.file)))?;
        f.write_all(format!("{}\n", e.to_json()).as_bytes()).map_err(|err| format!("cannot write {}: {err}", wiki::disp(&self.file)))?;
        self.entries.insert(key, e.clone());
        Ok(())
    }

    /// `author`'s entries held here from `from` on, in order.
    pub fn from(&self, author: &str, from: Pos) -> Vec<FeedEntry> {
        self.entries.range((author.to_string(), from)..).take_while(|((a, _), _)| a == author).map(|(_, e)| e.clone()).collect()
    }
}

// ---------------------------------------------------------------- where members are

/// Where each member's endpoint was last said to be (`<state>/sync/addrs.json`): from its own
/// hello, or from the curator's, which passes on its members' to each other so they can reach
/// each other with the curator away. Hints only: QUIC checks the endpoint id at every dial.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Addrs(pub BTreeMap<String, Vec<String>>);

/// The most addresses kept for one endpoint.
pub const MAX_ADDRS: usize = 8;

impl Addrs {
    fn file(paths: &AppPaths) -> PathBuf {
        dir(paths).join("addrs.json")
    }

    pub fn load(paths: &AppPaths) -> Addrs {
        let v = read_json(&Addrs::file(paths)).unwrap_or(Value::Null);
        let mut m = BTreeMap::new();
        for (ep, list) in v["addrs"].as_object().into_iter().flatten() {
            if is_endpoint(ep) {
                m.insert(ep.clone(), list.as_array().into_iter().flatten().filter_map(Value::as_str).filter(|a| a.parse::<std::net::SocketAddr>().is_ok()).take(MAX_ADDRS).map(String::from).collect());
            }
        }
        Addrs(m)
    }

    /// Records `addrs` for `endpoint`; whether that changed anything.
    pub fn put(&mut self, endpoint: &str, addrs: &[String]) -> bool {
        let list: Vec<String> = addrs.iter().filter(|a| a.parse::<std::net::SocketAddr>().is_ok()).take(MAX_ADDRS).cloned().collect();
        if !is_endpoint(endpoint) || list.is_empty() || self.0.get(endpoint) == Some(&list) {
            return false;
        }
        self.0.insert(endpoint.to_string(), list);
        true
    }

    pub fn save(&self, paths: &AppPaths) -> Result<(), String> {
        write_json(&Addrs::file(paths), &json!({ "v": 1, "addrs": self.0 }))
    }
}

// ---------------------------------------------------------------- the ledger

/// A page as one side last saw it: its version (wiki's 16-digit hash of the text) and, for the
/// curator, the count of versions it published.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageAt {
    pub version: String,
    pub n: u64,
}

/// What sync knows of a space's folder (`<state>/sync/ledger/<spc>.json`), so it can tell its own
/// writes from everyone else's: on the curator, what it published of each page and log day; on a
/// member, what it wrote into the copy from the curator, and the copy's edits it proposed already.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    pub pages: BTreeMap<String, PageAt>,
    pub logs: BTreeMap<String, String>,
    /// slug → the copy's version it proposed (a removal: "none").
    pub proposed: BTreeMap<String, String>,
    /// The epoch the copy's pages are from.
    pub epoch: u32,
}

fn ledger_file(paths: &AppPaths, space: &str) -> PathBuf {
    dir(paths).join("ledger").join(format!("{space}.json"))
}

impl Ledger {
    pub fn load(paths: &AppPaths, space: &str) -> Ledger {
        let Some(v) = read_json(&ledger_file(paths, space)) else { return Ledger::default() };
        let strs = |k: &str, ok: &dyn Fn(&str) -> bool| -> BTreeMap<String, String> {
            v[k].as_object().into_iter().flatten().filter(|(a, _)| ok(a)).filter_map(|(a, b)| b.as_str().filter(|h| is_base(h)).map(|h| (a.clone(), h.to_string()))).collect()
        };
        Ledger {
            pages: v["pages"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(s, _)| SLUG_RE.is_match(s))
                .filter_map(|(s, p)| Some((s.clone(), PageAt { version: p["version"].as_str().filter(|h| is_base(h))?.into(), n: p["n"].as_u64().unwrap_or(0) })))
                .collect(),
            logs: strs("logs", &|d| DATE_RE.is_match(d)),
            proposed: strs("proposed", &|s| SLUG_RE.is_match(s)),
            epoch: v["epoch"].as_u64().and_then(|e| u32::try_from(e).ok()).unwrap_or(0),
        }
    }

    pub fn save(&self, paths: &AppPaths, space: &str) -> Result<(), String> {
        write_json(
            &ledger_file(paths, space),
            &json!({
                "v": 1,
                "pages": self.pages.iter().map(|(s, p)| (s.clone(), json!({ "version": p.version, "n": p.n }))).collect::<Map<_, _>>(),
                "logs": self.logs,
                "proposed": self.proposed,
                "epoch": self.epoch,
            }),
        )
    }
}

// ---------------------------------------------------------------- the folder

fn page_file(dir: &Path, slug: &str) -> wiki::Result<PathBuf> {
    wiki::confined_path(dir, &dir.join("pages").join(format!("{slug}.md")))
}

/// A page's version as sync compares it, or "none".
pub fn page_version(dir: &Path, slug: &str) -> String {
    read_page(dir, slug).map(|t| hash_text(&t, 16)).unwrap_or_else(|| "none".into())
}

pub fn read_page(dir: &Path, slug: &str) -> Option<String> {
    page_file(dir, slug).ok().and_then(|f| wiki::read_if_exists(&f).ok().flatten())
}

/// The pages in a space's folder now, by slug.
pub fn pages(dir: &Path) -> BTreeMap<String, String> {
    wiki::list_pages(dir).unwrap_or_default().into_iter().map(|p| p.slug).filter(|s| SLUG_RE.is_match(s)).filter_map(|s| read_page(dir, &s).map(|t| (s, t))).collect()
}

/// The days of the log a member keeps a copy of, with their text.
pub fn recent_logs(dir: &Path) -> Vec<(String, String)> {
    wiki::days_back(LOG_DAYS).into_iter().filter_map(|d| wiki::read_log_day(dir, &d).map(|t| (d, t))).collect()
}

fn keep_history(dir: &Path, slug: &str, text: &str, suffix: &str) -> wiki::Result<String> {
    let rel = format!(".history/pages/{slug}/{}-{suffix}.md", history_stamp(&now()));
    let h = wiki::confined_path(dir, &dir.join(&rel))?;
    fs::create_dir_all(h.parent().unwrap_or(dir))?;
    wiki::write_if_missing(&h, text)?;
    Ok(rel)
}

/// Writes the curator's version of a page into a member's copy, keeping what it replaces in
/// .history/.
pub fn put_page(dir: &Path, slug: &str, text: &str) -> wiki::Result<()> {
    let file = page_file(dir, slug)?;
    wiki::with_lock(dir, || {
        let cur = wiki::read_if_exists(&file)?;
        if cur.as_deref() == Some(text) {
            return Ok(());
        }
        if let Some(c) = &cur {
            keep_history(dir, slug, c, "sync")?;
        }
        fs::create_dir_all(dir.join("pages"))?;
        wiki::atomic_write(&file, text, None, None)?;
        crate::readcache::forget(&file);
        wiki::refresh_index(dir, None)?;
        Ok(())
    })
}

/// Removes a page the curator removed from a member's copy, keeping it in .history/.
pub fn remove_page_here(dir: &Path, slug: &str) -> wiki::Result<bool> {
    let file = page_file(dir, slug)?;
    wiki::with_lock(dir, || {
        let Some(cur) = wiki::read_if_exists(&file)? else { return Ok(false) };
        keep_history(dir, slug, &cur, "removed")?;
        fs::remove_file(&file)?;
        crate::readcache::forget(&file);
        wiki::refresh_index(dir, None)?;
        Ok(true)
    })
}

/// Writes the curator's log of a day into a member's copy. Lines only the copy has stay, after the
/// curator's.
pub fn merge_log(dir: &Path, date: &str, theirs: &str) -> wiki::Result<()> {
    if !DATE_RE.is_match(date) {
        return wiki_err("a log day's date");
    }
    let file = wiki::confined_path(dir, &dir.join(wiki::log_rel(date)))?;
    let local = wiki::read_if_exists(&file)?;
    let text = match &local {
        None => theirs.to_string(),
        Some(mine) => {
            let known: BTreeSet<&str> = theirs.lines().map(str::trim).collect();
            let own: Vec<&str> = mine.lines().filter(|l| !l.trim().is_empty() && !known.contains(l.trim())).collect();
            if own.is_empty() { theirs.to_string() } else { format!("{}\n\n{}\n", theirs.trim_end(), own.join("\n")) }
        }
    };
    if local.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    if let Some(p) = file.parent() {
        fs::create_dir_all(p)?;
    }
    wiki::with_lock(dir, || wiki::atomic_write(&file, &text, None, None))
}

/// One edit made in a member's copy since the curator's version: a replace with the copy's text, or
/// a removal, against `base` (the curator's version the copy had).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub slug: String,
    pub base: String,
    /// The copy's text; None for a removal.
    pub text: Option<String>,
}

impl Edit {
    pub fn version(&self) -> String {
        self.text.as_deref().map_or_else(|| "none".to_string(), |t| hash_text(t, 16))
    }

    /// The `ctx.propose` body, its text (if any) beside it.
    pub fn body(&self) -> Value {
        let mut b = json!({ "slug": self.slug, "mode": if self.text.is_some() { "replace" } else { "delete" }, "base": self.base });
        if let Some(t) = &self.text {
            let (meta, _) = crate::frontmatter::parse(t);
            b["title"] = json!(Some(meta.str("title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| self.slug.clone()));
            b["text"] = text_ref(t.as_bytes());
        }
        b
    }
}

/// The edits in a member's copy that aren't the curator's version and weren't proposed yet: pages
/// changed or made here, and pages the curator has that the copy removed.
pub fn changes_in_copy(dir: &Path, ledger: &Ledger) -> Vec<Edit> {
    let here = pages(dir);
    let mut out = vec![];
    for (slug, text) in &here {
        let v = hash_text(text, 16);
        let base = ledger.pages.get(slug).map_or("none", |p| p.version.as_str());
        if v != base && ledger.proposed.get(slug) != Some(&v) {
            out.push(Edit { slug: slug.clone(), base: base.to_string(), text: Some(text.clone()) });
        }
    }
    for (slug, p) in &ledger.pages {
        if !here.contains_key(slug) && ledger.proposed.get(slug).map(String::as_str) != Some("none") {
            out.push(Edit { slug: slug.clone(), base: p.version.clone(), text: None });
        }
    }
    out
}

/// What changed in the curator's own folder since it last published: pages new or changed (with
/// the version before), pages removed, and log days changed.
pub struct Published {
    pub pages: Vec<(String, String, String)>,
    pub removed: Vec<(String, String)>,
    pub logs: Vec<(String, String)>,
}

pub fn changes_to_publish(dir: &Path, ledger: &Ledger) -> Published {
    let here = pages(dir);
    let mut p = Published { pages: vec![], removed: vec![], logs: vec![] };
    for (slug, text) in &here {
        let base = ledger.pages.get(slug).map_or("none".to_string(), |x| x.version.clone());
        if hash_text(text, 16) != base {
            p.pages.push((slug.clone(), text.clone(), base));
        }
    }
    for (slug, at) in &ledger.pages {
        if !here.contains_key(slug) && at.version != "none" && page_gone(dir, slug) {
            p.removed.push((slug.clone(), at.version.clone()));
        }
    }
    for (date, text) in recent_logs(dir) {
        if ledger.logs.get(&date) != Some(&hash_text(&text, 16)) {
            p.logs.push((date, text));
        }
    }
    p
}

/// Whether a page's file is really gone (not just unreadable for a moment).
fn page_gone(dir: &Path, slug: &str) -> bool {
    page_file(dir, slug).is_ok_and(|f| matches!(fs::symlink_metadata(f), Err(e) if e.kind() == std::io::ErrorKind::NotFound))
}

// ---------------------------------------------------------------- a member's notes

/// The notes waiting in a member's copy of a space: they go to its curator.
pub fn waiting_notes(dir: &Path) -> Vec<inbox::Note> {
    inbox::list_notes(dir).unwrap_or_default()
}

/// The `ctx.note` body for a waiting note, and its text.
pub fn note_body(n: &inbox::Note) -> (Value, String) {
    let mut b = json!({ "id": n.id, "kind": n.kind, "app": n.app, "source": n.source, "title": n.title, "tags": n.tags, "pages": n.pages, "text": text_ref(n.body.as_bytes()) });
    if let Some(p) = &n.page {
        b["page"] = p.clone();
    }
    (b, n.body.clone())
}

/// Moves a note that went to the curator out of the inbox into .curator/sent/, saying where it went.
pub fn mark_sent(dir: &Path, id: &str, to: &str) -> wiki::Result<()> {
    let p = curator_paths(dir);
    let from = wiki::confined_path(dir, &p.inbox.join(format!("{id}.md")))?;
    let Some(text) = wiki::read_if_exists(&from)? else { return Ok(()) };
    let to_file = wiki::confined_path(dir, &p.cur.join("sent").join(inbox::month_dir(id)).join(format!("{id}.md")))?;
    fs::create_dir_all(to_file.parent().unwrap_or(dir))?;
    let stamp = format!("sent to {to} at {}", local_iso(&now()));
    // A comment can't end early: "-->" in what was said would close it.
    let content = format!("<!-- {} -->\n{text}", stamp.replace("-->", "- ->"));
    wiki::atomic_write(&to_file, &content, None, None)?;
    wiki::retrying(|| fs::remove_file(&from))?;
    inbox::clear_state(dir, id);
    Ok(())
}

// ---------------------------------------------------------------- on the curator

/// What the curator made of a member's note or edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Taken {
    /// A note in the space's inbox, for its curator to file.
    Note(String),
    /// The same note was taken before.
    Duplicate,
    /// A removal against the page as it is: done.
    Removed,
    /// Against an older version: a held change for the person's OK.
    Held { batch: String },
    /// Refused for good: the member may not, or the entry isn't right.
    Refused(String),
}

impl Taken {
    pub fn as_str(&self) -> &'static str {
        match self {
            Taken::Note(_) => "accepted",
            Taken::Duplicate => "duplicate",
            Taken::Removed => "removed",
            Taken::Held { .. } => "conflict",
            Taken::Refused(_) => "refused",
        }
    }
}

/// Takes a member's `ctx.note` or `ctx.propose` (`kind`) on the curator, as 1.x's sync took a
/// pushed entry: `who` is the member as the log and held changes name it (`key:xns1_…`), `level`
/// its level by the space's chain, `id` the entry's id (what makes a second copy a duplicate), and
/// `text` the text the body names. Its source is never the person's (`agent`): a standalone peer's
/// note can't speak as the owner here (review revision 35).
/// `person` is the machine (its id in the network) a note from a Xenolith bound to the person's
/// own device came from (review revision 35): such a note keeps the `user` source it says, stamped
/// `person@<machine>`; any other peer's is `agent`.
#[allow(clippy::too_many_arguments)]
pub fn take(dir: &Path, kind: Kind, body: &Value, text: &str, who: &str, level: Level, id: &str, person: Option<&str>) -> Taken {
    let page = if kind == Kind::CtxPropose { body.clone() } else { body["page"].clone() };
    let is_page = kind == Kind::CtxPropose || body["kind"] == "page";
    let mode = if is_page { page["mode"].as_str().unwrap_or("append") } else { "" };
    let need = if matches!(mode, "replace" | "delete") { Level::Write } else { Level::Append };
    if level < need {
        return Taken::Refused(format!("{who} may not {} here", if need == Level::Write { "replace or remove pages" } else { "leave notes" }));
    }
    let slug = page["slug"].as_str().unwrap_or("").to_string();
    let base = page["base"].as_str().filter(|b| is_base(b));
    let r = (|| -> wiki::Result<Taken> {
        if mode == "delete" {
            let (true, Some(base)) = (SLUG_RE.is_match(&slug), base.filter(|b| *b != "none")) else {
                return Ok(Taken::Refused("a removal names its page's slug and the version removed".into()));
            };
            return match remove_page(dir, &slug, base, who)? {
                Removal::Removed => Ok(Taken::Removed),
                Removal::Gone => Ok(Taken::Duplicate),
                Removal::Moved => propose_removal(dir, who, id, body, &slug, base).map(|batch| Taken::Held { batch }),
            };
        }
        let title = if kind == Kind::CtxPropose { page["title"].as_str().unwrap_or(&slug).to_string() } else { body["title"].as_str().unwrap_or("").to_string() };
        if let (true, Some(base)) = (mode == "replace", base) {
            let slug = if slug.is_empty() { wiki::slugify(&title) } else { slug.clone() };
            let current = page_version(dir, &slug);
            if current != base {
                return propose(dir, who, id, &slug, &title, text, base, &current, body).map(|batch| Taken::Held { batch });
            }
        }
        // An edit made in a copy is a whole page: its body, without the frontmatter, is the content.
        let content = if kind == Kind::CtxPropose { crate::frontmatter::parse(text).1 } else { text.to_string() };
        let s = |k: &str| body[k].as_str().unwrap_or("").to_string();
        let input = NoteInput {
            kind: if is_page { "page".into() } else { "log".into() },
            source: if person.is_some() && body["source"] == "user" { "user".into() } else { "agent".into() },
            app: if kind == Kind::CtxPropose { "sync".into() } else { s("app") },
            title: if title.trim().is_empty() { slug.clone() } else { title },
            body: if content.trim().is_empty() { format!("# {slug}") } else { content },
            tags: body.get("tags").cloned().unwrap_or(Value::Null),
            pages: if kind == Kind::CtxPropose { json!([slug]) } else { body.get("pages").cloned().unwrap_or(Value::Null) },
            page: is_page.then(|| PageInput {
                slug: Some(slug.clone()).filter(|s| !s.is_empty()),
                title: page["title"].as_str().map(String::from),
                kind: page["type"].as_str().map(String::from),
                summary: page["summary"].as_str().map(String::from),
                mode: Some(mode.to_string()).filter(|m| m == "replace"),
                base: base.map(String::from),
            }),
            // The same entry taken twice is one note: by who sent it and the entry's id.
            idempotency_key: Some(format!("sync2:{who}:{id}")),
        };
        let stamp = person.map(|m| format!("{}{m}", inbox::PERSON_STAMP));
        let r = inbox::submit_note(dir, &input, "sync", &format!("sync {who}"), &By::Token(stamp.as_deref().unwrap_or(who)))?;
        Ok(if r.duplicate { Taken::Duplicate } else { Taken::Note(r.id) })
    })();
    r.unwrap_or_else(|e| Taken::Refused(e.to_string()))
}

enum Removal {
    Removed,
    Gone,
    Moved,
}

fn remove_page(dir: &Path, slug: &str, base: &str, who: &str) -> wiki::Result<Removal> {
    wiki::with_lock(dir, || {
        let file = page_file(dir, slug)?;
        let Some(cur) = wiki::read_if_exists(&file)? else { return Ok(Removal::Gone) };
        if hash_text(&cur, 16) != base {
            return Ok(Removal::Moved);
        }
        keep_history(dir, slug, &cur, "removed")?;
        fs::remove_file(&file)?;
        crate::readcache::forget(&file);
        wiki::refresh_index(dir, None)?;
        let title = Some(crate::frontmatter::parse(&cur).0.str("title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| slug.to_string());
        crate::held::log_line(dir, "sync", &format!("Removed {} [[{slug}]], as {who} did", one_line(&title)))?;
        Ok(Removal::Removed)
    })
}

#[allow(clippy::too_many_arguments)]
fn propose(dir: &Path, who: &str, id: &str, slug: &str, title: &str, text: &str, base: &str, current: &str, body: &Value) -> wiki::Result<String> {
    if !SLUG_RE.is_match(slug) {
        return wiki_err("A page write needs a page slug or title.");
    }
    let title = if title.trim().is_empty() { slug.to_string() } else { title.to_string() };
    let content = crate::frontmatter::parse(text).1;
    let content = if content.trim().is_empty() { text.to_string() } else { content };
    wiki::assert_no_secrets(&[("title", &title), ("content", &content)])?;
    let batch = inbox::new_note_id(&now());
    let cur = wiki::read_if_exists(&page_file(dir, slug)?)?;
    let op = json!({ "action": if cur.is_some() { "replace" } else { "create" }, "slug": slug, "title": title, "content": content });
    let Some(write) = crate::curator::page_write(&op, cur.as_deref(), &batch, &now()) else {
        return wiki_err("The page already says this.");
    };
    let what = if base == "none" { "a new page".to_string() } else { format!("version {base}") };
    let reason = if current == "none" {
        format!("{who} wrote this against {what}, but the page has been removed since.")
    } else {
        format!("{who} wrote this against {what}, but the page is at {current} now.")
    };
    hold(dir, &batch, (id, body, who), write, op, reason, &format!("{} [[{slug}]] (written against an older version)", one_line(&title)))?;
    Ok(batch)
}

fn propose_removal(dir: &Path, who: &str, id: &str, body: &Value, slug: &str, base: &str) -> wiki::Result<String> {
    let Some(cur) = wiki::read_if_exists(&page_file(dir, slug)?)? else { return wiki_err("The page is gone already.") };
    let current = hash_text(&cur, 16);
    let title = Some(crate::frontmatter::parse(&cur).0.str("title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| slug.to_string());
    let batch = inbox::new_note_id(&now());
    let op = json!({ "action": "delete", "slug": slug, "title": title });
    let write = PageWrite {
        slug: slug.to_string(),
        title: title.clone(),
        action: "removed".into(),
        rel: format!("pages/{slug}.md"),
        base_hash: Some(current.clone()),
        base_text: Some(cur),
        new_hash: "none".into(),
        text: String::new(),
        history: None,
        note_ids: vec![],
        reason: String::new(),
    };
    let reason = format!("{who} removed this page from version {base}, but the page is at {current} now.");
    hold(dir, &batch, (id, body, who), write, op, reason, &format!("removing {} [[{slug}]] (removed from an older version)", one_line(&title)))?;
    Ok(batch)
}

fn hold(dir: &Path, batch: &str, from: (&str, &Value, &str), write: PageWrite, op: Value, reason: String, what: &str) -> wiki::Result<()> {
    let (id, body, who) = from;
    let held = HeldOp {
        write,
        reasons: vec![reason],
        op,
        notes: vec![json!({ "id": id, "app": body["app"].as_str().unwrap_or("sync"), "title": body["title"], "source": "agent", "by": who })],
        kind: "sync".into(),
    };
    wiki::with_lock(dir, || {
        crate::held::record(dir, batch, &local_iso(&now()), std::slice::from_ref(&held))?;
        crate::held::log_line(dir, "curator", &format!("Proposal from {who} waiting for your OK: {what}"))
    })
}

/// Whether `id` is a note id (as `ctx.note` bodies carry their member's).
pub fn is_note(id: &str) -> bool {
    is_note_id(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_holds_only_the_curators_kinds_once_each_and_serves_them_in_order() {
        let base = std::env::temp_dir().join(format!("xenolith-relay-{}", random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), base.to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        let key = xenonet_id::sign::SeedSigner::from_seed(&[5; 32], xenonet_id::sign::KeyHome::File).unwrap();
        let author = xenonet_id::sign::Signer::public(&key).id();
        let space = "spc_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut w = Writer::resume(&author, space, 7, 0, 0, Link::ORIGIN, Link::ORIGIN);
        let page = w.write(&key, Kind::CtxPage, 1, json!({ "slug": "a", "epoch": 0 })).unwrap();
        let note = w.write(&key, Kind::CtxNote, 2, json!({ "title": "t" })).unwrap();
        let log = w.write(&key, Kind::CtxLog, 3, json!({ "date": "2026-11-11", "epoch": 0 })).unwrap();
        let mut r = Relay::load(&paths, space);
        for e in [&page, &note, &log, &page] {
            r.keep(e).unwrap();
        }
        let again = Relay::load(&paths, space);
        let held: Vec<Kind> = again.from(&author, Pos::ORIGIN).iter().map(|e| e.kind).collect();
        assert_eq!(held, [Kind::CtxPage, Kind::CtxLog], "never a note, each once, kept across a restart");
        assert_eq!(again.from(&author, Pos::new(7, 2)).len(), 1, "from a place on");
        assert!(again.from("xns1_other", Pos::ORIGIN).is_empty());

        let mut a = Addrs::load(&paths);
        let ep = "ab".repeat(32);
        assert!(a.put(&ep, &["127.0.0.1:47851".into(), "not an address".into()]));
        assert!(!a.put(&ep, &["127.0.0.1:47851".into()]), "the same again changes nothing");
        assert!(!a.put("short", &["127.0.0.1:1".into()]), "no endpoint id");
        a.save(&paths).unwrap();
        assert_eq!(Addrs::load(&paths).0.get(&ep), Some(&vec!["127.0.0.1:47851".to_string()]));
        let _ = fs::remove_dir_all(&base);
    }
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
    use xenonet_log::feed::{Feed, Offer};

    fn home() -> (PathBuf, AppPaths) {
        let home = std::env::temp_dir().join(format!("xenolith-sync-{}", random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), home.to_string_lossy().to_string())].into_iter().collect();
        (home.clone(), AppPaths::from_env(&env))
    }

    const S: &str = "spc_abcdefghijklmnopqrstuvwxyz234567";

    #[test]
    fn an_outbox_survives_a_restart_and_a_feed_store_takes_each_entry_once() {
        let (home, paths) = home();
        let k = SeedSigner::from_seed(&[3; 32], KeyHome::File).unwrap();
        let mut out = Outbox::load(&paths, S, &k.public().id(), 1_000).unwrap();
        let a = out.write(&k, Kind::CtxNote, json!({ "n": 1 })).unwrap();
        let b = out.write(&k, Kind::CtxNote, json!({ "n": 2 })).unwrap();
        drop(out);
        let mut out = Outbox::load(&paths, S, &k.public().id(), 2_000).unwrap();
        assert_eq!(out.entries(), [a.clone(), b.clone()], "read back as written");
        let c = out.write(&k, Kind::CtxNote, json!({ "n": 3 })).unwrap();
        assert_eq!((c.inc, c.seq), (a.inc, 3), "the same incarnation after a restart");
        assert_eq!(out.after(a.pos()), [b.clone(), c.clone()]);

        let mut f = Feed::at(Feeds::load(&paths, S), 0);
        for e in [&b, &a, &c, &a] {
            let _ = f.offer(e.clone().verify(&k.public()).unwrap()).unwrap();
        }
        f.store().save().unwrap();
        let mut again = Feed::at(Feeds::load(&paths, S), 0);
        assert_eq!(again.store().head_of(&k.public().id()), c.pos(), "all three, in order, kept across a restart");
        assert_eq!(again.offer(b.clone().verify(&k.public()).unwrap()).unwrap(), Offer::Duplicate);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn texts_are_kept_by_their_hash_and_dropped_by_a_forget() {
        let (home, paths) = home();
        let id = put_text(&paths, b"the secret is hunter2").unwrap();
        assert_eq!(id, text_id(b"the secret is hunter2"));
        assert_eq!(named_text(&json!({ "text": text_ref(b"the secret is hunter2") })), Some(id.clone()));
        assert_eq!(texts_with(&paths, "HUNTER2", true), vec![id.clone()]);
        assert!(texts_with(&paths, "HUNTER2", false).is_empty());
        assert_eq!(drop_texts(&paths, std::slice::from_ref(&id)), 1);
        assert!(text(&paths, &id).is_none());
        assert!(blob_file(&paths, "../x").is_none(), "an id is a hash, never a path");
        let _ = fs::remove_dir_all(home);
    }

    fn space_dir(home: &Path) -> PathBuf {
        let d = home.join("wiki").join("spaces").join("project").join("atlas");
        fs::create_dir_all(d.join("pages")).unwrap();
        fs::create_dir_all(d.join("inbox")).unwrap();
        d
    }

    #[test]
    fn the_curator_takes_a_note_once_and_holds_a_stale_replace_for_the_person() {
        let (home, _paths) = home();
        let dir = space_dir(&home);
        fs::write(dir.join("pages").join("db.md"), "---\ntitle: Atlas database\n---\n\n# Atlas database\n\nPostgres 16.\n").unwrap();
        let note = json!({ "id": "2026-11-11_09-00-00-000-abc123", "kind": "log", "app": "codex", "source": "user", "title": "Moving to 17", "tags": [], "pages": ["db"] });
        let who = "key:xns1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let Taken::Note(id) = take(&dir, Kind::CtxNote, &note, "Logical replication.", who, Level::Append, "e1", None) else { panic!() };
        let text = fs::read_to_string(dir.join("inbox").join(format!("{id}.md"))).unwrap();
        assert!(text.contains("source: agent"), "a peer's note never speaks as the person: {text}");
        assert!(text.contains("transport: sync"));
        assert_eq!(take(&dir, Kind::CtxNote, &note, "Logical replication.", who, Level::Append, "e1", None), Taken::Duplicate);
        assert!(matches!(take(&dir, Kind::CtxNote, &note, "x", who, Level::Read, "e2", None), Taken::Refused(_)), "a reader leaves no notes");
        // From a Xenolith bound to the person's own device on another machine: the person's, named
        // by that machine (review revision 35).
        let Taken::Note(id) = take(&dir, Kind::CtxNote, &note, "Logical replication.", who, Level::Append, "e9", Some("laptop")) else { panic!() };
        let text = fs::read_to_string(dir.join("inbox").join(format!("{id}.md"))).unwrap();
        assert!(text.contains("source: user") && text.contains("person@laptop"), "{text}");
        assert_eq!(crate::inbox::trusted_source("user", "person@laptop"), "user");
        assert_eq!(crate::inbox::trusted_source("user", "person@"), "agent");
        assert_eq!(crate::inbox::trusted_source("user", "key:xns1_x"), "agent");

        let page = "---\ntitle: Atlas database\n---\n\n# Atlas database\n\nPostgres 17 here.\n";
        let edit = Edit { slug: "db".into(), base: "0123456789abcdef".into(), text: Some(page.into()) };
        assert!(matches!(take(&dir, Kind::CtxPropose, &edit.body(), page, who, Level::Append, "e3", None), Taken::Refused(_)), "a replace needs write");
        let Taken::Held { batch } = take(&dir, Kind::CtxPropose, &edit.body(), page, who, Level::Write, "e3", None) else { panic!("a stale replace is held") };
        let held = crate::held::list(&dir);
        assert!(held.iter().any(|h| h["batch"] == batch && h["kind"] == "sync"), "{held:?}");
        let current = page_version(&dir, "db");
        let fresh = Edit { slug: "db".into(), base: current, text: Some(page.into()) };
        assert!(matches!(take(&dir, Kind::CtxPropose, &fresh.body(), page, who, Level::Write, "e4", None), Taken::Note(_)), "against the page as it is: a note");
        let gone = Edit { slug: "db".into(), base: page_version(&dir, "db"), text: None };
        assert_eq!(take(&dir, Kind::CtxPropose, &gone.body(), "", who, Level::Write, "e5", None), Taken::Removed);
        assert!(!dir.join("pages").join("db.md").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_copys_own_edits_are_found_once_and_the_curators_changes_are_found_to_publish() {
        let (home, _paths) = home();
        let dir = space_dir(&home);
        let mut l = Ledger::default();
        fs::write(dir.join("pages").join("a.md"), "# A\n").unwrap();
        let p = changes_to_publish(&dir, &l);
        assert_eq!(p.pages.iter().map(|(s, _, b)| (s.as_str(), b.as_str())).collect::<Vec<_>>(), [("a", "none")]);
        l.pages.insert("a".into(), PageAt { version: page_version(&dir, "a"), n: 1 });
        assert!(changes_to_publish(&dir, &l).pages.is_empty());
        assert!(changes_in_copy(&dir, &l).is_empty(), "the curator's version is no edit");
        fs::write(dir.join("pages").join("a.md"), "# A, edited\n").unwrap();
        let e = changes_in_copy(&dir, &l);
        assert_eq!(e.len(), 1);
        l.proposed.insert("a".into(), e[0].version());
        assert!(changes_in_copy(&dir, &l).is_empty(), "proposed once");
        fs::remove_file(dir.join("pages").join("a.md")).unwrap();
        let e = changes_in_copy(&dir, &l);
        assert_eq!((e.len(), e[0].text.is_none()), (1, true), "a removal");
        assert_eq!(changes_to_publish(&dir, &l).removed, [("a".to_string(), l.pages["a"].version.clone())]);
        let _ = fs::remove_dir_all(home);
    }
}
