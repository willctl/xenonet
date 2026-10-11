// Grok Bot's observer (docs/design/actors/grok-bot.md; interface.md,
// sections 5 and 7): presence and process ancestry, nothing more. Its
// desktop app is the Bots' only way onto this Mac, so:
//
//   working   a command its app runs is running, or ran in the last 30 s
//   idle      its app runs
//   unknown   its app isn't running: Bots go on in the cloud with it
//             closed, and nothing here can say they stopped (heartbeatMs
//             is null). So it's never offline, however long its app has
//             been closed, and it stays listed, out of sight, for as long
//             as it's installed (the owner's call in Wave 3, where other
//             installed products are listed as offline once they haven't
//             run for a week)
//
// A command is a process the app starts whose program lies outside its
// bundle (so not one of its helpers). Each counts as ran, gathered per
// minute in the project holding the folder it started in: by count and
// place alone, never its arguments, which can hold tokens or file
// contents. One that is claude or codex, or starts one, started a session
// (spawned, matched to the session as attribution.js does).
//
// It reads the process table, where its commands work, and of its files
// only whether the app is there, and two keys of its Info.plist: its
// version, CFBundleShortVersionString, and its bundle id,
// CFBundleIdentifier, since anyone can name a bundle Grok Bot.app (Cursor's
// is com.anysphere.sand). Never its data, logs, preferences or tokens.
// Grok Bot reports itself only once it registers, from a command its app
// runs or through the actor gateway (since 2026-10-08; at merge point A
// the owner had it observed alone). While the app runs, what it runs is
// looked at every 3 seconds with pgrep, a few milliseconds of CPU (the
// owner's decision at merge point B, against once a second); the whole
// table is read with the other observers', every 5 seconds.
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { Spawns } from "./attribution.js";
import { PRODUCTS } from "./products.js";
import { AppBundle } from "./app-bundle.js";
import { agentOf, wantsArgs } from "../procs.js";
import { pathKey } from "../paths.js";

const PRODUCT = PRODUCTS["grok-bot"];
const BUNDLE = "Grok Bot.app";
export const BUNDLE_ID = /^com\.anysphere\.sand(\.|$)/;
export const POLL_MS = 3000;
// It stays working this long after its last command, so a run of quick
// ones is one stretch of work.
const LINGER_MS = 30e3;
const TRACKED_MAX = 512;
const STRAY_MS = 60e3;
// What a Bot can do, as grok-bot.md has it: commands here, the rest in the
// cloud.
const CAPABILITIES = ["run-commands", "browse", "message-people", "message-agents", "schedule"];

const inBundle = (program) => String(program ?? "").includes(`/${BUNDLE}/`);
// The sessions it's watched for are Claude Code's and Codex's, as
// attribution has them (attribution.js); a grok it runs is a command like
// any other. Undefined, not yet known, stays so.
const attributed = (agent) => (agent === "claude" || agent === "codex" || agent === undefined ? agent : null);

// Whether a process still runs: signal 0 only checks.
const running = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
};

export class GrokBotObserver extends Observer {
  // `apps`: where the app may be installed. `alive`: whether a pid runs.
  // `binary`: how the version is read from an Info.plist written as binary.
  constructor({ actors, procs, home, apps = ["/Applications", path.join(home ?? "", "Applications")].map((d) => path.join(d, BUNDLE)), files = fs, alive = running, binary, pollMs = POLL_MS, observing, every, clock } = {}) {
    super({ name: "grok-bot-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.alive = alive;
    // Its app: whether it's there and Cursor's, and its version, from two
    // keys of its Info.plist (one, its version, at merge point B; its bundle
    // id since 2026-10-08, as Muse's).
    this.app = new AppBundle({ apps, id: BUNDLE_ID, files, ...(binary ? { binary } : {}) });
    this.pollMs = pollMs;
    this.id = `actor:${PRODUCT.slug}`;
    this.appPids = new Set();
    // The app's commands and what they run, by pid: { start, program, ppid,
    // command (the app started it), agent, handed (to attribution), place,
    // startedAt }.
    this.tracked = new Map();
    this.busyAt = null;
    this.wasInstalled = false;
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
    this.pollTimer = null;
    this.polling = null;
    this.tableState = "ok";
  }

  // A process below the app, seen for the first time: a command when the
  // app started it, and either way watched for a session it starts. Its
  // `agent` is claude, codex or null, or undefined while it isn't known: a
  // Node or Bun process the poll saw, whose arguments the next look's
  // census reads.
  #track(p, parentPid, agent, at) {
    const was = this.tracked.get(p.pid);
    if (was && was.start === p.start) return null;
    const entry = { start: p.start, program: p.program, ppid: parentPid, command: this.appPids.has(parentPid), agent, handed: false, place: null, startedAt: Math.min(p.start || at, at) };
    this.tracked.set(p.pid, entry);
    return entry;
  }

  // The new commands' places (one lsof for them all), each counted as
  // ran in its minute; and the sessions among `candidates` (the new ones,
  // unless the look says), each handed to attribution once.
  async #taken(fresh, at, candidates = fresh) {
    const commands = fresh.filter(([, e]) => e.command);
    if (commands.length) {
      const cwds = await this.procs.cwds(commands.map(([pid]) => pid));
      for (const [pid, e] of commands) {
        const cwd = cwds.get(pid) ?? null;
        e.place = cwd ? this.actors.placeOf(cwd) : null;
        e.cwd = cwd;
        this.busyAt = Math.max(this.busyAt ?? 0, at);
        if (this.actors.has(this.id)) this.gather(this.id, { kind: "ran", target: e.place, at: e.startedAt });
      }
    }
    const sessions = candidates.filter(([, e]) => e.agent && !e.handed && this.#above(e) === "clear");
    if (!sessions.length || !this.actors.has(this.id)) return;
    for (const [, e] of sessions) e.handed = true;
    await this.spawns.saw(this.id, sessions.map(([pid, e]) => ({ pid, start: e.start, agent: e.agent, ...(e.cwd !== undefined ? { cwd: e.cwd } : {}) })), at);
  }

  // What's above a tracked process, among what's tracked: a session
  // another tracked one started ("agent", and a session's own children are
  // that session's), a process whose agent isn't known yet ("unknown", so
  // it waits for the look to say), or neither ("clear").
  #above(e) {
    for (let q = this.tracked.get(e.ppid), n = 0; q && n < 32; q = this.tracked.get(q.ppid), n++) {
      if (q.agent) return "agent";
      if (q.agent === undefined) return "unknown";
    }
    return "clear";
  }

  async look(at = this.clock.now()) {
    const known = this.actors.has(this.id) && !this.actors.info(this.id)?.via.includes("demo");
    // Whether the app where apps go is Cursor's, from its Info.plist (read
    // again only when it changes).
    await this.app.read();
    // Not installed where apps go and not seen before: the process table
    // is read only once a minute, for an app run from somewhere else.
    if (!known && !this.app.installed()) {
      if (at - (this.lookedAt ?? -Infinity) < STRAY_MS) return;
      this.lookedAt = at;
    }
    let census;
    try {
      census = await this.procs.census();
      this.tableState = this.procs.supported === false ? "unsupported" : "ok";
    } catch (err) {
      this.tableState = "unreadable";
      throw err;
    }
    const { table, of } = census;
    const apps = [...table.values()].filter((p) => of(p).product?.product === PRODUCT.id && of(p).product.role === "app");
    const installed = apps.length > 0 || this.app.installed();
    this.wasInstalled = installed;
    if (!installed && !known) return;
    // The actor first, so the commands of its first look count.
    const version = this.app.ours() ? (this.app.plist?.version ?? null) : null;
    this.keep(this.id, { name: PRODUCT.name, version, via: ["observed", "attributed"], heartbeatMs: null, capabilities: CAPABILITIES, installed, at });
    if (!this.actors.enabled(this.id)) return;

    // The app's processes, and everything below them, from the table.
    this.appPids = new Set(apps.map((p) => p.pid));
    const kids = new Map();
    for (const p of table.values()) (kids.get(p.ppid) ?? kids.set(p.ppid, []).get(p.ppid)).push(p);
    const queue = [...apps];
    const seen = new Set(this.appPids);
    const fresh = [];
    // The sessions not yet handed to attribution, the new ones among them.
    const candidates = [];
    for (let i = 0; i < queue.length && i < 4096; i++) {
      const q = queue[i];
      for (const c of kids.get(q.pid) ?? []) {
        if (seen.has(c.pid)) continue;
        seen.add(c.pid);
        queue.push(c);
        if (inBundle(c.program)) {
          this.appPids.add(c.pid);
          continue;
        }
        const agent = attributed(of(c).agent);
        const e = this.#track(c, q.pid, agent, at);
        if (e) fresh.push([c.pid, e]);
        // One the poll saw first may be a Node or Bun process it couldn't
        // tell (Claude Code installed with npm is a Node script): the
        // census has read its arguments.
        const entry = e ?? this.tracked.get(c.pid);
        if (entry.agent === undefined) entry.agent = agent;
        if (entry.agent && !entry.handed) candidates.push([c.pid, entry]);
      }
    }
    // What's gone is let go: what the table no longer holds, or holds a
    // newer process at its pid. The table is shared for a moment
    // (procs.js), so it can be older than a command the poll saw start
    // since: one it doesn't hold goes only once it no longer runs, or the
    // next poll would find it again and count it twice.
    for (const [pid, e] of this.tracked) {
      const p = table.get(pid);
      if (p?.start === e.start || ((p == null || p.start < e.start) && this.alive(pid))) continue;
      this.tracked.delete(pid);
    }
    if (this.tracked.size > TRACKED_MAX) this.tracked = new Map([...this.tracked].slice(-TRACKED_MAX / 2));
    await this.#taken(fresh, at, candidates);
    this.#say(at, table);
  }

  // What it does now, in XenoNet's words, from what the app runs. `table`:
  // the look's process table, which says which of the processes waiting
  // for their session still run.
  #say(at, table) {
    const appRuns = this.appPids.size > 0;
    const live = [...this.tracked.values()].filter((e) => e.command);
    if (live.length) this.busyAt = Math.max(this.busyAt ?? 0, at);
    let presence;
    let status;
    if (appRuns && this.busyAt != null && at - this.busyAt < LINGER_MS) {
      presence = "working";
      const latest = live.sort((a, b) => b.startedAt - a.startedAt)[0];
      const place = this.actors.where(latest?.place ?? null)?.name;
      status = place ? `Running commands in ${place}` : "Running commands";
      if (latest?.place) this.touch(this.id, latest.place, "works-on", at);
    } else if (appRuns) [presence, status] = ["idle", "Its app is open on this Mac"];
    else [presence, status] = ["unknown", "Its app on this Mac is closed"];
    this.say(this.id, { presence, status, at });
    // One the table doesn't hold may have started since it was read (it's
    // shared for a moment): the system says whether it runs.
    const alive = (p) => {
      const row = table.get(p.pid);
      return row ? row.start === p.start : this.alive(p.pid);
    };
    for (const s of this.spawns.settle(at, { alive })) {
      if (s.session) this.happened(this.id, { ref: `spawned:${s.session}`, kind: "spawned", target: s.session, app: s.agent, via: "attributed", at: s.start });
      else this.happened(this.id, { ref: `spawned:${s.agent}:${s.start}:${pathKey(s.cwd) ?? ""}`.slice(0, 200), kind: "spawned", target: s.cwd ? this.actors.placeOf(s.cwd) : null, app: s.agent, via: "attributed", at: s.start });
    }
  }

  // Every 3 seconds while the app runs: its new children and theirs, by
  // pgrep, described and placed only when new. Commands that come and go
  // between two reads of the whole table are counted here, if they last
  // until a poll.
  async poll(at = this.clock.now()) {
    if (!this.observing() || !this.appPids.size || this.polling) return;
    this.polling = (async () => {
      for (const [pid] of this.tracked) if (!this.alive(pid)) this.tracked.delete(pid);
      const parents = [...this.appPids, ...this.tracked.keys()];
      const children = (await this.procs.children(parents)).filter((pid) => !this.appPids.has(pid) && !this.tracked.has(pid));
      if (!children.length) return;
      const described = await this.procs.describe(children);
      const fresh = [];
      // Parents before children, so a command's own children know it.
      for (const p of [...described.values()].sort((a, b) => a.start - b.start || a.pid - b.pid)) {
        if (!this.appPids.has(p.ppid) && !this.tracked.has(p.ppid)) continue;
        if (inBundle(p.program)) {
          this.appPids.add(p.pid);
          continue;
        }
        // Node and Bun run Claude Code and Codex too (an npm install), which
        // only their arguments say: the look's census reads those, and until
        // then such a process's agent isn't known.
        const e = this.#track(p, p.ppid, attributed(agentOf(p.program)) ?? (wantsArgs(p.program) ? undefined : null), at);
        if (e) fresh.push([p.pid, e]);
      }
      await this.#taken(fresh, at);
    })()
      .catch((err) => this.problem(err))
      .finally(() => (this.polling = null));
    return this.polling;
  }

  onStart() {
    super.onStart();
    const run = async () => {
      if (!this.started) return;
      await this.poll();
      if (!this.started) return;
      this.pollTimer = this.clock.setTimeout(run, this.pollMs);
      this.pollTimer?.unref?.();
    };
    this.pollTimer = this.clock.setTimeout(run, this.pollMs);
    this.pollTimer?.unref?.();
  }

  onStop() {
    super.onStop();
    this.clock.clearTimeout(this.pollTimer);
    this.pollTimer = null;
  }

  // What the doctor shows: whether the app is installed, and what this
  // observer reads: the processes, whether the app is there, and its
  // version. Whether it's installed is asked of where apps go each time,
  // its observer on or off, so the doctor can tell an observer you turned
  // off from an app that isn't here; one its last look saw running from
  // elsewhere counts too.
  status() {
    return {
      product: PRODUCT.id,
      name: PRODUCT.name,
      installed: this.app.installed() || this.wasInstalled,
      sources: [{ name: "processes", file: null, state: this.tableState }, ...this.app.sources(this.home)],
      recognized: null,
    };
  }
}
