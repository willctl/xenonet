// The doctor inside the server: a minute after it starts, then every half
// hour, and whenever asked. Each run goes to `onResult` (the console shows
// it); problems that weren't there last time also go to `onNewProblems` (a
// notification through the desktop app). The last run is kept in `file`,
// so a restart doesn't report the same problems again. `only` limits the
// checks (see runDoctor). check({ fresh: true }) asks for a run that looks
// after it's asked, as after drift is marked seen. check({ byHand: true })
// is a run you asked for yourself (xenonet doctor, Check again), which
// `context` hears of: it looks for actors' tokens in repos afresh, where a
// scheduled run takes the day's look.
import fs from "node:fs";
import path from "node:path";
import { runDoctor, newProblems } from "./index.js";

export function createDoctorService({ context, only = null, file = null, everyMs = 30 * 60e3, firstAfterMs = 60e3, onResult, onNewProblems }) {
  const before = read(file);
  let last = null;
  let timer = null;
  let running = null;
  // Results reported from elsewhere, and fresh runs asked for, so far.
  let epoch = 0;

  // One run at a time; a request during a run gets that run's result. A
  // run that was under way when a result was reported, or a fresh run was
  // asked for, can't take that place, since it looked before: it's
  // answered, and the request gets a run of its own (as do those after
  // it), which starts once it's done. A fresh run is one that looks after
  // it's asked: what was just changed (drift marked seen) shows in it. A
  // run you ask for during a scheduled one gets a run of its own too, and
  // any request during a run you asked for shares it.
  function check({ fresh = false, byHand = false } = {}) {
    if (fresh) epoch++;
    if (running?.epoch === epoch && (running.byHand || !byHand)) return running.promise;
    const previous = running?.promise.catch(() => {});
    const run = { epoch, byHand };
    run.promise = (async () => {
      await previous;
      const result = await runDoctor(await context({ byHand }), { only });
      if (run.epoch !== epoch) return result;
      const fresh = newProblems(last ?? before, result);
      last = result;
      write(file, result);
      onResult?.(result);
      if (fresh.length) onNewProblems?.(fresh, result);
      return result;
    })().finally(() => {
      if (running === run) running = null;
    });
    running = run;
    return run.promise;
  }

  function start() {
    const tick = () => {
      check().catch(() => {});
      timer = setTimeout(tick, everyMs);
      timer.unref?.();
    };
    timer = setTimeout(tick, firstAfterMs);
    timer.unref?.();
  }

  return {
    start,
    stop: () => clearTimeout(timer),
    check,
    // A result from elsewhere (XenoNet Dev's simulations), shown as the last
    // run until the next one: one that starts after it.
    report: (result) => (epoch++, (last = result), onResult?.(result)),
    get last() {
      return last;
    },
  };
}

function read(file) {
  try {
    return file ? JSON.parse(fs.readFileSync(file, "utf8")) : null;
  } catch {
    return null;
  }
}

// The file says what it is (contracts, section 1); the API's runs don't.
function write(file, result) {
  if (!file) return;
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ schema: "doctor/run/v1", ...result }));
    fs.renameSync(`${file}.tmp`, file);
  } catch {}
}
