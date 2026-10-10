import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { findClaudeDesktopDir, bundledCodexBinaries } from "../core/dirs.js";
import { pathKey, scratchZone } from "../core/paths.js";
import { gitAvailable, resetGitCheck } from "../core/git.js";
import { onPath } from "../core/paths.js";
import { commandText } from "../core/live.js";

const H = "/Users/me";
const has = (...files) => (f) => files.includes(f);

test("Claude desktop's data folder on each platform", () => {
  assert.equal(findClaudeDesktopDir({ env: {}, platform: "darwin", homeDir: H, exists: has(path.join(H, "Library", "Application Support", "Claude")) }), path.join(H, "Library", "Application Support", "Claude"));
  assert.equal(findClaudeDesktopDir({ env: {}, platform: "darwin", homeDir: H, exists: () => false }), null);
  assert.equal(findClaudeDesktopDir({ env: {}, platform: "linux", homeDir: H, exists: () => true }), path.join(H, ".config", "Claude"));
  assert.equal(findClaudeDesktopDir({ env: { XDG_CONFIG_HOME: "/x" }, platform: "linux", homeDir: H, exists: () => true }), path.join("/x", "Claude"));
  // Windows: the MSIX package's redirected folder wins over %APPDATA%.
  const msix = path.join("L", "Packages", "Claude_abc", "LocalCache", "Roaming", "Claude");
  assert.equal(findClaudeDesktopDir({ env: { LOCALAPPDATA: "L", APPDATA: "R" }, platform: "win32", homeDir: H, exists: () => true, readdir: () => ["Other_1", "Claude_abc"] }), msix);
  assert.equal(findClaudeDesktopDir({ env: { LOCALAPPDATA: "L", APPDATA: "R" }, platform: "win32", homeDir: H, exists: has(path.join("R", "Claude")), readdir: () => [] }), path.join("R", "Claude"));
  assert.equal(findClaudeDesktopDir({ env: { XENONET_CLAUDE_DESKTOP_DIR: "/c" }, platform: "darwin" }), "/c");
});

test("the codex binary bundled with the ChatGPT desktop app", () => {
  const sys = path.join("/Applications", "ChatGPT.app", "Contents", "Resources", "codex");
  const mine = path.join(H, "Applications", "ChatGPT.app", "Contents", "Resources", "codex");
  assert.deepEqual(bundledCodexBinaries({ platform: "darwin", homeDir: H, exists: has(sys, mine) }), [sys, mine]);
  assert.deepEqual(bundledCodexBinaries({ platform: "darwin", homeDir: H, exists: () => false }), []);
  const bin = path.join("L", "OpenAI", "Codex", "bin");
  const times = { [path.join(bin, "old", "codex.exe")]: 1, [path.join(bin, "new", "codex.exe")]: 2 };
  assert.deepEqual(
    bundledCodexBinaries({ env: { LOCALAPPDATA: "L" }, platform: "win32", homeDir: H, exists: (f) => f in times, readdir: () => ["old", "new", "empty"], mtime: (f) => times[f] }),
    [path.join(bin, "new", "codex.exe"), path.join(bin, "old", "codex.exe")],
  );
  assert.deepEqual(bundledCodexBinaries({ platform: "linux", homeDir: H }), []);
});

test("macOS paths through /private match their short form", () => {
  assert.equal(pathKey("/private/var/folders/ab/xyz/T/x"), pathKey("/var/folders/ab/xyz/T/x"));
  assert.equal(pathKey("/private/tmp/scratch/"), "/tmp/scratch");
  assert.equal(pathKey("/private/data/x"), "/private/data/x");
  assert.equal(scratchZone(pathKey("/private/var/folders/ab/xyz/T/claude-run/x"))?.id, "zone:temp");
  assert.equal(scratchZone(pathKey("/var/folders/ab/xyz/C/cache"))?.id, undefined);
  assert.equal(scratchZone(pathKey("/private/tmp"))?.id, "zone:temp");
  assert.equal(scratchZone(pathKey("/tmpfiles/x"))?.id, undefined);
  assert.equal(scratchZone(pathKey("C:\\Users\\me\\AppData\\Local\\Temp"))?.id, "zone:temp");
  assert.equal(scratchZone(pathKey("/Users/me/Library/Application Support/Claude/scratch-workspaces/abc"))?.id, "zone:claude-scratch");
  assert.equal(scratchZone(pathKey("/Users/me/Documents/Codex/2026-10-02/regex"))?.id, "zone:codex-projectless");
});

test("git is skipped on a Mac whose /usr/bin/git is the developer-tools stub", () => {
  const run = (opts) => (resetGitCheck(), gitAvailable(opts));
  const stub = () => "/usr/bin/git";
  assert.equal(run({ platform: "darwin", which: stub, devTools: () => {} }), true);
  assert.equal(run({ platform: "darwin", which: stub, devTools: () => { throw new Error("no tools"); } }), false);
  assert.equal(run({ platform: "darwin", which: () => "/opt/homebrew/bin/git", devTools: () => { throw new Error("no tools"); } }), true);
  assert.equal(run({ platform: "linux", which: () => null }), false);
  resetGitCheck();
});

test("onPath finds the first runnable match", () => {
  const env = { PATH: ["/a", "/b", "/c"].join(path.delimiter) };
  assert.equal(onPath("git", { env, platform: "darwin", canRun: (f) => { if (f !== path.join("/b", "git")) throw new Error(); } }), path.join("/b", "git"));
  assert.equal(onPath("git", { env, platform: "darwin", canRun: () => { throw new Error(); } }), null);
});

test("commands run through zsh and bash show their script", () => {
  assert.equal(commandText(["/bin/zsh", "-lc", "npm test"]), "npm test");
  assert.equal(commandText(["bash", "-c", "ls -la"]), "ls -la");
});

test("listFiles: the files a name test picks, down to a depth, past folders it can't read", async (t) => {
  const { listFiles } = await import("../core/paths.js");
  const fs = await import("node:fs");
  const os = await import("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-list-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const rel of ["a.jsonl", "b.txt", "x/c.jsonl", "x/y/d.jsonl", "x/y/z/e.jsonl", "dir.jsonl/f.jsonl"]) {
    fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true });
    fs.writeFileSync(path.join(root, rel), "");
  }
  const rel = (files) => files.map((f) => path.relative(root, f).split(path.sep).join("/")).sort();
  const jsonl = (name) => name.endsWith(".jsonl");
  assert.deepEqual(rel(await listFiles(root, jsonl)), ["a.jsonl", "dir.jsonl/f.jsonl", "x/c.jsonl", "x/y/d.jsonl", "x/y/z/e.jsonl"]);
  assert.deepEqual(rel(await listFiles(root, jsonl, 1)), ["a.jsonl", "dir.jsonl/f.jsonl", "x/c.jsonl"], "a folder named like a file is still a folder");
  assert.deepEqual(rel(await listFiles(root, jsonl, 0)), ["a.jsonl"]);
  assert.deepEqual(await listFiles(path.join(root, "missing"), jsonl), []);
});
