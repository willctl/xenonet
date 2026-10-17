// core/blocks/normalizer.js in Rust (crates/xenonet-blocks/src/normalizer.rs):
// a vendor's record to the one XenoNet works with. Rust takes each record
// through parsing (as JSON.parse would, unless `parse` is another), the
// inspect hook and normalize, and counts what happened; the hooks are the
// JS functions given here, and never throw into Rust: what one throws is
// handed over as a failure.
import { systemClock } from "../../../../../core/blocks/clock.js";
import { native } from "../../index.js";
import { Block, CORE, EMIT, messageOf } from "./lifecycle.js";

const FAILED = Symbol("failed");

export class Normalizer extends Block {
  #failure = null;
  #escaped = null;
  #guards = new WeakMap();
  #failureMessage = () => messageOf(this.#failure);
  #now = () => this.clock.now();

  // `source`: the vendor schema's path under vendor/ (for drift records).
  // `parse(text)`: a record from text (JSON by default). `normalize(record,
  // where)`: the record XenoNet works with.
  constructor({ name, source = null, parse = JSON.parse, normalize = (r) => r, clock = systemClock } = {}) {
    super({ name, kind: "normalizer", clock, core: new native.Normalizer(String(name), source ?? null, clock.now()) });
    this.source = source;
    this.parse = parse;
    this.normalize = normalize;
    this.inspect = null;
    this.unparsable = null;
  }

  // Sets the hooks: { inspect(record, where) → record | null, unparsable(text, where, err) }.
  use({ inspect = this.inspect, unparsable = this.unparsable } = {}) {
    this.inspect = inspect;
    this.unparsable = unparsable;
    return this;
  }

  // A function that doesn't throw: what it throws is kept, and FAILED
  // returned in its place.
  #guard(fn) {
    if (typeof fn !== "function") return null;
    let g = this.#guards.get(fn);
    if (!g) {
      g = (...args) => {
        try {
          return fn(...args);
        } catch (err) {
          this.#failure = err;
          return FAILED;
        }
      };
      this.#guards.set(fn, g);
    }
    return g;
  }

  // What the unparsable hook throws goes on to whoever read the line, as in
  // JS, once Rust is done with it.
  #unparsableHook(own) {
    if (typeof this.unparsable !== "function") return null;
    return (text, where, why) => {
      try {
        this.unparsable(text, where, own ? this.#failure : new SyntaxError(why));
      } catch (err) {
        this.#escaped ??= { err };
      }
    };
  }

  #settle(out) {
    this[EMIT]();
    const escaped = this.#escaped;
    this.#escaped = null;
    if (escaped) throw escaped.err;
    return out ?? null;
  }

  // A line of text, parsed and normalized; null when it isn't one.
  line(text, where = null) {
    const own = this.parse !== JSON.parse;
    const out = this[CORE].line(String(text), where, own ? this.#guard(this.parse) : null, this.#guard(this.inspect), this.#unparsableHook(own), this.#guard(this.normalize), FAILED, this.#failureMessage, this.#now);
    return this.#settle(out);
  }

  // A record already parsed, normalized; null when inspect drops it, or
  // when the hooks or normalize throw on it.
  record(raw, where = null) {
    const out = this[CORE].record(raw, where, this.#guard(this.inspect), this.#guard(this.normalize), FAILED, this.#failureMessage, this.#now);
    return this.#settle(out);
  }

  health() {
    this[CORE].look(this.clock.now());
    this[EMIT]();
    return super.health();
  }
}
