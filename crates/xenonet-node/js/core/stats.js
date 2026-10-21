// core/stats.js in Rust (crates/xenonet-state/src/stats.rs): a session's
// stats, from its transcript: the models it used and when, its tokens and
// what they'd cost at API prices on the day, its turns and how long they
// took, its tools, its subagents, and how full its context got.
//
// Rust finds the files and adds everything up. The transcripts are read
// here, through the lines twin (a compressed archive is unpacked there, and
// each line goes through the transcripts' normalizer), and Rust is handed
// each line's record.
import { native } from "../index.js";
import { readLines } from "./lines.js";

const strings = (list) => [...list].map(String);

// Where sessions' transcripts are. Codex rollouts are found by their thread
// id, from a listing of the rollout folders made once and again when an id
// isn't in it (at most every 10 seconds). A Grok Build session's updates
// are in its own folder, under the one for its working directory.
export class Transcripts {
  #core = new native.TranscriptsCore();

  constructor({ claudeRoot, codexRoots = [], grokRoot = null }) {
    this.claudeRoot = claudeRoot;
    this.codexRoots = codexRoots;
    this.grokRoot = grokRoot;
  }

  // The rollouts listed so far, by thread id.
  get rollouts() {
    return new Map(this.#core.rollouts);
  }
  get listedAt() {
    return this.#core.listedAt;
  }

  async files(sessionId) {
    return this.#core.files(String(sessionId), this.claudeRoot == null ? null : String(this.claudeRoot), strings(this.codexRoots), this.grokRoot == null ? null : String(this.grokRoot), Date.now());
  }

  async list() {
    this.#core.list(strings(this.codexRoots), Date.now());
  }
}

export function summarize(events, { app }) {
  return native.statsSummarize([...events], app);
}

// The whole thing, kept: the stats of the last 40 sessions asked about,
// while their files stay as they were. Rust says when they have; the stats
// themselves stay here, so the same answer is the same object.
export function createStats({ transcripts }) {
  const kept = new native.StatsKept();
  const cache = new Map(); // session id → stats

  return async function stats(sessionId) {
    const files = await transcripts.files(sessionId);
    if (!files.length) return null;
    const key = native.statsStamp(files.map((f) => String(f.file)));
    if (kept.hit(sessionId, key)) return cache.get(sessionId);
    const app = native.statsAppOf(sessionId);
    const read = await Promise.all(files.map(async (f) => ({ agentId: f.agentId, lines: (await readLines(f.file, app).catch(() => [])).map((o) => o.raw) })));
    const stats = native.sessionStats(sessionId, read);
    const gone = kept.keep(sessionId, key);
    cache.set(sessionId, stats);
    if (gone != null) cache.delete(gone);
    return stats;
  };
}
