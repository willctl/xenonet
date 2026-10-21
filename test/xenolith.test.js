// Xenolith on XenoNet's map: XenoNet finds the shared context's folder the
// way Xenolith's own programs do (falling back to Agent Wiki's), names the
// wiki after the server that owns it, and beams a session's context_* writes
// to the pages they name.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { contextName, contextServerOf, findContextDir, isContextServer } from "../core/xenolith.js";
import { readWiki } from "../core/wiki.js";
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
  assert.equal(GRAPH_NODES.wiki.safeParse({ ...root, actions: [], id: "resource:github" }).success, false, "only the context servers' ids");
  const old = buildGraph({ projectsDir: "/p", projects: [], sessions: [], wiki: await readWiki(wikiAt(tmp()), []), layout: {}, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  assert.equal(old.nodes.find((n) => n.kind === "wiki").id, "resource:agent-wiki", "Agent Wiki's map is as it was");
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
