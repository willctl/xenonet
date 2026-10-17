// core/hosts/frames.js in Rust (crates/xenonet-hosts/src/frames.rs): the
// relay's frames, one JSON object a line. Splitting the bytes into lines,
// reading a frame and its depth check are the Rust's; the callbacks are
// here.
import { native } from "../../index.js";

const C = native.hostsFrameConstants();

export const PROTOCOL = C.protocol;
export const MAX_FRAME = C.maxFrame;
export const MAX_REQUEST = C.maxRequest;

export const FROM_RELAY = new Set(C.fromRelay);
export const FROM_CONSOLE = new Set(C.fromConsole);

export function lineSplitter({ max = MAX_FRAME, onLine, onDrop = () => {} }) {
  const lines = new native.HostsLines(Number(max));
  return function push(chunk) {
    const buf = typeof chunk === "string" ? Buffer.from(chunk) : Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    for (const line of lines.push(buf)) {
      if (line === null) onDrop();
      else onLine(line);
    }
  };
}

export function parseFrame(line, known = null) {
  const r = native.hostsParseFrame(String(line), known ? [...known].filter((k) => typeof k === "string") : null);
  return r.error ? { error: r.error } : { frame: r.frame };
}

export const encode = (frame) => `${JSON.stringify(frame)}\n`;

export const MAX_DEPTH = C.maxDepth;
export const shallow = (value, max = MAX_DEPTH) => native.hostsShallow(value, Number(max));
