// Where headless runs find Claude Code and Codex (findCli and
// bundledClaudeCode in electron/lib/dirs.js). Made-up folders; nothing is
// run. Paths are this system's own, so the same cases hold on Windows.
import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { EventEmitter } from "node:events";
import { findCli, bundledClaudeCode } from "../electron/lib/dirs.js";

const p = (...parts) => path.join(path.sep, ...parts);
const PATHS = (...dirs) => dirs.join(path.delimiter);

// Made-up folders: the files there, by path, with their mtimes.
function disk(files) {
  const paths = Object.keys(files);
  return {
    exists: (f) => paths.some((x) => x === f || x.startsWith(`${f}${path.sep}`)),
    readdir: (d) => {
      const names = [...new Set(paths.filter((x) => x.startsWith(`${d}${path.sep}`)).map((x) => x.slice(d.length + 1).split(path.sep)[0]))];
      if (!names.length) throw Object.assign(new Error(`ENOENT: ${d}`), { code: "ENOENT" });
      return names;
    },
    mtime: (f) => files[f] ?? NaN,
    canRun: (f) => {
      if (!(f in files)) throw new Error(`EACCES: ${f}`);
    },
  };
}

test("Claude Code is found on PATH, then where its installer puts it, then where Claude desktop keeps it (the newest verified build), then at XENONET_CLAUDE_BIN", () => {
  const home = p("Users", "jo");
  const support = path.join(home, "Library", "Application Support", "Claude", "claude-code");
  const binary = (version, build) => path.join(support, version, build, "claude.app", "Contents", "MacOS", "claude");
  const verified = (version, build) => path.join(support, version, build, ".verified");
  const d = disk({
    [binary("2.1.308", "a93ded4b1b47")]: 2,
    [verified("2.1.308", "a93ded4b1b47")]: 2,
    [binary("2.1.99", "4fa9694b7975")]: 3,
    [verified("2.1.99", "4fa9694b7975")]: 3,
    // Newer, but not verified: a download under way.
    [binary("2.1.308", "b0")]: 9,
    [p("opt", "bin", "claude")]: 1,
  });
  assert.deepEqual(bundledClaudeCode(path.join(home, "Library", "Application Support", "Claude"), { platform: "darwin", ...d }), [binary("2.1.308", "a93ded4b1b47"), binary("2.1.99", "4fa9694b7975")], "2.1.308 is newer than 2.1.99");
  const unverified = disk({ [binary("2.2.0", "new")]: 2, [binary("2.1.0", "old")]: 1, [verified("2.1.0", "old")]: 1 });
  assert.deepEqual(bundledClaudeCode(path.join(home, "Library", "Application Support", "Claude"), { platform: "darwin", ...unverified }), [binary("2.1.0", "old")], "the newest verified build");
  const at = (env, look = d, platform = "darwin") => findCli("claude", { env, platform, homeDir: home, ...look });
  assert.equal(at({ PATH: PATHS(p("usr", "bin"), p("opt", "bin")) }), p("opt", "bin", "claude"), "PATH first");
  const installed = disk({ [path.join(home, ".local", "bin", "claude")]: 1, [binary("2.1.308", "a93ded4b1b47")]: 2, [verified("2.1.308", "a93ded4b1b47")]: 2 });
  assert.equal(at({ PATH: p("usr", "bin") }, installed), path.join(home, ".local", "bin", "claude"), "then where its installer puts it");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: p("x", "claude") }), binary("2.1.308", "a93ded4b1b47"), "then Claude desktop's own");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: p("x", "claude") }, disk({})), p("x", "claude"), "then its variable");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: "" }, disk({})), null, "or nowhere");
  // Windows: PATHEXT finds an npm install's claude.cmd; Claude desktop's
  // MSIX folder holds its claude.exe.
  assert.equal(at({ PATH: p("npm"), PATHEXT: ".EXE;.CMD" }, disk({ [p("npm", "claude.CMD")]: 1 }), "win32"), p("npm", "claude.CMD"));
  // Its installer's claude.exe, which it leaves off PATH.
  assert.equal(at({ PATH: p("npm"), PATHEXT: ".EXE;.CMD" }, disk({ [path.join(home, ".local", "bin", "claude.exe")]: 1 }), "win32"), path.join(home, ".local", "bin", "claude.exe"));
  const msix = path.join("L", "Packages", "Claude_pzs8sxrjxfjjc", "LocalCache", "Roaming", "Claude", "claude-code");
  const win = disk({
    [path.join(msix, "1.0.0", "x", "claude.exe")]: 1,
    [path.join(msix, "1.0.0", "x", ".verified")]: 1,
    [path.join(msix, "1.0.0", "y", "bin", "claude.exe")]: 2,
    [path.join(msix, "1.0.0", "y", ".verified")]: 2,
    [path.join(msix, "0.9.0", "z", "claude.exe")]: 9,
    [path.join(msix, "0.9.0", "z", ".verified")]: 9,
  });
  assert.equal(at({ PATH: p("npm"), PATHEXT: ".EXE;.CMD", LOCALAPPDATA: "L" }, win, "win32"), path.join(msix, "1.0.0", "y", "bin", "claude.exe"), "the newest version's newest build");
});

test("Codex is found by XENONET_CODEX_BIN, then in the ChatGPT app, then on PATH; nothing else is found", () => {
  const app = path.join("/Applications", "ChatGPT.app", "Contents", "Resources", "codex");
  const d = disk({ [app]: 1, [p("usr", "bin", "codex")]: 1, [p("usr", "bin", "opencode")]: 1 });
  const at = (cli, env, platform = "darwin") => findCli(cli, { env, platform, homeDir: p("Users", "jo"), ...d });
  assert.equal(at("codex", { PATH: p("usr", "bin"), XENONET_CODEX_BIN: p("c", "codex") }), p("c", "codex"));
  assert.equal(at("codex", { PATH: p("usr", "bin") }), app);
  assert.equal(at("codex", { PATH: p("usr", "bin") }, "linux"), p("usr", "bin", "codex"));
  assert.equal(at("opencode", { PATH: p("usr", "bin") }, "linux"), null);
});

// Processes that say what they're told to and exit when they're told to;
// every program started is listed, taskkill too.
function fakeSpawn() {
  const started = [];
  const calls = [];
  let pid = 100;
  const spawn = (program, args, opts) => {
    calls.push({ program, args });
    const child = new EventEmitter();
    child.pid = ++pid;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = { on() {}, end: (text) => started.push({ program, args, opts, text, child }) };
    child.kill = (signal) => ((child.signal = signal ?? "SIGTERM"), child.emit("exit", null, child.signal));
    setImmediate(() => child.emit("spawn"));
    return child;
  };
  return { spawn, started, calls };
}

