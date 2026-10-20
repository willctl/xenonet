// core/script.js in Rust (crates/xenonet-app/src/script.rs): steps run
// against a XenoNet Dev instance, one after another. Each step is a Rust
// state machine that says what to call next on the control client given
// (its driver, launch, input, window…), what to say and how long to wait;
// this makes the calls.
import { native } from "../index.js";
import { peer } from "../peer.js";

const { filmPlaySeconds } = await import(peer("frames.js"));

export const STEPS = native.appScriptSteps();

export function brief(st) {
  const r = native.appScriptBrief(st);
  if (r.error) throw new TypeError(r.error);
  return r.ok;
}

// One step: a string ("select:atlas-api") or { op, args }. `say` gets what
// each step has to show.
export async function runStep(s, step, say = () => {}) {
  let { state, out } = native.appScriptBegin(step, String(s.name), filmPlaySeconds());
  let thrown;
  for (;;) {
    for (const x of out.say) say(x);
    const then = out.then;
    if (then.done) return;
    if (then.rethrow) throw thrown;
    if (then.fail) throw then.fail.typeError ? new TypeError(then.fail.message) : new Error(then.fail.message);
    let ok, threw = null;
    if ("sleep" in then) await new Promise((r) => setTimeout(r, then.sleep));
    else {
      const { target, args } = then.call;
      try {
        ok = await (target === "driver" ? s.driver(...args) : s[target](...args));
      } catch (err) {
        thrown = err;
        threw = String(err?.message ?? err);
      }
    }
    ({ state, out } = native.appScriptResume(state, ok, threw));
  }
}

export async function runScript(s, steps, say) {
  for (const step of steps) await runStep(s, step, say);
}
