// core/projects.js in Rust (crates/xenonet-state/src/projects.rs): the
// projects under your projects folder. The scan is Rust's: the scanner
// block's breadth-first walk hands Rust each folder it reads and asks it
// which subfolders to enter, and then Rust does the rest, reading the
// folders, repos and files it looks at itself, and decides what each
// folder is. The relations between projects are Rust's too. The async
// helpers (mapLimit, limiter), whose scheduling is the event loop's, stay
// here.
import { native } from "../index.js";
import { Scanner } from "./blocks/scanner.js";
import { normalizeRemote } from "./schema/ids.js";

// A folder holding one of these is a repo: a project of its own.
export const VCS_MARKERS = native.projectVcsMarkers();

// How far and how wide the scan goes. Depth counts levels below the
// projects root (its own folders are depth 1); folders read is a budget
// for one scan.
export const SCAN_LIMITS = native.projectScanLimits();

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

// The bottom-up pass remembers what it saw of a folder for a while;
// forgetScans() drops that, for "detect projects again".
export const forgetScans = () => native.forgetProjectScans();

// The top-down walk, as a building block: breadth first, 16 folders read
// at once, and every folder read on every scan.
export const projectsScanner = ({ clock } = {}) => new Scanner({ name: "projects", concurrency: 16, cache: false, pick: () => false, clock });

// Finding projects under the root, top-down and then bottom-up (see
// core/projects.js). `signal` (an AbortSignal) stops a scan no longer
// wanted: it reads nothing more, and rejects with the signal's reason.
export async function scanProjects(projectsDir, { sessionDirs = [], rules = {}, limits = {}, scanner = projectsScanner(), signal = null } = {}) {
  const { depth, maxDirs, maxSubmodules } = { ...SCAN_LIMITS, ...limits };
  const started = Date.now();
  const scan = new native.ProjectScan(projectsDir ? String(projectsDir) : "", rules ?? {});
  await scanner.scanBreadth({
    root: projectsDir,
    depth,
    folders: maxDirs,
    signal,
    look(dir, ents, level) {
      scan.look(String(dir), ents.map((e) => String(e.name)), ents.map((e) => !!e.isDirectory()), Number(level), !!signal?.aborted);
    },
    enter: (name, dir, level) => scan.enter(String(name), String(dir), Number(level)),
  });
  signal?.throwIfAborted();
  if (!scan.rootRead) return Object.assign([], { scan: { ok: false, ms: Date.now() - started } });
  const dirsRead = scanner.last.read;
  const truncated = !!scanner.last.truncated;
  const found = scan.finish([...sessionDirs], maxSubmodules, Date.now());
  return Object.assign(found.projects, {
    scan: { ok: true, at: Date.now(), ms: Date.now() - started, dirsRead, truncated, depth, maxDirs, groups: found.groups, projects: found.count },
    folders: found.folders,
  });
}

// It lives with the ids, since a project merged across hosts is named by it
// (repo:<remote>), and stays here for what imports it from here.
export { normalizeRemote };

// Satellite projects hang off a parent, a linked worktree orbits its repo,
// and clones of the same remote are linked as siblings.
export function projectRelations(projects) {
  const { parentOf, sameRemote } = native.projectRelations(projects);
  return { parentOf: new Map(parentOf), sameRemote };
}
