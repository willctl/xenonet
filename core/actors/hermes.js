// Hermes Agent's observer (docs/design/actors/hermes.md; interface.md,
// section 7): observed and attributed only, with no installs and no change
// to Hermes's settings. One actor per profile: actor:hermes for the
// default one, actor:hermes-<profile> for each named one.
//
// What it reads (hermes-files.js, metadata only): Hermes's processes in the
// process table, its spawn ledger, each profile's gateway state, scheduler
// ticker, background processes, scheduled jobs and session database.
//
// Presence, by section 1's table for a local agent:
//
//   working   a process that hosts the agent runs (a desktop backend, the
//             gateway, the CLI or the terminal UI's backend), and a session
//             was active in the last two minutes, or its gateway, with a
//             fresh heartbeat, has agents at work
//   idle      one runs, with nothing recent; or only Hermes.app runs, with
//             no backend here, which says so in its status
//   away      its gateway's heartbeat is over 120 s old while its process
//             runs, and no session of it is active
//   offline   no process of it runs
//   unknown   its session database can't be read and nothing of it runs
//
// Its status is XenoNet's own words: what its busiest session's activity says
// it's doing, matched to Hermes's templates and never shown raw, with the
// project; or its gateway's state.
//
// Events, each from facts: ran and edited (its tools, gathered per minute;
// an edit's files are the paths SQLite takes out of its calls), messaged
// (its replies in sessions from a messaging platform), finished and failed
// (its sessions and scheduled jobs), and spawned: the Claude Code and Codex
// sessions it started, by process ancestry (a Hermes agent host above
// them, or a background process Hermes keeps), a Codex thread whose
// originator is "hermes", or one in a scratch worktree of its codex skill.
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { Spawns, startedUnder } from "./attribution.js";
import { PRODUCTS } from "./products.js";
import { hermesSlug } from "./names.js";
import { pathStyle, pathKey, isUnder, tilde } from "../paths.js";
import { hermesRoot, listProfiles, installed, readStamp, readLedger, readGateway, readTicker, readProcesses, readJobs, readStateDb, sameStart, msOf } from "./hermes-files.js";

const PRODUCT = PRODUCTS["hermes-agent"];
// Every Hermes heartbeat (the gateway's, the ticker's, a backend's) comes
// every 60 s.
export const HERMES_HEARTBEAT_MS = 60e3;
const ACTIVE_MS = 2 * 60e3;
const STALE_MS = 120e3;
const TICKER_STALE_MS = 150e3;
// A first read of a session database looks this far back; later reads take
// rows newer than this as new work, and older ones (a compaction writing
// history again) as history.
const REPLAY_MS = 10 * 60e3;
const HISTORY_MS = 3600e3;
const DAY_MS = 24 * 3600e3;
const PROFILES_MS = 60e3;
const RUNNING_GATEWAY = new Set(["starting", "running", "degraded", "draining"]);
// The purposes in the ledger whose process hosts the agent, so the process
// check must know it as Hermes's (the doctor's self-test).
const HOSTS = new Set(["cli", "serve", "dashboard", "gateway"]);

// What Hermes's tools are, as XenoNet counts them.
const TOOL_KINDS = { terminal: "ran", process_manage: "ran", execute_code: "ran" };
// Messaging sources of Hermes's sessions, as the platforms an event names.
const PLATFORMS = { telegram: "telegram", discord: "discord", slack: "slack", whatsapp: "whatsapp", signal: "signal", matrix: "matrix", mattermost: "mattermost", email: "email", sms: "sms", imessage: "imessage", bluebubbles: "imessage", teams: "teams" };

// A profile's actor's slug: names.js's hermesSlug, which a Hermes plugin
// registering later takes too, so both name the same actor. Profiles
// whose names differ only in case or punctuation get actors of their own,
// rather than one whose presence would flip between theirs at every look.
export const profileSlug = hermesSlug;

// What a tool does, in XenoNet's words, by its name.
function toolWords(tool) {
  const t = String(tool ?? "");
  if (TOOL_KINDS[t]) return "Running a command";
  if (t === "write_file" || t === "patch") return "Editing files";
  if (t === "read_file" || t === "search_files") return "Reading files";
  if (/^(browser_|web_)/.test(t)) return "Browsing the web";
  if (t === "delegate_task") return "Working with a subagent";
  if (t === "cronjob_manage") return "Scheduling a job";
  if (t === "computer_use") return "Using the computer";
  return null;
}

// What a session's activity stamp (last_activity_description, from fixed
// templates such as "executing tool: terminal" or "waiting for stream
// response (12s, …)") says it's doing, in XenoNet's words, or null. The
// stamp itself is never shown: some templates carry free text.
export function doingOf(description) {
  const d = String(description ?? "");
  const tool = /^(?:executing tool|tool completed|sequential tool running \([^)]*\)):\s*([a-z_][a-z0-9_]*)/i.exec(d)?.[1] ?? /^executing \d+ tools concurrently:\s*([a-z_][a-z0-9_]*)/i.exec(d)?.[1];
  if (tool) return toolWords(tool);
  if (/^(waiting for (stream|provider|non-streaming)|receiving stream|local model loading|stream retry|stale stream)/i.test(d)) return "Thinking";
  if (/^(context )?compress|compaction/i.test(d)) return "Tidying its context";
  return null;
}

// How a session ended, as an event's kind: failed when its reason says
// so, nothing for one that went on in another (compaction), else finished.
export function endingOf(reason) {
  const r = String(reason ?? "");
  if (/compress|compact/i.test(r)) return null;
  if (/error|fail|crash|unreachable|overflow|timed?_?out|out_of_credits|filter/i.test(r)) return "failed";
  return "finished";
}

// A file read again only when it changes: its mtime and size, or why it
// can't be had.
function stampOf(files, ...paths) {
  return paths
    .map((p) => {
      try {
        const st = files.statSync(p);
        return `${st.mtimeMs}:${st.size}`;
      } catch (err) {
        return err.code ?? "?";
      }
    })
    .join("|");
}

export class HermesObserver extends Observer {
  // `home`: the folder Hermes's root is in (~). `procs`: the process
  // table (core/procs.js). `env`: for HERMES_HOME.
  constructor({ actors, procs, home, env = process.env, files = fs, open, observing, every, clock } = {}) {
    super({ name: "hermes-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.root = hermesRoot(home, env);
    this.files = files;
    this.open = open;
    this.cache = new Map();
    this.profiles = new Map();
    this.profilesAt = -Infinity;
    this.profileList = [];
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
    this.sourceStates = new Map();
    this.recognized = null;
  }

  // A source read again only when its files change.
  #stamped(key, paths, read) {
    const stamp = stampOf(this.files, ...paths);
    const hit = this.cache.get(key);
    if (hit && hit.stamp === stamp) return hit.value;
    const value = read();
    this.cache.set(key, { stamp, value });
    return value;
  }

  #source(name, file, state) {
    this.sourceStates.set(name, { name, file: tilde(file, this.home), state });
  }

  // A profile's actor: its id and name, and what XenoNet keeps of it between
  // looks.
  #profile(name) {
    let p = this.profiles.get(name);
    if (!p) {
      const slug = profileSlug(name);
      p = {
        name,
        id: `actor:${slug}`,
        home: name === "default" ? this.root : path.join(this.root, "profiles", name),
        // The actor's name: "Hermes", and a named profile's in brackets when
        // it's plain enough to show.
        title: name === "default" ? "Hermes" : /^[A-Za-z0-9][\w .-]{0,31}$/.test(name) ? `Hermes (${name})` : "Hermes (a profile)",
        db: { stamp: null, lastId: null, read: false },
        sessions: new Map(),
        lastTool: new Map(),
      };
      this.profiles.set(name, p);
    }
    return p;
  }

  // The profiles there are: the folders Hermes made, and any a running
  // process of the ledger names.
  #profilesNow(at, ledger) {
    if (at - this.profilesAt >= PROFILES_MS) {
      this.profilesAt = at;
      this.profileList = listProfiles(this.root, { files: this.files }).map((p) => p.name);
    }
    const names = new Set(this.profileList);
    // Only a plain name, so a profile's folder stays inside Hermes's.
    for (const e of ledger) if (!e.profile || /^[A-Za-z0-9][\w.-]{0,63}$/.test(e.profile)) names.add(e.profile || "default");
    return [...names].map((n) => this.#profile(n));
  }

  // ---------- one look ----------

  async look(at = this.clock.now()) {
    const known = this.actors.ofProduct(PRODUCT.id).filter((id) => !this.actors.info(id)?.via.includes("demo"));
    const isInstalled = installed(this.root, { files: this.files });
    if (!isInstalled && !known.length) return;

    const census = await this.procs.census();
    const { table } = census;
    this.table = table;
    const roleOf = (p) => {
      const prod = census.of(p).product;
      return prod?.product === PRODUCT.id ? prod.role : null;
    };
    const hosts = [];
    const tuis = [];
    const apps = [];
    for (const p of table.values()) {
      const role = roleOf(p);
      if (role === "host") hosts.push(p);
      else if (role === "tui") tuis.push(p);
      else if (role === "app") apps.push(p);
    }

    const stampFile = path.join(this.root, "hermes-agent", "install-stamp.json");
    const stamp = this.#stamped("stamp", [stampFile], () => readStamp(this.root, { files: this.files }));
    this.#source("build stamp", stampFile, stamp.state);
    const version = stamp.version;
    const ledgerFile = path.join(this.root, "spawn-ledger.json");
    const ledger = this.#stamped("ledger", [ledgerFile], () => readLedger(this.root, { version, files: this.files }));
    this.#source("spawn ledger", ledgerFile, ledger.state);
    const live = ledger.entries.filter((e) => table.has(e.pid) && (e.createTime == null || sameStart(e.createTime, table.get(e.pid).start)));

    // The process check's self-test: every live host the ledger lists
    // should be one the process table knows as Hermes's.
    const expected = live.filter((e) => HOSTS.has(e.purpose));
    this.recognized = { expected: expected.length, found: expected.filter((e) => roleOf(table.get(e.pid)) === "host").length, purposes: expected.filter((e) => roleOf(table.get(e.pid)) !== "host").map((e) => e.purpose) };

    const profiles = this.#profilesNow(at, live);
    const profileOf = new Map();
    for (const e of live) if (this.profiles.has(e.profile || "default")) profileOf.set(e.pid, e.profile || "default");
    const read = new Map();
    for (const p of profiles) {
      const r = this.#readProfile(p, { version, at });
      read.set(p.name, r);
      if (r.gatewayAlive) profileOf.set(r.gateway.pid, p.name);
      if (r.tickerAlive) profileOf.set(r.ticker.pid, p.name);
      for (const hb of r.heartbeats) if (at - (msOf(hb.last_heartbeat) ?? 0) < TICKER_STALE_MS) profileOf.set(hb.pid, p.name);
    }

    // Each process of Hermes's to its profile: by the ledger, the gateway's
    // and the ticker's records and the backends' heartbeats; a terminal UI
    // by the backend it runs; the app by the backend it started.
    const profileOfHost = (p) => (profileOf.has(p.pid) ? profileOf.get(p.pid) : "default");
    const running = new Map(profiles.map((p) => [p.name, []]));
    const appsOf = new Map(profiles.map((p) => [p.name, []]));
    const add = (map, name, p) => (map.get(name) ?? map.set(name, []).get(name)).push(p);
    for (const p of hosts) add(running, profileOfHost(p), p);
    for (const t of tuis) add(running, profileOfHost(hosts.find((h) => h.ppid === t.pid) ?? t), t);
    const mainApps = apps.filter((a) => !apps.some((b) => b.pid === a.ppid));
    for (const a of mainApps) add(appsOf, live.find((e) => e.purpose === "serve" && e.spawnerPid === a.pid)?.profile || "default", a);

    for (const p of profiles) {
      const r = read.get(p.name);
      this.#observe(p, { ...r, processes: running.get(p.name) ?? [], apps: appsOf.get(p.name) ?? [], version, at, isInstalled });
    }

    // The sessions Hermes started.
    await this.#attribute({ census, hosts, profileOfHost, read, at });
  }

  // A profile's files, each read again only when it changes, and what's
  // new in its session database.
  #readProfile(p, { version, at }) {
    const f = (...parts) => path.join(p.home, ...parts);
    const gateway = this.#stamped(`${p.name}:gateway`, [f("gateway_state.json")], () => readGateway(p.home, { version, files: this.files }));
    const ticker = this.#stamped(`${p.name}:ticker`, [f("cron", "ticker_heartbeat"), f("cron", "ticker_last_success")], () => readTicker(p.home, { version, files: this.files }));
    const processes = this.#stamped(`${p.name}:processes`, [f("processes.json")], () => readProcesses(p.home, { version, files: this.files }));
    const jobs = this.#stamped(`${p.name}:jobs`, [f("cron", "jobs.json")], () => readJobs(p.home, { version, files: this.files }));
    const label = p.name === "default" ? "" : ` (${p.name})`;
    this.#source(`gateway's state${label}`, f("gateway_state.json"), gateway.state);
    this.#source(`scheduler's ticker${label}`, f("cron", "ticker_heartbeat"), ticker.state);
    this.#source(`background processes${label}`, f("processes.json"), processes.state);
    this.#source(`scheduled jobs${label}`, f("cron", "jobs.json"), jobs.state);

    // The session database, when it or its log changed.
    const dbFile = f("state.db");
    const stamp = stampOf(this.files, dbFile, `${dbFile}-wal`);
    let fresh = null;
    if (stamp !== p.db.stamp) {
      fresh = readStateDb(dbFile, { afterId: p.db.lastId, sinceMs: at - (p.db.read ? HISTORY_MS : REPLAY_MS), open: this.open, files: this.files });
      // A database that's busy just now is read again next time.
      if (fresh.state !== "busy") p.db.stamp = stamp;
      if (fresh.state === "ok") {
        p.db.lastId = fresh.lastId;
        p.db.read = true;
        p.db.heartbeats = fresh.heartbeats;
      }
      p.db.state = fresh.state === "busy" ? (p.db.state ?? "ok") : fresh.state;
    }
    this.#source(`session database${label}`, dbFile, p.db.state ?? "missing");
    const g = gateway.gateway;
    const t = ticker.ticker;
    return {
      gateway: g,
      gatewayAlive: !!g && RUNNING_GATEWAY.has(g.state) && this.#alive(g.pid, g.startTime),
      ticker: t,
      tickerAlive: !!t && this.#alive(t.pid) && at - (t.at ?? 0) < TICKER_STALE_MS,
      processes: processes.entries,
      jobs: jobs.jobs,
      fresh,
      heartbeats: p.db.heartbeats ?? [],
      dbState: p.db.state ?? "missing",
    };
  }

  // Whether a process Hermes recorded still runs (and is the one it
  // recorded, when it says when it started), from the last census.
  #alive(pid, startTime = null) {
    const p = pid ? this.table?.get(pid) : null;
    return !!p && (startTime == null || sameStart(startTime, p.start));
  }

  // Where a session works, as a place on the map: its repo's root, else its
  // folder.
  #placeOfSession(s) {
    if (!s) return null;
    for (const dir of [s.repo, s.cwd]) if (dir && path.isAbsolute(dir)) {
      const place = this.actors.placeOf(dir);
      if (place) return place;
    }
    return null;
  }

  // A path from a tool call, made absolute: Hermes takes them absolute,
  // relative to the session's folder, or under ~.
  #absolute(p, s) {
    if (typeof p !== "string" || !p || p.includes("\0")) return null;
    if (p === "~" || p.startsWith("~/")) return path.join(this.home, p.slice(1));
    if (path.isAbsolute(p)) return pathStyle(p).normalize(p);
    return s?.cwd && path.isAbsolute(s.cwd) ? pathStyle(s.cwd).resolve(s.cwd, p) : null;
  }

  // What a profile's database says, its presence and status, and the
  // events and targets they make.
  #observe(p, { gateway, gatewayAlive, tickerAlive, jobs, fresh, processes, apps, dbState, version, at, isInstalled }) {
    const there = processes.length > 0 || gatewayAlive || tickerAlive;
    // A named profile is an actor once something of it has run; the
    // default one is while Hermes is installed.
    if (!there && !apps.length && !this.actors.has(p.id) && !(p.name === "default" && isInstalled)) {
      if (fresh?.state === "ok") this.#take(p, fresh, at, { record: false });
      return;
    }
    const capabilities = ["run-commands", "edit-repos", "browse", "schedule"];
    if (this.#exists(path.join(p.home, "skills", "autonomous-ai-agents"))) capabilities.push("spawn-sessions");
    if (gateway?.platforms.some((x) => x.state === "connected")) capabilities.push("message-people");
    if (this.#cuaDriver()) capabilities.push("use-computer");
    // Installed: Hermes is, and so is the profile's folder.
    this.keep(p.id, { name: p.title, version, via: ["observed", "attributed"], heartbeatMs: HERMES_HEARTBEAT_MS, capabilities, installed: isInstalled && this.profileList.includes(p.name), at });
    if (!this.actors.enabled(p.id)) return;
    // What the database says that's new.
    if (fresh?.state === "ok") this.#take(p, fresh, at);

    // The sessions at work: open, not hidden, active in the last two
    // minutes.
    const active = [...p.sessions.values()].filter((s) => s.endedAt == null && !s.archived && !s.hidden && Math.max(s.activityAt ?? 0, s.lastMessageAt ?? 0) >= at - ACTIVE_MS).sort((a, b) => Math.max(b.activityAt ?? 0, b.lastMessageAt ?? 0) - Math.max(a.activityAt ?? 0, a.lastMessageAt ?? 0));
    // A gateway whose heartbeat is stale while its process runs has gone
    // quiet (hung, say): what it last wrote, the agents it had at work
    // included, no longer says what it's doing; only its sessions' own
    // activity does.
    const quietGateway = gatewayAlive && gateway.updatedAt != null && at - gateway.updatedAt > STALE_MS;
    const busyGateway = gatewayAlive && !quietGateway && gateway.activeAgents > 0;
    let presence;
    let status = null;
    if (there && (active.length || busyGateway)) {
      presence = "working";
      const top = active[0];
      if (top) {
        const words = (top.activityAt != null && top.activityAt >= at - ACTIVE_MS ? doingOf(top.activity) : null) ?? toolWords(p.lastTool.get(top.id)) ?? "Working";
        const place = this.actors.where(this.#placeOfSession(top))?.name;
        status = place ? `${words} in ${place}` : words;
      } else status = "Working through its gateway";
    } else if (quietGateway) {
      presence = "away";
      status = "Its gateway has gone quiet";
    } else if (there) {
      presence = "idle";
      status = gatewayAlive ? { starting: "Its gateway is starting", degraded: "Its gateway is degraded", draining: "Its gateway is finishing its work" }[gateway.state] ?? null : null;
    } else if (apps.length) {
      presence = "idle";
      status = "Hermes.app runs with no backend on this Mac";
    } else if (dbState === "denied" || dbState === "unreadable") {
      presence = "unknown";
      status = "XenoNet can't read its session database";
    } else {
      presence = "offline";
      status = gateway?.state === "startup_failed" ? "Its gateway failed to start" : null;
    }
    this.say(p.id, { presence, status, at });

    // Where it works: its sessions' places, and its jobs' folders.
    for (const s of active) this.touch(p.id, this.#placeOfSession(s), "works-on", at);
    for (const j of jobs) {
      if (j.enabled && j.workdir && path.isAbsolute(j.workdir)) this.touch(p.id, this.actors.placeOf(j.workdir), "watches", at);
      // A job that ran: finished or failed, once per run.
      if (j.lastRunAt == null || at - j.lastRunAt > DAY_MS) continue;
      const kind = j.status === "ok" ? "finished" : ["error", "delivery_failed", "blocked_config"].includes(j.status) ? "failed" : null;
      if (kind) this.happened(p.id, { ref: `job:${j.id}:${j.lastRunAt}`, kind, target: j.workdir && path.isAbsolute(j.workdir) ? this.actors.placeOf(j.workdir) : null, subject: "job", at: Math.min(j.lastRunAt, at) });
    }
  }

  // New rows of a profile's database: sessions kept current, its tools
  // gathered per minute, its replies on messaging platforms, sessions that
  // ended.
  #take(p, fresh, at, { record = true } = {}) {
    for (const s of fresh.sessions) {
      const was = p.sessions.get(s.id);
      p.sessions.set(s.id, {
        id: s.id,
        source: s.source ?? null,
        cwd: s.cwd ?? null,
        repo: s.git_repo_root ?? null,
        parent: s.parent_session_id ?? null,
        startedAt: msOf(s.started_at),
        endedAt: msOf(s.ended_at),
        endReason: s.end_reason ?? null,
        activityAt: msOf(s.last_activity_at),
        activity: s.last_activity_description ?? null,
        archived: !!s.archived,
        hidden: !!s.hidden,
        lastMessageAt: was?.lastMessageAt ?? null,
      });
    }
    const sessionOf = (id) => p.sessions.get(id) ?? null;
    for (const m of fresh.messages) {
      const s = sessionOf(m.session_id);
      const when = msOf(m.timestamp);
      if (!when) continue;
      if (s) s.lastMessageAt = Math.max(s.lastMessageAt ?? 0, when);
      if (m.role === "tool" && m.tool_name) {
        p.lastTool.set(m.session_id, m.tool_name);
        if (record && TOOL_KINDS[m.tool_name]) this.gather(p.id, { kind: TOOL_KINDS[m.tool_name], target: this.#placeOfSession(s), at: when });
      } else if (record && m.role === "assistant" && !m.calls && s && PLATFORMS[s.source]) this.gather(p.id, { kind: "messaged", platform: PLATFORMS[s.source], at: when });
    }
    if (!record) return;
    // Its edits: the files, each in a place XenoNet knows (the others are
    // left out, as a registered actor's are), else the session's place.
    for (const t of fresh.toolPaths) {
      const s = sessionOf(t.session_id);
      const when = msOf(t.timestamp);
      if (!when) continue;
      const file = this.#absolute(t.path, s);
      const place = file ? this.actors.placeOf(file) : null;
      this.gather(p.id, { kind: "edited", target: place ?? this.#placeOfSession(s), at: when, paths: place ? [file] : [] });
    }
    // Sessions that ended (not a subagent's, nor one that went on in
    // another), once each.
    for (const s of fresh.sessions) {
      const k = sessionOf(s.id);
      if (k?.endedAt == null || k.parent || k.source === "subagent" || at - k.endedAt > DAY_MS) continue;
      const kind = endingOf(k.endReason);
      if (kind) this.happened(p.id, { ref: `session:${k.id}:end`, kind, target: this.#placeOfSession(k), subject: "session", at: Math.min(k.endedAt, at) });
    }
    // Sessions long over are let go.
    for (const [id, s] of p.sessions) if (Math.max(s.endedAt ?? 0, s.activityAt ?? 0, s.lastMessageAt ?? 0, s.startedAt ?? 0) < at - DAY_MS) {
      p.sessions.delete(id);
      p.lastTool.delete(id);
    }
  }

  // The Claude Code and Codex sessions Hermes started.
  async #attribute({ census, hosts, profileOfHost, read, at }) {
    const actorOf = (profile) => this.#profile(profile).id;
    // Its background processes, which outlive the host that started them.
    const background = new Map();
    for (const [name, r] of read) for (const e of r.processes) if (this.#alive(e.pid, e.startTime)) background.set(e.pid, name);
    const hostIds = new Set(hosts.map((h) => h.pid));
    const started = startedUnder(census, (q) => hostIds.has(q.pid) || background.has(q.pid));
    const byActor = new Map();
    for (const s of started) {
      const id = actorOf(background.get(s.root) ?? profileOfHost(census.table.get(s.root)));
      if (!this.actors.has(id) || !this.actors.enabled(id)) continue;
      (byActor.get(id) ?? byActor.set(id, []).get(id)).push(s);
    }
    for (const [id, list] of byActor) await this.spawns.saw(id, list, at);
    // Which still run, by the same read of the table they were found in.
    for (const s of this.spawns.settle(at, { alive: (p) => census.table.get(p.pid)?.start === p.start })) {
      if (s.session) this.happened(s.actor, { ref: `spawned:${s.session}`, kind: "spawned", target: s.session, app: s.agent, via: "attributed", at: s.start });
      else this.happened(s.actor, { ref: `spawned:${s.agent}:${s.start}:${pathKey(s.cwd) ?? ""}`.slice(0, 200), kind: "spawned", target: s.cwd ? this.actors.placeOf(s.cwd) : null, app: s.agent, via: "attributed", at: s.start });
    }
    // Codex threads Hermes's own runtime started, and sessions in its codex
    // skill's scratch worktrees: the default profile's, which is all XenoNet
    // can tell.
    const id = actorOf("default");
    if (!this.actors.has(id) || !this.actors.enabled(id)) return;
    const scratch = pathKey(path.join(this.root, "cache", "scratch"));
    for (const s of this.actors.map?.sessions?.() ?? []) {
      if (s.createdAt == null || at - s.createdAt > DAY_MS) continue;
      const mine = (s.app === "codex" && s.originator === "hermes") || isUnder(pathKey(s.cwd), scratch) || isUnder(pathKey(s.worktree), scratch);
      if (mine) {
        this.spawns.claimed.add(s.id);
        this.happened(id, { ref: `spawned:${s.id}`, kind: "spawned", target: s.id, app: s.app, via: "attributed", at: s.createdAt });
      }
    }
  }

  #exists(p) {
    return this.#stamped(`exists:${p}`, [path.dirname(p)], () => this.files.existsSync(p));
  }

  // Whether Hermes's computer-use driver is installed among its tools.
  #cuaDriver() {
    const dir = path.join(this.root, "tools");
    return this.#stamped("cua-driver", [dir], () => {
      try {
        return this.files.readdirSync(dir).some((n) => n.startsWith("cua-driver"));
      } catch {
        return false;
      }
    });
  }

  // What the doctor shows of this observer: whether Hermes is installed,
  // each source XenoNet reads and whether it could, and the process check's
  // self-test. Whether it's installed is asked of its folder each time, its
  // observer on or off, so the doctor can tell an observer you turned off
  // from a product that isn't here; that's only whether two paths exist.
  status() {
    return { product: PRODUCT.id, name: PRODUCT.name, installed: installed(this.root, { files: this.files }), sources: [...this.sourceStates.values()], recognized: this.recognized };
  }
}
