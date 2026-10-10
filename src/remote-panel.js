// Remote connections in the Actors panel (actors-panel.js;
// docs/guide/cloud-assistants.md): the connections that let a remote actor,
// such as an OpenAI dot, read what's going on here, each with its actor,
// who can call it, what it reads and until when, and Revoke; the switch
// that turns all of them off; and pairing a new one, which shows exactly
// what would leave this computer before anything is approved.
//
// Everything here goes through the window's own routes with its key
// (server/routes/assistant.js): pairing, widening a grant and turning
// access back on are the window's alone. A credential never reaches the
// page: XenoNet writes it where only your account can read it, and the page
// shows the command the tunnel client runs, which holds none.
//
// For a connection through OpenAI's tunnel, XenoNet can run tunnel-client
// itself (core/assistant/tunnels.js): the tunnel view takes the tunnel's
// id, and its runtime key once, which goes to your computer's credential
// store and never comes back to the page.
import { escapeHtml as h, timeAgo } from "./format.js";

export const GRANT_DAYS = [1, 7, 14, 30, 60, 90];
const STATE_WORDS = { active: "active", expired: "expired", revoked: "revoked", disabled: "off: cloud access is off" };
const SCOPE_WORDS = { owner_endpoint: "only my account, at an endpoint of my own", authenticated_user: "signed-in people in my workspace or organization", other_approved_scope: "another audience I approved" };
const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// The connections, as the window reads them: refreshed as the panel opens
// and after each change.
export function createRemoteConnections({ calls }) {
  let data = null;
  let canSee = null;
  let off = false;
  let places = null;
  let tunnels = null;
  const view = () => ({ canSee: !!canSee, off, disabled: !!data?.disabled, device: data?.device ?? null, connections: data?.connections ?? [], products: data?.products ?? [], tunnels });
  const after = async (r) => (await refresh(), r);
  async function refresh() {
    const r = await calls.call("GET", "/api/assistant");
    if (r.ok) [canSee, data] = [true, r.data];
    else if (r.status === 503) [off, data] = [true, null];
    else if (r.code === "no-key" || r.status === 403) [canSee, data] = [false, null];
    // The tunnels XenoNet can run, when any connection goes through one.
    if (r.ok && r.data.connections.some((c) => c.transport_kind === TUNNEL)) {
      const t = await calls.call("GET", "/api/assistant/tunnels");
      tunnels = t.ok ? t.data : null;
    } else tunnels = null;
    return view();
  }
  const at = (grant, what = "") => `/api/assistant/${encodeURIComponent(grant)}/tunnel${what}`;
  return {
    refresh,
    view,
    async places() {
      const r = await calls.call("GET", "/api/assistant/places");
      if (r.ok) places = r.data;
      return places ?? { places: [], identities: [] };
    },
    preview: (body) => calls.call("POST", "/api/assistant/preview", body),
    pair: async (body) => after(await calls.call("POST", "/api/assistant/pair", body)),
    revoke: async (grant) => after(await calls.call("POST", `/api/assistant/${encodeURIComponent(grant)}/revoke`)),
    setAccess: async (on) => after(await calls.call("POST", on ? "/api/assistant/enable" : "/api/assistant/disable")),
    of: (grant) => (data?.connections ?? []).find((c) => c.grant_id === grant) ?? null,
    tunnelOf: (grant) => tunnels?.tunnels.find((t) => t.grant === grant) ?? null,
    tunnel: {
      set: async (grant, body) => after(await calls.call("POST", at(grant), body)),
      key: async (grant, key) => after(await calls.call("POST", at(grant, "/key"), { key })),
      forget: async (grant) => after(await calls.call("POST", at(grant, "/forget"))),
      start: async (grant) => after(await calls.call("POST", at(grant, "/start"))),
      stop: async (grant) => after(await calls.call("POST", at(grant, "/stop"))),
      check: (grant) => calls.call("POST", at(grant, "/check")),
    },
  };
}

// ---------- tunnels ----------

export const TUNNEL = "openai-secure-mcp-tunnel";
const TUNNEL_STATE = { off: "not running", starting: "starting", ready: "running", degraded: "running, with trouble", failed: "stopped", stopping: "stopping" };
const running = (t) => ["starting", "ready", "degraded"].includes(t?.state);
// A tunnel's id, short: its start and end, which tell yours apart.
const shortId = (id) => (id ? `${id.slice(0, 11)}…${id.slice(-4)}` : null);

// How a tunnel XenoNet runs stands, in a line: its tunnel, its key, whether
// it runs and why not, with Start or Stop when it's set up.
export function tunnelLine(t) {
  if (!t) return "";
  const state = `<span class="presence ${t.state === "ready" ? "working" : running(t) ? "idle" : "offline"}">tunnel ${h(TUNNEL_STATE[t.state] ?? t.state)}</span>`;
  const parts = [t.tunnel_id ? h(shortId(t.tunnel_id)) : "no tunnel set", t.key === "stored" ? "key kept" : "no key", t.autostart ? "starts with XenoNet" : null].filter(Boolean).join(" · ");
  const why = t.why && t.state !== "ready" ? ` <span class="dim">· ${h(t.why)}</span>` : "";
  const ready = t.tunnel_id && t.key === "stored";
  const button = running(t) ? `<button class="act" data-rc-tstop="${h(t.grant)}" title="tunnel-client stops; the dot can't reach XenoNet until it runs again">Stop</button>` : ready ? `<button class="act" data-rc-tstart="${h(t.grant)}" data-yours title="XenoNet runs tunnel-client for this connection">Start</button>` : "";
  return `<div class="ac-tunnel">${state} <span class="dim">· ${parts}</span>${why} ${button}</div>`;
}

// The tunnel view: tunnel-client found or not, which tunnel, the key (a
// field for it, never its value), whether it starts with XenoNet, and how
// it runs now.
export function tunnelHtml(x, t, { client = null, store = null, links = {}, replacing = false } = {}) {
  const found = client?.found ? `tunnel-client ${h(client.version ?? "")} at <code>${h(client.path)}</code>` : `XenoNet can't find tunnel-client. Install it from <a href="${h(links.releases ?? "")}" target="_blank" rel="noreferrer">OpenAI's releases</a>; XenoNet looks on your PATH and where its installs go.`;
  const profile = t?.profile?.tunnel_id && t.profile.tunnel_id !== t.tunnel_id ? `<small class="dim">Your tunnel-client profile “${h(t.profile.name)}” runs this connection with ${h(shortId(t.profile.tunnel_id))}. <button class="act" data-rc-tuse="${h(t.profile.tunnel_id)}">Use that tunnel</button></small>` : "";
  const kept = t?.key === "stored" && !replacing;
  const key = kept
    ? `<div class="ac-tnote">kept in ${h(store?.kind ?? "your credential store")} as “${h(t.key_target)}” <button class="act" data-rc-treplace>Replace</button>${t.key_named ? "" : ` <button class="act" data-rc-tforget title="XenoNet deletes it from your credential store">Forget</button>`}</div>`
    : `<div class="ac-keyrow"><input class="ac-input" type="password" data-rc-key maxlength="400" placeholder="sk-…, the tunnel's runtime API key" spellcheck="false" autocomplete="off" /><button class="act" data-rc-tkey data-yours>Keep it</button></div><small class="dim">Make it on OpenAI's API keys page, restricted to Tunnels: Read and Use. XenoNet keeps it in ${h(store?.kind ?? "your credential store")}, gives it only to tunnel-client, and never shows it again.</small>`;
  return `
      <p class="pj-note">XenoNet can run OpenAI's tunnel-client for “${h(x.label)}”, so ${h(x.actor_id)} reaches XenoNet while XenoNet runs, with no terminal open for it.</p>
      <div class="ac-form">
        <div class="ac-field"><span class="ac-k">client</span><div class="ac-tnote">${found}</div></div>
        <label class="ac-field"><span class="ac-k">tunnel</span><input class="ac-input" data-rc-tid maxlength="72" placeholder="tunnel_…, from OpenAI's Tunnels page" spellcheck="false" autocomplete="off" value="${h(t?.tunnel_id ?? "")}" /></label>
        ${profile ? `<div class="ac-field"><span class="ac-k"></span>${profile}</div>` : ""}
        <div class="ac-field"><span class="ac-k">key</span>${key}</div>
        <label class="ac-field"><span class="ac-k">kept as</span><input class="ac-input" data-rc-ktarget maxlength="127" placeholder="optional: the name of a key you keep there already" spellcheck="false" autocomplete="off" value="${h(t?.key_named ? t.key_target : "")}" /></label>
        <div class="ac-field"><span class="ac-k">with XenoNet</span><div class="ac-caps"><button class="toggle ${t?.autostart ? "on" : ""}" data-rc-tauto aria-pressed="${!!t?.autostart}">Start it when XenoNet starts</button></div><small class="dim">off: it runs only when you start it here</small></div>
        <div class="ac-field"><span class="ac-k">now</span><div data-rc-tstate>${tunnelStateHtml(t)}</div></div>
      </div>
      <pre class="ac-preview" data-rc-tout hidden></pre>
      <div class="ac-error bad" data-rc-error hidden></div>
      <p class="pj-note">Stopping it, revoking the connection or turning cloud access off stops tunnel-client at once. <a href="${h(links.tunnels ?? "")}" target="_blank" rel="noreferrer">OpenAI's Tunnels page</a> · <a href="${h(links.guide ?? "")}" target="_blank" rel="noreferrer">how tunnels work</a></p>
      <div class="m-foot"><button class="act" data-ac-back>Back</button><span class="grow"></span><button class="act" data-rc-tcheck title="tunnel-client doctor, with the key">Check</button><button class="act" data-rc-tsave>Save</button><span data-rc-tfoot>${tunnelFootHtml(t)}</span></div>`;
}

export function tunnelStateHtml(t) {
  if (!t) return `<span class="dim">not set up</span>`;
  const lines = (t.log ?? []).filter((l) => l.level === "error" || l.level === "warn").slice(-3);
  return `<span class="presence ${t.state === "ready" ? "working" : running(t) ? "idle" : "offline"}">${h(TUNNEL_STATE[t.state] ?? t.state)}</span>${t.why ? ` <span class="dim">· ${h(t.why)}</span>` : ""}${t.state === "ready" && t.polled_at ? ` <span class="dim">· last heard from OpenAI <span data-since="${h(Date.parse(t.polled_at))}">${timeAgo(Date.parse(t.polled_at))}</span></span>` : ""}${lines.length ? `<ul class="ac-tlog">${lines.map((l) => `<li>${h(l.msg)}</li>`).join("")}</ul>` : ""}`;
}

export const tunnelFootHtml = (t) => (running(t) ? `<button class="act primary" data-rc-tstop="${h(t.grant)}">Stop</button>` : `<button class="act primary" data-rc-tstart="${h(t?.grant ?? "")}" data-yours ${t?.tunnel_id && t.key === "stored" ? "" : "disabled"}>Start</button>`);

// ---------- the list ----------

export function remoteSectionHtml() {
  return `
      <div class="d-k ac-remote-head"><span>remote connections</span><button class="toggle" data-rc-access aria-pressed="true" title="Every remote connection, on or off at once">Cloud access</button></div>
      <p class="pj-note">Remote actors, such as an OpenAI dot, read what's going on here through connections you pair: only the projects you pick, read-only, until their grant expires. They can't run anything, read files or transcripts, or message anyone.</p>
      <ol class="ac-list" data-ac-remote></ol>`;
}

// One connection's row, in the reconciled list's shape ({ key, sig, live,
// html }).
export function remoteRow(x, { color = "var(--actor)", tunnel = null } = {}) {
  const projects = x.projects.length ? x.projects.map((p) => p.name ?? "a project no longer on the map").join(", ") : "no projects: it sees nothing yet";
  const reads = [projects, x.labels ? "with names and titles" : "ids and counts only", x.data_mode === "fixture" ? "made-up data until you've verified who can call it" : null].filter(Boolean).join(" · ");
  const head = `<div class="ac-name"><span class="ac-title">${h(x.label)}</span><span class="ac-what">${h(x.actor_id)} · ${h(x.transport_kind === "openai-secure-mcp-tunnel" ? "OpenAI Secure MCP Tunnel" : "a local MCP client")}</span></div>`;
  const until = x.state === "active" ? `until ${x.expires_at.slice(0, 10)}` : x.revoked_at ? `revoked ${x.revoked_at.slice(0, 10)}` : "";
  const used = x.last_used_at ? `used <span data-since="${h(Date.parse(x.last_used_at))}">${timeAgo(Date.parse(x.last_used_at))}</span>` : "never used";
  const live = `<span class="presence ${x.state === "active" ? (x.bridge === "connected" ? "working" : "idle") : "offline"}">${h(STATE_WORDS[x.state] ?? x.state)}</span>${x.state === "active" ? ` · ${x.bridge === "connected" ? "an adapter is connected" : "no adapter connected"}` : ""} <span class="dim">· ${used}${until ? ` · ${h(until)}` : ""}</span>${x.transport_kind === TUNNEL && x.state !== "revoked" ? tunnelLine(tunnel) : ""}`;
  const setup = x.transport_kind === TUNNEL && (x.state === "active" || x.state === "disabled") ? `<button class="act" data-rc-tunnel="${h(x.grant_id)}" title="XenoNet runs tunnel-client for this connection">Tunnel…</button>` : "";
  const acts = x.state === "active" || x.state === "disabled" ? `${setup}<button class="act" data-rc-revoke="${h(x.grant_id)}" title="It stops reading at once, for good">Revoke</button>` : "";
  const html = `<li class="ac-row ac-remote" style="--c:${color}"><span class="ac-dot"></span><div class="ac-main">${head}<div class="ac-live"></div><div class="ac-via">${h(reads)}</div><div class="ac-via">who can call it: ${h(x.audience.description)} (${h(SCOPE_WORDS[x.audience.scope] ?? x.audience.scope)}${x.audience.verification === "owner_verified" ? ", verified by you" : ", not verified"}) · account ${h(x.account_link.state === "owner_asserted" ? `${x.account_link.identity}, your word` : x.account_link.state)}</div></div><div class="ac-acts">${acts}</div></li>`;
  return { key: x.grant_id, sig: `${html}|${x.state}`, live, html };
}

// ---------- pairing ----------

export const newDraft = () => ({ product: "dots", actor: "", name: "", label: "", alias: "", transport: "openai-secure-mcp-tunnel", audience: "", scope: "owner_endpoint", verified: false, projects: [], labels: false, days: 30, link: "unverified", identity: "" });

// The form, from `draft`, the products that can be paired, the actors
// already here that a product's connection could join, and the places and
// identities to pick from.
export function pairHtml(draft, { products = [], actors = [], places = [], identities = [] } = {}) {
  const product = products.find((p) => (p.product ?? "custom") === (draft.product || "custom")) ?? products[0];
  const pick = (name, value, label, on, title = "") => `<button class="toggle ${on ? "on" : ""}" data-rc-${name}="${h(value)}" aria-pressed="${on}"${title ? ` title="${h(title)}"` : ""}>${h(label)}</button>`;
  const joinable = actors.filter((a) => a.kind !== "human" && (a.product?.id ?? null) === (product?.product ?? null) && !a.via?.includes("demo"));
  // Only the accounts a product could work under: a dot's are ChatGPT's.
  const accounts = product?.product === "dots" ? identities.filter((i) => i.kind === "chatgpt") : identities;
  return `
      <p class="pj-note">A remote actor reads through a connection you pair here, and nowhere else. Pick what it may read; the next step shows exactly what would leave this computer before anything is approved.</p>
      <div class="ac-form">
        <div class="ac-field"><span class="ac-k">actor</span><div class="ac-caps">${products.map((p) => pick("product", p.product ?? "custom", p.name, (p.product ?? "custom") === (draft.product || "custom"), p.support === "fixture" ? "Its own connection isn't tested yet" : "")).join("")}</div></div>
        <div class="ac-field"><span class="ac-k">which</span><div class="ac-caps">${pick("actor", "", product?.product === "dots" || !product?.product ? "a new one" : "a new one, if it isn't here", !draft.actor)}${joinable.map((a) => pick("actor", a.id, `${a.name} (${a.id})`, draft.actor === a.id)).join("")}</div></div>
        ${draft.actor ? "" : `<label class="ac-field"><span class="ac-k">its name</span><input class="ac-input" data-rc-name maxlength="64" placeholder="${product?.product ? h(product.name) : "what the map calls it"}" spellcheck="false" autocomplete="off" value="${h(draft.name)}" /></label>`}
        <label class="ac-field"><span class="ac-k">connection</span><input class="ac-input" data-rc-label maxlength="64" placeholder="a name for this connection, such as My dot" spellcheck="false" autocomplete="off" value="${h(draft.label)}" /></label>
        <label class="ac-field"><span class="ac-k">computer</span><input class="ac-input" data-rc-alias maxlength="64" placeholder="optional: what to call this computer; never its own name" spellcheck="false" autocomplete="off" value="${h(draft.alias)}" /></label>
        <div class="ac-field"><span class="ac-k">route</span><div class="ac-caps">${(product?.transports ?? []).map((t) => pick("transport", t.id, t.name, draft.transport === t.id)).join("")}</div></div>
        <label class="ac-field"><span class="ac-k">who calls</span><input class="ac-input" data-rc-audience maxlength="200" placeholder="who can call its endpoint, in your words" spellcheck="false" autocomplete="off" value="${h(draft.audience)}" /></label>
        <div class="ac-field"><span class="ac-k">audience</span><div class="ac-caps">${Object.entries(SCOPE_WORDS).map(([k, w]) => pick("scope", k, w, draft.scope === k)).join("")}</div></div>
        <div class="ac-field"><span class="ac-k">verified</span><div class="ac-caps">${pick("verified", "1", "I checked who can call this endpoint", draft.verified)}</div><small class="dim">until you have, it reads made-up data only</small></div>
        <div class="ac-field"><span class="ac-k">projects</span><div class="ac-caps ac-places">${places.length ? places.map((p) => pick("place", p.place, `${p.name ?? p.place}${p.sessions7d ? ` (${p.sessions7d})` : ""}`, draft.projects.includes(p.place))).join("") : `<span class="dim">no projects on the map yet</span>`}</div><small class="dim">none unless you pick some; new projects are never added by themselves</small></div>
        <div class="ac-field"><span class="ac-k">labels</span><div class="ac-caps">${pick("labels", "1", "Send project names and session titles", draft.labels)}</div><small class="dim">off: ids and counts only. SIs and people wrote the titles, and they can say anything</small></div>
        <div class="ac-field"><span class="ac-k">for</span><div class="ac-caps">${GRANT_DAYS.map((d) => pick("days", d, plural(d, "day"), draft.days === d)).join("")}</div></div>
        <div class="ac-field"><span class="ac-k">account</span><div class="ac-caps">${pick("link", "unverified", "Unverified", draft.link === "unverified")}${accounts.map((i) => pick("identity", i.id, `${i.name ?? i.id}, my word`, draft.link === "owner_asserted" && draft.identity === i.id, "Shown as your word, never as proof")).join("")}</div><small class="dim">its provider can't prove which account it is through this route</small></div>
      </div>
      <div class="ac-error bad" data-rc-error hidden></div>
      <div class="m-foot"><button class="act" data-ac-back>Back</button><span class="grow"></span><button class="act primary" data-rc-preview>Preview what leaves</button></div>`;
}

// The form's text fields, read into the draft.
export function readPairForm(body, draft) {
  const val = (sel) => body.querySelector(sel)?.value ?? "";
  return { ...draft, name: val("[data-rc-name]"), label: val("[data-rc-label]"), alias: val("[data-rc-alias]"), audience: val("[data-rc-audience]") };
}

// What the window sends to pair: the draft as the route takes it.
export function pairBody(draft) {
  return {
    product: draft.product === "custom" ? null : draft.product,
    ...(draft.actor ? { actor: draft.actor } : draft.name.trim() ? { name: draft.name.trim() } : {}),
    label: draft.label.trim(),
    ...(draft.alias.trim() ? { deviceAlias: draft.alias.trim() } : {}),
    transport: draft.transport,
    audience: { description: draft.audience.trim(), verification: draft.verified ? "owner_verified" : "unverified", scope: draft.scope },
    projects: draft.projects,
    labels: draft.labels,
    days: draft.days,
    accountLink: draft.link === "owner_asserted" ? { state: "owner_asserted", identity: draft.identity } : { state: "unverified" },
  };
}

const json = (v) => `<pre class="ac-preview">${h(JSON.stringify(v, null, 1))}</pre>`;

// What would leave, before anything is approved: made-up answers first,
// then the real fields for the projects picked.
export function previewHtml(p, draft) {
  const names = new Map((p.yours ?? []).map((y) => [y.project_id, y.name]));
  return `
      <p class="ac-lead">This is what it would read.</p>
      <p class="pj-note">Made up, first: the shape of every answer, with ${draft.labels ? "labels" : "labels left out"}.</p>
      ${json({ projects: p.synthetic?.projects?.projects ?? [], attention: p.synthetic?.attention?.items ?? [] })}
      <p class="pj-note">${draft.verified ? `Then the real fields, for ${plural(p.real.projects.length, "project")} you picked${p.real.projects.length ? ` (${[...names.values()].map((n) => h(n ?? "?")).join(", ")})` : ""}: exactly these, and no paths, branches, prompts, messages, questions' text, tools, accounts or errors.` : "It would read made-up data only, since you haven't verified who can call its endpoint; these are the real fields it would read once you have."}</p>
      ${json({ projects: p.real.projects, sessions: p.real.sessions })}
      <p class="pj-note">The service keeps a log of each read (times, codes and sizes, never content) for seven days. Revoking stops new reads at once, but an assistant may keep what it already read. Nothing starts by itself: the adapter runs only while your tunnel client runs it.</p>
      <div class="m-foot"><button class="act" data-rc-edit>Change</button><span class="grow"></span><button class="act primary" data-rc-approve data-yours>Approve and pair</button></div>`;
}

// Paired: what the tunnel client runs, and where the credential is.
export function pairedHtml(x) {
  const command = [x.launch.command, ...x.launch.args].join(" ");
  const tunnel = x.transport_kind === TUNNEL;
  return `
      <p class="ac-lead"><code>${h(x.actor_id)}</code> is paired as “${h(x.label)}”.</p>
      ${tunnel ? `<p class="pj-note">XenoNet can run OpenAI's tunnel-client for it: give it the tunnel's id and its runtime key once, under Set up the tunnel. Or run tunnel-client yourself, as below.</p>` : ""}
      <p class="pj-note">Its credential is saved where only your account can read it, in XenoNet's state folder; it never shows here. ${tunnel ? "To run it yourself, have tunnel-client run this" : "Have your tunnel client run this"}, as the MCP server it forwards to:</p>
      <code class="ac-token" data-rc-command>${h(command)}</code>
      <p class="pj-note">It holds no secret. Until you've verified who can call the endpoint it reads made-up data. It expires ${h(x.expires_at.slice(0, 10))}; revoke it here, or with <code>xenonet assistant revoke ${h(x.grant_id)}</code>, and turn everything off with <code>xenonet assistant disable</code>, which works with XenoNet stopped.</p>
      <div class="m-foot"><button class="act" data-rc-copy>Copy</button><span class="grow"></span>${tunnel ? `<button class="act" data-rc-tunnel="${h(x.grant_id)}">Set up the tunnel…</button>` : ""}<button class="act primary" data-ac-done>Done</button></div>`;
}

export function revokeHtml(x) {
  return `<p class="ac-lead">Revoke “${h(x.label)}”?</p><p class="pj-note">${h(x.actor_id)} stops reading through it at once, for good; its credential is deleted. Its actor, its history and its other connections stay. What it already read isn't taken back: revoke its endpoint with its provider too.</p>
      <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary ac-danger" data-rc-revoke-yes="${h(x.grant_id)}">Revoke</button></div>`;
}
