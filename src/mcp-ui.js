import { escapeHtml as h, timeAgo } from "./format.js";
import { colorVar } from "./theme.js";
import "./mcp.css";

const STATUS = { unknown: "Not reported", notStarted: "Not started", starting: "Connecting", connected: "Connected", authenticationRequired: "Authentication required", failed: "Connection failed", cancelled: "Cancelled", disabled: "Disabled" };
const AUTH = { unknown: "Not reported", unsupported: "OAuth unsupported", notLoggedIn: "Not signed in", bearerToken: "Bearer token", oAuth: "OAuth signed in" };
const badge = (info) => `<span class="mcp-status ${["failed", "authenticationRequired"].includes(info.status) ? "attention" : ""}">${h(info.status === "connected" && info.source === "Read-only MCP discovery" ? "Reachable" : STATUS[info.status] ?? STATUS.unknown)}</span>`;
const row = (label, value) => `<div class="kv"><div class="k">${h(label)}</div><div class="v">${value}</div></div>`;
const annotations = (t) => [t.annotations?.readOnlyHint === true && "read only", t.annotations?.destructiveHint === true && "destructive", t.annotations?.idempotentHint === true && "idempotent", t.annotations?.openWorldHint === true && "external access"].filter(Boolean).map((x) => `<span class="tag">${x}</span>`).join(" ");
const count = (info, key) => info.catalog ? info[key].length : "—";

// read: GETs another machine's details (node.host) with the window's key,
// as windowCalls.read does; this machine's go as any page's.
export function createMcpUi({ modals, toast, read = (url) => fetch(url) }) {
  const cache = new Map();
  const pending = new Map();
  let view = null;
  // A shared system's id, which its name replaces on the map when the id is a uuid.
  const serverOf = (node) => node.kind === "wiki" ? node.server ?? "agent-wiki" : node.server ?? node.name;
  // Where a name that isn't the server's id came from; a guess says so.
  const NAMED_FROM = { declared: "named in Claude desktop", instructions: "named by its instructions", results: "name guessed from its results" };
  const titleOf = (node) => node.kind === "wiki" ? serverOf(node) : node.name;
  const keyOf = (node) => `${node.host ?? ""}/${serverOf(node)}`;
  async function load(node, refresh = false) {
    const key = keyOf(node);
    if (!refresh && cache.has(key) && Date.now() - cache.get(key).checkedAt < 30000) return cache.get(key);
    if (!pending.has(key)) pending.set(key, (node.host ? read : fetch)(`/api/mcp?${new URLSearchParams({ server: serverOf(node), ...(node.host ? { host: node.host } : {}), ...(refresh ? { refresh: "1" } : {}) })}`).then(async (r) => {
      const info = await r.json();
      if (!r.ok) throw new Error(info.error ?? "MCP discovery unavailable");
      cache.set(key, info);
      return info;
    }).finally(() => pending.delete(key)));
    return pending.get(key);
  }
  function summary(info) {
    return `<div class="mcp-summary">${badge(info)}<div class="grid">${row("authentication", h(AUTH[info.auth] ?? AUTH.unknown))}${row("author", h(info.author ?? "Not declared"))}${row("version", h(info.version ?? "Not reported"))}</div><div class="mcp-counts"><span><b>${count(info, "tools")}</b> tools</span><span><b>${count(info, "resources")}</b> resources</span><span><b>${count(info, "templates")}</b> templates</span></div><button class="act" data-mcp-open>Explore MCP server →</button></div>`;
  }
  async function inspect(node) {
    const slot = document.getElementById("insp-mcp");
    if (!slot) return;
    slot.innerHTML = `<p class="dim" role="status">Reading MCP metadata…</p>`;
    try {
      const info = await load(node);
      if (!slot.isConnected) return;
      slot.innerHTML = summary(info);
      slot.querySelector("[data-mcp-open]").onclick = () => open(node);
    } catch (err) {
      if (slot.isConnected) slot.innerHTML = `<p class="dim">${h(err.message)}</p><button class="act" data-mcp-retry>Try again</button>`;
      slot.querySelector("[data-mcp-retry]")?.addEventListener("click", () => inspect(node));
    }
  }
  async function open(node) {
    const panel = modals.open(`<header class="m-head"><div><div class="m-chip">Shared system · MCP</div><h2>${h(titleOf(node))}</h2><div class="m-sub">Reading the server catalog…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"></div>`, { wide: true, cls: "mcp-panel", accent: colorVar("resource"), onClose: () => { if (view?.panel === panel) view = null; } });
    view = { node, panel, tab: "overview", query: "", info: null };
    try {
      const info = await load(node);
      if (view?.panel !== panel) return;
      view.info = info;
      render();
    } catch (err) {
      if (view?.panel === panel) panel.querySelector(".m-body").textContent = err.message;
    }
    return state();
  }
  function content(info, tab, query) {
    if (tab === "overview") return `<p class="mcp-about">${h(info.description ?? "This server has not declared a description.")}</p><div class="grid">${row("connection", badge(info))}${row("authentication", h(AUTH[info.auth] ?? AUTH.unknown))}${row("author", h(info.author ?? "Not declared by the installed package"))}${row("version", h(info.version ?? "Not reported"))}${row("website", info.website ? `<a href="${h(info.website)}" target="_blank" rel="noreferrer">${h(info.website)}</a>` : "Not declared")}${row("capabilities", info.capabilities.length ? info.capabilities.map((c) => `<span class="tag">${h(c)}</span>`).join(" ") : "Not reported")}</div><div class="panel-title sub"><span>Client configuration</span><span class="tick">${info.connections.length}</span></div>${info.connections.map((c) => `<div class="mcp-connection"><strong>${h(c.app)}</strong><span class="tag">${h(c.scope)}</span><span class="tag">${h(c.transport)}</span>${c.enabled ? "" : '<span class="tag pressure">disabled</span>'}${c.endpoint ? `<p><code>${h(c.endpoint)}</code></p>` : ""}<p class="dim">${c.credentials.length ? `Credential variables: ${c.credentials.map(h).join(", ")} · values hidden` : "No credential variables declared"}</p></div>`).join("") || '<p class="dim">Configuration is not available to this console.</p>'}<p class="mcp-note">${h(info.source)} · checked ${timeAgo(info.checkedAt)}. Authentication reflects the client’s report; observing a tool call does not prove a current connection.</p>`;
    if (tab === "tools") {
      const tools = info.tools.filter((t) => `${t.name} ${t.title ?? ""} ${t.description ?? ""}`.toLowerCase().includes(query.toLowerCase()));
      return `<label class="mcp-search">Search tools<input type="search" data-mcp-search placeholder="Name or description" value="${h(query)}"></label><div class="mcp-tool-list">${tools.map((t) => `<details class="mcp-tool"><summary><code>${h(t.name)}</code>${annotations(t)}</summary>${t.title ? `<h3>${h(t.title)}</h3>` : ""}<p>${h(t.description ?? "No description supplied.")}</p><div class="panel-title sub"><span>Input schema</span></div><pre>${h(JSON.stringify(t.inputSchema, null, 2))}</pre></details>`).join("") || `<p class="dim">${info.catalog ? query ? "No tools match this search." : "No tools advertised." : "The client has not supplied a tool catalog. Recent use is shown under Activity."}</p>`}</div><p class="mcp-note">Tool hints are declarations by the server. Inspecting a schema does not run the tool.</p>`;
    }
    if (tab === "context") return `${info.instructions ? `<div class="panel-title sub"><span>Server instructions</span></div><p class="mcp-about">${h(info.instructions)}</p>` : ""}` + [ ["Resources", info.resources], ["Resource templates", info.templates] ].map(([label, items]) => `<div class="panel-title sub"><span>${label}</span><span class="tick">${info.catalog ? items.length : "—"}</span></div>${items.map((r) => `<article class="mcp-resource"><h3>${h(r.title ?? r.name)}</h3><p>${h(r.description ?? "No description supplied.")}</p><code>${h(r.uri)}</code>${r.mimeType ? `<p class="dim">${h(r.mimeType)}</p>` : ""}</article>`).join("") || `<p class="dim">${info.catalog ? "None advertised." : "Resource discovery is unavailable."}</p>`}`).join("") + '<p class="mcp-note">These describe context the server can provide. The server sends resource contents only when a session asks for them.</p>';
    return `<div class="grid">${row("last observed call", info.lastCall ? timeAgo(info.lastCall) : "Not observed")}${row("sessions using it", String(info.sessions.length))}</div>${info.sessions.map((s) => `<button class="mcp-session" data-mgoto="${h(s.id)}"><span class="tag">${h(s.app)}</span>${h(s.title ?? s.id)}<span class="dim">${h(s.status ?? "")}</span></button>`).join("") || '<p class="dim">No sessions in the current live window.</p>'}`;
  }
  function render() {
    if (!view?.info) return;
    const { panel, info, tab, query } = view;
    panel.innerHTML = `<header class="m-head"><div><div class="m-chip">Shared system · MCP</div><h2>${h(info.title && info.title !== info.server ? info.title : titleOf(view.node))}</h2><div class="m-sub">${h(info.server)} · ${badge(info)}${NAMED_FROM[view.node.nameSource] ? ` · ${h(NAMED_FROM[view.node.nameSource])}` : ""}</div></div><div class="mcp-head-actions"><button class="act" data-mcp-refresh>Refresh</button><button class="act" data-mclose>Close ✕</button></div></header><div class="mcp-tabs" role="tablist" aria-label="MCP server details">${["overview", "tools", "context", "activity"].map((t) => `<button role="tab" aria-selected="${t === tab}" aria-controls="mcp-tab-content" id="mcp-tab-${t}" tabindex="${t === tab ? 0 : -1}" data-mcp-tab="${t}">${t}${t === "tools" ? ` <span>${count(info, "tools")}</span>` : ""}</button>`).join("")}</div><div class="m-body" id="mcp-tab-content" role="tabpanel" aria-labelledby="mcp-tab-${tab}">${content(info, tab, query)}</div>`;
    panel.querySelectorAll("[data-mcp-tab]").forEach((b) => {
      b.onclick = () => { view.tab = b.dataset.mcpTab; render(); panel.querySelector(`[data-mcp-tab="${view.tab}"]`).focus(); };
      b.onkeydown = (e) => {
        const tabs = ["overview", "tools", "context", "activity"];
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
        e.preventDefault();
        view.tab = tabs[e.key === "Home" ? 0 : e.key === "End" ? 3 : (tabs.indexOf(view.tab) + (e.key === "ArrowRight" ? 1 : 3)) % 4];
        render(); panel.querySelector(`[data-mcp-tab="${view.tab}"]`).focus();
      };
    });
    panel.querySelector("[data-mcp-search]")?.addEventListener("input", (e) => { view.query = e.target.value; const pos = e.target.selectionStart; render(); const input = panel.querySelector("[data-mcp-search]"); input.focus(); input.setSelectionRange(pos, pos); });
    panel.querySelector("[data-mcp-refresh]").onclick = async (e) => {
      e.target.disabled = true;
      const mine = view;
      try { const info = await load(mine.node, true); if (view === mine) { view.info = info; render(); } }
      catch (err) { toast(err.message, true); if (view === mine) e.target.disabled = false; }
    };
  }
  function state() { return view ? { server: serverOf(view.node), tab: view.tab, query: view.query, loading: !view.info, info: view.info } : null; }
  return { inspect, open, state, async tab(tab) { if (!view || !["overview", "tools", "context", "activity"].includes(tab)) throw new Error("open an MCP server before choosing its detail tab"); view.tab = tab; render(); return state(); }, async search(query) { if (!view) throw new Error("open an MCP server before searching its tools"); view.query = String(query); view.tab = "tools"; render(); return state(); } };
}
