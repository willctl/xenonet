// A display-only catalog. Configuration and credentials stay here; only
// presentation metadata and credential names reach the console.
import fs from "node:fs";
import path from "node:path";
import { config } from "./config.js";
import { codexClient } from "./codex.js";
import { discoverMcp } from "./mcp-discovery.js";

const object = (v) => v && typeof v === "object" && !Array.isArray(v) ? v : {};
const text = (v, max = 4000) => typeof v === "string" ? v.slice(0, max) : null;
const json = (file) => { try { return JSON.parse(fs.readFileSync(file, "utf8")); } catch { return {}; } };
function publicUri(value) {
  const raw = text(value, 1000);
  if (!raw) return null;
  try {
    const u = new URL(raw);
    u.username = u.password = "";
    if (!/\{[^}]*\?[^}]*\}/.test(raw)) u.search = u.hash = "";
    return u.href.replaceAll("%7B", "{").replaceAll("%7D", "}");
  } catch { return raw; }
}
export function publicUrl(value, originOnly = false) {
  try {
    const u = new URL(value);
    if (!["https:", "http:"].includes(u.protocol)) return null;
    u.username = u.password = u.search = u.hash = "";
    return originOnly ? u.origin : u.href;
  } catch { return null; }
}

// Defaults and examples can contain credentials. A schema here describes
// the input, without carrying those values or arbitrary vendor metadata.
export function displaySchema(value, depth = 0) {
  if (depth > 12) return null;
  const out = {};
  const v = object(value);
  for (const k of ["$ref", "$schema", "type", "title", "description", "format", "required", "enum", "minimum", "maximum", "minLength", "maxLength"]) {
    if (v[k] !== undefined && JSON.stringify(v[k]).length < 8000) out[k] = v[k];
  }
  if (v.properties) out.properties = Object.fromEntries(Object.entries(object(v.properties)).slice(0, 100).map(([k, s]) => [k, displaySchema(s, depth + 1)]));
  for (const k of ["$defs", "definitions"]) if (v[k]) out[k] = Object.fromEntries(Object.entries(object(v[k])).slice(0, 100).map(([name, s]) => [name, displaySchema(s, depth + 1)]));
  for (const k of ["items", "additionalProperties"]) if (v[k] !== undefined) out[k] = typeof v[k] === "boolean" ? v[k] : displaySchema(v[k], depth + 1);
  for (const k of ["oneOf", "anyOf", "allOf"]) if (Array.isArray(v[k])) out[k] = v[k].slice(0, 30).map((s) => displaySchema(s, depth + 1));
  return out;
}

export function displayCatalog(raw = {}) {
  const info = object(raw.serverInfo);
  const caps = object(raw.serverCapabilities);
  const tools = Array.isArray(raw.tools) ? raw.tools : Object.values(object(raw.tools));
  const resource = (r) => ({ name: text(r.name, 200), title: text(r.title, 200), description: text(r.description), mimeType: text(r.mimeType, 200), uri: publicUri(r.uri ?? r.uriTemplate) });
  return {
    title: text(info.title ?? info.name, 200), version: text(info.version, 100), description: text(info.description), website: publicUrl(info.websiteUrl),
    status: ["notStarted", "starting", "connected", "authenticationRequired", "failed", "cancelled", "disabled"].includes(raw.runtimeStatus) ? raw.runtimeStatus : "unknown",
    auth: ["unknown", "unsupported", "notLoggedIn", "bearerToken", "oAuth"].includes(raw.authStatus) ? raw.authStatus : "unknown",
    endpoint: publicUrl(raw.httpOrigin, true),
    capabilities: ["tools", "resources", "prompts", "logging", "completions", "tasks"].filter((k) => Object.hasOwn(caps, k)),
    tools: tools.slice(0, 500).map((t) => ({ name: text(t.name, 200), title: text(t.title, 200), description: text(t.description), inputSchema: displaySchema(t.inputSchema), annotations: Object.fromEntries(["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"].filter((k) => typeof t.annotations?.[k] === "boolean").map((k) => [k, t.annotations[k]])) })),
    resources: (raw.resources ?? []).slice(0, 500).map(resource), templates: (raw.resourceTemplates ?? []).slice(0, 500).map(resource),
    discoveryFailed: !!raw.toolsError,
    instructions: text(raw.instructions, 12000),
  };
}

function packageInfo(spec, read = json) {
  // Local entry points often have a package or plugin manifest beside them.
  // Looking does not run the command or install a package.
  for (const arg of [spec.command, ...(spec.args ?? [])].filter((a) => typeof a === "string" && path.isAbsolute(a))) {
    let dir = path.dirname(arg);
    for (let i = 0; i < 5; i++, dir = path.dirname(dir)) {
      const p = read(path.join(dir, ".claude-plugin", "plugin.json"));
      const v = Object.keys(p).length ? p : read(path.join(dir, "package.json"));
      if (v.name && (v.author || v.description)) return { author: text(typeof v.author === "string" ? v.author : v.author?.name, 200), description: text(v.description), version: text(v.version, 100), website: publicUrl(v.homepage) };
      if (path.dirname(dir) === dir) break;
    }
  }
  return {};
}

export function createMcpInfo({ request = (method, params) => codexClient.request(method, params), discover = discoverMcp, read = json, settings = config, now = Date.now } = {}) {
  const cache = new Map();
  const pending = new Map();
  async function collect(server, sessions) {
    const using = sessions.filter((s) => s.mcp?.some((m) => m.server === server));
    const result = { server, title: server, author: null, status: "unknown", auth: "unknown", tools: [], resources: [], templates: [], capabilities: [], connections: [], checkedAt: now(), catalog: false };
    const specs = [];
    const add = (app, scope, servers) => {
      for (const [name, spec] of Object.entries(object(servers))) if (name === server) {
        if (!spec || typeof spec !== "object") continue;
        specs.push(spec);
        const remote = spec.url ?? spec.serverUrl;
        result.connections.push({ app, scope, transport: remote ? "HTTP" : "stdio", endpoint: publicUrl(remote, true), enabled: spec.enabled !== false, credentials: Object.keys(object(spec.env)).filter((k) => /key|token|secret|auth|password/i.test(k)).slice(0, 30) });
      }
    };
    const claude = read(settings.claudeConfigFile);
    add("Claude Code", "user", claude.mcpServers);
    for (const [cwd, project] of Object.entries(object(claude.projects))) if (using.some((s) => s.cwd === cwd)) add("Claude Code", "project", project.mcpServers);
    for (const cwd of new Set(using.map((s) => s.cwd).filter(Boolean))) add("Claude Code", "project", read(path.join(cwd, ".mcp.json")).mcpServers);
    if (settings.claudeDesktopDir) add("Claude Desktop", "user", read(path.join(settings.claudeDesktopDir, "claude_desktop_config.json")).mcpServers);
    const installed = read(path.join(settings.homeDir, ".claude", "plugins", "installed_plugins.json"));
    const pluginName = server.startsWith("plugin:") ? server.split(":")[1] : null;
    for (const [key, entries] of Object.entries(object(installed.plugins))) {
      if (!pluginName || key.split("@")[0] !== pluginName) continue;
      for (const entry of Array.isArray(entries) ? entries : []) {
        if (!entry.installPath) continue;
        const manifest = read(path.join(entry.installPath, ".claude-plugin", "plugin.json"));
        result.author ??= text(typeof manifest.author === "string" ? manifest.author : manifest.author?.name, 200);
        result.description ??= text(manifest.description);
        result.version ??= text(manifest.version, 100);
        const servers = read(path.join(entry.installPath, ".mcp.json"));
        const localName = server.split(":").slice(2).join(":");
        const spec = (servers.mcpServers ?? servers)[localName];
        if (spec) add("Claude Code", "plugin", { [server]: { ...spec, args: spec.args?.map((a) => String(a).replaceAll("${CLAUDE_PLUGIN_ROOT}", entry.installPath)), cwd: entry.installPath } });
      }
    }
    let codexConfigured = false;
    try {
      const { config: effective } = await request("config/read", { includeLayers: false });
      const codex = object(effective?.mcp_servers);
      codexConfigured = Object.hasOwn(codex, server);
      add("Codex", "effective", codex);
    } catch {}
    if (codexConfigured || using.some((s) => s.app === "codex" || s.id?.startsWith("codex:"))) {
      try {
        const rows = [];
        let cursor = null;
        for (let i = 0; i < 10; i++) {
          const page = await request("mcpServerStatus/list", { serverName: server, detail: "full", limit: 100, cursor });
          rows.push(...(page.data ?? []));
          if (!page.nextCursor || page.nextCursor === cursor) break;
          cursor = page.nextCursor;
        }
        const raw = rows.find((r) => r.name === server);
        if (raw) Object.assign(result, displayCatalog(raw), { catalog: !raw.toolsError, source: "Codex MCP discovery" });
      } catch { result.discoveryFailed = true; }
    }
    const selected = specs.find((s) => s.enabled !== false && (s.command || s.url || s.serverUrl));
    if (!selected && specs.length && specs.every((s) => s.enabled === false)) result.status = "disabled";
    if (!result.catalog && selected) {
      try { Object.assign(result, displayCatalog(await discover({ ...selected, cwd: selected.cwd ?? settings.homeDir })), { catalog: true, source: "Read-only MCP discovery" }); }
      catch (err) { result.discoveryFailed = true; result.status = err?.name === "UnauthorizedError" || err?.code === 401 ? "authenticationRequired" : "failed"; }
    }
    const metadata = specs.map((s) => packageInfo(s, read)).find((m) => Object.keys(m).length) ?? {};
    for (const k of ["author", "description", "version", "website"]) result[k] ??= metadata[k] ?? null;
    // The transcript proves use, not that a server is connected right now.
    result.lastCall = using.reduce((latest, s) => Math.max(latest, ...s.mcp.filter((m) => m.server === server).map((m) => m.ts ?? 0)), 0) || null;
    result.sessions = using.map((s) => ({ id: s.id, app: s.app ?? s.id?.split(":")[0], title: text(s.title, 300), status: s.status }));
    if (!result.catalog) result.source = result.discoveryFailed ? "Discovery unavailable; observed session activity" : "Configuration and observed session activity";
    return result;
  }
  return {
    async get(server, sessions = [], { refresh = false } = {}) {
      const hit = cache.get(server);
      if (!refresh && hit && now() - hit.checkedAt < 30000) return hit;
      if (!pending.has(server)) pending.set(server, collect(server, sessions).then((r) => (cache.set(server, r), r)).finally(() => pending.delete(server)));
      return pending.get(server);
    },
  };
}

export function demoMcpInfo(server) {
  return { server, title: server === "agent-wiki" ? "Agent Wiki" : "Reference Library", author: "XenoNet demo", version: "1.0.0", description: "A made-up MCP catalog for exploring shared systems.", status: "connected", auth: "unsupported", catalog: true, source: "Demo MCP discovery", checkedAt: Date.now(), capabilities: ["tools", "resources"], connections: [{ app: "Codex", scope: "user", transport: "stdio", enabled: true, credentials: [] }], tools: [{ name: "search", title: "Search references", description: "Find relevant reference material by keyword.", annotations: { readOnlyHint: true, destructiveHint: false }, inputSchema: { type: "object", properties: { query: { type: "string", description: "Words to find" } }, required: ["query"] } }, { name: "save_note", description: "Save a note to the shared library.", annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true }, inputSchema: { type: "object", properties: { title: { type: "string" }, body: { type: "string" } }, required: ["title", "body"] } }], resources: [{ name: "Guide", description: "The shared reference guide.", uri: "library://guide", mimeType: "text/markdown" }], templates: [{ name: "Topic", description: "A reference page by topic.", uri: "library://topics/{topic}", mimeType: "text/markdown" }], sessions: [], lastCall: null };
}
