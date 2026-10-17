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

test("a wiki folder belongs to the server its protocol names", () => {
  assert.equal(contextServerOf(wikiAt(tmp(), "# Xenolith protocol\n\nThis folder is...")), "xenolith");
  assert.equal(contextServerOf(wikiAt(tmp(), "# Agent Wiki protocol\n")), "agent-wiki");
  assert.equal(contextServerOf(wikiAt(tmp())), "agent-wiki", "no protocol: as XenoNet always read it");
  assert.equal(contextServerOf(null), null);
  assert.deepEqual([contextName("xenolith"), contextName("agent-wiki")], ["Xenolith", "Agent Wiki"]);
});

test("the map's wiki takes its server's id and name, so beams to xenolith land on it", async () => {
  const dir = wikiAt(tmp(), "# Xenolith protocol\n");
  fs.writeFileSync(path.join(dir, "pages", "atlas.md"), "---\ntitle: Atlas\ntype: project\n---\nSee [[db]].");
  const wiki = await readWiki(dir, [], { server: contextServerOf(dir) });
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
