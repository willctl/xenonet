// The shared context in the Network panel (network.js; docs/design/
// authorization.md, "Forward grants"): Xenolith on this machine syncs its
// spaces over this machine's networks once you give it a grant.
//
//   - Asks: Xenolith asked for a grant for its context key (GET
//     /api/net/forwards' asks, each proved by the key itself). Allow, with
//     the networks it may reach ticked, asks you (POST
//     /api/net/forwards/approve, from the window); Turn down asks nothing.
//   - Grants: what you approved, the networks it reaches and until when the
//     grant Xenolith holds runs (it renews itself while Xenolith comes
//     back within 30 days); Take back ends its sync over the network at
//     once (POST /api/net/forwards/revoke).
//   - Spaces: the ones Xenolith curates, while it's attached, each with
//     what you can sign for it (POST /api/net/forwards/sign, from the
//     window): Make it yours (a space the standalone owner key owns), Share
//     into a network, first the ones its project is shared into ("from the
//     project"), and Stop sharing, marked when the project isn't shared
//     there any more (Xenolith holds it back from that network already).
//     Xenolith drafts the entry, XenoNet checks it does what the button
//     says, and your device signs it, with your presence where it widens.
//
// The grant itself never reaches this page: Xenolith collects it over
// XenoNet's owner-only socket.
//
// Markup and the lenient reads only: the panel holds the state and answers
// the buttons.
import { escapeHtml as h } from "./format.js";

const list = (v) => (Array.isArray(v) ? v : []);
const word = (v) => typeof v === "string" && v.length > 0;
const NAMES = { xenolith: "Xenolith" };
const nameOf = (actor) => NAMES[actor] ?? actor;

// GET /api/net/forwards, read leniently.
export function contextOf(raw) {
  const attached = list(raw?.forwards).filter((f) => f && word(f.alpn));
  return {
    asks: list(raw?.asks)
      .filter((a) => a && word(a.alpn) && word(a.key))
      .map((a) => ({ alpn: a.alpn, key: a.key, actor: word(a.actor) ? a.actor : null, at: typeof a.at === "number" ? a.at : null })),
    spaces: list(raw?.spaces)
      .filter((s) => s && word(s.id))
      .map((s) => ({
        id: s.id,
        address: word(s.address) ? s.address : s.id,
        owner: word(s.owner) ? s.owner : "",
        weaker: !!s.weaker,
        networks: list(s.networks).filter((n) => n && word(n.network)).map((n) => ({ network: n.network, level: word(n.level) ? n.level : "read" })),
        offers: list(s.offers)
          .filter((o) => o && ["moveOwner", "shareNetwork", "unshareNetwork"].includes(o.op))
          .map((o) => ({ op: o.op, network: word(o.network) ? o.network : null, fromProject: !!o.fromProject, narrowed: !!o.narrowed })),
      })),
    grants: list(raw?.grants)
      .filter((g) => g && word(g.alpn) && word(g.actor))
      .map((g) => ({
        actor: g.actor,
        alpn: g.alpn,
        key: word(g.key) ? g.key : "",
        networks: list(g.networks).filter(word),
        until: typeof g.until === "number" ? g.until : null,
        state: ["approved", "granted", "lapsed"].includes(g.state) ? g.state : "approved",
        attached: attached.some((f) => f.alpn === g.alpn && f.actor === g.actor),
        connections: list(attached.find((f) => f.alpn === g.alpn)?.connections).length,
      })),
  };
}

const short = (key) => (key.length > 14 ? `${key.slice(0, 10)}…${key.slice(-4)}` : key);
const day = (ms) => new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

// A grant's line: what it reaches, and how it stands.
export function grantLine(g, networks) {
  const names = g.networks.map((id) => networks.find((n) => n.id === id)?.name ?? id).join(", ");
  const how =
    g.state === "lapsed"
      ? "its grant ended while it was away: it asks you again"
      : g.state === "approved"
        ? "approved; it collects its grant when it next starts"
        : g.attached
          ? `syncing${g.connections ? ` with ${g.connections} machine${g.connections === 1 ? "" : "s"}` : ""}${g.until ? `, its grant renews before ${day(g.until)}` : ""}`
          : `not attached now${g.until ? `; its grant runs to ${day(g.until)}` : ""}`;
  return `reaches ${names || "no network"} · ${how}`;
}

// The Shared context section: asks with Allow and Turn down, grants with
// Take back. `networks` are this machine's ({ id, name }), ticked by
// default for an ask.
export function contextHtml(ctx, networks) {
  if (!ctx || (!ctx.asks.length && !ctx.grants.length && !ctx.spaces.length)) return "";
  const boxes = (a) =>
    networks.map((n) => `<label class="net-check"><input type="checkbox" data-net-ctx-net="${h(`${a.key}|${n.id}`)}" checked> ${h(n.name)}</label>`).join("");
  const ask = (a) =>
    `<li class="ac-row" data-net-ctx-ask="${h(a.key)}"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(nameOf(a.actor ?? a.alpn))} asks to sync over the network</span></div><div class="ac-what">${h(`context key ${short(a.key)} · on ${a.alpn}`)}</div>${boxes(a)}</div><div class="ac-acts"><button class="act" data-net-ctx-deny="${h(`${a.alpn}|${a.key}`)}">Turn down</button><button class="act primary" data-net-ctx-allow="${h(`${a.alpn}|${a.key}`)}" data-yours>Allow</button></div></li>`;
  const grant = (g) =>
    `<li class="ac-row" data-net-ctx-grant="${h(g.actor)}"><span class="ac-dot ${g.attached ? "on" : ""}"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(nameOf(g.actor))}</span>${g.attached ? `<span class="ac-badge on">attached</span>` : ""}</div><div class="ac-what">${h(grantLine(g, networks))}</div></div><div class="ac-acts"><button class="act ac-danger" data-net-ctx-revoke="${h(g.alpn)}" title="Its sync over the network ends now">Take back</button></div></li>`;
  const name = (id) => networks.find((n) => n.id === id)?.name ?? id;
  const button = (s, o) => {
    const at = `${s.id}|${o.op}|${o.network ?? ""}`;
    const words = o.op === "moveOwner" ? "Make it yours" : o.op === "shareNetwork" ? `Share into ${name(o.network)}${o.fromProject ? " (from the project)" : ""}` : `Stop sharing into ${name(o.network)}`;
    const title = o.op === "unshareNetwork" ? (o.narrowed ? "Its project isn't shared into that network any more: Xenolith holds it back already" : "That network's machines stop reading it") : "Asks you, with your device";
    return `<button class="act${o.op === "unshareNetwork" ? "" : " primary"}" data-net-ctx-sign="${h(at)}" title="${h(title)}" data-yours>${h(words)}</button>`;
  };
  const space = (s) => {
    const shared = s.networks.length ? `shared into ${s.networks.map((n) => `${name(n.network)} (${n.level})`).join(", ")}` : "shared into no network";
    const owner = s.weaker ? "the standalone owner key's" : "yours";
    const narrowed = s.offers.filter((o) => o.narrowed).map((o) => ` Its project isn't shared into ${name(o.network)} any more, so it's held back from there.`).join("");
    return `<li class="ac-row" data-net-ctx-space="${h(s.id)}"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(s.address)}</span></div><div class="ac-what">${h(`${owner} · ${shared}.${narrowed}`)}</div></div><div class="ac-acts">${s.offers.map((o) => button(s, o)).join("")}</div></li>`;
  };
  return `<div class="panel-title sub"><span>Shared context</span><span class="tick">${ctx.asks.length + ctx.grants.length}</span></div>
      <ol class="ac-list" data-net-context>${ctx.asks.map(ask).join("")}${ctx.grants.map(grant).join("")}${ctx.spaces.map(space).join("")}</ol>
      <p class="pj-note">Xenolith on this machine syncs the spaces you share into a network with that network's other machines, over XenoNet. Allow gives it a grant for the networks ticked, for 30 days, renewed while it keeps coming back; it asks you once. The grant goes to Xenolith directly, never through this window or a chat.</p>`;
}

// POST /api/net/forwards/sign's body for the button `space|op|network`.
export function signBody(at) {
  const [space, op, network] = at.split("|");
  return { space, op, ...(network ? { network } : {}), ...(op === "shareNetwork" ? { level: "append" } : {}) };
}

// POST /api/net/forwards/approve's body for the ask `alpn|key`, from the
// boxes ticked under it in `root`.
export function approveBody(root, pair) {
  const [alpn, key] = pair.split("|");
  const networks = [...root.querySelectorAll("[data-net-ctx-net]")]
    .filter((b) => b.checked && b.dataset.netCtxNet.startsWith(`${key}|`))
    .map((b) => b.dataset.netCtxNet.slice(key.length + 1));
  return { alpn, key, networks };
}
