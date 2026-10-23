//! Entry sync between machines, with one curator per space.
//!
//! Every space has one curator: the machine whose Xenolith files its notes into pages. A space
//! created here is curated here. Another machine can follow it: it keeps a copy of the space,
//! where its sessions read the pages and leave notes as usual, and its own curator leaves the
//! space alone. `xenolith sync` (and the curator process, once a minute) runs one pass per
//! followed space:
//!
//! 1. asks the curator for its list of pages, and checks it is the curator this machine pinned at
//!    its first pull (a random id each curated space has), so two curators of one space never mix;
//! 2. turns pages changed in the copy since the last pull into notes: a replace against the
//!    version last pulled, or a removal;
//! 3. pushes the notes waiting in the copy to the curator as entries, in batches of 100 (a backlog
//!    over 1,000 is pushed batch after batch until it is drained), moving each one the curator took
//!    to .curator/sent/ and each one it refused (403 or 400) to .curator/sync/dead/, where it stops
//!    being retried; and
//! 4. pulls whatever the push's outcome, unless the curator refused this machine's token: the
//!    curator's pages, its removals (tombstones, kept 30 days) and the last 7 days of its log.
//!
//! The curator's HTTP service takes entries at /sync/v1/spaces/<kind>/<name>/entries, with a token
//! whose subject is the other machine (machine:<name>): a note needs append there, a page replaced
//! or removed needs write. A page replaced or removed against an older version of the page than
//! the curator has (the hash the other machine saw, the note's base) is not applied: the curator
//! answers 409, and the change becomes a proposal, a held change of kind "sync" that waits for the
//! person's OK in the window even when approvals are automatic. Nothing written on either machine
//! is lost: a pull keeps what it replaces in .history/.
//!
//! Who follows what is sync.json in the config folder ({ machine, peers, follow, timeoutSeconds });
//! the token for each peer is peers/<name>.token there, readable by you alone. The peers' URLs are
//! the base of their Xenolith service, with or without a path: http://127.0.0.1:<port> on one
//! machine, or an https:// address that forwards to it (a tunnel or a gateway), which must strip
//! its own path prefix before the request reaches the curator.

use crate::access::{Level, Principal};
use crate::curator::{HeldOp, PageWrite};
use crate::inbox::{self, NoteInput, PageInput, curator_paths, is_base, is_note_id};
use crate::paths::AppPaths;
use crate::spaces::{self, SpaceId};
use crate::text::{hash_text, history_stamp, local_iso, now, now_ms, one_line, random_hex};
use crate::wiki::{self, DATE_RE, Result, SLUG_RE, wiki_err};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub const API: &str = "/sync/v1";
/// At most this many entries in one push.
pub const MAX_ENTRIES: usize = 100;
/// A backlog bigger than this is pushed batch after batch until it is drained; a smaller one goes
/// one batch a pass, so a curator is never flooded by machines catching up.
pub const DRAIN_ABOVE: usize = 1000;
/// How long one request to a peer may take, unless sync.json's timeoutSeconds says otherwise. A peer
/// reached through a forwarder can be slow.
pub const DEFAULT_TIMEOUT_SECS: u64 = 90;
/// How long a curator tells its followers about a page it removed.
pub const TOMBSTONE_DAYS: i64 = 30;
/// How many days of the curator's log a follower keeps a copy of.
pub const LOG_DAYS: i64 = 7;

// ---------------------------------------------------------------- configuration

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// This machine's name, as the curators it pushes to know it (their grants name machine:<it>).
    pub machine: Option<String>,
    /// Peer name → the base URL of its Xenolith service.
    pub peers: BTreeMap<String, String>,
    /// Followed space → the peer that curates it.
    pub follow: BTreeMap<String, String>,
    /// Followed space → its curator's id, pinned at the first pull.
    pub curators: BTreeMap<String, String>,
    /// sync.json's timeoutSeconds: how long one request to a peer may take.
    pub timeout_seconds: Option<u64>,
}

pub fn config_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("sync.json")
}

pub fn token_file(paths: &AppPaths, peer: &str) -> PathBuf {
    paths.config_dir.join("peers").join(format!("{peer}.token"))
}

pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_alphanumeric() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// https:// anywhere, or http:// to this machine only (a token never crosses a network in clear).
/// The base may carry a path (https://host/xenolith): the sync routes go under it.
pub fn valid_url(u: &str) -> bool {
    let u = u.trim_end_matches('/');
    let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| u.starts_with(p));
    (u.starts_with("https://") || local) && !u.contains(['?', '#', ' ', '"', '\\']) && u.len() < 300
}

/// A curator's id: 32 lowercase hex digits.
pub fn is_curator_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl Config {
    pub fn load(paths: &AppPaths) -> Config {
        let v: Value = fs::read_to_string(config_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let peers: BTreeMap<String, String> =
            v["peers"].as_object().into_iter().flatten().filter_map(|(n, u)| u.as_str().filter(|u| valid_name(n) && valid_url(u)).map(|u| (n.clone(), u.to_string()))).collect();
        let (mut follow, mut curators) = (BTreeMap::new(), BTreeMap::new());
        for (space, f) in v["follow"].as_object().into_iter().flatten() {
            if !SpaceId::parse(space).is_ok_and(|s| !s.is_local()) {
                continue;
            }
            // { "peer": ..., "curator": ... }, or the peer's name alone (sync.json before curator ids).
            let peer = f.as_str().or(f["peer"].as_str()).filter(|p| valid_name(p));
            let Some(peer) = peer else { continue };
            follow.insert(space.clone(), peer.to_string());
            if let Some(id) = f["curator"].as_str().filter(|c| is_curator_id(c)) {
                curators.insert(space.clone(), id.to_string());
            }
        }
        Config { machine: v["machine"].as_str().filter(|m| valid_name(m)).map(String::from), peers, follow, curators, timeout_seconds: v["timeoutSeconds"].as_u64().filter(|s| *s > 0) }
    }

    /// Writes sync.json for you alone (and, on Windows, the service, which reads who curates what).
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let follow: Map<String, Value> = self
            .follow
            .iter()
            .map(|(space, peer)| {
                let mut f = json!({ "peer": peer });
                if let Some(id) = self.curators.get(space) {
                    f["curator"] = json!(id);
                }
                (space.clone(), f)
            })
            .collect();
        let mut v = json!({ "machine": self.machine, "peers": self.peers, "follow": follow });
        if let Some(t) = self.timeout_seconds {
            v["timeoutSeconds"] = json!(t);
        }
        crate::private::write(&config_file(paths), &format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default()), &crate::private::service_readers())?;
        Ok(())
    }

    /// The peer that curates a space, when this machine only follows it.
    pub fn curator_of(&self, space: &SpaceId) -> Option<&str> {
        self.follow.get(&space.to_string()).map(String::as_str)
    }

    /// How long one request to a peer may take: timeoutSeconds (5 s to 10 min), else 90 s.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(5, 600))
    }

    /// Starts following a space (again): the curator it answers as is pinned at the next pull.
    pub fn follow(&mut self, space: &SpaceId, peer: &str) {
        self.follow.insert(space.to_string(), peer.to_string());
        self.curators.remove(&space.to_string());
    }

    pub fn unfollow(&mut self, space: &SpaceId) {
        self.follow.remove(&space.to_string());
        self.curators.remove(&space.to_string());
    }

    /// Forgets a peer, with what this machine followed from it.
    pub fn remove_peer(&mut self, peer: &str) {
        self.peers.remove(peer);
        let gone: Vec<String> = self.follow.iter().filter(|(_, p)| *p == peer).map(|(s, _)| s.clone()).collect();
        for s in gone {
            self.follow.remove(&s);
            self.curators.remove(&s);
        }
    }
}

pub fn update<T>(paths: &AppPaths, f: impl FnOnce(&mut Config) -> Result<T>) -> Result<T> {
    fs::create_dir_all(&paths.config_dir)?;
    let mut guard = match crate::lock::acquire(&paths.config_dir, "sync", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return wiki_err(format!("Sync settings are busy ({}).", b.0)),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.into()),
    };
    let mut c = Config::load(paths);
    let r = f(&mut c);
    if r.is_ok() {
        c.save(paths)?;
    }
    guard.release();
    r
}

/// Keeps a peer's token, readable by you alone from the moment it exists (on Windows too).
pub fn save_token(paths: &AppPaths, peer: &str, token: &str) -> Result<()> {
    crate::private::write(&token_file(paths, peer), &format!("{}\n", token.trim()), &[])?;
    Ok(())
}

fn read_token(paths: &AppPaths, peer: &str) -> Option<String> {
    fs::read_to_string(token_file(paths, peer)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// The base URL with the sync routes of one space under it. A base with a path keeps it:
/// https://box.example/xenolith → https://box.example/xenolith/sync/v1/spaces/<kind>/<name>.
pub fn space_url(base: &str, space: &SpaceId) -> String {
    format!("{}{API}/spaces/{}/{}", base.trim_end_matches('/'), space.kind, space.name)
}

// ---------------------------------------------------------------- files in a space

/// .curator/sync/ in a space: the curator's id and manifest, or a follower's state, status and
/// dead letters.
fn sync_dir(dir: &Path) -> PathBuf {
    curator_paths(dir).cur.join("sync")
}

fn dead_dir(dir: &Path) -> PathBuf {
    sync_dir(dir).join("dead")
}

fn page_file(dir: &Path, slug: &str) -> Result<PathBuf> {
    wiki::confined_path(dir, &dir.join("pages").join(format!("{slug}.md")))
}

fn log_file(dir: &Path, date: &str) -> Result<PathBuf> {
    wiki::confined_path(dir, &dir.join(wiki::log_rel(date)))
}

/// The page's hash as sync compares it, or "none".
pub fn page_base(dir: &Path, slug: &str) -> String {
    page_file(dir, slug).ok().and_then(|f| wiki::read_if_exists(&f).ok().flatten()).map(|t| hash_text(&t, 16)).unwrap_or_else(|| "none".into())
}

fn read_json(dir: &Path, file: &Path) -> Option<Value> {
    wiki::read_if_exists(&wiki::confined_path(dir, file).ok()?).ok().flatten().and_then(|t| serde_json::from_str(&t).ok())
}

fn write_json(dir: &Path, file: &Path, v: &Value) -> Result<()> {
    let file = wiki::confined_path(dir, file)?;
    wiki::atomic_write(&file, &format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default()), Some(&curator_paths(dir).tmp), None)
}

/// Keeps a page's text in .history/ before sync replaces or removes it.
fn keep_history(dir: &Path, slug: &str, text: &str, suffix: &str) -> Result<String> {
    let rel = format!(".history/pages/{slug}/{}-{suffix}.md", history_stamp(&now()));
    let h = wiki::confined_path(dir, &dir.join(&rel))?;
    fs::create_dir_all(h.parent().unwrap_or(dir))?;
    wiki::write_if_missing(&h, text)?;
    Ok(rel)
}

// ---------------------------------------------------------------- the curator's side

/// The curator's id and its manifest are read and written by the HTTP service's threads: one at a time.
static CURATOR_FILES: Mutex<()> = Mutex::new(());

/// This space's curator id: random, made the first time a follower asks, kept in
/// .curator/sync/curator.json. A follower pins it, so two curators of one space never mix.
pub fn curator_id(dir: &Path) -> Result<String> {
    let _one = CURATOR_FILES.lock().unwrap_or_else(|e| e.into_inner());
    curator_id_locked(dir)
}

fn curator_id_locked(dir: &Path) -> Result<String> {
    let file = sync_dir(dir).join("curator.json");
    if let Some(id) = read_json(dir, &file).and_then(|v| v["id"].as_str().filter(|i| is_curator_id(i)).map(String::from)) {
        return Ok(id);
    }
    fs::create_dir_all(sync_dir(dir))?;
    let id = random_hex(32);
    write_json(dir, &file, &json!({ "id": id, "created": local_iso(&now()) }))?;
    Ok(id)
}

/// What the curator remembers between two lists: the pages it showed, and those removed since.
#[derive(Default, Serialize, Deserialize, PartialEq)]
struct Manifest {
    #[serde(default)]
    pages: BTreeSet<String>,
    /// slug → when the curator noticed it was gone (ms).
    #[serde(default)]
    deleted: BTreeMap<String, i64>,
}

fn manifest_file(dir: &Path) -> PathBuf {
    sync_dir(dir).join("manifest.json")
}

fn load_manifest(dir: &Path) -> Manifest {
    let m: Manifest = read_json(dir, &manifest_file(dir)).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    // The file is in the wiki folder, so it is checked like any input.
    Manifest { pages: m.pages.into_iter().filter(|s| SLUG_RE.is_match(s)).collect(), deleted: m.deleted.into_iter().filter(|(s, _)| SLUG_RE.is_match(s)).collect() }
}

/// Whether a page's file is really gone (not just unreadable for a moment).
fn page_gone(dir: &Path, slug: &str) -> bool {
    page_file(dir, slug).is_ok_and(|f| matches!(fs::symlink_metadata(f), Err(e) if e.kind() == std::io::ErrorKind::NotFound))
}

/// Records a page removed here, for the followers' next pull.
fn tombstone(dir: &Path, slug: &str) -> Result<()> {
    let _one = CURATOR_FILES.lock().unwrap_or_else(|e| e.into_inner());
    let mut m = load_manifest(dir);
    m.pages.remove(slug);
    m.deleted.insert(slug.to_string(), now_ms());
    fs::create_dir_all(sync_dir(dir))?;
    write_json(dir, &manifest_file(dir), &serde_json::to_value(&m).unwrap_or_default())
}

/// The curator's answer to a follower: its id, its pages by slug and hash, the pages removed in the
/// last 30 days (tombstones), and the log's last 7 days by date and hash. A page the previous
/// answer listed whose file is gone now is a removal; one that only failed to read is not.
pub fn page_list(dir: &Path) -> Result<Value> {
    let _one = CURATOR_FILES.lock().unwrap_or_else(|e| e.into_inner());
    let id = curator_id_locked(dir)?;
    let mut pages: BTreeMap<String, String> = BTreeMap::new();
    for p in wiki::list_pages(dir)? {
        if !SLUG_RE.is_match(&p.slug) {
            continue;
        }
        if let Some(text) = wiki::read_if_exists(&page_file(dir, &p.slug)?)? {
            pages.insert(p.slug, hash_text(&text, 16));
        }
    }
    let old = load_manifest(dir);
    let now = now_ms();
    let mut m = Manifest { pages: pages.keys().cloned().collect(), deleted: old.deleted.clone() };
    for slug in old.pages.iter().filter(|s| !pages.contains_key(*s)) {
        if page_gone(dir, slug) {
            m.deleted.entry(slug.clone()).or_insert(now);
        } else {
            m.pages.insert(slug.clone()); // unreadable just now: still a page
        }
    }
    m.deleted.retain(|slug, at| !pages.contains_key(slug) && now - *at < TOMBSTONE_DAYS * 86_400_000);
    if m != old {
        fs::create_dir_all(sync_dir(dir))?;
        write_json(dir, &manifest_file(dir), &serde_json::to_value(&m).unwrap_or_default())?;
    }
    let log: Vec<Value> = wiki::days_back(LOG_DAYS).into_iter().filter_map(|d| wiki::read_log_day(dir, &d).map(|t| json!({ "date": d, "hash": hash_text(&t, 16) }))).collect();
    Ok(json!({
        "curator": id,
        "pages": pages.iter().map(|(slug, hash)| json!({ "slug": slug, "hash": hash })).collect::<Vec<_>>(),
        "deleted": m.deleted.keys().collect::<Vec<_>>(),
        "log": log,
    }))
}

pub fn page_get(dir: &Path, slug: &str) -> Option<Value> {
    if !SLUG_RE.is_match(slug) {
        return None;
    }
    let text = wiki::read_if_exists(&page_file(dir, slug).ok()?).ok().flatten()?;
    Some(json!({ "slug": slug, "hash": hash_text(&text, 16), "text": text }))
}

/// One day of the log, within the days the list shows.
pub fn log_get(dir: &Path, date: &str) -> Option<Value> {
    if !DATE_RE.is_match(date) || !wiki::days_back(LOG_DAYS).iter().any(|d| d == date) {
        return None;
    }
    let text = wiki::read_log_day(dir, date)?;
    Some(json!({ "date": date, "hash": hash_text(&text, 16), "text": text }))
}

/// What the curator answers about one entry.
fn result(id: &str, outcome: &str, extra: Vec<(&str, Value)>) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("result".into(), json!(outcome));
    for (k, v) in extra {
        o.insert(k.into(), v);
    }
    Value::Object(o)
}

/// An entry refused for good: 403 (the token may not do it) or 400 (the entry is invalid). The
/// follower moves it to its dead letters and stops sending it.
fn refused(id: &str, status: u16, why: &str) -> Value {
    result(id, "refused", vec![("status", json!(status)), ("error", json!(why))])
}

/// An entry that failed here for a reason of the curator's own (a full disk, a busy lock): the
/// follower keeps it and sends it again.
fn failed(id: &str, why: &str) -> Value {
    result(id, "error", vec![("status", json!(500)), ("error", json!(why))])
}

/// The answer for an entry the curator could not take: refused when the entry itself is at fault.
fn not_taken(id: &str, e: &wiki::Error) -> Value {
    match e {
        wiki::Error::Wiki(m) => refused(id, 400, m),
        other => failed(id, &other.to_string()),
    }
}

/// Records a page write that came in against an older version as a proposal: a held change of
/// kind "sync" that replaces the page as it is now with what the other machine wrote. Approving it
/// later applies it only if the page hasn't changed again.
fn propose(dir: &Path, who: &str, by: &str, id: &str, note: &Value, base: &str, current: &str) -> Result<(String, usize)> {
    let page = &note["page"];
    let slug = page["slug"].as_str().filter(|s| SLUG_RE.is_match(s)).map(String::from).unwrap_or_else(|| wiki::slugify(note["title"].as_str().unwrap_or("")));
    if !SLUG_RE.is_match(&slug) {
        return wiki_err("A page write needs a page slug or title.");
    }
    let title = page["title"].as_str().filter(|t| !t.trim().is_empty()).or(note["title"].as_str()).unwrap_or(&slug).to_string();
    let content = note["body"].as_str().unwrap_or("");
    wiki::assert_no_secrets(&[("title", &title), ("content", content)])?;
    let batch = inbox::new_note_id(&now());
    let cur = wiki::read_if_exists(&page_file(dir, &slug)?)?;
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
    hold(dir, &batch, (id, note, who, by), write, op, reason, &format!("{} [[{slug}]] (written against an older version)", one_line(&title)))
}

/// Records a removal that came in against an older version as a proposal: approving it removes the
/// page only if it is still the version the proposal was made on.
fn propose_removal(dir: &Path, who: &str, by: &str, id: &str, note: &Value, slug: &str, base: &str) -> Result<(String, usize)> {
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
    hold(dir, &batch, (id, note, who, by), write, op, reason, &format!("removing {} [[{slug}]] (removed from an older version)", one_line(&title)))
}

/// Records one proposal from another machine. `from`: the entry's id and note, who pushed it as the
/// log names it, and the subject its note is stamped with.
fn hold(dir: &Path, batch: &str, from: (&str, &Value, &str, &str), write: PageWrite, op: Value, reason: String, what: &str) -> Result<(String, usize)> {
    let (id, note, who, by) = from;
    // The note behind it is the pushing token's subject's, whatever source it claims.
    let held = HeldOp {
        write,
        reasons: vec![reason],
        op,
        notes: vec![json!({ "id": id, "app": note["app"], "title": note["title"], "source": inbox::trusted_source(note["source"].as_str().unwrap_or(""), ""), "by": by })],
        kind: "sync".into(),
    };
    wiki::with_lock(dir, || {
        crate::held::record(dir, batch, &local_iso(&now()), std::slice::from_ref(&held))?;
        crate::held::log_line(dir, "curator", &format!("Proposal from {who} waiting for your OK: {what}"))
    })?;
    Ok((batch.to_string(), 0))
}

/// How a removal from another machine went.
enum Removal {
    Removed,
    /// Nothing to remove: the page is gone already.
    Gone,
    /// The page moved on since the other machine's version.
    Moved,
}

/// Removes a page for another machine, if it is still the version that machine removed: the text
/// goes to .history/, the index is rebuilt, the log says who, and followers hear of it.
fn remove_page(dir: &Path, slug: &str, base: &str, who: &str) -> Result<Removal> {
    let r = wiki::with_lock(dir, || {
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
    })?;
    if matches!(r, Removal::Removed) {
        tombstone(dir, slug)?;
    }
    Ok(r)
}

/// Takes a push from another machine: each entry is a note from that machine's copy of the
/// space, by its id there. Returns the per-entry results and whether any was a conflict. `can`:
/// whether the caller holds a level in this space.
pub fn accept(dir: &Path, who: &Principal, entries: &[Value], can: &dyn Fn(Level) -> bool) -> (Vec<Value>, bool) {
    let name = who.name();
    let subject = match who {
        Principal::Local => "you".to_string(),
        Principal::Token { subject, .. } => subject.clone(),
    };
    let by = match who {
        Principal::Local => inbox::By::Local("sync"),
        Principal::Token { subject, .. } => inbox::By::Token(subject),
    };
    let mut out = vec![];
    let mut conflict = false;
    for e in entries {
        let id = e["id"].as_str().unwrap_or("");
        if !is_note_id(id) {
            out.push(refused(id, 400, "An entry needs its note id."));
            continue;
        }
        let note = &e["note"];
        let is_page = note["kind"] == "page";
        let mode = if is_page { note["page"]["mode"].as_str().unwrap_or("append") } else { "" };
        let need = if matches!(mode, "replace" | "delete") { Level::Write } else { Level::Append };
        if !can(need) {
            out.push(refused(id, 403, &format!("This token can't {} here.", need.as_str())));
            continue;
        }
        let slug = note["page"]["slug"].as_str().unwrap_or("").to_string();
        let base = note["page"]["base"].as_str().filter(|b| is_base(b));
        if mode == "delete" {
            // A page removed on the other machine: removed here too if it is still the version that
            // machine had, else a proposal.
            let (true, Some(base)) = (SLUG_RE.is_match(&slug), base.filter(|b| *b != "none")) else {
                out.push(refused(id, 400, "A removal names its page's slug and the hash of the version removed."));
                continue;
            };
            match remove_page(dir, &slug, base, &name) {
                Ok(Removal::Removed) => out.push(result(id, "accepted", vec![("removed", json!(slug))])),
                Ok(Removal::Gone) => out.push(result(id, "duplicate", vec![("removed", json!(slug))])),
                Ok(Removal::Moved) => match propose_removal(dir, &name, &subject, id, note, &slug, base) {
                    Ok((batch, index)) => {
                        conflict = true;
                        out.push(result(id, "conflict", vec![("current", json!(page_base(dir, &slug))), ("proposal", json!({ "batch": batch, "index": index }))]));
                    }
                    Err(err) => out.push(not_taken(id, &err)),
                },
                Err(err) => out.push(not_taken(id, &err)),
            }
            continue;
        }
        if let (true, Some(base)) = (mode == "replace", base) {
            let slug = if slug.is_empty() { wiki::slugify(note["title"].as_str().unwrap_or("")) } else { slug.clone() };
            let current = page_base(dir, &slug);
            if current != base {
                match propose(dir, &name, &subject, id, note, base, &current) {
                    Ok((batch, index)) => {
                        conflict = true;
                        out.push(result(id, "conflict", vec![("current", json!(current)), ("proposal", json!({ "batch": batch, "index": index }))]));
                    }
                    Err(err) => out.push(not_taken(id, &err)),
                }
                continue;
            }
        }
        let s = |k: &str| note[k].as_str().unwrap_or("").to_string();
        let input = NoteInput {
            kind: if is_page { "page".into() } else { "log".into() },
            source: s("source"),
            app: s("app"),
            title: s("title"),
            body: s("body"),
            tags: note.get("tags").cloned().unwrap_or(Value::Null),
            pages: note.get("pages").cloned().unwrap_or(Value::Null),
            page: is_page.then(|| PageInput {
                slug: Some(slug.clone()).filter(|s| !s.is_empty()),
                title: note["page"]["title"].as_str().map(String::from),
                kind: note["page"]["type"].as_str().map(String::from),
                summary: note["page"]["summary"].as_str().map(String::from),
                mode: Some(mode.to_string()).filter(|m| m == "replace"),
                base: base.map(String::from),
            }),
            // The same entry sent twice is one note: keyed by who sent it and its id there.
            idempotency_key: Some(format!("sync:{subject}:{id}")),
        };
        // Stamped as the pushing token's subject: its source can't be the curator's own user's.
        match inbox::submit_note(dir, &input, "sync", &format!("sync {subject}"), &by) {
            Ok(r) => out.push(result(id, if r.duplicate { "duplicate" } else { "accepted" }, vec![("note", json!(r.id))])),
            Err(err) => out.push(not_taken(id, &err)),
        }
    }
    (out, conflict)
}

// ---------------------------------------------------------------- the follower's side

/// One HTTP request to a peer through the system's curl (embed.rs says why), the token and body
/// on curl's stdin, never in its arguments. Ok((status, JSON body)).
fn request(method: &str, url: &str, token: &str, body: Option<&Value>, timeout: Duration) -> std::result::Result<(u16, Value), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut config = format!("header = \"Authorization: Bearer {token}\"\n");
    if let Some(b) = body {
        config.push_str(&format!("data-binary = \"{}\"\n", b.to_string().replace('\\', "\\\\").replace('"', "\\\"")));
    }
    let mut cmd = Command::new(crate::embed::curl());
    cmd.args(["-q", "-sS", "-X", method, "--max-time", &format!("{:.1}", timeout.as_secs_f64()), "-H", "Content-Type: application/json", "-H", "Accept: application/json", "-A"])
        .arg(format!("xenolith/{}", crate::VERSION))
        .args(["-w", "\n%{http_code}", "-K", "-"])
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(config.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("could not run curl: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (json_text, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let status: u16 = code.trim().parse().unwrap_or(0);
    if status == 0 {
        return Err(one_line(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok((status, serde_json::from_str(json_text).unwrap_or(Value::Null)))
}

/// Why a pass stopped short.
enum Stop {
    /// The curator refused this machine's token (401, or 403 for the whole request): nothing more
    /// is sent or taken until the token is fixed.
    Auth(String),
    /// The curator answered as another curator than the one this machine pinned.
    Curator(String),
    /// Anything else (unreachable, a server error, not granted): the next pass tries again.
    Other(String),
}

/// What a pass did.
#[derive(Default)]
struct Tally {
    pushed: usize,
    conflicts: usize,
    refused: usize,
    pulled: usize,
    removed: usize,
    skipped: usize,
    logs: usize,
    log_bytes: usize,
    errors: Vec<String>,
}

/// One pass over one followed space.
struct Pass<'a> {
    paths: &'a AppPaths,
    cfg: &'a Config,
    space: &'a SpaceId,
    peer: &'a str,
    url: String,
    token: String,
    timeout: Duration,
    dir: PathBuf,
    t: Tally,
}

fn why(v: &Value) -> String {
    one_line(v["error_description"].as_str().or(v["error"].as_str()).unwrap_or(""))
}

impl Pass<'_> {
    fn get(&self, rel: &str) -> std::result::Result<(u16, Value), String> {
        request("GET", &format!("{}/{rel}", self.url), &self.token, None, self.timeout)
    }

    fn auth(&self, status: u16, v: &Value) -> Stop {
        Stop::Auth(format!(
            "{} refused this machine's token ({status}{}): nothing was pushed or pulled. Ask it for a new token, and give it to `xenolith sync peer add {} <url>` on stdin.",
            self.peer,
            Some(why(v)).filter(|w| !w.is_empty()).map(|w| format!(": {w}")).unwrap_or_default(),
            self.peer
        ))
    }

    fn unexpected(&self, status: u16, v: &Value) -> Stop {
        if status == 421 {
            let at = v["curatorUrl"].as_str().map(|u| format!(" at {u}")).unwrap_or_default();
            return Stop::Other(format!("{} only follows {}; its curator is {}{at}: follow it from there", self.peer, self.space, v["curator"].as_str().unwrap_or("another machine")));
        }
        Stop::Other(format!("{} answered {status}: {}", self.peer, why(v)))
    }

    /// The curator's list, after checking it is the curator this machine pinned (pinning it at the
    /// first pull).
    fn list(&self) -> std::result::Result<Value, Stop> {
        let v = match self.get("pages") {
            Ok((200, v)) => v,
            Ok((s @ (401 | 403), v)) => return Err(self.auth(s, &v)),
            Ok((s, v)) => return Err(self.unexpected(s, &v)),
            Err(e) => return Err(Stop::Other(format!("{} could not be reached: {e}", self.peer))),
        };
        let space = self.space.to_string();
        let got = v["curator"].as_str().filter(|c| is_curator_id(c));
        let pinned = match (self.cfg.curators.get(&space), got) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(got)) => update(self.paths, |c| {
                // Pinned once, under the settings' lock: a pass that pinned meanwhile wins.
                if c.follow.contains_key(&space) && !c.curators.contains_key(&space) {
                    c.curators.insert(space.clone(), got.to_string());
                }
                Ok(c.curators.get(&space).cloned())
            })
            .map_err(|e| Stop::Other(format!("could not pin {}'s curator: {e}", self.space)))?,
            (None, None) => None, // a curator from before curator ids: nothing to pin
        };
        match (pinned, got) {
            (Some(p), Some(g)) if p == g => Ok(v),
            (None, _) => Ok(v),
            (Some(p), g) => Err(Stop::Curator(format!(
                "{} answers for {} as curator {}, but this machine follows curator {}: two curators for one space, or the curator was set up again. Nothing was pushed or pulled. If {} is the right curator now, follow the space again: xenolith sync follow {} {}",
                self.peer,
                self.space,
                g.map(|g| &g[..8]).unwrap_or("(no id)"),
                &p[..8],
                self.peer,
                self.space,
                self.peer
            ))),
        }
    }

    // ------------------------------------------------ push

    /// Sends the notes waiting in the copy: one batch, or every batch while the backlog is over
    /// DRAIN_ABOVE. Each note the curator took goes to .curator/sent/, each it refused to the dead
    /// letters; the rest stay for the next pass.
    fn push(&mut self) -> std::result::Result<(), Stop> {
        let notes: Vec<inbox::Note> = inbox::list_notes(&self.dir).unwrap_or_default().into_iter().filter(|n| is_note_id(&n.id)).collect();
        let take = if notes.len() > DRAIN_ABOVE { notes.len() } else { notes.len().min(MAX_ENTRIES) };
        for batch in notes[..take].chunks(MAX_ENTRIES) {
            self.send(batch)?;
        }
        Ok(())
    }

    fn send(&mut self, batch: &[inbox::Note]) -> std::result::Result<(), Stop> {
        let body = json!({ "entries": batch.iter().map(entry_of).collect::<Vec<_>>() });
        match request("POST", &format!("{}/entries", self.url), &self.token, Some(&body), self.timeout) {
            Ok((200 | 409, v)) => {
                let answered: HashSet<&str> = v["results"].as_array().into_iter().flatten().filter_map(|r| r["id"].as_str()).collect();
                for r in v["results"].as_array().into_iter().flatten() {
                    let id = r["id"].as_str().unwrap_or("");
                    if !batch.iter().any(|n| n.id == id) {
                        continue;
                    }
                    let moved = match r["result"].as_str() {
                        Some("accepted" | "duplicate") => {
                            self.t.pushed += 1;
                            mark_sent(&self.dir, id, self.peer, r)
                        }
                        Some("conflict") => {
                            self.t.conflicts += 1;
                            mark_sent(&self.dir, id, self.peer, r)
                        }
                        // Refused for good (403: not allowed; 400: invalid; a curator from before
                        // statuses says neither): a dead letter, never sent again.
                        Some("refused") if r["status"].as_u64().is_none_or(|s| s == 400 || s == 403) => {
                            self.t.refused += 1;
                            mark_dead(&self.dir, id, self.peer, &why(r))
                        }
                        // The curator's own trouble: sent again next pass.
                        _ => {
                            self.t.errors.push(format!("{} could not take note {id} yet: {}", self.peer, why(r)));
                            Ok(())
                        }
                    };
                    if let Err(e) = moved {
                        return Err(Stop::Other(format!("pushed, but could not move note {id}: {e}")));
                    }
                }
                let silent = batch.iter().filter(|n| !answered.contains(n.id.as_str())).count();
                if silent > 0 {
                    self.t.errors.push(format!("{} said nothing about {silent} note(s); they are sent again next pass", self.peer));
                }
                Ok(())
            }
            Ok((s @ (401 | 403), v)) => Err(self.auth(s, &v)),
            // The batch as a whole was invalid or too large: halve it until the entry at fault is
            // found, and dead-letter that one.
            Ok((400 | 413, _)) if batch.len() > 1 => {
                let (a, b) = batch.split_at(batch.len() / 2);
                self.send(a)?;
                self.send(b)
            }
            Ok((s @ (400 | 413), v)) => {
                self.t.refused += 1;
                let reason = format!("{s}: {}", why(&v));
                mark_dead(&self.dir, &batch[0].id, self.peer, &reason).map_err(|e| Stop::Other(format!("could not move note {}: {e}", batch[0].id)))
            }
            Ok((s, v)) => Err(self.unexpected(s, &v)),
            Err(e) => Err(Stop::Other(format!("{} could not be reached: {e}", self.peer))),
        }
    }

    // ------------------------------------------------ pull

    /// Takes the curator's pages, removals and recent log into the copy. A page changed here and not
    /// yet sent is left for the next pass, so the pull never drops a change unsent.
    fn pull(&mut self, list: &Value, st: &mut FollowState, baseline: bool, captured: &BTreeMap<String, String>) -> std::result::Result<(), Stop> {
        // Pages whose change still waits in the inbox (its push failed): not overwritten yet.
        let waiting: HashSet<String> = inbox::list_notes(&self.dir).unwrap_or_default().into_iter().filter_map(|n| n.page.and_then(|p| p["slug"].as_str().map(String::from))).collect();
        // A page may be overwritten when the copy has it as last pulled, or as the change this pass sent.
        let may_replace = |slug: &str, local: &str, st: &FollowState| {
            !waiting.contains(slug) && (!baseline || local == st.pulled.get(slug).map(String::as_str).unwrap_or("none") || captured.get(slug).map(String::as_str) == Some(local))
        };
        for p in list["pages"].as_array().into_iter().flatten() {
            let (Some(slug), Some(hash)) = (p["slug"].as_str().filter(|s| SLUG_RE.is_match(s)), p["hash"].as_str().filter(|h| is_base(h) && *h != "none")) else { continue };
            let local = page_base(&self.dir, slug);
            if local == hash {
                st.pulled.insert(slug.to_string(), hash.to_string());
                continue;
            }
            if !may_replace(slug, &local, st) {
                continue;
            }
            match self.get(&format!("pages/{slug}")) {
                Ok((200, page)) => {
                    let Some(text) = page["text"].as_str() else { continue };
                    if crate::secrets::find_secret(text).is_some() {
                        self.t.errors.push(format!("page {slug} holds what looks like a secret: not copied"));
                        continue; // the curator's own guard should have refused it; never copy one
                    }
                    match take_page(&self.dir, slug, text, &local) {
                        Ok(true) => {
                            self.t.pulled += 1;
                            st.pulled.insert(slug.to_string(), hash_text(text, 16));
                        }
                        Ok(false) => {}
                        Err(e) => self.t.errors.push(format!("could not write page {slug}: {e}")),
                    }
                }
                // Removed between the list and now: the next list says so.
                Ok((404, _)) => self.t.skipped += 1,
                Ok((s @ (401 | 403), v)) => return Err(self.auth(s, &v)),
                Ok((s, v)) => self.t.errors.push(format!("{} answered {s} for page {slug}: {}", self.peer, why(&v))),
                Err(e) => return Err(Stop::Other(format!("{} could not be reached: {e}", self.peer))),
            }
        }
        for slug in list["deleted"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|s| SLUG_RE.is_match(s)) {
            let local = page_base(&self.dir, slug);
            if local != "none" {
                if !may_replace(slug, &local, st) {
                    continue;
                }
                match remove_local(&self.dir, slug, &local) {
                    Ok(true) => self.t.removed += 1,
                    Ok(false) => continue,
                    Err(e) => {
                        self.t.errors.push(format!("could not remove page {slug}: {e}"));
                        continue;
                    }
                }
            }
            st.pulled.remove(slug);
        }
        let mut dates = BTreeSet::new();
        for l in list["log"].as_array().into_iter().flatten() {
            let (Some(date), Some(hash)) = (l["date"].as_str().filter(|d| DATE_RE.is_match(d)), l["hash"].as_str().filter(|h| is_base(h) && *h != "none")) else { continue };
            dates.insert(date.to_string());
            if st.logs.get(date).map(String::as_str) == Some(hash) {
                continue;
            }
            match self.get(&format!("log/{date}")) {
                Ok((200, day)) => {
                    let Some(text) = day["text"].as_str() else { continue };
                    if crate::secrets::find_secret(text).is_some() {
                        continue;
                    }
                    match merge_log(&self.dir, date, text) {
                        Ok(()) => {
                            self.t.logs += 1;
                            self.t.log_bytes += text.len();
                            st.logs.insert(date.to_string(), hash_text(text, 16));
                        }
                        Err(e) => self.t.errors.push(format!("could not write the log of {date}: {e}")),
                    }
                }
                Ok((404, _)) => self.t.skipped += 1,
                Ok((s @ (401 | 403), v)) => return Err(self.auth(s, &v)),
                Ok((s, v)) => self.t.errors.push(format!("{} answered {s} for the log of {date}: {}", self.peer, why(&v))),
                Err(e) => return Err(Stop::Other(format!("{} could not be reached: {e}", self.peer))),
            }
        }
        st.logs.retain(|d, _| dates.contains(d));
        Ok(())
    }
}

/// A note waiting in a followed space, as an entry for its curator.
fn entry_of(n: &inbox::Note) -> Value {
    let mut note = json!({ "kind": n.kind, "app": n.app, "source": n.source, "title": n.title, "body": n.body, "tags": n.tags, "pages": n.pages });
    if let Some(p) = &n.page {
        note["page"] = p.clone();
    }
    json!({ "id": n.id, "note": note })
}

/// Moves a note out of the inbox to `to`, with a stamp saying what became of it.
fn move_note(dir: &Path, id: &str, to: &Path, stamp: &str) -> Result<()> {
    let p = curator_paths(dir);
    let from = wiki::confined_path(dir, &p.inbox.join(format!("{id}.md")))?;
    let Some(text) = wiki::read_if_exists(&from)? else { return Ok(()) };
    let to = wiki::confined_path(dir, to)?;
    fs::create_dir_all(to.parent().unwrap_or(dir))?;
    // A comment can't end early: "-->" in what a peer said would close it.
    let content = format!("<!-- {} -->\n{text}", stamp.replace("-->", "- ->"));
    // Written whole (a temp file renamed), but without an fsync: the curator has the note already,
    // so a crash at worst loses this local record of it, or leaves the note in the inbox to be sent
    // again and taken as a duplicate. A backlog of thousands moves several times faster, as an
    // fsync is slow (a full flush on macOS).
    let tmp = to.with_file_name(format!("{id}.{}.tmp", random_hex(6)));
    fs::write(&tmp, content)?;
    // On Windows a reader or an antivirus scan may hold either file for a moment.
    if let Err(e) = wiki::retrying(|| fs::rename(&tmp, &to)) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    wiki::retrying(|| fs::remove_file(&from))?;
    inbox::clear_state(dir, id);
    Ok(())
}

/// Moves a pushed note out of the inbox into .curator/sent/, with what the curator said about it.
fn mark_sent(dir: &Path, id: &str, peer: &str, r: &Value) -> Result<()> {
    let to = curator_paths(dir).cur.join("sent").join(inbox::month_dir(id)).join(format!("{id}.md"));
    move_note(dir, id, &to, &format!("sent to {peer} at {}: {r}", local_iso(&now())))
}

/// Moves a note the curator refused for good into .curator/sync/dead/, with the reason.
fn mark_dead(dir: &Path, id: &str, peer: &str, reason: &str) -> Result<()> {
    move_note(dir, id, &dead_dir(dir).join(format!("{id}.md")), &format!("refused by {peer} at {}: {}", local_iso(&now()), one_line(reason)))
}

/// How many notes the curators refused for good in this copy.
pub fn dead_count(dir: &Path) -> usize {
    fs::read_dir(dead_dir(dir)).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count()
}

/// Writes the curator's version of a page into the copy, if the copy still has `expect` ("none":
/// no page), keeping what it replaces in .history/.
fn take_page(dir: &Path, slug: &str, text: &str, expect: &str) -> Result<bool> {
    let file = page_file(dir, slug)?;
    wiki::with_lock(dir, || {
        let cur = wiki::read_if_exists(&file)?;
        if cur.as_deref().map(|c| hash_text(c, 16)).unwrap_or_else(|| "none".into()) != expect {
            return Ok(false); // changed meanwhile: the next pass sends it
        }
        if let Some(c) = &cur {
            keep_history(dir, slug, c, "sync")?;
        }
        fs::create_dir_all(dir.join("pages"))?;
        let want = cur.as_deref().map(|c| hash_text(c, 16));
        wiki::atomic_write(&file, text, None, Some(want.as_deref()))?;
        wiki::refresh_index(dir, None)?;
        Ok(true)
    })
}

/// Removes a page the curator removed, if the copy still has `expect`, keeping it in .history/.
fn remove_local(dir: &Path, slug: &str, expect: &str) -> Result<bool> {
    let file = page_file(dir, slug)?;
    wiki::with_lock(dir, || {
        let Some(cur) = wiki::read_if_exists(&file)? else { return Ok(false) };
        if hash_text(&cur, 16) != expect {
            return Ok(false);
        }
        keep_history(dir, slug, &cur, "removed")?;
        fs::remove_file(&file)?;
        crate::readcache::forget(&file);
        wiki::refresh_index(dir, None)?;
        Ok(true)
    })
}

/// Writes the curator's log of a day into the copy. Lines only the copy has (written here
/// directly) stay, after the curator's.
fn merge_log(dir: &Path, date: &str, theirs: &str) -> Result<()> {
    let file = log_file(dir, date)?;
    let local = wiki::read_if_exists(&file)?;
    let text = match &local {
        None => theirs.to_string(),
        Some(mine) => {
            let known: HashSet<&str> = theirs.lines().map(str::trim).collect();
            let own: Vec<&str> = mine.lines().filter(|l| !l.trim().is_empty() && !known.contains(l.trim())).collect();
            if own.is_empty() { theirs.to_string() } else { format!("{}\n\n{}\n", theirs.trim_end(), own.join("\n")) }
        }
    };
    if local.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    wiki::with_lock(dir, || wiki::atomic_write(&file, &text, None, None))
}

/// What a follower remembers between passes: the hash of each page as last pulled, and of each
/// day of log. Kept in .curator/sync/state.json, and checked like any file in the wiki folder.
#[derive(Default)]
struct FollowState {
    pulled: BTreeMap<String, String>,
    logs: BTreeMap<String, String>,
}

fn state_file(dir: &Path) -> PathBuf {
    sync_dir(dir).join("state.json")
}

impl FollowState {
    /// None before the first pull: then there is nothing to tell a local edit by.
    fn load(dir: &Path) -> Option<FollowState> {
        let v = read_json(dir, &state_file(dir))?;
        let map = |k: &str, key_ok: &dyn Fn(&str) -> bool| -> BTreeMap<String, String> {
            v[k].as_object().into_iter().flatten().filter_map(|(a, b)| b.as_str().filter(|h| key_ok(a) && is_base(h) && *h != "none").map(|h| (a.clone(), h.to_string()))).collect()
        };
        Some(FollowState { pulled: map("pulled", &|s| SLUG_RE.is_match(s)), logs: map("logs", &|d| DATE_RE.is_match(d)) })
    }

    fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(sync_dir(dir))?;
        write_json(dir, &state_file(dir), &json!({ "v": 1, "pulled": self.pulled, "logs": self.logs }))
    }
}

/// Pages changed in the copy since the last pull become notes for the curator: a replace against
/// the version last pulled ("none" for a page made here), or a removal. Returns slug → the copy's
/// hash each note was made from ("none" for a removal), so the pull knows the change was sent.
fn queue_local_changes(dir: &Path, st: &FollowState, machine: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let mut captured = BTreeMap::new();
    let mut problems = vec![];
    let by = inbox::By::Local("sync");
    let mut submit = |slug: &str, input: NoteInput, hash: &str| match inbox::submit_note(dir, &input, "sync", "sync (a change made in this copy)", &by) {
        Ok(_) => {
            captured.insert(slug.to_string(), hash.to_string());
        }
        Err(e) => problems.push(format!("page {slug} changed here but can't be sent: {e}")),
    };
    let local: BTreeSet<String> = wiki::list_pages(dir).unwrap_or_default().into_iter().map(|p| p.slug).filter(|s| SLUG_RE.is_match(s)).collect();
    for slug in &local {
        let Some(text) = page_file(dir, slug).ok().and_then(|f| wiki::read_if_exists(&f).ok().flatten()) else { continue };
        let hash = hash_text(&text, 16);
        let base = st.pulled.get(slug).map(String::as_str).unwrap_or("none");
        if hash == base {
            continue;
        }
        let (meta, body) = crate::frontmatter::parse(&text);
        let title = Some(meta.str("title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| slug.clone());
        let input = NoteInput {
            kind: "page".into(),
            source: "agent".into(),
            app: "sync".into(),
            title: title.clone(),
            body: if body.trim().is_empty() { format!("# {title}") } else { body },
            tags: meta.get("tags").map(|t| json!(wiki::norm_tags_val(Some(t)))).unwrap_or(Value::Null),
            pages: json!([slug]),
            page: Some(PageInput {
                slug: Some(slug.clone()),
                title: Some(title),
                kind: Some(meta.str("type")).filter(|t| !t.is_empty()),
                summary: Some(meta.str("summary")).filter(|s| !s.is_empty()),
                mode: Some("replace".into()),
                base: Some(base.to_string()),
            }),
            // One note per change: the same edit seen again is the note already made.
            idempotency_key: Some(format!("sync-edit:{slug}:{base}:{hash}")),
        };
        submit(slug, input, &hash);
    }
    for (slug, base) in st.pulled.iter().filter(|(s, _)| !local.contains(*s)) {
        let input = NoteInput {
            kind: "page".into(),
            source: "agent".into(),
            app: "sync".into(),
            title: format!("Remove {slug}"),
            body: format!("Removed from the copy on {machine}."),
            tags: Value::Null,
            pages: json!([slug]),
            page: Some(PageInput { slug: Some(slug.clone()), mode: Some("delete".into()), base: Some(base.clone()), ..Default::default() }),
            idempotency_key: Some(format!("sync-remove:{slug}:{base}")),
        };
        submit(slug, input, "none");
    }
    (captured, problems)
}

fn status_file(dir: &Path) -> PathBuf {
    sync_dir(dir).join("status.json")
}

/// One sync of one followed space: check the curator, send what changed here, push, then pull.
pub fn sync_space(paths: &AppPaths, root: &Path, cfg: &Config, space: &SpaceId) -> Value {
    let Some(peer) = cfg.curator_of(space) else { return json!({ "space": space.to_string(), "error": "not followed" }) };
    let report = |extra: Value| {
        let mut o = json!({ "space": space.to_string(), "peer": peer });
        if let (Some(a), Some(b)) = (o.as_object_mut(), extra.as_object()) {
            a.extend(b.clone());
        }
        o
    };
    let Some(base) = cfg.peers.get(peer) else { return report(json!({ "error": format!("no peer named {peer}") })) };
    let Some(token) = read_token(paths, peer) else { return report(json!({ "error": format!("no token for {peer}: xenolith sync peer add {peer} <url> takes it on stdin") })) };
    let dir = match spaces::create(root, space) {
        Ok(d) => d,
        Err(e) => return report(json!({ "error": e.to_string() })),
    };
    // One pass per space at a time: the curator's loop and `xenolith sync` may both start one.
    let opts = crate::lock::LockOpts { timeout_ms: 2_000, max_hold_ms: 30 * 60_000, label: crate::lock::label() };
    let mut guard = match crate::lock::acquire(&dir, "sync", &opts) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return report(json!({ "error": format!("another sync of {space} is running ({})", b.0) })),
        Err(crate::lock::AcquireError::Io(e)) => return report(json!({ "error": e.to_string() })),
    };
    let who = cfg.machine.clone().map(|m| format!("machine:{m}")).unwrap_or_else(|| "this machine".into());
    let mut pass = Pass { paths, cfg, space, peer, url: space_url(base, space), token, timeout: cfg.timeout(), dir: dir.clone(), t: Tally::default() };
    let outcome = run_pass(&mut pass, &who);
    guard.release();
    let t = &pass.t;
    let (error, kind) = match &outcome {
        Ok(()) => (None, None),
        Err(Stop::Auth(e)) => (Some(e.clone()), Some("auth")),
        Err(Stop::Curator(e)) => (Some(e.clone()), Some("curator")),
        Err(Stop::Other(e)) => (Some(e.clone()), Some("other")),
    };
    let waiting = inbox::list_notes(&dir).map(|n| n.len()).unwrap_or(0);
    let mut r = report(json!({
        "pushed": t.pushed, "conflicts": t.conflicts, "refused": t.refused, "pulled": t.pulled, "removed": t.removed,
        "skipped": t.skipped, "logs": t.logs, "logBytes": t.log_bytes, "waiting": waiting, "dead": dead_count(&dir), "at": now_ms(),
    }));
    if let Some(e) = &error {
        r["error"] = json!(e);
        r["errorKind"] = json!(kind);
    }
    if !t.errors.is_empty() {
        r["problems"] = json!(t.errors);
    }
    if t.pushed + t.conflicts + t.refused + t.pulled + t.removed > 0 {
        crate::audit::best_effort(
            paths,
            "sync",
            "you",
            vec![
                ("space", json!(space.to_string())),
                ("peer", json!(peer)),
                ("as", json!(who)),
                ("pushed", json!(t.pushed)),
                ("conflicts", json!(t.conflicts)),
                ("refused", json!(t.refused)),
                ("pulled", json!(t.pulled)),
                ("removed", json!(t.removed)),
            ],
        );
    }
    let _ = fs::create_dir_all(sync_dir(&dir)).map_err(wiki::Error::from).and_then(|_| write_json(&dir, &status_file(&dir), &r));
    r
}

fn run_pass(pass: &mut Pass, who: &str) -> std::result::Result<(), Stop> {
    // 1. Who curates it, before anything is sent: a list from another curator stops the pass.
    let mut list = match pass.list() {
        Ok(v) => Some(v),
        Err(Stop::Other(e)) => {
            pass.t.errors.push(e);
            None
        }
        Err(stop) => return Err(stop),
    };
    // 2. Pages changed here since the last pull become notes.
    let state = FollowState::load(&pass.dir);
    let baseline = state.is_some();
    let mut st = state.unwrap_or_default();
    let (captured, problems) = if baseline { queue_local_changes(&pass.dir, &st, who) } else { Default::default() };
    pass.t.errors.extend(problems);
    // 3. Push. Whatever happens, unless the token was refused, the pull runs.
    let pushed = pass.push();
    let push_error = match pushed {
        Ok(()) => None,
        Err(Stop::Other(e)) => Some(e),
        Err(stop) => return Err(stop),
    };
    // 4. Pull: listed again when the push may have changed what the curator has.
    if list.is_none() || pass.t.pushed + pass.t.conflicts > 0 {
        match pass.list() {
            Ok(v) => list = Some(v),
            Err(Stop::Other(e)) => return Err(Stop::Other(push_error.map(|p| format!("push: {p}; pull: {e}")).unwrap_or(e))),
            Err(stop) => return Err(stop),
        }
    }
    let pulled = pass.pull(list.as_ref().unwrap_or(&Value::Null), &mut st, baseline, &captured);
    if let Err(e) = st.save(&pass.dir) {
        pass.t.errors.push(format!("could not save what was pulled: {e}"));
    }
    pulled?;
    match push_error {
        Some(e) => Err(Stop::Other(format!("push: {e}"))),
        None => Ok(()),
    }
}

/// Syncs every followed space; one report each.
pub fn sync_all(paths: &AppPaths, root: &Path) -> Vec<Value> {
    let cfg = Config::load(paths);
    cfg.follow.keys().filter_map(|s| SpaceId::parse(s).ok()).map(|s| sync_space(paths, root, &cfg, &s)).collect()
}

/// Every followed space as /status and `xenolith sync status` show it: its curator, the notes
/// waiting, the dead letters, and the last pass.
pub fn status(paths: &AppPaths, root: &Path) -> Value {
    let cfg = Config::load(paths);
    let mut dead = 0;
    let spaces: Vec<Value> = cfg
        .follow
        .iter()
        .filter_map(|(s, peer)| {
            let id = SpaceId::parse(s).ok()?;
            let dir = spaces::space_dir(root, &id);
            let d = dead_count(&dir);
            dead += d;
            let last = read_json(&dir, &status_file(&dir))
                .filter(|v| v.is_object())
                .map(|v| json!({ "at": v["at"], "pushed": v["pushed"], "pulled": v["pulled"], "error": v["error"], "errorKind": v["errorKind"] }));
            Some(json!({
                "space": s, "peer": peer, "curator": cfg.curators.get(s), "dead": d,
                "waiting": inbox::list_notes(&dir).map(|n| n.len()).unwrap_or(0), "last": last,
            }))
        })
        .collect();
    json!({ "dead": dead, "spaces": spaces })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_paths() -> (PathBuf, AppPaths) {
        let d = std::env::temp_dir().join(format!("xenolith-sync-{}", random_hex(8)));
        fs::create_dir_all(&d).unwrap();
        let mut env = crate::paths::Env::new();
        env.insert("XENOLITH_HOME".into(), d.join("home").to_string_lossy().into());
        (d.clone(), AppPaths::from_env(&env))
    }

    #[test]
    fn names_and_urls_are_checked() {
        assert!(valid_name("studio") && valid_name("laptop-2") && !valid_name("Studio") && !valid_name("") && !valid_name("../x"));
        assert!(valid_url("https://box.example/xenolith") && valid_url("http://127.0.0.1:47850") && valid_url("http://localhost:1/"));
        assert!(!valid_url("http://box.example") && !valid_url("ftp://x") && !valid_url("https://x?token=1"));
    }

    #[test]
    fn a_base_with_a_path_keeps_it() {
        let atlas = SpaceId::parse("project:atlas").unwrap();
        assert_eq!(space_url("https://box.example/xenolith/", &atlas), "https://box.example/xenolith/sync/v1/spaces/project/atlas");
        assert_eq!(space_url("http://127.0.0.1:47840/api/svc/studio/xenolith", &atlas), "http://127.0.0.1:47840/api/svc/studio/xenolith/sync/v1/spaces/project/atlas");
        assert_eq!(space_url("http://127.0.0.1:47850", &atlas), "http://127.0.0.1:47850/sync/v1/spaces/project/atlas");
    }

    #[test]
    fn the_config_keeps_only_what_it_can_use() {
        let (d, paths) = tmp_paths();
        update(&paths, |c| {
            c.machine = Some("studio".into());
            c.peers.insert("laptop".into(), "https://laptop.example/xenolith".into());
            c.follow(&SpaceId::parse("project:atlas").unwrap(), "laptop");
            c.curators.insert("project:atlas".into(), "0123456789abcdef0123456789abcdef".into());
            c.timeout_seconds = Some(120);
            Ok(())
        })
        .unwrap();
        let mut raw: Value = serde_json::from_str(&fs::read_to_string(config_file(&paths)).unwrap()).unwrap();
        assert_eq!(raw["follow"]["project:atlas"], json!({ "peer": "laptop", "curator": "0123456789abcdef0123456789abcdef" }));
        raw["peers"]["bad"] = json!("http://far.example");
        raw["follow"]["machine:local"] = json!("laptop");
        raw["follow"]["project:old"] = json!("laptop"); // sync.json before curator ids
        raw["follow"]["project:odd"] = json!({ "peer": "laptop", "curator": "not-an-id" });
        fs::write(config_file(&paths), raw.to_string()).unwrap();
        let c = Config::load(&paths);
        assert_eq!(c.peers.keys().collect::<Vec<_>>(), ["laptop"], "a clear-text URL to another machine is dropped");
        assert_eq!(c.follow.keys().collect::<Vec<_>>(), ["project:atlas", "project:odd", "project:old"], "this machine's own space is never followed");
        assert_eq!(c.curators.keys().collect::<Vec<_>>(), ["project:atlas"], "only a real curator id is pinned");
        assert_eq!(c.curator_of(&SpaceId::parse("project:atlas").unwrap()), Some("laptop"));
        assert_eq!(c.timeout(), Duration::from_secs(120));
        assert_eq!(Config::default().timeout(), Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        let mut again = c.clone();
        again.follow(&SpaceId::parse("project:atlas").unwrap(), "laptop");
        assert!(again.curators.is_empty(), "following again forgets the pinned curator");
        save_token(&paths, "laptop", "xenolith_x\n").unwrap();
        assert_eq!(read_token(&paths, "laptop").as_deref(), Some("xenolith_x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(token_file(&paths, "laptop")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn notes_are_taken_once_and_a_stale_replace_becomes_a_proposal() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        fs::write(dir.join("pages/db.md"), "---\ntitle: DB\n---\n\n# DB\n\nPostgres 16.\n").unwrap();
        let who = Principal::Token { id: "tok_x".into(), subject: "machine:laptop".into(), scopes: vec![] };
        let all = |_: Level| true;
        let id = inbox::new_note_id(&now());
        let note = json!({ "id": id, "note": { "kind": "log", "app": "codex", "source": "agent", "title": "Moved to 17", "body": "Postgres 17 now." } });
        let (r, conflict) = accept(&dir, &who, std::slice::from_ref(&note), &all);
        assert!(!conflict);
        assert_eq!(r[0]["result"], "accepted");
        let (again, _) = accept(&dir, &who, &[note], &all);
        assert_eq!((again[0]["result"].as_str(), again[0]["note"].as_str()), (Some("duplicate"), r[0]["note"].as_str()), "the same entry twice is one note");

        let fresh = page_base(&dir, "db");
        let ok =
            json!({ "id": inbox::new_note_id(&now()), "note": { "kind": "page", "app": "codex", "title": "DB", "body": "Postgres 17.", "page": { "slug": "db", "mode": "replace", "base": fresh } } });
        assert_eq!(accept(&dir, &who, &[ok], &all).0[0]["result"], "accepted", "a replace against the current page is a note for the curator");
        let stale_id = inbox::new_note_id(&now());
        let stale =
            json!({ "id": stale_id, "note": { "kind": "page", "app": "codex", "title": "DB", "body": "Postgres 15.", "page": { "slug": "db", "mode": "replace", "base": "0123456789abcdef" } } });
        let (r, conflict) = accept(&dir, &who, &[stale], &all);
        assert!(conflict);
        assert_eq!(r[0]["result"], "conflict");
        assert_eq!(r[0]["current"], json!(fresh));
        let held = crate::held::list(&dir);
        let pending = held.iter().find(|h| h["batch"] == r[0]["proposal"]["batch"]).expect("the proposal is a held change");
        assert_eq!(pending["kind"], "sync");
        assert!(pending["reasons"][0].as_str().unwrap().contains("machine:laptop (tok_x) wrote this against version 0123456789abcdef"));
        assert_eq!(crate::held::auto_apply_all(&dir).unwrap(), 0, "a proposal waits for the person even when approvals are automatic");
        assert!(fs::read_to_string(dir.join("pages/db.md")).unwrap().contains("Postgres 16."), "the page is untouched");

        let only_append = |l: Level| l <= Level::Append;
        let replace = json!({ "id": inbox::new_note_id(&now()), "note": { "kind": "page", "app": "codex", "title": "DB", "body": "x", "page": { "slug": "db", "mode": "replace", "base": fresh } } });
        let r = accept(&dir, &who, &[replace], &only_append).0;
        assert_eq!((r[0]["result"].as_str(), r[0]["status"].as_u64()), (Some("refused"), Some(403)), "a replace needs write: refused for good");
        let r = accept(&dir, &who, &[json!({ "id": "nope", "note": {} })], &all).0;
        assert_eq!((r[0]["result"].as_str(), r[0]["status"].as_u64()), (Some("refused"), Some(400)), "an invalid entry is refused for good");
        let empty = json!({ "id": inbox::new_note_id(&now()), "note": { "kind": "log", "app": "codex", "title": "", "body": "x" } });
        assert_eq!(accept(&dir, &who, &[empty], &all).0[0]["status"], 400, "a note the inbox refuses is invalid");
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn a_removal_applies_to_the_version_removed_and_is_a_proposal_otherwise() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        fs::write(dir.join("pages/old.md"), "---\ntitle: Old\n---\n\n# Old\n\nGone soon.\n").unwrap();
        fs::write(dir.join("pages/kept.md"), "---\ntitle: Kept\n---\n\n# Kept\n\nv1\n").unwrap();
        let first = page_list(&dir).unwrap();
        assert!(is_curator_id(first["curator"].as_str().unwrap()));
        assert_eq!(first["deleted"], json!([]));
        let who = Principal::Token { id: "tok_x".into(), subject: "machine:laptop".into(), scopes: vec![] };
        let all = |_: Level| true;
        let remove = |slug: &str, base: &str| json!({ "id": inbox::new_note_id(&now()), "note": { "kind": "page", "app": "sync", "title": format!("Remove {slug}"), "body": "x", "page": { "slug": slug, "mode": "delete", "base": base } } });

        let only_append = |l: Level| l <= Level::Append;
        assert_eq!(accept(&dir, &who, &[remove("old", &page_base(&dir, "old"))], &only_append).0[0]["status"], 403, "a removal needs write");
        let (r, conflict) = accept(&dir, &who, &[remove("old", &page_base(&dir, "old"))], &all);
        assert!(!conflict);
        assert_eq!((r[0]["result"].as_str(), r[0]["removed"].as_str()), (Some("accepted"), Some("old")));
        assert!(!dir.join("pages/old.md").exists());
        assert!(fs::read_dir(dir.join(".history/pages/old")).unwrap().count() == 1, "the removed page is kept in .history/");
        assert!(wiki::recent_log(&dir, 1, 10_000).contains("Removed Old [[old]], as machine:laptop (tok_x) did"));
        let list = page_list(&dir).unwrap();
        assert_eq!(list["deleted"], json!(["old"]), "followers hear of it");
        assert_eq!(list["curator"], first["curator"], "the curator id is made once");
        assert_eq!(accept(&dir, &who, &[remove("old", "0123456789abcdef")], &all).0[0]["result"], "duplicate", "a page removed already");

        let stale = remove("kept", "0123456789abcdef");
        let (r, conflict) = accept(&dir, &who, &[stale], &all);
        assert!(conflict);
        assert_eq!(r[0]["result"], "conflict");
        assert!(dir.join("pages/kept.md").exists(), "a stale removal waits");
        let held = crate::held::list(&dir);
        let p = held.iter().find(|h| h["batch"] == r[0]["proposal"]["batch"]).unwrap();
        assert_eq!((p["kind"].as_str(), p["action"].as_str()), (Some("sync"), Some("removed")));
        let done = crate::held::decide(&dir, p["batch"].as_str().unwrap(), 0, "approve", "owner").unwrap();
        assert_eq!(done["changed"], true);
        assert!(!dir.join("pages/kept.md").exists(), "approving the proposal removes the page");
        assert_eq!(page_list(&dir).unwrap()["deleted"], json!(["kept", "old"]));
        crate::held::undo(&dir, p["batch"].as_str().unwrap(), 0).unwrap();
        assert!(fs::read_to_string(dir.join("pages/kept.md")).unwrap().contains("v1"), "and undoing it puts the page back");
        assert_eq!(page_list(&dir).unwrap()["deleted"], json!(["old"]), "a page back is no longer a removal");

        // A page removed by hand is a removal too; one that only failed to list is not.
        fs::remove_file(dir.join("pages/kept.md")).unwrap();
        assert_eq!(page_list(&dir).unwrap()["deleted"], json!(["kept", "old"]));
        assert_eq!(accept(&dir, &who, &[remove("Bad Slug", "0123456789abcdef")], &all).0[0]["status"], 400);
        assert_eq!(accept(&dir, &who, &[remove("kept", "none")], &all).0[0]["status"], 400, "a removal names the version it removed");
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn tombstones_last_thirty_days() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        fs::write(dir.join("pages/a.md"), "# A\n").unwrap();
        page_list(&dir).unwrap();
        fs::remove_file(dir.join("pages/a.md")).unwrap();
        assert_eq!(page_list(&dir).unwrap()["deleted"], json!(["a"]));
        let mut m = load_manifest(&dir);
        m.deleted.insert("a".into(), now_ms() - (TOMBSTONE_DAYS + 1) * 86_400_000);
        m.deleted.insert("../x".into(), now_ms());
        write_json(&dir, &manifest_file(&dir), &serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(page_list(&dir).unwrap()["deleted"], json!([]), "older than 30 days, or not a slug: forgotten");
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn local_changes_become_notes_once() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        let v1 = "---\ntitle: DB\ntype: project\n---\n\n# DB\n\nPostgres 16.\n";
        fs::write(dir.join("pages/db.md"), v1).unwrap();
        fs::write(dir.join("pages/gone.md"), "# Gone\n").unwrap();
        let mut st = FollowState::default();
        st.pulled.insert("db".into(), hash_text(v1, 16));
        st.pulled.insert("gone".into(), page_base(&dir, "gone"));
        assert!(queue_local_changes(&dir, &st, "machine:laptop").0.is_empty(), "nothing changed here");

        fs::write(dir.join("pages/db.md"), "---\ntitle: DB\ntype: project\n---\n\n# DB\n\nPostgres 17, here.\n").unwrap();
        fs::remove_file(dir.join("pages/gone.md")).unwrap();
        fs::write(dir.join("pages/new.md"), "---\ntitle: New\n---\n\n# New\n\nMade here.\n").unwrap();
        let (captured, problems) = queue_local_changes(&dir, &st, "machine:laptop");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(captured.keys().collect::<Vec<_>>(), ["db", "gone", "new"]);
        assert_eq!(captured["gone"], "none");
        let notes = inbox::list_notes(&dir).unwrap();
        let page = |slug: &str| notes.iter().find(|n| n.page.as_ref().is_some_and(|p| p["slug"] == slug)).unwrap().page.clone().unwrap();
        assert_eq!((page("db")["mode"].as_str(), page("db")["base"].as_str()), (Some("replace"), Some(hash_text(v1, 16).as_str())));
        assert_eq!((page("gone")["mode"].as_str(), page("gone")["base"].as_str()), (Some("delete"), st.pulled.get("gone").map(String::as_str)));
        assert_eq!(page("new")["base"], "none", "a page made here is new to the curator");
        let entry = entry_of(notes.iter().find(|n| n.page.as_ref().is_some_and(|p| p["slug"] == "db")).unwrap());
        assert!(entry["note"]["body"].as_str().unwrap().contains("Postgres 17, here."));
        assert_eq!(entry["note"]["source"], "agent");
        queue_local_changes(&dir, &st, "machine:laptop");
        assert_eq!(inbox::list_notes(&dir).unwrap().len(), 3, "the same changes again are the same notes");
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn a_pulled_log_keeps_the_lines_only_this_copy_has() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        let theirs = "# 2026-10-22\n\n- 09:00 · curator · Filed a note [[db]]\n";
        merge_log(&dir, "2026-10-22", theirs).unwrap();
        assert_eq!(fs::read_to_string(dir.join("log/2026/2026-10-22.md")).unwrap(), theirs);
        fs::write(dir.join("log/2026/2026-10-22.md"), format!("{theirs}- 09:30 · codex · Written here\n")).unwrap();
        let newer = format!("{theirs}- 10:00 · curator · Filed another\n");
        merge_log(&dir, "2026-10-22", &newer).unwrap();
        assert_eq!(fs::read_to_string(dir.join("log/2026/2026-10-22.md")).unwrap(), format!("{}\n\n- 09:30 · codex · Written here\n", newer.trim_end()));
        crate::lock::remove_test_dir(&d);
    }

    #[test]
    fn a_refused_note_becomes_a_dead_letter() {
        let (d, _paths) = tmp_paths();
        let dir = d.join("wiki");
        wiki::ensure_wiki(&dir).unwrap();
        let input = NoteInput { kind: "log".into(), app: "codex".into(), title: "x".into(), body: "y".into(), ..Default::default() };
        let id = inbox::submit_note(&dir, &input, "", "", &inbox::By::Local("t")).unwrap().id;
        mark_dead(&dir, &id, "studio", "403: This token can't write here. -->").unwrap();
        assert_eq!(dead_count(&dir), 1);
        assert!(inbox::list_notes(&dir).unwrap().is_empty());
        let text = fs::read_to_string(dead_dir(&dir).join(format!("{id}.md"))).unwrap();
        assert!(text.starts_with("<!-- refused by studio at ") && text.contains("can't write here. - -> -->\n---\n"), "{text}");
        let (rel, read) = inbox::read_note(&dir, &id).unwrap();
        assert_eq!(rel, format!(".curator/sync/dead/{id}.md"), "note:<id> still opens it");
        assert!(read.starts_with("Refused for good by the space's curator"), "{read}");
        let sent =
            inbox::submit_note(&dir, &NoteInput { kind: "log".into(), app: "codex".into(), title: "z".into(), body: "w".into(), ..Default::default() }, "", "", &inbox::By::Local("t")).unwrap().id;
        mark_sent(&dir, &sent, "studio", &json!({ "id": sent, "result": "accepted" })).unwrap();
        assert!(inbox::read_note(&dir, &sent).unwrap().1.starts_with("Sent to the space's curator"));
        crate::lock::remove_test_dir(&d);
    }
}
