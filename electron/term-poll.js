// How Desktop reads its terminals' output from the service (service-pty.js),
// which keeps the shells: by asking. It asks every 32 ms while a shell is
// printing or being typed into, backs off to every 250 ms while all of them
// are quiet, and doesn't ask at all with no terminal open. It used to ask
// every 32 ms for each terminal, printing or not, which kept Desktop's main
// process and the service busy all day.
//
// `read(id, cursor)` asks the service for a terminal's output since
// `cursor`: { data, cursor, exited, exitCode }. `deliver(id, read, { replay })`
// passes it on to the page, and `ready()` says whether there's a window to
// pass it to: without one, nothing is read, so nothing is lost. `later` and
// `cancel` stand in for the timers in tests.
export const FAST_MS = 32;
export const SLOW_MS = 250;

export function createTermPoll({ read, deliver, ready = () => true, later = setTimeout, cancel = clearTimeout }) {
  const watching = new Map(); // id → { cursor, replay }
  let delay = FAST_MS, timer = null, busy = false, kicked = false, stopped = false;

  function schedule() {
    if (timer != null || busy || stopped || !watching.size) return;
    timer = later(poll, delay);
  }

  // Something is about to print (a key typed, a terminal watched or
  // resized): ask again at once, and quickly after.
  function soon() {
    kicked = true;
    delay = FAST_MS;
    if (timer != null) {
      cancel(timer);
      timer = null;
    }
    schedule();
  }

  async function poll() {
    timer = null;
    busy = true;
    kicked = false;
    let printed = false;
    try {
      for (const [id, { cursor, replay }] of watching) {
        if (!ready()) break;
        try {
          const got = await read(id, cursor);
          if (!watching.has(id)) continue;
          watching.set(id, { cursor: got.cursor, replay: false });
          if (got.data) printed = true;
          deliver(id, got, { replay });
          if (got.exited) watching.delete(id);
        } catch {
          watching.delete(id);
        }
      }
    } finally {
      busy = false;
    }
    delay = printed || kicked ? FAST_MS : Math.min(SLOW_MS, delay * 2);
    schedule();
  }

  return {
    watch(id, replay = false) {
      watching.set(id, { cursor: 0, replay });
      soon();
    },
    unwatch: (id) => watching.delete(id),
    soon,
    count: () => watching.size,
    get delay() {
      return delay;
    },
    stop() {
      stopped = true;
      if (timer != null) cancel(timer);
      timer = null;
    },
  };
}
