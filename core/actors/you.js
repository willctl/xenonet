// You at the console, actor:you (docs/design/actors/interface.md, section
// 6). XenoNet's window alone reports you, with the key only it holds (see
// server/routes/actors.js): your presence, from input in the focused
// window, and what you did there, as node ids and action names. XenoNet
// writes the words from its own map. The desktop app adds whether the
// window shows, over the utility process's port. Nothing you type is ever
// sent, and you're never named.
//
// What becomes an event:
//
//   looked      a selection that held 3 s   status   the node, watches
//   read        a preview, or stats         status   the session, reviews
//   opened      a session in its app        status   the session, reviews
//   terminal    a terminal in a folder      status   the place, works-on
//   started     a new session in a place    spawned  the place, works-on
//   handed-off  a session to another app    spawned  its place, works-on
//
// A handoff or a new session targets the place: the new session's id isn't
// known when you act, and XenoNet doesn't guess it from timing.
import { YOU } from "../schema/ids.js";
import { consoleStatus } from "./detail.js";

// The page reports every minute while the window shows, so two missed are
// away and ten (ten minutes of silence) are offline.
export const YOU_HEARTBEAT_MS = 60e3;
const DAY_MS = 24 * 3600e3;
const AHEAD_MS = 60e3;
const PLACES = new Set(["project", "zone", "native"]);
const APPS = new Set(["claude", "codex"]);
// What each action may be done to.
const ON = {
  looked: (k) => PLACES.has(k) || k === "session",
  read: (k) => k === "session",
  opened: (k) => k === "session",
  terminal: (k) => PLACES.has(k) || k === "session",
  started: (k) => PLACES.has(k),
  "handed-off": (k) => k === "session",
};
const RELATION = { looked: "watches", read: "reviews", opened: "reviews", terminal: "works-on" };

export function createYou({ actors }) {
  // The desktop app's word for the window (shown, hidden or closed), and
  // the presence the page last reported (working, idle or away). The
  // window's word counts only once the page has said something: the page
  // reports nothing while "Show me on the map" is off, so neither does the
  // window, and once you turn yourself off (`off`), nothing more is said of
  // you until the page reports again.
  let shown = "shown";
  let page = null;
  let off = false;
  const presence = () => (page == null ? null : shown === "closed" ? "offline" : shown === "hidden" ? "away" : page);

  function settle(at) {
    const p = presence();
    if (p) actors.report(YOU, { instance: "window", presence: p, heartbeatMs: YOU_HEARTBEAT_MS, via: "observed", at });
  }

  const placeOf = (session) => actors.map?.describe?.(session)?.placeId ?? null;

  // One thing you did, recorded; or why it isn't.
  function act(d, now) {
    if (!d || !Object.hasOwn(ON, d.what)) return "XenoNet doesn't know that action";
    let at = Number.isFinite(d.at) ? d.at : now;
    if (now - at > DAY_MS) return "that was more than a day ago";
    if (at - now > AHEAD_MS) at = now;
    if (actors.map?.has && !actors.map.has(d.node)) return "XenoNet doesn't know that node";
    const where = actors.where(d.node);
    if (!where || !ON[d.what](where.kind)) return `you can't do that to a ${where?.kind ?? "node like that"}`;
    if (d.what === "started") {
      actors.record(YOU, { kind: "spawned", target: d.node, relation: "works-on", app: APPS.has(d.app) ? d.app : "claude", via: "observed", at });
      return null;
    }
    if (d.what === "handed-off") {
      if (!APPS.has(d.to)) return "a handoff says which app it went to";
      const place = placeOf(d.node);
      if (!place) return "XenoNet doesn't know where that session lives";
      actors.record(YOU, { kind: "spawned", target: place, relation: "works-on", app: where.app, to: d.to, via: "observed", at });
      return null;
    }
    const target = d.what === "terminal" && where.kind === "session" ? placeOf(d.node) : d.node;
    const line = consoleStatus(d.what, where);
    if (!target || !line) return "XenoNet can't say what that is";
    actors.record(YOU, { kind: "status", status: line, target, relation: RELATION[d.what], via: "observed", at });
    return null;
  }

  return {
    // The page's report, an actor/console/v1 that fits its schema: its
    // presence (a report without one is a heartbeat that keeps the last),
    // and what you did. Says how many actions were taken, and why the
    // others weren't.
    report(body = {}, { at = actors.now() } = {}) {
      off = false;
      if (body.presence) page = body.presence;
      let accepted = 0;
      const dropped = [];
      actors.store.transaction(() => {
        settle(at);
        (body.did ?? []).forEach((d, index) => {
          const why = act(d, at);
          if (why) dropped.push({ index, reason: why });
          else accepted++;
        });
      });
      return { accepted, dropped };
    },

    // The desktop app says the window shows, hid (away) or closed (offline).
    window(state, at = actors.now()) {
      if (!["shown", "hidden", "closed"].includes(state)) return;
      shown = state;
      settle(at);
    },

    // You turned yourself off the map (DELETE /api/actors/you): your record
    // goes at once, and nothing more is said of you, not even the window's
    // word, until the page reports again.
    forget() {
      [page, off] = [null, true];
      actors.forgetYou();
    },
    // Whether you've turned yourself off since the page last reported: the
    // demo's made-up you keeps still meanwhile.
    off: () => off,

    presence,
  };
}
