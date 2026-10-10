// Run after a Claude desktop, ChatGPT desktop, Codex or Agent SDK update:
//   npm run contracts              the live checks, then the drift report
//   npm run contracts -- --json    the same, as JSON
//   npm run contracts -- --all     every difference XenoNet keeps, not only the new ones
//
// It reads every source XenoNet reads, your real data, the way XenoNet does.
// The live checks fail loudly when a field the UI depends on has
// disappeared, instead of letting the graph go quiet. The drift report says,
// by source, the versions this run read, the schema it read them with, how
// many records, and what differs from XenoNet's schemas that's new since you
// last looked: from this run, and from drift.json in the cache folder of
// the XenoNet it runs as (XenoNet Dev, from a checkout), which that XenoNet adds
// to as it reads (core/vendor.js, core/drift.js). What this run finds goes
// there too, for xenonet drift and that XenoNet's doctor. It doesn't mark it
// seen, since agents run this too, and the doctor's contracts.drift check
// would go quiet before you'd seen it: xenonet drift, which gives the same
// report without the live checks or npm, and the console do.
//
// It builds a search index of its own, in a temporary folder it deletes
// after, asks Anthropic for plan usage unless XENONET_CLAUDE_USAGE=off, and
// runs git config in every repo. It prints counts, versions and shapes,
// never an email, a handle or a value: this output may end up in a bug
// report.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { config } from "../core/config.js";
import { scanProjects } from "../core/projects.js";
import { listCodexThreads, listCodexNativeProjects, readCodexThread, codexClient } from "../core/codex.js";
import { findCodexBinary } from "../core/dirs.js";
import { listClaudeSessions, readClaudeSession } from "../core/claude.js";
import { buildGraph } from "../core/graph.js";
import { LiveTelemetry } from "../core/live.js";
import { fetchClaudeUsage } from "../core/claude-usage.js";
import { readWiki } from "../core/wiki.js";
import { TranscriptSearch } from "../core/search.js";
import { Transcripts } from "../core/stats.js";
import { IdentityWatch, emptySaved } from "../core/identities.js";
import { drift, DRIFT_FILE } from "../core/drift.js";
import { driftReport, reportLines } from "../core/vendor.js";
import { tilde } from "../core/paths.js";
import { applyRoot, instanceDir } from "../core/workspace.js";
import { instanceFrom } from "../core/instances.js";

const json = process.argv.includes("--json");
const all = process.argv.includes("--all");
const home = (p) => tilde(p, config.homeDir);
// The projects folder XenoNet reads: XENONET_PROJECTS_DIR, the one you chose
// (read from state.json as it is, since the file is a running XenoNet's to
// change), or ~/Projects. Run from a checkout, XenoNet is XenoNet Dev
// (core/instances.js; XENONET_INSTANCE picks another), so the choice is read
// from its state folder, where its server keeps it, and drift is kept in
// its cache folder, where xenonet drift and its doctor read it.
const instance = instanceFrom();
const own = !(config.demo || instance.demo);
applyRoot({ peek: true, stateDir: own ? instanceDir("state", instance.name) : config.stateDir });

// What this run finds joins what XenoNet found; the report is of what's new
// since you last looked.
const cacheDir = own ? instanceDir("cache", instance.name) : config.cacheDir;
drift.open(path.join(cacheDir, DRIFT_FILE));

const checks = [];
let section = null;
let failures = 0;
const heading = (name, detail) => {
  section = name;
  if (!json) console.log(`${name.padEnd(9)} (${detail})`);
};
const say = (status, message) => {
  checks.push({ section, status, message });
  if (status === "fail") failures++;
  if (!json) console.log(`  ${status === "fail" ? "FAIL" : status.padEnd(4)}  ${message}`);
};
const ok = (msg) => say("ok", msg);
const warn = (msg) => say("warn", msg);
const fail = (msg) => say("fail", msg);
const expectFields = (label, obj, fields) => {
  const missing = fields.filter((f) => obj?.[f] == null);
  missing.length ? warn(`${label}: empty fields ${missing.join(", ")}`) : ok(`${label}: fields present`);
};

// ---------- the live checks ----------

heading("projects", home(config.projectsDir));
const projects = await scanProjects(config.projectsDir);
projects.length ? ok(`${projects.scan.projects} projects in ${projects.scan.groups} group${projects.scan.groups === 1 ? "" : "s"}, ${projects.scan.dirsRead} folders read`) : warn("no project dirs");

heading("codex", home(findCodexBinary()));
let codex = [];
try {
  codex = await listCodexThreads();
  ok(`thread/list → ${codex.length} threads (Codex ${codexClient.version ?? "?"})`);
  if (codex[0]) {
    expectFields("thread", codex[0], ["rawId", "title", "cwd", "createdAt", "updatedAt"]);
    // As the console reads a thread: from its rollout when the app-server
    // can't list its items.
    const transcripts = new Transcripts({ codexRoots: [path.join(config.codexHome, "sessions"), path.join(config.codexHome, "archived_sessions")] });
    const page = await readCodexThread(codex[0].rawId, { limit: 10, rollout: async () => (await transcripts.files(codex[0].id))[0]?.file ?? null });
    const from = page.total != null ? "the thread's rollout (thread/items/list isn't answered)" : "thread/items/list";
    page.entries.length ? ok(`${from} → ${page.entries.length} items`) : warn(`${from} returned no items`);
    const unknown = page.entries.filter((e) => e.role === "system" && e.title).map((e) => e.title);
    if (unknown.length) warn(`unmapped item types: ${[...new Set(unknown)].join(", ")}`);
  }
} catch (err) {
  fail(`codex app-server: ${home(err.message)}`);
}
const native = listCodexNativeProjects();
ok(`codex native projects (state DB, optional) → ${native.length}`);

heading("claude", `desktop dir: ${config.claudeDesktopDir ? home(config.claudeDesktopDir) : "not found"}`);
let claude = [];
try {
  claude = await listClaudeSessions();
  ok(`listSessions → ${claude.length} sessions, ${claude.filter((s) => s.inDesktop).length} matched to desktop metadata`);
  if (claude[0]) {
    expectFields("session", claude[0], ["rawId", "title", "cwd", "updatedAt"]);
    const page = await readClaudeSession(claude[0].rawId, { limit: 10 });
    page.entries.length ? ok(`getSessionMessages → ${page.entries.length} entries`) : warn("getSessionMessages returned no entries");
  }
  if (claude.length && !claude.some((s) => s.inDesktop)) warn("no desktop metadata matched; Claude desktop's session file layout may have changed");
} catch (err) {
  fail(`claude agent sdk: ${home(err.message)}`);
}

heading("live", "transcript tailing");
const live = new LiveTelemetry({ claudeRoot: config.claudeProjectsDir, codexRoot: path.join(config.codexHome, "sessions"), claudeDesktopDir: config.claudeDesktopDir, claudeUsage: config.claudeUsage ? () => fetchClaudeUsage() : null });
await live.start();
const snap = live.snapshot();
live.stop();
ok(`primed ${live.status.files} transcript files, ${live.status.lines} lines, ${live.status.errors} errors`);
const withTokens = snap.sessions.filter((s) => s.tokens.processed > 0);
const withTools = snap.sessions.filter((s) => Object.keys(s.toolCounts).length);
for (const app of ["claude", "codex"]) {
  const recent = snap.sessions.filter((s) => s.app === app);
  if (!recent.length) warn(`no ${app} transcripts in the last 24h; nothing to check`);
  else if (!withTokens.some((s) => s.app === app)) warn(`${app}: transcripts found but no token usage parsed; the line format may have changed`);
  else if (!withTools.some((s) => s.app === app)) warn(`${app}: token usage parsed but no tool calls; the tool line format may have changed`);
  else ok(`${app}: tokens, context and tool calls parsed from ${recent.length} sessions`);
}
snap.limits.codex ? ok(`codex rate limits: ${snap.limits.codex.primary?.pct ?? "?"}%`) : warn("codex rate limits not seen in rollouts (optional)");
const cl = snap.limits.claude;
cl && !cl.stale ? ok(`claude plan usage from ${cl.source}: 5h ${cl.fiveHour}% · 7d ${cl.sevenDay}%`) : warn(`claude plan usage: no current reading${cl?.reason ? ` (${cl.reason})` : ""} (optional)`);

heading("wiki", config.wikiDir ? home(config.wikiDir) : "not found");
const wiki = await readWiki(config.wikiDir, projects, { root: config.projectsDir }).catch((e) => (fail(`wiki: ${home(e.message)}`), null));
if (wiki) wiki.pages.length ? ok(`${wiki.pages.length} pages, ${wiki.log.length} log entries, ${wiki.inbox.length} unfiled`) : warn("wiki folder has no pages");
else if (!config.wikiDir) warn("Agent Wiki folder not found (optional)");

// An index of its own, made afresh: XenoNet's has its server for a writer,
// and a second would trip over it. TranscriptSearch reads how far a file
// was indexed before it writes, and doesn't wait on SQLite's lock, so the
// two could each add a file's new lines (twice in the index), or fail as
// locked, the server's searches too. The folder holds transcripts' text,
// so it goes once the index is read.
heading("search", "transcript index, built in a temporary folder");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-contracts-"));
const search = new TranscriptSearch({
  dbFile: path.join(scratch, "search.sqlite"),
  claudeRoot: config.claudeProjectsDir,
  codexRoots: [path.join(config.codexHome, "sessions"), path.join(config.codexHome, "archived_sessions")],
});
try {
  const st = await search.index();
  st.docs ? ok(`${st.docs} messages indexed from ${st.files} files (${st.ms} ms)`) : warn("search index is empty");
} finally {
  search.close();
  fs.rmSync(scratch, { recursive: true, force: true });
}

const graph = buildGraph({ projectsDir: config.projectsDir, homeDir: config.homeDir, wiki, projects, sessions: [...codex, ...claude], nativeProjects: native, windows: config });
ok(`graph → ${graph.nodes.length} nodes, ${graph.links.length} links ${JSON.stringify(graph.stats)}`);

// Counts only, never an email or a handle.
heading("identities", "sign-ins, gh's accounts, git config");
if (!config.identities) warn("off (XENONET_IDENTITIES=off)");
else {
  const watch = new IdentityWatch({ home: config.homeDir, claudeConfigFile: config.claudeConfigFile, codexHome: config.codexHome, ghConfigDir: config.ghConfigDir, claudeProjectsDir: config.claudeProjectsDir });
  const acc = watch.accounts();
  acc.claude ? ok("claude: ~/.claude.json names the signed-in account") : warn("claude: no account in ~/.claude.json (signed out, or its shape changed)");
  acc.chatgpt ? ok(`codex: auth.json's ID token names the account${acc.chatgpt.since ? " and when it signed in" : ""}`) : warn("codex: no account in auth.json's ID token (signed out, an API key, or its shape changed)");
  acc.gh.length ? ok(`gh: ${acc.gh.reduce((n, h) => n + h.users.length, 0)} accounts on ${acc.gh.length} host(s)`) : warn("gh: no accounts in hosts.yml (optional)");
  if (claude.some((s) => s.inDesktop) && !claude.some((s) => s.claudeAccount)) warn("no Claude desktop session folder names an account; its layout may have changed");
  await watch.refreshRepos(projects);
  // A fresh state, so nothing is saved.
  const { identities } = watch.link(graph, emptySaved());
  const ran = graph.nodes.filter((n) => n.kind === "session" && n.ranAs);
  ok(`${identities.length} identities; ${ran.length} of ${graph.stats.sessions} sessions linked, ${ran.filter((n) => n.ranAs.confidence === "exact").length} recorded by the app`);
}

codexClient.stop();

// ---------- the drift report ----------

// The same report xenonet drift gives, with what this run read.
const report = driftReport({ log: drift, all, home: config.homeDir });
if (json) console.log(JSON.stringify({ checks, failures, drift: report }, null, 2));
else {
  console.log(`\nvendor data (read leniently against schemas/vendor; drift kept in ${report.file})`);
  for (const line of reportLines(report)) console.log(line);
}

// What this run found is kept, still new to you (see above).
drift.flush();
if (!json) console.log(failures ? `\n${failures} contract failure(s)` : "\nall contracts ok");
process.exit(failures ? 1 : 0);
