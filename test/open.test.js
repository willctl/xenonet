import { test } from "node:test";
import assert from "node:assert/strict";
import { actionsFor, urlFor, handoffUrl, revealCommand } from "../core/open.js";

const codex = { kind: "session", app: "codex", rawId: "0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b", cwd: "C:\\p" };
const desktopClaude = { kind: "session", app: "claude", rawId: "5e6f7a8b-9c0d-4e1f-a2b3-c4d5e6f7a8b9", desktopId: "local_aabbccdd-eeff-4011-8223-344556677889", cwd: "C:\\p" };
const cliClaude = { kind: "session", app: "claude", rawId: "5e6f7a8b-9c0d-4e1f-a2b3-c4d5e6f7a8b9", desktopId: null, cwd: "C:\\p" };

test("desktop deep links per app", () => {
  assert.equal(urlFor(codex, "open-app"), "codex://threads/0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b");
  assert.equal(urlFor(desktopClaude, "open-app"), "claude://code/continue?session=local_aabbccdd-eeff-4011-8223-344556677889");
  assert.equal(urlFor(cliClaude, "open-app"), "claude://resume?session=5e6f7a8b-9c0d-4e1f-a2b3-c4d5e6f7a8b9");
  assert.match(actionsFor(cliClaude)[0].label, /Import/);
});

test("ids that don't look like ids never become URLs", () => {
  assert.equal(urlFor({ ...codex, rawId: "x&calc.exe" }, "open-app"), null);
  assert.equal(urlFor({ ...desktopClaude, desktopId: "local_../../x", rawId: "nope" }, "open-app"), null);
  assert.deepEqual(actionsFor({ ...codex, rawId: "bad" }).map((a) => a.id), ["preview", "stats", "handoff", "open-folder"]);
});

test("handoff links prefill a new session in either app", () => {
  const brief = "Continue: fix & ship\nline 2";
  const c = new URL(handoffUrl("claude", "C:\\Projects\\a b", brief));
  assert.equal(c.protocol + "//" + c.host + c.pathname, "claude://code/new");
  assert.equal(c.searchParams.get("folder"), "C:\\Projects\\a b");
  assert.equal(c.searchParams.get("q"), brief);
  const x = new URL(handoffUrl("codex", "C:\\p", brief));
  assert.equal(x.protocol + "//" + x.host + x.pathname, "codex://threads/new");
  assert.equal(x.searchParams.get("prompt"), brief);
  assert.equal(x.searchParams.get("path"), "C:\\p");
  assert.equal(handoffUrl("other", "C:\\p", brief), null);
});

test("project actions target the project folder", () => {
  const p = { kind: "project", path: "C:\\Users\\me\\Projects\\a b" };
  assert.equal(urlFor(p, "open-folder"), "C:\\Users\\me\\Projects\\a b");
  assert.equal(urlFor(p, "open-vscode"), "vscode://file/C:/Users/me/Projects/a b");
  assert.equal(urlFor(p, "new-claude"), "claude://code/new?folder=C%3A%5CUsers%5Cme%5CProjects%5Ca%20b");
  assert.equal(urlFor(p, "bogus"), null);
});

test("macOS project actions use POSIX paths", () => {
  const p = { kind: "project", path: "/Users/me/Projects/a b#2" };
  assert.equal(urlFor(p, "open-folder"), "/Users/me/Projects/a b#2");
  assert.equal(urlFor(p, "open-vscode"), "vscode://file/Users/me/Projects/a%20b%232");
  assert.equal(new URL(urlFor(p, "open-vscode")).pathname, "/Users/me/Projects/a%20b%232");
  assert.equal(new URL(urlFor(p, "new-claude")).searchParams.get("folder"), "/Users/me/Projects/a b#2");
});

test("a file shown in the file manager: selected in Finder or Explorer, its folder elsewhere", () => {
  assert.deepEqual(revealCommand("/Users/will/state/themes/mine.json", "darwin"), ["open", ["-R", "/Users/will/state/themes/mine.json"]]);
  assert.deepEqual(revealCommand("C:\\Users\\will\\themes\\mine.json", "win32"), ["explorer.exe", ["/select,C:\\Users\\will\\themes\\mine.json"]]);
  assert.deepEqual(revealCommand("/home/will/.local/state/xenonet/themes/mine.json", "linux"), ["xdg-open", ["/home/will/.local/state/xenonet/themes"]]);
});
