// End to end: changing the projects folder on a real XenoNet Dev (the test
// instance, with the demo's made-up data; see xenonet.test.js). Besides
// ~/Projects the demo has ~/Clients: three repos and a labs group, whose
// loose snippets folder has the same path in it as ~/Projects' has, so
// Codex's picks for one folder can be told from the other's.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const api = (route, method = "GET", body) => fetch(`http://127.0.0.1:${port}${route}`, { method, ...(body ? { headers: { "content-type": "application/json" }, body: JSON.stringify(body) } : {}) }).then((r) => r.json());
// Your rules, and asking Codex and forgetting its picks, are yours
// (verify.person), and these tests run under an agent, so they're done from
// the page, as XenoNet's own window does them.
const fromWindow = (route, method, body) => s.eval(`return fetch(${JSON.stringify(route)}, { method: ${JSON.stringify(method)}, headers: { "content-type": "application/json" }, body: ${JSON.stringify(JSON.stringify(body))} }).then((r) => r.json())`);
// The projects on the map, with whose rule made each one, if any.
const projects = async () => (await api("/api/graph")).nodes.filter((n) => n.kind === "project").map((n) => `${n.rel}${n.rule ? `:${n.rule.by}` : ""}`).sort();
const CLIENTS = ["harbor-api", "harbor-app", "kiln", "labs", "labs/glaze"];
const askCodex = async () => fromWindow("/api/projects/codex", "POST", { fingerprint: (await api("/api/projects/codex")).fingerprint });

// Waits for `get` to give what `want` says.
async function until(get, want, ms = 10000) {
  let got;
  for (const end = Date.now() + ms; !want((got = await get())) && Date.now() < end; ) await sleep(150);
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
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("the projects folder and where it comes from, and a check of another", { timeout: limit(60000) }, shoot("root-check", async () => {
  const root = await s.driver("root");
  assert.deepEqual([root.source, path.basename(root.path), root.scan.ok], ["default", "Projects", true]);
  assert.equal(await s.eval('document.getElementById("root-name").textContent'), "~/Projects");
  const clients = await s.driver("checkRoot", "~/Clients");
  assert.deepEqual([clients.ok, clients.current, clients.scan.projects, clients.scan.groups], [true, false, 4, 1]);
  assert.match((await s.driver("checkRoot", "~")).error, /your home folder/);
  assert.match((await s.driver("checkRoot", "~/nowhere")).error, /doesn't exist/);
  assert.equal((await s.driver("checkRoot", root.path)).current, true);
}));

test("the palette's panel checks a folder as you type, and Use this folder reads it", { timeout: limit(90000) }, shoot("root-panel", async () => {
  await s.input({ keys: "Escape" });
  const before = await s.driver("state");
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "> change the projects folder" });
  await s.input({ keys: "Enter" });
  const focused = await until(() => s.eval('!!document.activeElement?.matches?.("[data-root-field]")'), (v) => v);
  assert.ok(focused, "the panel opens with its field ready");
  await s.input({ text: "~/Clients" });
  const answer = await until(() => s.eval('document.querySelector("[data-root-answer]")?.textContent ?? ""'), (t) => /projects? in/.test(t));
  assert.match(answer, /4 projects in 1 group/);
  assert.equal(await s.eval('document.querySelector("[data-root-use]").disabled'), false);
  await s.input({ keys: "Enter" });

  assert.deepEqual(await until(projects, (p) => p.includes("kiln"), 20000), CLIENTS);
  const st = await until(() => s.driver("state"), (x) => !x.overlays.modal);
  assert.equal(st.overlays.modal, false, "the panel closes");
  assert.equal(st.errors, 0);
  assert.equal((await s.driver("root")).source, "chosen");
  // Labels follow the folder's name, and sessions carry on, outside it.
  assert.equal(await until(() => s.eval('document.getElementById("root-name").textContent'), (t) => t === "~/Clients"), "~/Clients");
  assert.deepEqual(await s.eval('[...document.querySelectorAll("[data-root-label]")].map((e) => e.textContent.trim())'), ["Outside ~/Clients", "~/Clients", "Outside ~/Clients"]);
  const zones = (await s.driver("find", { kind: "zone", limit: 50 })).nodes.map((n) => n.title);
  assert.ok(zones.includes("atlas-api") && zones.includes("ledger"), zones.join(", "));
  assert.equal(st.counts.sessions, before.counts.sessions);

  const back = await s.driver("setRoot", null);
  assert.deepEqual([back.root.source, back.root.changed], ["default", true]);
  assert.ok((await projects()).includes("atlas-api"));
}));

test("going back to a folder brings its layout back", { timeout: limit(90000) }, shoot("root-layout", async () => {
  const atlas = (await s.driver("find", { query: "atlas-api", kind: "project", limit: 1 })).nodes[0].id;
  await api("/api/layout", "POST", { positions: { [atlas]: { x: 40, y: -20, z: 10 } } });
  const away = await s.driver("setRoot", "~/Clients");
  assert.equal(away.root.changed, true);
  assert.ok(!(await api("/api/graph")).nodes.some((n) => n.id === atlas));
  assert.deepEqual((await api("/api/layout"))[atlas], { x: 40, y: -20, z: 10 }, "kept while another folder is in use");
  await s.driver("setRoot", null);
  const node = (await api("/api/graph")).nodes.find((n) => n.id === atlas);
  assert.deepEqual([node.fx, node.fy, node.fz, node.pinned], [40, -20, 10, true]);
  assert.deepEqual(await s.eval(`return (() => { const n = window.__xenonet.scene.getNode(${JSON.stringify(atlas)}); return n && [Math.round(n.x), Math.round(n.y), Math.round(n.z)]; })()`), [40, -20, 10], "drawn where it was");
  await api("/api/layout", "POST", { positions: { [atlas]: null } });
}));

test("your rules and Codex's picks stay with their folder", { timeout: limit(120000) }, shoot("root-rules", async () => {
  const home = (await s.driver("root")).path;
  const docs = path.join(home, "docs");
  // In ~/Projects: a rule of yours, and Codex's picks.
  await fromWindow("/api/projects/rule", "POST", { path: docs, as: "ignore" });
  assert.deepEqual((await askCodex()).codex.picks.map((p) => p.path), ["labs/snippets"]);
  const mine = await projects();
  assert.ok(mine.includes("labs/snippets:codex") && !mine.some((p) => p.startsWith("docs")), mine.join(", "));

  // ~/Clients has a labs/snippets too, and ~/Projects' picks aren't laid over it.
  await s.driver("setRoot", "~/Clients");
  assert.deepEqual(await projects(), CLIENTS);
  let detection = await api("/api/projects");
  assert.equal(detection.codex, null, "Codex hasn't looked at this folder");
  assert.deepEqual(detection.rules.map((r) => [path.basename(r.path), r.inRoot]), [["docs", false]]);
  assert.deepEqual((await askCodex()).codex.picks.map((p) => p.path), ["labs/snippets"]);
  assert.deepEqual(await projects(), [...CLIENTS, "labs/snippets:codex"].sort());

  // Back in ~/Projects, its own rule and picks apply again.
  await s.driver("setRoot", null);
  assert.deepEqual(await projects(), mine);
  detection = await api("/api/projects");
  assert.deepEqual([detection.codex.picks.length, detection.codex.stale], [1, false]);

  // And ~/Clients keeps its picks. There, the rules list undoes the rule
  // for a folder outside it.
  await s.driver("setRoot", "~/Clients");
  assert.ok((await projects()).includes("labs/snippets:codex"));
  await s.driver("command", "projects");
  const row = await until(() => s.eval('document.querySelector(".pj-rules li")?.textContent ?? ""'), (t) => /docs/.test(t));
  assert.match(row, /outside this projects folder/);
  await s.eval('document.querySelector("[data-pj-undo]").click()');
  assert.deepEqual((await until(() => api("/api/projects"), (d) => !d.rules.length)).rules, []);
  await s.input({ keys: "Escape" });

  await fromWindow("/api/projects/codex", "DELETE");
  await s.driver("setRoot", null);
  await fromWindow("/api/projects/codex", "DELETE");
  const end = await projects();
  assert.ok(end.includes("docs") && !end.includes("labs/snippets:codex"), end.join(", "));
}));

test("the script step and no page errors", { timeout: limit(60000) }, async () => {
  const { runStep } = await import("../../core/script.js");
  const said = [];
  await runStep(s, "root:~/Clients", (v) => said.push(v));
  await runStep(s, "root:reset", (v) => said.push(v));
  assert.deepEqual(said.map((v) => path.basename(v.root)), ["Clients", "Projects"]);
  const { entries } = await s.logs({ level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
