// OpenCode as a way in (core/delivery/opencode.js): which of the running
// OpenCode servers a message goes to. Every OpenCode on a machine reads the
// same database, so every server knows every session; only the one whose
// TUI has the session open shows the turn a message starts. Made-up
// servers, folders and sessions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createOpenCode, samePath } from "../core/delivery/opencode.js";

// Servers on ports 1, 2 and 3, each started in its own folder. Every one
// knows every session, as OpenCode's shared database has it; `busy` names
// the server running a turn of it.
function servers({ dirs = { 1: "/w/ledger", 2: "/w/atlas", 3: "/w/atlas-docs" }, busy = null, sessionDir = "/w/atlas/" } = {}) {
  const posted = [];
  const fetch = async (url, opts = {}) => {
    const u = new URL(url);
    if (opts.method === "POST") return (posted.push(u.port + u.pathname), { status: 204, ok: true });
    const body =
      u.pathname === "/global/health" ? { healthy: true } :
      u.pathname === "/path" ? { directory: dirs[u.port], worktree: dirs[u.port] } :
      u.pathname === "/session/status" ? (u.port === busy ? { ses_abc123: { type: "busy" } } : {}) :
      u.pathname === "/session/ses_abc123" ? { id: "ses_abc123", title: "Ledger tests", directory: sessionDir } :
      null;
    return body ? { ok: true, status: 200, json: async () => body } : { ok: false, status: 404, json: async () => ({}) };
  };
  const oc = createOpenCode({ fetch, processes: async () => Object.keys(dirs).map((p) => ({ pid: Number(p), args: `opencode --port ${p}` })) });
  return { oc, posted };
}

test("an idle session's message goes to the OpenCode started in its folder, not the first one that knows it", async () => {
  const { oc, posted } = servers();
  assert.deepEqual(await oc.deliver("opencode:ses-abc123", "hi"), { ok: true, where: "http://127.0.0.1:2" });
  assert.deepEqual(posted, ["2/session/ses_abc123/prompt_async"]);
  assert.deepEqual((await oc.servers()).map((s) => s.dir), ["/w/ledger", "/w/atlas", "/w/atlas-docs"], "each server's own folder, from GET /path");
});

test("a server running a turn of the session still comes first, wherever it was started", async () => {
  const { oc, posted } = servers({ busy: "3" });
  await oc.deliver("opencode:ses-abc123", "hi");
  assert.deepEqual(posted, ["3/session/ses_abc123/prompt_async"]);
});

test("with no server in the session's folder, any that knows it takes the message, as before", async () => {
  const { oc, posted } = servers({ sessionDir: "/w/elsewhere" });
  await oc.deliver("opencode:ses-abc123", "hi");
  assert.deepEqual(posted, ["1/session/ses_abc123/prompt_async"]);
});

test("folders compare without a trailing slash, and on Windows whatever the case and slashes", () => {
  assert.ok(samePath("/w/atlas/", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/Atlas", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/atlas", "/w/atlas-docs", "darwin"));
  assert.ok(samePath("C:\\W\\Atlas\\", "c:/w/atlas", "win32"));
});
