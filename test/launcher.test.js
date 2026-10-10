// XenoNet's stable launcher (core/components/launcher.cjs), which hooks, MCP
// servers and the xenonet command run from a path that never moves: a hook
// goes to XenoNet by itself, quickly, and never fails its agent; it says what
// it is; without Tools it says so in a line; and XenoNet writes a newer one
// over an older one, never the other way.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn } from "node:child_process";
import { writeLauncher, launcherVersion, LAUNCHER_SOURCE } from "../core/components/install.js";

const SOURCE = fs.readFileSync(LAUNCHER_SOURCE, "utf8");

function runtime() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-launcher-"));
  const entry = path.join(dir, "launcher", "cli", "xenonet.js");
  writeLauncher(entry);
  return { dir, entry, done: () => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5 }) };
}

function run(entry, args, { input = "", env = {} } = {}) {
  return new Promise((resolve) => {
    const started = Date.now();
    const child = spawn(process.execPath, [entry, ...args], { env: { ...process.env, ...env }, windowsHide: true });
    let stdout = "", stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => resolve({ code, stdout, stderr, ms: Date.now() - started }));
    child.stdin.end(input);
  });
}

test("a hook goes to the running XenoNet by itself, and XenoNet's answer is what the agent hears", async () => {
  const r = runtime();
  const heard = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      heard.push({ url: req.url, body });
      res.writeHead(200, { "content-type": "application/json" });
      res.end('{"systemMessage":"from XenoNet"}');
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  try {
    const { port } = server.address();
    const out = await run(r.entry, ["hook", "codex"], { input: '{"hook_event_name":"Stop"}', env: { XENONET_PORT: String(port) } });
    assert.deepEqual([out.code, out.stdout, out.stderr], [0, '{"systemMessage":"from XenoNet"}', ""]);
    assert.deepEqual(heard, [{ url: "/api/hook/codex", body: '{"hook_event_name":"Stop"}' }]);
    const claude = await run(r.entry, ["hook", "claude", "--port", String(port)], { input: "" });
    assert.equal(claude.code, 0);
    assert.equal(heard.at(-1).url, "/api/hook/claude");
    assert.equal(heard.at(-1).body, "{}", "no event is an empty one");
  } finally {
    server.close();
    r.done();
  }
});

test("with no XenoNet there, or no Tools installed, a hook prints nothing and exits 0", async () => {
  const r = runtime();
  try {
    const free = await new Promise((ok) => {
      const s = http.createServer().listen(0, "127.0.0.1", () => {
        const { port } = s.address();
        s.close(() => ok(port));
      });
    });
    const out = await run(r.entry, ["hook", "codex"], { input: "{}", env: { XENONET_PORT: String(free) } });
    assert.deepEqual([out.code, out.stdout, out.stderr], [0, "", ""]);
    assert.ok(out.ms < 4000, `quickly (${out.ms} ms)`);
  } finally {
    r.done();
  }
});

test("it says what it is, and without Tools any other command says so in a line", async () => {
  const r = runtime();
  try {
    const probe = await run(r.entry, ["--launcher"]);
    const said = JSON.parse(probe.stdout);
    assert.deepEqual([said.launcher, said.protocol, said.tools], [launcherVersion(SOURCE), 1, null]);
    const mcp = await run(r.entry, ["mcp"]);
    assert.equal(mcp.code, 1);
    assert.match(mcp.stderr, /^XenoNet's Tools aren't installed here/);
    assert.equal(mcp.stderr.trim().split("\n").length, 1, "a line, not a stack trace");
  } finally {
    r.done();
  }
});

test("XenoNet writes a newer launcher over an older one, never an older over a newer", () => {
  const r = runtime();
  try {
    assert.equal(launcherVersion(SOURCE) >= 2, true);
    assert.equal(launcherVersion("// XenoNet's stable command launcher. Component files stay versioned.\n"), 1, "the first, which said no version");
    assert.equal(launcherVersion(""), 0);
    fs.writeFileSync(r.entry, "// XenoNet's stable command launcher. Component files stay versioned.\nold();\n");
    assert.equal(writeLauncher(r.entry), true, "the old one is replaced");
    assert.equal(fs.readFileSync(r.entry, "utf8"), SOURCE);
    assert.equal(writeLauncher(r.entry), false, "the same one stays");
    const newer = SOURCE.replace(/^\/\/ xenonet-launcher \d+/, "// xenonet-launcher 99");
    fs.writeFileSync(r.entry, newer);
    assert.equal(writeLauncher(r.entry), false, "a newer one, from a newer XenoNet, stays");
    assert.equal(fs.readFileSync(r.entry, "utf8"), newer);
    assert.deepEqual(fs.readdirSync(path.dirname(r.entry)), ["xenonet.js"], "no temporary file left");
  } finally {
    r.done();
  }
});
