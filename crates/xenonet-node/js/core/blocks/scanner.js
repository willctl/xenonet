// core/blocks/scanner.js in Rust (crates/xenonet-blocks/src/scanner.rs): a
// walk for the files wanted, within budgets of folders, depth and time,
// reading only the folders whose mtime changed since the last walk (and
// again those read within 2 s of their mtime).
//
// scan() is Rust's walk, reading the disk itself, or the `fs` given here
// when a test gives another. scanBreadth() is Rust's walk too, but the
// reading is done here, with fs.promises, a few folders at once: Rust says
// which folders to read and decides what each read means, and `look`
// (which may await) is handed each folder read.
import nodeFs from "node:fs";
import { systemClock } from "../../../../../core/blocks/clock.js";
import { native } from "../../index.js";
import { Block, CORE, EMIT, messageOf } from "./lifecycle.js";

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

const entriesOf = (ents) => ents.map((e) => [e.name, e.isDirectory(), e.isFile()]);

// Rules that answer true or false, and keep what they throw for the twin
// to throw again once Rust has stopped the walk.
function guarded(stop) {
  return (fn) =>
    (...args) => {
      try {
        return !!fn(...args);
      } catch (err) {
        stop.err ??= { err };
        throw err;
      }
    };
}

export class Scanner extends Block {
  #now = () => this.clock.now();

  // `root`: the folder walked. `depth`: how many levels below it the walk
  // goes. `folders`: how many folders one walk may read. `ms`: how long one
  // walk may take. `pick(name, dir)`: the files wanted. `enter(name, dir,
  // level)`: whether to walk into a subfolder. `cache`: read only what
  // changed (true), or every folder on every walk. For the breadth-first
  // walk, `look(dir, entries, level)` is handed each folder read, and
  // `concurrency` folders are read at once.
  constructor({ name, root, depth = Infinity, folders = Infinity, ms = Infinity, pick = () => true, enter = () => true, look = null, concurrency = 16, cache = true, fs = nodeFs, clock = systemClock } = {}) {
    super({ name, kind: "scanner", clock, core: new native.Scanner(String(name), depth, folders, ms, clock.now()) });
    this.root = root;
    this.limits = { depth, folders, ms };
    this.pick = pick;
    this.enter = enter;
    this.look = look;
    this.concurrency = concurrency;
    this.useCache = cache;
    this.fs = fs;
  }

  // folder → { mtimeMs, subdirs, racy }, from the last time it was read.
  get folders() {
    return new Map(this[CORE].folders());
  }

  get last() {
    return this[CORE].last;
  }

  get budget() {
    return this[CORE].budget;
  }

  // Forgets every folder, so the next walk reads them all.
  forget() {
    this[CORE].forget();
  }

  // The files picked in the folders this walk read.
  scan() {
    const fs = this.fs;
    const own = fs && (fs.statSync !== nodeFs.statSync || fs.readdirSync !== nodeFs.readdirSync);
    const stat = (dir) => {
      try {
        return fs.statSync(dir).mtimeMs ?? null;
      } catch {
        return null;
      }
    };
    const readdir = (dir) => {
      try {
        return entriesOf(fs.readdirSync(dir, { withFileTypes: true }));
      } catch (err) {
        return { code: err.code ?? null, message: messageOf(err) };
      }
    };
    const stop = {};
    const guard = guarded(stop);
    try {
      const { depth, folders, ms } = this.limits;
      return this[CORE].scan(String(this.root), depth, folders, ms, !!this.useCache, guard(this.pick), guard(this.enter), this.#now, own ? stat : null, own ? readdir : null);
    } catch (err) {
      throw stop.err ? stop.err.err : err;
    } finally {
      this[EMIT]();
    }
  }

  // The files picked, breadth first: the root, then the folders below it a
  // level at a time, `concurrency` folders read at once. Options given here
  // are this walk's own, in place of the scanner's. `signal` (an
  // AbortSignal) stops a walk no longer wanted.
  async scanBreadth({ root = this.root, depth = this.limits.depth, folders = this.limits.folders, ms = this.limits.ms, pick = this.pick, enter = this.enter, look = this.look, concurrency = this.concurrency, signal = null } = {}) {
    const core = this[CORE];
    const stop = {};
    const guard = guarded(stop);
    const [picks, enters] = [guard(pick), guard(enter)];
    const w = core.beginBreadth(String(root), depth, folders, ms, !!this.useCache, this.#now);
    try {
      while (w.more()) {
        const mtimes = this.useCache ? await inTurn(w.level(), concurrency, (dir) => this.fs.promises.stat(dir).then((st) => st.mtimeMs, () => null)) : null;
        const level = w.n;
        await inTurn(core.plan(w, mtimes), concurrency, async (read) => {
          const at = core.beginRead(w, !!signal?.aborted, this.#now);
          if (at == null) return core.skip(w, read);
          let ents = null;
          try {
            ents = await this.fs.promises.readdir(read.dir, { withFileTypes: true });
          } catch (err) {
            if (!core.readFailed(w, read.dir, err.code ?? null, messageOf(err))) return core.skip(w, read);
          }
          if (ents && look) {
            try {
              await look(read.dir, ents, level);
            } catch (err) {
              core.lookFailed(w, read.dir, messageOf(err));
              ents = null;
            }
          }
          core.finishRead(w, read, at, entriesOf(ents ?? []), picks, enters);
        });
        core.endLevel(w);
      }
      return core.finishBreadth(w, this.#now);
    } catch (err) {
      throw stop.err ? stop.err.err : err;
    } finally {
      this[EMIT]();
    }
  }
}
