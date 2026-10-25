import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { parseStatus } from "../core/git.js";
import { parseFrontmatter, parseLog, readWiki, mentionedProjects } from "../core/wiki.js";
import { TranscriptSearch, ftsQuery } from "../core/search.js";
import { summarize, handoffBrief } from "../core/digest.js";
import { buildGraph } from "../core/graph.js";
import { pathKey } from "../core/paths.js";

// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-test-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

test("git status: branch, ahead/behind and change counts", () => {
  const s = parseStatus("## main...origin/main [ahead 2, behind 1]\n M a.js\nM  b.js\n?? c.js\n");
  assert.deepEqual([s.branch, s.ahead, s.behind, s.staged, s.modified, s.untracked, s.dirty], ["main", 2, 1, 1, 1, 1, 3]);
  assert.equal(parseStatus("## No commits yet on main\n").branch, "main");
});

test("wiki: frontmatter, log entries, project mentions", async () => {
  const { meta, body } = parseFrontmatter('---\ntitle: "Ledger env"\ntags: ["ledger","x"]\n---\nSee [[home-vpn]].');
  assert.equal(meta.title, "Ledger env");
  assert.deepEqual(meta.tags, ["ledger", "x"]);
  assert.match(body, /home-vpn/);
  const log = parseLog("## 13:27 · codex · Recovered Ledger\n\npages: [[ledger]], [[windows]]\n\n- 13:53 · curator · Page updated: Agent Wiki [[agent-wiki]]\n", "2026-10-02");
  assert.deepEqual(log.map((e) => [e.app, e.pages]), [["codex", ["ledger", "windows"]], ["curator", ["agent-wiki"]]]);
  const projects = [{ name: "ledger", key: "k1" }, { name: "xenonet", key: "k2" }];
  assert.deepEqual(mentionedProjects("lives in C:\\Users\\me\\Projects\\xenonet", [], projects), ["k2"]);

  const dir = tmp();
  fs.mkdirSync(path.join(dir, "pages"));
  fs.mkdirSync(path.join(dir, "inbox"));
  fs.writeFileSync(path.join(dir, "pages", "ledger.md"), '---\ntitle: "Ledger"\ntype: project\ntags: ["ledger"]\n---\nRelated: [[xenonet]]');
  fs.writeFileSync(path.join(dir, "pages", "xenonet.md"), "---\ntitle: XenoNet\n---\nBody");
  fs.writeFileSync(path.join(dir, "inbox", "n1.md"), "---\ntitle: Pending note\napp: codex\n---\n");
  const wiki = await readWiki(dir, projects);
  assert.equal(wiki.pages.length, 2);
  assert.deepEqual(wiki.pages.find((p) => p.slug === "ledger").projects, ["k1"]);
  assert.deepEqual(wiki.inbox.map((n) => n.title), ["Pending note"]);
  // Paths under the projects folder in use, by its own name.
  fs.writeFileSync(path.join(dir, "pages", "notes.md"), "---\ntitle: Notes\n---\nThe code is in ~/Clients/xenonet, as before.");
  const mentions = async (opts) => (await readWiki(dir, projects, opts)).pages.find((p) => p.slug === "notes").projects;
  assert.deepEqual(await mentions({ root: "/Users/me/Clients" }), ["k2"]);
  assert.deepEqual(await mentions({}), [], "not a path into ~/Projects");
  fs.rmSync(path.join(dir, "pages", "notes.md"));

  const g = buildGraph({ projectsDir: "C:\\p", projects: [{ name: "ledger", key: "k1", path: "C:\\p\\ledger" }], sessions: [], wiki, layout: { "wiki:ledger": { x: 1, y: 2, z: 3 } }, windows: { activeWindowMs: 1, recentWindowMs: 1 } });
  assert.ok(g.nodes.some((n) => n.id === "resource:agent-wiki" && n.kind === "wiki" && n.inbox.length === 1));
  const page = g.nodes.find((n) => n.id === "wiki:ledger");
  assert.deepEqual([page.fx, page.fy, page.fz, page.pinned], [1, 2, 3, true]);
  assert.ok(g.relations.some((r) => r.kind === "wikilink"));
  assert.ok(g.relations.some((r) => r.kind === "mentions" && r.b === "project:k1"));
});

test("transcript search: indexes Claude, Codex and compressed archives, incrementally", async () => {
  const dir = tmp();
  const claude = path.join(dir, "claude");
  const codex = path.join(dir, "codex");
  fs.mkdirSync(path.join(claude, "proj"), { recursive: true });
  fs.mkdirSync(codex, { recursive: true });
  const cid = "11111111-1111-1111-1111-111111111111";
  const xid = "22222222-2222-2222-2222-222222222222";
  const zid = "33333333-3333-3333-3333-333333333333";
  const cfile = path.join(claude, "proj", `${cid}.jsonl`);
  fs.writeFileSync(
    cfile,
    [
      { type: "user", timestamp: "2026-10-02T10:00:00Z", message: { content: "Should we use SendGrid for customer email?" } },
      // Neither is anyone's message.
      { type: "user", timestamp: "2026-10-02T10:00:10Z", isCompactSummary: true, message: { content: "This session is being continued from a previous conversation." } },
      { type: "user", timestamp: "2026-10-02T10:00:20Z", message: { content: "[Request interrupted by user]" } },
    ].map((l) => JSON.stringify(l) + "\n").join(""),
  );
  fs.writeFileSync(path.join(codex, `rollout-2026-10-02T10-00-00-${xid}.jsonl`), JSON.stringify({ type: "event_msg", timestamp: "2026-10-02T10:00:00Z", payload: { type: "item_completed", item: { type: "AgentMessage", content: [{ type: "text", text: "Configured the Postgres replica." }] } } }) + "\n");
  fs.writeFileSync(path.join(codex, `rollout-2026-09-01T10-00-00-${zid}.jsonl.zst`), zlib.zstdCompressSync(Buffer.from(JSON.stringify({ type: "response_item", timestamp: "2026-09-01T10:00:00Z", payload: { type: "message", role: "assistant", content: [{ type: "output_text", text: "Archived note about SendGrid webhooks." }] } }) + "\n")));
  const s = new TranscriptSearch({ dbFile: path.join(dir, "search.sqlite"), claudeRoot: claude, codexRoots: [codex] });
  await s.index();
  assert.deepEqual(s.search("sendgrid").map((r) => r.sessionId).sort(), [`claude:${cid}`, `codex:${zid}`]);
  assert.match(s.search("postgres replica")[0].snippets[0].text, /⟦Postgres⟧/);
  assert.deepEqual([s.search("continued previous conversation"), s.search("interrupted")], [[], []]);
  fs.appendFileSync(cfile, JSON.stringify({ type: "assistant", timestamp: "2026-10-02T10:01:00Z", message: { content: [{ type: "text", text: "Mailgun is the alternative." }] } }) + "\n");
  await s.index();
  assert.equal(s.search("mailgun")[0].sessionId, `claude:${cid}`);
  assert.equal(s.search("sendgrid").length, 2, "re-indexing doesn't duplicate");
  assert.equal(ftsQuery("a-b c"), '"a"* AND "b"* AND "c"*');
  s.close();
});


test("transcript search: a Codex rollout's prompts are its user_message events while it has no completed user message", async () => {
  const dir = tmp();
  const codex = path.join(dir, "codex");
  fs.mkdirSync(codex, { recursive: true });
  const line = (min, payload) => JSON.stringify({ type: "event_msg", timestamp: `2026-10-02T10:0${min}:00Z`, payload }) + "\n";
  const event = (min, message) => line(min, { type: "user_message", message });
  const item = (min, text) => line(min, { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } });
  // codex exec writes only events.
  const exec = path.join(codex, "rollout-2026-10-02T10-00-00-44444444-4444-4444-4444-444444444444.jsonl");
  fs.writeFileSync(exec, event(0, "Summarize the flaky Kafka consumer tests") + line(1, { type: "agent_message", message: "Three of them time out." }));
  // A newer one says it as a completed user message, and an event that
  // isn't one more.
  const newer = path.join(codex, "rollout-2026-10-02T10-00-00-55555555-5555-5555-5555-555555555555.jsonl");
  fs.writeFileSync(newer, event(0, "Tune the Grafana dashboard") + item(0, "Tune the Grafana dashboard") + event(1, "an event alone about Prometheus"));
  const s = new TranscriptSearch({ dbFile: path.join(dir, "search.sqlite"), codexRoots: [codex] });
  await s.index();
  assert.deepEqual(s.search("kafka consumer").map((r) => [r.sessionId, r.snippets[0].role]), [["codex:44444444-4444-4444-4444-444444444444", "user"]]);
  assert.deepEqual([s.search("grafana")[0].hits, s.search("prometheus")], [1, []]);

  // One indexed by its events that has a completed user message now is
  // indexed again by those alone.
  fs.appendFileSync(exec, event(2, "Retry the Zookeeper connection") + item(2, "Retry the Zookeeper connection"));
  await s.index();
  assert.deepEqual([s.search("kafka"), s.search("zookeeper")[0].hits, s.search("them time out").length], [[], 1, 1], "what the agent said stays");
  s.close();
});

test("transcript search: an index from before Codex's user_message events counted is made again for Codex's rollouts", async () => {
  const dir = tmp();
  const claude = path.join(dir, "claude");
  const codex = path.join(dir, "codex");
  fs.mkdirSync(path.join(claude, "proj"), { recursive: true });
  fs.mkdirSync(codex, { recursive: true });
  const cfile = path.join(claude, "proj", "66666666-6666-6666-6666-666666666666.jsonl");
  fs.writeFileSync(cfile, JSON.stringify({ type: "user", timestamp: "2026-10-02T10:00:00Z", message: { content: "Plan the Redis upgrade" } }) + "\n");
  const exec = path.join(codex, "rollout-2026-10-02T10-00-00-77777777-7777-7777-7777-777777777777.jsonl");
  fs.writeFileSync(exec, JSON.stringify({ type: "event_msg", timestamp: "2026-10-02T10:00:00Z", payload: { type: "user_message", message: "Audit the Terraform modules" } }) + "\n");
  // The index as an older XenoNet left it: both files indexed, the rollout
  // with nothing found in it, and no version.
  const dbFile = path.join(dir, "search.sqlite");
  const { DatabaseSync } = await import("node:sqlite");
  const db = new DatabaseSync(dbFile);
  db.exec(`create virtual table docs using fts5(text, session_id unindexed, app unindexed, role unindexed, ts unindexed, file unindexed, tokenize = 'porter unicode61 remove_diacritics 2');
    create table files(path text primary key, size integer, mtime integer, offset integer);`);
  const put = db.prepare("insert into files(path, size, mtime, offset) values (?, ?, ?, ?)");
  for (const f of [cfile, exec]) put.run(f, fs.statSync(f).size, Math.floor(fs.statSync(f).mtimeMs), fs.statSync(f).size);
  db.prepare("insert into docs(text, session_id, app, role, ts, file) values (?, ?, ?, ?, ?, ?)").run("Plan the Redis upgrade", "claude:66666666-6666-6666-6666-666666666666", "claude", "user", Date.parse("2026-10-02T10:00:00Z"), cfile);
  db.close();

  const s = new TranscriptSearch({ dbFile, claudeRoot: claude, codexRoots: [codex] });
  const st = await s.index();
  assert.equal(s.search("terraform")[0].sessionId, "codex:77777777-7777-7777-7777-777777777777");
  assert.equal(s.search("redis").length, 1, "Claude Code's transcripts stay as they were");
  assert.equal(st.docs, 2);
  assert.equal(s.db.prepare("pragma user_version").get().user_version, 2);
  s.close();
});

test("transcript search: a Grok Build message is one record, which grows as its chunks come, read after read", async () => {
  const dir = tmp();
  const grok = path.join(dir, "grok");
  const file = path.join(grok, "%2Fatlas", "019a3b5c-0000-7000-8000-00000000000a", "updates.jsonl");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const chunk = (kind, text) => JSON.stringify({ timestamp: 1791201600, method: "session/update", params: { update: { sessionUpdate: kind, content: { type: "text", text } } } }) + "\n";
  fs.writeFileSync(file, chunk("user_message_chunk", "Add refunds ") + chunk("user_message_chunk", "to the API.") + chunk("agent_message_chunk", "Refunds"));
  const dbFile = path.join(dir, "search.sqlite");
  // An index as an older XenoNet left it, with the first chunks alone.
  const { DatabaseSync } = await import("node:sqlite");
  const db = new DatabaseSync(dbFile);
  db.exec(`create virtual table docs using fts5(text, session_id unindexed, app unindexed, role unindexed, ts unindexed, file unindexed, tokenize = 'porter unicode61 remove_diacritics 2');
    create table files(path text primary key, size integer, mtime integer, offset integer, items integer not null default 0);
    pragma user_version = 1;`);
  db.prepare("insert into files(path, size, mtime, offset) values (?, ?, ?, ?)").run(file, fs.statSync(file).size, Math.floor(fs.statSync(file).mtimeMs), fs.statSync(file).size);
  db.prepare("insert into docs(text, session_id, app, role, ts, file) values (?, ?, ?, ?, ?, ?)").run("Add refunds ", "grok:019a3b5c-0000-7000-8000-00000000000a", "grok", "user", 1791201600000, file);
  db.close();

  const s = new TranscriptSearch({ dbFile, grokRoot: grok });
  await s.index();
  const texts = () => s.db.prepare("select text from docs order by rowid").all().map((r) => r.text);
  assert.deepEqual(texts(), ["Add refunds to the API.", "Refunds"], "made again, each message whole");
  assert.equal(s.db.prepare("pragma user_version").get().user_version, 2);
  // The reply goes on in the next read, from where the last one left it.
  fs.appendFileSync(file, chunk("agent_message_chunk", " work now.") + chunk("agent_message_chunk", " Tests pass."));
  await s.index();
  assert.deepEqual(texts(), ["Add refunds to the API.", "Refunds work now. Tests pass."]);
  assert.deepEqual(s.search("tests pass").map((r) => r.snippets[0].role), ["assistant"]);
  s.close();
});
test("digest summarizes per session; handoff brief carries goal, state and files", () => {
  const events = [
    { sessionId: "s1", app: "claude", kind: "prompt", ts: 10 },
    { sessionId: "s1", app: "claude", kind: "tool", category: "edit", files: ["C:\\r\\a.js"], ts: 11 },
    { sessionId: "s1", app: "claude", kind: "tool-error", ts: 12 },
    { sessionId: "s1", app: "claude", kind: "turn-end", ts: 13 },
    { sessionId: "s2", app: "codex", kind: "tool", category: "shell", ts: 14 },
    { sessionId: "s2", app: "codex", kind: "tool", category: "shell", ts: 1 },
  ];
  const d = summarize(events, { since: 5, until: 20, titleOf: (id) => id.toUpperCase() });
  assert.deepEqual(d.totals, { sessions: 2, turns: 1, toolCalls: 2, files: 1, errors: 1, subagents: 0 });
  assert.equal(d.sessions[0].title, "S1");
  assert.deepEqual(d.sessions[0].files, ["a.js"]);

  const brief = handoffBrief({
    node: { app: "codex", title: "Build local Ledger", cwd: "C:\\Projects\\ledger", branch: "feature/seed" },
    notes: { firstPrompt: { text: "Set up Ledger locally" }, prompts: [{ text: "now seed data" }], messages: [{ text: "Seeded 40 rows. Want me to add tests?" }], files: ["C:/Projects/ledger/seed.mjs"], edits: [], needsYou: { kind: "question", text: "Want me to add tests?" } },
    target: "claude",
  });
  for (const part of ["a Codex session","Build local Ledger", "branch feature/seed", "Original goal: Set up Ledger locally", "Most recent request: now seed data", "Seeded 40 rows", "- C:/Projects/ledger/seed.mjs", "waiting on this question"]) assert.ok(brief.includes(part), part);
  // Each app's session by its name.
  for (const [app, name] of [["claude", "a Claude Code session"], ["grok", "a Grok Build session"]]) assert.match(handoffBrief({ node: { app, title: "x" }, notes: {}, target: "codex" }), new RegExp(`from ${name} titled`));
});

test("palette fuzzy match ranks substrings and word starts first", async () => {
  const { fuzzy } = await import("../src/palette.js");
  assert.ok(fuzzy("led", "ledger") > fuzzy("led", "build local ledger"));
  assert.ok(fuzzy("blam", "Build local Ledger with mock") > 0, "subsequence");
  assert.equal(fuzzy("zzz", "ledger"), 0);
  assert.equal(fuzzy("", "anything"), 1);
});

test("store persists layout and acks", async () => {
  process.env.XENONET_STATE_DIR = tmp();
  const store = await import(`../core/store.js?${Date.now()}`);
  store.setLayout({ a: { x: 1, y: 2, z: 3 }, b: { x: NaN, y: 0, z: 0 } });
  assert.deepEqual(store.getLayout(), { a: { x: 1, y: 2, z: 3 } });
  store.ack("s1", 42);
  store.flush();
  const saved = JSON.parse(fs.readFileSync(path.join(process.env.XENONET_STATE_DIR, "state.json"), "utf8"));
  assert.equal(saved.acks.s1, 42);
  assert.equal(pathKey("C:\\X"), "c:/x");
});

test("app dirs: XDG when set, else the platform's standard place, never a dot dir in ~", async () => {
  const { appDir } = await import("../core/config.js");
  const home = os.homedir();
  assert.equal(appDir("state", { env: { XDG_STATE_HOME: "/x/state" }, platform: "win32" }), path.join("/x/state", "xenonet"));
  assert.equal(appDir("cache", { env: { XDG_CACHE_HOME: "/x/cache" }, platform: "linux" }), path.join("/x/cache", "xenonet"));
  assert.equal(appDir("state", { env: { LOCALAPPDATA: "L" }, platform: "win32" }), path.join("L", "XenoNet"));
  assert.equal(appDir("cache", { env: { LOCALAPPDATA: "L" }, platform: "win32" }), path.join("L", "XenoNet", "Cache"));
  assert.equal(appDir("cache", { env: {}, platform: "darwin" }), path.join(home, "Library", "Caches", "XenoNet"));
  assert.equal(appDir("state", { env: {}, platform: "linux" }), path.join(home, ".local/state", "xenonet"));
  // A copy running from source keeps its own.
  assert.equal(appDir("state", { env: {}, platform: "darwin", name: "XenoNet Dev" }), path.join(home, "Library", "Application Support", "XenoNet Dev"));
  assert.equal(appDir("cache", { env: { XDG_CACHE_HOME: "/x/cache" }, platform: "linux", name: "XenoNet Dev" }), path.join("/x/cache", "xenonet-dev"));
  for (const platform of ["win32", "darwin", "linux"]) {
    for (const kind of ["state", "cache"]) {
      const first = path.relative(home, appDir(kind, { env: { LOCALAPPDATA: path.join(home, "AppData", "Local") }, platform })).split(/[\\/]/)[0];
      assert.ok(![".xenonet", "XenoNet"].includes(first), `${platform} ${kind} → ${first}`);
    }
  }
});
