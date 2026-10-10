// Only the selected, already configured server is started. Discovery lists
// capabilities; it never calls tools, reads resources or starts an OAuth flow.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport, getDefaultEnvironment } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { VERSION } from "./version.js";

export async function discoverMcp(spec) {
  const client = new Client({ name: "xenonet-inspector", version: VERSION }, { capabilities: {} });
  const options = { timeout: 8000, signal: AbortSignal.timeout(12000) };
  const url = spec.url ?? spec.serverUrl;
  const envValue = (v) => String(v).replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, k) => process.env[k] ?? "");
  const transport = url
    ? new (spec.type === "sse" ? SSEClientTransport : StreamableHTTPClientTransport)(new URL(url), { requestInit: { signal: options.signal, headers: Object.fromEntries(Object.entries(spec.headers ?? {}).map(([k, v]) => [k, envValue(v)])) } })
    : new StdioClientTransport({ command: spec.command, args: spec.args ?? [], cwd: spec.cwd, env: { ...getDefaultEnvironment(), ...Object.fromEntries(Object.entries(spec.env ?? {}).map(([k, v]) => [k, envValue(v)])) }, stderr: "ignore" });
  async function pages(method, key) {
    const rows = [];
    let cursor;
    for (let i = 0; i < 10; i++) {
      const page = await client[method](cursor ? { cursor } : {}, options);
      rows.push(...(page[key] ?? []));
      if (!page.nextCursor || cursor === page.nextCursor || rows.length >= 500) break;
      cursor = page.nextCursor;
    }
    return rows;
  }
  try {
    await client.connect(transport, options);
    const capabilities = client.getServerCapabilities() ?? {};
    const [tools, resources, resourceTemplates] = await Promise.all([
      capabilities.tools ? pages("listTools", "tools") : [],
      capabilities.resources ? pages("listResources", "resources") : [],
      capabilities.resources ? pages("listResourceTemplates", "resourceTemplates").catch(() => []) : [],
    ]);
    return { serverInfo: client.getServerVersion(), serverCapabilities: capabilities, instructions: client.getInstructions(), tools, resources, resourceTemplates, runtimeStatus: "connected", authStatus: url ? "unknown" : "unsupported" };
  } finally { await client.close().catch(() => {}); await transport.close().catch(() => {}); }
}
