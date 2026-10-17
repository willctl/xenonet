// A tailer reads files that grow, such as transcripts, a line at a time as
// lines are added. It keeps an offset in each file and reads from there;
// it hands on whole lines only, keeping a half-written last line (as bytes,
// since a read can end inside a character) for the next read; and it starts
// a file over when the file got shorter, as when it's rewritten.
//
// It learns of new lines three ways: a recursive watch on each root, which
// is quick but can miss events (macOS starts a watch a moment after
// fs.watch returns, and drops events under load); a sweep every couple of
// seconds over the files it reads, for lines no event announced; and every
// so often a walk of the folders (see scanner.js) for files no event
// announced.
//
// At start it reads every file written within `primeWindowMs` as history
// (prime: true), and leaves older ones dormant: one that's written to again
// later has what it held then read as history too, before its new lines.
// A file it can't read at all (another user's, say) waits the same way, so
// the rest are still read and followed.
//
//   const tailer = new Tailer({
//     roots: [{ dir: "/…/.claude/projects", app: "claude" }],
//     classify: (file) => ({ sessionId: "…" }) or null,   // the files it reads, and what each one is
//     onLine: (line, entry, prime) => …,                   // each whole, non-blank line, in order
//   });
//   await tailer.start();
//
// One read of a file at a time: two at once (a watch event and a sweep, or
// a read still going when the next event comes) would both start from the
// same offset and take its new lines twice.
import nodeFs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { Block } from "./lifecycle.js";
import { Scanner } from "./scanner.js";

const NOTHING = Buffer.alloc(0);

export class Tailer extends Block {
  // `roots`: [{ dir, … }], folders that exist, as real paths. `classify(file)`:
  // what a file is (an object kept with its offset), or null to leave it
  // alone. `pick(name)`: file names worth classifying. `onLine(line, entry,
  // prime)`: a line read. `onNewFile(file, entry)`: a file first read after
  // start. `onPrimed()`: start has read the history, before the watches
  // begin. `status`: the counters it keeps ({ watching, files, lines,
  // errors }), shared with its owner, who counts lines and the errors it
  // finds in them.
  constructor({ name = "tailer", roots = [], classify, pick = (name) => name.endsWith(".jsonl"), onLine, onNewFile = null, onPrimed = null, primeWindowMs = 24 * 3600e3, sweepMs = 2000, discoverEvery = 15, debounceMs = 30, depth = 6, watch = nodeFs.watch, status = null, clock } = {}) {
    super({ name, kind: "tailer", clock });
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
    this.files = new Map(); // file → { …classify(file), offset, rest }
    this.pending = new Map(); // file → the debounce timer of its watch events
    this.reads = new Map(); // file → the read of it under way
    this.dormant = new Map(); // file too old to read at start → its size then
    this.watchers = [];
    this.sweeping = null;
    this.sweepTimer = null;
  }

  // Every folder the scanners know, across the roots.
  get folders() {
    return new Map(this.scanners.flatMap((s) => [...s.folders]));
  }

  // Reads the history, then watches and sweeps. A file that can't be read
  // leaves the tailer partly working (degraded) until a read works again.
  // Stopped while it reads the history, it reads no more, and neither
  // watches nor sweeps.
  async onStart(run) {
    const now = this.clock.now();
    let failed = false;
    for (const scanner of this.scanners) {
      for (const file of scanner.scan()) {
        // Started again after a stop: a file it followed before is caught up
        // by the sweep, so what was written meanwhile reads as live, not as
        // history. (The scan lists it again when its folder changed.)
        if (this.files.has(file)) continue;
        const st = await fsp.stat(file).catch(() => null);
        if (!this.current(run)) return;
        if (!st) continue;
        if (now - st.mtimeMs > this.primeWindowMs) {
          this.dormant.set(file, st.size);
          continue;
        }
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

  // Lets go of the watches, the sweep and the reads waiting on a watch
  // event, so a start after this one begins with none: a watch it closed
  // can't count as one that works.
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

  // A file nothing could be read of waits like one too old to read at
  // start: no sweep tries it again (and fails again every couple of
  // seconds), and once it's written to and can be read, what it held up to
  // `size` is read as history, not as lines just written.
  #park(file, size) {
    if (this.files.get(file)?.offset) return;
    this.files.delete(file);
    this.dormant.set(file, size);
    this.status.files = this.files.size;
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

  // Catches up on what no watch event announced: new lines in the files
  // being read, and with `discover`, files that are new since the last
  // walk of the folders. One sweep at a time.
  sweep({ discover = true } = {}) {
    this.sweeping ??= this.#sweep(discover).finally(() => (this.sweeping = null));
    return this.sweeping;
  }

  async #sweep(discover) {
    this.count("sweeps");
    for (const [file, f] of this.files) {
      const st = await fsp.stat(file).catch(() => null);
      if (!st) this.files.delete(file);
      else if (st.size !== f.offset) await this.track(file, { prime: false }).catch((err) => this.#error(err));
    }
    this.status.files = this.files.size;
    if (!discover) return;
    this.count("walks");
    // A new file is read from its first line, as live; an old one (copied
    // in, say) waits with the others from before.
    for (const scanner of this.scanners) {
      for (const file of scanner.scan()) {
        if (this.files.has(file) || this.dormant.has(file) || !this.classify(file)) continue;
        const st = await fsp.stat(file).catch(() => null);
        if (st && this.clock.now() - st.mtimeMs > this.primeWindowMs) this.dormant.set(file, st.size);
        else if (st) await this.track(file, { prime: false }).catch((err) => this.#error(err));
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
    let f = this.files.get(file);
    if (!f) {
      const kind = this.classify(file);
      if (!kind) return;
      f = { ...kind, file, offset: 0, rest: NOTHING };
      this.files.set(file, f);
      this.status.files = this.files.size;
      if (!prime) this.onNewFile?.(file, f);
      // A file too old to read at start that's written to again, as when a
      // session is picked up days later: what it held then is its history,
      // taken in quietly like the others' at start.
      const held = this.dormant.get(file);
      this.dormant.delete(file);
      if (held) {
        try {
          await this.#take(file, f, { prime: true, until: held });
        } catch (err) {
          // Still can't be read: it waits again, its history with it.
          this.#park(file, held);
          throw err;
        }
      }
    }
    await this.#take(file, f, { prime });
  }

  // Hands on the lines added to `file` since the last read, up to `until`
  // bytes into it.
  async #take(file, f, { prime, until = Infinity }) {
    const st = await fsp.stat(file).catch(() => null);
    if (!st) return;
    // Rewritten from the top: start over, without the half line from before.
    if (st.size < f.offset) [f.offset, f.rest] = [0, NOTHING];
    const end = Math.min(st.size, until);
    if (end <= f.offset) return;
    const fh = await fsp.open(file, "r");
    try {
      const len = end - f.offset;
      const buf = Buffer.alloc(len);
      await fh.read(buf, 0, len, f.offset);
      f.offset = end;
      // Only whole lines are decoded, since a read can end inside a
      // character; the unfinished line waits, as bytes, for the next read.
      const data = f.rest.length ? Buffer.concat([f.rest, buf]) : buf;
      const cut = data.lastIndexOf(10) + 1;
      f.rest = Buffer.from(data.subarray(cut));
      for (const line of data.toString("utf8", 0, cut).split("\n")) if (line.trim()) this.onLine(line, f, prime);
    } finally {
      await fh.close();
    }
    // A read that worked is fine again, unless a root has no watch: then
    // new lines wait for the sweep, and it stays degraded.
    if (this.state !== "running" && !(this.started && this.watchers.length < this.roots.length)) this.ok();
  }

  health() {
    const h = super.health();
    h.counters = { ...h.counters, files: this.files.size, dormant: this.dormant.size, lines: this.status.lines, watchers: this.watchers.length };
    return h;
  }
}
