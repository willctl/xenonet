// core/git.js in Rust (crates/xenonet-sources/src/git.rs): uncommitted work
// per project, from `git status --porcelain -b`. Rust reads what git says
// (the status, the filter drivers a repo's config lists), decides the
// guards and overrides each status runs with and what each answer means
// for the next run, and words the watch's problems. Running git stays
// here, through `exec` (execFile, or what a test stands in for it), and so
// do the watch's turns (three repos at a time) and its gatherers.
import { execFile, execFileSync } from "node:child_process";
import { onPath } from "./paths.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";
import { native } from "../index.js";

// On a Mac without the Command Line Tools, /usr/bin/git is a stub that
// opens an "install the developer tools" dialog every time it runs. Use git
// only when it's real.
let gitOk = null;
export function gitAvailable({ platform = process.platform, which = onPath, devTools = () => execFileSync("/usr/bin/xcode-select", ["-p"], { stdio: "ignore", timeout: 3000 }) } = {}) {
  if (gitOk !== null) return gitOk;
  const git = which("git");
  gitOk = !!git;
  if (native.gitNeedsDevTools(String(platform), typeof git === "string" ? git : null)) {
    try {
      devTools();
    } catch {
      gitOk = false;
      console.log(native.gitNoDevTools());
    }
  }
  return gitOk;
}
export const resetGitCheck = () => (gitOk = null);

export const parseStatus = (out) => native.gitParseStatus(String(out ?? ""));

export const filterOverrides = (listing, { scoped = true } = {}) => native.gitFilterOverrides(String(listing ?? ""), !!scoped) ?? null;

// An exit code, as git's error carries it (a spawn error's is a string).
const codeOf = (err) => (typeof err?.code === "number" ? err.code : null);

// A repo's status, or null when git can't give it. `exec` stands in for
// execFile in tests.
export function gitStatus(dir, { exec = execFile } = {}) {
  const options = { timeout: 8000, windowsHide: true, maxBuffer: 4 << 20, env: { ...process.env, GIT_NO_LAZY_FETCH: "1" } };
  const run = (args) => new Promise((resolve) => exec("git", ["-C", dir, ...args], options, (err, stdout) => resolve({ err, stdout })));
  return (async () => {
    let scoped = true;
    let drivers = await run(native.gitDriversArgs(true));
    for (;;) {
      const next = native.gitAfterDrivers(codeOf(drivers.err), !!drivers.err, String(drivers.stdout ?? ""), scoped);
      if (next.unscoped) {
        [drivers, scoped] = [await run(native.gitDriversArgs(false)), false];
        continue;
      }
      if (!next.args) return null;
      const { err, stdout } = await run(next.args);
      return err ? null : { ...parseStatus(stdout), checkedAt: Date.now() };
    }
  })();
}

// Refreshes repo status for every git project, a few at a time. Each
// repo's status is a gatherer that keeps what it had when git can't
// answer. The watch is a block itself, "git-status", degraded while some
// repo's status couldn't be read.
export class RepoWatch extends Block {
  constructor({ status = gitStatus, available = gitAvailable, clock } = {}) {
    super({ name: "git-status", kind: "gatherer", clock });
    this.readStatus = status;
    this.available = available;
    this.status = new Map();
    this.sources = new Map(); // project key → { p, source }
  }

  #source(p) {
    let r = this.sources.get(p.key);
    if (!r) {
      r = {};
      r.source = new Gatherer({
        name: "git-status:repo",
        gather: async () => {
          const s = await this.readStatus(r.p.path);
          if (!s) throw new Error(native.gitUnreadable(String(r.p.path)));
          return s;
        },
        keep: true,
        clock: this.clock,
      });
      this.sources.set(p.key, r);
    }
    r.p = p;
    return r.source;
  }

  async refresh(projects) {
    if (!this.available()) return this.status;
    const queue = projects.filter((p) => p.git);
    const keep = new Set(queue.map((p) => p.key));
    for (const key of this.status.keys()) if (!keep.has(key)) this.status.delete(key);
    for (const key of this.sources.keys()) if (!keep.has(key)) this.sources.delete(key);
    const failed = [];
    const run = async () => {
      for (let p = queue.shift(); p; p = queue.shift()) {
        const source = this.#source(p);
        const s = await source.run();
        if (source.last.ok) this.status.set(p.key, s);
        else failed.push(source.last.error);
      }
    };
    await Promise.all([run(), run(), run()]);
    this.count("passes");
    const problem = native.gitProblem(failed.map(String));
    if (problem != null) this.problem(problem);
    else this.ok();
    return this.status;
  }

  health() {
    const h = super.health();
    h.counters = { ...h.counters, repos: this.status.size, reads: [...this.sources.values()].reduce((n, r) => n + (r.source.counters.runs ?? 0), 0) };
    return h;
  }
}
