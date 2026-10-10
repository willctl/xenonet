import fs from "node:fs/promises";
import path from "node:path";
import { pathKey, isUnder } from "./paths.js";
import { normalizeRemote } from "./schema/ids.js";
import { Scanner } from "./blocks/scanner.js";

const STACK_MARKERS = [
  ["package.json", "node"],
  ["tsconfig.json", "typescript"],
  ["pyproject.toml", "python"],
  ["requirements.txt", "python"],
  ["Cargo.toml", "rust"],
  ["go.mod", "go"],
  ["Gemfile", "ruby"],
  ["composer.json", "php"],
  ["docker-compose.yml", "docker"],
  ["compose.yaml", "docker"],
  ["Dockerfile", "docker"],
  ["supabase", "supabase"],
];

// A folder holding one of these is a repo: a project of its own.
export const VCS_MARKERS = [".git", ".hg", ".jj", ".sl", ".svn", ".bzr", "_darcs", ".pijul", ".fslckout", "_FOSSIL_"];
// A folder holding one of these is a monorepo's root: one project, however
// many packages it has. package.json, Cargo.toml and pyproject.toml count
// only when they declare a workspace (see workspaceOf).
const WORKSPACE_FILES = ["pnpm-workspace.yaml", "lerna.json", "nx.json", "turbo.json", "rush.json", "go.work", "MODULE.bazel", "WORKSPACE", "WORKSPACE.bazel"];
// Below the top level, XenoNet never looks inside dependencies, build output or
// caches. (At the top level only dot folders and node_modules are skipped, as
// before.)
const SKIP_DIRS = new Set(["node_modules", "bower_components", "vendor", "dist", "build", "out", "target", "coverage", "__pycache__", "venv", "Pods", "DerivedData", "site-packages", "cdk.out"]);

// How far and how wide the scan goes. Depth counts levels below the projects
// root (its own folders are depth 1); folders read is a budget for one scan.
export const SCAN_LIMITS = { depth: 3, maxDirs: 2000, maxSubmodules: 10 };

async function readText(file, max = 4096) {
  try {
    const fh = await fs.open(file, "r");
    try {
      const buf = Buffer.alloc(max);
      const { bytesRead } = await fh.read(buf, 0, max, 0);
      return buf.subarray(0, bytesRead).toString("utf8");
    } finally {
      await fh.close();
    }
  } catch {
    return null;
  }
}

async function gitInfo(dir) {
  let gitDir = path.join(dir, ".git");
  try {
    const st = await fs.stat(gitDir);
    if (st.isFile()) {
      // Worktree or submodule: `.git` is a file pointing at the real git dir.
      const m = /gitdir:\s*(.+)/.exec((await readText(gitDir)) ?? "");
      if (!m) return null;
      gitDir = path.resolve(dir, m[1].trim());
    }
  } catch {
    return null;
  }
  // A linked worktree's git dir names the main repo's .git in `commondir`.
  // The config, and so the remote, lives there.
  const common = (await readText(path.join(gitDir, "commondir")))?.trim();
  const commonDir = common ? path.resolve(gitDir, common) : gitDir;
  const head = (await readText(path.join(gitDir, "HEAD"))) ?? "";
  const branch = /ref:\s*refs\/heads\/(.+)/.exec(head)?.[1]?.trim() ?? (head.trim().slice(0, 12) || null);
  const cfg = (await readText(path.join(commonDir, "config"), 16384)) ?? "";
  const origin = /\[remote "origin"\][^[]*?url\s*=\s*(\S+)/.exec(cfg)?.[1] ?? null;
  // A submodule's git dir sits in its superproject's .git/modules.
  const sub = /^(.*)[\\/]\.git[\\/]modules[\\/]/.exec(gitDir);
  return {
    branch,
    origin,
    worktreeOf: common && path.basename(commonDir) === ".git" ? path.dirname(commonDir) : null,
    submoduleOf: !common && sub ? sub[1] : null,
  };
}

function firstParagraph(md) {
  if (!md) return null;
  for (const raw of md.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("<") || line.startsWith("![") || line.startsWith("[!")) continue;
    return line.replace(/[*_`]/g, "").slice(0, 180);
  }
  return null;
}

async function describe(dir, names) {
  const entries = names ?? new Set(await fs.readdir(dir).catch(() => []));
  const stack = [...new Set(STACK_MARKERS.filter(([f]) => entries.has(f)).map(([, s]) => s))];
  let description = null;
  if (entries.has("package.json")) {
    try {
      description = JSON.parse((await readText(path.join(dir, "package.json"), 65536)) ?? "{}").description ?? null;
    } catch {}
  }
  if (!description) {
    const readme = [...entries].find((e) => /^readme(\.md)?$/i.test(e));
    if (readme) description = firstParagraph(await readText(path.join(dir, readme)));
  }
  return { stack, description };
}

// Which file makes this folder a monorepo root, if any.
async function workspaceOf(dir, names) {
  const file = WORKSPACE_FILES.find((f) => names.has(f));
  if (file) return file;
  if (names.has("package.json")) {
    try {
      if (JSON.parse((await readText(path.join(dir, "package.json"), 65536)) ?? "{}").workspaces) return "package.json";
    } catch {}
  }
  if (names.has("Cargo.toml") && /^\s*\[workspace\]/m.test((await readText(path.join(dir, "Cargo.toml"), 65536)) ?? "")) return "Cargo.toml";
  if (names.has("pyproject.toml") && /^\s*\[tool\.uv\.workspace\]/m.test((await readText(path.join(dir, "pyproject.toml"), 65536)) ?? "")) return "pyproject.toml";
  return null;
}

// What one readdir says about a folder, from its entries.
async function factsOf(dir, ents) {
  const names = new Set(ents.map((e) => e.name));
  const vcs = VCS_MARKERS.find((m) => names.has(m)) ?? null;
  return { vcs, workspace: vcs ? null : await workspaceOf(dir, names), names, subdirs: ents.filter((e) => e.isDirectory()).map((e) => e.name) };
}

// What one readdir says about a folder. Null when it can't be read.
async function folderFacts(dir) {
  let ents;
  try {
    ents = await fs.readdir(dir, { withFileTypes: true });
  } catch {
    return null;
  }
  return factsOf(dir, ents);
}

// What the Codex pass may know about a folder: which of these well-known
// files it has, how many files and folders, and the commonest file types.
// Never other file names, and never what's in a file.
const KNOWN_FILES = new Set([...VCS_MARKERS, ...WORKSPACE_FILES, ...STACK_MARKERS.map(([f]) => f), ".gitmodules", "Makefile", "CMakeLists.txt", "Package.swift", "pom.xml", "build.gradle", "build.gradle.kts", "setup.py", "deno.json", "mix.exs", "uv.lock", "poetry.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "Cargo.lock", "go.sum", "index.html", "AGENTS.md", "CLAUDE.md", "LICENSE"]);
function outline(facts) {
  const subdirs = new Set(facts.subdirs);
  const files = [...facts.names].filter((n) => !subdirs.has(n));
  const kinds = new Map();
  for (const f of files) {
    if (f.startsWith(".")) continue;
    const ext = /\.([a-z0-9]{1,8})$/i.exec(f)?.[1]?.toLowerCase();
    if (ext) kinds.set(ext, (kinds.get(ext) ?? 0) + 1);
  }
  return {
    markers: [...facts.names].filter((n) => KNOWN_FILES.has(n) || /^readme(\.\w+)?$/i.test(n)).sort(),
    files: files.length,
    folders: facts.subdirs.filter((n) => !n.startsWith(".")).length,
    kinds: Object.fromEntries([...kinds].sort((a, b) => b[1] - a[1]).slice(0, 5)),
  };
}

// fn over the items, n at a time, with the results in order.
export async function mapLimit(items, n, fn) {
  const out = new Array(items.length);
  let next = 0;
  const worker = async () => {
    for (let i = next++; i < items.length; i = next++) out[i] = await fn(items[i]);
  };
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, worker));
  return out;
}

// Runs async functions at most n at a time, in the order they ask.
export function limiter(n) {
  let running = 0;
  const waiting = [];
  return async (fn) => {
    if (running < n) running++;
    else await new Promise((resolve) => waiting.push(resolve));
    try {
      return await fn();
    } finally {
      const next = waiting.shift();
      if (next) next();
      else running--;
    }
  };
}

const parentKey = (key) => key.slice(0, key.lastIndexOf("/"));

// The folder `key` names, spelled as on disk, found from a path inside it.
function spelled(dir, key) {
  for (let d = path.resolve(dir); ; d = path.dirname(d)) {
    if (pathKey(d) === key) return d;
    if (d === path.dirname(d)) return null;
  }
}

// The bottom-up pass reads the same few folders on every refresh, so it
// remembers what it saw for a while. forgetScans() drops that, for "detect
// projects again".
const FACTS_TTL_MS = 5 * 60e3;
const factsCache = new Map();
async function cachedFacts(dir, now = Date.now()) {
  const key = pathKey(dir);
  const hit = factsCache.get(key);
  if (hit && now - hit.at < FACTS_TTL_MS) return hit.facts;
  const facts = await folderFacts(dir);
  factsCache.set(key, { facts, at: now });
  return facts;
}
export const forgetScans = () => factsCache.clear();

// The top-down walk, as a building block (core/blocks/scanner.js): breadth
// first, 16 folders read at once, and every folder read on every scan,
// since what a folder is depends on what its files say (a package.json's
// workspaces) and on your rules, not only on the names in it. XenoNetState
// keeps one for the map's scans, so /api/health and the doctor say how
// they go; a scan without one (a check of another folder) has its own.
export const projectsScanner = ({ clock } = {}) => new Scanner({ name: "projects", concurrency: 16, cache: false, pick: () => false, clock });

// Finding projects under the root, top-down and then bottom-up.
//
// Top-down, the way ghq walks its root: a folder with its own repo is a
// project, and XenoNet doesn't look inside it. (VS Code's git scan, Project
// Manager and projectile reach the same answer by resolving every folder they
// find up to its repo.) XenoNet looks into every other folder, breadth first,
// down to `depth` levels and at most `maxDirs` folders.
//
// Bottom-up, the way git finds the repo you're in: from each folder a session
// ran in, XenoNet walks up to the nearest repo. That catches repos below the
// depth limit and repos nested inside another one. A linked worktree inside
// its own repo (Claude's .claude/worktrees/…) belongs to that repo.
//
// Then, for each folder:
//   it has its own repo                                    → a project
//   it holds a project somewhere below it                  → a group
//   it's at the top level, it defines a workspace (pnpm,
//   npm, Cargo, go.work, uv, …), or it sits in a group and
//   a session ran in it                                    → a project
// So repos are the unit: a folder of repos is a group even when it declares a
// workspace over them, and a monorepo in one repo is one project. A group
// below the top level that holds just one thing and no sessions of its own
// is only a step on the path, and is left out.
//
// `rules` (path key → { path, as }) override all of this: "project" makes a
// folder a project and stops XenoNet looking inside it, "group" makes XenoNet look
// inside even a repo, and "ignore" leaves the folder and everything in it out.
//
// `signal` (an AbortSignal) stops a scan no longer wanted, such as a check of
// a folder that took too long to wait for: it reads nothing more, and
// rejects with the signal's reason.
export async function scanProjects(projectsDir, { sessionDirs = [], rules = {}, limits = {}, scanner = projectsScanner(), signal = null } = {}) {
  const { depth, maxDirs, maxSubmodules } = { ...SCAN_LIMITS, ...limits };
  const started = Date.now();
  const rootKey = pathKey(projectsDir);
  const ruleFor = (key) => rules[key]?.as ?? null;
  const ignored = (key) => {
    for (let k = key; k.length > rootKey.length; k = parentKey(k)) if (ruleFor(k) === "ignore") return true;
    return false;
  };

  // ----- top-down -----
  // `why` is what makes a folder a project: "repo", "rule", "submodule",
  // "workspace", "sessions" or "folder"; null for the rest.
  const seen = new Map(); // key → { path, key, depth, facts, why }
  const depthOf = (key) => key.slice(rootKey.length).split("/").length - 1;
  const stops = (f) => f.why === "repo" || f.why === "rule" || f.why === "submodule";
  // The scanner's walk, a level at a time: each folder is looked at before
  // its subfolders are entered, and each level's first folders take the
  // budget. A folder you ignore is read, then left out with everything in
  // it; XenoNet doesn't look inside a repo or a folder you made one project.
  // At the top level only dot folders and node_modules are passed over,
  // below it dependencies, build output and caches too.
  const walked = new Map(); // folder → { f, inWorkspace } for its subfolders, or null when it's left out
  let rootRead = false;
  await scanner.scanBreadth({
    root: projectsDir,
    depth,
    folders: maxDirs,
    signal,
    async look(dir, ents, level) {
      if (level === 0) return void (rootRead = true);
      if (signal?.aborted) return;
      const key = pathKey(dir);
      const rule = ruleFor(key);
      if (rule === "ignore") return void walked.set(dir, null);
      const facts = await factsOf(dir, ents);
      // A workspace inside another one is one of its packages.
      const inWorkspace = walked.get(path.dirname(dir))?.inWorkspace ?? false;
      const why = rule === "project" ? "rule" : rule === "group" ? null : facts.vcs ? "repo" : facts.workspace && !inWorkspace ? "workspace" : null;
      const f = { path: dir, key, depth: level, facts, why };
      seen.set(key, f);
      walked.set(dir, { f, inWorkspace: inWorkspace || why === "workspace" });
    },
    enter(name, dir, level) {
      if (name.startsWith(".")) return false;
      if (level === 1) return name !== "node_modules";
      const parent = walked.get(dir);
      return !!parent && !stops(parent.f) && !SKIP_DIRS.has(name);
    },
  });
  signal?.throwIfAborted();
  if (!rootRead) return Object.assign([], { scan: { ok: false, ms: Date.now() - started } });
  const dirsRead = scanner.last.read;
  const truncated = !!scanner.last.truncated;
  const add = async (dir, why, facts) => {
    const key = pathKey(dir);
    if ((seen.get(key) && stops(seen.get(key))) || ignored(key)) return;
    facts ??= await folderFacts(dir);
    if (facts) seen.set(key, { path: dir, key, depth: depthOf(key), facts, why });
  };
  // The nearest folder at or above `key` that `test` picks.
  const nearest = (key, test, { self = true } = {}) => {
    for (let k = self ? key : parentKey(key); k.length > rootKey.length; k = parentKey(k)) if (seen.has(k) && test(seen.get(k))) return k;
    return null;
  };

  // Folders pinned as projects that the walk didn't reach. Codex's picks
  // inside a folder you made one project don't count: that folder keeps
  // everything in it.
  const yoursAbove = (key) => {
    for (let k = parentKey(key); k.length > rootKey.length; k = parentKey(k)) if (rules[k]?.as === "project" && rules[k].by !== "codex") return true;
    return false;
  };
  for (const [key, r] of Object.entries(rules)) {
    signal?.throwIfAborted();
    if (r.as !== "project" || !isUnder(key, rootKey) || key === rootKey || seen.get(key)?.why) continue;
    if (r.by === "codex" && yoursAbove(key)) continue;
    await add(r.path, "rule");
  }

  // Submodules are repos of their own, listed in .gitmodules.
  for (const f of [...seen.values()]) {
    signal?.throwIfAborted();
    if (f.facts.vcs !== ".git" || !f.facts.names.has(".gitmodules") || ruleFor(f.key) === "project") continue;
    const listed = [...((await readText(path.join(f.path, ".gitmodules"), 65536)) ?? "").matchAll(/^\s*path\s*=\s*(.+?)\s*$/gm)].map((m) => m[1]);
    for (const rel of listed.slice(0, maxSubmodules)) {
      const dir = path.join(f.path, rel);
      const facts = await folderFacts(dir);
      if (facts?.vcs) await add(dir, "submodule", facts);
    }
  }

  // ----- bottom-up, from where sessions ran -----
  const sessionKeys = new Map(); // key → folder
  const sessionCount = new Map(); // key → sessions that ran there
  for (const dir of sessionDirs) {
    const key = pathKey(dir);
    if (!key || !isUnder(key, rootKey) || key === rootKey || ignored(key)) continue;
    sessionKeys.set(key, dir);
    sessionCount.set(key, (sessionCount.get(key) ?? 0) + 1);
  }
  for (const [key, dir] of sessionKeys) {
    signal?.throwIfAborted();
    const repo = nearest(key, stops);
    // A folder you made one project keeps everything in it.
    if (repo && ruleFor(repo) === "project") continue;
    const inside = repo ?? nearest(key, (f) => f.why === "workspace");
    for (let d = dir, k = key; k.length > rootKey.length && k !== repo; d = path.dirname(d), k = parentKey(k)) {
      if (ruleFor(k) === "group" || k === inside) continue;
      const facts = seen.get(k)?.facts ?? (await cachedFacts(d));
      // Inside a repo or workspace only another repo counts.
      if (!facts?.vcs && (inside || !facts?.workspace)) continue;
      const git = facts.vcs === ".git" ? await gitInfo(d) : null;
      if (git?.worktreeOf && isUnder(k, pathKey(git.worktreeOf))) break;
      await add(d, facts.vcs ? "repo" : "workspace", facts);
      break;
    }
  }

  // ----- groups, and folders made projects by their sessions -----
  const groups = new Set();
  const findGroups = () => {
    groups.clear();
    const keys = [...seen.values()].filter((f) => f.why).map((f) => f.key);
    for (const f of seen.values()) if (!stops(f) && (ruleFor(f.key) === "group" || keys.some((k) => k !== f.key && isUnder(k, f.key)))) groups.add(f.key);
  };
  findGroups();
  const homeOf = (key) => nearest(key, (f) => !!f.why || groups.has(f.key));
  for (const [key, dir] of sessionKeys) {
    signal?.throwIfAborted();
    const home = homeOf(key);
    if (!home || !groups.has(home) || home === key) continue;
    const child = home + key.slice(home.length).match(/^\/[^/]+/)[0];
    if (seen.has(child)) seen.get(child).why = "sessions";
    else await add(spelled(dir, child), "sessions");
  }
  findGroups();
  // Plain top-level folders stay projects.
  for (const f of seen.values()) if (f.depth === 1 && !f.why && !groups.has(f.key)) f.why = "folder";

  // A group that is only a step on the path to one thing is left out.
  const kept = new Set([...seen.values()].filter((f) => f.why || groups.has(f.key)).map((f) => f.key));
  const above = (key) => nearest(key, (f) => kept.has(f.key), { self: false });
  const children = new Map();
  for (const key of kept) if (above(key)) children.set(above(key), (children.get(above(key)) ?? 0) + 1);
  const ownSessions = new Set([...sessionKeys.keys()].map(homeOf));
  for (const key of [...groups].sort((a, b) => b.length - a.length)) {
    if (seen.get(key).depth > 1 && !ruleFor(key) && children.get(key) === 1 && !ownSessions.has(key)) kept.delete(key);
  }

  signal?.throwIfAborted();
  const out = await Promise.all(
    [...kept].map(async (key) => {
      const f = seen.get(key);
      const group = groups.has(key);
      const [git, desc, st] = await Promise.all([f.facts.vcs === ".git" ? gitInfo(f.path) : null, describe(f.path, f.facts.names), fs.stat(f.path).catch(() => null)]);
      return {
        name: path.basename(f.path),
        path: f.path,
        key,
        rel: path.relative(projectsDir, f.path).split(path.sep).join("/"),
        role: group ? "group" : "project",
        detected: group ? (ruleFor(key) === "group" ? "rule" : "contains") : f.why,
        marker: f.facts.vcs ?? f.facts.workspace ?? null,
        parent: above(key),
        git,
        stack: desc.stack,
        description: desc.description,
        modifiedAt: st?.mtimeMs ?? null,
      };
    }),
  );
  out.sort((a, b) => a.key.localeCompare(b.key));
  const count = (role) => out.filter((p) => p.role === role).length;
  // Every folder the scan looked at, as names and counts only, for the
  // optional Codex pass (see classify.js) to judge.
  const sessionsIn = (key) => [...sessionCount].reduce((n, [k, c]) => n + (isUnder(k, key) ? c : 0), 0);
  const folders = [...seen.values()]
    .sort((a, b) => a.key.localeCompare(b.key))
    .map((f) => ({
      rel: path.relative(projectsDir, f.path).split(path.sep).join("/"),
      key: f.key,
      path: f.path,
      repo: !!f.facts.vcs,
      call: groups.has(f.key) ? "group" : f.why ? "project" : "folder",
      why: groups.has(f.key) ? "contains" : f.why,
      ...outline(f.facts),
      sessions: sessionsIn(f.key),
    }));
  return Object.assign(out, { scan: { ok: true, at: Date.now(), ms: Date.now() - started, dirsRead, truncated, depth, maxDirs, groups: count("group"), projects: count("project") }, folders });
}

// It lives with the ids, since a project merged across hosts is named by it
// (repo:<remote>), and stays here for what imports it from here.
export { normalizeRemote };

// Satellite projects hang off a parent: `ledger-local-supabase` orbits
// `ledger`, when both sit in the same folder. A linked worktree orbits its
// repo. Clones of the same remote are linked as siblings.
export function projectRelations(projects) {
  const real = projects.filter((p) => p.role !== "group");
  const byName = new Map(real.map((p) => [`${p.parent ?? ""}|${p.name.toLowerCase()}`, p]));
  const byKey = new Map(real.map((p) => [p.key, p]));
  const parentOf = new Map();
  for (const p of real) {
    const main = p.git?.worktreeOf ? byKey.get(pathKey(p.git.worktreeOf)) : null;
    if (main && main !== p) {
      parentOf.set(p.key, main.key);
      continue;
    }
    const parts = p.name.toLowerCase().split(/[-_.]/);
    for (let i = parts.length - 1; i > 0; i--) {
      const at = (sep) => byName.get(`${p.parent ?? ""}|${parts.slice(0, i).join(sep)}`);
      const candidate = at("-") ?? at("_");
      if (candidate && candidate !== p) {
        parentOf.set(p.key, candidate.key);
        break;
      }
    }
  }
  const sameRemote = [];
  const byRemote = new Map();
  for (const p of real) {
    const r = normalizeRemote(p.git?.origin);
    if (!r) continue;
    if (byRemote.has(r)) sameRemote.push([byRemote.get(r).key, p.key]);
    else byRemote.set(r, p);
  }
  return { parentOf, sameRemote };
}
