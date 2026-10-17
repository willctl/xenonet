import { createMcpInfo, demoMcpInfo } from "../../core/mcp-info.js";

export function mcpRoutes({ state, demo, send, hub = null, catalog = createMcpInfo() }) {
  return async (req, res, url) => {
    if (req.method !== "GET" || url.pathname !== "/api/mcp") return false;
    const server = url.searchParams.get("server");
    if (!server || server.length > 160 || /[\x00-\x1f]/.test(server)) { send(res, 400, { error: "say which MCP server to inspect" }); return true; }
    const host = url.searchParams.get("host");
    if (host) {
      if (!hub) { send(res, 404, { error: "that host isn't on this map" }); return true; }
      try { const r = await hub.get(host, `/api/mcp?${new URLSearchParams({ server })}`); send(res, r.status, r.body); }
      catch { send(res, 503, { error: "this host's MCP catalog is unavailable" }); }
      return true;
    }
    const known = [...state.index.values()].some((n) => n.kind === "resource" && n.name === server || n.kind === "wiki" && server === (n.server ?? "agent-wiki"));
    if (!known) { send(res, 404, { error: "that MCP server isn't on this map" }); return true; }
    const sessions = state.liveSnapshot().sessions.map((s) => ({ ...s, cwd: state.index.get(s.id)?.cwd, title: state.index.get(s.id)?.title }));
    const info = demo ? { ...demoMcpInfo(server), sessions: sessions.filter((s) => s.mcp.some((m) => m.server === server)).map(({ id, app, title, status }) => ({ id, app, title, status })) } : await catalog.get(server, sessions, { refresh: url.searchParams.get("refresh") === "1" });
    send(res, 200, info);
    return true;
  };
}
