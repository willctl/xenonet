// core/native/copilot-cli.js in Rust (crates/xenonet-sources/src/native/
// copilot_cli.rs): each GitHub Copilot CLI session's event log is a session
// on the map, "copilot:<session id>", with its edit tools' files and its
// shutdown's changed files as its edits. Rust reads the folder and the logs
// (through `files` when a test gives its own) and keeps the runs; the
// vendor check stays vendor.js's, called back from Rust.
import fs from "node:fs";
import { Block } from "../blocks/lifecycle.js";
import { reader } from "../vendor.js";
import { native } from "../../index.js";
import { FAILED, guarded, ioOf, liveOf, mapOfRuns, pathArg } from "../../native-readers.js";

export const SOURCE = "github/copilot-cli/session-event";
// Copilot CLI's built-in file tools; str_replace_editor's view isn't an edit.
export const EDIT_TOOLS = new Set(["str_replace_editor", "edit", "create", "write", "apply_patch"]);

export const sessionIdOf = (id) => native.copilotSessionIdOf(String(id)) ?? null;

// The files a tool call names, if it's an edit.
export const editedBy = (toolName, args) => native.copilotEditedBy(toolName, args);

// One session's event log as XenoNet reads it, or null when it has no
// session.start.
export function readEvents(text, { check = (r) => r } = {}) {
  if (typeof text !== "string") throw new TypeError("text.split is not a function or its return value is not iterable");
  const box = {};
  const out = native.copilotReadEvents(text, guarded((rec) => check(rec), box), FAILED);
  if (out === FAILED) throw box.err;
  return out;
}

export const copilotRoot = ({ home }) => native.copilotRoot(pathArg(home));

export class CopilotCliSessions extends Block {
  #core = new native.CopilotCliCore();
  #runs = null;

  constructor({ root, files = fs, clock } = {}) {
    super({ name: "copilot-sessions", kind: "gatherer", ...(clock ? { clock } : {}) });
    this.root = root;
    this.files = files;
  }

  get runs() {
    return (this.#runs ??= mapOfRuns(this.#core.runs()));
  }

  get rootState() {
    return this.#core.rootState;
  }

  read(now = this.clock.now()) {
    try {
      const r = reader(SOURCE);
      const check = guarded((rec, version) => r.check(rec, { version }));
      this.#core.read(this.root == null ? null : String(this.root), Number(now), check, FAILED, ioOf(this.files, ["readdirSync", "statSync", "readFileSync"]));
      this.#runs = null;
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  sessions() {
    return this.#core.sessions();
  }

  live(id) {
    return liveOf(this.runs.get(id), false);
  }

  ids() {
    return this.#core.ids();
  }

  status() {
    return { name: "copilot", root: this.root, state: this.rootState, sessions: this.#core.size };
  }
}
