// core/assistant/cursors.js in Rust (crates/xenonet-assistant/src/
// cursors.rs): page tokens for the assistant's lists, signed with a key
// made when the service starts and never written down. The clock is here.
import { native } from "../../index.js";

export const CURSOR_MS = native.asstConstants().cursorMs;

// The Rust's own, for the service to sign with.
export const CORE = Symbol.for("xenonet.assistant.cursors");

export function createCursors({ now = Date.now, key } = {}) {
  const core = new native.AsstCursors(key === undefined ? null : Buffer.from(key));
  return {
    make: ({ grant, revision, op, filters, after }) => core.make({ grant, revision, op, filters, after }, now()),
    read: (cursor, { grant, revision, op, filters }) => core.read(String(cursor), { grant, revision, op, filters }, now()),
    [CORE]: { core, now },
  };
}
