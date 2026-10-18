//! core/integrations/reviews.js: text change review for OpenShell jobs. A
//! Git project's selected text is frozen before upload (its manifest is
//! the owner's final check), what the sandbox hands back is validated as
//! untrusted, and the difference is a patch you can export. Nothing a
//! sandbox returns is ever used as a host path.
//!
//! The program that collects text inside the sandbox (COLLECT_TEXT) runs
//! in the sandbox's Node, so it's the JS's, made from reviews.js's own
//! functions; the binary carries it as text (collect-text.mjs writes it,
//! and a test checks the two agree). git runs through the twin.

/// The program that collects text inside a sandbox, as reviews.js makes it.
pub const COLLECT_TEXT: &str = include_str!("collect_text.js");

use crate::b64;
use crate::common::{fail, failed, write_new, Fail};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::OnceLock;
use xenonet_app::fsx;
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;

pub const FILES: usize = 512;
pub const FILE_BYTES: usize = 256 * 1024;
pub const TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const DEPTH: usize = 24;
pub const COUNT: usize = 32;
pub const RECORD_BYTES: usize = 24 * 1024 * 1024;
pub const STORE_BYTES: u64 = 128 * 1024 * 1024;
pub const CACHED: usize = 4;
const PREVIEW_MS: f64 = 10.0 * 60e3;

pub fn limits() -> Value {
    json!({ "files": FILES, "fileBytes": FILE_BYTES, "totalBytes": TOTAL_BYTES, "depth": DEPTH })
}

pub fn storage() -> Value {
    json!({ "count": COUNT, "recordBytes": RECORD_BYTES, "totalBytes": STORE_BYTES, "cached": CACHED })
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-')
}

fn hash(s: &str) -> String {
    b64::sha256_hex(s.as_bytes())
}

/// The files' digest: [path, content] pairs, sorted by path.
pub fn digest(files: &[Value]) -> String {
    let mut pairs: Vec<(String, Value)> = files.iter().map(|f| (f.get("path").map(js_string).unwrap_or_default(), json!([f.get("path"), f.get("content")]))).collect();
    pairs.sort_by(|a, b| xenonet_app::jstext::locale_compare(&a.0, &b.0));
    hash(&xenonet_base::json::stringify(&Value::Array(pairs.into_iter().map(|(_, p)| p).collect()), 0))
}

// JS's `.` (no line terminators), as a class.
const DOT: &str = r"[^\n\r\u{2028}\u{2029}]";

fn excluded_res() -> &'static [Regex; 4] {
    static R: OnceLock<[Regex; 4]> = OnceLock::new();
    R.get_or_init(|| {
        [
            Regex::new(r"^(?i-u:\.git|node_modules|\.ssh|\.aws|\.azure|\.gnupg|\.venv|venv|__pycache__|\.cache|\.xenonet)$").expect("a fixed pattern"),
            Regex::new(&format!(r"^(?i-u:\.env)(?:\.{DOT}*)?$")).expect("a fixed pattern"),
            Regex::new(&format!(r"^(?:(?i-u:\.npmrc|\.pypirc|\.netrc|\.git-credentials|credentials(?:\.json)?|id_rsa|id_ed25519)|(?i-u:secrets?)(?:\.{DOT}*)?)$"))
                .expect("a fixed pattern"),
            Regex::new(r"\.(?i-u:pem|key|p12|pfx|keystore)$").expect("a fixed pattern"),
        ]
    })
}

/// Whether a path is out of scope: VCS and dependency folders, keys,
/// credentials and environment files. The same rules run in the sandbox.
pub fn excluded_path(name: &str) -> bool {
    name.split('/').any(|part| excluded_res().iter().any(|r| r.is_match(part)))
}

/// Whether a path is portable: plain characters, at most 24 levels, no
/// part a Windows device name, ending in a dot or a space.
pub fn review_path(name: &Value) -> bool {
    let Some(name) = name.as_str() else { return false };
    if name.is_empty() || name.len() > 240 || !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.@ +/-".contains(&c)) {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() <= DEPTH
        && parts.iter().all(|p| {
            if p.is_empty() || *p == "." || *p == ".." || p.ends_with('.') || p.ends_with(' ') {
                return false;
            }
            let stem = p.split('.').next().unwrap_or("").to_ascii_lowercase();
            let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
                || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && matches!(stem.as_bytes()[3], b'1'..=b'9'));
            !device
        })
}

/// Text: valid UTF-8, without a NUL.
pub fn text_bytes(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Whether paths stay apart on a system that doesn't tell case apart, and
/// no file is also a folder.
pub fn portable_set(names: &[String]) -> bool {
    let mut prefixes: std::collections::HashMap<String, (String, bool)> = std::collections::HashMap::new();
    for name in names {
        let parts: Vec<&str> = name.split('/').collect();
        let mut prefix = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(part);
            let file = i == parts.len() - 1;
            let key = prefix.to_lowercase();
            if let Some((seen, seen_file)) = prefixes.get(&key) {
                if *seen != prefix || *seen_file != file || file {
                    return false;
                }
            }
            prefixes.insert(key, (prefix.clone(), file));
        }
    }
    true
}

// ---------- the frozen snapshot ----------

#[cfg(unix)]
fn stamp(m: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}:{}:{}:{}.{}:{}.{}", m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec())
}

#[cfg(not(unix))]
fn stamp(m: &std::fs::Metadata) -> String {
    use std::os::windows::fs::MetadataExt;
    format!("{}:{}:{}:{}", m.file_size(), m.last_write_time(), m.creation_time(), m.file_attributes())
}

fn git_env(env: &Map<String, Value>) -> Map<String, Value> {
    env.iter().filter(|(k, _)| !k.starts_with("GIT_")).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn fs_fail(e: fsx::FsError) -> Failure {
    Failure::from(e)
}

fn refused(f: Fail) -> Failure {
    let mut m = Map::new();
    m.insert("status".into(), json!(f.status));
    m.insert("code".into(), json!(f.code));
    m.insert("integration".into(), json!(true));
    Failure::with(f.message, m)
}

fn realpath(p: &str) -> Result<String, Failure> {
    fsx::realpath(p).map_err(fs_fail)
}

async fn git(io: &Io, args: Vec<String>, env: &Map<String, Value>, max_buffer: usize) -> Result<String, Failure> {
    let out = io.ask(json!({ "op": "git", "args": args, "options": { "env": env, "timeout": 10000, "maxBuffer": max_buffer } })).await.map_err(|_| Failure::Rethrow)?;
    Ok(out.get("stdout").map(js_string).unwrap_or_default())
}

fn open_nofollow(file: &str) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW);
    }
    o.open(file)
}

/// A Git project's selected text, frozen: { files, excluded, omittedPaths,
/// bytes, digest }. `env` is XenoNet's, without git's own settings.
pub async fn source_snapshot(io: Io, source: String, env: Map<String, Value>) -> Result<Value, Failure> {
    // Windows can spell one directory with either short or long names.
    let source = realpath(&source)?;
    let env = git_env(&env);
    let base: Vec<String> = ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=", "-C", &source].iter().map(ToString::to_string).collect();
    let with = |more: &[&str]| -> Vec<String> { base.iter().cloned().chain(more.iter().map(ToString::to_string)).collect() };
    let root = git(&io, with(&["rev-parse", "--show-toplevel"]), &env, 4096).await?;
    if realpath(xenonet_base::text::js_trim(&root))? != source {
        return Err(refused(Fail { status: 400, code: "bad-request".into(), message: "Change review needs the root of a Git working tree.".into() }));
    }
    let listed = git(&io, with(&["ls-files", "--cached", "--others", "--exclude-standard", "-z"]), &env, 1024 * 1024).await?;
    let mut names: Vec<String> = Vec::new();
    for n in listed.split('\0').filter(|n| !n.is_empty()) {
        if !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    }
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let f = |m: &str| refused(Fail { status: 400, code: "bad-request".into(), message: m.into() });
    if names.len() > 10000 {
        return Err(f("This project has too many files to prepare a bounded review. Choose a smaller project."));
    }
    let (mut files, mut excluded, mut bytes) = (Vec::new(), Vec::new(), 0usize);
    let omit = |excluded: &mut Vec<Value>, name: &str, reason: &str| -> Result<(), Failure> {
        if excluded.len() >= FILES {
            return Err(f("Too many exclusions to review. Choose a smaller project."));
        }
        excluded.push(json!({ "path": xenonet_app::jstext::slice16(name, 240), "reason": reason }));
        Ok(())
    };
    let changed = || refused(Fail { status: 409, code: "changed".into(), message: "The project changed while preparing its snapshot. Preview again.".into() });
    let sep = std::path::MAIN_SEPARATOR;
    for name in &names {
        if excluded_path(name) {
            omit(&mut excluded, name, "Excluded from upload and review scope")?;
            continue;
        }
        if !review_path(&json!(name)) {
            omit(&mut excluded, name, "Unsupported portable path")?;
            continue;
        }
        let mut parts: Vec<&str> = vec![&source];
        parts.extend(name.split('/'));
        let file = NATIVE.join(&parts);
        let st = match std::fs::symlink_metadata(&file) {
            Ok(st) => st,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(fs_fail(fsx::FsError::of(e, "lstat", &file))),
        };
        let ignore_rule = NATIVE.basename(name, None) == ".gitignore";
        let regular = st.is_file() && !st.file_type().is_symlink() && realpath(&file)?.starts_with(&format!("{source}{sep}"));
        if !regular {
            if ignore_rule {
                return Err(f("A repository .gitignore is a link or unsupported file. Change review cannot freeze its rules."));
            }
            omit(&mut excluded, name, "Links and non-regular files are excluded")?;
            continue;
        }
        if st.len() as usize > FILE_BYTES {
            if ignore_rule {
                return Err(f("A repository .gitignore exceeds the review limit."));
            }
            omit(&mut excluded, name, "Larger than the per-file limit")?;
            continue;
        }
        // Every parent must still be a real directory inside the source.
        let mut parent = NATIVE.dirname(&file);
        let mut safe = true;
        while parent != source {
            let m = std::fs::symlink_metadata(&parent).map_err(|e| fs_fail(fsx::FsError::of(e, "lstat", &parent)))?;
            if m.file_type().is_symlink() {
                safe = false;
                break;
            }
            let up = NATIVE.dirname(&parent);
            if up == parent {
                break;
            }
            parent = up;
        }
        if !safe {
            omit(&mut excluded, name, "A parent directory is a link")?;
            continue;
        }
        let mut fd = open_nofollow(&file).map_err(|e| fs_fail(fsx::FsError::of(e, "open", &file)))?;
        let before = stamp(&st);
        if stamp(&fd.metadata().map_err(|e| fs_fail(fsx::FsError::of(e, "fstat", &file)))?) != before {
            return Err(changed());
        }
        let mut content = Vec::with_capacity(st.len() as usize);
        use std::io::Read;
        (&mut fd).take(st.len()).read_to_end(&mut content).map_err(|e| fs_fail(fsx::FsError::of(e, "read", &file)))?;
        if content.len() as u64 != st.len() || stamp(&fd.metadata().map_err(|e| fs_fail(fsx::FsError::of(e, "fstat", &file)))?) != before {
            return Err(changed());
        }
        drop(fd);
        if !text_bytes(&content) {
            if ignore_rule {
                return Err(f("A repository .gitignore is not valid UTF-8 text."));
            }
            omit(&mut excluded, name, "Binary or invalid UTF-8 file")?;
            continue;
        }
        bytes += content.len();
        if bytes > TOTAL_BYTES || files.len() >= FILES {
            return Err(f("The selected text exceeds the review limits (512 files, 4 MiB total)."));
        }
        files.push(json!({ "path": name, "content": b64::encode(&content) }));
    }
    if files.is_empty() {
        return Err(f("No supported text files remain in this Git project."));
    }
    let paths: Vec<String> = files.iter().map(|x| js_string(&x["path"])).collect();
    if !portable_set(&paths) {
        return Err(f("The project has paths that collide on another platform. Rename them before preparing a review."));
    }
    let ignored = git(&io, with(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "--no-empty-directory", "-z"]), &env, 128 * 1024).await?;
    let omitted: Vec<String> = ignored.split('\0').filter(|n| !n.is_empty()).map(String::from).collect();
    if omitted.len() > FILES || omitted.iter().any(|n| xenonet_app::jstext::utf16_len(n) > 241) {
        return Err(f("Too many existing ignored paths to freeze the review scope."));
    }
    let d = digest(&files);
    Ok(json!({ "files": files, "excluded": excluded, "omittedPaths": omitted, "bytes": bytes, "digest": d }))
}

// ---------- what a sandbox hands back ----------

fn invalid<T>(message: &str) -> Result<T, Fail> {
    failed(message, "retrieval-invalid", 502)
}

/// What the collector returned, as untrusted: bounded, portable, in scope,
/// canonical base64 of UTF-8 text, and no file also a folder.
pub fn validate_result(value: &Value) -> Result<Value, Fail> {
    let files = value.get("files").and_then(Value::as_array);
    let problems = value.get("problems").and_then(Value::as_array);
    let omissions = value.get("omissions").and_then(Value::as_array);
    let (Some(files), Some(problems), Some(omissions)) = (files, problems, omissions) else { return invalid("The gateway returned an invalid text snapshot.") };
    if !truthy(Some(value)) || files.len() > FILES || problems.len() > 30 || omissions.len() > FILES {
        return invalid("The gateway returned an invalid text snapshot.");
    }
    let paths: Vec<String> = files.iter().map(|f| f.get("path").map(js_string).unwrap_or_default()).collect();
    if !files.iter().all(|f| truthy(Some(f)) && review_path(f.get("path").unwrap_or(&Value::Null))) || !portable_set(&paths) {
        return invalid("The gateway returned colliding or unsupported file paths.");
    }
    let mut seen: Vec<String> = Vec::new();
    let mut total = 0usize;
    for f in files {
        let path = f.get("path").and_then(Value::as_str).unwrap_or("");
        let content = f.get("content").and_then(Value::as_str);
        let Some(content) = content.filter(|c| review_path(&json!(path)) && !excluded_path(path) && c.len() <= FILE_BYTES.div_ceil(3) * 4) else {
            return invalid("The gateway returned an unsupported file.");
        };
        let key = path.to_lowercase();
        if seen.contains(&key) {
            return invalid("The gateway returned colliding file paths.");
        }
        seen.push(key);
        let bytes = b64::decode(content);
        total += bytes.len();
        if b64::encode(&bytes) != content || bytes.len() > FILE_BYTES || total > TOTAL_BYTES || !text_bytes(&bytes) {
            return invalid("The gateway returned invalid or oversized text.");
        }
    }
    // A file can't also be the parent of another returned file.
    for key in &seen {
        let mut cut = key.len();
        while let Some(i) = key[..cut].rfind('/') {
            if i == 0 {
                break;
            }
            if seen.iter().any(|s| s == &key[..i]) {
                return invalid("The gateway returned colliding file paths.");
            }
            cut = i;
        }
    }
    for entry in problems.iter().chain(omissions) {
        let ok = truthy(Some(entry))
            && entry.get("path").and_then(Value::as_str).is_some_and(|p| xenonet_app::jstext::utf16_len(p) <= 240)
            && entry.get("reason").and_then(Value::as_str).is_some_and(|r| xenonet_app::jstext::utf16_len(r) <= 160);
        if !ok {
            return invalid("The gateway returned invalid retrieval diagnostics.");
        }
    }
    Ok(value.clone())
}

// ---------- the patch ----------

fn quote(name: &str) -> String {
    xenonet_base::json::stringify(&json!(name), 0)
}

fn lines(content: &str, prefix: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let mut data: Vec<&str> = content.split('\n').collect();
    let newline = data.last() == Some(&"");
    if newline {
        data.pop();
    }
    let body: String = data.iter().flat_map(|l| [prefix, *l, "\n"]).collect();
    if newline {
        body
    } else {
        format!("{body}\\ No newline at end of file\n")
    }
}

fn line_count(v: &str) -> usize {
    if v.is_empty() {
        0
    } else {
        v.split('\n').count() - usize::from(v.ends_with('\n'))
    }
}

/// The text change from `before` to `after` as a Git patch: { changes,
/// patch }, each change its path, status, counts and diff.
pub fn text_patch(before: &[Value], after: &[Value]) -> Value {
    let map = |list: &[Value]| -> Vec<(String, Value)> {
        let mut m: Vec<(String, Value)> = Vec::new();
        for f in list {
            let p = f.get("path").map(js_string).unwrap_or_default();
            let c = f.get("content").cloned().unwrap_or(Value::Null);
            match m.iter_mut().find(|(k, _)| *k == p) {
                Some(slot) => slot.1 = c,
                None => m.push((p, c)),
            }
        }
        m
    };
    let (old, next) = (map(before), map(after));
    let get = |m: &[(String, Value)], k: &str| m.iter().find(|(p, _)| p == k).map(|(_, c)| c.clone());
    let mut names: Vec<String> = Vec::new();
    for (k, _) in old.iter().chain(&next) {
        if !names.contains(k) {
            names.push(k.clone());
        }
    }
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut changes = Vec::new();
    let mut patch = String::new();
    for name in names {
        let (a0, b0) = (get(&old, &name), get(&next, &name));
        if a0 == b0 {
            continue;
        }
        let text = |c: &Option<Value>| c.as_ref().map_or_else(String::new, |c| String::from_utf8_lossy(&b64::decode(&js_string(c))).into_owned());
        let (a, b) = (text(&a0), text(&b0));
        let (removed, added) = (line_count(&a), line_count(&b));
        let status = if a0.is_none() {
            "added"
        } else if b0.is_none() {
            "deleted"
        } else {
            "modified"
        };
        let mut diff = format!("diff --git {} {}\n", quote(&format!("a/{name}")), quote(&format!("b/{name}")));
        if status == "added" {
            diff.push_str("new file mode 100644\n");
        }
        if status == "deleted" {
            diff.push_str("deleted file mode 100644\n");
        }
        if removed == 0 && added == 0 {
            let blob = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
            let _ = if status == "added" { writeln!(diff, "index 0000000..{blob}") } else { writeln!(diff, "index {blob}..0000000") };
        } else {
            let from = if a0.is_some() { quote(&format!("a/{name}")) } else { "/dev/null".into() };
            let to = if b0.is_some() { quote(&format!("b/{name}")) } else { "/dev/null".into() };
            let _ = write!(diff, "--- {from}\n+++ {to}\n@@ -{},{removed} +{},{added} @@\n{}{}", usize::from(removed > 0), usize::from(added > 0), lines(&a, "-"), lines(&b, "+"));
        }
        patch.push_str(&diff);
        changes.push(json!({ "path": name, "status": status, "added": added, "removed": removed, "diff": diff }));
    }
    json!({ "changes": changes, "patch": patch })
}

// ---------- local reviews ----------

const SCOPE_NOTE: &str =
    "New files use frozen repository .gitignore rules. Existing omitted paths stay excluded. Global and local-only ignore rules are not applied to newly created paths.";

/// The local reviews: a folder of one per job, readable by you alone,
/// and the last few read kept in memory.
pub struct Reviews {
    dir: Option<String>,
    memory: Vec<Value>,
    pub demo: bool,
}

type R<T> = Result<T, Fail>;

impl Reviews {
    pub fn new(dir: Option<String>, demo: bool) -> Self {
        Reviews { dir, memory: Vec::new(), demo }
    }

    fn root(&mut self) -> R<String> {
        let d = match &self.dir {
            Some(d) => d.clone(),
            None => {
                let d = fsx::mkdtemp(&NATIVE.join(&[&std::env::temp_dir().to_string_lossy(), "xenonet-reviews-"]))?;
                self.dir = Some(d.clone());
                d
            }
        };
        fsx::mkdir_p(&d, Some(0o700))?;
        Ok(d)
    }

    fn location(&mut self, id: &Value) -> R<String> {
        let id = id.as_str().filter(|i| is_uuid(i)).map(String::from);
        let Some(id) = id else { return failed("No such local review.", "missing", 404) };
        Ok(NATIVE.join(&[&self.root()?, &id]))
    }

    fn cache(&mut self, value: Value) -> Value {
        self.memory.retain(|v| v["id"] != value["id"]);
        self.memory.push(value.clone());
        while self.memory.len() > CACHED {
            self.memory.remove(0);
        }
        value
    }

    fn disk_bytes(&mut self, folder: Option<&str>) -> R<u64> {
        let folder = match folder {
            Some(f) => f.to_string(),
            None => self.root()?,
        };
        let mut total = 0u64;
        for name in fsx::read_dir_names(&folder)? {
            let file = NATIVE.join(&[&folder, &name]);
            let m = std::fs::symlink_metadata(&file).map_err(|e| fsx::FsError::of(e, "lstat", &file))?;
            if m.file_type().is_symlink() {
                return failed("The review storage contains an unexpected link.", "storage-invalid", 409);
            }
            if m.is_dir() {
                total += self.disk_bytes(Some(&file))?;
            } else if m.is_file() {
                total += m.len();
            }
            if total > STORE_BYTES {
                break;
            }
        }
        Ok(total)
    }

    fn count(&mut self) -> R<usize> {
        Ok(fsx::read_dir_names(&self.root()?)?.iter().filter(|n| is_uuid(n)).count())
    }

    pub fn save(&mut self, value: &Value) -> R<Value> {
        let mut m: Map<String, Value> = value.as_object().cloned().unwrap_or_default();
        m.insert("generation".into(), json!(b64::uuid()));
        let value = Value::Object(m);
        let serialized = xenonet_base::json::stringify(&value, 0);
        let bytes = serialized.len() as u64;
        let folder = self.location(&value["id"])?;
        if bytes > RECORD_BYTES as u64 {
            return failed("This diff is too large to retain safely. Retrieve a smaller text change; the previous local review is retained.", "review-too-large", 413);
        }
        let existing = NATIVE.join(&[&folder, "review.json"]);
        let old = if fsx::exists(&existing) { std::fs::symlink_metadata(&existing).map_err(|e| fsx::FsError::of(e, "lstat", &existing))?.len() } else { 0 };
        if old == 0 && self.count()? >= COUNT {
            return failed("Local review storage is full. Forget an old review before preparing another.", "review-capacity", 409);
        }
        if self.disk_bytes(None)? as f64 - old as f64 + bytes as f64 > STORE_BYTES as f64 {
            return failed("Local review storage is full. Forget an old review first; existing reviews are retained.", "review-capacity", 409);
        }
        fsx::mkdir_p(&folder, Some(0o700))?;
        let tmp = NATIVE.join(&[&folder, &format!("{}.tmp", b64::uuid())]);
        let r = write_new(&tmp, serialized.as_bytes(), 0o600).and_then(|()| fsx::rename(&tmp, &existing));
        let _ = std::fs::remove_file(&tmp);
        r?;
        Ok(self.cache(value))
    }

    pub fn load(&mut self, id: &Value) -> R<Value> {
        if let Some(v) = self.memory.iter().find(|v| xenonet_app::jsv::strict_eq(v.get("id"), Some(id))).cloned() {
            return Ok(self.cache(v));
        }
        let file = NATIVE.join(&[&self.location(id)?, "review.json"]);
        let read = || -> Option<Value> {
            let st = std::fs::symlink_metadata(&file).ok()?;
            if !st.is_file() || st.file_type().is_symlink() || st.len() > RECORD_BYTES as u64 {
                return None;
            }
            let v = xenonet_base::json::parse(&fsx::read_text(&file).ok()?).ok()?;
            xenonet_app::jsv::strict_eq(v.get("id"), Some(id)).then_some(v)
        };
        match read() {
            Some(v) => Ok(self.cache(v)),
            None => failed("No local baseline remains for this job.", "missing", 404),
        }
    }

    pub fn describe(v: &Value) -> Value {
        let files: Vec<Value> = v
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|f| json!({ "path": f.get("path"), "bytes": b64::byte_length(&f.get("content").map(js_string).unwrap_or_default()) }))
            .collect();
        json!({ "id": v.get("id"), "digest": v.get("digest"), "at": v.get("at"), "bytes": v.get("bytes"), "files": files, "excluded": v.get("excluded"), "omittedPaths": v.get("omittedPaths").filter(|x| !x.is_null()).cloned().unwrap_or_else(|| json!([])), "scopeNote": SCOPE_NOTE, "limits": limits() })
    }

    pub fn forget(&mut self, id: &Value) -> R<Value> {
        self.memory.retain(|v| !xenonet_app::jsv::strict_eq(v.get("id"), Some(id)));
        fsx::rm_rf(&self.location(id)?)?;
        Ok(json!({ "forgotten": true }))
    }

    /// Git with nothing of yours: no system or global config, no hooks or
    /// fsmonitor, and an empty excludes file. { args, options }.
    pub fn isolated_git(&mut self, folder: &str, args: &[&str], env: &Map<String, Value>) -> R<Value> {
        let empty = NATIVE.join(&[&NATIVE.dirname(folder), "empty-git-config"]);
        match write_new(&empty, b"", 0o600) {
            Err(e) if !e.is("EEXIST") => return Err(e.into()),
            _ => {}
        }
        let st = std::fs::symlink_metadata(&empty).map_err(|e| fsx::FsError::of(e, "lstat", &empty))?;
        if !st.is_file() || st.file_type().is_symlink() || st.len() != 0 {
            return failed("The isolated Git configuration changed.", "storage-invalid", 409);
        }
        let mut a: Vec<String> =
            ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=", "-c", &format!("core.excludesFile={empty}"), "-C", folder].iter().map(ToString::to_string).collect();
        a.extend(args.iter().map(ToString::to_string));
        let mut e = git_env(env);
        e.insert("GIT_CONFIG_NOSYSTEM".into(), json!("1"));
        e.insert("GIT_CONFIG_GLOBAL".into(), json!(empty));
        e.insert("GIT_CONFIG_SYSTEM".into(), json!(empty));
        Ok(json!({ "args": a, "options": { "env": e, "timeout": 10000, "maxBuffer": 256 * 1024 } }))
    }

    pub fn seed(&mut self, options: &Value, now: f64) -> R<Value> {
        if !self.demo {
            return fail("Synthetic reviews belong to the demo instance.");
        }
        let encode = |v: Option<&Value>| -> Vec<Value> {
            v.and_then(Value::as_object).into_iter().flatten().map(|(p, t)| json!({ "path": p, "content": b64::encode(js_string(t).as_bytes()) })).collect()
        };
        let files = encode(options.get("before"));
        let comparison = text_patch(&files, &encode(options.get("after")));
        let problems = options.get("problems").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!([]));
        let result = if truthy(options.get("unavailable")) {
            Value::Null
        } else {
            let mut m: Map<String, Value> = comparison.as_object().cloned().unwrap_or_default();
            m.insert("revision".into(), json!(b64::uuid()));
            m.insert("at".into(), xenonet_base::json::number(now));
            m.insert("sourceState".into(), options.get("sourceState").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("unchanged")));
            m.insert("complete".into(), json!(problems.as_array().is_some_and(Vec::is_empty)));
            m.insert("problems".into(), problems);
            m.insert("omissions".into(), json!([]));
            m.insert("execution".into(), json!("unknown"));
            m.insert("patchHash".into(), json!(hash(&js_string(&comparison["patch"]))));
            Value::Object(m)
        };
        let bytes: usize = files.iter().map(|f| b64::byte_length(&js_string(&f["content"]))).sum();
        let d = digest(&files);
        let saved = self.save(&json!({ "id": b64::uuid(), "at": xenonet_base::json::number(now), "launched": true, "source": "", "files": files, "excluded": [], "bytes": bytes, "digest": d, "result": result }))?;
        Ok(saved["id"].clone())
    }

    pub fn view(&mut self, id: &Value) -> R<Value> {
        let v = self.load(id)?;
        Ok(json!({ "baseline": Self::describe(&v), "result": v.get("result") }))
    }

    pub fn verify_uploaded(&mut self, id: &Value, input: &Value) -> R<()> {
        let value = validate_result(input)?;
        let v = self.load(id)?;
        let empty = |k: &str| value.get(k).and_then(Value::as_array).is_none_or(Vec::is_empty);
        if !empty("problems") || !empty("omissions") || digest(value["files"].as_array().map_or(&[][..], Vec::as_slice)) != js_string(&v["digest"]) {
            return failed("The uploaded files do not match the frozen manifest. The agent was not released.", "upload-mismatch", 409);
        }
        Ok(())
    }

    pub fn export(&mut self, id: &Value, revision: &Value) -> R<Value> {
        let v = self.load(id)?;
        let r = v.get("result").filter(|r| truthy(Some(r)));
        let Some(r) = r.filter(|r| xenonet_app::jsv::strict_eq(r.get("revision"), Some(revision))) else {
            return failed("The reviewed result changed. Open it again before exporting.", "changed", 409);
        };
        if !truthy(r.get("complete")) {
            return failed("Retrieval is incomplete. Resolve the reported files before exporting.", "incomplete", 409);
        }
        if r.get("changes").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            return fail("There are no text changes to export.");
        }
        let receipt = json!({
            "version": 1, "baseline": v.get("digest"), "review": id, "revision": revision, "retrievedAt": r.get("at"), "patchHash": r.get("patchHash"),
            "sourceStateAtRetrieval": r.get("sourceState"), "execution": "unknown", "scope": format!("Selected UTF-8 text only; file modes are not reviewed. {SCOPE_NOTE}"),
            "excluded": v.get("excluded"), "omittedPaths": v.get("omittedPaths").filter(|x| !x.is_null()).cloned().unwrap_or_else(|| json!([])), "omissions": r.get("omissions"),
        });
        let short: String = js_string(id).chars().take(8).collect();
        Ok(json!({ "filename": format!("xenonet-{short}.patch"), "patch": r.get("patch"), "receipt": receipt }))
    }
}

fn fail_of(f: Fail) -> Failure {
    if f.status == 0 {
        let mut m = Map::new();
        if !f.code.is_empty() {
            m.insert("code".into(), json!(f.code));
        }
        return Failure::with(f.message, m);
    }
    refused(f)
}

pub type Shared = Rc<RefCell<Reviews>>;

async fn snapshot(io: &Io, source: &Value) -> Result<Value, Failure> {
    io.ask(json!({ "op": "snapshot", "source": source })).await.map_err(|_| Failure::Rethrow)
}

/// prepare(source): the frozen baseline and its staged upload.
pub async fn prepare(io: Io, r: Shared, source: Value, env: Map<String, Value>) -> Result<Value, Failure> {
    let now = now_of(&io).await?;
    // Abandoned launch previews must not keep source code indefinitely.
    let root = r.borrow_mut().root().map_err(fail_of)?;
    for id in fsx::read_dir_names(&root).map_err(fs_fail)?.into_iter().filter(|n| is_uuid(n)) {
        let mut rv = r.borrow_mut();
        if let Ok(v) = rv.load(&json!(id)) {
            if !truthy(v.get("launched")) && xenonet_base::text::js_number(v.get("at")) + PREVIEW_MS <= now {
                let _ = rv.forget(&json!(id));
            }
        }
    }
    if r.borrow_mut().count().map_err(fail_of)? >= COUNT {
        return Err(fail_of(Fail { status: 409, code: "review-capacity".into(), message: "Local review storage is full. Forget an old review before preparing another.".into() }));
    }
    let selected = snapshot(&io, &source).await?;
    let sel_bytes = xenonet_base::text::js_number(selected.get("bytes"));
    if (r.borrow_mut().disk_bytes(None).map_err(fail_of)? as f64) + sel_bytes * 3.0 > STORE_BYTES as f64 {
        return Err(fail_of(Fail { status: 409, code: "review-capacity".into(), message: "Local review storage is full. Forget an old review before preparing another.".into() }));
    }
    let now = now_of(&io).await?;
    let mut m: Map<String, Value> = selected.as_object().cloned().unwrap_or_default();
    m.insert("id".into(), json!(b64::uuid()));
    m.insert("source".into(), source.clone());
    m.insert("at".into(), xenonet_base::json::number(now));
    m.insert("launched".into(), json!(false));
    m.insert("result".into(), Value::Null);
    let value = r.borrow_mut().save(&Value::Object(m)).map_err(fail_of)?;
    let folder = r.borrow_mut().location(&value["id"]).map_err(fail_of)?;
    let upload = NATIVE.join(&[&folder, "project"]);
    fsx::mkdir(&upload, Some(0o700)).map_err(fs_fail)?;
    for f in value["files"].as_array().into_iter().flatten() {
        let p = js_string(&f["path"]);
        let mut parts: Vec<&str> = vec![&upload];
        parts.extend(p.split('/'));
        let target = NATIVE.join(&parts);
        fsx::mkdir_p(&NATIVE.dirname(&target), Some(0o700)).map_err(fs_fail)?;
        write_new(&target, &b64::decode(&js_string(&f["content"])), 0o600).map_err(fs_fail)?;
    }
    if let Err(e) = prepare_ignores(&io, &r, &value, &env).await {
        let _ = r.borrow_mut().forget(&value["id"]);
        return Err(e);
    }
    let mut out: Map<String, Value> = Reviews::describe(&value).as_object().cloned().unwrap_or_default();
    out.insert("upload".into(), json!(upload));
    Ok(Value::Object(out))
}

async fn now_of(io: &Io) -> Result<f64, Failure> {
    io.ask(json!({ "op": "now" })).await.map(|v| xenonet_base::text::js_number(Some(&v))).map_err(|_| Failure::Rethrow)
}

async fn prepare_ignores(io: &Io, r: &Shared, v: &Value, env: &Map<String, Value>) -> Result<(), Failure> {
    let folder = NATIVE.join(&[&r.borrow_mut().location(&v["id"]).map_err(fail_of)?, "ignore"]);
    fsx::mkdir(&folder, Some(0o700)).map_err(fs_fail)?;
    for f in v["files"].as_array().into_iter().flatten() {
        let p = js_string(&f["path"]);
        if xenonet_app::nodepath::Style::Posix.basename(&p, None) != ".gitignore" {
            continue;
        }
        let mut parts: Vec<&str> = vec![&folder];
        parts.extend(p.split('/'));
        let dest = NATIVE.join(&parts);
        fsx::mkdir_p(&NATIVE.dirname(&dest), Some(0o700)).map_err(fs_fail)?;
        fsx::write_file(&dest, &b64::decode(&js_string(&f["content"])), Some(0o600)).map_err(fs_fail)?;
    }
    let g = r.borrow_mut().isolated_git(&folder, &["init", "--quiet", "--template="], env).map_err(fail_of)?;
    io.ask(json!({ "op": "git", "args": g["args"], "options": g["options"] })).await.map_err(|_| Failure::Rethrow)?;
    Ok(())
}

/// launch(id): the frozen upload checked once more, then marked launched.
pub async fn launch(io: Io, r: Shared, id: Value) -> Result<Value, Failure> {
    let v = r.borrow_mut().load(&id).map_err(fail_of)?;
    let now = now_of(&io).await?;
    let changed = |m: &str| fail_of(Fail { status: 409, code: "changed".into(), message: m.into() });
    if truthy(v.get("launched")) || now >= xenonet_base::text::js_number(v.get("at")) + PREVIEW_MS {
        return Err(fail_of(Fail { status: 409, code: "expired".into(), message: "The snapshot preview expired or was already launched.".into() }));
    }
    let fresh = snapshot(&io, v.get("source").unwrap_or(&Value::Null)).await?;
    if !xenonet_app::jsv::strict_eq(fresh.get("digest"), v.get("digest")) {
        return Err(changed("The project changed after preview. Preview the upload again."));
    }
    if !xenonet_app::jsv::strict_eq(r.borrow_mut().load(&id).map_err(fail_of)?.get("generation"), v.get("generation")) {
        return Err(changed("This local review changed or was forgotten."));
    }
    // Check the staged bytes too: the live project is never uploaded here.
    let project = NATIVE.join(&[&r.borrow_mut().location(&id).map_err(fail_of)?, "project"]);
    let files: Vec<Value> = v["files"].as_array().cloned().unwrap_or_default();
    for f in &files {
        let p = js_string(&f["path"]);
        let mut parts: Vec<&str> = vec![&project];
        parts.extend(p.split('/'));
        let file = NATIVE.join(&parts);
        let st = std::fs::symlink_metadata(&file).map_err(|e| fs_fail(fsx::FsError::of(e, "lstat", &file)))?;
        let same = st.is_file()
            && !st.file_type().is_symlink()
            && b64::encode(&std::fs::read(&file).map_err(|e| fs_fail(fsx::FsError::of(e, "open", &file)))?) == js_string(&f["content"]);
        if !same {
            return Err(changed("The frozen upload changed. Preview again."));
        }
    }
    fn walk(folder: &str, prefix: &str, files: &[Value], staged: &mut Vec<String>) -> Result<(), Failure> {
        let changed = |m: &str| fail_of(Fail { status: 409, code: "changed".into(), message: m.into() });
        for name in fsx::read_dir_names(folder).map_err(fs_fail)? {
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let full = NATIVE.join(&[folder, &name]);
            let m = std::fs::symlink_metadata(&full).map_err(|e| fs_fail(fsx::FsError::of(e, "lstat", &full)))?;
            if m.file_type().is_symlink() || (!m.is_file() && !m.is_dir()) {
                return Err(changed("The frozen upload contains an unexpected file or link. Preview again."));
            }
            if m.is_dir() {
                if !files.iter().any(|f| js_string(&f["path"]).starts_with(&format!("{rel}/"))) {
                    return Err(changed("The frozen upload contains an unexpected directory. Preview again."));
                }
                walk(&full, &rel, files, staged)?;
            } else {
                staged.push(rel);
            }
        }
        Ok(())
    }
    let mut staged = Vec::new();
    walk(&project, "", &files, &mut staged)?;
    staged.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut want: Vec<String> = files.iter().map(|f| js_string(&f["path"])).collect();
    want.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    if staged != want {
        return Err(changed("The frozen upload manifest changed. Preview again."));
    }
    let mut m: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    m.insert("launched".into(), json!(true));
    r.borrow_mut().save(&Value::Object(m)).map_err(fail_of)?;
    Ok(Value::Null)
}

// What came back, kept to the frozen upload's scope: new paths that the
// repository's own ignore rules, the omitted paths or the exclusions
// cover are left out, and said so.
async fn filter_scope(io: &Io, r: &Shared, v: &Value, value: &Value, env: &Map<String, Value>) -> Result<Value, Failure> {
    let baseline: Vec<String> = v["files"].as_array().into_iter().flatten().map(|f| js_string(&f["path"])).collect();
    let mut candidates: Vec<String> = Vec::new();
    for p in value["files"].as_array().into_iter().flatten().chain(value["problems"].as_array().into_iter().flatten()).map(|f| js_string(&f["path"])) {
        if !baseline.contains(&p) && !candidates.contains(&p) {
            candidates.push(p);
        }
    }
    if candidates.is_empty() {
        return Ok(value.clone());
    }
    let excluded: Vec<String> = v["excluded"].as_array().into_iter().flatten().map(|e| js_string(&e["path"]).to_lowercase()).collect();
    let omitted_paths: Vec<String> = v.get("omittedPaths").and_then(Value::as_array).into_iter().flatten().map(js_string).collect();
    let omitted = |name: &str| {
        let n = name.to_lowercase();
        omitted_paths.iter().any(|p| if p.ends_with('/') { n.starts_with(&p.to_lowercase()) } else { n == p.to_lowercase() })
            || excluded.iter().any(|p| n == *p || n.starts_with(&format!("{p}/")))
    };
    let mut ignored: Vec<String> = Vec::new();
    if !r.borrow().demo {
        let folder = NATIVE.join(&[&r.borrow_mut().location(&v["id"]).map_err(fail_of)?, "ignore"]);
        let g = r.borrow_mut().isolated_git(&folder, &["check-ignore", "--no-index", "--stdin", "-z"], env).map_err(fail_of)?;
        let input = format!("{}\0", candidates.join("\0"));
        // check-ignore exits 1 when nothing matches.
        let out = io.ask(json!({ "op": "git", "args": g["args"], "options": g["options"], "input": input, "allowExit": 1 })).await.map_err(|_| Failure::Rethrow)?;
        ignored = js_string(&out["stdout"]).split('\0').filter(|n| !n.is_empty()).map(String::from).collect();
    }
    let blocked = |name: &str| !baseline.iter().any(|b| b == name) && (ignored.iter().any(|i| i == name) || omitted(name));
    let keep = |list: &Value| -> Vec<Value> { list.as_array().into_iter().flatten().filter(|f| !blocked(&js_string(&f["path"]))).cloned().collect() };
    let mut omissions: Vec<Value> = value["omissions"].as_array().cloned().unwrap_or_default();
    omissions.extend(candidates.iter().filter(|c| blocked(c)).map(|p| json!({ "path": p, "reason": "Outside the frozen upload scope" })));
    if omissions.len() > FILES {
        return Err(fail_of(Fail { status: 502, code: "retrieval-invalid".into(), message: "Too many excluded result paths to retain a bounded review.".into() }));
    }
    Ok(json!({ "files": keep(&value["files"]), "problems": keep(&value["problems"]), "omissions": omissions }))
}

/// accept(id, input): what came back, validated and kept to scope, as the
/// review's result.
pub async fn accept(io: Io, r: Shared, id: Value, input: Value, env: Map<String, Value>) -> Result<Value, Failure> {
    let valid = validate_result(&input).map_err(fail_of)?;
    let v = r.borrow_mut().load(&id).map_err(fail_of)?;
    let value = filter_scope(&io, &r, &v, &valid, &env).await?;
    let state = match snapshot(&io, v.get("source").unwrap_or(&Value::Null)).await {
        Ok(s) if xenonet_app::jsv::strict_eq(s.get("digest"), v.get("digest")) => "unchanged",
        Ok(_) => "changed",
        Err(_) => "unknown",
    };
    if !xenonet_app::jsv::strict_eq(r.borrow_mut().load(&id).map_err(fail_of)?.get("generation"), v.get("generation")) {
        return Err(fail_of(Fail { status: 409, code: "changed".into(), message: "This local review changed or was forgotten.".into() }));
    }
    let comparison = text_patch(v["files"].as_array().map_or(&[][..], Vec::as_slice), value["files"].as_array().map_or(&[][..], Vec::as_slice));
    let complete = value["problems"].as_array().is_none_or(Vec::is_empty);
    let now = now_of(&io).await?;
    let mut res = Map::new();
    res.insert("revision".into(), json!(b64::uuid()));
    res.insert("at".into(), xenonet_base::json::number(now));
    res.insert("sourceState".into(), json!(state));
    res.insert("complete".into(), json!(complete));
    res.insert("omissions".into(), value["omissions"].clone());
    res.insert("problems".into(), value["problems"].clone());
    for (k, x) in comparison.as_object().into_iter().flatten() {
        res.insert(k.clone(), x.clone());
    }
    res.insert("patchHash".into(), json!(hash(&js_string(&comparison["patch"]))));
    res.insert("execution".into(), json!("unknown"));
    let result = Value::Object(res);
    let mut m: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    m.insert("result".into(), result.clone());
    r.borrow_mut().save(&Value::Object(m)).map_err(fail_of)?;
    Ok(json!({ "baseline": Reviews::describe(&v), "result": result }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_portable_paths() {
        assert!(excluded_path("a/.ENV.local") && excluded_path("keys/server.PEM") && excluded_path("node_modules/x") && excluded_path("secret"));
        assert!(!excluded_path("src/environment.ts") && !excluded_path("secretary.txt"));
        assert!(
            review_path(&json!("src/main.rs")) && !review_path(&json!("CON.txt")) && !review_path(&json!("a/../b")) && !review_path(&json!("x.")) && review_path(&json!("com0"))
        );
        assert!(!portable_set(&["A.txt".into(), "a.txt".into()]) && !portable_set(&["a".into(), "a/b".into()]));
    }

    #[test]
    fn a_patch_as_git_writes_it() {
        let enc = |s: &str| b64::encode(s.as_bytes());
        let p = text_patch(&[json!({ "path": "a.txt", "content": enc("one\n") })], &[json!({ "path": "a.txt", "content": enc("two") }), json!({ "path": "e", "content": "" })]);
        assert_eq!(p["patch"], json!("diff --git \"a/a.txt\" \"b/a.txt\"\n--- \"a/a.txt\"\n+++ \"b/a.txt\"\n@@ -1,1 +1,1 @@\n-one\n+two\n\\ No newline at end of file\ndiff --git \"a/e\" \"b/e\"\nnew file mode 100644\nindex 0000000..e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\n"));
    }
}
