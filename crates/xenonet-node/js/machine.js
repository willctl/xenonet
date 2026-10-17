// Drives a Rust machine (crates/xenonet-node/src/machine.rs): asynchronous
// work whose decisions are Rust's, and whose questions (a command's output,
// the clock, a store, whatever a test passed in) are answered here, awaiting
// what they must. `handle(question)` answers one, or throws.
//
// A question with a `share` key waits for the answer to one asked under the
// same key that's still under way, rather than asking again, as JS's shared
// promises did; `shared` is the map of those under way, one per instance.
export async function drive(machine, handle, shared = null) {
  const answer = (q) => {
    let p;
    const run = () => {
      try {
        return Promise.resolve(handle(q));
      } catch (err) {
        return Promise.reject(err);
      }
    };
    if (q.share != null && shared) {
      p = shared.get(q.share);
      if (!p) {
        p = run();
        shared.set(q.share, p);
        const drop = () => shared.get(q.share) === p && shared.delete(q.share);
        p.then(drop, drop);
      }
    } else p = run();
    return p.then(
      (ok) => ({ ok: ok === undefined ? null : ok }),
      (err) => ({ error: failure(err) }),
    );
  };
  let out = machine.next(undefined);
  for (;;) {
    if (out.ask) out = machine.next(await answer(out.ask));
    else if (out.all) out = machine.next(await Promise.all(out.all.map(answer)));
    else break;
  }
  if ("error" in out) throw Object.assign(new Error(out.error), out.code ? { code: out.code } : {});
  return out.done;
}

// What an error thrown here tells the Rust: its message, its code, and the
// output a failed command wrote anyway.
const failure = (err) => ({
  message: String(err?.message ?? err),
  ...(err?.code != null ? { code: err.code } : {}),
  ...(typeof err?.stdout === "string" ? { stdout: err.stdout } : {}),
});

