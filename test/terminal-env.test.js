// A shell XenoNet opens in its terminal drawer (server/terminals.js) doesn't
// pass on what XenoNet set for its own processes, so XenoNet Dev started there
// doesn't take the installed XenoNet's port or folders, and the reverse.
import { test } from "node:test";
import assert from "node:assert/strict";
import { shellEnv, OWN_ENV } from "../server/terminals.js";

test("a terminal's shell gets your environment without XenoNet's own settings", () => {
  const base = { PATH: "/usr/bin", HOME: "/Users/nova", XENONET_PROJECTS_DIR: "/Users/nova/Code", ELECTRON_RUN_AS_NODE: "1", NODE_OPTIONS: "--inspect", ...Object.fromEntries(OWN_ENV.map((k) => [k, "x"])) };
  const env = shellEnv({}, base);
  assert.deepEqual(OWN_ENV.filter((k) => k in env), [], "none of XenoNet's own");
  assert.equal(env.ELECTRON_RUN_AS_NODE, undefined);
  assert.equal(env.NODE_OPTIONS, undefined);
  assert.deepEqual([env.PATH, env.HOME, env.XENONET_PROJECTS_DIR, env.TERM_PROGRAM], ["/usr/bin", "/Users/nova", "/Users/nova/Code", "XenoNet"], "yours stay");
  assert.equal(shellEnv({ XENONET_PORT: "47842" }, base).XENONET_PORT, "47842", "unless the terminal's profile sets one itself");
});
