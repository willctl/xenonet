// What waits on you, as one queue: sessions waiting on a question or a
// permission, actors waiting on you, programs asking to be actors, and
// sessions' asks for an actor's gateway token. The Needs you panel
// (src/needs-you.js) lists it and answers it, and the tray and xenonet
// status read it from /api/summary.
//
// The queue is in the order things started waiting, the longest wait
// first, so the next one to answer is always at the top. What XenoNet
// can't date goes after what it can, and a wait on another machine that
// isn't live, which may be over, goes last.
//
// Each item says what answers it (`actions`):
//   show      selects it on the map, where its inspector says the rest
//   open      opens a session in its app (this machine's sessions only)
//   dismiss   puts a session's wait away, as the strip's ✓ does
//   message   writes to an actor, from XenoNet's window
//   allow     a request's Allow, which only XenoNet's window can press, a
//   deny      moment after the request shows, and its Deny
// A window without the key (a browser tab) knows only how many programs
// ask, never who: one `requests` item says how many, with nothing to press.
//
// Ready-for-review sessions, collisions, stuck loops and the doctor's
// problems stay in the attention strip: they ask you to look, not to answer.
//
// No Node here: the page imports it too.

export const KINDS = ["question", "permission", "actor", "request", "gateway", "requests", "ask", "sign-in"];

// What each kind of item waits for, in a few words after its name.
const ACTOR_WAITS = { question: "asks", permission: "needs permission", review: "is ready for review" };
export function waitsWords(it) {
  if (it.kind === "question") return "asks";
  if (it.kind === "permission") return "needs permission";
  if (it.kind === "actor") return ACTOR_WAITS[it.waits] ?? "waits for you";
  if (it.kind === "request") return "asks to join XenoNet";
  if (it.kind === "gateway") return "is asked for a gateway token";
  if (it.kind === "ask") return "wants to run a tool";
  if (it.kind === "sign-in") return "needs a sign-in for XenoNet's runs";
  if (it.kind === "requests") return it.count === 1 ? "asks to join XenoNet" : "ask to join XenoNet";
  return "waits for you";
}

const num = (v) => (Number.isFinite(v) ? v : null);
const str = (v) => (typeof v === "string" ? v : v == null ? null : String(v));

// sessions: [{ id, app, title, kind (question or permission), text, since,
//              host, stale }], as the live snapshot has them with their
//              titles, or /api/summary's needsYou
// actors:   actors as actor/list/v1 has them; those with needsYou wait
// requests, gatewayAsks: the window's view of registrations
//              (src/actor-requests.js), with `canDecide`
// waiting:  how many programs ask, for a window without the key
// asks:     the permission prompts of the runs XenoNet started, here or on
//           another machine ({ id, host, session, run, title, tool, input,
//           at, until }), which only the window answers
// hints:    machines whose Claude Code needs a sign-in for XenoNet's runs
//           ({ host, fix, since })
export function needsYouQueue({ sessions = [], actors = [], requests = [], gatewayAsks = [], canDecide = false, waiting = 0, asks = [], hints = [] } = {}) {
  const items = [];
  for (const s of sessions ?? []) {
    if (!s?.id || (s.kind !== "question" && s.kind !== "permission")) continue;
    const local = !s.host;
    items.push({
      key: `session:${s.id}`,
      kind: s.kind,
      id: s.id,
      title: str(s.title) ?? s.id,
      app: str(s.app),
      text: str(s.text),
      since: num(s.since),
      ...(s.host ? { host: s.host } : {}),
      ...(s.stale ? { stale: true } : {}),
      actions: local ? ["show", "open", "dismiss"] : ["show"],
    });
  }
  for (const a of actors ?? []) {
    if (!a?.needsYou || a.kind === "human" || a.id === "actor:you") continue;
    items.push({
      key: `actor:${a.id}`,
      kind: "actor",
      id: a.id,
      title: str(a.name) ?? a.id,
      waits: str(a.needsYou.kind),
      text: str(a.needsYou.text) || "Waiting for you",
      since: num(a.needsYou.since),
      actions: canDecide ? ["show", "message"] : ["show"],
    });
  }
  for (const h of hints ?? []) {
    if (typeof h?.host !== "string") continue;
    items.push({ key: `sign-in:${h.host}`, kind: "sign-in", id: h.host, title: `Claude Code on ${h.host}`, host: h.host, text: str(h.fix), since: num(h.since), actions: ["dismiss"] });
  }
  if (canDecide) {
    for (const a of asks ?? []) {
      if (!a?.id) continue;
      items.push({ key: `ask:${a.id}`, kind: "ask", id: a.id, title: str(a.title) ?? a.id, host: str(a.host), session: str(a.session), run: str(a.run), tool: str(a.tool), text: str(a.input), since: num(a.at), expiresAt: num(a.until), actions: ["allow", "deny"] });
    }
  }
  if (canDecide) {
    for (const r of requests ?? []) if (r?.id) items.push({ key: `request:${r.id}`, kind: "request", id: r.id, title: str(r.name) ?? r.id, actor: str(r.actor), since: num(r.at), expiresAt: num(r.expiresAt), actions: ["allow", "deny"] });
    for (const g of gatewayAsks ?? []) if (g?.id) items.push({ key: `gateway:${g.id}`, kind: "gateway", id: g.id, title: str(g.name) ?? g.id, actor: str(g.actor), since: num(g.at), expiresAt: num(g.expiresAt), actions: ["allow", "deny"] });
  } else if (Number.isInteger(waiting) && waiting > 0) {
    items.push({ key: "requests", kind: "requests", id: null, title: waiting === 1 ? "A program" : `${waiting} programs`, count: waiting, since: null, actions: [] });
  }
  return items.sort(order);
}

// The longest wait first; undated after dated, a host's stale waits last;
// then by key, so two that started together keep their places.
export function order(a, b) {
  return !!a.stale - !!b.stale || (a.since == null) - (b.since == null) || (a.since ?? 0) - (b.since ?? 0) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
}

// The queue from /api/summary (state.summary with the server's additions):
// its sessions, the actors that wait and how many programs ask.
export function queueOfSummary(summary) {
  return needsYouQueue({ sessions: summary?.needsYou ?? [], actors: (summary?.actorsNeedYou ?? []).map((a) => ({ id: a.id, name: a.name, needsYou: { kind: a.kind, text: a.text, since: a.since } })), waiting: summary?.requests ?? 0 });
}

// How many wait on you: every program that asks counts, though a window
// without the key shows them as one item, and a wait on a host that isn't
// live doesn't, as the strip's count leaves it out.
export const countOf = (items) => items.reduce((n, it) => n + (it.stale ? 0 : it.kind === "requests" ? it.count : 1), 0);
