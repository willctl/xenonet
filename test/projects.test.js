import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { scanProjects, projectRelations, limiter } from "../core/projects.js";
import { Scanner } from "../core/blocks/scanner.js";
import { pathKey } from "../core/paths.js";
import { buildGraph } from "../core/graph.js";
import { outlineFolders, fingerprint, picksPrompt, parsePicks } from "../core/classify.js";
import { mentionedProjects } from "../core/wiki.js";
import { scopeGraph } from "../src/scope.js";

// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-test-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

// Writes a made-up folder tree: "a/b.txt": "text" is a file, "a/c/": null a
// folder, and "a/": "repo" a git repo with HEAD and an origin.
function tree(spec) {
  const root = tmp();
  for (const [rel, v] of Object.entries(spec)) {
    const p = path.join(root, rel);
    if (v === "repo") {
      fs.mkdirSync(path.join(p, ".git"), { recursive: true });
      fs.writeFileSync(path.join(p, ".git", "HEAD"), "ref: refs/heads/main\n");
      fs.writeFileSync(path.join(p, ".git", "config"), `[remote "origin"]\n\turl = git@example.com:me/${path.basename(p)}.git\n`);
    } else if (v === null) fs.mkdirSync(p, { recursive: true });
    else {
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, v);
    }
  }
  return root;
}

const shape = (ps) => Object.fromEntries(ps.map((p) => [p.rel, `${p.role}:${p.detected}${p.parent ? ` in ${ps.find((q) => q.key === p.parent).rel}` : ""}`]));

test("a repo inside a plain folder is a project, and the folder a group", async () => {
  const root = tree({ "studio/NOTES.md": "notes", "studio/app/": "repo", "studio/app/package.json": "{}", "ledger/": "repo" });
  const ps = await scanProjects(root);
  assert.deepEqual(shape(ps), { ledger: "project:repo", studio: "group:contains", "studio/app": "project:repo in studio" });
  assert.equal(ps.find((p) => p.rel === "studio/app").git.origin, "git@example.com:me/app.git");
  assert.deepEqual(ps.scan.groups, 1);
});

test("a top-level folder with no repo in it stays one project, sessions in its subfolders or not", async () => {
  const root = tree({ "notes/README.md": "# Notes", "notes/drafts/a.md": "", "notes/site/package.json": "{}" });
  const ps = await scanProjects(root, { sessionDirs: [path.join(root, "notes", "drafts")] });
  assert.deepEqual(shape(ps), { notes: "project:folder" });
});

test("a monorepo is one project, its packages and workspaces inside it are not", async () => {
  const root = tree({ "mono/": "repo", "mono/package.json": '{"workspaces":["packages/*"]}', "mono/packages/a/package.json": "{}", "mono/packages/b/Cargo.toml": "[workspace]\n", "mono/apps/web/package.json": "{}" });
  const ps = await scanProjects(root, { sessionDirs: [path.join(root, "mono", "packages", "a")] });
  assert.deepEqual(shape(ps), { mono: "project:repo" });
});

test("a workspace without a repo is a project, even inside a group", async () => {
  const root = tree({
    "clients/": null,
    "clients/site/": "repo",
    "clients/shop/pnpm-workspace.yaml": "packages: ['*']\n",
    "clients/shop/web/package.json": "{}",
    "clients/shop/api/go.work": "go 1.22\n",
    "clients/shop/api/main/go.mod": "",
    "clients/tools/Cargo.toml": '[package]\nname = "tools"\n',
    "tooling/pyproject.toml": "[tool.uv.workspace]\nmembers = ['a']\n",
    "tooling/a/pyproject.toml": "",
  });
  const ps = await scanProjects(root);
  // shop's go.work is one of its packages; tools is a plain crate, not a workspace.
  assert.deepEqual(shape(ps), { clients: "group:contains", "clients/shop": "project:workspace in clients", "clients/site": "project:repo in clients", tooling: "project:workspace" });
});

test("a grab-bag folder: its repos and the folders sessions ran in are projects, the rest aren't", async () => {
  const root = tree({
    "misc/pyproject.toml": "[project]\nname='misc'\n\n[tool.uv.workspace]\nmembers = ['tool-a', 'tool-b']\n",
    "misc/notebook.ipynb": "{}",
    "misc/tool-a/": "repo",
    "misc/tool-a/pyproject.toml": "",
    "misc/tool-b/": "repo",
    "misc/examples/swift/Package.swift": "",
    "misc/fastapi_demo/pyproject.toml": "",
    "misc/scratch/sub/x.py": "",
    "misc/payloads/": null,
  });
  const ps = await scanProjects(root, { sessionDirs: [path.join(root, "misc"), path.join(root, "misc", "fastapi_demo"), path.join(root, "misc", "scratch", "sub")] });
  assert.deepEqual(shape(ps), {
    misc: "group:contains",
    "misc/fastapi_demo": "project:sessions in misc",
    "misc/scratch": "project:sessions in misc",
    "misc/tool-a": "project:repo in misc",
    "misc/tool-b": "project:repo in misc",
  });
  assert.deepEqual(ps.find((p) => p.rel === "misc").stack, ["python"]);
});

test("worktrees: one beside its repo orbits it; one inside its repo belongs to it", async () => {
  const root = tree({ "app/": "repo", "app/.claude/worktrees/fix-1/src/a.js": "", "app-fix/README.md": "fix" });
  // git worktree add: the checkout's .git file points into the main repo's
  // .git/worktrees, whose commondir points back at the main .git.
  const linked = (dir, name) => {
    const gitDir = path.join(root, "app", ".git", "worktrees", name);
    fs.mkdirSync(gitDir, { recursive: true });
    fs.writeFileSync(path.join(gitDir, "HEAD"), `ref: refs/heads/${name}\n`);
    fs.writeFileSync(path.join(gitDir, "commondir"), "../..\n");
    fs.writeFileSync(path.join(dir, ".git"), `gitdir: ${gitDir}\n`);
  };
  linked(path.join(root, "app", ".claude", "worktrees", "fix-1"), "fix-1");
  linked(path.join(root, "app-fix"), "app-fix");
  const ps = await scanProjects(root, { sessionDirs: [path.join(root, "app", ".claude", "worktrees", "fix-1", "src")] });
  assert.deepEqual(shape(ps), { app: "project:repo", "app-fix": "project:repo" });
  const wt = ps.find((p) => p.rel === "app-fix");
  assert.equal(wt.git.branch, "app-fix");
  assert.equal(wt.git.origin, "git@example.com:me/app.git", "the remote comes from the main repo's config");
  assert.equal(pathKey(wt.git.worktreeOf), pathKey(path.join(root, "app")));
  assert.equal(projectRelations(ps).parentOf.get(wt.key), ps.find((p) => p.rel === "app").key);
});

test("submodules are projects inside their superproject, up to a limit", async () => {
  const root = tree({ "game/": "repo", "game/.gitmodules": '[submodule "engine"]\n\tpath = libs/engine\n[submodule "art"]\n\tpath = art\n[submodule "gone"]\n\tpath = gone\n' });
  for (const [rel, name] of [["libs/engine", "engine"], ["art", "art"]]) {
    const gitDir = path.join(root, "game", ".git", "modules", name);
    fs.mkdirSync(gitDir, { recursive: true });
    fs.writeFileSync(path.join(gitDir, "HEAD"), "ref: refs/heads/main\n");
    fs.writeFileSync(path.join(gitDir, "config"), `[remote "origin"]\n\turl = https://example.com/${name}.git\n`);
    fs.mkdirSync(path.join(root, "game", rel), { recursive: true });
    fs.writeFileSync(path.join(root, "game", rel, ".git"), `gitdir: ${path.relative(path.join(root, "game", rel), gitDir)}\n`);
  }
  const ps = await scanProjects(root);
  assert.deepEqual(shape(ps), { game: "project:repo", "game/art": "project:submodule in game", "game/libs/engine": "project:submodule in game" });
  assert.equal(ps.find((p) => p.rel === "game/art").git.origin, "https://example.com/art.git");
  assert.equal(pathKey(ps.find((p) => p.rel === "game/art").git.submoduleOf), pathKey(path.join(root, "game")));
  assert.equal((await scanProjects(root, { limits: { maxSubmodules: 1 } })).length, 2);
});

test("repos below the depth limit show up once a session runs in them, without the steps in between", async () => {
  const root = tree({ "hosts/github.com/me/tool/": "repo", "hosts/github.com/me/other/": "repo", "deep/a/b/c/repo/": "repo", "deep/a/b/c/repo/src/x.js": "" });
  assert.deepEqual(shape(await scanProjects(root, { limits: { depth: 3 } })), { deep: "project:folder", hosts: "project:folder" });
  const ps = await scanProjects(root, { limits: { depth: 4 }, sessionDirs: [path.join(root, "deep", "a", "b", "c", "repo", "src")] });
  assert.deepEqual(shape(ps), {
    deep: "group:contains",
    "deep/a/b/c/repo": "project:repo in deep",
    hosts: "group:contains",
    "hosts/github.com/me": "group:contains in hosts",
    "hosts/github.com/me/other": "project:repo in hosts/github.com/me",
    "hosts/github.com/me/tool": "project:repo in hosts/github.com/me",
  });
});

test("the scan stops at its folder budget, shallow levels first, and skips dependencies and build output", async () => {
  const root = tree({ "a/": "repo", "b/": "repo", "c/x/": "repo", "d/node_modules/pkg/": "repo", "d/dist/": "repo", "d/.cache/": "repo" });
  const full = await scanProjects(root);
  assert.deepEqual(shape(full), { a: "project:repo", b: "project:repo", c: "group:contains", "c/x": "project:repo in c", d: "project:folder" });
  assert.equal(full.scan.truncated, false);
  const cut = await scanProjects(root, { limits: { maxDirs: 4 } });
  assert.deepEqual(shape(cut), { a: "project:repo", b: "project:repo", c: "project:folder", d: "project:folder" });
  assert.equal(cut.scan.truncated, true);
  assert.equal(cut.scan.dirsRead, 4);
});

test("a budget that ends inside a level reads its first folders, and the next level begins with the first folder's own", async () => {
  // Which folders come first is the file system's order, so this counts:
  // five plain folders, each holding two repos.
  const spec = {};
  for (const p of ["p1", "p2", "p3", "p4", "p5"]) for (const r of ["r1", "r2"]) spec[`${p}/${r}/`] = "repo";
  const root = tree(spec);
  const inFirst = await scanProjects(root, { limits: { maxDirs: 3 } });
  assert.deepEqual([inFirst.scan.dirsRead, inFirst.scan.truncated, inFirst.length, inFirst.filter((p) => p.detected === "folder").length], [3, true, 3, 3]);
  // All five, then two of the second level: the first folder's two repos,
  // which make it a group.
  const inSecond = await scanProjects(root, { limits: { maxDirs: 7 } });
  assert.deepEqual([inSecond.scan.dirsRead, inSecond.scan.truncated, inSecond.scan.groups, inSecond.scan.projects], [7, true, 1, 6]);
  const group = inSecond.find((p) => p.role === "group");
  assert.deepEqual(inSecond.filter((p) => p.parent === group.key).map((p) => p.name).sort(), ["r1", "r2"]);
});

test("a folder the scan can't read is passed over, and counts against the budget all the same", { skip: process.platform === "win32" || process.getuid?.() === 0 }, async () => {
  const root = tree({ "locked/inner/": "repo", "open/": "repo" });
  fs.chmodSync(path.join(root, "locked"), 0o000);
  try {
    const ps = await scanProjects(root);
    assert.deepEqual(shape(ps), { open: "project:repo" });
    assert.deepEqual([ps.scan.ok, ps.scan.dirsRead, ps.scan.truncated], [true, 2, false]);
  } finally {
    fs.chmodSync(path.join(root, "locked"), 0o755);
  }
});

test("a scan no longer wanted stops: it reads nothing more, and rejects", async () => {
  const spec = {};
  for (let i = 0; i < 12; i++) spec[`p${i}/r/`] = "repo";
  const root = tree(spec);
  // A file system that says when the scan has read three folders below the root.
  const stop = new AbortController();
  let reads = 0;
  const watched = { ...fs, promises: { ...fs.promises, readdir: (...args) => (++reads === 4 && stop.abort(), fs.promises.readdir(...args)) } };
  const scanner = new Scanner({ name: "projects", concurrency: 1, cache: false, pick: () => false, fs: watched });
  await assert.rejects(scanProjects(root, { scanner, signal: stop.signal }), { name: "AbortError" });
  assert.deepEqual([scanner.last.read, scanner.last.truncated], [3, "cancelled"]);
  assert.equal(reads, 4, "no folder read after it");
  // Aborted before it starts: it reads nothing below the root.
  await assert.rejects(scanProjects(root, { signal: AbortSignal.abort() }), { name: "AbortError" });
});

test("rules: pin a folder as one project, look inside a repo, or leave a folder out", async () => {
  const root = tree({ "studio/app/": "repo", "studio/web/": "repo", "lab/": "repo", "lab/one/": null, "lab/two/": "repo", "old/": "repo", "deep/x/y/z/": null });
  const rule = (rel, as) => [pathKey(path.join(root, rel)), { path: path.join(root, rel), as }];
  const rules = Object.fromEntries([rule("studio", "project"), rule("lab", "group"), rule("old", "ignore"), rule("deep/x/y/z", "project")]);
  const ps = await scanProjects(root, { rules, sessionDirs: [path.join(root, "lab", "one"), path.join(root, "old"), path.join(root, "studio", "app")] });
  assert.deepEqual(shape(ps), {
    deep: "group:contains",
    "deep/x/y/z": "project:rule in deep",
    lab: "group:rule",
    "lab/one": "project:sessions in lab",
    "lab/two": "project:repo in lab",
    studio: "project:rule",
  });
  assert.ok(ps.find((p) => p.rel === "lab").git, "a repo made a group keeps its repo status");
});

test("satellites orbit a project named after them only in the same folder", () => {
  const p = (rel, parent = null) => ({ name: rel.split("/").pop(), rel, key: `/p/${rel}`, parent, role: "project", git: null });
  const ps = [p("ledger"), p("ledger-db"), p("misc", null), p("misc/ledger-cli", "/p/misc")];
  ps[2].role = "group";
  const { parentOf } = projectRelations(ps);
  assert.equal(parentOf.get("/p/ledger-db"), "/p/ledger");
  assert.equal(parentOf.has("/p/misc/ledger-cli"), false);
});

test("on the map, a group holds its projects and the sessions run in the folder itself", async () => {
  const root = tree({ "studio/NOTES.md": "", "studio/app/": "repo", "studio/app/src/": null, "ledger/": "repo" });
  const now = Date.parse("2026-10-02T12:00:00Z");
  const session = (id, cwd, ago) => ({ id, app: "claude", rawId: id, title: id, cwd: path.join(root, ...cwd), updatedAt: now - ago });
  const sessions = [session("claude:a", ["studio"], 5 * 86400e3), session("claude:b", ["studio", "app", "src"], 60e3), session("claude:c", ["ledger"], 20 * 86400e3)];
  const projects = await scanProjects(root, { sessionDirs: sessions.map((s) => s.cwd) });
  for (const p of projects) p.modifiedAt = now - 60 * 86400e3; // the folders were just made
  const g = buildGraph({ projectsDir: root, projects, sessions, now, windows: { activeWindowMs: 10 * 60e3, recentWindowMs: 24 * 3600e3, projectActiveMs: 3600e3 } });
  const id = (rel) => `project:${pathKey(path.join(root, rel))}`;
  const parent = (target) => g.links.find((l) => l.target === target);
  assert.deepEqual([parent(id("studio")).source, parent(id("studio")).kind], ["core", "contains"]);
  assert.deepEqual([parent(id("studio/app")).source, parent(id("studio/app")).kind], [id("studio"), "nested"]);
  assert.equal(parent("claude:a").source, id("studio"));
  assert.equal(parent("claude:b").source, id("studio/app"));
  const node = (rel) => g.nodes.find((n) => n.id === id(rel));
  assert.equal(node("studio").group, true);
  assert.equal(node("studio").sessionCount, 2, "a group counts its projects' sessions");
  assert.equal(node("studio").active, true, "and lights while one of them is active");
  assert.deepEqual([g.stats.projects, g.stats.groups], [2, 1]);

  // In the 7-day scope the group comes along with its project, and isn't
  // counted as a project itself; without anything in it drawn, it goes.
  const week = scopeGraph(g, { window: "7d", background: false }, { now });
  assert.ok(week.nodes.some((n) => n.id === id("studio")));
  assert.deepEqual(week.scope.projects, { shown: 1, total: 2 });
  const quiet = scopeGraph({ ...g, nodes: g.nodes.filter((n) => n.id !== "claude:b" && n.id !== "claude:a").map((n) => (n.id === id("studio/app") ? { ...n, lastActivity: now - 30 * 86400e3 } : n)) }, { window: "7d", background: false }, { now });
  assert.ok(!quiet.nodes.some((n) => n.id === id("studio")));
});

test("wiki pages mentioning a path into a group name the project, not the group", () => {
  const ps = [
    { name: "studio", rel: "studio", key: "/p/studio" },
    { name: "app", rel: "studio/app", key: "/p/studio/app" },
    { name: "ledger", rel: "ledger", key: "/p/ledger" },
  ];
  assert.deepEqual(mentionedProjects("Lives in ~/Projects/studio/app/src", [], ps), ["/p/studio/app"]);
  assert.deepEqual(mentionedProjects("See Projects/studio for the lot, and Projects/ledger-old", [], ps), ["/p/studio"]);
});

test("wiki pages mention projects under the projects folder's own name", () => {
  const ps = [
    { name: "studio", rel: "studio", key: "/p/studio" },
    { name: "app", rel: "studio/app", key: "/p/studio/app" },
    { name: "ledger", rel: "ledger", key: "/p/ledger" },
  ];
  const code = "/Users/ada/code";
  assert.deepEqual(mentionedProjects("Lives in ~/code/studio/app/src", [], ps, code), ["/p/studio/app"]);
  assert.deepEqual(mentionedProjects("Lives in ~/Projects/studio/app", [], ps, code), [], "another folder's path");
  assert.deepEqual(mentionedProjects("unicode/ledger and barcode/studio", [], ps, code), [], "the name starts a path or a word");
  assert.deepEqual(mentionedProjects("C:\\Users\\ada\\Code\\ledger\\README.md", [], ps, "C:\\Users\\ada\\Code\\"), ["/p/ledger"]);
  assert.deepEqual(mentionedProjects("Tagged only", ["ledger"], ps, code), ["/p/ledger"], "tags don't need a path");
  // A sentence can end on the path; a longer name can't.
  assert.deepEqual(mentionedProjects("The guide is in ~/code/ledger.", [], ps, code), ["/p/ledger"]);
  assert.deepEqual(mentionedProjects("Moved to ~/code/ledger.old and ~/code/ledger.js", [], ps, code), []);
  // A folder whose name has regex characters in it is taken as it is.
  assert.deepEqual(mentionedProjects("~/work (old)/ledger, not ~/work old/ledger", [], ps, "/Users/ada/work (old)"), ["/p/ledger"]);
  assert.deepEqual(mentionedProjects("~/work old/ledger", [], ps, "/Users/ada/work (old)"), []);
});

test("the Codex pass gets names and well-known files only, and only picks that change something", async () => {
  const root = tree({
    "misc/pyproject.toml": "",
    "misc/tool/": "repo",
    "misc/examples/secret-client-name.py": "print('hi')",
    "misc/examples/notes.txt": "private",
    "misc/charts/package.json": '{"description":"private"}',
    "misc/charts/index.html": "",
  });
  const scan = await scanProjects(root, { sessionDirs: [path.join(root, "misc", "charts")] });
  const outline = outlineFolders(scan.folders);
  assert.deepEqual(outline.map((f) => [f.path, f.xenonet]), [["misc", "group"], ["misc/charts", "project"], ["misc/examples", "folder"], ["misc/tool", "repo"]]);
  assert.deepEqual(outline.find((f) => f.path === "misc/tool"), { path: "misc/tool", xenonet: "repo", markers: [], files: null, folders: null, types: {}, sessions: 0 });
  assert.deepEqual(outline.find((f) => f.path === "misc/examples"), { path: "misc/examples", xenonet: "folder", markers: [], files: 2, folders: 0, types: { py: 1, txt: 1 }, sessions: 0 });
  assert.equal(outline.find((f) => f.path === "misc").sessions, 1);
  const prompt = picksPrompt(outline);
  for (const secret of [root, path.basename(root), "secret-client-name", "notes.txt", "private", "print("]) assert.ok(!prompt.includes(secret), `the prompt leaks ${secret}`);

  // The fingerprint follows folders and their well-known files, not counts.
  const more = outline.map((f) => ({ ...f, files: f.files + 3, sessions: 9 }));
  assert.equal(fingerprint(more), fingerprint(outline));
  assert.notEqual(fingerprint(outline.slice(1)), fingerprint(outline));

  const answer = JSON.stringify({
    picks: [
      { path: "misc/examples", as: "project", reason: "Sample code for one tool." },
      { path: "misc/examples", as: "group", reason: "A second answer for the same folder." },
      { path: "misc/tool", as: "group", reason: "Repos aren't Codex's to change." },
      { path: "misc/charts", as: "project", reason: "Already a project." },
      { path: "../elsewhere", as: "project", reason: "Not a folder it was sent." },
      { path: "misc", as: "ignore", reason: "Not an allowed answer." },
    ],
  });
  assert.deepEqual(parsePicks("```json\n" + answer + "\n```", outline), [{ path: "misc/examples", as: "project", reason: "Sample code for one tool." }]);
  assert.throws(() => parsePicks("I think misc is a group.", outline), /expected shape/);
});

test("a folder you made one project keeps Codex's picks inside it out, but not your own", async () => {
  const root = tree({ "misc/tool/": "repo", "misc/examples/": null, "misc/charts/": null });
  const rule = (rel, as, by) => [pathKey(path.join(root, rel)), { path: path.join(root, rel), as, by }];
  const rules = Object.fromEntries([rule("misc", "project", "you"), rule("misc/examples", "project", "codex"), rule("misc/charts", "project", "you")]);
  assert.deepEqual(shape(await scanProjects(root, { rules })), { misc: "project:rule", "misc/charts": "project:rule in misc" });
});

test("Codex's picks and your rules feed the scan, yours winning", async () => {
  const root = tree({ "misc/tool/": "repo", "misc/examples/": null, "misc/charts/package.json": "{}", "notes/a/": null });
  const rule = (rel, as, by) => [pathKey(path.join(root, rel)), { path: path.join(root, rel), as, by }];
  const rules = Object.fromEntries([rule("misc/examples", "project", "codex"), rule("misc/charts", "project", "codex"), rule("notes", "group", "codex"), rule("misc/charts", "ignore", "you")]);
  assert.deepEqual(shape(await scanProjects(root, { rules })), {
    misc: "group:contains",
    "misc/examples": "project:rule in misc",
    "misc/tool": "project:repo in misc",
    notes: "group:rule",
  });
});

test("limiter runs at most n at a time, in the order asked", async () => {
  const run = limiter(2);
  let now = 0;
  let most = 0;
  const started = [];
  const task = (id) => run(async () => {
    started.push(id);
    most = Math.max(most, ++now);
    await new Promise((r) => setTimeout(r, 5));
    now--;
    return id;
  });
  const done = await Promise.all([1, 2, 3, 4, 5].map(task));
  assert.deepEqual(done, [1, 2, 3, 4, 5]);
  assert.deepEqual(started, [1, 2, 3, 4, 5]);
  assert.equal(most, 2);
});

test("limiter frees its place when a function throws", async () => {
  const run = limiter(1);
  await assert.rejects(run(async () => { throw new Error("no"); }), /no/);
  assert.equal(await run(async () => "next"), "next");
});
