import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createMcpInfo, displayCatalog, publicUrl } from "../core/mcp-info.js";
import { discoverMcp } from "../core/mcp-discovery.js";

test("the display catalog excludes credentials, vendor metadata and schema defaults", () => {
  const secret = "do-not-display-this";
  const info = displayCatalog({ runtimeStatus: "connected", authStatus: "oAuth", httpOrigin: `https://user:${secret}@example.com/mcp?token=${secret}`, serverInfo: { name: "library", version: "1", websiteUrl: "javascript:alert(1)", _meta: { token: secret } }, serverCapabilities: { tools: {}, secret }, tools: { search: { name: "search", inputSchema: { type: "object", properties: { token: { type: "string", default: secret, examples: [secret] } }, _meta: { secret } }, annotations: { readOnlyHint: true, token: secret }, _meta: { secret } } }, resources: [], resourceTemplates: [] });
  assert.equal(info.endpoint, "https://example.com");
  assert.equal(info.website, null);
  assert.equal(info.auth, "oAuth");
  assert.deepEqual(info.tools[0].annotations, { readOnlyHint: true });
  assert.ok(!JSON.stringify(info).includes(secret));
  assert.equal(publicUrl("https://example.com/about?key=hidden#secret"), "https://example.com/about");
});

test("configured metadata and selected-server discovery never return config values", async () => {
  const calls = [];
  let time = 100;
  const spec = { command: "node", args: ["/workspace/library/index.js"], env: { API_TOKEN: "private-value" }, headers: { Authorization: "Bearer private-value" } };
  const service = createMcpInfo({ settings: { homeDir: "/workspace", claudeConfigFile: "claude-config" }, now: () => time,
    read: (file) => file === "claude-config" ? { mcpServers: { library: spec, other: { command: "never-run" } } } : file.replaceAll("\\", "/") === "/workspace/library/package.json" ? { name: "library", author: { name: "Library authors", email: "not-needed" }, version: "2" } : {},
    request: async (method) => { calls.push(method); return { config: {} }; },
    discover: async (selected) => { assert.equal(selected.command, "node"); assert.equal(selected.env.API_TOKEN, "private-value"); calls.push("discover-library"); return { serverInfo: { name: "library", version: "2" }, runtimeStatus: "connected", authStatus: "unsupported", tools: [], resources: [], resourceTemplates: [] }; },
  });
  const using = [{ id: "claude:one", app: "claude", title: "Look up references", mcp: [{ server: "library", ts: 30 }] }];
  const info = await service.get("library", using);
  assert.equal(info.author, "Library authors");
  assert.equal(info.lastCall, 30);
  assert.deepEqual(info.connections[0].credentials, ["API_TOKEN"]);
  assert.ok(!JSON.stringify(info).includes("private-value"));
  assert.ok(!JSON.stringify(info).includes("not-needed"));
  await service.get("library", using);
  assert.equal(calls.filter((c) => c === "discover-library").length, 1);
  time += 31000;
  await service.get("library", using);
  assert.equal(calls.filter((c) => c === "discover-library").length, 2);
});

test("Codex reports auth and capabilities; observed use alone stays unknown", async () => {
  const params = [];
  const service = createMcpInfo({ settings: { homeDir: "/workspace", claudeConfigFile: "missing" }, read: () => ({}), request: async (method, p) => {
    params.push([method, p]);
    return method === "config/read" ? { config: { mcp_servers: { library: { url: "https://example.com/mcp" } } } } : { data: [{ name: "library", authStatus: "notLoggedIn", runtimeStatus: "authenticationRequired", tools: {}, resources: [], resourceTemplates: [] }] };
  }, discover: async () => { throw new Error("should not bypass the client's auth report"); } });
  const info = await service.get("library");
  assert.equal(info.status, "authenticationRequired");
  assert.equal(info.auth, "notLoggedIn");
  assert.equal(params[1][1].serverName, "library");
  const observed = await createMcpInfo({ settings: { homeDir: "/workspace" }, read: () => ({}), request: async () => { throw new Error("unavailable"); } }).get("observed", [{ id: "claude:one", mcp: [{ server: "observed", ts: 50 }] }]);
  assert.equal(observed.status, "unknown");
  assert.equal(observed.auth, "unknown");
  assert.equal(observed.catalog, false);
  assert.equal(observed.lastCall, 50);
});

test("Grok Build's MCP servers, from its config.toml and a folder's, without their secrets", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-mcp-grok-"));
  try {
    const grokHome = path.join(dir, ".grok");
    const repo = path.join(dir, "atlas");
    fs.mkdirSync(grokHome, { recursive: true });
    fs.mkdirSync(path.join(repo, ".grok"), { recursive: true });
    fs.writeFileSync(
      path.join(grokHome, "config.toml"),
      [
        "# Grok Build's own settings",
        "[cli]",
        'installer = "brew"',
        "",
        "[mcp_servers.library]",
        'command = "node"',
        'args = ["/workspace/library/index.js"]',
        "enabled = true",
        "",
        "[mcp_servers.library.env]",
        'LIBRARY_TOKEN = "private-value"',
        "",
        "[mcp_servers.remote]",
        'url = "https://example.com/mcp?key=private-value"',
        "enabled = false",
      ].join("\n"),
    );
    fs.writeFileSync(path.join(repo, ".grok", "config.toml"), '[mcp_servers.library]\ncommand = "npx"\nargs = ["library-mcp"]\n');
    const settings = { homeDir: dir, claudeConfigFile: path.join(dir, "missing.json"), grokHome };
    const using = [{ id: "grok:019a3b5c-0000-7000-8000-00000000000a", app: "grok", title: "Look up references", cwd: repo, mcp: [{ server: "library", ts: 40 }] }];
    const service = createMcpInfo({ settings, request: async () => ({ config: {} }), discover: async (selected) => {
      assert.deepEqual([selected.command, selected.args], ["node", ["/workspace/library/index.js"]], "the user's settings first");
      return { serverInfo: { name: "library", version: "1" }, runtimeStatus: "connected", authStatus: "unsupported", tools: [], resources: [], resourceTemplates: [] };
    } });
    const info = await service.get("library", using);
    assert.deepEqual(info.connections.map((c) => [c.app, c.scope, c.transport, c.enabled, c.credentials]), [
      ["Grok Build", "user", "stdio", true, ["LIBRARY_TOKEN"]],
      ["Grok Build", "project", "stdio", true, []],
    ]);
    assert.deepEqual(info.sessions.map((s) => s.app), ["grok"]);
    assert.ok(!JSON.stringify(info).includes("private-value"));
    const remote = await service.get("remote", []);
    assert.deepEqual(remote.connections.map((c) => [c.app, c.transport, c.endpoint, c.enabled]), [["Grok Build", "HTTP", "https://example.com", false]]);
    assert.equal(remote.status, "disabled");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("stdio discovery initializes and lists, without executing a tool or reading content", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-mcp-discovery-"));
  const file = path.join(dir, "server.cjs");
  fs.writeFileSync(file, `require("node:readline").createInterface({ input: process.stdin }).on("line", (line) => {
    const r = JSON.parse(line); if (r.id == null) return;
    const results = { initialize: { protocolVersion: "2025-06-18", serverInfo: { name: "test-library", version: "1" }, capabilities: { tools: {}, resources: {} }, instructions: "Use the reference catalog." }, "tools/list": { tools: [{ name: "search", description: "Search references", inputSchema: { type: "object" } }] }, "resources/list": { resources: [{ name: "Guide", uri: "library://guide" }] }, "resources/templates/list": { resourceTemplates: [] } };
    if (!results[r.method]) process.exit(7);
    console.log(JSON.stringify({ jsonrpc: "2.0", id: r.id, result: results[r.method] }));
  });`);
  try {
    const info = await discoverMcp({ command: process.execPath, args: [file] });
    assert.equal(info.serverInfo.name, "test-library");
    assert.equal(info.tools[0].name, "search");
    assert.equal(info.resources[0].uri, "library://guide");
    assert.equal(info.instructions, "Use the reference catalog.");
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
