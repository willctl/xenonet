// XenoNet's doctor (core/doctor): each check against a stand-in machine, the
// verdicts, what counts as a new problem, and the server's schedule.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { machine, runDoctor, newProblems, CHECKS } from "../core/doctor/index.js";
import { createDoctorService } from "../core/doctor/service.js";
import { regPath, samePlace, savedPath } from "../core/command.js";

const HOME = "/Users/ada";

// XenoNet's parts as /api/health lists them, all well.
const BLOCKS = () => [
  { name: "state", kind: "store", state: "running", since: 1, lastOk: 2, lastError: null, counters: { errors: 0, saves: 3 } },
  { name: "transcripts", kind: "tailer", state: "running", since: 1, lastOk: 2, lastError: null, counters: { errors: 0, files: 40 } },
  { name: "codex-app-server", kind: "client", state: "idle", since: 1, lastOk: null, lastError: null, counters: { errors: 0 } },
];

// XenoNet's observers as the server reports them, each reading what it needs.
const OBSERVERS = () => [
  { product: "hermes-agent", name: "Hermes Agent", installed: true, on: true, recognized: { expected: 1, found: 1, purposes: [] }, sources: [{ name: "session database", file: "~/.hermes/state.db", state: "ok" }, { name: "spawn ledger", file: "~/.hermes/spawn-ledger.json", state: "ok" }, { name: "gateway's state", file: "~/.hermes/gateway_state.json", state: "missing" }] },
  { product: "grok-bot", name: "Grok Bot", installed: true, on: true, recognized: null, sources: [{ name: "processes", file: null, state: "ok" }, { name: "app", file: "/Applications/Grok Bot.app", state: "ok" }] },
];

// A machine where everything is in place; each test breaks one thing.
function ctxWith(overrides = {}) {
  const bins = { claude: `${HOME}/.local/bin/claude`, codex: "/usr/local/bin/codex", grok: `${HOME}/.grok/bin/grok`, git: "/usr/bin/git", codesign: "/usr/bin/codesign", ...overrides.bins };
  const answers = {
    "claude --version": { code: 0, stdout: "2.1.288 (Claude Code)\n", stderr: "" },
    "claude auth status": { code: 0, stdout: JSON.stringify({ loggedIn: true, authMethod: "claude.ai" }), stderr: "" },
    "codex --version": { code: 0, stdout: "codex-cli 0.146.0\n", stderr: "" },
    "codex login status": { code: 0, stdout: "", stderr: "Logged in using ChatGPT\n" },
    "grok --version": { code: 0, stdout: "grok 1.0.50 (c58f321264ba)\n", stderr: "" },
    ...overrides.answers,
  };
  const dirs = { [`${HOME}/.claude/projects`]: ["-Users-ada-atlas"], [`${HOME}/.codex/sessions`]: ["2026"], [`${HOME}/.grok/sessions`]: ["%2FUsers%2Fada%2Fatlas"], [`${HOME}/.grok/auth.json`]: [], [`${HOME}/Projects`]: ["atlas", "ledger"], [`${HOME}/state`]: [], ...overrides.dirs };
  const denied = new Set(overrides.denied ?? []);
  return machine({
    home: HOME,
    platform: "darwin",
    node: "22.22.3",
    electron: null,
    env: { PATH: "/usr/bin" },
    which: (cmd) => bins[cmd] ?? null,
    run: async (file, args) => answers[`${file.split("/").pop()} ${args.join(" ")}`] ?? { code: 1, stdout: "", stderr: "unknown" },
    // A folder or file the test took away (set to undefined) isn't there.
    exists: (p) => dirs[p] !== undefined || Object.values(bins).includes(p),
    readdir: async (p) => {
      if (denied.has(p)) throw Object.assign(new Error("denied"), { code: "EPERM" });
      return dirs[p];
    },
    access: async () => {},
    statfs: async () => ({ bavail: 50e6, bsize: 4096 }),
    paths: { claudeProjectsDir: `${HOME}/.claude/projects`, codexHome: `${HOME}/.codex`, grokHome: `${HOME}/.grok`, projectsDir: `${HOME}/Projects`, stateDir: `${HOME}/state` },
    readText: (p) => overrides.files?.[p] ?? null,
    self: { port: 47840, name: "XenoNet" },
    health: { sources: [{ name: "projects", ok: true, count: 2 }, { name: "claude", ok: true, count: 9 }, { name: "codex", ok: true, count: 4 }, { name: "wiki", ok: false, error: "Xenolith folder not found" }], blocks: BLOCKS() },
    live: { limits: { claude: { source: "anthropic" } } },
    connect: { port: 47840, apps: ["claude", "codex", "grok"].map((app) => ({ app, installed: true, changes: [{ id: "mcp", state: "present" }, { id: "hooks", state: "present" }], broken: [] })) },
    gitAvailable: () => true,
    procs: async () => ({ supported: true, peer: true, chain: true }),
    launcher: async () => ({ dir: `${HOME}/state/runtime/launcher`, version: 2, ships: 2, said: { launcher: 2, protocol: 1, tools: "0.8.0", node: "v24.19.0" } }),
    stateFile: async (dir) => ({ file: `${dir}/state.json`, exists: true, known: 2, version: 2, unreadable: null, issues: [], broken: [], backups: [] }),
    drift: () => ({ file: `${HOME}/Library/Caches/XenoNet/drift.json`, lookedAt: null, records: [] }),
    command: () => ({ file: `${HOME}/.local/bin/xenonet`, installed: true, ours: true, runs: ["/usr/local/bin/node", `${HOME}/xenonet/cli/xenonet.js`], broken: false, onPath: true }),
    actors: { expired: () => [], observers: OBSERVERS, repos: () => [`${HOME}/Projects/atlas`, `${HOME}/Projects/ledger`], gateway: async () => ({ on: false }), wakes: async () => [] },
    login: () => LOGIN(),
    ...overrides.ctx,
  });
}

// The service started at login, as the supervisor's review sees it: on,
// current and running under launchd.
const LOGIN = (over = {}) => ({
  supported: true,
  supervisor: "launchd",
  file: `${HOME}/Library/LaunchAgents/dev.xenonet.service.plist`,
  registered: true,
  port: "47840",
  loaded: true,
  state: "running",
  pid: 4242,
  lastExit: "(never exited)",
  user: "ada",
  current: true,
  program: `${HOME}/Library/Application Support/XenoNet/runtime/launcher/xenonet`,
  programFound: true,
  ...over,
});

const byId = (r) => Object.fromEntries(r.results.map((x) => [x.id, x]));

test("everything in place: every check passes, and the optional wiki doesn't count", async () => {
  const r = await runDoctor(ctxWith());
  assert.deepEqual(r.results.map((x) => x.id), CHECKS.map((c) => c.id));
  // Lingering is Linux's, so on a Mac its check doesn't apply.
  const elsewhere = ["service.boot"];
  assert.deepEqual(r.results.filter((x) => x.status !== "ok" && !elsewhere.includes(x.id)).map((x) => [x.id, x.status, x.detail]), []);
  assert.deepEqual(elsewhere.map((id) => byId(r)[id].status), ["skip"]);
  assert.equal(byId(r)["claude.cli"].detail, "version 2.1.288, ~/.local/bin/claude");
  assert.equal(byId(r)["codex.auth"].detail, "signed in using ChatGPT");
  assert.equal(r.summary.ok, CHECKS.length - elsewhere.length);
});

test("the service started at login: on and current, off, or what to fix, with how", async () => {
  const run = async (login, ctx = {}) => byId(await runDoctor(ctxWith({ ctx: { login: () => login, ...ctx } }), { only: ["service"] }));
  const on = await run(LOGIN());
  assert.deepEqual(Object.keys(on), ["service.login", "service.file", "service.loaded", "service.boot"]);
  assert.deepEqual([on["service.login"].status, on["service.login"].detail], ["ok", "on, through launchd: ~/Library/LaunchAgents/dev.xenonet.service.plist"]);
  assert.equal(on["service.file"].detail, "~/Library/LaunchAgents/dev.xenonet.service.plist is what xenonet service install writes");
  assert.equal(on["service.loaded"].detail, "launchd has it, running as process 4242");
  assert.deepEqual([on["service.boot"].status, on["service.boot"].detail], ["skip", "for Linux, where lingering starts a user's services at boot"]);
  // Registered for another XenoNet's port.
  assert.match((await run(LOGIN({ port: "47850" })))["service.login"].detail, /, for the service on port 47850$/);

  // Off, as it is until you turn it on: a note with how, and the rest
  // don't apply.
  const off = await run({ supported: true, supervisor: "launchd", file: LOGIN().file, registered: false, loaded: false });
  assert.deepEqual([off["service.login"].status, off["service.login"].detail], ["info", "off: Desktop starts the service when it opens, and it runs until you log out"]);
  assert.match(off["service.login"].fix, /Background service in XenoNet's palette, or run xenonet service install/);
  for (const id of ["service.file", "service.loaded"]) assert.deepEqual([off[id].status, off[id].detail], ["skip", "the service doesn't start at login"]);
  // No ctx.login (an older XenoNet, the JS's own machine), or no supervisor here.
  const none = byId(await runDoctor(ctxWith({ ctx: { login: undefined } }), { only: ["service.login"] }));
  assert.deepEqual([none["service.login"].status, none["service.login"].detail], ["skip", "XenoNet can't look at how its service starts from here"]);
  assert.equal((await run({ supported: false, registered: false }))["service.login"].detail, "this system has no supervisor XenoNet registers with");

  // The registration: a program that's gone, or not what install writes now.
  const gone = await run(LOGIN({ programFound: false, program: `${HOME}/old/xenonet`, current: false }));
  assert.deepEqual([gone["service.file"].status, gone["service.file"].detail], ["fail", "~/Library/LaunchAgents/dev.xenonet.service.plist runs ~/old/xenonet, which isn't there"]);
  assert.equal(gone["service.file"].fix, "Run xenonet service install again, which points it at XenoNet's launcher.");
  const stale = (await run(LOGIN({ current: false })))["service.file"];
  assert.deepEqual([stale.status, stale.detail], ["warn", "~/Library/LaunchAgents/dev.xenonet.service.plist isn't what xenonet service install writes now, so the service may not start the way this XenoNet expects"]);
  assert.match(stale.fix, /^Run xenonet service install again, which writes it afresh and reloads it\./);
  const unsure = (await run(LOGIN({ current: null, expected: "no account to start it for" })))["service.file"];
  assert.deepEqual([unsure.status, unsure.detail], ["info", "XenoNet couldn't tell whether ~/Library/LaunchAgents/dev.xenonet.service.plist is current: no account to start it for"]);

  // What launchd, systemd or Task Scheduler says.
  const unloaded = (await run(LOGIN({ loaded: false, state: null, pid: null })))["service.loaded"];
  assert.deepEqual([unloaded.status, unloaded.detail], ["warn", "launchd doesn't have it, so it doesn't start at login"]);
  assert.match(unloaded.fix, /Login Items & Extensions/);
  const asked = (await run(LOGIN({ supervisor: "systemd", error: "Failed to connect to bus: No medium found" })))["service.loaded"];
  assert.deepEqual([asked.status, asked.detail, asked.fix], ["warn", "XenoNet couldn't ask systemd about it: Failed to connect to bus: No medium found", "systemctl --user status xenonet says more."]);
  const disabled = (await run(LOGIN({ supervisor: "Task Scheduler", loaded: false, state: "disabled", pid: null })))["service.loaded"];
  assert.deepEqual([disabled.status, disabled.detail], ["warn", "it's turned off in Task Scheduler, so it doesn't start when you sign in"]);
  const failed = (await run(LOGIN({ supervisor: "systemd", state: "failed", pid: null, lastExit: "1" })))["service.loaded"];
  assert.deepEqual([failed.status, failed.detail], ["warn", "systemd gave up on it after it kept stopping (it last exited with 1)"]);
  const stopped = (await run(LOGIN({ state: "not running", pid: null, lastExit: "0" })))["service.loaded"];
  assert.deepEqual([stopped.status, stopped.detail], ["info", "launchd has it, but it isn't running now"]);

  // Lingering, on Linux: wanted where there's no graphical session, as on
  // a host reached over SSH.
  const linux = (over) => run(LOGIN({ supervisor: "systemd", file: "/home/ada/.config/systemd/user/xenonet.service", ...over }), { platform: "linux" });
  assert.deepEqual([(await linux({ linger: true }))["service.boot"].status, (await linux({ linger: true }))["service.boot"].detail], ["ok", "lingering is on, so it starts at boot and keeps running after you log out"]);
  const desk = (await linux({ linger: false, graphical: true }))["service.boot"];
  assert.deepEqual([desk.status, desk.detail], ["info", "lingering is off: it starts when you log in, and stops when you log out"]);
  assert.match(desk.fix, /run loginctl enable-linger ada once/);
  const host = (await linux({ linger: false, graphical: false }))["service.boot"];
  assert.equal(host.status, "warn");
  assert.match(host.detail, /no graphical session here, as on a host reached over SSH/);
  assert.match(host.fix, /sudo loginctl enable-linger ada\.$/);
  const unknown = (await linux({ linger: null, user: "" }))["service.boot"];
  assert.deepEqual([unknown.status, unknown.fix], ["info", "loginctl show-user $USER --property=Linger says."]);
});

test("sign-ins: signed out is a failure with the fix; no Claude Code skips what needs it", async () => {
  const out = byId(await runDoctor(ctxWith({ answers: { "claude auth status": { code: 1, stdout: JSON.stringify({ loggedIn: false }), stderr: "" }, "codex login status": { code: 1, stdout: "Not logged in\n", stderr: "" } } })));
  assert.deepEqual([out["claude.auth"].status, out["claude.auth"].fix], ["fail", "Run claude and sign in with /login."]);
  assert.deepEqual([out["codex.auth"].status, out["codex.auth"].fix], ["fail", "Run codex login."]);
  const none = byId(await runDoctor(ctxWith({ bins: { claude: undefined } })));
  assert.equal(none["claude.cli"].status, "warn");
  assert.equal(none["claude.auth"].status, "skip");
});

test("Grok Build: not installed is a note, and so is no sign-in; its sessions folder, and hooks your organization or you turned off", async () => {
  const everything = byId(await runDoctor(ctxWith()));
  assert.equal(everything["grok.cli"].detail, "version 1.0.50, ~/.grok/bin/grok");
  assert.equal(everything["grok.sessions"].detail, "1 folder of sessions in ~/.grok/sessions");
  // Off PATH, where its installer puts it.
  const offPath = byId(await runDoctor(ctxWith({ bins: { grok: undefined }, dirs: { [`${HOME}/.grok/bin/grok`]: [] } })));
  assert.deepEqual([offPath["grok.cli"].status, offPath["grok.cli"].detail], ["ok", "version 1.0.50, ~/.grok/bin/grok"]);
  const none = byId(await runDoctor(ctxWith({ bins: { grok: undefined }, ctx: { connect: { port: 47840, apps: [{ app: "grok", installed: false, changes: [{ id: "mcp", state: "missing" }, { id: "hooks", state: "missing" }], broken: [] }] } } })));
  assert.deepEqual([none["grok.cli"].status, none["grok.cli"].fix], ["info", "To use Grok Build, install it (x.ai/cli), then run grok login."]);
  assert.deepEqual(["grok.auth", "grok.hooks", "agents.grok"].map((id) => none[id].status), ["skip", "skip", "skip"]);
  const signedOut = byId(await runDoctor(ctxWith({ dirs: { [`${HOME}/.grok/auth.json`]: undefined } })));
  assert.deepEqual([signedOut["grok.auth"].status, signedOut["grok.auth"].fix], ["info", "Run grok login."]);
  const keyed = byId(await runDoctor(ctxWith({ dirs: { [`${HOME}/.grok/auth.json`]: undefined }, ctx: { env: { PATH: "/usr/bin", XAI_API_KEY: "made-up" } } })));
  assert.equal(keyed["grok.auth"].detail, "with an API key (XAI_API_KEY)");
  const noSessions = byId(await runDoctor(ctxWith({ dirs: { [`${HOME}/.grok/sessions`]: undefined } })));
  assert.equal(noSessions["grok.sessions"].status, "info");
  const denied = byId(await runDoctor(ctxWith({ denied: [`${HOME}/.grok/sessions`] })));
  assert.match(denied["grok.sessions"].fix, /Privacy & Security/);
  const managed = byId(await runDoctor(ctxWith({ files: { "/etc/grok/requirements.toml": "allow_managed_hooks_only = true\n" } })));
  assert.equal(managed["grok.hooks"].status, "warn");
  assert.match(managed["grok.hooks"].detail, /only the hooks your organization sets/);
  const off = byId(await runDoctor(ctxWith({ files: { [`${HOME}/.grok/disabled-hooks`]: "global:xenonet.json:PostToolUse\n" } })));
  assert.deepEqual([off["grok.hooks"].status, off["grok.hooks"].fix], ["warn", "Run /hooks in Grok Build, select XenoNet's hooks and press Space to turn them on."]);
});

test("folders: missing, or unreadable with the macOS privacy fix", async () => {
  const r = byId(await runDoctor(ctxWith({ dirs: { [`${HOME}/Projects`]: undefined }, denied: [`${HOME}/.claude/projects`] })));
  assert.equal(r["claude.transcripts"].status, "fail");
  assert.match(r["claude.transcripts"].detail, /can't read ~\/\.claude\/projects \(EPERM\)/);
  assert.match(r["claude.transcripts"].fix, /Privacy & Security/);
  const missing = byId(await runDoctor(ctxWith({ ctx: { exists: (p) => p !== `${HOME}/Projects` } })));
  assert.equal(missing["system.projects"].status, "fail");
  assert.match(missing["system.projects"].fix, /Change the projects folder… in XenoNet's palette, or xenonet root <folder>/);
  const set = byId(await runDoctor(ctxWith({ ctx: { exists: (p) => p !== `${HOME}/Projects`, env: { PATH: "/usr/bin", XENONET_PROJECTS_DIR: `${HOME}/Projects` } } })));
  assert.match(set["system.projects"].fix, /set XENONET_PROJECTS_DIR/, "it decides, so that's the fix");
});

test("the runtime, XenoNet's own state and the agents' connection", async () => {
  assert.equal(byId(await runDoctor(ctxWith({ ctx: { node: "22.12.0" } })))["system.runtime"].status, "fail");
  assert.equal(byId(await runDoctor(ctxWith({ ctx: { node: "20.0.0", electron: "44.5.1" } })))["system.runtime"].status, "ok");
  const low = byId(await runDoctor(ctxWith({ ctx: { statfs: async () => ({ bavail: 100, bsize: 4096 }) } })));
  assert.equal(low["xenonet.state"].status, "warn");

  const apps = (states, broken = []) => ({ connect: { port: 47840, apps: ["claude", "codex", "grok"].map((app) => ({ app, installed: true, changes: states.map((state, i) => ({ id: ["mcp", "hooks"][i], state })), broken })) } });
  assert.equal(byId(await runDoctor(ctxWith({ ctx: apps(["missing", "missing"]) })))["agents.claude"].status, "info");
  assert.match(byId(await runDoctor(ctxWith({ ctx: apps(["present", "missing"]) })))["agents.codex"].detail, /partly connected \(hooks missing\)/);
  const gone = byId(await runDoctor(ctxWith({ ctx: apps(["present", "present"], ["~/old/xenonet/cli/xenonet.js is gone"]) })));
  assert.deepEqual([gone["agents.claude"].status, gone["agents.claude"].fix], ["fail", "Run xenonet connect again."]);
  // XenoNet's plugin for Claude Code: what it carries counts as connected,
  // connect's own entries beside it are a warning, and the command it runs
  // missing from PATH is a failure of its own.
  const plugin = (states, broken = []) => ({ connect: { port: 47840, apps: [{ app: "claude", changes: states.map((state, i) => ({ id: ["mcp", "hooks"][i], state, ...(state === "present" ? { via: "plugin" } : {}) })), broken }] } });
  const carried = byId(await runDoctor(ctxWith({ ctx: plugin(["present", "present"]) })))["agents.claude"];
  assert.deepEqual([carried.status, carried.detail], ["ok", "connected to XenoNet on port 47840, through its plugin for Claude Code"]);
  const twice = byId(await runDoctor(ctxWith({ ctx: plugin(["remove", "remove"]) })))["agents.claude"];
  assert.deepEqual([twice.status, twice.fix], ["warn", "Run xenonet connect claude, which takes connect's own entries out and keeps the plugin."]);
  assert.equal(twice.detail, "connected twice, through XenoNet's plugin and through xenonet connect's own entries: each event still arrives once, but sessions get XenoNet's tools twice");
  const hooksOnly = byId(await runDoctor(ctxWith({ ctx: plugin(["present", "remove"]) })))["agents.claude"];
  assert.equal(hooksOnly.detail, "connected twice, through XenoNet's plugin and through xenonet connect's own entries: each event still arrives once", "the tools come from the plugin alone");
  const noCommand = byId(await runDoctor(ctxWith({ ctx: plugin(["present", "present"], ["XenoNet's plugin runs the xenonet command, which isn't on your PATH"]) })))["agents.claude"];
  assert.deepEqual([noCommand.status, noCommand.fix], ["fail", "Install XenoNet, or put its xenonet command on your PATH."]);
  // Without a running XenoNet, what only it knows is skipped.
  const alone = byId(await runDoctor(ctxWith({ ctx: { self: null, base: null, health: null, live: null, connect: null } })));
  assert.deepEqual(["xenonet.server", "xenonet.sources", "claude.usage", "agents.claude"].map((id) => alone[id].status), ["warn", "skip", "skip", "skip"]);
});

test("XenoNet's state file: moved aside, unreadable, from a newer XenoNet, not being saved, or with parts it can't use", async () => {
  const state = async (file, ctx = {}) => byId(await runDoctor(ctxWith({ ctx: { stateFile: async (dir) => ({ file: `${dir}/state.json`, exists: true, known: 2, version: 2, unreadable: null, issues: [], broken: [], backups: [], ...file }), ...ctx } }), { only: ["xenonet.state"] }))["xenonet.state"];
  assert.equal((await state({})).status, "ok");
  assert.equal((await state({ version: 1, backups: [`${HOME}/state/state.v1.json`] })).status, "ok", "an older file is moved up when XenoNet starts");

  const broken = await state({ broken: [`${HOME}/state/state.broken-2026-10-05T12-00-00.000Z.json`] });
  assert.equal(broken.status, "warn");
  assert.equal(broken.detail, "~/state/state.broken-2026-10-05T12-00-00.000Z.json is a state.json XenoNet couldn't read, so it moved it aside and started afresh");
  assert.match(broken.fix, /put it back as state.json while XenoNet is closed/);
  assert.match((await state({ unreadable: "isn't valid JSON", version: null })).detail, /^state.json isn't valid JSON, so XenoNet will move it aside/);
  const newer = await state({ version: 3 });
  assert.deepEqual([newer.status, newer.detail], ["warn", "state.json is from a newer XenoNet (version 3), so this one reads it but doesn't save changes to it"]);
  const stuck = await state({ version: 1 }, { stateLoad: { readOnly: true, problem: "XenoNet couldn't keep a copy of state.json (ENOSPC) before moving it to version 2, so it leaves the file as it is" } });
  assert.match(stuck.detail, /couldn't keep a copy .* \(ENOSPC\).*; changes last only until XenoNet quits$/);
  const odd = await state({ issues: [{ path: "layout", code: "invalid_type", message: "Invalid input: expected record, received array" }, { path: "acks.s", message: "x" }] });
  assert.deepEqual([odd.status, odd.detail], ["warn", "state.json has 2 parts XenoNet can't use, such as layout: expected record, received array"]);
  // Under a key that's a node id or a path key (rules' are lowercased), the
  // home folder shows as ~.
  const keyed = await state({ issues: [{ path: `layout["project:${HOME}/Projects/atlas"].x`, message: "Invalid input: expected number, received string" }] });
  assert.equal(keyed.detail, `state.json has 1 part XenoNet can't use, such as layout["project:~/Projects/atlas"].x: expected number, received string`);
  const rule = await state({ issues: [{ path: `projects.rules["${HOME.toLowerCase()}/projects/atlas"].as`, message: "x" }] });
  assert.match(rule.detail, /such as projects\.rules\["~\/projects\/atlas"\]\.as: x$/);
  assert.equal((await state(null, { stateFile: async () => { throw new Error("no"); } })).status, "ok", "a look that fails says nothing");
});

test("XenoNet's parts: a warning names each that hasn't worked for a minute, failed first, with what went wrong", async () => {
  const blocks = async (list, ctx = {}) => byId(await runDoctor(ctxWith({ ctx: { health: list === undefined ? { sources: [] } : { sources: [], blocks: list }, ...ctx } }), { only: ["xenonet.blocks"] }))["xenonet.blocks"];
  const fine = await blocks(BLOCKS());
  assert.deepEqual([fine.status, fine.detail], ["ok", "3 parts, none with a problem"]);
  const list = BLOCKS();
  const minutesAgo = (m) => Date.now() - m * 60e3;
  list[1] = { ...list[1], state: "degraded", since: minutesAgo(5), lastError: { at: 3, message: `EMFILE: too many open files, watch '${HOME}/.claude/projects'` } };
  list[2] = { ...list[2], state: "failed", since: minutesAgo(2), lastError: { at: 4, message: "spawn codex ENOENT" } };
  const bad = await blocks(list);
  assert.equal(bad.status, "warn");
  assert.equal(bad.detail, "codex-app-server isn't working (spawn codex ENOENT); transcripts is partly working (EMFILE: too many open files, watch '~/.claude/projects')");
  assert.match(bad.fix, /restart XenoNet/);
  assert.doesNotMatch(bad.fix, /below|above/, "it may be shown alone");
  // One that went wrong a moment ago is only a note: it's tried again.
  list[1] = { ...list[1], since: minutesAgo(0.1) };
  assert.equal((await blocks(list)).detail, "codex-app-server isn't working (spawn codex ENOENT)", "only what has lasted");
  list[2] = { ...list[2], since: minutesAgo(0.5) };
  const blip = await blocks(list);
  assert.deepEqual([blip.status, blip.detail], ["info", "codex-app-server isn't working (spawn codex ENOENT); transcripts is partly working (EMFILE: too many open files, watch '~/.claude/projects'), a moment ago; XenoNet tries again by itself"]);
  assert.equal((await blocks(undefined)).status, "skip", "a XenoNet that doesn't list its parts");
  assert.equal((await blocks(BLOCKS(), { health: null })).status, "skip", "no XenoNet running");
});

test("XenoNet's parts: one that reads for a source the sources check reports as failed isn't said twice", async () => {
  const run = async (sources, list) => byId(await runDoctor(ctxWith({ ctx: { health: { sources, blocks: list } } }), { only: ["xenonet.sources", "xenonet.blocks"] }));
  const since = Date.now() - 3 * 60e3;
  const down = (name, source, message) => ({ name, kind: "gatherer", source, state: "failed", since, lastOk: null, lastError: { at: 5, message }, counters: { errors: 9 } });
  const sdk = "the Agent SDK couldn't list sessions";
  const claude = await run([{ name: "claude", ok: false, error: sdk }], [...BLOCKS(), down("claude-sessions", "claude", sdk)]);
  assert.deepEqual([claude["xenonet.sources"].status, claude["xenonet.sources"].detail], ["fail", `claude: ${sdk}`]);
  assert.deepEqual([claude["xenonet.blocks"].status, claude["xenonet.blocks"].detail], ["ok", "4 parts, none with a problem besides those reading for claude, which XenoNet can't read"]);
  // Shown alone (xenonet doctor --only xenonet.blocks), it reads the same: it
  // names the source, rather than pointing at a check that isn't there.
  const alone = byId(await runDoctor(ctxWith({ ctx: { health: { sources: [{ name: "claude", ok: false, error: sdk }, { name: "codex", ok: false, error: "no app-server" }], blocks: [...BLOCKS(), down("claude-sessions", "claude", sdk), down("codex-threads", "codex", "no app-server")] } } }), { only: ["xenonet.blocks"] }))["xenonet.blocks"];
  assert.equal(alone.detail, "5 parts, none with a problem besides those reading for claude and codex, which XenoNet can't read");

  // A part with no source, or whose source is fine, is still said.
  const both = await run([{ name: "claude", ok: false, error: sdk }], [...BLOCKS(), down("claude-sessions", "claude", sdk), down("git-status", undefined, "git status timed out")]);
  assert.deepEqual([both["xenonet.blocks"].status, both["xenonet.blocks"].detail], ["warn", "git-status isn't working (git status timed out)"]);
  assert.equal((await run([{ name: "claude", ok: true, count: 3 }], [...BLOCKS(), down("claude-sessions", "claude", sdk)]))["xenonet.blocks"].status, "warn");
  // The sources check passes over identities' failures, so their part says it.
  const ids = await run([{ name: "identities", ok: false, error: "no git" }], [...BLOCKS(), down("identities", "identities", "no git")]);
  assert.deepEqual([ids["xenonet.sources"].status, ids["xenonet.blocks"].status, ids["xenonet.blocks"].detail], ["ok", "warn", "identities isn't working (no git)"]);
});

test("agents passing for each other: checked on macOS, a note elsewhere, a warning when the lookup breaks", async () => {
  const check = async (ctx) => byId(await runDoctor(ctxWith({ ctx }), { only: ["agents.checked"] }))["agents.checked"];
  assert.equal((await check({})).status, "ok");
  const elsewhere = await check({ platform: "linux" });
  assert.equal(elsewhere.status, "info");
  assert.match(elsewhere.detail, /takes each session at its word/);
  // netstat with no line for a request: agents are taken at their word,
  // so their tools keep working.
  const blind = await check({ procs: async () => ({ supported: true, peer: false, chain: false, exe: false }) });
  assert.deepEqual([blind.status, blind.detail, blind.fix], ["warn", "netstat doesn't say which process sends a request, so XenoNet takes each session at its word", "netstat answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose."]);
  // ps that can't trace it: refused.
  const untraced = await check({ procs: async () => ({ supported: true, peer: true, chain: false, exe: false }) });
  assert.deepEqual([untraced.status, untraced.detail, untraced.fix], ["warn", "ps doesn't say which processes a request comes from, so XenoNet refuses every request in a session's name, and sessions can't use its tools", "ps answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose."]);
  assert.equal((await check({ procs: async () => ({ supported: true, peer: true, chain: true, exe: false }) })).status, "ok", "an agent's check doesn't rest on lsof");
  assert.equal((await check({ exists: () => false })).status, "warn", "no codesign");
});

test("the xenonet command: not installed is a note; gone or off PATH, a warning", async () => {
  const st = (over) => ({ ctx: { command: () => ({ file: `${HOME}/.local/bin/xenonet`, installed: true, ours: true, runs: ["/n", "/e"], broken: false, onPath: true, ...over }) } });
  const check = async (over) => byId(await runDoctor(ctxWith(st(over)), { only: ["system.command"] }))["system.command"];
  assert.deepEqual([(await check({})).status, (await check({})).detail], ["ok", "~/.local/bin/xenonet"]);
  assert.equal((await check({ installed: false, ours: false, runs: null })).status, "info");
  assert.match((await check({ ours: false })).detail, /another program's/);
  assert.match((await check({ broken: true })).detail, /runs \/n \/e, which is gone/);
  assert.equal((await check({ onPath: false })).status, "warn");
  assert.match((await check({ onPath: false })).fix, /export PATH=/);
  assert.equal((await check({ file: null })).status, "skip");
  // Windows: Setup adds the folder; no shell-profile advice.
  const win = byId(await runDoctor(ctxWith({ ctx: { platform: "win32", command: () => ({ file: `${HOME}/AppData/Local/XenoNet/bin/xenonet.cmd`, installed: true, ours: true, runs: ["/n", "/e"], broken: false, onPath: false }) } }), { only: ["system.command"] }))["system.command"];
  assert.equal(win.status, "warn");
  assert.match(win.fix, /Setup again.*~\/AppData\/Local\/XenoNet\/bin.*user Path/);
  assert.doesNotMatch(win.fix, /export/);
  // A XenoNet started before Setup added the folder doesn't have it in its own
  // PATH, but a new terminal does: that's what counts.
  const winCmd = { platform: "win32", command: () => ({ file: "C:\\Users\\ada\\AppData\\Local\\XenoNet\\bin\\xenonet.cmd", installed: true, ours: true, runs: ["/n", "/e"], broken: false, onPath: false }) };
  const saved = async (dirs) => byId(await runDoctor(ctxWith({ ctx: { ...winCmd, savedPath: async () => dirs } }), { only: ["system.command"] }))["system.command"];
  const later = await saved(["C:\\Windows\\system32", "c:\\users\\ada\\appdata\\local\\xenonet\\bin\\"]);
  assert.equal(later.status, "ok");
  assert.match(later.detail, /xenonet\.cmd, on your Path for new terminals$/);
  assert.equal((await saved(["C:\\Windows\\system32"])).status, "warn", "not there either");
  assert.equal((await saved(null)).status, "warn", "the registry couldn't be read");
});

test("the Path a new terminal gets on Windows: the machine's and the user's, with %VARS% filled in", async () => {
  const machinePath = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment\r\n    Path    REG_EXPAND_SZ    %SystemRoot%\\system32;C:\\Program Files\\Git\\cmd;\r\n\r\n";
  const userPath = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    %USERPROFILE%\\.local\\bin;%LocalAppData%\\XenoNet\\bin;%NOT_SET%\\x\r\n\r\n";
  const env = { SYSTEMROOT: "C:\\Windows", USERPROFILE: "C:\\Users\\ada", LOCALAPPDATA: "C:\\Users\\ada\\AppData\\Local" };
  const calls = [];
  const run = async (file, args) => {
    calls.push([file, ...args]);
    return args[1].startsWith("HKLM") ? { code: 0, stdout: machinePath } : { code: 0, stdout: userPath };
  };
  assert.deepEqual(await savedPath({ run, env }), ["C:\\Windows\\system32", "C:\\Program Files\\Git\\cmd", "C:\\Users\\ada\\.local\\bin", "C:\\Users\\ada\\AppData\\Local\\XenoNet\\bin", "%NOT_SET%\\x"]);
  assert.equal(calls[0][0], "C:\\Windows\\System32\\reg.exe", "reg from Windows' own folder, not whatever PATH finds");
  assert.deepEqual(await savedPath({ run: async (f, a) => (a[1].startsWith("HKLM") ? { code: 1, stdout: "" } : { code: 0, stdout: userPath }), env }), ["C:\\Users\\ada\\.local\\bin", "C:\\Users\\ada\\AppData\\Local\\XenoNet\\bin", "%NOT_SET%\\x"], "one unreadable key leaves the other");
  assert.equal(await savedPath({ run: async () => ({ code: 1, stdout: "" }), env }), null);
  assert.deepEqual(regPath("    Path    REG_SZ    C:\\a;;C:\\b\r\n", {}), ["C:\\a", "C:\\b"]);
  assert.ok(samePlace("C:\\Users\\Ada\\XenoNet\\bin\\", "c:\\users\\ada\\xenonet\\bin", "win32"));
  assert.ok(!samePlace("C:\\Users\\Ada\\XenoNet\\bin2", "C:\\Users\\Ada\\XenoNet\\bin", "win32"));
});

test("plan usage: a sign-in to refresh is a warning; waiting on Anthropic is only a note", async () => {
  const usage = async (claude) => byId(await runDoctor(ctxWith({ ctx: { live: { limits: { claude } } } }), { only: ["claude.usage"] }))["claude.usage"];
  const old = Date.now() - 47 * 3600e3;
  const waiting = await usage({ stale: true, observedAt: old, reason: "Anthropic asked to wait (429)" });
  assert.equal(waiting.status, "info");
  assert.match(waiting.detail, /^Anthropic asked to wait \(429\), so there's no reading since .+\. XenoNet tries again by itself\.$/);
  const expired = await usage({ stale: true, observedAt: old, reason: "Claude Code's sign-in has expired; it refreshes on its next request" });
  assert.deepEqual([expired.status, expired.fix], ["warn", "Open Claude Code once; it refreshes its sign-in, and XenoNet reads it on its next try."]);
  assert.equal((await usage({ stale: true, observedAt: old, reason: "Anthropic answered 500" })).status, "warn");
  assert.equal((await usage({ source: "anthropic" })).status, "ok");
});

test("a check that hangs or throws gets a warning, not a stuck doctor", async () => {
  const r = byId(await runDoctor(ctxWith({ ctx: { run: () => new Promise(() => {}), readdir: async () => { throw new Error("boom"); } } }), { timeoutMs: 50, only: ["claude.cli", "codex.sessions"] }));
  assert.deepEqual(Object.keys(r).sort(), ["claude.cli", "codex.sessions"]);
  assert.match(r["claude.cli"].detail, /didn't finish in 0.05 seconds/);
  assert.equal(r["codex.sessions"].status, "fail");
});

test("new problems: only what got worse since the last run", () => {
  const run = (statuses) => ({ results: Object.entries(statuses).map(([id, status]) => ({ id, status })) });
  assert.deepEqual(newProblems(null, run({ a: "warn", b: "ok" })).map((r) => r.id), ["a"]);
  assert.deepEqual(newProblems(run({ a: "warn" }), run({ a: "warn" })), []);
  assert.deepEqual(newProblems(run({ a: "warn" }), run({ a: "fail" })).map((r) => r.id), ["a"]);
  assert.deepEqual(newProblems(run({ a: "fail" }), run({ a: "ok", b: "info" })), []);
});

test("the service: on a schedule, one run at a time, and news only of new problems", async () => {
  let status = "ok";
  let runs = 0;
  const news = [];
  const service = createDoctorService({
    context: async () => (runs++, ctxWith({ answers: { "codex login status": { code: 1, stdout: status === "ok" ? "Logged in using ChatGPT" : "Not logged in", stderr: "" } } })),
    everyMs: 40,
    firstAfterMs: 10,
    onNewProblems: (fresh) => news.push(fresh.map((r) => r.id)),
  });
  const [a, b] = [service.check(), service.check()];
  assert.equal(await a, await b, "a request during a run gets that run");
  assert.equal(runs, 1);
  status = "bad";
  await service.check();
  await service.check();
  assert.deepEqual(news, [["codex.auth"]]);
  service.start();
  await new Promise((r) => setTimeout(r, 120));
  service.stop();
  assert.ok(runs >= 4, `ran ${runs} times`);
  assert.equal(service.last.summary.fail, 1);
});

test("a result reported while a run is under way stays until a run that starts after it", async () => {
  // Each run waits until the test lets its look at the machine finish.
  const gates = [];
  const shown = [];
  const service = createDoctorService({ context: () => new Promise((resolve) => gates.push(() => resolve(ctxWith()))), onResult: (r) => shown.push(r) });
  const until = async (ok) => {
    for (let i = 0; i < 100 && !ok(); i++) await new Promise((r) => setImmediate(r));
    assert.ok(ok());
  };
  const first = service.check();
  await until(() => gates.length === 1);
  // XenoNet Dev's doctor simulation, while that run looks.
  const simulated = { at: 1, ms: 1, results: [{ id: "codex.auth", group: "Codex", title: "Codex is signed in", status: "fail", detail: "not signed in", ms: 1 }], summary: { ok: 0, info: 0, warn: 0, fail: 1, skip: 0 } };
  service.report(simulated);
  const second = service.check();
  assert.notEqual(second, first, "a request after the report gets a run of its own");
  assert.equal(service.check(), second, "which the requests after it share");
  gates.shift()();
  assert.equal((await first).summary.fail, 0, "the run from before is answered");
  assert.equal(service.last, simulated, "but doesn't take the reported result's place");
  assert.deepEqual(shown, [simulated]);
  await until(() => gates.length === 1);
  gates.shift()();
  const fresh = await second;
  assert.deepEqual([service.last, shown.at(-1)], [fresh, fresh], "the run that started after it does");
});

test("a fresh run looks after it's asked: one under way, which looked before, doesn't stand for it", async () => {
  // Drift marked seen while a run looks: that run saw it new.
  let lookedAt = null;
  const record = { schema: "drift/record/v1", source: "openai/codex/rollout-line", vendorVersion: null, kind: "missing-field", path: "payload.cli_version", shape: null, firstSeen: 5, lastSeen: 5, count: 1 };
  const gates = [];
  const shown = [];
  const service = createDoctorService({
    context: () => {
      const seen = lookedAt;
      return new Promise((resolve) => gates.push(() => resolve(ctxWith({ ctx: { drift: () => ({ file: `${HOME}/drift.json`, lookedAt: seen, records: [record] }) } }))));
    },
    onResult: (r) => shown.push(r),
    only: ["contracts.drift"],
  });
  const until = async (ok) => {
    for (let i = 0; i < 100 && !ok(); i++) await new Promise((r) => setImmediate(r));
    assert.ok(ok());
  };
  const status = (r) => byId(r)["contracts.drift"].status;
  const under = service.check();
  await until(() => gates.length === 1);
  lookedAt = 10;
  const fresh = service.check({ fresh: true });
  assert.notEqual(fresh, under, "a run of its own");
  assert.equal(service.check(), fresh, "which the requests after it share");
  gates.shift()();
  assert.equal(status(await under), "warn", "the run from before still saw it new");
  assert.deepEqual(shown, [], "and isn't shown");
  await until(() => gates.length === 1);
  gates.shift()();
  assert.equal(status(await fresh), "ok");
  assert.deepEqual([status(service.last), shown.length], ["ok", 1], "the fresh run is what the console shows");
  // With no run under way, a fresh one is a run like any other.
  const next = service.check({ fresh: true });
  await until(() => gates.length === 1);
  gates.shift()();
  assert.equal(status(await next), "ok");
});

test("a run you ask for yourself, during a scheduled one, gets a run of its own, which looks for tokens afresh; requests during it share it", async () => {
  // Each run waits until the test lets its look at the machine finish,
  // and says whether it was asked for by hand.
  const gates = [];
  const asked = [];
  const service = createDoctorService({
    context: ({ byHand } = {}) => (asked.push(byHand), new Promise((resolve) => gates.push(() => resolve(ctxWith())))),
    only: ["xenonet.server"],
  });
  const until = async (ok) => {
    for (let i = 0; i < 100 && !ok(); i++) await new Promise((r) => setImmediate(r));
    assert.ok(ok());
  };
  const scheduled = service.check();
  await until(() => gates.length === 1);
  const mine = service.check({ byHand: true });
  assert.notEqual(mine, scheduled, "the scheduled run took the day's look, so it can't stand for yours");
  assert.equal(service.check({ byHand: true }), mine, "another of yours shares it");
  assert.equal(service.check(), mine, "and so does a scheduled one");
  gates.shift()();
  await scheduled;
  await until(() => gates.length === 1);
  gates.shift()();
  await mine;
  assert.deepEqual(asked, [false, true], "the context hears which is which");
  // With none under way, yours is a run like any other, and a scheduled
  // one during it shares it.
  const again = service.check({ byHand: true });
  assert.equal(service.check(), again);
  await until(() => gates.length === 1);
  gates.shift()();
  await again;
  assert.deepEqual(asked, [false, true, true]);
});

test("the service remembers its last run, so a restart doesn't report the same problem again", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-doctor-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const file = path.join(dir, "doctor.json");
  const signedOut = async () => ctxWith({ answers: { "codex login status": { code: 1, stdout: "Not logged in", stderr: "" } } });
  const news = [];
  const first = createDoctorService({ context: signedOut, file, onNewProblems: (fresh) => news.push(fresh.length) });
  assert.equal((await first.check()).schema, undefined, "the run as the API sends it");
  const saved = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.equal(saved.schema, "doctor/run/v1", "the file says what it is");
  const { check, explain } = await import("../core/schema/index.js");
  const fits = check("doctor/run/v1", saved, { strict: true });
  assert.ok(fits.ok, explain(fits));
  const second = createDoctorService({ context: signedOut, file, onNewProblems: (fresh) => news.push(fresh.length) });
  assert.equal(second.last, null, "a new run is still needed for the console");
  await second.check();
  assert.deepEqual(news, [1]);
  const only = createDoctorService({ context: signedOut, only: ["system"] });
  assert.ok((await only.check()).results.every((r) => r.id.startsWith("system.")));
});

test("vendor data: what's changed or been added since you last looked, and how to see it", async () => {
  const record = (kind, path, shape, firstSeen, source = "openai/codex/rollout-line", vendorVersion = "0.150.0") => ({ schema: "drift/record/v1", source, vendorVersion, kind, path, shape, firstSeen, lastSeen: firstSeen, count: 3 });
  const check = async (records, lookedAt = null) => byId(await runDoctor(ctxWith({ ctx: { drift: () => ({ file: `${HOME}/Library/Caches/XenoNet/drift.json`, lookedAt, records }) } }), { only: ["contracts.drift"] }))["contracts.drift"];
  assert.deepEqual(Object.values(await check([])).slice(3, 5), ["ok", "no differences from XenoNet's schemas"]);

  const added = [record("new-variant", "type", '"world_view"', 10), record("new-field", "message.usage.iterations", "array<object>", 20, "anthropic/claude-code/transcript-line", "2.1.288")];
  const note = await check(added);
  assert.equal(note.status, "info", "an addition can't break what XenoNet reads");
  assert.equal(note.detail, `the apps' data has 2 additions since you last looked: Codex's rollouts: type "world_view" isn't one the schema lists; Claude Code's transcripts: message.usage.iterations is new (array<object>)`);
  assert.equal(note.fix, "xenonet drift lists them by source and marks them seen. XenoNet keeps them in ~/Library/Caches/XenoNet/drift.json.");

  const changed = [...added, record("type-change", "payload.rate_limits.primary.resets_at", "string", 30), record("missing-field", "payload.cli_version", null, 40)];
  const warned = await check(changed);
  assert.equal(warned.status, "warn");
  assert.equal(warned.detail, "2 changes to what XenoNet reads since you last looked: Codex's rollouts: payload.rate_limits.primary.resets_at is string now; Codex's rollouts: payload.cli_version is missing; and 2 more");

  // Once you've looked (the report), only what's first seen after counts.
  assert.deepEqual([(await check(changed, 40)).status, (await check(changed, 40)).detail], ["ok", "nothing new since you last looked (4 differences known)"]);
  assert.equal((await check(changed, 25)).status, "warn");
  assert.equal((await check(changed, 15)).detail, "2 changes to what XenoNet reads since you last looked: Codex's rollouts: payload.rate_limits.primary.resets_at is string now; Codex's rollouts: payload.cli_version is missing; and 1 more");
  // Without a cache folder (and no XenoNet running) there's nothing to read.
  assert.equal(byId(await runDoctor(ctxWith({ ctx: { drift: () => null } }), { only: ["contracts.drift"] }))["contracts.drift"].status, "skip");
});

// The command the doctor's look for token files runs in a repo, as ctxWith
// keys its answers.
const grep = (repo) => `git --no-optional-locks -C ${repo} -c core.fsmonitor=false grep -l -I --untracked -E -e xenonet_(actor|gateway)_[0-9a-f]{64}`;

test("actors: a token file in a repo is a warning that names it; a repo that doesn't answer in time leaves a note; without XenoNet, skipped", async () => {
  const check = async (answers, ctx = {}) => byId(await runDoctor(ctxWith({ answers, ctx }), { only: ["actors.repos"] }))["actors.repos"];
  assert.deepEqual([(await check({})).status, (await check({})).detail], ["ok", "none in 2 repos"]);
  const found = await check({ [grep(`${HOME}/Projects/atlas`)]: { code: 0, stdout: ".xenonet-token\nconfig/wren.token\n", stderr: "" } });
  assert.equal(found.status, "warn");
  assert.equal(found.detail.replace(/\\/g, "/"), "2 files in a repo hold an actor's token: ~/Projects/atlas/.xenonet-token, ~/Projects/atlas/config/wren.token");
  assert.match(found.fix, /New token in XenoNet's Actors panel/);
  const slow = await check({ [grep(`${HOME}/Projects/ledger`)]: { code: 1, stdout: "", stderr: "", timedOut: true } });
  assert.deepEqual([slow.status, slow.detail], ["info", "none in the 1 repo looked in; the other 1 didn't answer in time"]);
  const notRepo = await check({ [grep(`${HOME}/Projects/ledger`)]: { code: 128, stdout: "", stderr: "fatal: not a git repository" } });
  assert.equal(notRepo.status, "info");
  assert.equal((await check({}, { actors: { ...ctxWith().actors, repos: () => [] } })).detail, "no repos to look in");
  assert.equal((await check({}, { actors: undefined })).status, "skip");
});

test("actors: the server's look for token files is kept for a day, taken afresh for a run you asked for, and says when it looked if that was before this run", async () => {
  const looks = [];
  const look = (result) => async (opts) => (looks.push(opts), result);
  const check = async (tokenFiles, ctx = {}) => byId(await runDoctor(ctxWith({ ctx: { actors: { ...ctxWith().actors, tokenFiles }, ...ctx } }), { only: ["actors.repos"] }))["actors.repos"];
  // Scheduled: the day's look, from three hours ago.
  const old = Date.now() - 3 * 3600e3;
  const quiet = await check(look({ at: old, repos: 4, checked: 4, files: [] }));
  assert.deepEqual([quiet.status, quiet.detail], ["ok", "none in 4 repos (XenoNet looked 3 h ago, as it does once a day and whenever you run the doctor)"]);
  assert.deepEqual(looks.at(-1), { fresh: false });
  // By hand: a look of its own, made during this run, which needs no date.
  const fresh = await check(async (opts) => (looks.push(opts), { at: Date.now(), repos: 4, checked: 4, files: [] }), { byHand: true });
  assert.deepEqual([fresh.status, fresh.detail, looks.at(-1)], ["ok", "none in 4 repos", { fresh: true }]);
  // What an older look found is still a warning, with when.
  const found = await check(look({ at: old, repos: 2, checked: 2, files: [`${HOME}/Projects/atlas/.env`] }));
  assert.equal(found.status, "warn");
  assert.equal(found.detail, "1 file in a repo holds an actor's token: ~/Projects/atlas/.env (XenoNet looked 3 h ago, as it does once a day and whenever you run the doctor)");
  assert.match(found.fix, /a line of a \.env/);
  assert.match(found.fix, /New token in XenoNet's Actors panel/);
  const slow = await check(look({ at: Date.now() + 1, repos: 3, checked: 1, files: [] }));
  assert.deepEqual([slow.status, slow.detail], ["info", "none in the 1 repo looked in; the other 2 didn't answer in time"]);
  assert.equal((await check(look({ at: old, repos: 0, checked: 0, files: [] }))).detail, "no repos to look in (XenoNet looked 3 h ago, as it does once a day and whenever you run the doctor)");
});

test("actors: tokens that expired are a warning naming their actors, with what to do", async () => {
  const check = async (expired) => byId(await runDoctor(ctxWith({ ctx: { actors: { ...ctxWith().actors, expired: () => expired } } }), { only: ["actors.expired"] }))["actors.expired"];
  assert.deepEqual([(await check([])).status, (await check([])).detail], ["ok", "no actor's token has expired"]);
  const one = await check([{ slug: "wren", name: "Wren" }]);
  assert.deepEqual([one.status, one.detail], ["warn", `"Wren" (actor:wren) has a token that expired after 30 days without a call, so it stays offline`]);
  assert.match(one.fix, /New token in XenoNet's Actors panel, or remove the ones you don't use there or with xenonet actor remove <slug>/);
  const many = await check([{ slug: "wren", name: "Wren" }, { slug: "moth", name: "Moth" }, { slug: "heron", name: "Heron" }]);
  assert.equal(many.detail, `"Wren" (actor:wren), "Moth" (actor:moth) and 1 more have tokens that expired after 30 days without a call, so they stay offline`);
});

test("actors: the gateway off, its problem with what to do, reachable, and each app's connection that needs you", async () => {
  const check = async (gateway) => byId(await runDoctor(ctxWith({ ctx: { actors: { ...ctxWith().actors, gateway: async () => gateway } } }), { only: ["actors.gateway"] }))["actors.gateway"];
  assert.equal((await check({ on: false })).status, "ok");
  assert.equal((await check(null)).status, "skip");
  const broken = await check({ on: true, problem: "nothing answered at xenonet.example.com (ECONNREFUSED): is the tunnel running?", reach: { state: "failing" } });
  assert.deepEqual([broken.status, broken.detail], ["warn", "nothing answered at xenonet.example.com (ECONNREFUSED): is the tunnel running?"]);
  assert.match(broken.fix, /Repair sets the tunnel up again/);
  const fine = { on: true, problem: null, host: "xenonet.example.com", url: "https://xenonet.example.com/mcp", tunnel: { label: "ngrok", through: "ngrok" }, reach: { state: "ok" }, actors: [] };
  assert.deepEqual([(await check(fine)).status, (await check(fine)).detail], ["ok", "reachable at https://xenonet.example.com/mcp through ngrok"]);
  assert.equal((await check({ ...fine, reach: { state: "unknown" } })).status, "info");
  const apps = await check({
    ...fine,
    actors: [
      { name: "Muse", token: true, oldAddress: true, lastHost: "old.example.com", expiring: false },
      { name: "Grok Bot", token: true, oldAddress: false, expiring: true, expiresAt: Date.UTC(2026, 10, 2) },
      { name: "Moth", token: false, refused: { code: "revoked", reason: "rotated" } },
    ],
  });
  assert.equal(apps.status, "warn");
  assert.equal(apps.detail, "Muse last called old.example.com, an older address; Grok Bot's gateway token stops working on 2026-11-02 unless it calls before then; Moth calls with a gateway token that was replaced");
  assert.match(apps.fix, /Connect… on the actor/);
});

test("actors: webhook wakes, none set, all answering, and one that failed with what to do", async () => {
  const check = async (wakes) => byId(await runDoctor(ctxWith({ ctx: { actors: { ...ctxWith().actors, wakes: async () => wakes } } }), { only: ["actors.wakes"] }))["actors.wakes"];
  assert.equal((await check(null)).status, "skip");
  assert.deepEqual([(await check([{ actor: "actor:muse", kind: "notify" }])).status, (await check([])).detail], ["ok", "no actor is woken by a webhook"]);
  assert.equal((await check([{ actor: "actor:grok-bot", kind: "webhook", last: { ok: true } }])).detail, "1 actor woken by a webhook, the last wake of each answered");
  const failed = await check([{ actor: "actor:grok-bot", kind: "webhook", last: { ok: false, why: "api.example.com answered HTTP 401", reason: "HTTP 401" } }]);
  assert.deepEqual([failed.status, failed.detail], ["warn", "actor:grok-bot's last wake failed: HTTP 401"], "without its host");
  assert.match(failed.fix, /Wake… on the actor sets its webhook again; xenonet actor wake <slug> test/);
});

test("actors: what the observers read, nothing to observe, observers off, and a source XenoNet can't read", async () => {
  const check = async (observers, platform = "darwin") => byId(await runDoctor(ctxWith({ ctx: { platform, actors: { ...ctxWith().actors, observers: () => observers } } }), { only: ["actors.observers"] }))["actors.observers"];
  const fine = await check(OBSERVERS());
  assert.deepEqual([fine.status, fine.detail], ["ok", "Hermes Agent: 2 sources read; Grok Bot: 2 sources read"], "a source that isn't there (no gateway) is no problem");
  const none = await check(OBSERVERS().map((o) => ({ ...o, installed: false })));
  assert.deepEqual([none.status, none.detail], ["info", "Hermes Agent and Grok Bot aren't on this computer, so there's nothing to observe"]);
  const off = await check(OBSERVERS().map((o) => ({ ...o, on: false })));
  assert.deepEqual([off.status, off.detail], ["info", "the observers of Hermes Agent and Grok Bot are off"]);
  const denied = OBSERVERS();
  denied[0].sources[0].state = "denied";
  denied[0].sources[1].state = "unreadable";
  const bad = await check(denied);
  assert.equal(bad.status, "warn");
  assert.equal(bad.detail, "Hermes Agent's session database (~/.hermes/state.db) can't be read; Hermes Agent's spawn ledger (~/.hermes/spawn-ledger.json) isn't in a shape XenoNet reads");
  assert.match(bad.fix, /macOS privacy/);
  assert.equal(byId(await runDoctor(ctxWith({ ctx: { actors: undefined } }), { only: ["actors.observers"] }))["actors.observers"].status, "skip");
});

test("actors: their tokens held to processes on macOS, a note elsewhere, and a product's process the check wouldn't know", async () => {
  const check = async (ctx) => byId(await runDoctor(ctxWith({ ctx }), { only: ["actors.checked"] }))["actors.checked"];
  assert.equal((await check({})).status, "ok");
  assert.match((await check({ platform: "linux" })).detail, /an actor's token alone decides/);
  // Where XenoNet can look, a request it can't see is refused: an actor's,
  // and yours and the window's.
  const blind = await check({ procs: async () => ({ supported: true, peer: false, chain: false, exe: false }) });
  assert.deepEqual([blind.status, blind.detail, blind.fix], ["warn", "netstat doesn't say which process sends a request, so XenoNet refuses every actor's token, and what only you or its window may do, such as allowing or removing actors", "netstat answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose."]);
  const untraced = await check({ procs: async () => ({ supported: true, peer: true, chain: false, exe: false }) });
  assert.deepEqual([untraced.status, untraced.detail.split(",")[0], untraced.fix], ["warn", "ps doesn't say which processes a request comes from", "ps answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose."]);
  // lsof that doesn't say which program a process runs: the window's own
  // routes and Hermes's actor, which rest on real paths, are refused.
  const noExe = await check({ procs: async () => ({ supported: true, peer: true, chain: true, exe: false }) });
  assert.deepEqual([noExe.status, noExe.detail, noExe.fix], ["warn", "lsof doesn't say which program a process runs, so XenoNet refuses Hermes's actor, and what only its window may do, such as allowing actors", "lsof answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose."]);
  assert.equal((await check({ procs: async () => ({ supported: true, peer: true, chain: true, exe: true }) })).status, "ok");
  // Requests it refused because it couldn't see who sent them, since it started.
  const refused = await check({ verify: () => ({ checked: 40, refused: 3, unchecked: 0, unseen: 2, lastRefusal: null }) });
  assert.deepEqual([refused.status, refused.detail], ["info", "an actor's token never works from under a session or an actor app's own process, and a product's only from under that product; since XenoNet started, it refused 2 requests because it couldn't see which program sent them"]);
  assert.match(refused.fix, /exits before the answer can't be seen/);
  assert.match((await check({ verify: () => ({ unseen: 1 }) })).detail, /refused 1 request because it couldn't see which program sent it$/);
  assert.equal((await check({ verify: () => ({ unseen: 0 }) })).status, "ok");
  const odd = OBSERVERS();
  odd[0].recognized = { expected: 2, found: 1, purposes: ["gateway"] };
  const unknown = await check({ actors: { ...ctxWith().actors, observers: () => odd } });
  assert.deepEqual([unknown.status, unknown.detail], ["warn", "Hermes Agent runs gateway processes XenoNet doesn't know as Hermes Agent's, so its actor's token would be refused from under them"]);
  // Off, an observer's self-test doesn't count.
  odd[0].on = false;
  assert.equal((await check({ actors: { ...ctxWith().actors, observers: () => odd } })).status, "ok");
});

test("actors: with actors off, their checks say so rather than asking for XenoNet", async () => {
  const off = byId(await runDoctor(ctxWith({ ctx: { actors: null } }), { only: ["actors"] }));
  assert.deepEqual(Object.values(off).map((r) => [r.id, r.status, r.detail]), ["actors.repos", "actors.expired", "actors.gateway", "actors.wakes", "actors.observers", "actors.checked"].map((id) => [id, "skip", "actors are off here (XENONET_ACTORS=off)"]));
  const alone = byId(await runDoctor(ctxWith({ ctx: { actors: undefined } }), { only: ["actors"] }));
  assert.deepEqual(Object.values(alone).map((r) => r.status), ["skip", "skip", "skip", "skip", "skip", "ok"], "the xenonet command alone can still check the process lookups");
});

test("the launcher check: what hooks and MCP servers run, current and finding XenoNet's Tools", async () => {
  const check = async (launcher) => byId(await runDoctor(ctxWith({ ctx: { launcher: async () => launcher } }), { only: ["xenonet.launcher"] }))["xenonet.launcher"];
  assert.equal((await check(null)).status, "skip", "none from a checkout");
  const dir = `${HOME}/state/runtime/launcher`;
  const good = await check({ dir, version: 2, ships: 2, said: { launcher: 2, protocol: 1, tools: "0.8.0", node: "v24.19.0" } });
  assert.deepEqual([good.status, good.detail], ["ok", "launcher 2, running Tools 0.8.0 on Node 24.19.0"]);
  // The xenonet binary, launcher 3, has no node of its own to name.
  const binary = await check({ dir, version: 3, ships: 3, said: { launcher: 3, protocol: 1, tools: "0.9.0", node: null } });
  assert.deepEqual([binary.status, binary.detail], ["ok", "launcher 3, running Tools 0.9.0"]);
  assert.equal((await check({ dir, version: 1, ships: 2, said: { launcher: 1, protocol: 1, tools: "0.7.0", node: "v24.19.0" } })).status, "warn");
  const noTools = await check({ dir, version: 2, ships: 2, said: { launcher: 2, protocol: 1, tools: null, node: "v24.19.0" } });
  assert.equal(noTools.status, "fail");
  assert.match(noTools.fix, /Open XenoNet to repair them/);
  const broken = await check({ dir, version: 2, ships: 2, said: null });
  assert.equal(broken.status, "fail");
  assert.match(broken.detail, /~\/state\/runtime\/launcher doesn't run/, "the home folder as ~");
});
