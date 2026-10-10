// XenoNet's doctor: whether what XenoNet needs is in place, and how to fix it
// when it isn't. Sign-ins for Claude Code and Codex, the folders XenoNet
// reads, the apps and tools it talks to, its own server, whether your
// agents are connected to it, and actors: tokens left in repos or expired,
// what the observers read, and the process check they rely on.
//
// The xenonet command runs it (xenonet doctor). The server runs it every half
// hour and reports new problems in the console and as a notification
// (service.js). The MCP tools and the console's palette show the latest
// results.
//
// A check returns { status, detail, fix? }, where status is ok, info (a fact,
// not a problem), warn, fail or skip. Checks run in parallel, each with a time
// limit, and take everything they touch from `ctx`, so tests can stand in for
// the machine.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { commandDir, commandState, samePlace, savedPath } from "../command.js";
import { pathStyle, onPath, tilde } from "../paths.js";
import { selfTest } from "../procs.js";
import { inspectState } from "../statefile.js";
import { plural, duration } from "../units.js";
import { readDriftFile, describe, sourceName, ADDITIONS, DRIFT_FILE } from "../drift.js";
import { findTokenFiles } from "../actors/token-file.js";
import { keyChecks } from "../hosts/doctor.js";
import { appDir } from "../dirs.js";
import { LAUNCHER_SOURCE, launcherVersion } from "../components/install.js";
import { logDir, readLogs, summarize as byCode } from "../log.js";

export const STATUSES = ["ok", "info", "warn", "fail", "skip"];
const MIN_NODE = [22, 13];
const BLOCK_GRACE_MS = 60e3;

// The machine as the checks see it. `base` is a running XenoNet's address, or
// null; `health`, `live` and `connect` come from it, or from the server
// itself when it runs the doctor, which adds `stateLoad`, what its store
// found reading state.json (see statefile.js), `drift`, the drift its
// readers keep (see drift.js; without it, drift.json in the cache folder),
// `verify`, what its process check counted since it started (the state of
// core/engine/verify.js), `byHand`, whether you asked for this run
// yourself, and `actors`: the allowed actors whose tokens expired
// (expired()), what each observer reads (observers()), and its look for
// actors' tokens in repos (tokenFiles({ fresh }), createTokenScan in
// core/actors/token-file.js), or else the repos XenoNet knows (repos()).
export function machine(overrides = {}) {
  const home = overrides.home ?? os.homedir();
  const env = overrides.env ?? process.env;
  const platform = overrides.platform ?? process.platform;
  const cacheDir = overrides.paths?.cacheDir;
  return {
    env,
    platform,
    home,
    node: process.versions.node,
    electron: process.versions.electron ?? null,
    exists: (p) => fs.existsSync(p),
    readdir: (p) => fs.promises.readdir(p),
    access: (p, mode) => fs.promises.access(p, mode),
    statfs: (p) => fs.promises.statfs(p),
    which: (cmd) => onPath(cmd, { env }),
    run: (file, args, ms = 5000) => run(file, args, ms),
    command: () => commandState({ dir: commandDir({ home, platform, env }), env, platform }),
    savedPath: () => (platform === "win32" ? savedPath({ run: (file, args) => run(file, args, 5000), env }) : Promise.resolve(null)),
    procs: () => selfTest(),
    // What XenoNet's error log has from the last day (core/log.js).
    errors: () => (overrides.paths?.stateDir ? readLogs(logDir(overrides.paths.stateDir), { since: Date.now() - 24 * 3600e3, level: "error" }) : null),
    // The stable launcher hooks and MCP servers run, as it says it is
    // (launcher.cjs --launcher), or null when there's none here.
    async launcher() {
      const dir = path.join(env.XENONET_RUNTIME_DIR || path.join(appDir("state", { env, platform, homeDir: home }), "runtime"), "launcher");
      const node = path.join(dir, platform === "win32" ? "node.exe" : "node");
      const entry = path.join(dir, "cli", "xenonet.js");
      if (!fs.existsSync(node) || !fs.existsSync(entry)) return null;
      const r = await run(node, [entry, "--launcher"], 8000);
      let said = null;
      try {
        said = JSON.parse(r.stdout);
      } catch {}
      return { dir, said, version: launcherVersion(fs.readFileSync(entry, "utf8")), ships: launcherVersion(fs.readFileSync(LAUNCHER_SOURCE, "utf8")) };
    },
    stateFile: (dir) => inspectState(dir),
    drift: () => (cacheDir ? (readDriftFile(path.join(cacheDir, DRIFT_FILE)) ?? { file: path.join(cacheDir, DRIFT_FILE), lookedAt: null, records: [] }) : null),
    base: null,
    health: null,
    live: null,
    connect: null,
    ...overrides,
  };
}


function run(file, args, ms) {
  return new Promise((resolve) =>
    execFile(file, args, { timeout: ms, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) =>
      resolve({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout), stderr: String(stderr), timedOut: !!err?.killed }),
    ),
  );
}

const ok = (detail) => ({ status: "ok", detail });
const info = (detail, fix) => ({ status: "info", detail, ...(fix ? { fix } : {}) });
const warn = (detail, fix) => ({ status: "warn", detail, ...(fix ? { fix } : {}) });
const fail = (detail, fix) => ({ status: "fail", detail, ...(fix ? { fix } : {}) });
const skip = (detail) => ({ status: "skip", detail });

// A folder XenoNet reads: missing, unreadable (macOS privacy, or permissions)
// or readable with this many entries.
async function folder(ctx, dir) {
  if (!dir || !ctx.exists(dir)) return { state: "missing" };
  try {
    return { state: "ok", count: (await ctx.readdir(dir)).length };
  } catch (err) {
    return { state: "denied", code: err.code };
  }
}

// Why an actor check can't run: no XenoNet to ask (the xenonet command alone),
// or actors turned off there; null when it can.
const noActors = (ctx) => (ctx.actors === null ? skip("actors are off here (XENONET_ACTORS=off)") : !ctx.actors ? skip("needs XenoNet running") : null);

// The fix when one of the commands the process check reads answers in a
// way XenoNet doesn't read.
const unreadable = (tool) => `${tool} answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose.`;

const privacyFix = (ctx, what) =>
  ctx.platform === "darwin" ? `Let XenoNet read ${what}: System Settings › Privacy & Security › Files and Folders (or Full Disk Access), then restart XenoNet.` : `Check the permissions on ${what}.`;

// The sources of the map that xenonet.sources reports as failed: Agent Wiki
// and identities are extras, so theirs go without a word.
const OPTIONAL_SOURCES = new Set(["wiki", "identities"]);
const failedSources = (health) => (health?.sources ?? []).filter((s) => !s.ok && !OPTIONAL_SOURCES.has(s.name));

export const CHECKS = [
  // ---------- XenoNet ----------
  {
    id: "xenonet.server",
    group: "XenoNet",
    title: "XenoNet is running",
    async run(ctx) {
      if (ctx.self) return ok(`${ctx.self.name ?? "XenoNet"} on port ${ctx.self.port}`);
      if (!ctx.base) return warn("XenoNet isn't running, so the map, its alerts and the agents' tools are off", "Open XenoNet (or XenoNet Dev).");
      return ok(`${ctx.baseName ?? "XenoNet"} on ${ctx.base.replace(/^http:\/\//, "")}`);
    },
  },
  {
    id: "xenonet.sources",
    group: "XenoNet",
    title: "XenoNet can read what it shows",
    async run(ctx) {
      if (!ctx.health) return skip("needs XenoNet running");
      const bad = failedSources(ctx.health);
      const counts = Object.fromEntries(ctx.health.sources.filter((s) => s.ok).map((s) => [s.name, s.count]));
      if (bad.length) return fail(bad.map((s) => `${s.name}: ${s.error ?? "failed"}`).join("; "), "Run xenonet doctor again after fixing the checks below; if it stays, xenonet logs --errors says more.");
      const parts = [counts.projects != null && plural(counts.projects, "project"), counts.claude != null && plural(counts.claude, "Claude session"), counts.codex != null && plural(counts.codex, "Codex thread")].filter(Boolean);
      return ok(`reading ${parts.join(", ")}`);
    },
  },
  {
    id: "xenonet.blocks",
    group: "XenoNet",
    title: "XenoNet's parts are working",
    async run(ctx) {
      if (!ctx.health) return skip("needs XenoNet running");
      const blocks = ctx.health.blocks;
      if (!blocks) return skip("this XenoNet doesn't report its parts");
      // A part that failed has nothing to show; a degraded one shows what
      // it had, or less than all of it (see core/blocks/lifecycle.js). One
      // that reads for a source xenonet.sources reports as failed would only
      // say the same again, so it's left to that check.
      // The check can be shown alone (xenonet doctor --only xenonet.blocks), so
      // it names those sources rather than pointing at another check.
      const reported = new Set(failedSources(ctx.health).map((s) => s.name));
      const wrong = blocks.filter((b) => b.state === "failed" || b.state === "degraded");
      const bad = wrong.filter((b) => !reported.has(b.source)).sort((a, b) => (a.state === "failed" ? 0 : 1) - (b.state === "failed" ? 0 : 1));
      const left = [...new Set(wrong.filter((b) => reported.has(b.source)).map((b) => b.source))];
      if (!bad.length) return ok(`${plural(blocks.length, "part")}, none with a problem${left.length ? ` besides those reading for ${left.join(" and ")}, which XenoNet can't read` : ""}`);
      const say = (list) => list.map((b) => `${b.name} ${b.state === "failed" ? "isn't working" : "is partly working"}${b.lastError ? ` (${tilde(b.lastError.message, ctx.home)})` : ""}`).join("; ");
      // A source that missed one beat (a timeout, a file half written) is
      // fine again a few seconds later: only a part that's stayed that way
      // for a minute is worth a warning, and a notification.
      const lasting = bad.filter((b) => Date.now() - b.since >= BLOCK_GRACE_MS);
      if (!lasting.length) return info(`${say(bad)}, a moment ago; XenoNet tries again by itself`);
      return warn(say(lasting), "Most of these clear up once what they read is back, and xenonet doctor's other checks say what's missing. If one stays, restart XenoNet; xenonet logs --errors says more.");
    },
  },
  {
    id: "xenonet.state",
    group: "XenoNet",
    title: "XenoNet can save its state",
    async run(ctx) {
      const dir = ctx.paths?.stateDir;
      if (!dir) return skip("no state folder configured");
      if (!ctx.exists(dir)) return info(`${tilde(dir, ctx.home)} doesn't exist yet; XenoNet makes it on first save`);
      try {
        await ctx.access(dir, fs.constants.W_OK);
      } catch {
        return fail(`can't write to ${tilde(dir, ctx.home)}`, `Fix the folder's permissions, or set XENONET_STATE_DIR to a folder you own.`);
      }
      // The file itself: one XenoNet moved aside because it couldn't read it,
      // one it can't read now, one from a newer XenoNet, one it isn't saving
      // to, or parts of it that don't fit its schema.
      const file = await ctx.stateFile?.(dir).catch(() => null);
      const loaded = ctx.stateLoad ?? null;
      if (file?.broken.length) return warn(`${tilde(file.broken.at(-1), ctx.home)} is a state.json XenoNet couldn't read, so it moved it aside and started afresh`, "To get back what was in it, fix it and put it back as state.json while XenoNet is closed; delete it once you don't need it.");
      if (file?.unreadable) return warn(`state.json ${file.unreadable}, so XenoNet will move it aside and start afresh`, "To keep what's in it, fix it before XenoNet next starts.");
      if (file?.version > file?.known) return warn(`state.json is from a newer XenoNet (version ${file.version}), so this one reads it but doesn't save changes to it`, "Use the newer XenoNet, or update this one.");
      if (loaded?.readOnly) return warn(`${loaded.problem}; changes last only until XenoNet quits`, "Fix the file or its folder, then restart XenoNet.");
      if (file?.issues.length) {
        const [first] = file.issues;
        // A part's path can be a node id or a path key (layout's, your
        // rules'), with the home folder in it.
        return warn(`state.json has ${plural(file.issues.length, "part")} XenoNet can't use, such as ${tilde(first.path, ctx.home) || "the whole file"}: ${first.message.replace(/^Invalid input: /, "")}`, "Fix that part, or move state.json aside while XenoNet is closed to start afresh.");
      }
      try {
        const s = await ctx.statfs(dir);
        const free = s.bavail * s.bsize;
        if (free < 1e9) return warn(`${(free / 1e9).toFixed(1)} GB free where XenoNet keeps its state`, "Free some disk space; posters and replay clips need room too.");
      } catch {}
      return ok(tilde(dir, ctx.home));
    },
  },

  {
    id: "contracts.drift",
    group: "XenoNet",
    title: "The apps' data is the shape XenoNet expects",
    async run(ctx) {
      const d = await ctx.drift?.();
      if (!d) return skip("no cache folder configured");
      // What's new since you last looked (xenonet drift, or the console).
      const fresh = d.records.filter((r) => r.firstSeen > (d.lookedAt ?? -Infinity));
      if (!fresh.length) return ok(d.records.length ? `nothing new since you last looked (${plural(d.records.length, "difference")} known)` : "no differences from XenoNet's schemas");
      // A change to what XenoNet reads is a warning; something added (a field,
      // a kind of line) can't break it, and is a note.
      const changes = fresh.filter((r) => !ADDITIONS.has(r.kind));
      const shown = [...changes, ...fresh.filter((r) => ADDITIONS.has(r.kind))];
      const say = `${shown.slice(0, 2).map((r) => `${sourceName(r.source)}: ${describe(r)}`).join("; ")}${shown.length > 2 ? `; and ${shown.length - 2} more` : ""}`;
      const fix = `xenonet drift lists them by source and marks them seen. XenoNet keeps them in ${tilde(d.file ?? DRIFT_FILE, ctx.home)}.`;
      if (changes.length) return warn(`${plural(changes.length, "change")} to what XenoNet reads since you last looked: ${say}`, fix);
      return info(`the apps' data has ${plural(fresh.length, "addition")} since you last looked: ${say}`, fix);
    },
  },

  // ---------- Claude Code ----------
  {
    id: "claude.cli",
    group: "Claude Code",
    title: "Claude Code is installed",
    async run(ctx) {
      const bin = ctx.which("claude");
      if (!bin) return warn("the claude command isn't on your PATH: handing off to Claude and connecting agents need it", "Install Claude Code: https://claude.com/claude-code");
      const v = await ctx.run(bin, ["--version"], 4000);
      const version = (v.stdout.match(/\d+\.\d+\.\d+/) ?? [])[0];
      return ok(`${version ? `version ${version}, ` : ""}${tilde(bin, ctx.home)}`);
    },
  },
  {
    id: "claude.auth",
    group: "Claude Code",
    title: "Claude Code is signed in",
    async run(ctx) {
      const bin = ctx.which("claude");
      if (!bin) return skip("Claude Code isn't installed");
      const r = await ctx.run(bin, ["auth", "status"], 6000);
      if (r.timedOut) return warn("claude auth status didn't answer in 6 seconds");
      let s = null;
      try {
        s = JSON.parse(r.stdout);
      } catch {}
      if (s?.loggedIn) return ok(`signed in${s.authMethod ? ` with ${s.authMethod}` : ""}`);
      if (s && s.loggedIn === false) return fail("not signed in: XenoNet can't read Claude plan usage, and new sessions can't start", "Run claude and sign in with /login.");
      return warn("couldn't tell from claude auth status", "Run claude auth status to see why.");
    },
  },
  {
    id: "claude.transcripts",
    group: "Claude Code",
    title: "XenoNet can read Claude Code's sessions",
    async run(ctx) {
      const dir = ctx.paths?.claudeProjectsDir;
      const f = await folder(ctx, dir);
      if (f.state === "missing") return info(`no ${tilde(dir, ctx.home)} yet: no Claude Code sessions on this machine`);
      if (f.state === "denied") return fail(`can't read ${tilde(dir, ctx.home)} (${f.code})`, privacyFix(ctx, tilde(dir, ctx.home)));
      return ok(`${plural(f.count, "project")} of transcripts in ${tilde(dir, ctx.home)}`);
    },
  },
  {
    id: "claude.usage",
    group: "Claude Code",
    title: "Claude plan usage is current",
    async run(ctx) {
      if (!ctx.live) return skip("needs XenoNet running");
      const c = ctx.live.limits?.claude;
      if (!c) return info("no plan usage reading yet");
      if (!c.stale) return ok(`read ${c.source === "anthropic" ? "from Anthropic" : "from Claude desktop"}`);
      const since = c.observedAt ? `no reading since ${new Date(c.observedAt).toLocaleString("en-US", { weekday: "short", hour: "numeric", minute: "2-digit" })}` : "no reading yet";
      // A sign-in Claude Code will refresh is yours to nudge; waiting,
      // the network and a missing sign-in (claude.auth says so) aren't.
      if (/expired|refused/.test(c.reason ?? "")) return warn(`${c.reason}, so there's ${since}`, "Open Claude Code once; it refreshes its sign-in, and XenoNet reads it on its next try.");
      if (/asked to wait|couldn't reach|no Claude Code sign-in/.test(c.reason ?? "")) return info(`${c.reason}, so there's ${since}. XenoNet tries again by itself.`);
      return warn(c.reason ? `${c.reason}, so there's ${since}` : `there's ${since}`);
    },
  },

  // ---------- Codex ----------
  {
    id: "codex.cli",
    group: "Codex",
    title: "Codex is installed",
    async run(ctx) {
      const bins = [ctx.codexBin, ctx.which("codex")].filter(Boolean);
      const bin = bins.find((b) => path.isAbsolute(b) && ctx.exists(b)) ?? bins[0];
      if (!bin || (path.isAbsolute(bin) && !ctx.exists(bin)) || (!path.isAbsolute(bin) && !ctx.which(bin))) return warn("no Codex: neither the ChatGPT app's codex nor a codex command", "Install the ChatGPT desktop app or the Codex CLI.");
      const v = await ctx.run(bin, ["--version"], 4000);
      const version = (v.stdout.match(/\d+\.\d+\.\d+[\w.-]*/) ?? [])[0];
      return ok(`${version ? `version ${version}, ` : ""}${tilde(bin, ctx.home)}`);
    },
  },
  {
    id: "codex.auth",
    group: "Codex",
    title: "Codex is signed in",
    async run(ctx) {
      const bin = ctx.which("codex") ?? ctx.codexBin;
      if (!bin) return skip("Codex isn't installed");
      const r = await ctx.run(bin, ["login", "status"], 6000);
      const text = `${r.stdout}${r.stderr}`.trim();
      if (/logged in/i.test(text) && !/not logged in/i.test(text)) return ok(text.split("\n")[0].replace(/^Logged in/, "signed in"));
      if (/not logged in/i.test(text)) return fail("not signed in", "Run codex login.");
      return warn(text ? text.split("\n")[0] : "couldn't tell from codex login status");
    },
  },
  {
    id: "codex.sessions",
    group: "Codex",
    title: "XenoNet can read Codex's sessions",
    async run(ctx) {
      const dir = ctx.paths?.codexHome && pathStyle(ctx.paths.codexHome).join(ctx.paths.codexHome, "sessions");
      const f = await folder(ctx, dir);
      if (f.state === "missing") return info(`no ${tilde(dir, ctx.home)} yet: no Codex sessions on this machine`);
      if (f.state === "denied") return fail(`can't read ${tilde(dir, ctx.home)} (${f.code})`, privacyFix(ctx, tilde(dir, ctx.home)));
      return ok(`${tilde(dir, ctx.home)}`);
    },
  },

  // ---------- this machine ----------
  {
    id: "system.projects",
    group: "This computer",
    title: "XenoNet can read your projects folder",
    async run(ctx) {
      const dir = ctx.paths?.projectsDir;
      const f = await folder(ctx, dir);
      if (f.state === "missing") return fail(`${tilde(dir, ctx.home)} doesn't exist`, ctx.env?.XENONET_PROJECTS_DIR ? "Make it, or set XENONET_PROJECTS_DIR to the folder that holds your projects." : "Make it, or choose the folder that holds your projects: Change the projects folder… in XenoNet's palette, or xenonet root <folder>.");
      if (f.state === "denied") return fail(`can't read ${tilde(dir, ctx.home)} (${f.code})`, privacyFix(ctx, tilde(dir, ctx.home)));
      return ok(`${tilde(dir, ctx.home)}, ${f.count.toLocaleString("en-US")} ${f.count === 1 ? "entry" : "entries"}`);
    },
  },
  {
    id: "system.git",
    group: "This computer",
    title: "git is available",
    async run(ctx) {
      if (ctx.gitAvailable && !ctx.gitAvailable()) return warn("git isn't installed, so repo status (uncommitted work, branches) is off", ctx.platform === "darwin" ? "Run xcode-select --install, or install git with Homebrew, then restart XenoNet." : "Install git, then restart XenoNet.");
      const bin = ctx.which("git");
      return bin ? ok(tilde(bin, ctx.home)) : warn("git isn't on your PATH");
    },
  },
  {
    id: "system.command",
    group: "This computer",
    title: "The xenonet command is installed",
    async run(ctx) {
      const st = ctx.command();
      if (!st.file) return skip("not on Windows yet");
      const where = tilde(st.file, ctx.home);
      if (!st.installed) return info("not installed; once it is, xenonet doctor, xenonet agents and the rest work in a terminal", "Install it from the palette (Install the xenonet command), or run node cli/xenonet.js install in a checkout of XenoNet.");
      if (!st.ours) return info(`${where} is another program's`);
      if (st.broken) return warn(`${where} runs ${st.runs.map((f) => tilde(f, ctx.home)).join(" ")}, which is gone`, "Install it again from the palette (Install the xenonet command).");
      const dir = tilde(path.dirname(st.file), ctx.home);
      // On Windows, what this process was started with can be older than
      // the Path a new terminal gets, which is what the person will use.
      if (!st.onPath && ctx.platform === "win32" && (await ctx.savedPath?.())?.some((d) => samePlace(d, path.win32.dirname(st.file), "win32"))) return ok(`${where}, on your Path for new terminals`);
      if (!st.onPath) return warn(`${where} is there, but ${dir} isn't on your PATH`, ctx.platform === "win32" ? `Run XenoNet's Setup again, which adds it, or add ${dir} to your user Path in Windows' Environment Variables. Then open a new terminal.` : `Add it in your shell's profile: export PATH="$HOME/.local/bin:$PATH"`);
      return ok(where);
    },
  },
  {
    id: "system.runtime",
    group: "This computer",
    title: "The runtime is new enough",
    async run(ctx) {
      if (ctx.electron) return ok(`Electron ${ctx.electron} (Node ${ctx.node})`);
      const [major, minor] = ctx.node.split(".").map(Number);
      if (major < MIN_NODE[0] || (major === MIN_NODE[0] && minor < MIN_NODE[1])) return fail(`Node ${ctx.node}; XenoNet needs ${MIN_NODE.join(".")} or later`, "Update Node.js.");
      return ok(`Node ${ctx.node}`);
    },
  },

  {
    id: "xenonet.errors",
    group: "XenoNet",
    title: "No errors in XenoNet's log lately",
    async run(ctx) {
      const entries = ctx.errors?.();
      if (!entries) return skip("needs XenoNet's state folder");
      // The doctor's own findings are its other checks.
      const rows = byCode(entries.filter((e) => !String(e.code).startsWith("doctor.")));
      if (!rows.length) return ok("none in the last day");
      const n = rows.reduce((t, r) => t + r.count, 0);
      return warn(`${plural(n, "error")} in the last day: ${rows.slice(0, 3).map((r) => `${r.code}${r.count > 1 ? ` ×${r.count}` : ""}`).join(", ")}${rows.length > 3 ? `, and ${plural(rows.length - 3, "other kind")}` : ""}`, "xenonet logs --errors shows them, with a fix for each when there's one; xenonet diagnose --ask has an agent look at them with you.");
    },
  },
  {
    id: "xenonet.launcher",
    group: "XenoNet",
    title: "Hooks and MCP servers reach XenoNet through its launcher",
    async run(ctx) {
      const l = await ctx.launcher?.();
      if (!l) return skip("no launcher here: XenoNet runs from a checkout");
      // One older than this XenoNet's doesn't know --launcher yet.
      if (l.version < l.ships) return warn(`launcher ${l.version}; this XenoNet ships ${l.ships}`, "XenoNet replaces it the next time it starts.");
      if (!l.said) return fail(`the launcher in ${tilde(l.dir, ctx.home)} doesn't run`, "Open XenoNet, which writes it again, or install XenoNet again.");
      if (!l.said.tools) return fail("the launcher can't find XenoNet's Tools, so the xenonet command and MCP servers don't start (hooks still reach XenoNet)", "Open XenoNet to repair them, or install XenoNet again.");
      return ok(`launcher ${l.version}, running Tools ${l.said.tools} on Node ${String(l.said.node).replace(/^v/, "")}`);
    },
  },

  // ---------- agents ----------
  ...["claude", "codex"].map((app) => ({
    id: `agents.${app}`,
    group: "Agents",
    title: `${app === "claude" ? "Claude Code" : "Codex"} sessions are connected to XenoNet`,
    async run(ctx) {
      const a = ctx.connect?.apps.find((x) => x.app === app);
      if (!a) return skip("needs XenoNet running");
      const states = a.changes.map((c) => c.state);
      if (states.every((s) => s === "missing")) return info("not connected, so these sessions can't see or message each other through XenoNet", "Connect them from XenoNet's palette (Connect your agents to XenoNet), or run xenonet connect.");
      const stale = a.changes.filter((c) => c.state !== "present");
      if (stale.length) return warn(`partly connected (${stale.map((c) => `${c.id} ${c.state}`).join(", ")})`, "Run xenonet connect again.");
      if (a.broken?.length) return fail(`connected, but ${a.broken.join("; ")}`, "Run xenonet connect again.");
      return ok(`connected to XenoNet on port ${ctx.connect.port}`);
    },
  })),
  {
    id: "agents.checked",
    group: "Agents",
    title: "Agents can't pass for each other",
    async run(ctx) {
      if (ctx.platform !== "darwin") return info("XenoNet can't see yet which process sends a request on this system, so it takes each agent at its word");
      const t = await ctx.procs();
      // On macOS an agent's request goes through unchecked only when
      // netstat has no line for it at all, so agents' tools keep working;
      // one whose process ps can't trace is refused (core/engine/verify.js).
      if (!t.peer) return warn("netstat doesn't say which process sends a request, so XenoNet takes each agent at its word", unreadable("netstat"));
      if (!t.chain) return warn("ps doesn't say which processes a request comes from, so XenoNet refuses every request in an agent's name, and agents can't use its tools", unreadable("ps"));
      if (!ctx.exists("/usr/bin/codesign")) return warn("codesign is missing, so Claude Code's own hooks can't settle which process holds a session");
      return ok("each request in an agent's name is checked against the process that sent it");
    },
  },

  // ---------- actors (docs/design/actors/interface.md, section 7) ----------
  {
    id: "actors.repos",
    group: "Actors",
    title: "No actor's token is in a repo",
    async run(ctx) {
      // Agents read files freely in a repo, and files there get committed.
      const off = noActors(ctx);
      if (off) return off;
      // The server's look (tokenFiles) is kept for a day, and taken again
      // when you run the doctor yourself (the decision at merge point B);
      // without one, the doctor looks for itself.
      const started = Date.now();
      let found;
      if (ctx.actors.tokenFiles) found = await ctx.actors.tokenFiles({ fresh: ctx.byHand === true });
      else {
        const repos = ctx.actors.repos?.() ?? [];
        found = { at: null, repos: repos.length, ...(repos.length ? await findTokenFiles(repos, { run: ctx.run }) : { checked: 0, files: [] }) };
      }
      // A look from before this run says when it was.
      const when = found.at != null && found.at < started ? ` (XenoNet looked ${duration(started - found.at)} ago, as it does once a day and whenever you run the doctor)` : "";
      if (!found.repos) return ok(`no repos to look in${when}`);
      if (found.files.length) {
        const shown = found.files.slice(0, 3).map((f) => tilde(f, ctx.home)).join(", ");
        return warn(
          `${plural(found.files.length, "file")} in a repo ${found.files.length === 1 ? "holds" : "hold"} an actor's token: ${shown}${found.files.length > 3 ? ` and ${found.files.length - 3} more` : ""}${when}`,
          "Move each token out of the repo, whether it's in a file of its own or a line of a .env: keep it in a file only you can read, outside any repo (for example ~/.config/<name>/xenonet.token, named in XENONET_ACTOR_TOKEN_FILE), and give its actor a new token with New token in XenoNet's Actors panel, since agents may have read it. If it was committed, the old token has leaked.",
        );
      }
      if (found.checked < found.repos) return info(`none in the ${plural(found.checked, "repo")} looked in; the other ${found.repos - found.checked} didn't answer in time${when}`);
      return ok(`none in ${plural(found.repos, "repo")}${when}`);
    },
  },
  {
    id: "actors.expired",
    group: "Actors",
    title: "Registered actors' tokens are current",
    async run(ctx) {
      const off = noActors(ctx);
      if (off) return off;
      const expired = ctx.actors.expired?.() ?? [];
      if (!expired.length) return ok("no actor's token has expired");
      const names = expired.map((a) => `"${a.name}" (actor:${a.slug})`);
      const said = names.length > 2 ? `${names.slice(0, 2).join(", ")} and ${names.length - 2} more` : names.join(" and ");
      return warn(`${said} ${expired.length === 1 ? "has a token that" : "have tokens that"} expired after 30 days without a call, so ${expired.length === 1 ? "it stays" : "they stay"} offline`, "Give each a new token with New token in XenoNet's Actors panel, or remove the ones you don't use there or with xenonet actor remove <slug>.");
    },
  },
  {
    id: "actors.gateway",
    group: "Actors",
    title: "The actor gateway reaches the internet",
    async run(ctx) {
      const off = noActors(ctx);
      if (off) return off;
      const st = await ctx.actors.gateway?.();
      if (!st) return skip("the actor gateway isn't in this XenoNet");
      if (!st.on) return ok("it's off, so nothing outside this computer reaches XenoNet through it");
      const fix = "In XenoNet's Actors panel, Repair sets the tunnel up again and Change… picks another; xenonet gateway check checks again from a terminal.";
      if (st.problem) return warn(st.problem, fix);
      if (st.reach?.state !== "ok") return info(`it's on, at ${st.host ?? "no address yet"}; XenoNet hasn't checked that address yet`);
      // Each app's connection: an old address, a token about to stop
      // working, or calls with a token that no longer works.
      const words = { rotated: "replaced", removed: "taken away", expired: "expired" };
      const day = (t) => new Date(t).toISOString().slice(0, 10);
      const said = (st.actors ?? []).flatMap((a) => [
        a.oldAddress ? `${a.name} last called ${a.lastHost}, an older address` : null,
        a.refused ? `${a.name} calls with a gateway token that was ${words[a.refused.reason] ?? "refused"}` : null,
        a.token && a.expiring ? `${a.name}'s gateway token stops working on ${day(a.expiresAt)} unless it calls before then` : null,
      ]).filter(Boolean);
      if (said.length) return warn(said.join("; "), "In XenoNet's Actors panel, Connect… on the actor gives its app the address and a token again.");
      return ok(`reachable at ${st.url}${st.tunnel?.through ? ` through ${st.tunnel.through}` : ""}${st.reach.local ? ", checked on this computer alone" : ""}`);
    },
  },
  {
    id: "actors.wakes",
    group: "Actors",
    title: "Actors with a webhook can be woken",
    async run(ctx) {
      const off = noActors(ctx);
      if (off) return off;
      const list = await ctx.actors.wakes?.();
      if (!list) return skip("waking actors isn't in this XenoNet");
      const hooks = list.filter((w) => w.kind === "webhook");
      if (!hooks.length) return ok("no actor is woken by a webhook");
      const failing = hooks.filter((w) => w.last && !w.last.ok);
      // Without the webhook's host: anyone on this computer may read the
      // doctor (xenonet actor wake <slug> says it, to you).
      if (failing.length) return warn(failing.map((w) => `${w.actor}'s last wake failed: ${w.last.reason ?? "it didn't answer"}`).join("; "), "In XenoNet's Actors panel, Wake… on the actor sets its webhook again; xenonet actor wake <slug> test tries it now.");
      return ok(`${plural(hooks.length, "actor")} woken by a webhook, the last wake of each answered`);
    },
  },
  {
    id: "actors.observers",
    group: "Actors",
    title: "XenoNet can read what its observers look at",
    async run(ctx) {
      const off = noActors(ctx);
      if (off) return off;
      const list = ctx.actors.observers?.() ?? [];
      // Hermes Agent, Grok Bot and Muse.
      const names = (os) => (os.length > 1 ? `${os.slice(0, -1).map((o) => o.name).join(", ")} and ${os.at(-1).name}` : (os[0]?.name ?? ""));
      const installed = list.filter((o) => o.installed);
      if (!installed.length) return info(`${names(list) || "No product XenoNet observes"} ${list.length === 1 ? "isn't" : "aren't"} on this computer, so there's nothing to observe`);
      const on = installed.filter((o) => o.on);
      if (!on.length) return info(`the ${installed.length === 1 ? "observer" : "observers"} of ${names(installed)} ${installed.length === 1 ? "is" : "are"} off`, "Turn one on in XenoNet's Actors panel, or with xenonet actor observe <product> on.");
      const words = { denied: "can't be read", unreadable: "isn't in a shape XenoNet reads", unsupported: "can't be looked at on this system" };
      const bad = on.flatMap((o) => o.sources.filter((s) => words[s.state]).map((s) => `${o.name}'s ${s.name}${s.file ? ` (${s.file})` : ""} ${words[s.state]}`));
      if (bad.length) return warn(bad.join("; "), ctx.platform === "darwin" ? "A file that can't be read is usually macOS privacy or its permissions: check them, then restart XenoNet. One in another shape means the product changed it: update XenoNet, and npm run contracts in a checkout lists what changed." : "XenoNet observes these products on macOS for now.");
      return ok(on.map((o) => `${o.name}: ${plural(o.sources.filter((s) => s.state === "ok").length, "source")} read`).join("; "));
    },
  },
  {
    id: "actors.checked",
    group: "Actors",
    title: "Actors' tokens are held to their processes",
    async run(ctx) {
      if (ctx.actors === null) return skip("actors are off here (XENONET_ACTORS=off)");
      if (ctx.platform !== "darwin") return info("XenoNet can't see yet which process sends a request on this system, so an actor's token alone decides");
      const t = await ctx.procs();
      // On macOS a request whose process XenoNet can't see is refused, an
      // actor's and yours alike (core/engine/verify.js).
      const blind = !t.peer ? "netstat doesn't say which process sends a request" : !t.chain ? "ps doesn't say which processes a request comes from" : null;
      if (blind) return warn(`${blind}, so XenoNet refuses every actor's token, and what only you or its window may do, such as allowing or removing actors`, unreadable(!t.peer ? "netstat" : "ps"));
      // XenoNet's own window and Hermes's agent hosts are known by their
      // executables' real paths, which lsof reads.
      if (t.exe === false) return warn("lsof doesn't say which program a process runs, so XenoNet refuses Hermes's actor, and what only its window may do, such as allowing actors", unreadable("lsof"));
      // The observers' own self-test: a product's running processes, as its
      // own records list them, are ones the process check knows as its.
      const unknown = (ctx.actors?.observers?.() ?? []).filter((o) => o.on && o.recognized && o.recognized.found < o.recognized.expected);
      if (unknown.length) return warn(unknown.map((o) => `${o.name} runs ${o.recognized.purposes.join(", ")} processes XenoNet doesn't know as ${o.name}'s, so its actor's token would be refused from under them`).join("; "), "The product may have moved where it keeps its programs; update XenoNet, or report it.");
      const said = "an actor's token never works from under an agent or an actor app's own process, and a product's only from under that product";
      // The requests refused since XenoNet started because it couldn't see
      // which program sent them (verify.state.unseen).
      const unseen = ctx.verify?.()?.unseen ?? 0;
      if (unseen) return info(`${said}; since XenoNet started, it refused ${plural(unseen, "request")} because it couldn't see which program sent ${unseen === 1 ? "it" : "them"}`, "A program that sends a request and exits before the answer can't be seen; one that waits for it can. If this keeps growing, netstat or ps may be answering in a way XenoNet doesn't read: update XenoNet, or report it.");
      return ok(said);
    },
  },
];

// ---------- hosts ----------

// With hosts on (XENONET_HOSTS=1, docs/design/hosts.md), a check for each
// host this XenoNet shows: that it's reachable, that its relay answers in a
// protocol this XenoNet speaks, and that its clock is within two minutes of
// this one's. `hosts` is how each stands (the hub's list).
const SKEW_MS = 120e3;
const span = (ms) => {
  const s = Math.abs(ms) / 1000;
  return s < 90 ? `${Math.round(s)} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${Math.round(s / 3600)} h`;
};

export function hostCheck(h, now = Date.now()) {
  const fix = h.error?.fix;
  if (h.state === "error") return fail(h.error?.message ?? `${h.id} needs you`, fix);
  if (h.state === "offline") return warn(`offline${h.lastHeard ? `, last heard ${span(now - h.lastHeard)} ago` : ""}: ${h.error?.message ?? "the connection ended"}${h.cached ? "; the map shows its last snapshot" : ""}`, fix ?? "XenoNet tries again by itself.");
  if (h.state === "stale") return warn(`no word from it for ${span(now - (h.lastHeard ?? now))}`, "XenoNet waits a little longer, then connects again.");
  if (h.state === "connecting") return info(`connecting to ${h.target ?? h.id}`);
  const skew = h.offsetMs;
  const off = Number.isFinite(skew) && Math.abs(skew) >= 2000 ? `its clock ${span(skew)} ${skew > 0 ? "ahead of" : "behind"} this one's` : null;
  if (Number.isFinite(skew) && Math.abs(skew) > SKEW_MS) return warn(`live, but ${off}`, `Have ${h.id} set its clock by itself: on Linux, timedatectl set-ntp true; on a Mac, System Settings › General › Date & Time.`);
  return ok(`live: XenoNet ${h.version ?? "of an unknown version"} on ${h.platform ?? "an unknown system"}${h.demo ? ", with made-up data" : ""}${off ? `, ${off}` : ""}`);
}

export const hostChecks = (hosts) => (hosts ?? []).map((h) => ({ id: `hosts.${h.id}`, group: "Hosts", title: `${h.id} is connected`, run: async () => hostCheck(h) }));

// Runs every check (or those whose id starts with one of `only`), each with
// `timeoutMs`, and sums them up. With hosts on, those of each host, and the
// hosts' own check of them all (core/hosts/doctor.js).
export async function runDoctor(ctx = machine(), { only = null, timeoutMs = 8000 } = {}) {
  const started = Date.now();
  const hosts = ctx.hosts?.() ?? null;
  const all = [...CHECKS, ...hostChecks(hosts), ...keyChecks(hosts)];
  const checks = only ? all.filter((c) => only.some((o) => c.id === o || c.id.startsWith(`${o}.`))) : all;
  const results = await Promise.all(
    checks.map(async (c) => {
      const t0 = Date.now();
      let r, timer;
      try {
        r = await Promise.race([c.run(ctx), new Promise((resolve) => (timer = setTimeout(() => resolve(warn(`didn't finish in ${timeoutMs / 1000} seconds`)), timeoutMs)))]);
      } catch (err) {
        r = warn(`the check failed: ${err.message}`);
      } finally {
        clearTimeout(timer);
      }
      return { id: c.id, group: c.group, title: c.title, ...r, ms: Date.now() - t0 };
    }),
  );
  return { at: started, ms: Date.now() - started, results, summary: summarize(results) };
}

// How many results have each status.
export const summarize = (results) => Object.fromEntries(STATUSES.map((s) => [s, results.filter((r) => r.status === s).length]));

// The problems in `next` that weren't in `prev`: what's worth telling you about.
export function newProblems(prev, next) {
  const before = new Map((prev?.results ?? []).map((r) => [r.id, r.status]));
  const rank = { ok: 0, info: 0, skip: 0, warn: 1, fail: 2 };
  return next.results.filter((r) => rank[r.status] > 0 && rank[r.status] > (rank[before.get(r.id)] ?? 0));
}
