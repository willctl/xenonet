// XenoNet's doctor: whether what XenoNet needs is in place, and how to fix it
// when it isn't. Sign-ins for Claude Code, Codex and Grok Build, the folders
// XenoNet reads, the apps and tools it talks to, its own server, whether your
// agents are connected to it, actors: tokens left in repos or expired, what
// the observers read, and the process check they rely on; the service
// started at login (ctx.login(), the supervisor's own review of it, which
// only the Rust machines answer); and on the network's gateway, whether its
// firewall lets other machines in (ctx.network(), which only the service
// answers).
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
// the tokens it issued in repos (tokenFiles({ fresh }), createTokenScan in
// core/actors/token-file.js), or else the repos XenoNet knows (repos()),
// where any token counts, as nothing here can tell which XenoNet issued.
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
    // A small settings file's text, or null when there's none.
    readText: (p) => {
      try {
        return fs.readFileSync(p, "utf8");
      } catch {
        return null;
      }
    },
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
    // (--launcher), or null when there's none here: the xenonet binary, or
    // the node and launcher.cjs that an earlier XenoNet's entries run.
    async launcher() {
      const dir = path.join(env.XENONET_RUNTIME_DIR || path.join(appDir("state", { env, platform, homeDir: home }), "runtime"), "launcher");
      const ships = launcherVersion(fs.readFileSync(LAUNCHER_SOURCE, "utf8"));
      const bin = path.join(dir, platform === "win32" ? "xenonet.exe" : "xenonet");
      if (fs.existsSync(bin)) {
        const r = await run(bin, ["--launcher"], 8000);
        let said = null;
        try {
          said = JSON.parse(r.stdout);
        } catch {}
        // A copy of the Tools' own, so it's as new as they are.
        return { dir, said, version: said?.launcher ?? ships, ships };
      }
      const node = path.join(dir, platform === "win32" ? "node.exe" : "node");
      const entry = path.join(dir, "cli", "xenonet.js");
      if (!fs.existsSync(node) || !fs.existsSync(entry)) return null;
      const r = await run(node, [entry, "--launcher"], 8000);
      let said = null;
      try {
        said = JSON.parse(r.stdout);
      } catch {}
      return { dir, said, version: launcherVersion(fs.readFileSync(entry, "utf8")), ships };
    },
    stateFile: (dir) => inspectState(dir),
    // A file with its links followed, or null.
    realpath: (p) => {
      try {
        return fs.realpathSync.native(p);
      } catch {
        return null;
      }
    },
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

// Grok Build's folder (GROK_HOME's, or ~/.grok), a path in it, and its
// command: grok on PATH, or where its installer puts it, which a shell's
// PATH may not have yet.
const grokHome = (ctx) => ctx.paths?.grokHome ?? pathStyle(ctx.home).join(ctx.home, ".grok");
const inGrok = (ctx, ...parts) => pathStyle(grokHome(ctx)).join(grokHome(ctx), ...parts);
function grokBin(ctx) {
  const own = inGrok(ctx, "bin", ctx.platform === "win32" ? "grok.exe" : "grok");
  return ctx.which("grok") ?? (ctx.exists(own) ? own : null);
}
// The files where an organization can let Grok Build run only the hooks it
// sets (allow_managed_hooks_only).
const grokPolicies = (ctx) => ["/etc/grok/requirements.toml", "/etc/grok/managed_config.toml", inGrok(ctx, "requirements.toml"), inGrok(ctx, "managed_config.toml")];

// Codex's command: the ChatGPT app's codex (ctx.codexBin, when it's
// there) or a codex on PATH, or null when there's neither. That's Codex not
// installed, which is no problem: XenoNet shows its threads once it is.
function codexBin(ctx) {
  const bins = [ctx.codexBin, ctx.which("codex")].filter(Boolean);
  const bin = bins.find((b) => path.isAbsolute(b) && ctx.exists(b)) ?? bins[0];
  if (!bin || (path.isAbsolute(bin) && !ctx.exists(bin)) || (!path.isAbsolute(bin) && !ctx.which(bin))) return null;
  return bin;
}
// The map's sources for apps that aren't installed: what they can't read
// isn't a failure. Only Codex's runs a program of the app's own.
const absentSources = (ctx) => (codexBin(ctx) ? [] : ["codex"]);

// The sources of the map that xenonet.sources reports as failed: Xenolith
// and identities are extras, so theirs go without a word, and so do those
// of an app that isn't installed (`absent`).
const OPTIONAL_SOURCES = new Set(["wiki", "identities"]);
const failedSources = (health, absent = []) => (health?.sources ?? []).filter((s) => !s.ok && !OPTIONAL_SOURCES.has(s.name) && !absent.includes(s.name));

export const CHECKS = [
  // ---------- XenoNet ----------
  {
    id: "xenonet.server",
    group: "XenoNet",
    title: "XenoNet is running",
    async run(ctx) {
      if (ctx.self) return ok(`${ctx.self.name ?? "XenoNet"} on port ${ctx.self.port}`);
      if (!ctx.base) return warn("XenoNet isn't running, so the map, its alerts and the tools sessions use are off", "Open XenoNet (or XenoNet Dev).");
      return ok(`${ctx.baseName ?? "XenoNet"} on ${ctx.base.replace(/^http:\/\//, "")}`);
    },
  },
  {
    id: "xenonet.sources",
    group: "XenoNet",
    title: "XenoNet can read what it shows",
    async run(ctx) {
      if (!ctx.health) return skip("needs XenoNet running");
      const absent = absentSources(ctx);
      const bad = failedSources(ctx.health, absent);
      const counts = Object.fromEntries(ctx.health.sources.filter((s) => s.ok && !absent.includes(s.name)).map((s) => [s.name, s.count]));
      if (bad.length) return fail(bad.map((s) => `${s.name}: ${s.error ?? "failed"}`).join("; "), "Run xenonet doctor again after fixing the checks below; if it stays, xenonet logs --errors says more.");
      const parts = [counts.projects != null && plural(counts.projects, "project"), counts.claude != null && plural(counts.claude, "Claude session"), counts.codex != null && plural(counts.codex, "Codex thread"), counts.grok ? plural(counts.grok, "Grok Build session") : null].filter(Boolean);
      return ok(`reading ${parts.join(", ")}${absent.includes("codex") ? "; Codex isn't installed" : ""}`);
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
      // it names those sources rather than pointing at another check. One
      // that reads for an app that isn't installed has nothing to read.
      const absent = absentSources(ctx);
      const reported = new Set(failedSources(ctx.health, absent).map((s) => s.name));
      const wrong = blocks.filter((b) => b.state === "failed" || b.state === "degraded");
      const bad = wrong.filter((b) => !reported.has(b.source) && !absent.includes(b.source)).sort((a, b) => (a.state === "failed" ? 0 : 1) - (b.state === "failed" ? 0 : 1));
      const left = [...new Set(wrong.filter((b) => reported.has(b.source)).map((b) => b.source))];
      const besides = [left.length ? `those reading for ${left.join(" and ")}, which XenoNet can't read` : null, wrong.some((b) => absent.includes(b.source)) ? "those reading for Codex, which isn't installed" : null].filter(Boolean);
      if (!bad.length) return ok(`${plural(blocks.length, "part")}, none with a problem${besides.length ? ` besides ${besides.join(", and ")}` : ""}`);
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
      if (!bin) return warn("the claude command isn't on your PATH, and handing off to Claude and connecting Claude Code need it", "Install Claude Code: https://claude.com/claude-code");
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
      const bin = codexBin(ctx);
      if (!bin) return info("not installed; XenoNet shows Codex's threads once it is", "To use Codex, install the ChatGPT desktop app or the Codex CLI.");
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
      const bin = ctx.which("codex") ?? codexBin(ctx);
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

  // ---------- Grok Build ----------
  {
    id: "grok.cli",
    group: "Grok Build",
    title: "Grok Build is installed",
    async run(ctx) {
      const bin = grokBin(ctx);
      if (!bin) return info("not installed; XenoNet shows Grok Build's sessions once it is", "To use Grok Build, install it (x.ai/cli), then run grok login.");
      const v = await ctx.run(bin, ["--version"], 4000);
      const version = (v.stdout.match(/\d+\.\d+\.\d+[\w.-]*/) ?? [])[0];
      return ok(`${version ? `version ${version}, ` : ""}${tilde(bin, ctx.home)}`);
    },
  },
  {
    id: "grok.auth",
    group: "Grok Build",
    title: "Grok Build is signed in",
    async run(ctx) {
      if (!grokBin(ctx)) return skip("Grok Build isn't installed");
      // Whether its sign-in is there, never what's in it: auth.json holds
      // the tokens.
      if (ctx.exists(inGrok(ctx, "auth.json"))) return ok("signed in");
      if (ctx.env?.XAI_API_KEY) return ok("with an API key (XAI_API_KEY)");
      return info("not signed in yet", "Run grok login.");
    },
  },
  {
    id: "grok.sessions",
    group: "Grok Build",
    title: "XenoNet can read Grok Build's sessions",
    async run(ctx) {
      const dir = inGrok(ctx, "sessions");
      const f = await folder(ctx, dir);
      if (f.state === "missing") return info(`no ${tilde(dir, ctx.home)} yet: no Grok Build sessions on this machine`);
      if (f.state === "denied") return fail(`can't read ${tilde(dir, ctx.home)} (${f.code})`, privacyFix(ctx, tilde(dir, ctx.home)));
      return ok(`${plural(f.count, "folder")} of sessions in ${tilde(dir, ctx.home)}`);
    },
  },
  {
    id: "grok.hooks",
    group: "Grok Build",
    title: "Grok Build runs XenoNet's hooks",
    async run(ctx) {
      const a = ctx.connect?.apps.find((x) => x.app === "grok");
      if (!a) return skip("needs XenoNet running");
      if (a.changes.find((c) => c.id === "hooks")?.state !== "present") return skip("XenoNet's hooks aren't in Grok Build's settings");
      for (const file of grokPolicies(ctx)) {
        if (/^\s*(allow_managed_hooks_only|allowManagedHooksOnly)\s*=\s*true\b/m.test(ctx.readText?.(file) ?? "")) return warn(`Grok Build runs only the hooks your organization sets (${tilde(file, ctx.home)}), so XenoNet's don't run`, "Ask whoever manages Grok Build here to allow them. XenoNet's tools still work without them.");
      }
      // Hooks you turned off in Grok's /hooks.
      if (/xenonet/i.test(ctx.readText?.(inGrok(ctx, "disabled-hooks")) ?? "")) return warn("XenoNet's hooks are turned off in Grok Build", "Run /hooks in Grok Build, select XenoNet's hooks and press Space to turn them on.");
      return ok(`in ${tilde(inGrok(ctx, "hooks"), ctx.home)}, which Grok Build runs without asking`);
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
      return warn(`${plural(n, "error")} in the last day: ${rows.slice(0, 3).map((r) => `${r.code}${r.count > 1 ? ` ×${r.count}` : ""}`).join(", ")}${rows.length > 3 ? `, and ${plural(rows.length - 3, "other kind")}` : ""}`, "xenonet logs --errors shows them, with a fix for each when there's one. xenonet diagnose --ask has a Codex or Claude Code session look at them with you.");
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
      return ok(`launcher ${l.version}, running Tools ${l.said.tools}${l.said.node ? ` on Node ${String(l.said.node).replace(/^v/, "")}` : ""}`);
    },
  },

  // ---------- agents ----------
  ...[
    ["claude", "Claude Code"],
    ["codex", "Codex"],
    ["grok", "Grok Build"],
  ].map(([app, name]) => ({
    id: `agents.${app}`,
    group: "Sessions",
    title: `${name} sessions are connected to XenoNet`,
    async run(ctx) {
      const a = ctx.connect?.apps.find((x) => x.app === app);
      if (!a) return skip("needs XenoNet running");
      const states = a.changes.map((c) => c.state);
      if (a.installed === false && states.every((s) => s === "missing")) return skip(`${name} isn't installed`);
      if (states.every((s) => s === "missing")) return info("not connected, so these sessions can't see or message each other through XenoNet", "Connect them from XenoNet's palette (Connect your agents to XenoNet), or run xenonet connect.");
      // XenoNet's plugin for Claude Code carries what connect would add;
      // connect's own entries beside it are to come out.
      if (states.includes("remove")) {
        const tools = a.changes.some((c) => c.id === "mcp" && c.state === "remove");
        return warn(`connected twice, through XenoNet's plugin and through xenonet connect's own entries: each event still arrives once${tools ? ", but sessions get XenoNet's tools twice" : ""}`, "Run xenonet connect claude, which takes connect's own entries out and keeps the plugin.");
      }
      const plugin = a.changes.some((c) => c.via === "plugin");
      const stale = a.changes.filter((c) => c.state !== "present");
      if (stale.length) return warn(`partly connected (${stale.map((c) => `${c.id} ${c.state}`).join(", ")})`, "Run xenonet connect again.");
      if (a.broken?.length) return fail(`connected, but ${a.broken.join("; ")}`, plugin ? "Install XenoNet, or put its xenonet command on your PATH." : "Run xenonet connect again.");
      return ok(`connected to XenoNet on port ${ctx.connect.port}${plugin ? ", through its plugin for Claude Code" : ""}`);
    },
  })),
  {
    id: "agents.checked",
    group: "Sessions",
    title: "Sessions can't pass for each other",
    async run(ctx) {
      if (ctx.platform !== "darwin") return info("XenoNet doesn't check on this system yet that a request in a session's name comes from that session's process, so it takes each session at its word; what's yours to do is still refused from under an agent");
      const t = await ctx.procs();
      // On macOS an agent's request goes through unchecked only when
      // netstat has no line for it at all, so agents' tools keep working;
      // one whose process ps can't trace is refused (core/engine/verify.js).
      if (!t.peer) return warn("netstat doesn't say which process sends a request, so XenoNet takes each session at its word", unreadable("netstat"));
      if (!t.chain) return warn("ps doesn't say which processes a request comes from, so XenoNet refuses every request in a session's name, and sessions can't use its tools", unreadable("ps"));
      if (!ctx.exists("/usr/bin/codesign")) return warn("codesign is missing, so Claude Code's own hooks can't settle which process holds a session");
      return ok("XenoNet checks each request in a session's name against the process that sent it");
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
        const one = found.files.length === 1;
        // Only a token XenoNet issued counts. When it couldn't read its own
        // tokens, every token-shaped string did, a made-up one too.
        const what = found.unchecked ? "what looks like an actor's token" : "an actor's token";
        const unchecked = found.unchecked ? `; XenoNet couldn't check ${one ? "it" : "them"} against the tokens it issued` : "";
        return warn(
          `${plural(found.files.length, "file")} in a repo ${one ? "holds" : "hold"} ${what}: ${shown}${found.files.length > 3 ? ` and ${found.files.length - 3} more` : ""}${when}${unchecked}`,
          "Move each token out of the repo, whether it's in a file of its own or a line of a .env. Keep it in a file only you can read, outside any repo (for example ~/.config/<name>/xenonet.token, named in XENONET_ACTOR_TOKEN_FILE). Give its actor a new token with New token in XenoNet's Actors panel, since a session may have read it. If it was committed, the old token has leaked.",
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
      // Funnel publishes /mcp alone: any other path it puts on the
      // internet is a warning.
      const more = st.tunnel?.kind === "tailscale" ? (st.tunnel.public ?? []).filter((p) => p !== "/mcp") : [];
      if (more.length) return warn(`Tailscale Funnel puts more than the gateway's /mcp on the internet: ${st.tunnel.public.join(", ")}`, "In XenoNet's Actors panel, Repair sets Funnel back to /mcp alone. A path another program serves on port 443 has to move first.");
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
      // What's public: Funnel's /mcp alone, or a port tunnel's everything.
      const kind = st.tunnel?.kind;
      const pub = kind === "tailscale" ? (Array.isArray(st.tunnel.public) ? ", with /mcp alone public" : "") : ["cloudflare", "cloudflare-quick", "ngrok"].includes(kind) ? ", with the gateway's whole port public" : "";
      return ok(`reachable at ${st.url}${st.tunnel?.through ? ` through ${st.tunnel.through}` : ""}${pub}${st.reach.local ? ", checked on this computer alone" : ""}`);
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
      if (ctx.platform !== "darwin") return info("XenoNet doesn't hold actors' tokens to their processes on this system yet, so an actor's token alone decides");
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
      const said = "an actor's token never works from under a session or an actor app's own process, and a product's only from under that product";
      // The requests refused since XenoNet started because it couldn't see
      // which program sent them (verify.state.unseen).
      const unseen = ctx.verify?.()?.unseen ?? 0;
      if (unseen) return info(`${said}; since XenoNet started, it refused ${plural(unseen, "request")} because it couldn't see which program sent ${unseen === 1 ? "it" : "them"}`, "A program that sends a request and exits before the answer can't be seen; one that waits for it can. If this keeps growing, netstat or ps may be answering in a way XenoNet doesn't read: update XenoNet, or report it.");
      return ok(said);
    },
  },

  // ---------- the service started at login (docs/design/always-on.md) ----------
  {
    id: "service.login",
    group: "Background service",
    title: "The service starts at login",
    async run(ctx) {
      const { l, why } = await registration(ctx);
      if (why !== NOT_REGISTERED && !l) return skip(why);
      if (!l) return info("off: Desktop starts the service when it opens, and it runs until you log out", "To keep sessions coordinating while Desktop is closed, turn on Background service in XenoNet's palette, or run xenonet service install.");
      const other = ctx.self && l.port != null && String(l.port) !== String(ctx.self.port) ? `, for the service on port ${l.port}` : "";
      return ok(`on, through ${l.supervisor}: ${tilde(l.file, ctx.home)}${other}`);
    },
  },
  {
    id: "service.file",
    group: "Background service",
    title: "The service's registration is current",
    async run(ctx) {
      const { l, why } = await registration(ctx);
      if (!l) return skip(why);
      const file = tilde(l.file, ctx.home);
      if (l.programFound === false) return fail(`${file} runs ${tilde(l.program ?? "", ctx.home)}, which isn't there`, "Run xenonet service install again, which points it at XenoNet's launcher.");
      if (l.current === false) return warn(`${file} isn't what xenonet service install writes now, so the service may not start the way this XenoNet expects`, "Run xenonet service install again, which writes it afresh and reloads it. That restarts the service, so close its terminals first.");
      if (l.current !== true) return info(`XenoNet couldn't tell whether ${file} is current: ${l.expected ?? "it couldn't work out what install writes here"}`);
      return ok(`${file} is what xenonet service install writes`);
    },
  },
  {
    id: "service.loaded",
    group: "Background service",
    title: "The system has the service loaded",
    async run(ctx) {
      const { l, why } = await registration(ctx);
      if (!l) return skip(why);
      const by = l.supervisor;
      if (l.error) return warn(`XenoNet couldn't ask ${by} about it: ${l.error}`, LOOK[by]);
      if (l.state === "disabled") return warn("it's turned off in Task Scheduler, so it doesn't start when you sign in", "Turn XenoNet Service on in Task Scheduler, or run xenonet service install, which turns it on again.");
      if (!l.loaded) return warn(`${by} doesn't have it, so it doesn't start at login`, UNLOADED[by] ?? "Run xenonet service install again.");
      const exited = l.lastExit && !["0", "(never exited)"].includes(String(l.lastExit)) ? ` (it last exited with ${l.lastExit})` : "";
      if (l.state === "failed") return warn(`${by} gave up on it after it kept stopping${exited}`, "xenonet logs --errors says why it stopped. Then xenonet service start starts it again.");
      if (l.pid) return ok(`${by} has it, running as process ${l.pid}`);
      return info(`${by} has it, but it isn't running now${exited}`, "xenonet service start starts it. If it didn't stop on purpose, xenonet logs --errors says why.");
    },
  },
  {
    id: "service.boot",
    group: "Background service",
    title: "The service starts at boot",
    async run(ctx) {
      if (ctx.platform !== "linux") return skip("for Linux, where lingering starts a user's services at boot");
      const { l, why } = await registration(ctx);
      if (!l) return skip(why);
      const user = l.user || "$USER";
      if (l.linger === true) return ok("lingering is on, so it starts at boot and keeps running after you log out");
      if (l.linger !== false) return info("XenoNet couldn't ask loginctl whether lingering is on", `loginctl show-user ${user} --property=Linger says.`);
      if (l.graphical === false) return warn("lingering is off, and you have no graphical session here, as on a host reached over SSH, so the service runs only while you're logged in", `Run loginctl enable-linger ${user} once, which starts it at boot and keeps it running. It may need an administrator: sudo loginctl enable-linger ${user}.`);
      return info("lingering is off: it starts when you log in, and stops when you log out", `To start it at boot instead, before anyone logs in, as a host needs, run loginctl enable-linger ${user} once. It may need an administrator.`);
    },
  },

  // ---------- the network's gateway (docs/guide/network.md) ----------
  {
    id: "network.firewall",
    group: "Network",
    title: "Other machines can reach this gateway",
    run: (ctx) => firewall(ctx),
  },
];

// On a LAN or tailnet address other machines connect to the network's
// gateway straight, and the firewalls of macOS and Windows let a program
// accept them by its path. On a Mac, socketfilterfw says whether the
// firewall is on, blocks everything, or lets the service's program in;
// --getappblocked says "permitted" of any path, even one that isn't there,
// so the list of programs decides. On Windows, PowerShell says which of
// Windows Defender Firewall's profiles are on, which this computer's
// networks use, and the inbound rules for the service's program. It reads
// ctx.network() (this machine's part in its network), the service's own
// program (self.exe) and the one a restart runs (ctx.restartExe()), which
// only the service's machine answers, and ctx.realpath().
const SOCKETFILTERFW = "/usr/libexec/ApplicationFirewall/socketfilterfw";

// --getglobalstate's State: 0 off, 1 on, 2 on and blocking every incoming
// connection; null when it doesn't say.
const globalState = (text) => {
  const m = /State\s*=\s*([0-9]+)/.exec(text);
  return m ? Number(m[1]) : null;
};

// The programs --listapps lists, each with whether it may accept incoming
// connections: after "Total number of apps = N", each is "N : <path>", and
// "(Allow incoming connections)" or "(Block incoming connections)" after
// it. Null when it isn't that.
function listed(text) {
  if (!/Total number of apps\s*=\s*[0-9]+/.test(text)) return null;
  const out = [];
  let name = null;
  for (const raw of text.split(/\r?\n/)) {
    const entry = /^\s*[0-9]+\s*:\s(.*)$/.exec(raw);
    const line = entry ? entry[1] : raw;
    const verdict = /\(\s*(Allow|Block) incoming connections\s*\)/.exec(line);
    if (entry) name = line.slice(0, verdict ? verdict.index : line.length).trimEnd();
    if (name !== null && verdict) {
      out.push([name, verdict[1] === "Allow"]);
      name = null;
    }
  }
  return out;
}

// A path for Terminal: under the home folder as "$HOME/…", which keeps the
// account's name off the page; anywhere else in single quotes.
function shellPath(p, home) {
  const rest = home && p.startsWith(home) ? p.slice(home.length) : null;
  if (rest?.startsWith("/")) return `"$HOME${rest.replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("$", "\\$").replaceAll("`", "\\`")}"`;
  return `'${p.replaceAll("'", "'\\''")}'`;
}

// How to let `paths` in on a Mac: the prompt macOS shows once, unless one
// was blocked, which it doesn't ask about again; the commands; or System
// Settings.
function allowFix(paths, blocked, home) {
  const commands = paths.map((p) => {
    const q = shellPath(p, home);
    return `sudo ${SOCKETFILTERFW} --add ${q} then sudo ${SOCKETFILTERFW} --unblockapp ${q}`;
  });
  return `${blocked ? "In Terminal: " : "macOS asks once whether xenonet may accept incoming connections: choose Allow. Or in Terminal: "}${commands.join(", then ")}. Or System Settings › Network › Firewall › Options.`;
}

// A program's standing in the list: true let in, false blocked, null not
// listed. The firewall keeps the path it was given, so one listed under
// another spelling of the same file counts too.
async function listing(ctx, apps, file) {
  const exact = apps.find(([p]) => p === file);
  if (exact) return exact[1];
  const name = file.split("/").pop();
  for (const [p, allow] of apps.filter(([q]) => q.split("/").pop() === name)) {
    const real = await ctx.realpath?.(p);
    if (real != null && String(real) === file) return allow;
  }
  return null;
}

// Windows Defender Firewall, read through PowerShell's NetSecurity cmdlets,
// whose answers don't change with the system's language: each profile's
// state, the profiles of the networks this computer is on, and the inbound
// rules for each of the service's programs. Only read: XenoNet never changes
// a rule.
const POWERSHELL = "powershell.exe";
const PROFILES = { Public: "Public", Private: "Private", DomainAuthenticated: "Domain" };
const psQuote = (s) => `'${String(s).replaceAll("'", "''")}'`;
function windowsFirewallScript(paths) {
  return [
    "$ErrorActionPreference = 'Stop'",
    `$paths = @(${paths.map(psQuote).join(", ")})`,
    "$profiles = @(Get-NetFirewallProfile | ForEach-Object { [pscustomobject]@{ name = [string]$_.Name; enabled = [string]$_.Enabled; inbound = [string]$_.DefaultInboundAction; rules = [string]$_.AllowInboundRules } })",
    "$networks = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object { [string]$_.NetworkCategory })",
    "$rules = @(Get-NetFirewallApplicationFilter | ForEach-Object { $p = if ($_.Program) { [Environment]::ExpandEnvironmentVariables($_.Program) } else { '' }; foreach ($want in $paths) { if ($p -ieq $want) { $f = $_; Get-NetFirewallRule -AssociatedNetFirewallApplicationFilter $f | Where-Object { [string]$_.Direction -eq 'Inbound' } | ForEach-Object { [pscustomobject]@{ program = $want; name = [string]$_.DisplayName; enabled = [string]$_.Enabled; action = [string]$_.Action; profile = [string]$_.Profile } } } } })",
    "[pscustomobject]@{ profiles = $profiles; networks = $networks; rules = $rules } | ConvertTo-Json -Depth 4 -Compress",
  ].join("\n");
}

// The netsh line that lets a program in, run as administrator.
const netshAllow = (exe, profiles) => `netsh advfirewall firewall add rule name="XenoNet Service" dir=in action=allow program="${exe}" enable=yes profile=${profiles.map((p) => p.toLowerCase()).join(",")}`;
// "Private network", "Domain and Private networks".
const networksOf = (names) => `${names.join(" and ")} network${names.length > 1 ? "s" : ""}`;

// What the firewall does to the service's programs (`exe`, and `next`, the
// one a restart runs) on the networks this computer is on: off, letting
// them in, blocking every incoming connection, blocking them, or not having
// let them in yet, with the netsh line that lets them in. A public network
// is no place to accept connections, so that fix makes it private first.
async function windowsFirewall(ctx, exe, next) {
  const paths = next ? [exe, next] : [exe];
  const fix = (profiles) => {
    const want = [...new Set(profiles.map((n) => (n === "Public" ? "Private" : n)))];
    const lines = paths.map((p) => netshAllow(p, want)).join(", then ");
    return profiles.includes("Public") ? `This network is Public: make it Private in Settings › Network & internet, in its properties. Then in a terminal run as administrator: ${lines}` : `In a terminal run as administrator: ${lines}`;
  };
  const r = await ctx.run(POWERSHELL, ["-NoProfile", "-NonInteractive", "-Command", windowsFirewallScript(paths)], 30000);
  let seen = null;
  try {
    seen = r?.code === 0 && !r.timedOut ? JSON.parse(String(r.stdout ?? "")) : null;
  } catch {}
  const shown = tilde(exe, ctx.home);
  const list = (v) => (Array.isArray(v) ? v : v == null ? [] : [v]);
  if (!seen || typeof seen !== "object" || !list(seen.profiles).length) return info("XenoNet couldn't read Windows Defender Firewall's settings", `If other machines can't reach this gateway, let ${shown} in. ${fix(["Private"])}`);
  const profile = (name) => list(seen.profiles).find((p) => p?.name === name) ?? {};
  // The profiles of the networks this computer is on (all three when it
  // can't tell), and those the firewall is on for.
  const known = [...new Set(list(seen.networks).map((n) => PROFILES[n]).filter(Boolean))];
  const used = known.length ? known : ["Domain", "Private", "Public"];
  const on = used.filter((name) => profile(name).enabled === "True");
  if (!on.length) return ok(`Windows Defender Firewall is off for ${networksOf(used)}`);
  const all = on.filter((name) => profile(name).rules === "False");
  if (all.length) return warn(`Windows Defender Firewall blocks every incoming connection on ${networksOf(all)}, so no other machine can reach this gateway`, "Turn off Blocks all incoming connections: Windows Security › Firewall & network protection, then that network's profile.");
  const covers = (rule, name) => rule?.enabled === "True" && (rule.profile === "Any" || String(rule.profile).split(/,\s*/).includes(name));
  const rulesOf = (path) => list(seen.rules).filter((rule) => String(rule?.program).toLowerCase() === path.toLowerCase());
  const blocking = paths.flatMap(rulesOf).filter((rule) => rule.action === "Block" && on.some((name) => covers(rule, name)));
  const blocked = on.filter((name) => blocking.some((rule) => covers(rule, name)));
  if (blocked.length) {
    const names = [...new Set(blocking.map((rule) => `"${rule.name}"`))].join(", ");
    return warn(`Windows Defender Firewall blocks ${shown} on ${networksOf(blocked)}, so other machines can't reach this gateway`, `Turn off the rule that blocks it (${names}) in Windows Defender Firewall with Advanced Security › Inbound Rules. ${fix(blocked)}`);
  }
  const open = (path, name) => profile(name).inbound === "Allow" || rulesOf(path).some((rule) => rule.action === "Allow" && covers(rule, name));
  const missing = on.filter((name) => paths.some((path) => !open(path, name)));
  if (!missing.length) return ok(`Windows Defender Firewall lets ${shown} in on ${networksOf(on)}${next ? `, and ${tilde(next, ctx.home)}, which the service runs from after a restart` : ""}`);
  return warn(`Windows Defender Firewall hasn't let ${shown} in on ${networksOf(missing)}, so other machines can't reach this gateway`, fix(missing));
}

// What socketfilterfw said, when it said it: its output, or null.
async function said(ctx, args) {
  const r = await ctx.run(SOCKETFILTERFW, args, 5000);
  if (r?.code !== 0 || r.timedOut) return null;
  return String(r.stdout ?? "");
}

async function firewall(ctx) {
  if (typeof ctx.network !== "function") return skip("needs XenoNet running");
  const net = await ctx.network();
  if (!net) return skip("this machine isn't in a network");
  if (!net.gateway) return skip("this machine isn't the network's gateway");
  const kind = typeof net.kind === "string" ? net.kind : null;
  if (kind === null) return skip("the gateway has no address yet");
  if (kind === "loopback") return skip("the gateway's address is on this computer alone, where no firewall stands in the way");
  if (kind !== "lan") return skip("other machines reach the gateway through its tunnel, which connects on this computer, where no firewall stands in the way");
  if (ctx.platform !== "darwin" && ctx.platform !== "win32") return skip("for macOS and Windows, whose firewalls let programs in by their path");
  const exe = ctx.self?.exe ? String(ctx.self.exe) : null;
  if (!exe) return info("XenoNet couldn't tell which program its service runs as");
  const restart = await ctx.restartExe?.();
  const next = restart && String(restart) !== exe ? String(restart) : null;
  if (ctx.platform === "win32") return windowsFirewall(ctx, exe, next);
  const home = ctx.home ?? "";
  const stateText = await said(ctx, ["--getglobalstate"]);
  const state = stateText === null ? null : globalState(stateText);
  if (state === null) return info("XenoNet couldn't read whether the firewall is on", unreadable("socketfilterfw"));
  if (state === 0) return ok("the firewall is off");
  if (state === 2 || /enabled/i.test((await said(ctx, ["--getblockall"])) ?? "")) return warn("the firewall blocks every incoming connection, so no other machine can reach this gateway", "Turn off Block all incoming connections: System Settings › Network › Firewall › Options.");
  const listText = await said(ctx, ["--listapps"]);
  const apps = listText === null ? null : listed(listText);
  if (!apps) return info("the firewall is on, and XenoNet couldn't read which programs it lets in", allowFix([exe], false, home));
  const now = await listing(ctx, apps, exe);
  const later = next ? await listing(ctx, apps, next) : now;
  const [shown, after] = [tilde(exe, ctx.home), next ? tilde(next, ctx.home) : null];
  if (now === true && later === true) return ok(`the firewall is on and lets ${shown} in${after ? `, and ${after}, which the service runs from after a restart` : ""}`);
  let detail =
    now === true && after
      ? `the firewall lets ${shown} in, but after a restart the service runs from ${after}, which it hasn't let in`
      : now === false
        ? `the firewall blocks ${shown}, so other machines can't reach this gateway`
        : `the firewall is on and hasn't let ${shown} in, so other machines can't reach this gateway`;
  if (now !== true && after) detail += `; after a restart it runs from ${after}`;
  const paths = [...(now !== true ? [exe] : []), ...(next && later !== true ? [next] : [])];
  return warn(detail, allowFix(paths, now === false || later === false, home));
}

// The registration the service.* checks read (ctx.login(), as
// xenonet-components' supervisor review sees it), or why there's none to
// read.
const NOT_REGISTERED = "the service doesn't start at login";
async function registration(ctx) {
  const l = await ctx.login?.();
  if (!l) return { l: null, why: "XenoNet can't look at how its service starts from here" };
  if (!l.supported) return { l: null, why: "this system has no supervisor XenoNet registers with" };
  if (!l.registered) return { l: null, why: NOT_REGISTERED };
  return { l, why: null };
}

// Where each supervisor says more, and what loads the service again.
const LOOK = {
  launchd: "launchctl print gui/$(id -u)/dev.xenonet.service says more.",
  systemd: "systemctl --user status xenonet says more.",
  "Task Scheduler": 'schtasks /query /tn "XenoNet Service" /v /fo list says more.',
};
const UNLOADED = {
  launchd: "If you turned it off in System Settings › General › Login Items & Extensions, turn it on there. Otherwise run xenonet service install again, which loads it.",
  systemd: "Run xenonet service install again, which enables it, or systemctl --user enable --now xenonet.service.",
  "Task Scheduler": "Run xenonet service install again, which creates the task again.",
};

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
