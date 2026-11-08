// Putting XenoNet's plugin in Hermes's plugins folder, and what the doctor
// says of it (core/actors/hermes-plugin.js). Every path is a temp HERMES_HOME.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { enabledIn, inspect, install, uninstall, ownerSteps, pluginDir, PLUGIN_FILES } from "../core/actors/hermes-plugin.js";
import { CHECKS } from "../core/doctor/index.js";

function world() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "xn-hermes-plugin-"));
  const env = { HERMES_HOME: path.join(root, "hermes") };
  fs.mkdirSync(env.HERMES_HOME, { recursive: true });
  return { root, env, home: root };
}
const check = (w, now) => CHECKS.find((c) => c.id === "actors.hermes-plugin").run({ home: w.home, env: w.env, exists: fs.existsSync, now: () => now });

test("enabledIn finds xenonet under plugins.enabled in both YAML forms, and nowhere else", () => {
  assert.equal(enabledIn("plugins:\n  enabled:\n    - other\n    - xenonet\n"), true);
  // Hermes's own config puts column-0 comments inside the block.
  assert.equal(enabledIn("plugins:\n  clone: 1\n\n# ===\n# Model\n# ===\n  enabled:\n    - xenonet\n  disabled: []\nmodel:\n"), true);
  assert.equal(enabledIn("plugins:\n  enabled:\n    - other\n# note\n    - xenonet\n"), true);
  assert.equal(enabledIn("plugins:\n  enabled: [other, 'xenonet']\n"), true);
  assert.equal(enabledIn("plugins:\n  enabled: []\n"), false);
  assert.equal(enabledIn("plugins:\n  enabled:\n    - other\nmodel: xenonet\n"), false);
  assert.equal(enabledIn("other:\n  enabled:\n    - xenonet\n"), false);
  assert.equal(enabledIn("plugins:\n  disabled:\n    - xenonet\n"), false);
  assert.equal(enabledIn(null), false);
});

test("install copies the two files under HERMES_HOME, touches nothing else, and uninstall takes the folder out", () => {
  const w = world();
  fs.writeFileSync(path.join(w.env.HERMES_HOME, "config.yaml"), "model: x\n");
  const dir = install(w);
  assert.equal(dir, path.join(w.env.HERMES_HOME, "plugins", "xenonet"));
  assert.deepEqual(fs.readdirSync(dir).sort(), [...PLUGIN_FILES].sort());
  assert.equal(fs.readFileSync(path.join(w.env.HERMES_HOME, "config.yaml"), "utf8"), "model: x\n");
  const st = inspect(w);
  assert.deepEqual([st.installed, st.current, st.enabled, st.token, st.reported], [true, true, false, "none", null]);
  assert.equal(ownerSteps(st).length, 2);
  fs.appendFileSync(path.join(dir, "__init__.py"), "# edited\n");
  assert.equal(inspect(w).current, false);
  assert.equal(uninstall(w), true);
  assert.equal(fs.existsSync(pluginDir(w.home, w.env)), false);
  assert.equal(uninstall(w), false);
});

test("the doctor walks from not installed to reporting", async () => {
  const w = world();
  const now = 1_000_000;
  assert.equal((await check(w, now)).status, "info");
  const dir = install(w);
  let r = await check(w, now);
  assert.match(r.detail, /has no token and isn't turned on/);
  const token = path.join(dir, "token");
  fs.writeFileSync(token, "xenonet_actor_x\n", { mode: 0o644 });
  fs.chmodSync(token, 0o644);
  // Windows has no mode bits to read, so a token file is never "loose" there.
  if (process.platform !== "win32") assert.equal((await check(w, now)).status, "warn");
  fs.chmodSync(token, 0o600);
  fs.writeFileSync(path.join(w.env.HERMES_HOME, "config.yaml"), "plugins:\n  enabled:\n    - xenonet\n");
  assert.match((await check(w, now)).detail, /hasn't reported yet/);
  fs.writeFileSync(path.join(dir, "state.json"), JSON.stringify({ ok: true, at: now - 5000 }));
  assert.equal((await check(w, now)).status, "ok");
  assert.equal((await check(w, now + 10 * 60e3)).status, "info");
  fs.writeFileSync(path.join(dir, "state.json"), JSON.stringify({ ok: false, at: now }));
  assert.equal((await check(w, now)).status, "warn");
  assert.equal((await CHECKS.find((c) => c.id === "actors.hermes-plugin").run({ home: w.home, env: { HERMES_HOME: path.join(w.root, "none") }, exists: fs.existsSync })).status, "ok");
});

test("nothing in the installed plugin or the doctor's text carries a token", () => {
  const w = world();
  const dir = install(w);
  for (const f of PLUGIN_FILES) assert.doesNotMatch(fs.readFileSync(path.join(dir, f), "utf8"), /xenonet_actor_[A-Za-z0-9]{20,}/);
});
