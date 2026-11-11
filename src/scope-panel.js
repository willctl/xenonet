// What an actor's grant reaches (docs/design/authorization.md, "The actor
// grant"), in the Actors panel (actors-panel.js): a line in its row, and the
// Scope view, which sets and narrows its two scopes:
//
//   - its sessions scope, the sessions it reaches (a selector such as
//     project:xenonet, with an end), set with POST /api/delivery/grant and
//     taken away with POST /api/delivery/revoke;
//   - its allow scope, what it may Allow of the permission prompts it
//     answers (every session or a list, the kinds of prompt, an end), for an
//     actor allowed to answer prompts, set with PUT /api/actors/<slug>/allows
//     and ended with DELETE.
//
// Setting either asks you (Touch ID on a Mac) when it gives the actor more;
// narrowing, taking away and ending ask nothing. Markup only: the panel
// reads the scopes (`grant`, one of GET /api/delivery's grants; `allows`,
// from GET /api/actors/tokens), holds what's typed, and answers the buttons.
import { escapeHtml as h } from "./format.js";

const DAY_MS = 24 * 3600e3;
export const PROMPT_KINDS = ["read", "edit", "command", "web", "tool"];
const KIND_WORDS = { read: "reads", edit: "edits", command: "commands", web: "web", tool: "other tools" };

const left = (until, now) => {
  const ms = until - now;
  if (!(ms > 0)) return "ended";
  const days = Math.ceil(ms / DAY_MS);
  return ms < DAY_MS ? "under a day left" : `${days} day${days === 1 ? "" : "s"} left`;
};
const live = (grant, now) => !!grant && !grant.expired && grant.expiresAt > now;
const liveAllows = (allows, now) => !!allows && allows.until > now;
const sessionsWords = (s) => (s === "all" ? "every session" : `${s.length} session${s.length === 1 ? "" : "s"}`);

// The sessions scope in words: its selector and how long it has, or none.
export function grantWords(grant, now = Date.now()) {
  return live(grant, now) ? `reaches ${grant.selector} · ${left(grant.expiresAt, now)}` : "reaches no sessions";
}

// The allow scope in words, or what an Allow is without one.
export function allowsWords(allows, now = Date.now()) {
  if (!liveAllows(allows, now)) return "allows no prompts, only denies";
  return `allows ${allows.kinds.map((k) => KIND_WORDS[k] ?? k).join(", ") || "nothing"} in ${sessionsWords(allows.sessions)} · ${left(allows.until, now)}`;
}

// The line in an actor's row: what it reaches, what it may Allow when it
// answers prompts, and Scope… to change them.
export function scopeHtml(id, { grant, allows, answers }, now = Date.now()) {
  const allow = answers ? ` · <span data-ac-allow-state>${h(allowsWords(allows, now))}</span>` : "";
  return `<div class="ac-scope-state"><span data-ac-scope-state>${h(grantWords(grant, now))}</span>${allow}<button class="ac-link" data-ac-scope="${h(id)}" title="Which sessions it reaches, and what it may allow">Scope…</button></div>`;
}

// What the Scope view's fields hold before anything's typed: the scopes as
// they are, else the defaults (30 days for sessions, a week for prompts).
export function scopeDraft({ grant, allows }, now = Date.now()) {
  const days = (until, d) => (until > now ? Math.max(1, Math.ceil((until - now) / DAY_MS)) : d);
  const a = liveAllows(allows, now) ? allows : null;
  return {
    selector: live(grant, now) ? grant.selector : "",
    sessionDays: days(live(grant, now) ? grant.expiresAt : 0, 30),
    allowAll: a ? a.sessions === "all" : true,
    allowList: a && a.sessions !== "all" ? a.sessions.join(" ") : "",
    kinds: a ? [...a.kinds] : [],
    allowDays: days(a?.until ?? 0, 7),
  };
}

// The Scope view: both scopes, each with its fields, its Set and the way
// to narrow it to nothing.
export function scopeViewHtml(v, { grant, allows, answers, draft }, now = Date.now()) {
  const sessions = `<div class="d-k">sessions it reaches</div>
      <p class="pj-note" data-ac-scope-now>${h(live(grant, now) ? `${grant.selector}, ${left(grant.expiresAt, now)}` : "None: it reaches no sessions, here or on other machines.")}</p>
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">scope</span><input class="ac-input" data-ac-scope-sessions value="${h(draft.selector)}" placeholder="project:xenonet" spellcheck="false" autocomplete="off"><small class="dim">terms such as project:, identity:, machine:, app: or by:, or all</small></label>
        <label class="ac-field"><span class="ac-k">for</span><input class="ac-input ac-beat" data-ac-scope-days type="number" min="1" max="90" step="1" value="${h(draft.sessionDays)}"><small class="dim">days, 90 at most</small></label>
      </div>
      <div class="ac-scope-acts">${live(grant, now) ? `<button class="act" data-ac-scope-revoke="${h(v.id)}" title="It reaches no sessions; asks nothing">Take away</button>` : ""}<span class="grow"></span><button class="act primary" data-ac-scope-set="${h(v.id)}" data-yours title="Asks you, since it may reach more">Set</button></div>`;
  const kinds = PROMPT_KINDS.map((k) => `<button class="toggle ${draft.kinds.includes(k) ? "on" : ""}" data-ac-allow-kind="${k}" aria-pressed="${draft.kinds.includes(k)}">${h(KIND_WORDS[k])}</button>`).join("");
  const prompts = answers
    ? `<div class="d-k">prompts it may allow</div>
      <p class="pj-note" data-ac-allow-now>${h(liveAllows(allows, now) ? allowsWords(allows, now).replace(/^allows /, "") : "None: it may deny the prompts it sees, and allows none.")}</p>
      <div class="ac-form">
        <div class="ac-field"><span class="ac-k">in</span><div class="ac-caps"><button class="toggle ${draft.allowAll ? "on" : ""}" data-ac-allow-all aria-pressed="${draft.allowAll}">every session</button></div></div>
        <label class="ac-field"><span class="ac-k">or only</span><input class="ac-input" data-ac-allow-sessions value="${h(draft.allowList)}" placeholder="claude:… codex:…" spellcheck="false" autocomplete="off" ${draft.allowAll ? "disabled" : ""}><small class="dim">session ids, separated by spaces</small></label>
        <div class="ac-field"><span class="ac-k">kinds</span><div class="ac-caps">${kinds}</div></div>
        <label class="ac-field"><span class="ac-k">for</span><input class="ac-input ac-beat" data-ac-allow-days type="number" min="1" max="365" step="1" value="${h(draft.allowDays)}"><small class="dim">days</small></label>
      </div>
      <div class="ac-scope-acts">${liveAllows(allows, now) ? `<button class="act" data-ac-allow-end="${h(v.id)}" title="It may only deny; asks nothing">End</button>` : ""}<span class="grow"></span><button class="act primary" data-ac-allow-set="${h(v.id)}" data-yours title="Asks you when it allows more than now">Set</button></div>`
    : `<p class="pj-note">It can't answer permission prompts: that's the answer-prompts capability, which it wasn't allowed.</p>`;
  return `<p class="pj-note">Setting either asks you when it gives more. Narrowing, taking away and ending ask nothing.</p>
      ${sessions}
      ${prompts}
      <p class="ac-error bad" data-ac-error hidden></p>
      <div class="m-foot"><span class="grow"></span><button class="act" data-ac-back>Done</button></div>`;
}

// What's typed in the Scope view, else what was there before (`draft`).
export function readScope(body, draft) {
  const val = (sel, d) => body.querySelector(sel)?.value ?? d;
  const all = body.querySelector("[data-ac-allow-all]");
  const kinds = body.querySelector("[data-ac-allow-kind]") ? [...body.querySelectorAll("[data-ac-allow-kind].on")].map((b) => b.dataset.acAllowKind) : draft.kinds;
  return {
    selector: val("[data-ac-scope-sessions]", draft.selector).trim(),
    sessionDays: Number(val("[data-ac-scope-days]", draft.sessionDays)),
    allowAll: all ? all.classList.contains("on") : draft.allowAll,
    allowList: val("[data-ac-allow-sessions]", draft.allowList).trim(),
    kinds,
    allowDays: Number(val("[data-ac-allow-days]", draft.allowDays)),
  };
}

// The calls the two Sets make, from what's typed, or why not.
export function grantBody(id, d) {
  if (!d.selector) return { error: "Say which sessions it reaches, such as project:xenonet, or all" };
  if (!Number.isFinite(d.sessionDays) || d.sessionDays < 1 || d.sessionDays > 90) return { error: "A sessions scope lasts 1 to 90 days" };
  return { body: { actor: id, scope: d.selector, hours: Math.round(d.sessionDays) * 24 } };
}

// The allow scope's end stays as it is while its days aren't changed, so
// taking a kind or a session away narrows it and asks nothing.
export function allowsBody(d, { allows = null } = {}, now = Date.now()) {
  const list = d.allowList.split(/\s+/).filter(Boolean);
  if (!d.allowAll && !list.length) return { error: "Say which sessions, or choose every session" };
  if (!d.kinds.length) return { error: "Choose the kinds of prompt it may allow" };
  if (!Number.isFinite(d.allowDays) || d.allowDays < 1 || d.allowDays > 365) return { error: "An allow scope lasts 1 to 365 days" };
  const kept = liveAllows(allows, now) && Math.round(d.allowDays) === scopeDraft({ allows }, now).allowDays;
  return { body: { sessions: d.allowAll ? "all" : list, kinds: d.kinds, until: kept ? allows.until : now + Math.round(d.allowDays) * DAY_MS } };
}
