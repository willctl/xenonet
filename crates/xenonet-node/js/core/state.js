// core/state.js beside Rust (crates/xenonet-state/src/state.rs): what the
// service knows, refreshed. The class stays JS's (an EventEmitter that owns
// the blocks, the timers and the map, whose index and parents callers read
// and change), and Rust makes its decisions: which sessions a refresh maps,
// each source's status, the projects folder's rules and detection, where a
// session lives, the live snapshot as the page gets it, the tray's summary,
// and what it remembers between refreshes (pulses, the map's signature,
// sign-ins, nudges to commit).
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import { config } from "../../../../core/config.js";
import { scanProjects, forgetScans, projectsScanner } from "./projects.js";
import { outlineFolders, fingerprint, askCodexForPicks, askDeciderForPicks } from "./classify.js";
import { pathKey } from "./paths.js";
import * as workspace from "./workspace.js";
import { codexThreadsGatherer, codexProjectsGatherer, codexClient, readCodexThread, renameCodexThread } from "./codex.js";
import { findCodexBinary } from "./dirs.js";
import { claudeSessionsGatherer, readClaudeSession, renameClaudeSession } from "./claude.js";
import { grokSessionsGatherer, readGrokSession } from "../../../../core/grok.js";
import { cleanTitle, renamable } from "./rename.js";
import { buildGraph } from "./graph.js";
import { actionsFor } from "../../../../core/open.js";
import { LiveTelemetry } from "./live.js";
import { fetchClaudeUsage } from "./claude-usage.js";
import { RepoWatch } from "./git.js";
import { IdentityWatch } from "./identities.js";
import { readWiki } from "./wiki.js";
import { TranscriptSearch } from "./search.js";
import { Transcripts, createStats } from "./stats.js";
import { summarize, dayLog, summaryText, handoffBrief, narrate } from "./digest.js";
import { constellationOf, labelsOf, posterSalt } from "./redact.js";
import * as store from "./store.js";
import { Blocks } from "./blocks/lifecycle.js";
import { claudeLines, codexLines, grokLines } from "./lines.js";
import { NumbatSessions } from "./numbat.js";
import { nativeReaders } from "../../../../core/native/index.js";
import { ReportedRuns, observedSessions } from "./reported-runs.js";
import { tailedOr, untailed } from "../../../../core/apps.js";
import { demoSessions, demoNativeProjects, demoTranscript, demoNarration, demoPicks, startDriver, clientsReady } from "../../../../core/demo.js";
// The addon as `rust`: #refresh has a `native` of its own, Codex's projects.
import { native as rust } from "../index.js";

const LIVE_THROTTLE_MS = 500;
const REPO_POLL_MS = 20000;

// A gatherer's run as a source of the map: what it found, and how it went.
async function gathered(name, g) {
  const value = await g.run();
  const { ok, ms, error } = g.last;
  return { value: ok ? value : [], status: rust.sourceStatus(name, !!ok, ms, ok ? value : undefined, error) };
}

async function settle(name, fn) {
  const started = Date.now();
  try {
    const value = await fn();
    return { value, status: rust.sourceStatus(name, true, Date.now() - started, value, undefined) };
  } catch (err) {
    return { value: [], status: rust.sourceStatus(name, false, Date.now() - started, undefined, String(err?.message ?? err)) };
  }
}

const visibleResource = (server) => rust.visibleResource(String(server), [...config.hiddenResources]);

// A reader's line in the map's sources (core/native/reader.js), as Rust
// words it.
function readerStatus(name, reader, what) {
  const { state, sessions } = reader.status();
  return rust.readerStatus(name, String(state), sessions, what);
}

// Runs `fn` one call at a time. A call while one runs waits for it, and the
// calls that wait together share the next run, which sees things as they
// are when it starts.
function serial(fn) {
  let running = null;
  let queued = null;
  const next = () => {
    if (queued) return queued;
    if (!running) return (running = fn().finally(() => (running = null)));
    queued = running.catch(() => {}).then(() => ((queued = null), next()));
    return queued;
  };
  return next;
}

export class XenoNetState extends EventEmitter {
  // `native`: a test's own app registry and native readers' table
  // ({ apps, table }, as core/native/index.js takes them).
  constructor({ native } = {}) {
    super();
    // The projects folder you chose, if you did (see workspace.js).
    workspace.applyRoot();
    this.graph = null;
    this.index = new Map();
    this.parent = new Map();
    this.sources = [];
    // What a refresh remembers for the next one (state.rs's Memory).
    this.memory = new rust.StateMemory();
    this.refreshing = null;
    this.projects = [];
    this.repos = new RepoWatch();
    this.identities = config.identities
      ? new IdentityWatch({ home: config.homeDir, claudeConfigFile: config.claudeConfigFile, codexHome: config.codexHome, ghConfigDir: config.ghConfigDir, claudeProjectsDir: config.claudeProjectsDir })
      : null;
    // The names you gave the demo's sessions, which exist only in XenoNet.
    this.demoTitles = new Map();
    // The building blocks this is made of (core/blocks), for /api/health
    // and the doctor. The ones that read for a source of the map (see
    // `sources`) say which, so a source that fails is reported once.
    this.blocks = new Blocks();
    this.blocks.add(store.stateStore);
    this.claudeSessions = this.blocks.add(claudeSessionsGatherer({ gather: config.demo ? async () => demoSessions("claude") : undefined }), { source: "claude" });
    this.codexThreads = this.blocks.add(codexThreadsGatherer({ gather: config.demo ? async () => demoSessions("codex") : undefined }), { source: "codex" });
    this.codexProjects = this.blocks.add(codexProjectsGatherer({ gather: config.demo ? async () => demoNativeProjects() : undefined }), { source: "codex-projects" });
    this.blocks.add(codexClient, { source: "codex" });
    this.grokSessions = this.blocks.add(grokSessionsGatherer({ gather: config.demo ? async () => demoSessions("grok") : undefined }), { source: "grok" });
    // Runs XenoNet observes without a reader of its own: numbat's records,
    // when XENONET_NUMBAT_FILE names them (core/numbat.js), and runs actors
    // report through the gateway (core/reported-runs.js). Never in the demo.
    this.numbat = config.numbatFile && !config.demo ? this.blocks.add(new NumbatSessions({ file: config.numbatFile }), { source: "numbat" }) : null;
    this.reportedRuns = new ReportedRuns();
    // Experimental native readers (core/native): sessions read from an
    // app's own records, for each app the registry gives one that
    // XENONET_READERS names. Each is { app, source, what, reader }.
    this.nativeReaders = nativeReaders(config, native);
    for (const n of this.nativeReaders) this.blocks.add(n.reader, { source: n.source });
    // Sessions from outside XenoNet's Claude Code and Codex readers. The
    // native readers come first, so a run they read wins over numbat's.
    this.observed = observedSessions([...this.nativeReaders.map((n) => n.reader), this.numbat, this.reportedRuns]);
    this.live = new LiveTelemetry({
      claudeRoot: config.claudeProjectsDir,
      codexRoot: path.join(config.codexHome, "sessions"),
      grokRoot: path.join(config.grokHome, "sessions"),
      claudeDesktopDir: config.claudeDesktopDir,
      claudeUsage: config.claudeUsage ? () => fetchClaudeUsage() : null,
      home: config.homeDir,
      contextServer: config.wikiServer,
    });
    this.blocks.add(this.live.tailer, { source: "live" });
    this.blocks.add(claudeLines);
    this.blocks.add(codexLines);
    this.blocks.add(grokLines);
    for (const reading of [this.live.usage.anthropic, this.live.usage.desktop]) if (reading) this.blocks.add(reading);
    if (this.identities) this.blocks.add(this.identities, { source: "identities" });
    this.blocks.add(this.repos);
    // The projects scan's walk, for the map's scans (see scanProjects).
    this.projectsScanner = this.blocks.add(projectsScanner(), { source: "projects" });
    const codexRoots = [path.join(config.codexHome, "sessions"), path.join(config.codexHome, "archived_sessions")];
    const grokRoot = path.join(config.grokHome, "sessions");
    this.search = new TranscriptSearch({
      dbFile: path.join(config.cacheDir, "search.sqlite"),
      claudeRoot: config.claudeProjectsDir,
      codexRoots,
      grokRoot,
    });
    // Where sessions' transcripts are, for a session's models, tokens,
    // costs and timing (see stats.js), and a Codex thread's rollout.
    this.transcripts = new Transcripts({ claudeRoot: config.claudeProjectsDir, codexRoots, grokRoot });
    this.sessionStats = createStats({ transcripts: this.transcripts });
  }

  // Coalesces concurrent refreshes into one.
  refresh() {
    this.refreshing ??= this.#refresh().finally(() => (this.refreshing = null));
    return this.refreshing;
  }

  // A refresh that sees a change just made: one already running read the
  // state from before it.
  async refreshAfterChange() {
    await this.refreshing?.catch(() => {});
    return this.refresh();
  }

  async #refresh() {
    // The projects folder as this refresh found it, for all of it, should
    // another be chosen meanwhile (see setRoot).
    const root = config.projectsDir;
    const [codex, native, claude, grok] = await Promise.all([
      gathered("codex", this.codexThreads),
      gathered("codex-projects", this.codexProjects),
      gathered("claude", this.claudeSessions),
      gathered("grok", this.grokSessions),
    ]);
    // Keep the previous sessions for a source that failed this round, so a
    // transient error doesn't make half the graph vanish.
    const prev = this.graph?.nodes.filter((n) => n.kind === "session") ?? [];
    // Observed runs, unless XenoNet reads the same run itself.
    this.numbat?.read();
    for (const n of this.nativeReaders) n.reader.read();
    const lists = [["codex", codex], ["claude", claude], ["grok", grok]].map(([app, g]) => [app, g.status.ok, g.value]);
    const sessions = rust.mergeSessions(prev, lists, this.observed.sessions(), [...this.demoTitles]);
    // Where sessions ran is evidence of where the projects are (see
    // scanProjects), so the scan comes after the session lists.
    this.sessionDirs = rust.sessionDirs(sessions);
    const rules = this.projectRules({ root });
    const projects = await settle("projects", () => scanProjects(root, { sessionDirs: this.sessionDirs, rules, limits: config.projectLimits, scanner: this.projectsScanner }));
    for (const p of projects.value) p.rule = rules[p.key] ?? null;
    this.projects = projects.value;
    this.scan = projects.value.scan ?? null;
    if (this.scan?.ok) Object.assign(projects.status, { count: this.scan.projects, note: rust.projectsNote(this.scan) });
    // Just after the projects folder changed, its repos' status and git
    // identities are read first, so the new map goes out whole (see setRoot).
    if (this.repoPassFor === pathKey(root)) {
      this.repoPassFor = null;
      await this.#repoPass().catch(() => {});
    }
    const wiki = await readWiki(config.wikiDir, projects.value, { root, server: config.wikiServer }).catch(() => null);

    const graph = buildGraph({
      projectsDir: root,
      homeDir: config.homeDir,
      projects: projects.value,
      sessions,
      nativeProjects: native.value,
      resources: [...this.live.snapshot().resources, ...(config.demo ? [{ server: "reference-library", ts: null }] : [])].filter((r) => visibleResource(r.server)),
      wiki,
      repos: this.repos.status,
      layout: store.getLayout(),
      windows: config,
    });
    for (const n of graph.nodes) n.actions = actionsFor(n);
    // Who each session ran as, and whose each project is (see identities.js).
    const identities = await settle("identities", async () => {
      if (!this.identities) return [];
      const { identities, dirty } = this.identities.link(graph, store.getIdentities());
      if (dirty) store.saveIdentities();
      this.#noteSignins();
      return identities;
    });
    graph.identities ??= [];
    graph.hideIdentities = config.hideIdentities;
    // The demo's sessions exist only in XenoNet (renaming one says so).
    graph.demo = !!config.demo;
    if (identities.status.ok) identities.status.note = rust.identitiesNote(identities.value);
    this.sources = [
      projects.status,
      codex.status,
      native.status,
      claude.status,
      grok.status,
      ...this.nativeReaders.map((n) => readerStatus(n.source, n.reader, n.what)),
      ...(this.numbat ? [readerStatus("numbat", this.numbat, "numbat's file")] : []),
      { name: "live", ok: this.live.status.watching, count: this.live.status.files },
      { name: "wiki", ok: !!wiki, count: wiki?.pages.length ?? 0, error: wiki ? undefined : "Xenolith folder not found" },
      { name: "search", ok: !this.search.status.errors, count: this.search.status.docs, ms: this.search.status.ms },
      ...(this.identities ? [identities.status] : []),
    ];
    graph.sources = this.sources;

    const pulses = this.memory.updated(sessions);
    const changed = this.memory.changed(graph);
    this.graph = graph;
    this.index = new Map(graph.nodes.map((n) => [n.id, n]));
    this.parent = new Map(graph.links.filter((l) => l.kind !== "lineage-cross").map((l) => [l.target, l.source]));
    if (changed) this.emit("graph", graph);
    for (const id of pulses) this.emit("pulse", { nodeId: id, reason: "updated" });
    return graph;
  }

  // ----- how the projects folder is read -----

  // The rules a scan of `root` follows: Codex's picks for that folder, then
  // yours on top. Picks are paths inside the folder Codex looked at, so
  // they're kept per folder and never laid over another one's.
  projectRules({ codex = true, root = config.projectsDir } = {}) {
    return rust.projectRules(!!codex, String(root), codex ? store.getCodexPicks(pathKey(root)) : null, store.getProjectRules());
  }

  // Sets or clears (as: null) your rule for one folder. Rules are made for
  // folders in the projects folder, but they stay when it changes (they
  // apply under whichever holds them), so any of yours can be undone,
  // wherever it is.
  async setProjectRule(dir, as) {
    const full = path.resolve(String(dir ?? ""));
    const problem = rust.ruleProblem(as, full, String(config.projectsDir), !!as && !!fs.statSync(full, { throwIfNoEntry: false })?.isDirectory());
    if (problem) throw new Error(problem);
    store.setProjectRule(pathKey(full), as ? { path: full, as, at: Date.now() } : null);
    await this.refreshAfterChange();
    return this.projectDetection();
  }

  // Clears your rules for the folders in the projects folder in use, or
  // with `all`, for every folder, others included.
  async clearProjectRules({ all = false } = {}) {
    store.clearProjectRules(all ? null : pathKey(config.projectsDir));
    await this.refreshAfterChange();
    return this.projectDetection();
  }

  // "Detect projects again": nothing cached, a fresh scan.
  async rescanProjects() {
    forgetScans();
    await this.refreshAfterChange();
    return this.projectDetection();
  }

  // What Codex would be sent: the folders as XenoNet reads them on its own
  // rules and yours, without Codex's earlier picks.
  async codexOutline(root = config.projectsDir) {
    const folders = (await scanProjects(root, { sessionDirs: this.sessionDirs ?? [], rules: this.projectRules({ codex: false, root }), limits: config.projectLimits })).folders ?? [];
    const outline = outlineFolders(folders);
    return { outline, fingerprint: fingerprint(outline) };
  }

  // Asks Codex, only for the outline you were shown (same fingerprint). The
  // picks are kept for the folder it was shown, should another be chosen
  // while Codex thinks.
  // `decider`: ask a decision model (core/decide.js) instead of Codex,
  // the same outline as one choice a folder.
  async askCodexAboutProjects(expected, { decider = null } = {}) {
    const root = config.projectsDir;
    const { outline, fingerprint: fp } = await this.codexOutline(root);
    if (expected && expected !== fp) throw Object.assign(new Error("The folders changed since you looked. Look again before sending."), { status: 409 });
    const startedAt = Date.now();
    let picks;
    try {
      picks = config.demo ? demoPicks(outline) : decider ? await askDeciderForPicks(decider, outline) : await askCodexForPicks(findCodexBinary(), outline);
    } catch (err) {
      // The earlier picks, if any, stay.
      throw Object.assign(new Error(`${decider ? "The decision model" : "Codex"} couldn't sort the folders: ${err.message ?? err}`), { status: err.status === 409 ? 409 : 502 });
    }
    store.setCodexPicks(pathKey(root), { ranAt: Date.now(), ms: Date.now() - startedAt, fingerprint: fp, sent: outline.length, picks, by: decider ? decider.model : "codex" });
    await this.refreshAfterChange();
    return this.projectDetection();
  }

  // Forgets Codex's picks for the folder in use; other folders keep theirs.
  async forgetCodexPicks() {
    store.setCodexPicks(pathKey(config.projectsDir), null);
    await this.refreshAfterChange();
    return this.projectDetection();
  }

  // The scan's state: when it ran, what it found, your rules (each saying
  // whether it's in this projects folder) and Codex's picks for this folder,
  // which are stale once its folders have changed since Codex looked.
  async projectDetection() {
    const root = config.projectsDir;
    const codex = store.getCodexPicks(pathKey(root));
    const current = codex ? (await this.codexOutline(root)).fingerprint : null;
    return rust.projectDetection(String(root), this.scan, store.getProjectRules(), codex, current);
  }

  // ----- the projects folder (see workspace.js) -----

  // The folder XenoNet reads, where that comes from, and its last scan.
  rootInfo() {
    return { ...workspace.root(), scan: this.scan ?? null };
  }

  // What XenoNet would find in a folder, with the folders sessions ran in and
  // your rules and Codex's picks for it, without reading it for the map.
  async checkRoot(dir) {
    if (config.demo) await clientsReady();
    const full = workspace.resolveFolder(dir);
    return workspace.check(dir, { sessionDirs: this.sessionDirs ?? [], rules: full ? this.projectRules({ root: full }) : {} });
  }

  // Reads another projects folder, once it's checked (null: back to
  // XENONET_PROJECTS_DIR's or ~/Projects). What that does to the rest:
  // - the layout stays as it is: node ids are absolute paths, so a folder's
  //   positions come back with it;
  // - your rules stay, by absolute path, and apply under whichever folder
  //   holds them;
  // - Codex's picks are kept per folder (see projectRules);
  // - the projects scan (its caches forgotten), the git status loop,
  //   identities' repo scan and the map follow in one pass, and the map goes
  //   out like any other. Transcripts, sessions and the engine carry on, and
  //   sessions outside the new folder become folders outside it.
  async setRoot(dir) {
    const no = workspace.locked();
    if (no) throw no;
    if (config.demo) await clientsReady();
    let full = null;
    if (dir != null) {
      const v = workspace.validate(dir);
      if (!v.ok) throw Object.assign(new Error(v.error), { status: 400 });
      full = v.path;
    }
    const before = config.projectsDir;
    const after = workspace.choose(full);
    const changed = pathKey(before) !== pathKey(after.path);
    if (changed) {
      forgetScans();
      this.repoPassFor = pathKey(after.path);
      await this.refreshAfterChange();
    }
    return { ...this.rootInfo(), changed };
  }

  // ----- identities (see identities.js) -----

  // A sign-in that changed since the last refresh goes to the activity feed:
  // Claude Code's, Codex's, and gh's active account, which HTTPS pushes sign
  // in as. The page writes the names, so hiding identities hides them too.
  #noteSignins() {
    for (const ev of this.memory.noteSignins(this.identities.accounts(), Date.now())) this.emit("activity", ev);
  }

  // Who's signed in now, as the last refresh found it.
  get signedIn() {
    return this.memory.signedIn ?? undefined;
  }

  identityDetection() {
    return rust.identityDetection(!!this.identities, this.graph?.generatedAt ?? null, this.graph?.identities ?? [], store.getIdentities(), (id) => this.index.get(id) ?? null);
  }

  #known(id) {
    const known = rust.knownIdentity(id, this.graph?.identities ?? [], store.getIdentities().known);
    if (known == null) throw new Error("XenoNet doesn't know that identity");
    return known;
  }

  // The name you give an identity; identities with the same name show as one.
  async nameIdentity(id, name) {
    store.setIdentityName(this.#known(id), rust.cleanIdentityName(name) ?? null);
    await this.refreshAfterChange();
    return this.identityDetection();
  }

  // Renames a session in its own app, so Claude Code, Claude desktop or
  // Codex shows the new name too (see rename.js). The demo's sessions exist
  // only in XenoNet, which keeps their new names itself. Says where the name
  // went, and anything you should know about it.
  async renameSession(nodeId, title) {
    const node = this.index.get(nodeId);
    if (!renamable(node)) throw Object.assign(new Error("XenoNet doesn't know that session"), { status: 404 });
    const name = cleanTitle(title);
    if (!name) throw Object.assign(new Error("A session's name can't be empty"), { status: 400 });
    const saved = [];
    let note = null;
    if (config.demo) this.demoTitles.set(node.id, name);
    // renamable takes Claude Code's sessions and Codex's alone.
    else if (node.app === "codex") {
      await renameCodexThread(node.rawId, name);
      saved.push("Codex");
    } else {
      const done = await renameClaudeSession(node, name);
      saved.push("Claude Code");
      if (done.desktop) saved.push("Claude desktop");
      // The desktop holds its sessions while it runs, and reads its records
      // when it starts.
      note = rust.renameNote(!!done.desktop, done.desktopError);
    }
    await this.refreshAfterChange();
    return { id: node.id, title: this.index.get(node.id)?.title ?? name, saved, note, demo: config.demo };
  }

  async hideIdentity(id, hidden) {
    store.setIdentityHidden(this.#known(id), !!hidden);
    await this.refreshAfterChange();
    return this.identityDetection();
  }

  // "This session ran as" or "this project belongs to" an identity; null
  // goes back to what XenoNet found.
  async pinIdentity(nodeId, identity) {
    const kind = this.index.get(nodeId)?.kind ?? (store.getIdentities().pins[nodeId] ? "pinned" : null);
    const problem = rust.pinProblem(typeof kind === "string" ? kind : null);
    if (problem) throw new Error(problem);
    store.setIdentityPin(nodeId, identity ? this.#known(identity) : null);
    await this.refreshAfterChange();
    return this.identityDetection();
  }

  // "Detect identities again": every file and repo read afresh.
  async rescanIdentities() {
    this.identities?.forget();
    await this.identities?.refreshRepos(this.projects).catch(() => {});
    await this.refreshAfterChange();
    return this.identityDetection();
  }

  // The project (or zone) a session lives in.
  homeOf(id) {
    if (!id) return null;
    const kindOf = (cur) => {
      const n = this.index.get(cur);
      return n ? String(n.kind) : null;
    };
    return rust.homeOf(String(id), kindOf, (cur) => this.parent.get(cur) ?? null) ?? null;
  }

  titleOf = (id) => {
    const n = this.index.get(id);
    return rust.titleOf(n?.title, n?.name, id);
  };

  // The tailer's snapshot as the page gets it (state.rs's live_snapshot).
  // Plan limits name the account signed in now; the tailer keeps its own.
  liveSnapshot() {
    return rust.liveSnapshotFor(this.live.snapshot(), [...config.hiddenResources], store.getAcks(), this.memory.signedIn, (id) => this.index.get(id) ?? null);
  }

  // Snapshots go out at most every LIVE_THROTTLE_MS, and every 2s regardless
  // so rates decay on screen when activity stops.
  #queueLive() {
    if (this.liveTimer) return;
    this.liveTimer = setTimeout(() => {
      this.liveTimer = null;
      this.emit("live", this.liveSnapshot());
    }, LIVE_THROTTLE_MS);
  }

  // When an agent ends a turn in a repo with uncommitted work, nudge (at most
  // every 30 minutes per project). Not a turn you stopped: its work is
  // likely half done.
  #maybeShipIt(ev) {
    if (ev.kind !== "turn-end" || ev.interrupted) return;
    const home = this.homeOf(ev.sessionId);
    const node = home ? this.index.get(home) : null;
    const nudge = this.memory.shipIt(ev, node?.repo ? [home, node.name ?? null, node.repo] : null, Date.now());
    if (nudge) this.emit("activity", nudge);
  }

  // The git status loop's pass, with each repo's git identity read on the
  // same beat (when its config changed), over the projects as they are when
  // it starts. One pass at a time, so one over an old projects folder can't
  // land after one over the new. Says whether anything changed.
  #repoPass = serial(async () => {
    const projects = this.projects;
    const status = () => JSON.stringify([...this.repos.status.entries()].map(([k, v]) => [k, v.dirty, v.branch, v.ahead]));
    const before = status();
    await this.repos.refresh(projects);
    const identities = await this.identities?.refreshRepos(projects).catch(() => false);
    return before !== status() || !!identities;
  });

  // A pass that found nothing new may have waited behind one that did, and
  // that one's refresh may still be under way: wait for it too, so the map
  // has the repos' status once this resolves.
  async refreshRepos() {
    if (await this.#repoPass()) await this.refresh();
    else await this.refreshing?.catch(() => {});
  }

  async start() {
    // Refusals, from history as it's read too, so none read at start is
    // missed (core/refusals).
    this.live.on("refusal", (ev) => this.emit("refusal", ev));
    await this.live.start().catch((e) => console.error("[xenonet] live telemetry failed:", e));
    this.live.on("activity", (ev) => {
      this.emit("activity", ev);
      this.#maybeShipIt(ev);
      this.#queueLive();
    });
    this.live.on("tokens", (ev) => this.emit("tokens", ev));
    // A brand-new transcript means a new session: rebuild the graph now.
    this.live.on("new-file", () => {
      clearTimeout(this.newFileTimer);
      this.newFileTimer = setTimeout(() => this.refresh().catch(() => {}), 1200);
    });
    if (config.demo) this.stopDemo = startDriver();
    const tick = () => this.refresh().catch((e) => console.error("[xenonet] refresh failed:", e));
    await tick();
    this.timer = setInterval(tick, config.pollMs);
    this.heartbeat = setInterval(() => this.emit("live", this.liveSnapshot()), 2000);
    this.refreshRepos().catch(() => {});
    this.repoTimer = setInterval(() => this.refreshRepos().catch(() => {}), REPO_POLL_MS);
    if (config.wikiDir) {
      try {
        this.wikiWatch = fs.watch(config.wikiDir, { recursive: true }, () => {
          clearTimeout(this.wikiTimer);
          this.wikiTimer = setTimeout(() => this.refresh().catch(() => {}), 800);
        });
        this.wikiWatch.on("error", () => {});
      } catch {}
    }
    // Build the search index in the background, then keep it current.
    const reindex = () => this.search.index().catch((e) => console.error("[xenonet] search index failed:", e));
    this.searchStart = setTimeout(reindex, 1500);
    this.searchTimer = setInterval(reindex, 60000);
  }

  stop() {
    // The pending one-off timers too: a stopped state starts no more work,
    // and a first index after stopping would open the search file again.
    for (const t of [this.timer, this.heartbeat, this.repoTimer, this.searchTimer]) clearInterval(t);
    for (const t of [this.searchStart, this.newFileTimer, this.wikiTimer]) clearTimeout(t);
    this.stopDemo?.();
    this.wikiWatch?.close();
    this.live.stop();
    this.search.close();
    store.flush();
  }

  // ----- features -----

  async searchTranscripts(q) {
    if (Date.now() - this.search.status.lastIndexAt > 15000 && !this.search.status.indexing) await this.search.index().catch(() => {});
    return this.search.search(q).map((r) => ({ ...r, inGraph: this.index.has(r.sessionId), title: this.titleOf(r.sessionId) }));
  }

  digest(since) {
    return summarize(this.live.historySince(since), { since, titleOf: this.titleOf, live: this.liveSnapshot() });
  }

  dayLog() {
    return dayLog(this.live.history, {
      titleOf: this.titleOf,
      homeOf: (id) => this.homeOf(id),
      nameOf: (id) => this.index.get(id)?.name ?? id,
      live: this.liveSnapshot(),
      repos: this.projects.filter((p) => this.repos.status.get(p.key)?.dirty).map((p) => ({ name: p.name, ...this.repos.status.get(p.key) })),
    });
  }

  // The day's constellation for a poster or a film (redact.js): star names
  // in place of project names, and no hours. The real names come only
  // when `names` asks for them, for an export that reveals them.
  constellation({ names = false } = {}) {
    return constellationOf(this.dayLog(), { salt: posterSalt(config.stateDir), names, nodes: [...this.index.values()] });
  }

  // The map's own labels for an export: each labelled node's star name, by
  // id, from the same names as the constellation (redact.js, labelsOf).
  exportLabels() {
    return labelsOf([...this.index.values()], this.dayLog(), { salt: posterSalt(config.stateDir) });
  }

  async narrate(kind, since) {
    if (config.demo) return demoNarration();
    const text = kind === "day" ? summaryText(this.dayLog(), "Today so far") : summaryText(this.digest(since), "While the user was away");
    return narrate(findCodexBinary(), text);
  }

  // A brief for continuing a session elsewhere. Uses what the tailer saw, and
  // falls back to the transcript APIs for sessions older than its 24h window.
  async handoff(nodeId, target) {
    const node = this.index.get(nodeId);
    if (!node || node.kind !== "session") throw new Error("unknown session");
    let notes = this.live.sessionNotes(nodeId) ?? { prompts: [], messages: [], files: [], edits: [] };
    // A session of an app XenoNet doesn't read the transcripts of is
    // handed off with what the tailer saw of it.
    const app = config.demo ? node.app : tailedOr(node.app, null);
    if (app && rust.handoffNeedsTranscript(notes)) notes = rust.handoffNotes(notes, await this.readTranscript(node, { limit: app === "codex" ? 60 : 80 }));
    return { brief: handoffBrief({ node, notes, target }), folder: node.worktree || node.cwd || null };
  }

  // A session's transcript, a page at a time. A Codex thread whose items
  // the app-server can't list is read from its rollout (see codex.js). A
  // session of an app XenoNet doesn't tail is refused (a 400), not read as
  // Claude Code's.
  readTranscript(node, opts) {
    if (config.demo) return demoTranscript(node.rawId);
    const app = tailedOr(node.app, null);
    if (!app) throw Object.assign(new Error(untailed(node.app)), { status: 400 });
    if (app === "grok") return readGrokSession(node.rawId, opts);
    if (app === "codex") return readCodexThread(node.rawId, { ...opts, rollout: async () => (await this.transcripts.files(node.id))[0]?.file ?? null });
    return readClaudeSession(node.rawId, opts);
  }

  // Compact status for the tray app.
  summary() {
    return rust.trayStatus(this.liveSnapshot(), this.titleOf, rust.trayUrl(String(config.host), config.port));
  }
}
