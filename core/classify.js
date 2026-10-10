// The optional Codex pass over the projects folder. XenoNet's own rules (see
// scanProjects) decide by repos, workspaces and where sessions ran. A folder
// with none of those, like a folder of examples or a top-level folder of
// experiments, is a guess. When you ask, and only then, XenoNet sends Codex an
// outline of the folders it looked at and asks which of them are really one
// project, or a group of separate ones.
//
// What Codex gets, for each folder: its path inside the projects folder (never
// the path above it, so never your user name), which well-known files it has
// (package.json, README, ...), how many files and folders it holds, its
// commonest file types, how many sessions ran there, and XenoNet's own call.
// For a repo, only the path and the sessions. Never other file names, what's
// in any file, or transcripts.
//
// Codex's picks can make a folder a project or a group. They never hide
// anything, and your own rules win over them. XenoNet keeps them with a
// fingerprint of the outline they came from, so it can tell when the folders
// have changed since, and never asks again by itself.
import crypto from "node:crypto";
import { askCodex } from "./digest.js";

export const PICKS_SCHEMA = {
  type: "object",
  properties: {
    picks: {
      type: "array",
      items: {
        type: "object",
        properties: {
          path: { type: "string" },
          as: { type: "string", enum: ["project", "group"] },
          reason: { type: "string" },
        },
        required: ["path", "as", "reason"],
        additionalProperties: false,
      },
    },
  },
  required: ["picks"],
  additionalProperties: false,
};

// The outline sent to Codex, from a scan's `folders`. Repos are settled, so
// they go as context only: a path and a session count.
export function outlineFolders(folders) {
  return folders.map((f) => (f.repo ? { path: f.rel, xenonet: "repo", markers: [], files: null, folders: null, types: {}, sessions: f.sessions } : { path: f.rel, xenonet: f.call, markers: f.markers, files: f.files, folders: f.folders, types: f.kinds, sessions: f.sessions }));
}

// Folders come and go and gain or lose a repo or a manifest; file counts
// and sessions change all the time and don't count.
export function fingerprint(outline) {
  const shape = outline.map((f) => [f.path, f.xenonet === "repo", f.markers]);
  return crypto.createHash("sha256").update(JSON.stringify(shape)).digest("hex").slice(0, 16);
}

export function picksPrompt(outline) {
  return `You are sorting the folders in a developer's projects folder, for a tool that draws a map of the projects their Claude Code and Codex sessions work in. Every project you add becomes a node on that map, so add one only when the developer would want to see it there.

Each entry below is one folder, by its path inside the projects folder. It lists the well-known files the folder holds (markers), how many files and subfolders it has, its commonest file types, how many coding sessions ran in it or below it, and how the tool sees it now ("xenonet"):
- "repo": it has its own version control, so it is a project. Don't pick it.
- "project": one project.
- "group": a folder that holds separate projects; each is judged on its own.
- "folder": neither, just a folder inside a project or a group.

Pick only the folders the tool clearly has wrong:
- "project": something the developer builds and comes back to as a unit (an app, a library, a service, a tool), even without a repo of its own. Signs of one: a manifest together with a README, a lockfile, tests, a Dockerfile or compose file, an AGENTS.md or CLAUDE.md, or sessions run in it. Not a project: examples and samples, tutorials worked through, scratch or one-off scripts, generated output or media, and a part of a bigger project next to it or above it (its infrastructure, docs, assets or tests). A manifest alone doesn't make a project.
- "group": a top-level "project" that is really a collection of separate projects, so its subfolders should count on their own.
Leave out repos and anything you aren't sure about. Most folders need no pick, and none at all is a fine answer. Give each pick a reason of under 15 words. Do not run any commands.

Folders:
${JSON.stringify(outline)}`;
}

// Codex's answer, keeping only picks for folders it was sent that would
// change something.
export function parsePicks(text, outline) {
  let data;
  try {
    data = JSON.parse(String(text).trim().replace(/^```(?:json)?\s*|\s*```$/g, ""));
  } catch {
    throw new Error("Codex didn't answer in the expected shape");
  }
  const byPath = new Map(outline.map((f) => [f.path, f]));
  const picks = new Map();
  for (const p of Array.isArray(data?.picks) ? data.picks : []) {
    const f = byPath.get(p?.path);
    if (!f || f.xenonet === "repo" || !["project", "group"].includes(p.as) || f.xenonet === p.as || picks.has(f.path)) continue;
    picks.set(f.path, { path: f.path, as: p.as, reason: String(p.reason ?? "").trim().slice(0, 200) });
  }
  return [...picks.values()];
}

export async function askCodexForPicks(codexBin, outline, opts) {
  return parsePicks(await askCodex(codexBin, picksPrompt(outline), { schema: PICKS_SCHEMA, timeoutMs: 180000, ...opts }), outline);
}

// The same question for a decision model (core/decide.js), as one choice a
// folder: project, group or folder, with the outline as what it knows. It
// says how sure it is of each, and only a pick it's sure of (CONFIDENT) that
// would change something is kept, its reason how sure it was. A big
// projects folder goes in a few requests.
export const CONFIDENT = 0.75;
const PER_REQUEST = 40;
const CRITERIA = {
  project: "One project: something the developer builds and comes back to as a unit (an app, a library, a service, a tool), even without a repo of its own. Signs: a manifest with a README, a lockfile, tests, a Dockerfile, an AGENTS.md or CLAUDE.md, or coding sessions run in it.",
  group: "A collection of separate projects, whose subfolders should each count on their own.",
  folder: "Neither: examples or samples, tutorials, scratch or one-off scripts, generated output or media, or a part of a bigger project beside or above it (its docs, assets, tests or infrastructure).",
};

export function jevQuestions(outline) {
  const asked = outline.filter((f) => f.xenonet !== "repo");
  const batches = [];
  for (let i = 0; i < asked.length; i += PER_REQUEST) {
    const part = asked.slice(i, i + PER_REQUEST);
    batches.push({
      folders: part,
      questions: Object.fromEntries(part.map((f, j) => [`f${i + j}`, { type: "choice", instructions: `For a map of a developer's projects folder, what is the folder "${f.path}"? The tool sees it now as "${f.xenonet}". Most folders are what the tool sees; answer otherwise only when the folder clearly is.`, criteria: CRITERIA }])),
    });
  }
  return batches;
}

export function picksFromAnswers(answers, folders, offset = 0) {
  const picks = [];
  folders.forEach((f, j) => {
    const a = answers[`f${offset + j}`];
    if (!a || !["project", "group"].includes(a.choice) || a.choice === f.xenonet || !(a.confidence >= CONFIDENT)) return;
    picks.push({ path: f.path, as: a.choice, reason: `the decision model was ${Math.round(a.confidence * 100)}% sure` });
  });
  return picks;
}

export async function askDeciderForPicks(decider, outline) {
  const state = { about: "Folders in a developer's projects folder, by path inside it, with the well-known files each holds (markers), how many files and subfolders, its commonest file types, how many coding sessions ran in it, and how the tool sees it now (xenonet). Repos are settled and listed only as context.", folders: outline };
  const picks = [];
  let offset = 0;
  for (const batch of jevQuestions(outline)) {
    const r = await decider.decide({ kind: "projects", state, questions: batch.questions });
    picks.push(...picksFromAnswers(r.answers, batch.folders, offset));
    offset += batch.folders.length;
  }
  return picks;
}
