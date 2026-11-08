import { test } from "node:test";
import assert from "node:assert/strict";
import { pathKey, isUnder, scratchZone } from "../electron/lib/paths.js";
const windows = { activeWindowMs: 10 * 60e3, recentWindowMs: 24 * 3600e3 };
const now = Date.parse("2026-10-02T12:00:00Z");
const project = (name, extra = {}) => ({ name, path: `C:\\Users\\me\\Projects\\${name}`, key: pathKey(`C:\\Users\\me\\Projects\\${name}`), git: null, stack: [], ...extra });
const session = (id, cwd, extra = {}) => ({ id, app: id.split(":")[0], rawId: id.split(":")[1], title: id, cwd, updatedAt: now - 3600e3, ...extra });

test("pathKey normalizes Windows spellings of the same folder", () => {
  assert.equal(pathKey("\\\\?\\C:\\Users\\Me\\Projects\\x\\"), "c:/users/me/projects/x");
  assert.equal(pathKey("c:/users/me/projects/x"), "c:/users/me/projects/x");
  assert.ok(isUnder("c:/a/b/c", "c:/a/b"));
  assert.ok(!isUnder("c:/a/bc", "c:/a/b"));
});

test("scratch folders from both apps collapse into one zone each", () => {
  assert.equal(scratchZone(pathKey("C:\\Users\\me\\AppData\\Roaming\\Claude\\scratch-workspaces\\a\\b\\scratch-1"))?.id, "zone:claude-scratch");
  assert.equal(scratchZone(pathKey("\\\\?\\C:\\Users\\me\\Documents\\Codex\\2026-10-01\\scratch-task"))?.id, "zone:codex-projectless");
  assert.equal(scratchZone(pathKey("C:\\Users\\me\\Projects\\x")), null);
});

test("paths under home show as ~, and only whole folder names match", async () => {
  const { tilde } = await import("../electron/lib/paths.js");
  const { setHome, tildePath } = await import("../src/format.js");
  const r = String.raw;
  assert.equal(tilde(r`Read C:\Users\me\Projects\a.js`, r`C:\Users\me`), r`Read ~\Projects\a.js`);
  assert.equal(tilde("cd c:/users/me/x && ls", r`C:\Users\me`), "cd ~/x && ls");
  assert.equal(tilde(r`C:\Users\meg\x`, r`C:\Users\me`), r`C:\Users\meg\x`);
  assert.equal(tilde("/home/me/src", "/home/me/"), "~/src");
  setHome(r`C:\Users\me`);
  assert.equal(tildePath(r`\\?\C:\Users\me\Projects\a`), r`~\Projects\a`);
  assert.equal(tildePath(r`C:\Users\meg`), r`C:\Users\meg`);
  setHome(null);
});

