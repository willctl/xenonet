// core/log.js in Rust (crates/xenonet-base/src/log.rs): XenoNet's error log,
// JSON lines under <state>/logs, scrubbed as they're written (the home
// folder as ~, keys out), once every ten seconds for the same code and
// words, and moved aside past their size.
import os from "node:os";
import path from "node:path";
import { native } from "../index.js";

const C = native.logConstants();
export const LEVELS = C.levels;
export const COMPONENTS = C.components;

export const scrub = (text, home = os.homedir()) => (text == null ? text : native.scrub(String(text), String(home ?? "")));

export const unsafe = (text, home = os.homedir()) => native.unsafeText(String(text), String(home ?? "")) ?? null;

export const logDir = (stateDir) => path.join(stateDir, "logs");

export function createLog({ dir, component, instance = null, version = null, home = os.homedir(), now = Date.now, maxBytes = C.maxBytes, mirror = null }) {
  const core = new native.LogCore(dir ? String(dir) : null, String(component), instance ? String(instance) : null, version ? String(version) : null, String(home ?? ""), Number(maxBytes));
  const errOf = (err) => (err ? { message: err.message == null ? null : String(err.message), stack: typeof err.stack === "string" ? err.stack : null, code: typeof err.code === "string" ? err.code : null } : null);
  function write(level, code, message, { err = null, fix = null, ctx = null } = {}) {
    const entry = core.write(level, String(code), message == null ? null : String(message), errOf(err), fix ? String(fix) : null, ctx || null, now()) ?? null;
    if (entry) mirror?.(entry);
    return entry;
  }
  return {
    error: (code, message, opts) => write("error", code, message, opts),
    warn: (code, message, opts) => write("warn", code, message, opts),
    info: (code, message, opts) => write("info", code, message, opts),
    file: dir ? path.join(dir, `${component}.ndjson`) : null,
  };
}

export function readLogs(dir, { since = 0, level = "info", component = null, code = null, limit = 500 } = {}) {
  return native.readLogs(String(dir), Number(since), String(level), component ?? null, code ?? null, Number(limit));
}

export const summarize = (entries) => native.summarize(entries);
