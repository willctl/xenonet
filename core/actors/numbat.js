// The numbat bridge: an optional observer that reads the NDJSON event
// records numbat writes (numbat, by Perplexity, records what coding agents
// do through their hooks and session files; https://github.com/perplexityai/numbat)
// and shows the agents XenoNet doesn't read itself as observed actors, one
// per agent: actor:numbat-<agent>, such as actor:numbat-gemini-cli.
//
// It's off unless you name the file numbat writes (XENONET_NUMBAT_FILE),
// the one its hooks write with `--emit events --output file`. XenoNet never
// installs or configures numbat, and never writes to its file.
//
// From each event record (numbat's published record schema, versions 0.2.0
// to 0.5.0), XenoNet keeps only:
//
//   source_agent    which actor it is
//   event_type      what it did: file.write and file.delete are edits,
//                   command.exec a command run, anything else presence
//   file_path       an edited file, joined to project_path when relative,
//                   and kept only inside a place on the map
//   project_path    where it worked
//   session_id      how many sessions it has at work
//   timestamp       when
//   event_id        so an event counts once, across restarts too
//
// Never the endpoint (host, user), commands, prompts, previews, tool input or
// results. An edit in a place XenoNet knows counts in the warnings sessions
// get before an edit, marked observed, as Hermes's do.
//
// Claude Code and Codex are left out, since XenoNet reads their sessions
// itself, and Hermes, which has an observer of its own: nothing counts
// twice. Only the agents named in AGENT_NAMES are shown.
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { PRODUCTS } from "./products.js";

const PRODUCT = PRODUCTS.numbat;
export const SCHEMA_VERSIONS = new Set(["0.2.0", "0.3.0", "0.4.0", "0.5.0"]);
// Agents XenoNet reads by itself or through another observer, agents it
// doesn't show through numbat yet, and records that name no agent.
export const LEFT_OUT = new Set(["claude-code", "codex", "hermes", "grok", "unknown"]);
// The names numbat's source_agent values go by.
export const AGENT_NAMES = {
  cowork: "Claude Cowork",
  "gemini-cli": "Gemini CLI",
  cursor: "Cursor",
  windsurf: "Windsurf",
  copilot: "GitHub Copilot CLI",
  vscode: "VS Code agent mode",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  antigravity: "Antigravity",
  factory: "Factory Droid",
  "devin-cli": "Devin CLI",
  "kimi-code": "Kimi Code",
  pi: "Pi",
  "qwen-code": "Qwen Code",
  cline: "Cline",
  amp: "Amp",
  auggie: "Auggie",
  kiro: "Kiro",
  goose: "Goose",
  kilo: "Kilo Code",
  openhands: "OpenHands",
  crush: "Crush",
  junie: "Junie",
};
const EDITS = new Set(["file.write", "file.delete"]);
const WORKING_MS = 2 * 60e3;
const IDLE_MS = 30 * 60e3;
const DAY_MS = 24 * 3600e3;
const START_BACK = 1 << 20; // a first look reads at most the last MiB
const READ_MAX = 4 << 20; // and each look at most 4 MiB
const LINE_MAX = 1 << 20;

const str = (v) => (typeof v === "string" && v.length ? v : null);
export const slugOf = (agent) => `numbat-${agent}`.slice(0, 32);

// One NDJSON line as XenoNet reads it: { ok: true, event } with the fields
// above, or { ok: false, why } for a line it passes over.
export function readRecord(line) {
  let r;
  try {
    r = JSON.parse(line);
  } catch {
    return { ok: false, why: "malformed" };
  }
  if (!r || typeof r !== "object" || Array.isArray(r)) return { ok: false, why: "malformed" };
  if (r.record_type !== "event") return { ok: false, why: "not-an-event" };
  if (!SCHEMA_VERSIONS.has(r.schema_version)) return { ok: false, why: "unsupported-version" };
  const agent = str(r.source_agent);
  const type = str(r.event_type);
  const id = str(r.event_id);
  if (!agent || !type || !id || !/^[a-z0-9][a-z0-9-]*$/.test(agent)) return { ok: false, why: "malformed" };
  if (LEFT_OUT.has(agent) || !Object.hasOwn(AGENT_NAMES, agent)) return { ok: false, why: "left-out" };
  const at = Date.parse(r.timestamp ?? "");
  const project = str(r.project_path);
  let file = str(r.file_path);
  if (file && !path.isAbsolute(file)) file = project && path.isAbsolute(project) ? path.join(project, file) : null;
  return {
    ok: true,
    event: { id, agent, type, at: Number.isFinite(at) ? at : null, session: str(r.session_id), project: project && path.isAbsolute(project) ? project : null, file: EDITS.has(type) ? file : null },
  };
}

export class NumbatObserver extends Observer {
  // `file`: the NDJSON file numbat writes. `files`: fs, for the tests.
  constructor({ actors, file, files = fs, observing, every, clock } = {}) {
    super({ name: "numbat-observer", product: PRODUCT, actors, observing, every, clock });
    this.file = file;
    this.files = files;
    this.offset = null;
    this.partial = "";
    this.seen = new Map(); // agent → { at, sessions: Map<session, at>, place }
    this.fileState = "unread";
    this.skipped = 0;
  }

  // The new lines since the last look, whole ones only.
  #read() {
    let st;
    try {
      st = this.files.statSync(this.file);
    } catch {
      this.fileState = "missing";
      return [];
    }
    let skipFirst = false;
    if (this.offset == null || st.size < this.offset || (this.ino != null && st.ino !== this.ino)) {
      // A first look, or a file rotated or cut short: from the last MiB.
      const from = this.offset == null ? Math.max(0, st.size - START_BACK) : 0;
      skipFirst = from > 0;
      this.offset = from;
      this.partial = "";
    }
    this.ino = st.ino;
    const len = Math.min(st.size - this.offset, READ_MAX);
    if (len <= 0) {
      this.fileState = "ok";
      return [];
    }
    const buf = Buffer.alloc(len);
    const fd = this.files.openSync(this.file, "r");
    try {
      this.files.readSync(fd, buf, 0, len, this.offset);
    } finally {
      this.files.closeSync(fd);
    }
    this.offset += len;
    this.fileState = "ok";
    const lines = (this.partial + buf.toString("utf8")).split("\n");
    this.partial = lines.pop() ?? "";
    if (this.partial.length > LINE_MAX) this.partial = "";
    if (skipFirst) lines.shift();
    return lines.filter((l) => l.trim() && l.length <= LINE_MAX);
  }

  async look(at = this.clock.now()) {
    for (const line of this.#read()) {
      const r = readRecord(line);
      if (!r.ok) {
        this.skipped++;
        continue;
      }
      this.#take(r.event, at);
    }
    for (const [agent, s] of this.seen) this.#say(agent, s, at);
  }

  #take(e, now) {
    const when = Math.min(e.at ?? now, now);
    if (now - when > DAY_MS) return;
    const id = `actor:${slugOf(e.agent)}`;
    this.keep(id, { name: `${AGENT_NAMES[e.agent]} (through numbat)`, via: ["observed"], heartbeatMs: null, installed: true, at: now });
    const s = this.seen.get(e.agent) ?? { at: 0, sessions: new Map(), place: null };
    s.at = Math.max(s.at, when);
    if (e.session) s.sessions.set(e.session, Math.max(s.sessions.get(e.session) ?? 0, when));
    const place = this.actors.placeOf(e.file ?? e.project);
    if (place) s.place = place;
    this.seen.set(e.agent, s);
    if (!this.actors.enabled(id)) return;
    if (place) this.touch(id, place, "works-on", now);
    if (e.type === "file.write" || e.type === "file.delete") {
      // An edit outside every place XenoNet knows is left out.
      if (e.file && place) this.happened(id, { ref: `numbat:${e.id}`.slice(0, 200), kind: "edited", target: place, paths: [path.resolve(e.file)], at: when });
    } else if (e.type === "command.exec" && place) this.gather(id, { kind: "ran", target: place, at: when });
  }

  #say(agent, s, now) {
    const id = `actor:${slugOf(agent)}`;
    if (!this.actors.enabled(id)) return;
    for (const [k, t] of s.sessions) if (now - t > IDLE_MS) s.sessions.delete(k);
    const ago = now - s.at;
    const presence = ago < WORKING_MS ? "working" : ago < IDLE_MS ? "idle" : "away";
    const where = s.place ? this.actors.map?.describe?.(s.place)?.name : null;
    const n = s.sessions.size;
    const status = presence === "away" ? "Nothing from numbat in the last half hour" : `${n > 1 ? `${n} sessions` : "At work"}${where ? ` in ${where}` : ""}, as numbat records it`;
    this.say(id, { presence, status, at: now });
  }

  // What the doctor shows: the file it reads, and whether it could.
  status() {
    return { product: PRODUCT.id, name: PRODUCT.name, installed: this.fileState === "ok", sources: [{ name: "numbat events", file: this.file, state: this.fileState }], recognized: null };
  }
}
