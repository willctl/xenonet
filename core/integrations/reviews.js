import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash, randomUUID } from "node:crypto";
import { fail } from "./common.js";

const execute = promisify(execFile);
export const REVIEW_LIMITS = { files: 512, fileBytes: 256 * 1024, totalBytes: 4 * 1024 * 1024, depth: 24 };
export const REVIEW_STORAGE = { count: 32, recordBytes: 24 * 1024 * 1024, totalBytes: 128 * 1024 * 1024, cached: 4 };
const uuid = /^[a-f0-9-]{36}$/;
const hash = (value) => createHash("sha256").update(value).digest("hex");
const digest = (files) => hash(JSON.stringify(files.map(({ path, content }) => [path, content]).sort(([a], [b]) => a.localeCompare(b))));

// The same scope rules run on the host and in the untrusted sandbox. They
// reduce accidental uploads; the manifest remains the owner's final check.
export function excludedPath(name) {
  return name.split("/").some((part) => /^(?:\.git|node_modules|\.ssh|\.aws|\.azure|\.gnupg|\.venv|venv|__pycache__|\.cache|\.xenonet)$/i.test(part)
    || /^\.env(?:\..*)?$/i.test(part) || /^(?:\.npmrc|\.pypirc|\.netrc|\.git-credentials|credentials(?:\.json)?|secrets?(?:\..*)?|id_rsa|id_ed25519)$/i.test(part)
    || /\.(?:pem|key|p12|pfx|keystore)$/i.test(part));
}
export function reviewPath(name) {
  if (typeof name !== "string" || !name || name.length > 240 || !/^[a-zA-Z0-9_.@ +\/-]+$/.test(name)) return false;
  const parts = name.split("/");
  return parts.length <= REVIEW_LIMITS.depth && parts.every((part) => part && part !== "." && part !== ".." && !/[. ]$/.test(part) && !/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part));
}
function textBytes(bytes) {
  if (bytes.includes(0)) return false;
  try { new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes); return true; } catch { return false; }
}
const inside = (base, file) => file.startsWith(`${base}${path.sep}`);
const stamp = (s) => `${s.dev}:${s.ino}:${s.size}:${s.mtimeMs}:${s.ctimeMs}`;
function portableSet(names) {
  const prefixes = new Map();
  for (const name of names) {
    const parts = name.split("/"); let prefix = "";
    for (let i = 0; i < parts.length; i++) {
      prefix += `${i ? "/" : ""}${parts[i]}`;
      const key = prefix.toLowerCase(), kind = i === parts.length - 1 ? "file" : "directory", seen = prefixes.get(key);
      if (seen && (seen.name !== prefix || seen.kind !== kind || kind === "file")) return false;
      prefixes.set(key, { name: prefix, kind });
    }
  }
  return true;
}

export async function sourceSnapshot(source, { git = execute } = {}) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")));
  const args = ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=", "-C", source];
  const { stdout: root } = await git("git", [...args, "rev-parse", "--show-toplevel"], { env, timeout: 10000, maxBuffer: 4096 });
  if (fs.realpathSync(String(root).trim()) !== source) fail("Change review needs the root of a Git working tree.");
  const { stdout } = await git("git", [...args, "ls-files", "--cached", "--others", "--exclude-standard", "-z"], { env, timeout: 10000, maxBuffer: 1024 * 1024 });
  const names = [...new Set(String(stdout).split("\0").filter(Boolean))].sort();
  if (names.length > 10000) fail("This project has too many files to prepare a bounded review. Choose a smaller project.");
  const files = [], excluded = []; let bytes = 0;
  const omit = (name, reason) => { if (excluded.length >= REVIEW_LIMITS.files) fail("Too many exclusions to review. Choose a smaller project."); excluded.push({ path: name.slice(0, 240), reason }); };
  for (const name of names) {
    if (excludedPath(name)) { omit(name, "Excluded from upload and review scope"); continue; }
    if (!reviewPath(name)) { omit(name, "Unsupported portable path"); continue; }
    const file = path.join(source, ...name.split("/"));
    let st; try { st = fs.lstatSync(file); } catch (err) { if (err.code === "ENOENT") continue; throw err; }
    const ignoreRule = path.basename(name) === ".gitignore";
    if (!st.isFile() || st.isSymbolicLink() || !inside(source, fs.realpathSync(file))) { if (ignoreRule) fail("A repository .gitignore is a link or unsupported file. Change review cannot freeze its rules."); omit(name, "Links and non-regular files are excluded"); continue; }
    if (st.size > REVIEW_LIMITS.fileBytes) { if (ignoreRule) fail("A repository .gitignore exceeds the review limit."); omit(name, "Larger than the per-file limit"); continue; }
    // Every parent must still be a real directory inside the source.
    let parent = path.dirname(file), safe = true;
    while (parent !== source) { if (fs.lstatSync(parent).isSymbolicLink()) { safe = false; break; } parent = path.dirname(parent); }
    if (!safe) { omit(name, "A parent directory is a link"); continue; }
    const fd = fs.openSync(file, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
    let content;
    try {
      if (stamp(fs.fstatSync(fd)) !== stamp(st)) fail("The project changed while preparing its snapshot. Preview again.", "changed", 409);
      content = Buffer.alloc(st.size);
      let read = 0; while (read < content.length) { const n = fs.readSync(fd, content, read, content.length - read, read); if (!n) break; read += n; }
      if (read !== st.size || stamp(fs.fstatSync(fd)) !== stamp(st)) fail("The project changed while preparing its snapshot. Preview again.", "changed", 409);
    } finally { fs.closeSync(fd); }
    if (!textBytes(content)) { if (ignoreRule) fail("A repository .gitignore is not valid UTF-8 text."); omit(name, "Binary or invalid UTF-8 file"); continue; }
    bytes += content.length;
    if (bytes > REVIEW_LIMITS.totalBytes || files.length >= REVIEW_LIMITS.files) fail("The selected text exceeds the review limits (512 files, 4 MiB total).");
    files.push({ path: name, content: content.toString("base64") });
  }
  if (!files.length) fail("No supported text files remain in this Git project.");
  if (!portableSet(files.map((f) => f.path))) fail("The project has paths that collide on another platform. Rename them before preparing a review.");
  const ignored = await git("git", [...args, "ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "--no-empty-directory", "-z"], { env, timeout: 10000, maxBuffer: 128 * 1024 });
  const omittedPaths = String(ignored.stdout).split("\0").filter(Boolean);
  if (omittedPaths.length > REVIEW_LIMITS.files || omittedPaths.some((n) => n.length > 241)) fail("Too many existing ignored paths to freeze the review scope.");
  return { files, excluded, omittedPaths, bytes, digest: digest(files) };
}

// This program only reads. No archive is extracted and no sandbox-provided
// path is ever used as a host destination. Output is validated again below.
function collectText(root, limits, excluded, portable) {
  const fs = require("node:fs"), path = require("node:path");
  const files = [], omissions = [], problems = [], stamps = [], directories = []; let bytes = 0, entries = 0;
  const bad = (name, reason) => { if (problems.length < 30) problems.push({ path: name.slice(0, 240), reason }); };
  const mark = (s) => `${s.dev}:${s.ino}:${s.size}:${s.mtimeMs}:${s.ctimeMs}`;
  const base = fs.realpathSync(root);
  if (base !== root || !fs.lstatSync(root).isDirectory()) throw new Error("Review root is not a real directory.");
  function walk(dir, prefix = "", depth = 0) {
    if (depth > limits.depth) { bad(prefix, "Directory depth exceeded"); return; }
    const names = fs.readdirSync(dir).sort(); directories.push([dir, JSON.stringify(names)]);
    for (const name of names) {
      if (++entries > 2048) throw new Error("Review entry limit exceeded.");
      const rel = prefix ? `${prefix}/${name}` : name, file = path.join(dir, name);
      if (excluded(rel)) { if (omissions.length >= limits.files) throw new Error("Too many exclusions."); omissions.push({ path: rel.slice(0, 240), reason: "Outside review scope" }); continue; }
      if (!portable(rel)) { bad(rel, "Unsupported portable path"); continue; }
      const st = fs.lstatSync(file);
      if (st.isSymbolicLink()) { bad(rel, "Links cannot be retrieved"); continue; }
      if (st.isDirectory()) { walk(file, rel, depth + 1); continue; }
      if (!st.isFile() || st.size > limits.fileBytes) { bad(rel, "Non-regular or oversized file"); continue; }
      bytes += st.size;
      if (bytes > limits.totalBytes || files.length >= limits.files) throw new Error("Review size limit exceeded.");
      const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
      let data;
      try {
        const actual = fs.realpathSync(`/proc/self/fd/${fd}`);
        if (!actual.startsWith(`${base}/`) || mark(fs.fstatSync(fd)) !== mark(st)) throw new Error("File changed or escaped review root.");
        data = Buffer.alloc(st.size); let read = 0;
        while (read < data.length) { const n = fs.readSync(fd, data, read, data.length - read, read); if (!n) break; read += n; }
        if (read !== st.size || mark(fs.fstatSync(fd)) !== mark(st)) throw new Error("File changed during retrieval.");
      } finally { fs.closeSync(fd); }
      stamps.push([file, mark(st)]);
      try { new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(data); if (data.includes(0)) throw new Error("binary"); }
      catch { bad(rel, "Binary or invalid UTF-8 file"); continue; }
      files.push({ path: rel, content: data.toString("base64") });
    }
  }
  walk(root);
  for (const [file, before] of stamps) if (mark(fs.lstatSync(file)) !== before) throw new Error("Files changed during retrieval.");
  for (const [dir, before] of directories) if (JSON.stringify(fs.readdirSync(dir).sort()) !== before) throw new Error("Files changed during retrieval.");
  return { files, omissions, problems };
}
export const COLLECT_TEXT = `const REVIEW_LIMITS=${JSON.stringify(REVIEW_LIMITS)}; const excluded=${excludedPath.toString()}; const portable=${reviewPath.toString()}; process.stdout.write(JSON.stringify((${collectText.toString()})("/workspace/project", REVIEW_LIMITS, excluded, portable)));`;

function validateResult(value) {
  if (!value || !Array.isArray(value.files) || value.files.length > REVIEW_LIMITS.files || !Array.isArray(value.problems) || !Array.isArray(value.omissions) || value.problems.length > 30 || value.omissions.length > REVIEW_LIMITS.files) fail("The gateway returned an invalid text snapshot.", "retrieval-invalid", 502);
  const seen = new Set(); let total = 0;
  if (!value.files.every((f) => f && reviewPath(f.path)) || !portableSet(value.files.map((f) => f.path))) fail("The gateway returned colliding or unsupported file paths.", "retrieval-invalid", 502);
  for (const file of value.files) {
    if (!reviewPath(file.path) || excludedPath(file.path) || typeof file.content !== "string" || file.content.length > Math.ceil(REVIEW_LIMITS.fileBytes / 3) * 4) fail("The gateway returned an unsupported file.", "retrieval-invalid", 502);
    const key = file.path.toLowerCase();
    if (seen.has(key)) fail("The gateway returned colliding file paths.", "retrieval-invalid", 502); seen.add(key);
    const bytes = Buffer.from(file.content, "base64"); total += bytes.length;
    if (bytes.toString("base64") !== file.content || bytes.length > REVIEW_LIMITS.fileBytes || total > REVIEW_LIMITS.totalBytes || !textBytes(bytes)) fail("The gateway returned invalid or oversized text.", "retrieval-invalid", 502);
  }
  // A file cannot also be the parent of another returned file.
  for (const key of seen) for (let part = key.lastIndexOf("/"); part > 0; part = key.lastIndexOf("/", part - 1)) if (seen.has(key.slice(0, part))) fail("The gateway returned colliding file paths.", "retrieval-invalid", 502);
  for (const entry of [...value.problems, ...value.omissions]) if (!entry || typeof entry.path !== "string" || entry.path.length > 240 || typeof entry.reason !== "string" || entry.reason.length > 160) fail("The gateway returned invalid retrieval diagnostics.", "retrieval-invalid", 502);
  return value;
}
const quote = (name) => JSON.stringify(name);
function lines(content, prefix) {
  if (!content) return "";
  const data = content.split("\n"), newline = data.at(-1) === ""; if (newline) data.pop();
  return data.map((line) => `${prefix}${line}\n`).join("") + (newline ? "" : "\\ No newline at end of file\n");
}
const lineCount = (value) => value ? value.split("\n").length - (value.endsWith("\n") ? 1 : 0) : 0;
export function textPatch(before, after) {
  const old = new Map(before.map((f) => [f.path, f.content])), next = new Map(after.map((f) => [f.path, f.content]));
  const changes = [];
  for (const name of [...new Set([...old.keys(), ...next.keys()])].sort()) {
    if (old.get(name) === next.get(name)) continue;
    const a = old.has(name) ? Buffer.from(old.get(name), "base64").toString("utf8") : "", b = next.has(name) ? Buffer.from(next.get(name), "base64").toString("utf8") : "";
    const removed = lineCount(a), added = lineCount(b), status = !old.has(name) ? "added" : !next.has(name) ? "deleted" : "modified";
    let diff = `diff --git ${quote(`a/${name}`)} ${quote(`b/${name}`)}\n`;
    if (status === "added") diff += "new file mode 100644\n";
    if (status === "deleted") diff += "deleted file mode 100644\n";
    if (!removed && !added) {
      const blob = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
      diff += status === "added" ? `index 0000000..${blob}\n` : `index ${blob}..0000000\n`;
    } else {
      diff += `--- ${old.has(name) ? quote(`a/${name}`) : "/dev/null"}\n+++ ${next.has(name) ? quote(`b/${name}`) : "/dev/null"}\n`;
      diff += `@@ -${removed ? 1 : 0},${removed} +${added ? 1 : 0},${added} @@\n${lines(a, "-")}${lines(b, "+")}`;
    }
    changes.push({ path: name, status, added, removed, diff });
  }
  return { changes, patch: changes.map((f) => f.diff).join("") };
}

export function createReviews({ dir = null, now = Date.now, snapshot = sourceSnapshot, demo = false } = {}) {
  const memory = new Map();
  const root = () => { if (!dir) dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-reviews-")); fs.mkdirSync(dir, { recursive: true, mode: 0o700 }); return dir; };
  const location = (id) => { if (!uuid.test(id ?? "")) fail("No such local review.", "missing", 404); return path.join(root(), id); };
  const cache = (value) => { memory.delete(value.id); memory.set(value.id, value); while (memory.size > REVIEW_STORAGE.cached) memory.delete(memory.keys().next().value); return value; };
  function diskBytes(folder = root()) {
    let total = 0;
    for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
      const file = path.join(folder, entry.name);
      if (entry.isSymbolicLink()) fail("The review storage contains an unexpected link.", "storage-invalid", 409);
      if (entry.isDirectory()) total += diskBytes(file); else if (entry.isFile()) total += fs.lstatSync(file).size;
      if (total > REVIEW_STORAGE.totalBytes) break;
    }
    return total;
  }
  const save = (value) => {
    value = { ...value, generation: randomUUID() };
    const serialized = JSON.stringify(value), bytes = Buffer.byteLength(serialized), folder = location(value.id);
    if (bytes > REVIEW_STORAGE.recordBytes) fail("This diff is too large to retain safely. Retrieve a smaller text change; the previous local review is retained.", "review-too-large", 413);
    const existing = path.join(folder, "review.json"), oldBytes = fs.existsSync(existing) ? fs.lstatSync(existing).size : 0;
    if (!oldBytes && fs.readdirSync(root()).filter((n) => uuid.test(n)).length >= REVIEW_STORAGE.count) fail("Local review storage is full. Forget an old review before preparing another.", "review-capacity", 409);
    if (diskBytes() - oldBytes + bytes > REVIEW_STORAGE.totalBytes) fail("Local review storage is full. Forget an old review first; existing reviews are retained.", "review-capacity", 409);
    fs.mkdirSync(folder, { recursive: true, mode: 0o700 });
    const tmp = path.join(folder, `${randomUUID()}.tmp`);
    try { fs.writeFileSync(tmp, serialized, { flag: "wx", mode: 0o600 }); fs.renameSync(tmp, existing); }
    finally { try { fs.unlinkSync(tmp); } catch {} }
    return cache(value);
  };
  const load = (id) => {
    if (memory.has(id)) return cache(memory.get(id));
    const file = path.join(location(id), "review.json");
    try { const st = fs.lstatSync(file); if (!st.isFile() || st.isSymbolicLink() || st.size > REVIEW_STORAGE.recordBytes) throw new Error(); const v = JSON.parse(fs.readFileSync(file, "utf8")); if (v.id !== id) throw new Error(); return cache(v); } catch { fail("No local baseline remains for this job.", "missing", 404); }
  };
  const scopeNote = "New files use frozen repository .gitignore rules. Existing omitted paths stay excluded. Global and local-only ignore rules are not applied to newly created paths.";
  const describe = (v) => ({ id: v.id, digest: v.digest, at: v.at, bytes: v.bytes, files: v.files.map((f) => ({ path: f.path, bytes: Buffer.byteLength(f.content, "base64") })), excluded: v.excluded, omittedPaths: v.omittedPaths ?? [], scopeNote, limits: REVIEW_LIMITS });
  const isolatedGit = (folder, args) => ({ args: ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=", "-c", `core.excludesFile=${os.devNull}`, "-C", folder, ...args], options: { env: { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GIT_"))), GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: os.devNull, GIT_CONFIG_SYSTEM: os.devNull }, timeout: 10000, maxBuffer: 256 * 1024 } });
  async function prepareIgnores(v) {
    const folder = path.join(location(v.id), "ignore"); fs.mkdirSync(folder, { mode: 0o700 });
    for (const f of v.files.filter((f) => path.posix.basename(f.path) === ".gitignore")) { const dest = path.join(folder, ...f.path.split("/")); fs.mkdirSync(path.dirname(dest), { recursive: true, mode: 0o700 }); fs.writeFileSync(dest, Buffer.from(f.content, "base64"), { mode: 0o600 }); }
    const { args, options } = isolatedGit(folder, ["init", "--quiet", "--template="]); await execute("git", args, options);
  }
  async function filterScope(v, value) {
    const baseline = new Set(v.files.map((f) => f.path));
    const candidates = [...new Set([...value.files.map((f) => f.path), ...value.problems.map((f) => f.path)].filter((name) => !baseline.has(name)))];
    if (!candidates.length) return value;
    const excluded = new Set(v.excluded.map((e) => e.path.toLowerCase()));
    const omitted = (name) => (v.omittedPaths ?? []).some((p) => p.endsWith("/") ? name.toLowerCase().startsWith(p.toLowerCase()) : name.toLowerCase() === p.toLowerCase()) || [...excluded].some((p) => name.toLowerCase() === p || name.toLowerCase().startsWith(`${p}/`));
    const folder = path.join(location(v.id), "ignore");
    // Demo fixtures have no live project or ignore repository to read.
    let ignored = new Set();
    if (!demo) {
      const { args, options } = isolatedGit(folder, ["check-ignore", "--no-index", "--stdin", "-z"]);
      const pending = execute("git", args, options); pending.child.stdin.end(`${candidates.join("\0")}\0`);
      let stdout; try { ({ stdout } = await pending); } catch (err) { if (err.code !== 1) throw err; stdout = err.stdout; }
      ignored = new Set(String(stdout).split("\0").filter(Boolean));
    }
    const blocked = (name) => !baseline.has(name) && (ignored.has(name) || omitted(name));
    const files = value.files.filter((f) => !blocked(f.path)), problems = value.problems.filter((p) => !blocked(p.path));
    const omissions = [...value.omissions, ...candidates.filter(blocked).map((path) => ({ path, reason: "Outside the frozen upload scope" }))];
    if (omissions.length > REVIEW_LIMITS.files) fail("Too many excluded result paths to retain a bounded review.", "retrieval-invalid", 502);
    return { files, problems, omissions };
  }
  function forget(id) { memory.delete(id); fs.rmSync(location(id), { recursive: true, force: true }); return { forgotten: true }; }
  return {
    seed({ before, after, sourceState = "unchanged", problems = [], unavailable = false }) {
      if (!demo) fail("Synthetic reviews belong to the demo instance.");
      const encode = (entries) => Object.entries(entries).map(([path, text]) => ({ path, content: Buffer.from(text).toString("base64") }));
      const files = encode(before), comparison = textPatch(files, encode(after));
      const result = unavailable ? null : { ...comparison, revision: randomUUID(), at: now(), sourceState, complete: problems.length === 0, problems, omissions: [], execution: "unknown", patchHash: hash(comparison.patch) };
      return save({ id: randomUUID(), at: now(), launched: true, source: "", files, excluded: [], bytes: files.reduce((n, f) => n + Buffer.byteLength(f.content, "base64"), 0), digest: digest(files), result }).id;
    },
    async prepare(source) {
      // Abandoned launch previews must not retain source code indefinitely.
      for (const id of fs.readdirSync(root()).filter((n) => uuid.test(n))) { try { const v = load(id); if (!v.launched && v.at + 10 * 60e3 <= now()) forget(id); } catch {} }
      if (fs.readdirSync(root()).filter((n) => uuid.test(n)).length >= REVIEW_STORAGE.count) fail("Local review storage is full. Forget an old review before preparing another.", "review-capacity", 409);
      const selected = await snapshot(source);
      if (diskBytes() + selected.bytes * 3 > REVIEW_STORAGE.totalBytes) fail("Local review storage is full. Forget an old review before preparing another.", "review-capacity", 409);
      const value = save({ ...selected, id: randomUUID(), source, at: now(), launched: false, result: null });
      const upload = path.join(location(value.id), "project"); fs.mkdirSync(upload, { mode: 0o700 });
      for (const file of value.files) { const target = path.join(upload, ...file.path.split("/")); fs.mkdirSync(path.dirname(target), { recursive: true, mode: 0o700 }); fs.writeFileSync(target, Buffer.from(file.content, "base64"), { flag: "wx", mode: 0o600 }); }
      try { await prepareIgnores(value); } catch (err) { forget(value.id); throw err; }
      return { ...describe(value), upload };
    },
    async launch(id) {
      const v = load(id);
      if (v.launched || now() >= v.at + 10 * 60e3) fail("The snapshot preview expired or was already launched.", "expired", 409);
      if ((await snapshot(v.source)).digest !== v.digest) fail("The project changed after preview. Preview the upload again.", "changed", 409);
      if (load(id).generation !== v.generation) fail("This local review changed or was forgotten.", "changed", 409);
      // Check the staged bytes too: the live project is never uploaded here.
      for (const f of v.files) { const file = path.join(location(id), "project", ...f.path.split("/")); const st = fs.lstatSync(file); if (!st.isFile() || st.isSymbolicLink() || fs.readFileSync(file).toString("base64") !== f.content) fail("The frozen upload changed. Preview again.", "changed", 409); }
      const staged = [];
      function walk(folder, prefix = "") {
        for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
          const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
          if (entry.isSymbolicLink() || !entry.isFile() && !entry.isDirectory()) fail("The frozen upload contains an unexpected file or link. Preview again.", "changed", 409);
          if (entry.isDirectory()) { if (!v.files.some((f) => f.path.startsWith(`${rel}/`))) fail("The frozen upload contains an unexpected directory. Preview again.", "changed", 409); walk(path.join(folder, entry.name), rel); }
          else staged.push(rel);
        }
      }
      walk(path.join(location(id), "project"));
      if (JSON.stringify(staged.sort()) !== JSON.stringify(v.files.map((f) => f.path).sort())) fail("The frozen upload manifest changed. Preview again.", "changed", 409);
      save({ ...v, launched: true });
    },
    view(id) { const v = load(id); return { baseline: describe(v), result: v.result }; },
    verifyUploaded(id, input) {
      const value = validateResult(input), v = load(id);
      if (value.problems.length || value.omissions.length || digest(value.files) !== v.digest) fail("The uploaded files do not match the frozen manifest. The agent was not released.", "upload-mismatch", 409);
    },
    async accept(id, input) {
      const valid = validateResult(input), v = load(id);
      const value = await filterScope(v, valid);
      let sourceState = "unknown"; try { sourceState = (await snapshot(v.source)).digest === v.digest ? "unchanged" : "changed"; } catch {}
      if (load(id).generation !== v.generation) fail("This local review changed or was forgotten.", "changed", 409);
      const comparison = textPatch(v.files, value.files);
      const complete = value.problems.length === 0;
      const result = { revision: randomUUID(), at: now(), sourceState, complete, omissions: value.omissions, problems: value.problems, ...comparison, patchHash: hash(comparison.patch), execution: "unknown" };
      save({ ...v, result }); return { baseline: describe(v), result };
    },
    export(id, revision) {
      const v = load(id), r = v.result;
      if (!r || r.revision !== revision) fail("The reviewed result changed. Open it again before exporting.", "changed", 409);
      if (!r.complete) fail("Retrieval is incomplete. Resolve the reported files before exporting.", "incomplete", 409);
      if (!r.changes.length) fail("There are no text changes to export.");
      const receipt = { version: 1, baseline: v.digest, review: id, revision, retrievedAt: r.at, patchHash: r.patchHash, sourceStateAtRetrieval: r.sourceState, execution: "unknown", scope: `Selected UTF-8 text only; file modes are not reviewed. ${scopeNote}`, excluded: v.excluded, omittedPaths: v.omittedPaths ?? [], omissions: r.omissions };
      return { filename: `xenonet-${id.slice(0, 8)}.patch`, patch: r.patch, receipt };
    },
    forget,
  };
}
