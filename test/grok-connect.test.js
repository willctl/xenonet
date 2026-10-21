// Connecting Grok Build (core/engine/connect.js's grok-hooks, and toml.js):
// the xenonet MCP server as tables in config.toml, with the rest of the
// file as it was, and XenoNet's hooks in a file of its own in Grok's hooks
// folder; each XenoNet's entries its own; and all of it taken out again.
// A temporary home stands in for yours.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createConnect } from "../core/engine/connect.js";
import { keyParts, tomlLines, readServer, withServer, withoutServer, foreignServer } from "../core/engine/toml.js";

const MINE = `# Grok's settings, as you keep them
[cli]
installer = "internal" # kept as it is

[mcp_servers.linear]
command = "npx"
args = ["-y", "linear-mcp"]
`;

function home(t, config = MINE) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-connect-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  fs.mkdirSync(path.join(dir, ".grok"));
  if (config != null) fs.writeFileSync(path.join(dir, ".grok", "config.toml"), config, { mode: 0o600 });
  return dir;
}

const connectAt = (dir, port, bin = "/xenonet/bin/xenonet") => createConnect({ platform: "darwin", port, bin, home: dir, env: { PATH: "" }, run: async (_file, args) => (args[1] === "get" ? Promise.reject(new Error("not found")) : "") });
const grokOf = (preview) => preview.apps.find((a) => a.app === "grok");
const read = (dir, file) => fs.readFileSync(path.join(dir, ".grok", file), "utf8");

test("TOML: headers, keys and values as XenoNet reads and writes them", () => {
  assert.deepEqual(keyParts('mcp_servers."xenonet-dev" . env'), ["mcp_servers", "xenonet-dev", "env"]);
  assert.deepEqual([keyParts("a."), keyParts("a b"), keyParts("1, 2")], [null, null, null]);
  assert.deepEqual(tomlLines('x = """\n[not.a.header]\n"""\n[cli]').map((l) => l.header), [false, false, false, true]);
  const env = { XENONET_PORT: "47840" };
  const text = withServer(MINE, "xenonet", { command: "/bin/xenonet", args: ["mcp"], env });
  assert.equal(text, `${MINE.trimEnd()}\n\n[mcp_servers.xenonet]\ncommand = "/bin/xenonet"\nargs = ["mcp"]\n\n[mcp_servers.xenonet.env]\nXENONET_PORT = "47840"\n`);
  assert.deepEqual(readServer(text, "xenonet"), { command: "/bin/xenonet", args: ["mcp"], env });
  assert.deepEqual(readServer(text, "linear"), { command: "npx", args: ["-y", "linear-mcp"], env: null });
  assert.equal(withServer(text, "xenonet", { command: "/bin/xenonet", args: ["mcp"], env }), text, "again, the same");
  assert.equal(withoutServer(text, "xenonet").trimEnd(), MINE.trimEnd());
  assert.equal(readServer(MINE, "xenonet"), null);
  for (const other of ['[mcp_servers]\nxenonet = { command = "x" }', 'mcp_servers.xenonet.command = "x"']) {
    assert.ok(foreignServer(other, "xenonet"), other);
    assert.throws(() => withServer(other, "xenonet", { command: "x" }), /in a form XenoNet doesn't edit/);
  }
});

test("Grok Build connects through config.toml and its hooks folder, and disconnects again, each XenoNet its own", async (t) => {
  const dir = home(t);
  // The binaries the entries run, which are there: an entry whose files
  // are gone is anyone's to clear.
  const bins = ["installed", "dev"].map((n) => {
    const f = path.join(dir, n, "xenonet");
    fs.mkdirSync(path.dirname(f));
    fs.writeFileSync(f, "");
    return f;
  });
  const installed = connectAt(dir, 47840, bins[0]);
  const before = grokOf(await installed.preview());
  assert.deepEqual([before.installed, before.changes.map((c) => [c.id, c.state])], [true, [["mcp", "missing"], ["hooks", "missing"]]]);
  assert.ok(before.notes.some((n) => /can't find the grok command/.test(n)));

  const after = grokOf(await installed.apply(["grok"]));
  assert.deepEqual(after.changes.map((c) => c.state), ["present", "present"]);
  const config = read(dir, "config.toml");
  assert.ok(config.startsWith(MINE.trimEnd()), "your settings and comments, as they were");
  assert.deepEqual(readServer(config, "xenonet"), { command: bins[0], args: ["mcp"], env: { XENONET_PORT: "47840" } });
  assert.equal(read(dir, "config.toml.before-xenonet"), MINE, "a copy as it was, the first time");
  if (process.platform !== "win32") assert.equal(fs.statSync(path.join(dir, ".grok", "config.toml")).mode & 0o777, 0o600, "its mode kept");
  const hooks = JSON.parse(read(dir, "hooks/xenonet.json")).hooks;
  assert.deepEqual(Object.keys(hooks), ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "Stop", "StopCancelled", "StopFailure", "Notification", "SessionEnd"]);
  assert.equal(hooks.PreToolUse[0].matcher, "Edit|Write|MultiEdit|search_replace|write|edit|hashline_edit|apply_patch");
  assert.equal(hooks.Notification[0].matcher, "permission_prompt");
  assert.deepEqual(hooks.Stop[0].hooks, [{ type: "command", command: `XENONET_PORT=47840 '${bins[0]}' hook grok`, timeout: 10 }]);
  assert.equal(hooks.SessionEnd[0].hooks[0].timeout, 3);

  // XenoNet Dev beside it: its own server, by its own name, and its own hooks.
  const dev = connectAt(dir, 47842, bins[1]);
  await dev.apply(["grok"]);
  const both = read(dir, "config.toml");
  assert.deepEqual([readServer(both, "xenonet")?.env, readServer(both, "xenonet-dev")?.env], [{ XENONET_PORT: "47840" }, { XENONET_PORT: "47842" }]);
  assert.deepEqual(JSON.parse(read(dir, "hooks/xenonet.json")).hooks.Stop.flatMap((g) => g.hooks).map((h) => /XENONET_PORT=(\d+)/.exec(h.command)[1]), ["47840", "47842"], "both XenoNets' Stop hooks");
  assert.ok(grokOf(await installed.preview()).notes.some((n) => /Another XenoNet \(port 47842, XenoNet Dev\)/.test(n)));

  // Disconnecting one takes out its own, and the other's stay.
  await installed.remove();
  assert.equal(readServer(read(dir, "config.toml"), "xenonet"), null);
  assert.ok(readServer(read(dir, "config.toml"), "xenonet-dev"));
  assert.ok(fs.existsSync(path.join(dir, ".grok", "hooks", "xenonet.json")));
  await dev.remove();
  assert.equal(read(dir, "config.toml").trimEnd(), MINE.trimEnd(), "the file as you had it");
  assert.equal(fs.existsSync(path.join(dir, ".grok", "hooks", "xenonet.json")), false, "the hooks file goes with the last of them");
});

test("a server written another way is left alone, and Grok Build that isn't here is left out of connecting everything", async (t) => {
  const dir = home(t, '[mcp_servers]\nxenonet = { command = "mine" }\n');
  const connect = connectAt(dir, 47840);
  const g = grokOf(await connect.preview());
  assert.equal(g.changes.find((c) => c.id === "mcp").state, "different");
  assert.ok(g.notes.some((n) => /names mcp_servers\.xenonet in a form XenoNet doesn't edit/.test(n)), g.notes.join(" | "));
  await assert.rejects(connect.apply(["grok"]), /in a form XenoNet doesn't edit/);
  assert.equal(read(dir, "config.toml"), '[mcp_servers]\nxenonet = { command = "mine" }\n');

  const elsewhere = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-none-"));
  t.after(() => fs.rmSync(elsewhere, { recursive: true, force: true }));
  const none = connectAt(elsewhere, 47840);
  assert.equal(grokOf(await none.preview()).installed, false);
  await none.apply();
  assert.equal(fs.existsSync(path.join(elsewhere, ".grok")), false, "nothing written for an app that isn't here");
  // GROK_HOME moves it.
  const moved = path.join(elsewhere, "grok-home");
  fs.mkdirSync(moved);
  const there = createConnect({ platform: "darwin", port: 47840, bin: "/x/xenonet", home: elsewhere, env: { PATH: "", GROK_HOME: moved }, run: async () => "" });
  await there.apply(["grok"]);
  assert.ok(fs.existsSync(path.join(moved, "config.toml")) && fs.existsSync(path.join(moved, "hooks", "xenonet.json")));
});
