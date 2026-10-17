// core/blocks/tailer.js in Rust (crates/xenonet-blocks/src/tailer.rs): files
// read a line at a time as they grow. Rust keeps where each file is read to
// and its half line, and the dormant files, and decides each step: what a
// walk's files are, what a sweep does, what to read of a file and which
// lines that read finishes. The watches, the timers, the reads themselves
// and one read of a file at a time stay here, with the callbacks, as the
// runtime the Rust service will have in tokio.
//
// It has the fix on fix/scanner-racy-mtime: started again after a stop, a
// file it followed before is caught up by the sweep, as live, not history.
import nodeFs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { systemClock } from "../../../../../core/blocks/clock.js";
import { native } from "../../index.js";
import { Block, CORE } from "./lifecycle.js";
import { Scanner } from "./scanner.js";

export class Tailer extends Block {
  // file → what classify(file) said it is, with the file: the entry each
  // line comes with, as JS objects its owner can keep.
  #entries = new Map();

  // `roots`: [{ dir, … }], folders that exist, as real paths. `classify(file)`:
  // what a file is (an object kept with its offset), or null to leave it
  // alone. `pick(name)`: file names worth classifying. `onLine(line, entry,
  // prime)`: a line read. `onNewFile(file, entry)`: a file first read after
  // start. `onPrimed()`: start has read the history, before the watches
  // begin. `status`: the counters it keeps ({ watching, files, lines,
  // errors }), shared with its owner, who counts lines and the errors it
  // finds in them.
  constructor({ name = "tailer", roots = [], classify, pick = (name) => name.endsWith(".jsonl"), onLine, onNewFile = null, onPrimed = null, primeWindowMs = 24 * 3600e3, sweepMs = 2000, discoverEvery = 15, debounceMs = 30, depth = 6, watch = nodeFs.watch, status = null, clock = systemClock } = {}) {
    super({ name, kind: "tailer", clock, core: new native.Tailer(String(name), primeWindowMs, clock.now()) });
    this.roots = roots;
    this.classify = classify;
    this.pick = pick;
    this.onLine = onLine;
    this.onNewFile = onNewFile;
    this.onPrimed = onPrimed;
    this.primeWindowMs = primeWindowMs;
    this.sweepMs = sweepMs;
    this.discoverEvery = discoverEvery;
    this.debounceMs = debounceMs;
    this.watchFn = watch;
    this.status = status ?? { watching: false, files: 0, lines: 0, errors: 0 };
    this.scanners = roots.map((r) => new Scanner({ name: `${name}:${r.dir}`, root: r.dir, depth, pick: (n) => this.pick(n), clock }));
    this.pending = new Map(); // file → the debounce timer of its watch events
    this.reads = new Map(); // file → the read of it under way
    this.watchers = [];
    this.sweeping = null;
    this.sweepTimer = null;
  }

  // file → { …classify(file), file, offset, rest }
  get files() {
    return new Map(this[CORE].files().map((f) => [f, this.#entries.get(f)]));
  }

  // file too old to read at start → its size then
  get dormant() {
    return new Map(this[CORE].dormant());
  }

  // Every folder the scanners know, across the roots.
  get folders() {
    return new Map(this.scanners.flatMap((s) => [...s.folders]));
  }

  #entry(file, kind) {
    const core = this[CORE];
    const f = { ...kind, file };
    Object.defineProperties(f, {
      offset: { get: () => core.offsetOf(file) ?? 0, enumerable: true, configurable: true },
      rest: { get: () => core.restOf(file) ?? Buffer.alloc(0), enumerable: true, configurable: true },
    });
    return f;
  }

  #forget(file) {
    if (!this[CORE].has(file)) this.#entries.delete(file);
    this.status.files = this[CORE].fileCount();
  }

  async onStart(run) {
    const core = this[CORE];
    const now = this.clock.now();
    let failed = false;
    for (const scanner of this.scanners) {
      for (const file of scanner.scan()) {
        if (!core.listedAtStart(file)) continue;
        const st = await fsp.stat(file).catch(() => null);
        if (!this.current(run)) return;
        if (core.atStart(file, st?.mtimeMs ?? null, st?.size ?? null, now) !== "track") continue;
        try {
          await this.track(file, { prime: true });
        } catch (err) {
          failed = true;
          this.#park(file, st.size);
          this.#error(err);
        }
        if (!this.current(run)) return;
      }
    }
    this.onPrimed?.();
    if (!failed) this.ok(now);
    for (const root of this.roots) {
      try {
        const w = this.watchFn(root.dir, { recursive: true }, (_evt, name) => name && this.schedule(path.join(root.dir, name)));
        w.on("error", (err) => this.#error(err));
        this.watchers.push(w);
      } catch (err) {
        this.#error(err);
      }
    }
    this.status.watching = this.watchers.length > 0;
    // The safety net for missed watch events. The first sweep also finds
    // files made while the watches were starting.
    let sweeps = 0;
    this.sweepTimer = this.clock.setInterval(() => this.sweep({ discover: sweeps++ % this.discoverEvery === 0 }).catch((err) => this.#error(err)), this.sweepMs);
  }

  onStop() {
    for (const w of this.watchers) w.close();
    this.watchers = [];
    this.status.watching = false;
    this.clock.clearInterval(this.sweepTimer);
    this.sweepTimer = null;
    for (const timer of this.pending.values()) this.clock.clearTimeout(timer);
    this.pending.clear();
  }

  #error(err) {
    this.status.errors += 1;
    this.problem(err);
  }

  #park(file, size) {
    this[CORE].park(file, size);
    this.#forget(file);
  }

  // A watch event for `file`: read it a moment later, once a burst of
  // writes has settled.
  schedule(file) {
    if (!this.pick(path.basename(file))) return;
    this.clock.clearTimeout(this.pending.get(file));
    this.pending.set(
      file,
      this.clock.setTimeout(() => {
        this.pending.delete(file);
        this.track(file, { prime: false }).catch((err) => this.#error(err));
      }, this.debounceMs),
    );
  }

  sweep({ discover = true } = {}) {
    this.sweeping ??= this.#sweep(discover).finally(() => (this.sweeping = null));
    return this.sweeping;
  }

  async #sweep(discover) {
    const core = this[CORE];
    this.count("sweeps");
    for (let next = core.nextFollowed(null); next; next = core.nextFollowed(next[0])) {
      const file = next[1];
      const st = await fsp.stat(file).catch(() => null);
      const what = core.inSweep(file, st?.size ?? null);
      if (what === "drop") this.#entries.delete(file);
      else if (what === "read") await this.track(file, { prime: false }).catch((err) => this.#error(err));
    }
    this.status.files = core.fileCount();
    if (!discover) return;
    this.count("walks");
    for (const scanner of this.scanners) {
      for (const file of scanner.scan()) {
        if (!core.mayDiscover(file) || !this.classify(file)) continue;
        const st = await fsp.stat(file).catch(() => null);
        if (core.discovered(file, st?.mtimeMs ?? null, st?.size ?? null, () => this.clock.now()) === "track") await this.track(file, { prime: false }).catch((err) => this.#error(err));
      }
    }
  }

  // Settles once no read or sweep is under way.
  async idle() {
    while (this.reads.size || this.sweeping) await Promise.all([...this.reads.values(), this.sweeping?.catch(() => {})]);
  }

  // Reads what's new in `file`, after any read of it under way.
  track(file, opts) {
    const run = (this.reads.get(file) ?? Promise.resolve()).then(() => this.#read(file, opts));
    const done = run.catch(() => {});
    this.reads.set(file, done);
    done.then(() => this.reads.get(file) === done && this.reads.delete(file));
    return run;
  }

  async #read(file, { prime }) {
    const core = this[CORE];
    let f = core.has(file) ? this.#entries.get(file) : null;
    if (!f) {
      const kind = this.classify(file);
      if (!kind) return;
      f = this.#entry(file, kind);
      this.#entries.set(file, f);
      core.addFile(file);
      this.status.files = core.fileCount();
      if (!prime) this.onNewFile?.(file, f);
      const held = core.takeHeld(file);
      if (held != null) {
        try {
          await this.#take(file, f, { prime: true, until: held });
        } catch (err) {
          this.#park(file, held);
          throw err;
        }
      }
    }
    await this.#take(file, f, { prime });
  }

  async #take(file, f, { prime, until = null }) {
    const core = this[CORE];
    const st = await fsp.stat(file).catch(() => null);
    if (!st) return;
    const span = core.planTake(file, st.size, until);
    if (!span) return;
    const fh = await fsp.open(file, "r");
    try {
      const [from, to] = span;
      const buf = Buffer.alloc(to - from);
      await fh.read(buf, 0, buf.length, from);
      for (const line of core.took(file, buf, to)) this.onLine(line, f, prime);
    } finally {
      await fh.close();
    }
    if (core.okAfterRead(this.watchers.length, this.roots.length)) this.ok();
  }

  health() {
    const h = super.health();
    h.counters = { ...h.counters, files: this[CORE].fileCount(), dormant: this[CORE].dormantCount(), lines: this.status.lines, watchers: this.watchers.length };
    return h;
  }
}
