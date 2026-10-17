// core/mcp-discovery.js in Rust (crates/xenonet-app/src/mcp_discovery.rs):
// only the selected, already configured server is started, and it's only
// listed: no tool is called, no resource read, no OAuth flow started. The
// Rust says how to reach it, how far to page and what the answer is; the
// talking is the MCP SDK's client, until the Rust binary has its own.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport, getDefaultEnvironment } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { native } from "../index.js";
import { peer } from "../peer.js";

const { VERSION } = await import(peer("version.js"));
const C = native.appMcpDiscoveryConstants();

export async function discoverMcp(spec) {
  const client = new Client({ name: "xenonet-inspector", version: VERSION }, { capabilities: {} });
  const options = { timeout: C.timeoutMs, signal: AbortSignal.timeout(C.signalMs) };
  const how = native.appMcpTransport(spec, { ...process.env });
  const transport =
    how.kind === "stdio"
      ? new StdioClientTransport({ command: how.command ?? undefined, args: how.args, cwd: how.cwd ?? undefined, env: { ...getDefaultEnvironment(), ...how.env }, stderr: "ignore" })
      : new (how.kind === "sse" ? SSEClientTransport : StreamableHTTPClientTransport)(new URL(how.url), { requestInit: { signal: options.signal, headers: how.headers } });
  async function pages(method, key) {
    const rows = [];
    let cursor;
    for (let n = 1; ; n++) {
      const page = await client[method](cursor ? { cursor } : {}, options);
      rows.push(...(page[key] ?? []));
      if (!native.appMcpAnotherPage(n, rows.length, cursor ?? null, page.nextCursor ?? null)) break;
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
    return native.appMcpFound(client.getServerVersion(), capabilities, client.getInstructions(), tools, resources, resourceTemplates, how.kind !== "stdio");
  } finally {
    await client.close().catch(() => {});
    await transport.close().catch(() => {});
  }
}
