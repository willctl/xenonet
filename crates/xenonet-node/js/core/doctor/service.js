// core/doctor/service.js in Rust (crates/xenonet-doctor/src/run.rs): the
// doctor inside the server, a minute after it starts, then every half
// hour, and whenever asked. What's a new problem and the last run's file
// are the Rust's; the schedule and the runs' promises are here.
import { native } from "../../index.js";
import { runDoctor, newProblems } from "./index.js";

export function createDoctorService({ context, only = null, file = null, everyMs = 30 * 60e3, firstAfterMs = 60e3, onResult, onNewProblems }) {
  const before = native.doctorReadRun(file ?? null);
  let last = null;
  let timer = null;
  let running = null;
  let epoch = 0;

  // One run at a time; a request during a run gets that run's result,
  // unless a result was reported or a fresh run asked for meanwhile, or a
  // run you asked for comes during a scheduled one.
  function check({ fresh = false, byHand = false } = {}) {
    if (fresh) epoch++;
    if (running?.epoch === epoch && (running.byHand || !byHand)) return running.promise;
    const previous = running?.promise.catch(() => {});
    const run = { epoch, byHand };
    run.promise = (async () => {
      await previous;
      const result = await runDoctor(await context({ byHand }), { only });
      if (run.epoch !== epoch) return result;
      const found = newProblems(last ?? before, result);
      last = result;
      native.doctorWriteRun(file ?? null, result);
      onResult?.(result);
      if (found.length) onNewProblems?.(found, result);
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
    report: (result) => (epoch++, (last = result), onResult?.(result)),
    get last() {
      return last;
    },
  };
}
