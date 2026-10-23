// Xenolith on XenoNet's map: XenoNet finds the shared context's folder the
// way Xenolith's own programs do (falling back to Agent Wiki's), names the
// shared context after the server that owns it, and beams a session's
// context_* writes to the pages they name.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { CONTEXT_FORMAT, CONTEXT_TOOLS, contextName, contextServerOf, findContextDir, formatProblem, isContextCall, isContextServer, MARKER_FILE, readMarker, SPACE_KINDS, spaceOf } from "../core/xenolith.js";
import { parseLog, readWiki, wikiLinks } from "../core/wiki.js";
import { buildGraph } from "../core/graph.js";
import { GRAPH_NODES } from "../core/schema/graph.js";
import { LiveTelemetry, mcpServerOf } from "../core/live.js";

const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-xenolith-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

const wikiAt = (dir, protocol) => {
  fs.mkdirSync(path.join(dir, "pages"), { recursive: true });
  if (protocol) fs.writeFileSync(path.join(dir, "PROTOCOL.md"), protocol);
  return dir;
};

test("the shared context is Xenolith's where Xenolith is, else Agent Wiki's", () => {
  const home = tmp();
  const find = (env = {}, platform = "darwin") => findContextDir({ env, home, platform });
  assert.equal(find(), null, "neither is installed");

  wikiAt(path.join(home, "AgentWiki"));
  assert.deepEqual(find(), { dir: path.join(home, "AgentWiki"), server: "agent-wiki" });

  // Agent Wiki 1.3 and later keep config.json in the platform's folder.
  const awWiki = wikiAt(path.join(home, "Notes", "aw"));
  fs.mkdirSync(path.join(home, "Library", "Application Support", "Agent Wiki"), { recursive: true });
  fs.writeFileSync(path.join(home, "Library", "Application Support", "Agent Wiki", "config.json"), JSON.stringify({ wikiDir: awWiki }));
  assert.deepEqual(find(), { dir: awWiki, server: "agent-wiki" });

  // Xenolith wins wherever it is: its default folder, then its config.
  wikiAt(path.join(home, "Xenolith"));
  assert.deepEqual(find(), { dir: path.join(home, "Xenolith"), server: "xenolith" });
  const xWiki = wikiAt(path.join(home, "Notes", "x"));
  fs.mkdirSync(path.join(home, "Library", "Application Support", "Xenolith"), { recursive: true });
  fs.writeFileSync(path.join(home, "Library", "Application Support", "Xenolith", "config.json"), `﻿${JSON.stringify({ wikiDir: xWiki })}`);
  assert.deepEqual(find(), { dir: xWiki, server: "xenolith" });
  assert.deepEqual(find({ XENOLITH_DIR: awWiki }), { dir: awWiki, server: "xenolith" }, "XENOLITH_DIR first");

  // The same lookups on Linux (XDG) and Windows (APPDATA), and portable mode.
  const linux = tmp();
  const lWiki = wikiAt(path.join(linux, "w"));
  fs.mkdirSync(path.join(linux, "cfg", "xenolith"), { recursive: true });
  fs.writeFileSync(path.join(linux, "cfg", "xenolith", "config.json"), JSON.stringify({ wikiDir: lWiki }));
  assert.equal(findContextDir({ env: { XDG_CONFIG_HOME: path.join(linux, "cfg") }, home: linux, platform: "linux" })?.dir, lWiki);
  // (A folder of its own: macOS folders don't tell xenolith from Xenolith.)
  assert.equal(findContextDir({ env: { APPDATA: path.join(linux, "roaming") }, home: linux, platform: "win32" }), null, "Windows looks in APPDATA\\Xenolith");
  fs.mkdirSync(path.join(linux, "roaming", "Xenolith"), { recursive: true });
  fs.writeFileSync(path.join(linux, "roaming", "Xenolith", "config.json"), JSON.stringify({ wikiDir: lWiki }));
  assert.equal(findContextDir({ env: { APPDATA: path.join(linux, "roaming") }, home: linux, platform: "win32" })?.dir, lWiki);
  assert.equal(findContextDir({ env: { XENOLITH_HOME: path.join(linux, "cfg", "xenolith") }, home: path.join(linux, "nohome"), platform: "darwin" })?.dir, lWiki);

  // A config.json that names a folder that is gone is passed over.
  const old = tmp();
  fs.mkdirSync(path.join(old, ".agent-wiki"));
  fs.writeFileSync(path.join(old, ".agent-wiki", "config.json"), JSON.stringify({ wikiDir: path.join(old, "gone") }));
  assert.equal(findContextDir({ env: {}, home: old, platform: "darwin" }), null);
  fs.mkdirSync(path.join(old, "kept"));
  fs.writeFileSync(path.join(old, ".agent-wiki", "config.json"), JSON.stringify({ wikiDir: path.join(old, "kept") }));
  assert.deepEqual(findContextDir({ env: {}, home: old, platform: "darwin" }), { dir: path.join(old, "kept"), server: "agent-wiki" }, "the pre-1.3 config");
});

test("Xenolith's config is found where Xenolith's own programs look", () => {
  const home = tmp();
  const find = (env = {}) => findContextDir({ env, home, platform: "linux" });
  const config = (dir, wikiDir) => {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, "config.json"), JSON.stringify({ wikiDir }));
  };

  // XENOLITH_DIR counts before Xenolith has made the folder.
  const later = path.join(home, "not-yet");
  assert.deepEqual(find({ XENOLITH_DIR: later }), { dir: later, server: "xenolith" });
  assert.equal(fs.existsSync(later), false, "XenoNet only reads");
  assert.equal(findContextDir({ env: { AGENT_WIKI_DIR: later }, home, platform: "linux" }), null, "Agent Wiki's must be there");

  // The config from before the platform's folder, ~/.xenolith/config.json,
  // until one is in the platform's folder.
  const legacyWiki = wikiAt(path.join(home, "legacy"));
  config(path.join(home, ".xenolith"), legacyWiki);
  assert.deepEqual(find(), { dir: legacyWiki, server: "xenolith" });
  const portable = tmp();
  assert.equal(find({ XENOLITH_HOME: portable }), null, "never in portable mode");
  assert.equal(find({ XENOLITH_CONFIG_DIR: "" }), null, "nor with XENOLITH_CONFIG_DIR set at all");
  const newWiki = wikiAt(path.join(home, "new"));
  config(path.join(home, ".config", "xenolith"), newWiki);
  assert.deepEqual(find(), { dir: newWiki, server: "xenolith" });

  // XENOLITH_CONFIG_DIR wins over XENOLITH_HOME, as a folder of its own
  // does over portable mode.
  const cfgWiki = wikiAt(path.join(home, "cfg-wiki"));
  config(path.join(home, "cfg"), cfgWiki);
  config(portable, legacyWiki);
  assert.equal(find({ XENOLITH_HOME: portable })?.dir, legacyWiki);
  assert.equal(find({ XENOLITH_HOME: portable, XENOLITH_CONFIG_DIR: path.join(home, "cfg") })?.dir, cfgWiki);
});

test("a wiki folder belongs to the server whose lookup found it, and to its protocol only as a last resort", () => {
  const home = tmp();
  const of = (dir, env = {}) => contextServerOf(dir, { env, home, platform: "linux" });
  assert.equal(of(wikiAt(tmp(), "# Xenolith protocol\n\nThis folder is...")), "xenolith");
  assert.equal(of(wikiAt(tmp(), "# Agent Wiki protocol\n")), "agent-wiki");
  assert.equal(of(wikiAt(tmp())), "agent-wiki", "no protocol: as XenoNet always read it");
  assert.equal(of(null), null);
  assert.deepEqual([contextName("xenolith"), contextName("agent-wiki")], ["Xenolith", "Agent Wiki"]);

  // A wiki moved somewhere Xenolith's config names, its protocol edited:
  // still Xenolith's, so its writes don't make a second node.
  const moved = wikiAt(path.join(home, "Notes", "wiki"), "# Our notes\n\nRules for this folder.");
  assert.equal(of(moved), "agent-wiki");
  fs.mkdirSync(path.join(home, ".config", "xenolith"), { recursive: true });
  fs.writeFileSync(path.join(home, ".config", "xenolith", "config.json"), JSON.stringify({ wikiDir: moved }));
  assert.equal(of(moved), "xenolith");
  assert.deepEqual(findContextDir({ env: {}, home, platform: "linux" }), { dir: moved, server: "xenolith" });
  assert.equal(of(`${moved}/`), "xenolith", "however the folder is spelled");
  // XENOLITH_DIR names it, there or not; and Agent Wiki's own folder is
  // Agent Wiki's, whatever its protocol says.
  const aside = wikiAt(tmp(), "# Notes\n");
  assert.equal(of(aside, { XENOLITH_DIR: aside }), "xenolith");
  assert.equal(of(wikiAt(path.join(home, "AgentWiki"), "# Xenolith protocol\n")), "agent-wiki");
  if (process.platform !== "win32") {
    const link = path.join(tmp(), "link");
    fs.symlinkSync(moved, link);
    assert.equal(of(link), "xenolith", "a link to it");
  }
});

test("the folder's marker names its server, before the lookup's and the protocol's", () => {
  const home = tmp();
  const wiki = wikiAt(path.join(home, "Xenolith"));
  const mark = (text) => fs.writeFileSync(path.join(wiki, MARKER_FILE), text);
  const find = () => findContextDir({ env: {}, home, platform: "linux" });
  const of = (dir) => contextServerOf(dir, { env: {}, home, platform: "linux" });
  assert.equal(MARKER_FILE, "xenolith.json");
  assert.deepEqual([readMarker(wiki), find()], [null, { dir: wiki, server: "xenolith" }], "no marker: the lookup's");

  mark('﻿{\n  "format": 1,\n  "server": "ctx"\n}\n');
  assert.deepEqual(readMarker(wiki), { format: 1, server: "ctx" });
  assert.deepEqual([find(), of(wiki)], [{ dir: wiki, server: "ctx" }, "ctx"], "the marker's, before the lookup's");
  assert.equal(contextName("ctx"), "Xenolith");

  // A folder no lookup names, its protocol Agent Wiki's: the marker
  // decides, and without one the protocol does.
  const aside = wikiAt(tmp(), "# Agent Wiki protocol\n");
  assert.equal(of(aside), "agent-wiki");
  fs.writeFileSync(path.join(aside, MARKER_FILE), '{"format":1,"server":"xenolith"}');
  assert.equal(of(aside), "xenolith");

  // A server that isn't a name: the lookup's. The format still counts.
  mark('{"format":2,"server":"my context"}');
  assert.deepEqual(readMarker(wiki), { format: 2, server: null });
  assert.deepEqual(find(), { dir: wiki, server: "xenolith" });

  // Markers XenoNet can't take: as if there were none.
  for (const bad of ["{", "[]", "null", '{"server":"ctx"}', '{"format":"1","server":"ctx"}', '{"format":0,"server":"ctx"}', '{"format":1.5,"server":"ctx"}', '{"format":9007199254740992,"server":"ctx"}']) {
    mark(bad);
    assert.deepEqual([readMarker(wiki), of(wiki)], [null, "xenolith"], bad);
  }
  fs.rmSync(path.join(wiki, MARKER_FILE));
  fs.mkdirSync(path.join(wiki, MARKER_FILE));
  assert.equal(readMarker(wiki), null, "a folder");
  fs.rmSync(path.join(wiki, MARKER_FILE), { recursive: true });
  if (process.platform !== "win32") {
    const elsewhere = path.join(tmp(), "elsewhere.json");
    fs.writeFileSync(elsewhere, '{"format":1,"server":"ctx"}');
    fs.symlinkSync(elsewhere, path.join(wiki, MARKER_FILE));
    assert.equal(readMarker(wiki), null, "a link");
  }
});

test("a folder in a later format keeps its node, shows nothing in it and says why", async () => {
  assert.equal(CONTEXT_FORMAT, 1);
  assert.equal(formatProblem(null), null);
  assert.equal(formatProblem({ format: 1, server: null }), null);
  assert.match(formatProblem({ format: 2, server: null }), /version 2 of Xenolith's format.*reads version 1/);
  assert.deepEqual([contextName("xenolith"), contextName("agent-wiki"), contextName(""), contextName(null)], ["Xenolith", "Agent Wiki", "Agent Wiki", "Agent Wiki"]);

  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  fs.writeFileSync(path.join(dir, "pages", "atlas.md"), "# Atlas\n");
  fs.mkdirSync(path.join(dir, "inbox"));
  fs.writeFileSync(path.join(dir, "inbox", "note.md"), "# A note\n");
  const graphOf = (wiki) => buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki, layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });

  fs.writeFileSync(path.join(dir, MARKER_FILE), '{"format":1,"server":"xenolith"}');
  const now = await readWiki(dir, [], { server: "xenolith" });
  assert.deepEqual([now.pages.length, now.inbox.length, "problem" in now], [1, 1, false], "the format it reads");
  assert.equal("problem" in graphOf(now).nodes.find((n) => n.kind === "wiki"), false, "a node it reads is as it was");

  fs.writeFileSync(path.join(dir, MARKER_FILE), '{"format":2,"server":"xenolith"}');
  const later = await readWiki(dir, [], { server: "xenolith" });
  assert.deepEqual(Object.keys(later), ["dir", "server", "pages", "inbox", "log", "problem"]);
  assert.deepEqual([later.pages, later.inbox, later.log, later.server], [[], [], [], "xenolith"]);
  assert.match(later.problem, /version 2/);
  const g = graphOf(later);
  const root = g.nodes.find((n) => n.kind === "wiki");
  assert.deepEqual([root.id, root.pageCount, root.problem], ["resource:xenolith", 0, later.problem]);
  assert.equal(g.nodes.filter((n) => n.kind === "page").length, 0);
  const parsed = GRAPH_NODES.wiki.safeParse({ ...root, actions: [] });
  assert.equal(parsed.success, true, JSON.stringify(parsed.error?.issues));

  fs.writeFileSync(path.join(dir, MARKER_FILE), '{"format":');
  const broken = await readWiki(dir, []);
  assert.deepEqual([broken.pages.length, "problem" in broken], [1, false], "a marker it can't take: read as before");
});

test("the server a folder's marker names takes the map's node and its writes", async () => {
  const dir = wikiAt(tmp());
  fs.writeFileSync(path.join(dir, "pages", "refunds.md"), "# Refunds\n");
  fs.writeFileSync(path.join(dir, MARKER_FILE), '{"format":1,"server":"team-ctx"}');
  const server = contextServerOf(dir, { env: {}, home: tmp() });
  assert.equal(server, "team-ctx");
  const g = buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki: await readWiki(dir, [], { server }), resources: [{ server, ts: 1 }], layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  const root = g.nodes.find((n) => n.kind === "wiki");
  assert.deepEqual([root.id, root.name, root.server], ["resource:team-ctx", "Xenolith", "team-ctx"]);
  assert.equal(g.nodes.filter((n) => n.id === "resource:team-ctx").length, 1, "the server's calls don't make a second node");
  const parsed = GRAPH_NODES.wiki.safeParse({ ...root, actions: [] });
  assert.equal(parsed.success, true, JSON.stringify(parsed.error?.issues));

  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null, contextServer: server });
  assert.deepEqual(live.wikiPagesOf("team-ctx", { pages: ["refunds"] }, "context_log"), ["refunds"]);
  assert.deepEqual(live.wikiPagesOf("xenolith", { pages: ["refunds"] }, "context_log"), ["refunds"], "the known names still count");
  assert.equal(live.wikiPagesOf("other", { pages: ["refunds"] }, "context_log"), null);
});

test("the map's wiki takes its server's id and name, so beams to xenolith land on it", async () => {
  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  fs.writeFileSync(path.join(dir, "pages", "atlas.md"), "---\ntitle: Atlas\ntype: project\n---\nSee [[db]].");
  const wiki = await readWiki(dir, [], { server: contextServerOf(dir, { env: {}, home: tmp() }) });
  const g = buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki, resources: [{ server: "xenolith", ts: 1 }], layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  const root = g.nodes.find((n) => n.kind === "wiki");
  assert.deepEqual([root.id, root.name, root.server], ["resource:xenolith", "Xenolith", "xenolith"]);
  assert.equal(g.nodes.filter((n) => n.id === "resource:xenolith").length, 1, "the server's calls don't make a second node");
  // (The state adds the node's actions before it goes out.)
  const parsed = GRAPH_NODES.wiki.safeParse({ ...root, actions: [] });
  assert.equal(parsed.success, true, JSON.stringify(parsed.error?.issues));
  assert.equal(GRAPH_NODES.wiki.safeParse({ ...root, actions: [], id: "resource:not a name" }).success, false, "only a server's name in its id");
  const old = buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki: await readWiki(wikiAt(tmp()), []), layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  assert.equal(old.nodes.find((n) => n.kind === "wiki").id, "resource:agent-wiki", "Agent Wiki's map is as it was");
});

test("pages read as Xenolith reads them: [[slug|text]] links, titles from headings, types in lowercase, aliases", async () => {
  assert.deepEqual(wikiLinks("See [[atlas|the Atlas API]], [[DB]] and [[db|again]]; not [[x|a]b]] or [[y|z]."), ["atlas", "db"]);
  assert.deepEqual(parseLog("## 13:27 · codex · Filed\n\npages: [[atlas|Atlas]], [[refunds]]\n", "2026-10-02")[0].pages, ["atlas", "refunds"]);
  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  const page = (slug, text) => fs.writeFileSync(path.join(dir, "pages", `${slug}.md`), text);
  page("atlas", '---\ntitle: Atlas\ntype: Project\naliases: ["Atlas API", "#Orders", "atlas api"]\n---\n# Not the title\n\nSee [[db|the database]].');
  page("db", "---\ntype: HowTo\naliases: Postgres, #pg ,\n---\n\n#\tThe database \n\nText.");
  page("bare", "No heading here.\n#hashtag\n");
  const wiki = await readWiki(dir, []);
  const by = Object.fromEntries(wiki.pages.map((p) => [p.slug, p]));
  assert.deepEqual([by.atlas.title, by.atlas.type, by.atlas.aliases, by.atlas.links], ["Atlas", "project", ["atlas api", "orders"], ["db"]]);
  assert.deepEqual([by.db.title, by.db.type, by.db.aliases], ["The database", "howto", ["postgres", "pg"]]);
  assert.deepEqual([by.bare.title, by.bare.type, by.bare.aliases], ["bare", "topic", []], "no heading: the slug");
  const g = buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki, layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  const node = (id) => g.nodes.find((n) => n.id === id);
  assert.deepEqual(node("wiki:atlas").aliases, ["atlas api", "orders"]);
  assert.equal("aliases" in node("wiki:bare"), false, "left out when there are none");
  assert.ok(g.relations.some((r) => r.kind === "wikilink" && r.a === "wiki:atlas" && r.b === "wiki:db"));
  const parsed = GRAPH_NODES.page.safeParse({ ...node("wiki:atlas"), actions: [] });
  assert.equal(parsed.success, true, JSON.stringify(parsed.error?.issues));
});

test("pages, logs and notes linked in from elsewhere are passed over, as Xenolith refuses them", { skip: process.platform === "win32" && "links need privileges on Windows" }, async () => {
  const outside = tmp();
  fs.writeFileSync(path.join(outside, "secret.md"), "---\ntitle: Not the wiki's\n---\n");
  fs.mkdirSync(path.join(outside, "2026"));
  fs.writeFileSync(path.join(outside, "2026", "2026-10-03.md"), "## 09:00 · codex · Elsewhere\n\npages: [[atlas]]\n");
  const now = new Date(2026, 9, 3, 12).getTime();
  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  fs.writeFileSync(path.join(dir, "pages", "atlas.md"), "---\ntitle: Atlas\n---\n");
  fs.symlinkSync(path.join(outside, "secret.md"), path.join(dir, "pages", "linked.md"));
  fs.mkdirSync(path.join(dir, "inbox"));
  fs.writeFileSync(path.join(dir, "inbox", "kept.md"), "# Kept\n");
  fs.symlinkSync(path.join(outside, "secret.md"), path.join(dir, "inbox", "linked.md"));
  fs.mkdirSync(path.join(dir, "log"));
  fs.symlinkSync(path.join(outside, "2026"), path.join(dir, "log", "2026"));
  const wiki = await readWiki(dir, [], { now });
  assert.deepEqual(wiki.pages.map((p) => p.slug), ["atlas"]);
  assert.deepEqual(wiki.inbox.map((n) => n.id), ["kept"]);
  assert.deepEqual(wiki.log, [], "a year of the log linked in");
  fs.rmSync(path.join(dir, "log", "2026"));
  fs.mkdirSync(path.join(dir, "log", "2026"));
  fs.symlinkSync(path.join(outside, "2026", "2026-10-03.md"), path.join(dir, "log", "2026", "2026-10-03.md"));
  assert.deepEqual((await readWiki(dir, [], { now })).log, [], "a day of it");
  fs.copyFileSync(path.join(outside, "2026", "2026-10-03.md"), path.join(dir, "log", "2026", "2026-10-02.md"));
  assert.deepEqual((await readWiki(dir, [], { now })).log.map((e) => e.title), ["Elsewhere"], "a day that's the wiki's own");

  // The pages folder linked in, and the wiki's own folder a link, which
  // Xenolith allows.
  const other = wikiAt(tmp());
  fs.writeFileSync(path.join(other, "pages", "x.md"), "# X\n");
  const linkedPages = wikiAt(tmp());
  fs.rmSync(path.join(linkedPages, "pages"), { recursive: true });
  fs.symlinkSync(path.join(other, "pages"), path.join(linkedPages, "pages"));
  assert.deepEqual((await readWiki(linkedPages, [])).pages, []);
  const root = path.join(tmp(), "wiki");
  fs.symlinkSync(other, root);
  assert.deepEqual((await readWiki(root, [])).pages.map((p) => p.slug), ["x"]);
});

test("Xenolith's spaces show as their own pages, tied to a project's space's project", async () => {
  const now = new Date(2026, 9, 3, 12).getTime();
  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  const write = (rel, text) => {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), text);
  };
  write("pages/refunds.md", "---\ntitle: Refunds here\n---\n");
  write("spaces/project/atlas/pages/refunds.md", "---\ntitle: Atlas refunds\n---\nSee [[guide|the guide]] and [[elsewhere]].");
  write("spaces/project/atlas/pages/guide.md", "# Atlas guide\n");
  write("spaces/project/atlas/log/2026/2026-10-03.md", "## 09:00 · codex · Filed the refund rules\n\npages: [[refunds]]\n");
  write("spaces/identity/adal/pages/prefs.md", "# Preferences\n");
  // Not spaces: a name Xenolith wouldn't make, this machine's own, a kind
  // it doesn't have.
  write("spaces/project/Atlas-Old/pages/x.md", "# X\n");
  write("spaces/machine/local/pages/x.md", "# X\n");
  write("spaces/notes/x/pages/x.md", "# X\n");
  if (process.platform !== "win32") {
    write("outside/pages/x.md", "# X\n");
    fs.symlinkSync(path.join(dir, "outside"), path.join(dir, "spaces", "project", "linked"));
  }
  const projects = [{ name: "Atlas", key: "/p/atlas", rel: "atlas" }, { name: "ledger", key: "/p/ledger", rel: "ledger" }];
  const wiki = await readWiki(dir, projects, { now });
  assert.deepEqual(wiki.pages.map((p) => [p.space, p.slug]), [[null, "refunds"], ["identity:adal", "prefs"], ["project:atlas", "guide"], ["project:atlas", "refunds"]], "this machine's, then by kind and name");
  const by = (space, slug) => wiki.pages.find((p) => p.space === space && p.slug === slug);
  assert.deepEqual(by("project:atlas", "refunds").writes.map((w) => w.title), ["Filed the refund rules"]);
  assert.deepEqual(by(null, "refunds").writes, [], "a write in a space isn't the same-named page's here");
  assert.deepEqual(by("project:atlas", "guide").projects, ["/p/atlas"], "a project's space is its context");
  assert.deepEqual(by("identity:adal", "prefs").projects, []);
  assert.equal(wiki.log.length, 1);

  const g = buildGraph({ projectsDir: "/p", projects: projects.map((p) => ({ ...p, path: p.key })), sessions: [], wiki, layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  const page = g.nodes.find((n) => n.id === "wiki:project:atlas/refunds");
  assert.deepEqual([page.name, page.slug, page.space], ["Atlas refunds", "refunds", "project:atlas"]);
  assert.equal("space" in g.nodes.find((n) => n.id === "wiki:refunds"), false, "this machine's pages are as they were");
  assert.ok(g.relations.some((r) => r.kind === "wikilink" && r.a === "wiki:project:atlas/refunds" && r.b === "wiki:project:atlas/guide"), "links stay in their space");
  assert.ok(g.relations.some((r) => r.kind === "mentions" && r.a === "wiki:project:atlas/guide" && r.b === "project:/p/atlas"));
  assert.ok(g.links.some((l) => l.kind === "page" && l.source === "resource:agent-wiki" && l.target === "wiki:identity:adal/prefs"), "a space's pages hang from the wiki");
  const parsed = GRAPH_NODES.page.safeParse({ ...page, actions: [] });
  assert.equal(parsed.success, true, JSON.stringify(parsed.error?.issues));
});

test("a write to a space beams to that space's pages, and one Xenolith refuses to none", () => {
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  const of = (space) => live.wikiPagesOf("xenolith", { title: "x", pages: ["refunds"], space });
  assert.deepEqual(of("project:atlas"), ["project:atlas/refunds"]);
  assert.deepEqual(of(" Project:Atlas "), ["project:atlas/refunds"], "as Xenolith reads it");
  assert.deepEqual(of("machine:local"), ["refunds"], "this machine's own");
  assert.deepEqual([of(undefined), of(""), of(5)], [["refunds"], ["refunds"], ["refunds"]], "none given: this machine's");
  for (const bad of ["atlas", "team:atlas", "project:../x", "project:a b", "project:.x", "project:a..b"]) assert.equal(of(bad), null, bad);
  assert.deepEqual(live.wikiPagesOf("xenolith", { slug: "guide", space: "identity:adal" }), ["identity:adal/guide"]);
  assert.deepEqual([spaceOf("ACTOR:hermes"), spaceOf("machine:local"), spaceOf(null), spaceOf("project:")], ["actor:hermes", null, null, undefined]);
  assert.deepEqual(SPACE_KINDS, ["project", "identity", "actor", "machine"]);
});

test("context_* writes, and the wiki_* names before them, beam to the pages they name", () => {
  assert.equal(mcpServerOf("mcp__xenolith__context_log"), "xenolith");
  assert.equal(mcpServerOf("mcp__plugin_xenolith_xenolith__context_upsert_page"), "xenolith");
  assert.ok(isContextServer("xenolith") && isContextServer("agent-wiki") && !isContextServer("github"));
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  assert.deepEqual(live.wikiPagesOf("xenolith", { title: "x", pages: ["atlas", "db"] }), ["atlas", "db"]);
  assert.deepEqual(live.wikiPagesOf("xenolith", { title: "x", slug: "atlas" }), ["atlas"]);
  assert.deepEqual(live.wikiPagesOf("agent-wiki", { pages: ["ledger"] }), ["ledger"]);
  assert.equal(live.wikiPagesOf("github", { pages: ["x"] }), null);
});

test("Xenolith's tools write to the shared context under the server the config names", () => {
  assert.deepEqual([...CONTEXT_TOOLS], ["context_start", "context_search", "context_read", "context_log", "context_upsert_page"]);
  for (const tool of CONTEXT_TOOLS) assert.ok(isContextCall("ctx", tool, "ctx"), tool);
  assert.ok(isContextCall("xenolith", "write_page", null) && isContextCall("agent-wiki", "wiki_log", "ctx"), "the known names, whatever the tool");
  assert.ok(!isContextCall("ctx", "context_log", null) && !isContextCall("ctx", "context_log", ""), "no server named");
  assert.ok(!isContextCall("ctx", "write_page", "ctx") && !isContextCall("github", "context_log", "ctx"));

  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null, contextServer: "ctx" });
  assert.deepEqual(live.wikiPagesOf("ctx", { pages: ["refunds"] }, "context_log"), ["refunds"]);
  assert.equal(live.wikiPagesOf("ctx", { pages: ["refunds"] }, "write_page"), null);
  assert.equal(live.wikiPagesOf("ctx", { pages: ["refunds"] }), null);
  assert.deepEqual(live.wikiPagesOf("xenolith", { pages: ["refunds"] }), ["refunds"]);
  const unnamed = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  assert.equal(unnamed.wikiPagesOf("ctx", { pages: ["refunds"] }, "context_log"), null);

  // Through the transcripts: a Claude Code tool call, a plugin's server
  // among them, and a Codex MCP call.
  const at = new Date(Date.now() - 60e3).toISOString();
  const claude = { sessionId: "claude:aaaaaaaa-0000-0000-0000-00000000000c" };
  const use = (id, name, input) => ({ type: "tool_use", id, name, input });
  live.applyClaude(claude, { type: "assistant", timestamp: at, message: { id: "msg_1", model: "claude-x", stop_reason: "tool_use", content: [use("tu1", "mcp__ctx__context_upsert_page", { slug: "atlas" }), use("tu2", "mcp__plugin_xenolith_ctx__context_log", { pages: ["refunds"], space: "project:atlas" }), use("tu3", "mcp__ctx__write_page", { slug: "other" })] } }, false);
  const codex = { sessionId: "codex:bbbbbbbb-0000-0000-0000-00000000000c" };
  live.applyCodex(codex, { type: "event_msg", timestamp: at, payload: { type: "task_started" } }, false);
  live.applyCodex(codex, { type: "event_msg", timestamp: at, payload: { type: "item_completed", item: { type: "McpToolCall", server: "ctx", tool: "context_log", arguments: { pages: ["ledger"] }, status: "completed" } } }, false);
  const pages = live.history.filter((e) => e.kind === "tool").map((e) => [e.tool, e.pages ?? null]);
  assert.deepEqual(pages, [
    ["mcp__ctx__context_upsert_page", ["atlas"]],
    ["mcp__plugin_xenolith_ctx__context_log", ["project:atlas/refunds"]],
    ["mcp__ctx__write_page", null],
    ["mcp__ctx__context_log", ["ledger"]],
  ]);
});
