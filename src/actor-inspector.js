// What the inspector shows for an actor (src/actors.js draws them): whether
// it's there and what it's doing, how XenoNet knows of it, what it's working
// on, with each place on the map a link, and its recent events. And the
// tooltip when the pointer's over one. hud.js puts them in place.
//
// Details and statuses are plain text and escaped. XenoNet writes an event's
// detail from facts (actor/event/v1); a status line is the actor's own, and
// so are a product's id, maker and version when XenoNet doesn't know the
// product (actor-words.js). The person at the console is "You", never a
// name.
import "./actors.css";
import { escapeHtml as h, timeAgo } from "./format.js";
import { RELATION_WORDS, VIA_WORDS, PRESENCE_WORDS, CAPABILITY_WORDS, TRANSPORT_WORDS, presenceWord, gateOf } from "./actors-model.js";
import { productWhat, productMade } from "./actor-words.js";

const KIND_GLYPH = { presence: "◌", status: "✎", spawned: "▶", edited: "✎", ran: "›_", messaged: "✉", said: "✉", finished: "■", failed: "✕" };
const NODE_WORDS = { project: "a project", native: "a Codex project", zone: "a folder", claude: "a Claude Code session", codex: "a Codex session" };

const presenceOf = (s) => (PRESENCE_WORDS[s.presence] ? s.presence : "unknown");
const row = (k, v, cls = "") => (v == null || v === "" ? "" : `<div class="kv ${cls}"><span class="k">${h(k)}</span><span class="v">${v}</span></div>`);

// What it is, in a few words: "Hermes Agent", "“acme-sync”, its own word",
// "an actor of your own", "you".
export function actorWhat(s) {
  if (s.kind === "human") return "You, at the console";
  if (s.product?.id) return productWhat(s.product);
  return s.via?.includes("registered") ? "An actor of your own" : s.via?.includes("remote") ? "A remote actor" : "An actor";
}

// A node it acts on, by name when it's on the map, else by what it is.
function place(id, index) {
  const n = index.get(id);
  if (n) return { name: n.kind === "session" ? n.title : n.name, kind: n.kind, app: n.app ?? null, on: true };
  const kind = String(id).split(":")[0];
  return { name: NODE_WORDS[kind] ?? "something on the map", kind, app: null, on: false };
}

// The inspector's contents for an actor record (actors.js): { snapshot,
// color }. `index` is the map's nodes by id; `beams` the nodes it beams
// into now; `identity` names the account it acts as, or null.
export function actorInspectorHtml(rec, { index, beams = [], identity = null }) {
  const s = rec.snapshot;
  const pres = presenceOf(s);
  const live = new Set(beams);
  const sub = [productMade(s.product), s.host && `on ${s.host}`].filter(Boolean).join(" · ") || (s.kind === "human" ? "the person at this console" : s.via?.includes("registered") ? "it registered itself" : "");
  const need = s.needsYou;
  const targets = (s.targets ?? []).slice().sort((a, b) => (b.lastAt ?? 0) - (a.lastAt ?? 0));
  const targetHtml = targets
    .map((t) => {
      const p = place(t.id, index);
      const what = `${h(RELATION_WORDS[t.relation] ?? t.relation)} · ${timeAgo(t.lastAt)}`;
      const sw = p.kind === "session" ? `<i class="sw ${h(p.app ?? "")}"></i>` : `<i class="sw ${h(p.kind)}"></i>`;
      return p.on
        ? `<li data-goto="${h(t.id)}" class="${live.has(t.id) ? "live" : ""}">${sw}<span class="feed-title">${h(p.name)}</span><span class="feed-time">${what}</span></li>`
        : `<li class="gone"><i class="sw"></i><span class="feed-title">${h(p.name)}</span><span class="feed-time">${what} · not on the map</span></li>`;
    })
    .join("");
  const recent = (s.recent ?? [])
    .slice()
    .reverse()
    .slice(0, 12)
    .map((e) => `<li class="${h(e.kind)}"><span class="ev-glyph">${KIND_GLYPH[e.kind] ?? "•"}</span><span class="ev-body">${h(e.detail ?? e.kind)}${e.target && index.has(e.target) ? ` <a href="#" data-goto="${h(e.target)}">↗</a>` : ""}</span><span class="feed-time">${timeAgo(e.at)}</span></li>`)
    .join("");
  const counts = s.counts ? [`${s.counts.events24h} event${s.counts.events24h === 1 ? "" : "s"}`, s.counts.spawned24h && `${s.counts.spawned24h} session${s.counts.spawned24h === 1 ? "" : "s"} started`, s.counts.messages24h && `${s.counts.messages24h} message${s.counts.messages24h === 1 ? "" : "s"}`].filter(Boolean).join(" · ") : null;
  const grid =
    row("presence", `<span class="presence ${pres}">${h(presenceWord(s))}</span>${s.presenceSince ? ` <span class="dim">· since ${timeAgo(s.presenceSince)}</span>` : ""}`) +
    row("doing", s.status ? h(s.status) : `<span class="dim">nothing it says</span>`, "wrap") +
    row("last seen", s.lastSeen ? timeAgo(s.lastSeen) : `<span class="dim">never</span>`) +
    row("heartbeat", s.heartbeatMs ? `every ${Math.round(s.heartbeatMs / 1000)} s` : null) +
    row("last 24 h", counts ? h(counts) : null);
  const knows = (s.via ?? []).filter((v) => v !== "demo" || s.via.length === 1).map((v) => `<li>${h(VIA_WORDS[v] ?? v)}</li>`).join("");
  // Its remote connections, apart from what it can do elsewhere: it reads
  // through them, and the bridge being up says nothing of the actor itself.
  const r = s.remote;
  const remote = r ? `${r.active ? `reads only · ${r.connections} connection${r.connections === 1 ? "" : "s"}` : "no active connection"}${r.active ? ` · ${r.bridge === "connected" ? "an adapter is connected" : "no adapter connected"}` : ""} · account ${r.accountLink === "owner_asserted" ? "your word" : r.accountLink}` : null;
  // The route it takes, and for a tunnel, whether its gate on the map is
  // open: an adapter connected through it now.
  const gate = gateOf(s);
  const route = r?.active && r.transports?.length ? `${r.transports.map((t) => TRANSPORT_WORDS[t] ?? t).join(", ")}${gate != null ? ` · ${gate === 1 ? "open" : "shut: nothing is connected through it"}` : ""}` : null;
  const about =
    row("can", s.capabilities?.length ? s.capabilities.map((c) => `<span class="tag">${h(CAPABILITY_WORDS[c] ?? c)}</span>`).join(" ") : null, "wrap") + row("remote", remote ? h(remote) : null, "wrap") + row("through", route ? h(route) : null, "wrap") + row("acts as", identity ? h(identity) : null) + row("demo", s.via?.includes("demo") ? `<span class="dim">made up, like the rest of the demo</span>` : null);
  return `
    <div class="panel-title"><span class="chip">Actor · ${h(actorWhat(s))}</span><button class="close" data-close title="close">✕</button></div>
    <h2 class="insp-title">${h(s.kind === "human" ? "You" : s.name)}</h2>
    <div class="insp-sub">${h(sub)}</div>
    ${s.kind !== "human" && s.via?.includes("registered") ? `<div class="insp-actions"><button class="act" data-message-actor="${h(s.id)}" title="Write to it from here; it reads it in its inbox, marked as from you at XenoNet's window">Message…</button></div>` : ""}
    ${need ? `<div class="need-box ${h(need.kind ?? "question")}"><div class="need-k">◉ waiting for you · ${timeAgo(need.since)}</div><div class="need-text">${h(need.text ?? "")}</div></div>` : ""}
    <div class="panel-title sub"><span>Now</span><span class="tick actor-presence ${pres}">● ${h(presenceWord(s))}</span></div>
    <div class="grid">${grid}</div>
    <div class="panel-title sub"><span>How XenoNet knows it</span></div>
    ${knows ? `<ul class="actor-via">${knows}</ul>` : ""}
    <div class="grid">${about}</div>
    <div class="panel-title sub"><span>Working on</span><span class="tick">${live.size ? `${live.size} now` : targets.length || ""}</span></div>
    ${targetHtml ? `<ol class="sessions actor-targets">${targetHtml}</ol>` : `<div class="live-none">Nothing on the map lately</div>`}
    ${recent ? `<div class="panel-title sub"><span>Recent</span><span class="tick">${s.recent.length}</span></div><ol class="actor-events">${recent}</ol>` : ""}`;
}

// The tooltip: what it is, its presence and what it's doing.
export function actorTipHtml(rec) {
  const s = rec.snapshot;
  const pres = presenceOf(s);
  return `<div class="tip-k">Actor · ${h(actorWhat(s))} · ${h(presenceWord(s))}</div>
    <div class="tip-t">${h(s.kind === "human" ? "You" : s.name)}</div>
    <div class="tip-m">${s.status ? h(s.status) : s.lastSeen ? `seen ${timeAgo(s.lastSeen)}` : "not heard from yet"}</div>`;
}
