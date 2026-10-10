// Uncommitted work per project, from `git status --porcelain -b`.
import { execFile, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { onPath } from "./paths.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";


// On a Mac without the Command Line Tools, /usr/bin/git is a stub that opens
// an "install the developer tools" dialog every time it runs, and XenoNet checks
// repos every 20 seconds. Use git only when it's real.
let gitOk = null;
export function gitAvailable({ platform = process.platform, which = onPath, devTools = () => execFileSync("/usr/bin/xcode-select", ["-p"], { stdio: "ignore", timeout: 3000 }) } = {}) {
  if (gitOk !== null) return gitOk;
  const git = which("git");
  gitOk = !!git;
  if (platform === "darwin" && git === "/usr/bin/git") {
    try {
      devTools();
    } catch {
      gitOk = false;
      console.log("[xenonet] git needs the Command Line Tools (xcode-select --install); repo status is off until they're installed");
    }
  }
  return gitOk;
}
export const resetGitCheck = () => (gitOk = null);

export function parseStatus(out) {
  const lines = String(out ?? "").split(/\r?\n/).filter(Boolean);
  const head = lines[0]?.startsWith("## ") ? lines.shift().slice(3) : "";
  const branch = head.startsWith("No commits yet on ") ? head.slice(18) : head.split("...")[0].replace(/^HEAD \(no branch\)$/, "detached");
  const ahead = Number(/ahead (\d+)/.exec(head)?.[1] ?? 0);
  const behind = Number(/behind (\d+)/.exec(head)?.[1] ?? 0);
  let staged = 0, modified = 0, untracked = 0;
  for (const l of lines) {
    if (l.startsWith("??")) untracked++;
    else {
      if (l[0] !== " ") staged++;
      if (l[1] !== " ") modified++;
    }
  }
  return { branch: branch || null, ahead, behind, staged, modified, untracked, dirty: lines.length };
}

// A repo's status, or null when git can't give it. `exec` stands in for
// execFile in tests.
//
// XenoNet runs git status in every repo in the projects folder every 20
// seconds, ones you cloned or unpacked included, and a repo's own config
// (an unpacked one's .git came with it) can name programs for git status to
// run: an fsmonitor hook, the clean command of a filter driver its
// attributes pick, the hook git runs when it rewrites the index, and the
// commands of a fetch. So status runs
//   - with core.fsmonitor off: a repo that uses one still answers, by
//     looking at its files itself;
//   - with hooks pointed at nothing, and without optional locks, so it never
//     rewrites the index either;
//   - with the filter drivers the repo's own config gives commands (in its
//     .git/config or config.worktree, or a file those include) emptied,
//     but for one whose commands there are git-lfs's alone. git config
//     lists them first, with where each comes from, and runs nothing.
//     Yours, from your global and system config, run as they do anywhere,
//     unless the repo's config gives git-lfs extensions, commands git-lfs
//     runs: then every driver is emptied. A changed file is read as it is
//     on disk, so one an emptied filter keeps different in the repo can
//     count as modified;
//   - without looking for work left inside submodules, which takes a status
//     in each, under its own config: a submodule counts as changed when
//     it's on another commit;
//   - without fetching objects a partial clone left out (GIT_NO_LAZY_FETCH,
//     from git 2.44), which would run the repo's remote's commands.
// A driver whose name -c can't say (one with an =) means no status at all.
const GUARDS = ["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"];
const STATUS_ARGS = ["status", "--porcelain=v1", "-b", "--untracked-files=normal", "--ignore-submodules=dirty"];
const DRIVERS_ARGS = ["config", "--show-scope", "--get-regexp", "-z", "^(filter|lfs)\\."];

// git-lfs's commands, as git lfs install writes them (with --skip-smudge or
// not, and with --local in the repo's own config): a driver that runs only
// these is left on, so files kept in LFS don't all count as modified once
// they're touched.
const LFS = {
  clean: ["git-lfs clean -- %f"],
  smudge: ["git-lfs smudge -- %f", "git-lfs smudge --skip -- %f"],
  process: ["git-lfs filter-process", "git-lfs filter-process --skip"],
};

// The scopes git config says a setting comes from that are yours: your
// system and global config (and files those include) and a -c of XenoNet's
// own. Any other (local, worktree, and the files those include, which git
// says are in the including file's scope) is the repo's.
const YOURS = new Set(["system", "global", "command"]);

// The -c settings that empty the drivers in `listing` that the repo's own
// config gives commands, but for one whose every command there is
// git-lfs's, or null when one can't be emptied. `listing` is what
// DRIVERS_ARGS prints: scope, NUL, key, newline, value, NUL; without
// `scoped` (git before 2.26), key, newline, value, NUL, all of it counted
// as the repo's. Git-lfs extensions in the repo's config are commands
// git-lfs runs, through whichever driver runs it, yours too: then every
// driver is emptied. An emptied driver isn't required, or git would stop
// at the first file it filters.
export function filterOverrides(listing, { scoped = true } = {}) {
  const drivers = new Set();
  const lfsOnly = new Map(); // driver name → whether the commands the repo gives it are all git-lfs's
  let extensions = false;
  const fields = String(listing ?? "").split("\0");
  for (let i = 0; i < fields.length; i++) {
    const scope = scoped ? fields[i++] : "local";
    const entry = fields[i];
    if (!entry) continue;
    const repo = !YOURS.has(scope);
    const nl = entry.indexOf("\n");
    const key = nl < 0 ? entry : entry.slice(0, nl);
    const value = nl < 0 ? null : entry.slice(nl + 1);
    if (/^lfs\.extension\./i.test(key)) {
      extensions ||= repo;
      continue;
    }
    // filter.<name>.<variable>; a name can hold dots and spaces.
    const dot = key.lastIndexOf(".");
    if (!key.startsWith("filter.") || dot < 7) continue;
    const name = key.slice(7, dot);
    const variable = key.slice(dot + 1).toLowerCase();
    drivers.add(name);
    if (!repo || !Object.hasOwn(LFS, variable)) continue;
    lfsOnly.set(name, (lfsOnly.get(name) ?? true) && LFS[variable].includes(value));
  }
  const emptied = extensions ? [...drivers] : [...lfsOnly].filter(([, lfs]) => !lfs).map(([name]) => name);
  const args = [];
  for (const name of emptied) {
    if (name.includes("=")) return null;
    for (const setting of ["clean=", "smudge=", "process=", "required=false"]) args.push("-c", `filter.${name}.${setting}`);
  }
  return args;
}

export function gitStatus(dir, { exec = execFile } = {}) {
  const options = { timeout: 8000, windowsHide: true, maxBuffer: 4 << 20, env: { ...process.env, GIT_NO_LAZY_FETCH: "1" } };
  const run = (args) => new Promise((resolve) => exec("git", ["-C", dir, ...args], options, (err, stdout) => resolve({ err, stdout })));
  return (async () => {
    let drivers = await run(DRIVERS_ARGS);
    let scoped = true;
    // git before 2.26 doesn't know --show-scope, and exits 129 on an
    // option it doesn't know: there every driver counts as the repo's.
    if (drivers.err?.code === 129) [drivers, scoped] = [await run(DRIVERS_ARGS.filter((a) => a !== "--show-scope")), false];
    // Exit 1: no key matched, so no drivers.
    if (drivers.err && drivers.err.code !== 1) return null;
    const filters = filterOverrides(drivers.err ? "" : drivers.stdout, { scoped });
    if (!filters) return null;
    const { err, stdout } = await run([...GUARDS, ...filters, ...STATUS_ARGS]);
    return err ? null : { ...parseStatus(stdout), checkedAt: Date.now() };
  })();
}

// Refreshes repo status for every git project, a few at a time. Each
// repo's status is a gatherer (core/blocks/gatherer.js) that keeps what it
// had when git can't answer. The watch is a block itself, "git-status",
// degraded while some repo's status couldn't be read. `status` and
// `available` stand in for git in tests. Repos no longer among the
// projects (a folder gone or left out, or another projects folder in use)
// drop out, so nothing counts them any more.
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
          if (!s) throw new Error(`git couldn't read the status of ${r.p.path}`);
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
    if (failed.length) this.problem(failed.length === 1 ? failed[0] : `${failed[0]}, and ${failed.length - 1} more`);
    else this.ok();
    return this.status;
  }

  health() {
    const h = super.health();
    h.counters = { ...h.counters, repos: this.status.size, reads: [...this.sources.values()].reduce((n, r) => n + (r.source.counters.runs ?? 0), 0) };
    return h;
  }
}
