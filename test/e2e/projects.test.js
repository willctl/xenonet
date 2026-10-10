// End to end: how XenoNet reads the projects folder, on a real XenoNet Dev (the
// test instance, with the demo's made-up data; see xenonet.test.js). The demo
// has `labs`, a plain folder holding two repos and a loose `snippets` folder.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const api = (route, opts) => fetch(`http://127.0.0.1:${port}${route}`, opts).then((r) => r.json());
// Asking Codex and forgetting its picks are yours (verify.person), and these
// tests run under an agent, so they're done from the page, as XenoNet's own
// window does them.
const fromWindow = (route, method, body) => s.eval(`return fetch(${JSON.stringify(route)}, { method: ${JSON.stringify(method)}, headers: { "content-type": "application/json" }, body: ${JSON.stringify(JSON.stringify(body))} }).then((r) => r.json())`);
const labs = async () => (await api("/api/graph")).nodes.filter((n) => n.kind === "project" && /^labs(\/|$)/.test(n.rel ?? "")).map((n) => `${n.rel}:${n.role}`).sort();

// Waits for the map to show what `want` says, after something changed.
async function until(want, ms = 8000) {
  const end = Date.now() + ms;
  let got;
  while (Date.now() < end && !want((got = await labs()))) await sleep(200);
  return got;
}

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("a folder of repos is a group, and selecting it gathers its projects", { timeout: 60000 }, shoot("projects-group", async () => {
  assert.deepEqual(await labs(), ["labs/geofence:project", "labs/route-sim:project", "labs:group"]);
  const { nodes } = await s.driver("find", { query: "labs", kind: "project", limit: 1 });
  const st = await s.driver("select", nodes[0].id);
  assert.equal(st.gathered?.root, nodes[0].id);
  const projects = (await api("/api/graph")).nodes.filter((n) => /^labs\//.test(n.rel ?? "")).map((n) => n.id);
  for (const id of projects) assert.ok(st.gathered.members.includes(id), `${id} is gathered`);
}));

test("the palette makes the group one project, and undoes it", { timeout: 60000 }, shoot("projects-rule", async () => {
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "> treat labs as one project" });
  await s.input({ keys: "Enter" });
  assert.deepEqual(await until((g) => g.length === 1), ["labs:project"]);
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "> undo your rule for labs" });
  await s.input({ keys: "Enter" });
  assert.deepEqual(await until((g) => g.length === 3), ["labs/geofence:project", "labs/route-sim:project", "labs:group"]);
}));

test("asking Codex shows the outline first, and Send records its picks", { timeout: 60000 }, shoot("projects-codex", async () => {
  await s.input({ keys: "Escape" });
  await s.driver("command", "projects-codex");
  let rows = 0;
  for (const end = Date.now() + 5000; Date.now() < end && !rows; await sleep(200)) rows = await s.eval('document.querySelectorAll(".pj-table tbody tr").length');
  assert.ok(rows >= 8, `the outline lists the folders: ${rows}`);
  assert.equal((await api("/api/projects")).codex, null, "nothing is asked before Send");
  await s.eval('document.querySelector("[data-pj-send]").click()');
  assert.deepEqual(await until((g) => g.length === 4), ["labs/geofence:project", "labs/route-sim:project", "labs/snippets:project", "labs:group"]);
  const { codex } = await api("/api/projects");
  assert.deepEqual(codex.picks.map((p) => [p.path, p.as]), [["labs/snippets", "project"]]);
  assert.equal(codex.stale, false);
  await fromWindow("/api/projects/codex", "DELETE");
  assert.equal((await until((g) => g.length === 3)).length, 3);
  await s.input({ keys: "Escape" });
}));

test("no page errors", { timeout: 30000 }, async () => {
  const { entries } = await s.logs({ level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
