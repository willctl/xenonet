// core/drift.js in Rust (crates/xenonet-schema/src/drift.rs): where the
// vendor data XenoNet reads differs from its schemas, kept as drift
// records in drift.json. Rust keeps the records, works out shapes and
// keys, and reads, merges and writes the file; the save's timer stays here.
import { systemClock } from "../../../../core/blocks/clock.js";
import { native } from "../index.js";

const C = native.driftConstants();
export const DRIFT_MAX = C.max;
export const DRIFT_FILE = C.file;
const SAVE_AFTER_MS = 5000;

export const KINDS = C.kinds;
export const ADDITIONS = new Set(C.additions);

export const sourceName = (source) => native.sourceName(String(source));

export const jsonType = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v === "object" ? "object" : typeof v);

export const isField = (k) => native.isField(String(k));

export const shapeOf = (value, { depth = 4, budget = 400, names = true } = {}) => native.shapeOf(value, depth, budget, !!names);

export const variantShape = (value) => native.variantShape(value);

export const cleanVersion = (v) => (typeof v === "string" ? (native.cleanVersion(v) ?? null) : null);

const isValue = (v) => typeof v === "string" || typeof v === "number" || typeof v === "boolean";
export const cleanVariant = (variant) => (Array.isArray(variant) && variant.length && variant.length <= 8 && variant.every(isValue) ? variant : null);

export const keyOf = (r) => native.keyOf(r);

export const describe = (r) => native.describe(r);

// The Rust core under a log, for the vendor readers' twin.
export const LOG = Symbol("xenonet.drift");

export class DriftLog {
  #timer = null;

  constructor({ max = DRIFT_MAX, clock = systemClock, saveAfterMs = SAVE_AFTER_MS } = {}) {
    Object.defineProperty(this, LOG, { value: new native.DriftLogCore(max) });
    this.max = max;
    this.clock = clock;
    this.saveAfterMs = saveAfterMs;
  }

  get lookedAt() {
    return this[LOG].lookedAt ?? null;
  }
  get file() {
    return this[LOG].file ?? null;
  }
  get dirty() {
    return this[LOG].dirty;
  }

  // Adds a piece of drift. `shape` can be a function, called only for a
  // record that's new.
  record({ source, vendorVersion = null, kind, path = null, variant = null, shape = null, type = null }, now = this.clock.now()) {
    const piece = { source, vendorVersion, kind, path, variant, shape: typeof shape === "function" ? null : shape, type };
    if (typeof shape === "function" && !this[LOG].knows(piece)) piece.shape = shape();
    const r = this[LOG].record(piece, now);
    this.#changed();
    return r;
  }

  // Drift recorded in Rust (by a vendor reader): saved like the rest.
  touch() {
    if (this[LOG].dirty) this.#changed();
  }

  list() {
    return this[LOG].list();
  }

  since(t) {
    return this.list().filter((r) => r.firstSeen > (t ?? -Infinity));
  }

  looked(at = this.clock.now()) {
    this[LOG].looked(at);
    this.#changed();
  }

  #changed() {
    if (!this.file || this.#timer) return;
    this.#timer = this.clock.setTimeout(() => {
      this.#timer = null;
      this.flush();
    }, this.saveAfterMs);
    this.#timer?.unref?.();
  }

  open(file) {
    this[LOG].open(String(file));
    if (this[LOG].dirty) this.#changed();
    return this;
  }

  flush() {
    if (this.#timer) {
      this.clock.clearTimeout(this.#timer);
      this.#timer = null;
    }
    const err = this[LOG].flush();
    if (err) console.error(`[xenonet] couldn't save drift to ${this.file}: ${err}`);
  }

  view() {
    return this[LOG].view();
  }

  clear() {
    this[LOG].clear();
  }
}

export const readDriftFile = (file) => native.readDriftFile(String(file)) ?? null;

export const drift = new DriftLog();
