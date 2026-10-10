// A scanner walks a folder for the files it wants, within a budget of
// folders read, levels deep and time spent, and remembers each folder's
// mtime, so the next walk reads only the folders that changed.
//
//   const scanner = new Scanner({ name: "transcripts", root, depth: 6, pick: (name) => name.endsWith(".jsonl") });
//   scanner.scan(); // every transcript, the first time
//   scanner.scan(); // then only those in folders that changed since
//
// A folder's mtime changes when a file is added to it, removed or renamed,
// not when a file in it is written to, so a walk with the cache finds new
// files at the cost of a stat for each folder that didn't change. Folders
// that went are forgotten, with everything under them.
//
// scan() walks depth first, in the order each folder lists its entries: a
// folder's files, then its subfolders. It's synchronous: a stat or a
// readdir takes microseconds, and a trip to the thread pool for each costs
// many times that.
//
// scanBreadth() walks breadth first and asynchronously, a level at a time,
// several folders read at once, handing each folder's entries to `look`:
// for a walk the server makes while it serves, which a synchronous one
// would hold up (the projects scan reads up to 2,000 folders on every
// refresh), and for a walk that wants the shallow levels first when its
// budget runs out.
import nodeFs from "node:fs";
import path from "node:path";
import { Block, messageOf } from "./lifecycle.js";

// fn over the items, n at a time, with the results in order.
async function inTurn(items, n, fn) {
  const out = new Array(items.length);
  let next = 0;
  const worker = async () => {
    for (let i = next++; i < items.length; i = next++) out[i] = await fn(items[i], i);
  };
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, worker));
  return out;
}

export class Scanner extends Block {
  // `root`: the folder walked. `depth`: how many levels below it the walk
  // goes. `folders`: how many folders one walk may read. `ms`: how long one
  // walk may take. `pick(name, dir)`: the files wanted. `enter(name, dir,
  // level)`: whether to walk into a subfolder. `cache`: read only what
  // changed (true), or every folder on every walk. For the breadth-first
  // walk, `look(dir, entries, level)` is handed each folder read, and
  // `concurrency` folders are read at once.
  constructor({ name, root, depth = Infinity, folders = Infinity, ms = Infinity, pick = () => true, enter = () => true, look = null, concurrency = 16, cache = true, fs = nodeFs, clock } = {}) {
    super({ name, kind: "scanner", clock });
    this.root = root;
    this.limits = { depth, folders, ms };
    this.pick = pick;
    this.enter = enter;
    this.look = look;
    this.concurrency = concurrency;
    this.useCache = cache;
    this.fs = fs;
    // folder → { mtimeMs, subdirs }, from the last time it was read.
    this.folders = new Map();
    this.last = null;
    this.budget = { depth, folders, ms, last: null };
  }

  // Forgets every folder, so the next walk reads them all.
  forget() {
    this.folders.clear();
  }

  // The files picked in the folders this walk read.
  scan() {
    const started = this.clock.now();
    const run = { files: 0, read: 0, seen: 0, truncated: null, problem: null, started };
    const out = [];
    this.#walk(this.root, 0, out, run);
    this.#report(run, this.limits);
    return out;
  }

  // How a walk went: `last`, the counters, and the block's state.
  #report(run, limits) {
    const started = run.started;
    run.ms = this.clock.now() - started;
    this.last = { files: run.files, read: run.read, seen: run.seen, truncated: run.truncated, ms: run.ms };
    this.budget.last = this.last;
    this.count("scans");
    this.count("read", run.read);
    // A walk stopped as no longer wanted says nothing of the scanner.
    if (run.truncated === "cancelled") return;
    if (run.truncated) this.problem(run.truncated === "folders" ? `stopped at the ${limits.folders}-folder budget` : `stopped after ${limits.ms} ms`, started);
    else if (run.problem) this.problem(run.problem, started);
    else this.ok(started);
  }

  #walk(dir, level, out, run) {
    if (run.truncated) return;
    let st;
    try {
      st = this.fs.statSync(dir);
    } catch {
      this.folders.delete(dir);
      return;
    }
    run.seen++;
    let f = this.folders.get(dir);
    if (!this.useCache || !f || f.mtimeMs !== st.mtimeMs) {
      if (run.read >= this.limits.folders) return void (run.truncated = "folders");
      if (this.clock.now() - run.started > this.limits.ms) return void (run.truncated = "time");
      run.read++;
      const before = f;
      f = { mtimeMs: st.mtimeMs, subdirs: [] };
      let ents = [];
      try {
        ents = this.fs.readdirSync(dir, { withFileTypes: true });
      } catch (err) {
        if (err.code !== "ENOENT") run.problem ??= `couldn't read ${dir} (${err.code ?? err.message})`;
      }
      for (const e of ents) {
        const full = path.join(dir, e.name);
        if (e.isDirectory()) {
          if (level < this.limits.depth && this.enter(e.name, dir, level + 1)) f.subdirs.push(full);
        } else if (e.isFile() && this.pick(e.name, dir)) {
          out.push(full);
          run.files++;
        }
      }
      this.#gone(before, f);
      this.folders.set(dir, f);
    }
    for (const sub of f.subdirs) this.#walk(sub, level + 1, out, run);
  }

  // Folders that went since a folder was last read, and everything that
  // was under them.
  #gone(before, now) {
    for (const gone of before?.subdirs.filter((d) => !now.subdirs.includes(d)) ?? []) this.#forget(gone);
  }

  #forget(dir) {
    for (const k of this.folders.keys()) if (k === dir || k.startsWith(dir + path.sep)) this.folders.delete(k);
  }

  // The files picked, breadth first: the root, then the folders below it a
  // level at a time, shallow levels first, each level in order (a folder's
  // subfolders in the order it lists them, after those of the folders
  // before it on its level), `concurrency` folders read at once.
  //
  // `look(dir, entries, level)` is handed each folder read, with its
  // entries (fs.Dirent), and may be async; `enter` is asked about the
  // folder's subfolders once `look` is done with it, so it can go by what
  // `look` found there. The root is always read; `folders` budgets the
  // folders below it, and `read` and `seen` count those. A walk that runs
  // out of budget stops after the level it ran out on.
  //
  // Options given here are this walk's own, in place of the scanner's: a
  // scanner whose root and rules change from one walk to the next (the
  // projects scan follows the folder in use, and your rules) passes them
  // each time. `signal` (an AbortSignal) stops a walk no longer wanted: it
  // reads no folder after it aborts, and its last says "cancelled".
  async scanBreadth({ root = this.root, depth = this.limits.depth, folders = this.limits.folders, ms = this.limits.ms, pick = this.pick, enter = this.enter, look = this.look, concurrency = this.concurrency, signal = null } = {}) {
    const limits = { depth, folders, ms };
    this.budget = { ...limits, last: this.budget.last };
    const started = this.clock.now();
    const run = { files: 0, read: 0, seen: 0, truncated: null, problem: null, started };
    const out = [];
    const walk = { limits, pick, enter, look, run, started, signal };
    let level = [root];
    for (let n = 0; level.length && !run.truncated; n++) {
      // With the cache, a folder that didn't change since it was last read
      // is only looked at: its subfolders are the ones it had.
      const stats = this.useCache ? await inTurn(level, concurrency, (dir) => this.fs.promises.stat(dir).catch(() => null)) : null;
      const plan = [];
      let reads = 0;
      for (const [i, dir] of level.entries()) {
        const st = stats?.[i];
        if (stats && !st) {
          this.#forget(dir);
          continue;
        }
        if (n > 0) run.seen++;
        const known = this.folders.get(dir);
        if (st && known?.mtimeMs === st.mtimeMs) {
          plan.push({ dir, subdirs: known.subdirs, files: [] });
          continue;
        }
        // The budget is spent in order, so the same folders are read every
        // time: the first ones on the level.
        if (n > 0 && run.read + reads >= folders) {
          run.truncated = "folders";
          break;
        }
        if (n > 0) reads++;
        plan.push({ dir, read: true, mtimeMs: st?.mtimeMs ?? null });
      }
      const found = await inTurn(plan, concurrency, (p) => (p.read ? this.#readFolder(p.dir, n, p.mtimeMs, walk) : p));
      level = [];
      for (const f of found) {
        if (!f) continue;
        out.push(...f.files);
        // (A folder kept from an earlier walk may have been read for a
        // deeper one.)
        if (n < depth) level.push(...f.subdirs);
      }
    }
    this.#report(run, limits);
    return out;
  }

  // One folder of the breadth-first walk: its entries to `look`, then its
  // files picked and its subfolders entered. Null when it wasn't read.
  async #readFolder(dir, level, mtimeMs, { limits, pick, enter, look, run, started, signal }) {
    if (signal?.aborted) {
      run.truncated ??= "cancelled";
      return null;
    }
    if (this.clock.now() - started > limits.ms) {
      run.truncated ??= "time";
      return null;
    }
    if (level > 0) run.read++;
    const f = { dir, files: [], subdirs: [] };
    let ents;
    try {
      ents = await this.fs.promises.readdir(dir, { withFileTypes: true });
    } catch (err) {
      if (err.code === "ENOENT") {
        this.#forget(dir);
        return null;
      }
      run.problem ??= `couldn't read ${dir} (${err.code ?? err.message})`;
    }
    if (ents && look) {
      try {
        await look(dir, ents, level);
      } catch (err) {
        // What looks at the folders is the caller's: a folder it couldn't
        // look at is passed over, like one that couldn't be read.
        run.problem ??= `couldn't look at ${dir} (${messageOf(err)})`;
        ents = null;
      }
    }
    ents ??= [];
    for (const e of ents) {
      const full = path.join(dir, e.name);
      if (e.isDirectory()) {
        if (level < limits.depth && enter(e.name, dir, level + 1)) f.subdirs.push(full);
      } else if (e.isFile() && pick(e.name, dir)) {
        f.files.push(full);
        run.files++;
      }
    }
    if (this.useCache) {
      const now = { mtimeMs, subdirs: f.subdirs };
      this.#gone(this.folders.get(dir), now);
      this.folders.set(dir, now);
    }
    return f;
  }
}
