// core/refusals/store.js in Rust (crates/xenonet-sources/src/refusals/store.rs):
// every refusal XenoNet sees, kept on this computer to build a dataset
// from, with its context when you've turned that on. Rust reads and writes
// the files; the time and whether to keep context come from the callers
// given here, asked when JS asked them.
import os from "node:os";
import { native } from "../../index.js";

export const SCHEMA = native.refusalSchema();

// An answer from Rust, or its error thrown as Node would have thrown it.
function unwrap(a) {
  if (!a.error) return a.ok;
  const { message, code } = a.error;
  if (code === "RangeError") throw new RangeError(message);
  throw Object.assign(new Error(message), code ? { code } : {});
}

export const claudeContext = (all, sig) => native.refusalClaudeContext(all, sig);
export const codexContext = (all, sig) => native.refusalCodexContext(all, sig);

export const contextOn = (dir, env = process.env) => native.refusalContextOn(String(dir), env.XENONET_REFUSAL_CONTEXT ?? null);
export const setContext = (dir, on) => unwrap(native.refusalSetContext(String(dir), !!on));

const text = (v) => (v ? String(v) : null);

export function createRefusals({ dir, home = os.homedir(), now = Date.now, context = () => false }) {
  const core = new native.RefusalStore(String(dir), String(home ?? ""));
  function record(sig, at = {}) {
    if (core.known(sig, at)) return null;
    return unwrap(core.record(sig, at, now(), !!context()));
  }
  const get = (id) => core.get(String(id));
  const list = ({ since = 0, provider = null, kind = null } = {}) => core.list(Number(since), text(provider), text(kind));
  const capture = (id) => unwrap(core.capture(String(id)));
  const exportLines = ({ since = 0, provider = null, kind = null, withContext = false, scrubbed = false } = {}) => core.exportLines(Number(since), text(provider), text(kind), !!withContext, !!scrubbed);
  const contextFile = (id) => core.contextFile(String(id));
  return { record, get, list, capture, exportLines, contextFile, dir };
}
