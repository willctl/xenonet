// XenoNet's stable launcher (crates/xenonet-components/src/launcher.cjs), which hooks, MCP
// servers and the xenonet command run from a path that never moves: a hook
// goes to XenoNet by itself, quickly, and never fails its agent; it says what
// it is; and without Tools it says so in a line.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn } from "node:child_process";

// The script the binary writes as the launcher (LAUNCHER_SCRIPT in
// crates/xenonet-components/src/install.rs, whose tests cover writing it).
const SOURCE = fs.readFileSync(new URL("../crates/xenonet-components/src/launcher.cjs", import.meta.url), "utf8");
// The version its first line says (// xenonet-launcher <n>).
const VERSION = Number(/^\/\/ xenonet-launcher (\d+)/.exec(SOURCE)[1]);

function runtime() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-launcher-"));
  const entry = path.join(dir, "launcher", "cli", "xenonet.js");
  fs.mkdirSync(path.dirname(entry), { recursive: true });
  fs.writeFileSync(entry, SOURCE);
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
      heard.push({ url: req.url, body, credential: req.headers["x-xenonet-credential"] ?? null });
      res.writeHead(200, { "content-type": "application/json" });
      res.end('{"systemMessage":"from XenoNet"}');
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  try {
    const { port } = server.address();
    // The API's credential the XenoNet on that port wrote in the runtime
    // folder goes with it.
    fs.writeFileSync(path.join(r.dir, `api-${port}.cred`), "0a1b2c\n");
    const out = await run(r.entry, ["hook", "codex"], { input: '{"hook_event_name":"Stop"}', env: { XENONET_PORT: String(port), XENONET_RUNTIME_DIR: "" } });
    assert.deepEqual([out.code, out.stdout, out.stderr], [0, '{"systemMessage":"from XenoNet"}', ""]);
    assert.deepEqual(heard, [{ url: "/api/hook/codex", body: '{"hook_event_name":"Stop"}', credential: "0a1b2c" }]);
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
    assert.deepEqual([said.launcher, said.protocol, said.tools], [VERSION, 1, null]);
    const mcp = await run(r.entry, ["mcp"]);
    assert.equal(mcp.code, 1);
    assert.match(mcp.stderr, /^XenoNet's Tools aren't installed here/);
    assert.equal(mcp.stderr.trim().split("\n").length, 1, "a line, not a stack trace");
  } finally {
    r.done();
  }
});

test("with Tools that carry the xenonet binary, it runs the binary, and tells it where the launcher's own copy is", { skip: process.platform === "win32" ? "a shell script stands in for the binary" : false }, async () => {
  const r = runtime();
  try {
    const sha = "a".repeat(64);
    const tools = path.join(r.dir, "components", "tools", `1.2.3-${sha.slice(0, 16)}`);
    fs.mkdirSync(path.join(tools, "bin"), { recursive: true });
    fs.writeFileSync(path.join(r.dir, "components", "tools", "current.json"), JSON.stringify({ version: "1.2.3", sha256: sha }));
    fs.writeFileSync(path.join(tools, "bin", "xenonet"), `#!/bin/sh\nprintf '%s|%s|%s' "$*" "$XENONET_LAUNCHER_BIN" "$ELECTRON_RUN_AS_NODE"\n`, { mode: 0o755 });
    const own = path.join(r.dir, "launcher", "xenonet");
    fs.writeFileSync(own, "", { mode: 0o755 });
    const ran = await run(r.entry, ["agents", "--json"], { env: { ELECTRON_RUN_AS_NODE: "1" } });
    assert.deepEqual([ran.code, ran.stdout], [0, `agents --json|${fs.realpathSync(own)}|`]);
    assert.equal(JSON.parse((await run(r.entry, ["--launcher"])).stdout).tools, "1.2.3");
  } finally {
    r.done();
  }
});
