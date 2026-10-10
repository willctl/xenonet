// A normalizer turns a vendor's record (a line of a transcript, an item an
// app's API returns) into the record XenoNet works with, so a vendor's names
// and quirks stay in one place, and every reader of the same source goes
// through the same door.
//
//   const lines = new Normalizer({ name: "claude-lines", source: "anthropic/claude-code/transcript-line", normalize: (o) => new ClaudeLine(o) });
//   const line = lines.line(text, where);    // parsed and normalized, or null
//   const rec = lines.record(object, where); // already parsed
//
// Between parsing and normalizing, the record passes through `inspect`,
// the hook for checking it against the vendor's schema leniently and
// recording drift (contracts, section 1): it gets the record and `where` it
// came from (an object the caller keeps per file or request), and returns
// the record to use (the same, or with what doesn't fit taken out), or
// null to drop it. A line that isn't JSON goes to `unparsable` instead.
// The transcripts' normalizers (lines.js) set both through use(), from the
// vendor readers (vendor.js).
//
// It never throws for a record: what's wrong with one is the reader's to
// find, where it was found before.
import { Block, messageOf } from "./lifecycle.js";

export class Normalizer extends Block {
  // `source`: the vendor schema's path under vendor/ (for drift records).
  // `parse(text)`: a record from text (JSON by default). `normalize(record,
  // where)`: the record XenoNet works with.
  constructor({ name, source = null, parse = JSON.parse, normalize = (r) => r, clock } = {}) {
    super({ name, kind: "normalizer", clock });
    this.source = source;
    this.parse = parse;
    this.normalize = normalize;
    this.inspect = null;
    this.unparsable = null;
    this.counters = { errors: 0, records: 0, unparsable: 0, dropped: 0 };
    this.seen = 0;
  }

  // Sets the hooks: { inspect(record, where) → record | null, unparsable(text, where, err) }.
  use({ inspect = this.inspect, unparsable = this.unparsable } = {}) {
    this.inspect = inspect;
    this.unparsable = unparsable;
    return this;
  }

  // A line of text, parsed and normalized; null when it isn't one.
  line(text, where = null) {
    let raw;
    try {
      raw = this.parse(text);
    } catch (err) {
      this.counters.unparsable++;
      this.unparsable?.(text, where, err);
      return null;
    }
    return this.record(raw, where);
  }

  // A record already parsed, normalized; null when inspect drops it, or
  // when the hooks or normalize throw on it: that's counted as an error
  // (lastError says what), and the reader goes on to the next record.
  record(raw, where = null) {
    this.counters.records++;
    try {
      if (this.inspect) {
        raw = this.inspect(raw, where);
        if (raw == null) {
          this.counters.dropped++;
          return null;
        }
      }
      return this.normalize(raw, where);
    } catch (err) {
      // One record isn't the normalizer's health: its state stays as it is.
      this.lastError = { at: this.clock.now(), message: messageOf(err) };
      this.count("errors");
      return null;
    }
  }

  // Records go through by the hundred thousand, so the clock isn't read
  // for each: lastOk is when records were last seen going through, as of
  // the last look.
  health() {
    if (this.counters.records !== this.seen) {
      this.seen = this.counters.records;
      this.ok();
    }
    return super.health();
  }
}
