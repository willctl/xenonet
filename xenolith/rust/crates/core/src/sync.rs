//! Entry sync between machines, with one curator per space.
//!
//! Every space has one curator: the machine whose Xenolith files its notes into pages. A space
//! created here is curated here. Another machine can follow it: it keeps a copy of the space,
//! where its sessions read the pages and leave notes as usual, and its own curator leaves the
//! space alone. `xenolith sync` (and the curator process, once a minute) then
//!
//! 1. pushes the notes waiting in each followed space to the space's curator, as entries, and
//!    moves each one the curator took to .curator/sent/; and
//! 2. pulls the curator's pages back, so the copy shows what the curator filed.
//!
//! The curator's HTTP service takes entries at /sync/v1/spaces/<kind>/<name>/entries, with a token
//! whose subject is the other machine (machine:<name>): a note needs append there, a page replaced
//! needs write. A page replaced against an older version of the page than the curator has (the
//! hash the session saw, recorded as the note's base) is not filed: the curator answers 409, and
//! the write becomes a proposal, a held change of kind "sync" that waits for the person's OK in
//! the window even when approvals are automatic. Nothing written on either machine is lost.
//!
//! Who follows what is sync.json in the config folder ({ machine, peers, follow }); the token for
//! each peer is peers/<name>.token there, readable by you alone. The peers' URLs are the base of
//! their Xenolith service: http://127.0.0.1:<port> on one machine, or an https:// address that
//! forwards to it (XenoNet's gateway, or a tunnel of your own).

use crate::access::{Level, Principal};
use crate::curator::HeldOp;
use crate::inbox::{self, NoteInput, PageInput, curator_paths, is_base, is_note_id};
use crate::paths::AppPaths;
use crate::spaces::{self, SpaceId};
use crate::text::{hash_text, local_iso, now, now_ms};
use crate::wiki::{self, Result, wiki_err};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const API: &str = "/sync/v1";
/// At most this many entries in one push.
pub const MAX_ENTRIES: usize = 100;

// ---------------------------------------------------------------- configuration

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// This machine's name, as the curators it pushes to know it (their grants name machine:<it>).
    pub machine: Option<String>,
    /// Peer name → the base URL of its Xenolith service.
    pub peers: BTreeMap<String, String>,
    /// Followed space → the peer that curates it.
    pub follow: BTreeMap<String, String>,
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
pub fn valid_url(u: &str) -> bool {
    let u = u.trim_end_matches('/');
    let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| u.starts_with(p));
    (u.starts_with("https://") || local) && !u.contains(['?', '#', ' ', '"', '\\']) && u.len() < 300
}

impl Config {
    pub fn load(paths: &AppPaths) -> Config {
        let v: Value = fs::read_to_string(config_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let map = |k: &str, ok: &dyn Fn(&str, &str) -> bool| -> BTreeMap<String, String> {
            v[k].as_object().into_iter().flatten().filter_map(|(a, b)| b.as_str().filter(|b| ok(a, b)).map(|b| (a.clone(), b.to_string()))).collect()
        };
        Config {
            machine: v["machine"].as_str().filter(|m| valid_name(m)).map(String::from),
            peers: map("peers", &|name, url| valid_name(name) && valid_url(url)),
            follow: map("follow", &|space, peer| SpaceId::parse(space).is_ok_and(|s| !s.is_local()) && valid_name(peer)),
        }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let v = json!({ "machine": self.machine, "peers": self.peers, "follow": self.follow });
        wiki::atomic_write(&config_file(paths), &format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default()), None, None)
    }

    /// The peer that curates a space, when this machine only follows it.
    pub fn curator_of(&self, space: &SpaceId) -> Option<&str> {
        self.follow.get(&space.to_string()).map(String::as_str)
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

/// Keeps a peer's token, readable by you alone.
pub fn save_token(paths: &AppPaths, peer: &str, token: &str) -> Result<()> {
    let file = token_file(paths, peer);
    wiki::atomic_write(&file, &format!("{}\n", token.trim()), None, None)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_token(paths: &AppPaths, peer: &str) -> Option<String> {
    fs::read_to_string(token_file(paths, peer)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------- the curator's side

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

fn page_file(dir: &Path, slug: &str) -> Result<PathBuf> {
    wiki::confined_path(dir, &dir.join("pages").join(format!("{slug}.md")))
}

/// The page's hash as sync compares it, or "none".
pub fn page_base(dir: &Path, slug: &str) -> String {
    page_file(dir, slug).ok().and_then(|f| wiki::read_if_exists(&f).ok().flatten()).map(|t| hash_text(&t, 16)).unwrap_or_else(|| "none".into())
}

/// Records a page write that came in against an older version as a proposal: a held change of
/// kind "sync" that replaces the page as it is now with what the other machine wrote. Approving it
/// later applies it only if the page hasn't changed again.
fn propose(dir: &Path, who: &str, by: &str, id: &str, note: &Value, base: &str, current: &str) -> Result<(String, usize)> {
    let page = &note["page"];
    let slug = page["slug"].as_str().filter(|s| wiki::SLUG_RE.is_match(s)).map(String::from).unwrap_or_else(|| wiki::slugify(note["title"].as_str().unwrap_or("")));
    if !wiki::SLUG_RE.is_match(&slug) {
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
    // The note behind it is the pushing token's subject's, whatever source it claims.
    let held = HeldOp {
        write,
        reasons: vec![reason],
        op,
        notes: vec![json!({ "id": id, "app": note["app"], "title": note["title"], "source": inbox::trusted_source(note["source"].as_str().unwrap_or(""), ""), "by": by })],
        kind: "sync".into(),
    };
    wiki::with_lock(dir, || {
        crate::held::record(dir, &batch, &local_iso(&now()), std::slice::from_ref(&held))?;
        crate::held::log_line(dir, "curator", &format!("Proposal from {who} waiting for your OK: {} [[{slug}]] (written against an older version)", crate::text::one_line(&title)))
    })?;
    Ok((batch, 0))
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
            out.push(result(id, "refused", vec![("error", json!("An entry needs its note id."))]));
            continue;
        }
        let note = &e["note"];
        let is_page = note["kind"] == "page";
        let replace = is_page && note["page"]["mode"] == "replace";
        let need = if replace { Level::Write } else { Level::Append };
        if !can(need) {
            out.push(result(id, "refused", vec![("error", json!(format!("This token can't {} here.", need.as_str())))]));
            continue;
        }
        let slug = note["page"]["slug"].as_str().unwrap_or("").to_string();
        let base = note["page"]["base"].as_str().filter(|b| is_base(b));
        if let (true, Some(base)) = (replace, base) {
            let slug = if slug.is_empty() { wiki::slugify(note["title"].as_str().unwrap_or("")) } else { slug.clone() };
            let current = page_base(dir, &slug);
            if current != base {
                match propose(dir, &name, &subject, id, note, base, &current) {
                    Ok((batch, index)) => {
                        conflict = true;
                        out.push(result(id, "conflict", vec![("current", json!(current)), ("proposal", json!({ "batch": batch, "index": index }))]));
                    }
                    Err(err) => out.push(result(id, "refused", vec![("error", json!(err.to_string()))])),
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
                mode: note["page"]["mode"].as_str().map(String::from),
                base: base.map(String::from),
            }),
            // The same entry sent twice is one note: keyed by who sent it and its id there.
            idempotency_key: Some(format!("sync:{subject}:{id}")),
        };
        // Stamped as the pushing token's subject: its source can't be the curator's own user's.
        match inbox::submit_note(dir, &input, "sync", &format!("sync {subject}"), &by) {
            Ok(r) => out.push(result(id, if r.duplicate { "duplicate" } else { "accepted" }, vec![("note", json!(r.id))])),
            Err(err) => out.push(result(id, "refused", vec![("error", json!(err.to_string()))])),
        }
    }
    (out, conflict)
}

/// The curator's pages, by slug and hash, for a copy to compare against.
pub fn page_list(dir: &Path) -> Value {
    let pages: Vec<Value> = wiki::list_pages(dir)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            let text = wiki::read_if_exists(&page_file(dir, &p.slug).ok()?).ok().flatten()?;
            Some(json!({ "slug": p.slug, "hash": hash_text(&text, 16) }))
        })
        .collect();
    json!({ "pages": pages })
}

pub fn page_get(dir: &Path, slug: &str) -> Option<Value> {
    if !wiki::SLUG_RE.is_match(slug) {
        return None;
    }
    let text = wiki::read_if_exists(&page_file(dir, slug).ok()?).ok().flatten()?;
    Some(json!({ "slug": slug, "hash": hash_text(&text, 16), "text": text }))
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
        return Err(crate::text::one_line(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok((status, serde_json::from_str(json_text).unwrap_or(Value::Null)))
}

fn space_url(base: &str, space: &SpaceId) -> String {
    format!("{}{API}/spaces/{}/{}", base.trim_end_matches('/'), space.kind, space.name)
}

/// A note waiting in a followed space, as an entry for its curator.
fn entry_of(n: &inbox::Note) -> Value {
    let mut note = json!({ "kind": n.kind, "app": n.app, "source": n.source, "title": n.title, "body": n.body, "tags": n.tags, "pages": n.pages });
    if let Some(p) = &n.page {
        note["page"] = p.clone();
    }
    json!({ "id": n.id, "note": note })
}

/// Moves a pushed note out of the inbox into .curator/sent/, with what the curator said about it.
fn mark_sent(dir: &Path, id: &str, peer: &str, r: &Value) -> Result<()> {
    let p = curator_paths(dir);
    let from = wiki::confined_path(dir, &p.inbox.join(format!("{id}.md")))?;
    let Some(text) = wiki::read_if_exists(&from)? else { return Ok(()) };
    let to = wiki::confined_path(dir, &p.cur.join("sent").join(inbox::month_dir(id)).join(format!("{id}.md")))?;
    fs::create_dir_all(to.parent().unwrap_or(dir))?;
    let stamp = format!("<!-- sent to {peer} at {}: {} -->\n", local_iso(&now()), r);
    wiki::atomic_write(&to, &format!("{stamp}{text}"), None, None)?;
    fs::remove_file(&from)?;
    inbox::clear_state(dir, id);
    Ok(())
}

/// Writes the curator's version of a page into the copy, keeping what it replaces in .history/.
fn take_page(dir: &Path, slug: &str, text: &str) -> Result<bool> {
    let file = page_file(dir, slug)?;
    let cur = wiki::read_if_exists(&file)?;
    if cur.as_deref() == Some(text) {
        return Ok(false);
    }
    wiki::with_lock(dir, || {
        if let Some(c) = &cur {
            let rel = format!(".history/pages/{slug}/{}-sync.md", crate::text::history_stamp(&now()));
            let h = wiki::confined_path(dir, &dir.join(&rel))?;
            fs::create_dir_all(h.parent().unwrap_or(dir))?;
            wiki::write_if_missing(&h, c)?;
        }
        fs::create_dir_all(dir.join("pages"))?;
        wiki::atomic_write(&file, text, None, None)?;
        wiki::refresh_index(dir, None).map(|_| ())
    })?;
    Ok(true)
}

/// One sync of one followed space: push its waiting notes, then pull the curator's pages.
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
    let url = space_url(base, space);
    let timeout = Duration::from_secs(30);
    let who = cfg.machine.clone().map(|m| format!("machine:{m}")).unwrap_or_else(|| "this machine".into());

    // 1. Push.
    let (mut pushed, mut conflicts, mut refused) = (0, 0, 0);
    let notes: Vec<inbox::Note> = inbox::list_notes(&dir).unwrap_or_default().into_iter().filter(|n| is_note_id(&n.id)).take(MAX_ENTRIES).collect();
    if !notes.is_empty() {
        let body = json!({ "entries": notes.iter().map(entry_of).collect::<Vec<_>>() });
        match request("POST", &format!("{url}/entries"), &token, Some(&body), timeout) {
            Ok((200 | 409, v)) => {
                for r in v["results"].as_array().into_iter().flatten() {
                    let id = r["id"].as_str().unwrap_or("");
                    match r["result"].as_str() {
                        Some("accepted" | "duplicate") => pushed += 1,
                        Some("conflict") => conflicts += 1,
                        _ => {
                            refused += 1;
                            continue;
                        }
                    }
                    if let Err(e) = mark_sent(&dir, id, peer, r) {
                        return report(json!({ "error": format!("pushed, but could not move note {id}: {e}") }));
                    }
                }
            }
            Ok((status, v)) => return report(json!({ "error": format!("{peer} answered {status}: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("")) })),
            Err(e) => return report(json!({ "error": format!("{peer} could not be reached: {e}") })),
        }
    }

    // 2. Pull.
    let mut pulled = 0;
    match request("GET", &format!("{url}/pages"), &token, None, timeout) {
        Ok((200, v)) => {
            for p in v["pages"].as_array().into_iter().flatten() {
                let (Some(slug), Some(hash)) = (p["slug"].as_str(), p["hash"].as_str()) else { continue };
                if !wiki::SLUG_RE.is_match(slug) || page_base(&dir, slug) == hash {
                    continue;
                }
                match request("GET", &format!("{url}/pages/{slug}"), &token, None, timeout) {
                    Ok((200, page)) => {
                        let Some(text) = page["text"].as_str() else { continue };
                        if crate::secrets::find_secret(text).is_some() {
                            continue; // the curator's own guard should have refused it; never copy one
                        }
                        match take_page(&dir, slug, text) {
                            Ok(true) => pulled += 1,
                            Ok(false) => {}
                            Err(e) => return report(json!({ "error": format!("could not write page {slug}: {e}") })),
                        }
                    }
                    Ok((status, _)) => return report(json!({ "error": format!("{peer} answered {status} for page {slug}") })),
                    Err(e) => return report(json!({ "error": format!("{peer} could not be reached: {e}") })),
                }
            }
        }
        Ok((status, v)) => return report(json!({ "error": format!("{peer} answered {status}: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("")) })),
        Err(e) => return report(json!({ "error": format!("{peer} could not be reached: {e}") })),
    }
    if pushed + conflicts + refused + pulled > 0 {
        crate::audit::best_effort(
            paths,
            "sync",
            "you",
            vec![
                ("space", json!(space.to_string())),
                ("peer", json!(peer)),
                ("as", json!(who)),
                ("pushed", json!(pushed)),
                ("conflicts", json!(conflicts)),
                ("refused", json!(refused)),
                ("pulled", json!(pulled)),
            ],
        );
    }
    report(json!({ "pushed": pushed, "conflicts": conflicts, "refused": refused, "pulled": pulled, "at": now_ms() }))
}

/// Syncs every followed space; one report each.
pub fn sync_all(paths: &AppPaths, root: &Path) -> Vec<Value> {
    let cfg = Config::load(paths);
    cfg.follow.keys().filter_map(|s| SpaceId::parse(s).ok()).map(|s| sync_space(paths, root, &cfg, &s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::random_hex;

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
    fn the_config_keeps_only_what_it_can_use() {
        let (d, paths) = tmp_paths();
        update(&paths, |c| {
            c.machine = Some("studio".into());
            c.peers.insert("laptop".into(), "https://laptop.example/xenolith".into());
            c.follow.insert("project:atlas".into(), "laptop".into());
            Ok(())
        })
        .unwrap();
        let mut raw: Value = serde_json::from_str(&fs::read_to_string(config_file(&paths)).unwrap()).unwrap();
        raw["peers"]["bad"] = json!("http://far.example");
        raw["follow"]["machine:local"] = json!("laptop");
        fs::write(config_file(&paths), raw.to_string()).unwrap();
        let c = Config::load(&paths);
        assert_eq!(c.peers.keys().collect::<Vec<_>>(), ["laptop"], "a clear-text URL to another machine is dropped");
        assert_eq!(c.follow.keys().collect::<Vec<_>>(), ["project:atlas"], "this machine's own space is never followed");
        assert_eq!(c.curator_of(&SpaceId::parse("project:atlas").unwrap()), Some("laptop"));
        save_token(&paths, "laptop", "xenolith_x\n").unwrap();
        assert_eq!(read_token(&paths, "laptop").as_deref(), Some("xenolith_x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(token_file(&paths, "laptop")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(d).unwrap();
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
        assert_eq!(accept(&dir, &who, &[replace], &only_append).0[0]["result"], "refused", "a replace needs write");
        assert_eq!(accept(&dir, &who, &[json!({ "id": "nope", "note": {} })], &all).0[0]["result"], "refused");
        fs::remove_dir_all(d).unwrap();
    }
}
