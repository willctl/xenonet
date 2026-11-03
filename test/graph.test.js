import { test } from "node:test";
import assert from "node:assert/strict";
import { buildGraph, activityOf } from "../core/graph.js";
import { pathKey, isUnder, scratchZone } from "../core/paths.js";
import { projectRelations } from "../core/projects.js";

const windows = { activeWindowMs: 10 * 60e3, recentWindowMs: 24 * 3600e3 };
const now = Date.parse("2026-10-02T12:00:00Z");
const project = (name, extra = {}) => ({ name, path: `C:\\Users\\me\\Projects\\${name}`, key: pathKey(`C:\\Users\\me\\Projects\\${name}`), git: null, stack: [], ...extra });
const session = (id, cwd, extra = {}) => ({ id, app: id.split(":")[0], rawId: id.split(":")[1], title: id, cwd, updatedAt: now - 3600e3, ...extra });

test("pathKey normalizes Windows spellings of the same folder", () => {
  assert.equal(pathKey("\\\\?\\C:\\Users\\Me\\Projects\\x\\"), "c:/users/me/projects/x");
  assert.equal(pathKey("c:/users/me/projects/x"), "c:/users/me/projects/x");
  assert.ok(isUnder("c:/a/b/c", "c:/a/b"));
  assert.ok(!isUnder("c:/a/bc", "c:/a/b"));
});

test("scratch folders from both apps collapse into one zone each", () => {
  assert.equal(scratchZone(pathKey("C:\\Users\\me\\AppData\\Roaming\\Claude\\scratch-workspaces\\a\\b\\scratch-1"))?.id, "zone:claude-scratch");
  assert.equal(scratchZone(pathKey("\\\\?\\C:\\Users\\me\\Documents\\Codex\\2026-10-01\\scratch-task"))?.id, "zone:codex-projectless");
  assert.equal(scratchZone(pathKey("C:\\Users\\me\\Projects\\x")), null);
});

test("satellite projects orbit the project they're named after", () => {
  const ps = [project("ledger"), project("ledger-local-supabase"), project("agent-wiki")];
  const { parentOf } = projectRelations(ps);
  assert.equal(parentOf.get(ps[1].key), ps[0].key);
  assert.equal(parentOf.has(ps[2].key), false);
});

test("sessions attach to the deepest matching project, else a zone", () => {
  const g = buildGraph({
    projectsDir: "C:\\Users\\me\\Projects",
    homeDir: "C:\\Users\\me",
    projects: [project("ledger")],
    sessions: [
      session("codex:a", "\\\\?\\C:\\Users\\me\\Projects\\ledger\\web"),
      session("claude:b", "C:\\Users\\me"),
      session("codex:c", "C:\\Users\\me\\Documents\\Codex\\2026-10-01\\x"),
      session("codex:d", "C:\\Users\\me\\Documents\\Codex\\2026-10-02\\y"),
    ],
    now,
    windows,
  });
  const parent = (id) => g.links.find((l) => l.target === id).source;
  assert.equal(parent("codex:a"), `project:${pathKey("C:\\Users\\me\\Projects\\ledger")}`);
  assert.equal(parent("claude:b"), "zone:home");
  assert.equal(parent("codex:c"), "zone:codex-projectless");
  assert.equal(parent("codex:d"), "zone:codex-projectless");
  assert.equal(g.nodes.find((n) => n.id === "zone:codex-projectless").sessionCount, 2);
});

test("a folder in your home folder is no Temp zone, even when home sits in a temp folder", () => {
  const home = "/var/folders/ab/xyz/T/xenonet-demo-1/home";
  const g = buildGraph({
    projectsDir: `${home}/Clients`,
    homeDir: home,
    projects: [],
    sessions: [session("claude:a", `${home}/Projects/atlas`), session("claude:b", "/var/folders/ab/xyz/T/run-1")],
    now,
    windows,
  });
  const parent = (id) => g.links.find((l) => l.target === id).source;
  assert.equal(parent("claude:a"), `zone:${pathKey(`${home}/Projects/atlas`)}`);
  assert.equal(g.nodes.find((n) => n.id === parent("claude:a")).name, "atlas");
  assert.equal(parent("claude:b"), "zone:temp");
});

test("the core is named for the projects folder in use", () => {
  const name = (projectsDir, homeDir) => buildGraph({ projectsDir, homeDir, projects: [], sessions: [], now, windows }).nodes.find((n) => n.id === "core").name;
  assert.equal(name("/home/me/Projects", "/home/me"), "~/Projects");
  assert.equal(name("/home/me/code/work", "/home/me/"), "~/code/work");
  assert.equal(name("C:\\Users\\me\\src\\work", "C:\\Users\\me"), "~/src/work");
  assert.equal(name("/Volumes/Data/code", "/home/me"), "/Volumes/Data/code");
  assert.equal(name("C:\\Users\\me\\Projects", null), "~/Projects", "no home folder: as before");
});

test("spawned sessions nest under their parent only when they share a home", () => {
  const g = buildGraph({
    projectsDir: "C:\\Users\\me\\Projects",
    projects: [project("wiki")],
    sessions: [
      session("claude:p", "C:\\Users\\me\\Projects\\wiki"),
      session("claude:c1", "C:\\Users\\me\\Projects\\wiki", { parentId: "claude:p" }),
      session("claude:s", "C:\\Users\\me\\AppData\\Roaming\\Claude\\scratch-workspaces\\a\\b\\s"),
      session("claude:c2", "C:\\Users\\me\\Projects\\wiki", { parentId: "claude:s" }),
    ],
    now,
    windows,
  });
  const has = (s, t, kind) => g.links.some((l) => l.source === s && l.target === t && l.kind === kind);
  assert.ok(has("claude:p", "claude:c1", "lineage"));
  assert.ok(has(`project:${pathKey("C:\\Users\\me\\Projects\\wiki")}`, "claude:c2", "session"));
  assert.ok(has("claude:s", "claude:c2", "lineage-cross"));
});

test("archived sessions are never shown as live", () => {
  const g = buildGraph({
    projectsDir: "C:\\Users\\me\\Projects",
    projects: [],
    sessions: [session("codex:x", null, { updatedAt: now - 1000, archived: true }), session("codex:y", null, { updatedAt: now - 1000 })],
    now,
    windows,
  });
  assert.equal(g.nodes.find((n) => n.id === "codex:x").activity, "recent");
  assert.equal(g.nodes.find((n) => n.id === "codex:y").activity, "active");
  assert.equal(activityOf(now - 2 * 24 * 3600e3, now, windows), "idle");
});

test("paths under home show as ~, and only whole folder names match", async () => {
  const { tilde } = await import("../core/paths.js");
  const { setHome, tildePath } = await import("../src/format.js");
  const r = String.raw;
  assert.equal(tilde(r`Read C:\Users\me\Projects\a.js`, r`C:\Users\me`), r`Read ~\Projects\a.js`);
  assert.equal(tilde("cd c:/users/me/x && ls", r`C:\Users\me`), "cd ~/x && ls");
  assert.equal(tilde(r`C:\Users\meg\x`, r`C:\Users\me`), r`C:\Users\meg\x`);
  assert.equal(tilde("/home/me/src", "/home/me/"), "~/src");
  setHome(r`C:\Users\me`);
  assert.equal(tildePath(r`\\?\C:\Users\me\Projects\a`), r`~\Projects\a`);
  assert.equal(tildePath(r`C:\Users\meg`), r`C:\Users\meg`);
  setHome(null);
});

test("a project is active while one of its sessions ran within the window, whatever its folder did", () => {
  const g = buildGraph({
    projectsDir: "C:\\Users\\me\\Projects",
    projects: [project("busy"), project("quiet", { modifiedAt: now - 60e3 }), project("empty")],
    sessions: [
      session("claude:a", "C:\\Users\\me\\Projects\\busy", { updatedAt: now - 20 * 60e3 }),
      session("claude:b", "C:\\Users\\me\\Projects\\quiet", { updatedAt: now - 3 * 3600e3 }),
    ],
    now,
    windows: { ...windows, projectActiveMs: 60 * 60e3 },
  });
  const active = (name) => g.nodes.find((n) => n.id === `project:${pathKey(`C:\\Users\\me\\Projects\\${name}`)}`).active;
  assert.equal(active("busy"), true);
  assert.equal(active("quiet"), false, "a folder changing doesn't make it active");
  assert.equal(active("empty"), false);
});

test("a session's project is the deepest one holding its folder, never one that only shares a prefix", () => {
  const root = "/home/me/Projects";
  const p = (rel, extra = {}) => ({ name: rel.split("/").pop(), path: `${root}/${rel}`, key: pathKey(`${root}/${rel}`), rel, role: "project", ...extra });
  const projects = [p("labs", { role: "group" }), p("labs/geofence", { parent: pathKey(`${root}/labs`) }), p("labs/geofence/tools"), p("ledger"), p("ledger-local")];
  const at = (id, rel) => session(id, `${root}/${rel}`);
  const g = buildGraph({
    projectsDir: root,
    homeDir: "/home/me",
    projects,
    sessions: [at("claude:tools", "labs/geofence/tools/bin"), at("claude:geo", "labs/geofence/src"), at("claude:labs", "labs/notes"), at("claude:local", "ledger-local/db"), at("claude:ledger", "ledger"), at("claude:loose", "anywhere")],
    nativeProjects: [{ id: "np", name: "Tools", roots: [`${root}/labs/geofence/tools/cli`] }],
    now,
    windows,
  });
  const parent = (id) => g.links.find((l) => l.target === id).source;
  assert.equal(parent("claude:tools"), `project:${pathKey(`${root}/labs/geofence/tools`)}`);
  assert.equal(parent("claude:geo"), `project:${pathKey(`${root}/labs/geofence`)}`);
  assert.equal(parent("claude:labs"), `project:${pathKey(`${root}/labs`)}`);
  assert.equal(parent("claude:local"), `project:${pathKey(`${root}/ledger-local`)}`);
  assert.equal(parent("claude:ledger"), `project:${pathKey(`${root}/ledger`)}`);
  assert.equal(parent("native:codex:np"), `project:${pathKey(`${root}/labs/geofence/tools`)}`);
  assert.equal(parent("claude:loose"), "zone:/home/me/projects/anywhere");
});

test("a shared system shows as its name; a uuid shows as the name it says it is, or its first characters", () => {
  const uuid = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";
  const other = "a9d3f0b2-6e41-4c57-8b12-77c0e5a49d36";
  const g = buildGraph({
    projectsDir: "/home/me/Projects",
    projects: [],
    sessions: [],
    resources: [
      { server: "firefox", ts: 1 },
      { server: uuid, ts: 2, name: "Tunebox", nameSource: "results" },
      { server: other, ts: 3 },
    ],
    now,
    windows,
  });
  const node = (server) => g.nodes.find((n) => n.id === `resource:${server}`);
  assert.deepEqual(node("firefox"), { ...node("firefox"), name: "firefox", lastActivity: 1 });
  assert.equal("server" in node("firefox"), false, "a name someone gave needs no second field");
  assert.equal(node(uuid).name, "Tunebox");
  assert.equal(node(uuid).server, uuid, "the id stays findable");
  assert.equal(node(uuid).nameSource, "results");
  assert.equal(node(other).name, "Connector a9d3f0b2");
  assert.equal(node(other).server, other);
  assert.equal("nameSource" in node(other), false);
  assert.ok(g.links.some((l) => l.target === `resource:${uuid}` && l.kind === "resource"), "beams still land on the id");
});
