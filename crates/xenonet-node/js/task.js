// Runs a Rust task (crates/xenonet-app/src/task.rs): each request it makes
// ({ op, … }) is done by `perform`, or here for a sleep, and the reply is
// handed back, until the task is over. What a request threw goes back
// described; if the task throws it on, it's thrown as it was.
export async function drive(task, perform) {
  let out = task.next(undefined, null);
  let thrown;
  for (;;) {
    if ("done" in out) return out.done;
    if (out.fail) {
      if (out.fail.rethrow) throw thrown;
      const { message, kind, ...fields } = out.fail;
      throw Object.assign(kind === "TypeError" ? new TypeError(message) : new Error(message), fields);
    }
    let ok;
    let threw = null;
    try {
      ok = out.ask.op === "sleep" ? await new Promise((r) => setTimeout(r, out.ask.ms)) : await perform(out.ask);
    } catch (err) {
      thrown = err;
      threw = describe(err);
    }
    out = task.next(ok, threw);
  }
}

// drive() for a task whose requests are all answered at once: what was a
// synchronous call in the JS stays one.
export function driveSync(task, perform) {
  let out = task.next(undefined, null);
  let thrown;
  for (;;) {
    if ("done" in out) return out.done;
    if (out.fail) {
      if (out.fail.rethrow) throw thrown;
      const { message, kind, ...fields } = out.fail;
      throw Object.assign(kind === "TypeError" ? new TypeError(message) : new Error(message), fields);
    }
    let ok;
    let threw = null;
    try {
      ok = perform(out.ask);
    } catch (err) {
      thrown = err;
      threw = describe(err);
    }
    out = task.next(ok, threw);
  }
}

// What a task reads of an error: its message, code, name and status, its
// cause's code and message (fetch's reasons), and a child process's
// standard error (execFile's).
export const describe = (err) => ({ message: String(err?.message ?? err), code: err?.code, name: err?.name, status: err?.status, causeCode: err?.cause?.code, causeMessage: err?.cause?.message, stderr: typeof err?.stderr === "string" ? err.stderr : undefined });

// A fetch as a task asks for it: `read` is "none", "json" (a body that
// isn't JSON is said so, as bodyError) or "json!" (one that isn't throws).
export async function fetchAsked(q, fetch = globalThis.fetch) {
  const init = {};
  if (q.method) init.method = q.method;
  if (q.headers) init.headers = q.headers;
  if (q.body != null) init.body = q.body;
  if (q.timeoutMs) init.signal = AbortSignal.timeout(q.timeoutMs);
  const res = await fetch(q.url, init);
  const out = { ok: !!res.ok, status: res.status, statusText: res.statusText };
  if (q.read === "json") {
    try {
      out.body = await res.json();
    } catch {
      out.bodyError = true;
    }
  } else if (q.read === "json!") out.body = await res.json();
  return out;
}
