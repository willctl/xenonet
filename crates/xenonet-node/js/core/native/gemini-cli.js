// core/native/gemini-cli.js in Rust (crates/xenonet-sources/src/native/
// gemini_cli.rs): each Gemini CLI session in its chat records is a session
// on the map, "gemini-cli:<sessionId>", a subagent's under its parent, with
// its write_file and replace calls' files as its edits. Rust reads the
// folders and the records (through `files` when a test gives its own) and
// keeps the runs; the vendor check stays vendor.js's, called back from Rust.
import fs from "node:fs";
import { NativeReader } from "../../../../../core/native/reader.js";
import { reader } from "../vendor.js";
import { native } from "../../index.js";
import { APP } from "../../../../../core/apps.js";
import { FAILED, guarded, ioOf, mapOfRuns, pathArg } from "../../native-readers.js";

export const SOURCE = "google/gemini-cli/chat-record";
export const EDIT_TOOLS = new Set(APP["gemini-cli"].editTools);

export const sessionIdOf = (id) => native.geminiSessionIdOf(String(id)) ?? null;

// One chat record file as XenoNet reads it: { id, kind, summary,
// startedAt, updatedAt, edits }, or null when it isn't one.
export function readChat(text, { check = () => {} } = {}) {
  if (typeof text !== "string") throw new TypeError("text.trimStart is not a function");
  const box = {};
  const out = native.geminiReadChat(text, guarded((rec) => check(rec), box), FAILED);
  if (out === FAILED) throw box.err;
  return out;
}

// The Gemini CLI's home: GEMINI_CLI_HOME, else your home.
export const geminiRoot = ({ env = process.env, home }) => {
  const own = env.GEMINI_CLI_HOME;
  return native.geminiRoot(own ? pathArg(own) : null, own ? "" : pathArg(home));
};

export class GeminiCliSessions extends NativeReader {
  #core = new native.GeminiCliCore();
  #runs = null;

  constructor({ root, files = fs, clock } = {}) {
    super({ name: "gemini-cli-sessions", clock });
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
      const check = guarded((rec) => r.check(rec, { version: null }));
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

  ids() {
    return this.#core.ids();
  }

  status() {
    return { name: "gemini-cli", root: this.root, state: this.rootState, sessions: this.#core.size };
  }
}
