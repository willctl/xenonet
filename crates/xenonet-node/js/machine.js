// Drives a Rust machine (crates/xenonet-node/src/machine.rs): asynchronous
// work whose decisions are Rust's, and whose questions (a command's output,
// the clock, a collaborator's method, whatever a test passed in) are
// answered here, awaiting what they must. `handle(question)` answers one,
// or throws.
//
// A question with a `share` key waits for the answer to one asked under the
// same key that's still under way, rather than asking again, as JS's shared
// promises did; `shared` is the map of those under way, one per instance.
//
// What an answer threw is kept: if the machine fails with it, that very
// value is thrown. An answer that isn't a promise goes back at once.
export async function drive(machine, handle, shared = null) {
  const thrown = new Map();
  const failure = (err) => {
    const h = thrown.size + 1;
    thrown.set(h, err);
    return {
      message: String(err?.message ?? err),
      ...(err?.code != null ? { code: err.code } : {}),
      ...(typeof err?.stdout === "string" ? { stdout: err.stdout } : {}),
      handle: h,
    };
  };
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
      (ok) => ({ ok }),
      (err) => ({ error: failure(err) }),
    );
  };
  // A question whose answer isn't a promise is answered at once, without
  // an await, as the JS's own call was: between two calls it made without
  // awaiting, nothing else runs.
  const now = (q) => {
    if (q.share != null && shared) return null;
    let v;
    try {
      v = handle(q);
    } catch (err) {
      return { error: failure(err) };
    }
    if (v != null && typeof v.then === "function") return { later: v };
    return { ok: v };
  };
  let out = machine.next(undefined);
  for (;;) {
    if (out.ask) {
      const a = now(out.ask);
      if (a && !a.later) out = machine.next(a);
      else if (a) out = machine.next(await Promise.resolve(a.later).then((ok) => ({ ok }), (err) => ({ error: failure(err) })));
      else out = machine.next(await answer(out.ask));
    } else if (out.all) out = machine.next(await Promise.all(out.all.map(answer)));
    else break;
  }
  if ("error" in out) {
    if (out.handle != null && thrown.has(out.handle)) throw thrown.get(out.handle);
    throw Object.assign(new Error(out.error), out.code != null ? { code: out.code } : {});
  }
  return out.done;
}

const at = (root, path) => (path ? String(path).split(".").reduce((o, k) => o?.[k], root) : root);

// A handler for a machine's calls to its collaborators: `call` runs
// deps.<path>.<method>(...args), `get` reads deps.<path>; anything else
// goes to `rest`.
export const calls =
  (deps, rest = () => undefined) =>
  (q) => {
    if (q.op === "call") {
      const target = at(deps, q.path);
      return target[q.method](...(q.args ?? []));
    }
    if (q.op === "get") return at(deps, q.path);
    if (q.op === "kind") {
      const v = at(deps, q.path);
      return v === null ? "null" : typeof v;
    }
    return rest(q);
  };
