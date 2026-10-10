// Folders of made-up files for tests: real files in a temp folder, with the
// mtimes a test sets, and a stand-in for fs.watch that sends the events a
// test says, when it says. Nothing here reads a real home folder.
//
//   const tree = tempTree();
//   tree.write("claude/projects/-p/s1.jsonl", lines, { mtime: "2026-10-04T12:00:00Z" });
//   tree.mkdir("codex/sessions/2026/10/05", { mtime: clock.now() });
//   const watcher = fakeWatch();
//   new Tailer({ ..., watch: watcher.watch });
//   watcher.emit(tree.path("claude/projects/-p/s1.jsonl"));
//   tree.cleanup();
import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const toDate = (t) => (t instanceof Date ? t : new Date(typeof t === "number" ? t : Date.parse(t)));
const jsonl = (records) => records.map((r) => (typeof r === "string" ? r : JSON.stringify(r)) + "\n").join("");

// Every tree a test made, for cleanupAll() in an after() hook.
const made = new Set();

export function tempTree(prefix = "xenonet-tree-") {
  // The real path: on macOS the temp folder is a link into /private, and
  // what XenoNet reads keeps real paths.
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), prefix)));
  made.add(root);
  const at = (rel = "") => path.join(root, ...String(rel).split("/").filter(Boolean));
  const stamp = (rel, mtime) => {
    if (mtime == null) return;
    const t = toDate(mtime);
    fs.utimesSync(at(rel), t, t);
  };
  const tree = {
    root,
    path: at,
    // A file with `text` (a string, a Buffer, or records written as JSON
    // lines), its folders made on the way.
    write(rel, text = "", { mtime } = {}) {
      fs.mkdirSync(path.dirname(at(rel)), { recursive: true });
      fs.writeFileSync(at(rel), Array.isArray(text) ? jsonl(text) : text);
      stamp(rel, mtime);
      return at(rel);
    },
    append(rel, text, { mtime } = {}) {
      fs.mkdirSync(path.dirname(at(rel)), { recursive: true });
      fs.appendFileSync(at(rel), Array.isArray(text) ? jsonl(text) : text);
      stamp(rel, mtime);
      return at(rel);
    },
    mkdir(rel, { mtime } = {}) {
      fs.mkdirSync(at(rel), { recursive: true });
      stamp(rel, mtime);
      return at(rel);
    },
    // Sets a file's or folder's mtime (a Date, ms or an ISO string).
    touch(rel, mtime) {
      stamp(rel, mtime);
      return at(rel);
    },
    remove(rel) {
      fs.rmSync(at(rel), { recursive: true, force: true });
    },
    read: (rel) => fs.readFileSync(at(rel), "utf8"),
    exists: (rel) => fs.existsSync(at(rel)),
    stat: (rel) => fs.statSync(at(rel)),
    // Every file under the tree, relative, sorted.
    files() {
      const out = [];
      const walk = (dir) => {
        for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
          const full = path.join(dir, e.name);
          if (e.isDirectory()) walk(full);
          else out.push(path.relative(root, full).split(path.sep).join("/"));
        }
      };
      walk(root);
      return out.sort();
    },
    cleanup() {
      made.delete(root);
      fs.rmSync(root, { recursive: true, force: true, maxRetries: 10 });
    },
  };
  return tree;
}

export const cleanupAll = () => [...made].forEach((root) => (made.delete(root), fs.rmSync(root, { recursive: true, force: true, maxRetries: 10 })));
export { jsonl };

// A stand-in for fs.watch(dir, { recursive }, listener). Nothing is watched
// for real: emit(file) calls each watcher whose folder holds the file (any
// level down when recursive) with the file's name relative to it, as Node
// does; error() sends an error to every watcher; refuse(dir) makes watching
// that folder throw, as it can on some file systems.
export function fakeWatch() {
  const watchers = new Set();
  const refused = new Set();
  function watch(dir, options, listener) {
    if (typeof options === "function") [options, listener] = [{}, options];
    if (refused.has(dir)) throw Object.assign(new Error(`EMFILE: too many open files, watch '${dir}'`), { code: "EMFILE" });
    const w = new EventEmitter();
    Object.assign(w, { dir, recursive: !!options?.recursive, listener, closed: false });
    w.close = () => {
      w.closed = true;
      watchers.delete(w);
    };
    watchers.add(w);
    return w;
  }
  return {
    watch,
    watching: () => [...watchers].map((w) => w.dir),
    emit(file, event = "change") {
      for (const w of watchers) {
        const rel = path.relative(w.dir, file);
        if (!rel || rel.startsWith("..") || path.isAbsolute(rel)) continue;
        if (!w.recursive && rel.includes(path.sep)) continue;
        w.listener?.(event, rel);
      }
    },
    error(err = new Error("watch failed")) {
      for (const w of watchers) w.emit("error", err);
    },
    refuse: (dir) => refused.add(dir),
  };
}
