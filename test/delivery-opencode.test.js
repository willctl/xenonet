// OpenCode as a way in (core/delivery/opencode.js): which of the running
// OpenCode servers a message goes to. Every OpenCode on a machine reads the
// same database, so every server knows every session; only the one whose
// TUI has the session open shows the turn a message starts. And consent,
// kept under the id the router looks it up by. Made-up servers, folders and
// sessions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createOpenCode, samePath, openCodeId, fromOpenCodeId } from "../core/delivery/opencode.js";
import { DeliveryPolicy } from "../core/delivery/policy.js";
import { deliveryRoutes } from "../server/routes/delivery.js";

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

test("xenonet live allow takes the id OpenCode shows, keeps it as XenoNet's, and refuses what isn't a session's id", async () => {
  assert.equal(fromOpenCodeId("ses_abc123"), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId(" opencode:ses_abc123 "), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId("opencode:ses-abc123"), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId("claude:11111111-2222"), "claude:11111111-2222");
  assert.equal(openCodeId(fromOpenCodeId("ses_abc123")), "ses_abc123");
  const policy = new DeliveryPolicy();
  const route = deliveryRoutes({ policy, router: { list: () => [] }, verify: { person: async () => ({ ok: true }) }, readJson: async (r) => r.body, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (path, body) => {
    const res = {};
    await route({ method: "POST", headers: {}, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  const ok = await go("/api/delivery/allow", { session: "ses_abc123", hours: 1 });
  assert.equal(ok.status, 200);
  assert.equal(ok.body.session, "opencode:ses-abc123", "the answer says which id the consent is kept under");
  assert.ok(policy.consented("opencode:ses-abc123"), "under the id the router asks about");
  for (const bad of ["atlas", "", "opencode:", "ses_"]) {
    const res = await go("/api/delivery/allow", { session: bad });
    assert.equal(res.status, 400, bad);
    assert.match(res.body.error, /isn't a session's id/);
  }
  assert.deepEqual(Object.keys(policy.data.consent), ["opencode:ses-abc123"], "nothing kept for the refused ones");
  await go("/api/delivery/disallow", { session: "ses_abc123" });
  assert.equal(policy.consented("opencode:ses-abc123"), false);
});

test("folders compare without a trailing slash, and on Windows whatever the case and slashes", () => {
  assert.ok(samePath("/w/atlas/", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/Atlas", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/atlas", "/w/atlas-docs", "darwin"));
  assert.ok(samePath("C:\\W\\Atlas\\", "c:/w/atlas", "win32"));
});
