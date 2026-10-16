// You at the console, actor:you (docs/design/actors/interface.md, section
// 6): the page tells XenoNet what you do here, so the actors layer can draw
// you. It goes only to XenoNet's own server, with the key only this window
// holds (from the desktop app, through the preload bridge), and it's only
// node ids and the names of actions. Never what you type into the palette
// or the terminal, never your searches, the pointer, the camera, filters,
// themes, or the panels you open. In a browser tab there's no key, so
// nothing is sent; nor when XenoNet's actors are off, or you've turned
// yourself off the map, which deletes what XenoNet kept of you at once.
//
//   presence  working: input in the last two minutes, in the focused
//             window; idle: the window shows, with no input for 2 to 10
//             minutes; away: no input for 10 minutes, or the window hidden
//   did       looked (a selection that held 3 s in view, so stepping
//             through a gathering reports only where you stop), read,
//             opened, handed-off, started, terminal
//
// It reports on each change of presence and every minute while the window
// shows, at most once a second, with whatever you did meanwhile. Hidden, it
// says you're away once and then nothing, so ten minutes on XenoNet has you
// offline (ten silent heartbeats), and after ten minutes it sends nothing
// at all until the window shows again. The desktop app says when its
// window hides (onWindowState), and its word is the one that counts: its
// page also reads as hidden while other windows cover it, which is still
// you at the computer, not away; a test window that's never shown counts
// as shown.
//
// With the same key it reads your record back (GET /api/actors/you), which
// no other program can, so the layer draws where you've looked, after a
// reload too.

import { createWindowCalls } from "./actor-requests.js";

const LOOK_MS = 3000;
const BEAT_MS = 60e3;
const SEND_MS = 1000;
const CHECK_MS = 5000;
const WORKING_MS = 2 * 60e3;
const AWAY_MS = 10 * 60e3; // as "while you were away" counts it (main.js)
const QUEUE_MAX = 20;
// "Show me on the map": off keeps this page from reporting you at all.
export const SWITCH = "xenonet.you";
const WAIT_MS = 50;

// `calls` are the window's own calls with its key (actor-requests.js), for
// your record; the reports keep their own key and pace. `onRecord(record)`
// hears each time your record is read, or let go.
export function createYou({
  desktop = null,
  doc = globalThis.document,
  win = globalThis.window,
  storage = globalThis.localStorage,
  request = (...a) => globalThis.fetch(...a),
  calls = createWindowCalls({ desktop, request }),
  onRecord = () => {},
  now = Date.now,
  later = (fn, ms) => setTimeout(fn, ms),
  cancel = (t) => clearTimeout(t),
  every = (fn, ms) => setInterval(fn, ms),
} = {}) {
  let shown = true;
  try {
    shown = storage?.getItem(SWITCH) !== "off";
  } catch {}
  // Actors off on the server (503) stay off for as long as the page lives.
  let off = !desktop?.windowKey;
  let key = null;
  // When the desktop app last had no key to give (a server it didn't
  // start): asked again a minute later, not on every keystroke.
  let keyless = -Infinity;
  let lastInput = now();
  let sentPresence = null;
  let sentAt = -Infinity;
  let timer = null;
  let looking = null;
  let busy = false;
  // The desktop app's word on its window, and since when it's been hidden
  // (the window, or a browser tab's page), or null while it shows.
  let windowShown = true;
  let hiddenAt = null;
  // What XenoNet keeps of you, your actor/snapshot/v1 as only this window
  // may read it (GET /api/actors/you): where you've looked and what you've
  // done, which the actors' list leaves out. Read once you've been
  // reported, so after a reload your beams come back, and again after what
  // you did has been taken. Null until then, and once you're off the map.
  let record = null;
  let unread = true;
  let reading = null;
  const queue = [];
  const live = () => shown && !off;
  const hidden = () => (!desktop?.onWindowState && doc?.visibilityState === "hidden") || !windowShown;
  // Hidden ten minutes: XenoNet has you offline by now, and nothing more goes.
  const gone = () => hiddenAt != null && now() - hiddenAt >= AWAY_MS;

  function presence() {
    if (hidden()) return "away";
    const quiet = now() - lastInput;
    if (quiet >= AWAY_MS) return "away";
    return doc?.hasFocus?.() && quiet < WORKING_MS ? "working" : "idle";
  }

  // A report goes at most once a second; what's done meanwhile waits for it.
  function schedule(ms = Math.max(0, sentAt + SEND_MS - now())) {
    if (timer != null || !live() || gone()) return;
    timer = later(() => {
      timer = null;
      flush();
    }, ms);
  }

  // What didn't go out goes back in front, the newest of it when there's
  // no room for all.
  function requeue(did) {
    const room = QUEUE_MAX - queue.length;
    if (room > 0) queue.unshift(...did.slice(-room));
  }

  async function flush() {
    if (!live()) return void (queue.length = 0);
    if (gone()) return;
    if (busy) return schedule();
    if (!key && now() - keyless < BEAT_MS) return;
    busy = true;
    try {
      // A refusal may mean the server started again with a new key: ask
      // for it once more before giving up on what was sent.
      for (let tries = 0; tries < 2; tries++) {
        key ??= (await desktop.windowKey().catch(() => null)) ?? null;
        // No key (a server the desktop app didn't start): nothing to say it with.
        if (!key) return void (keyless = now());
        // Turned off the map while the key was on its way: nothing goes.
        if (!live()) return void (queue.length = 0);
        const did = queue.splice(0, QUEUE_MAX);
        const body = { schema: "actor/console/v1", presence: presence(), ...(did.length ? { did } : {}) };
        sentAt = now();
        let res;
        try {
          res = await request("/api/actors/you", { method: "POST", headers: { "content-type": "application/json", "x-xenonet-window": key }, body: JSON.stringify(body) });
        } catch {
          return requeue(did);
        }
        if (res.ok) {
          sentPresence = body.presence;
          if (did.length) unread = true;
          if (unread) read();
          return;
        }
        if (res.status === 503) return void ((off = true), (queue.length = 0));
        if (res.status === 429) {
          requeue(did);
          return schedule((Number(res.headers?.get?.("retry-after")) || 1) * 1000);
        }
        if (res.status !== 403) return;
        key = null;
        if (!tries) requeue(did);
      }
    } finally {
      busy = false;
    }
  }

  // Whether the window or the page is hidden now, noted as it changes: a
  // look still in progress is no look, and away is said once. Shown again,
  // you're said at once, whatever your presence, since the words stopped.
  function seen() {
    if (!hidden()) {
      if (hiddenAt != null) sentPresence = null;
      return void (hiddenAt = null);
    }
    if (hiddenAt != null) return;
    hiddenAt = now();
    if (looking != null) cancel(looking);
    looking = null;
  }

  function check() {
    seen();
    if (live() && presence() !== sentPresence) schedule();
  }

  // The desktop app's word on its window: "shown" or "hidden".
  function windowSays(state) {
    if (state !== "shown" && state !== "hidden") return;
    windowShown = state === "shown";
    check();
  }

  // Your record, read with the window's key; one read at a time, and one
  // more after it when something asked meanwhile. A XenoNet that won't give
  // it (actors off, a refused key) isn't asked again until you do something.
  async function read() {
    if (reading) return void (reading.again = true);
    reading = { again: false };
    try {
      unread = false;
      const r = await calls.call("GET", "/api/actors/you");
      if (r.ok && live() && r.data?.id === "actor:you") {
        record = r.data;
        onRecord(record);
      }
    } finally {
      const again = reading.again;
      reading = null;
      if (again && live()) read();
    }
  }

  // Off the map, what XenoNet kept of you goes at once (DELETE
  // /api/actors/you), after any report still on its way, which could bring
  // it back. Whether it went: a XenoNet without the route (404), with actors
  // off, or reached from a browser tab keeps it, for a day at most.
  async function forget() {
    while (busy) await new Promise((resolve) => later(resolve, WAIT_MS));
    if (shown) return { deleted: false };
    const r = await calls.call("DELETE", "/api/actors/you");
    return { deleted: r.ok };
  }

  function did(what, node, extra = {}) {
    if (!live() || typeof node !== "string" || !node) return;
    const d = { what, node, at: now() };
    if (extra.to) d.to = extra.to;
    if (extra.app) d.app = extra.app;
    queue.push(d);
    if (queue.length > QUEUE_MAX) queue.shift();
    schedule();
  }

  for (const type of ["pointerdown", "keydown", "wheel"]) {
    win?.addEventListener?.(
      type,
      () => {
        lastInput = now();
        check();
      },
      { capture: true, passive: true },
    );
  }
  win?.addEventListener?.("focus", check);
  win?.addEventListener?.("blur", check);
  doc?.addEventListener?.("visibilitychange", check);
  desktop?.onWindowState?.(windowSays);
  every(check, CHECK_MS);
  every(() => !hidden() && schedule(), BEAT_MS);
  // Say you're here as soon as the page is, once the desktop app has said
  // whether its window shows: XenoNet started at login starts hidden.
  Promise.resolve(desktop?.windowState?.())
    .then(windowSays, () => {})
    .then(() => schedule(0));

  return {
    // Something you did to a node of the map: read, opened, handed-off
    // (with `to`), started (with `app`) or terminal.
    did,
    // The selection, reported once it has held for 3 s in view.
    looked(node) {
      if (looking != null) cancel(looking);
      looking = node && live() && !hidden() ? later(() => ((looking = null), did("looked", node)), LOOK_MS) : null;
    },
    // "Show me on the map" (for the actors layer): off reports nothing
    // more and deletes your record, on starts again. Kept for this window.
    // Resolves to { deleted }: whether turning it off deleted the record.
    setShown(on) {
      shown = !!on;
      try {
        storage?.setItem(SWITCH, shown ? "on" : "off");
      } catch {}
      if (shown) {
        schedule(0);
        return Promise.resolve({ deleted: false });
      }
      queue.length = 0;
      if (looking != null) cancel(looking);
      looking = null;
      record = null;
      unread = true;
      onRecord(null);
      return forget();
    },
    shown: () => shown,
    reporting: live,
    presence,
    // Your record as last read, or null (youHere draws you from it).
    record: () => record,
    // How long the window (or the page) has been hidden, in ms, or null
    // while it shows: the page draws you as XenoNet has you (youHere).
    hiddenMs() {
      seen();
      return hiddenAt == null ? null : now() - hiddenAt;
    },
  };
}
